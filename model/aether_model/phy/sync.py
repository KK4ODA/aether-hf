"""Frame acquisition: timing, carrier-frequency offset and header detection (roadmap P1-5).

Stages, on complex baseband at ``fs_baseband``:

1. **Coarse detection** — the two identical Schmidl–Cox symbols make the preamble periodic
   with the symbol period; the normalised lag-``symbol_samples`` autocorrelation metric
   ``M(d) = |P(d)|² / R(d)²`` rises to ≈ (SNR/(1+SNR))² over the preamble and is ≈ 0
   elsewhere. Its plateau centre gives timing to within roughly ±CP.
2. **Fractional CFO** — from the phase of the half-symbol (lag N/2) autocorrelation at the
   coarse position; unambiguous only modulo twice the subcarrier spacing (80 Hz).
3. **Fine timing and the 80 Hz ambiguity** — the CFO-corrected signal is cross-correlated
   with the known two-symbol Schmidl–Cox waveform around the coarse position, once for
   every hypothesis ``f_coarse + 80·k`` (k = −3 … 3, covering ±280 Hz). A residual CFO of
   even a few hertz decorrelates the 62 ms matched filter, so the hypothesis with the
   highest normalised peak is unambiguous; its peak locates the preamble to the sample
   (strongest path; the mid-CP FFT window tolerates ±2.5 ms of earlier or later paths).
   The CFO is then refined from the full-symbol-lag autocorrelation (±16 Hz range, but the
   residual is now within a couple of hertz).
4. **Integer CFO and header** — FFTs of the second Schmidl–Cox symbol, the unique word and
   the full pilot symbol that follows it. Three channel-blind statistics are correlated with
   their known values for every (header code, integer bin shift) hypothesis: the SC2 → UW
   ratio on the even carriers, the UW's adjacent-carrier differential, and the UW → pilot
   ratio. Ratios of consecutive symbols cancel the channel (and any residual timing ramp);
   the differential cancels it too because adjacent carriers see nearly the same channel.
   The best hypothesis gives the header and completes the CFO estimate.

The result is a :class:`FrameSync` (start sample of the first preamble symbol, total CFO,
header, and quality metrics for diagnostics).
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.phy.ofdm import OfdmDemodulator, OfdmModulator
from aether_model.phy.passband import band_limit_taps
from aether_model.phy.preamble import FrameHeader, preamble
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

MAX_INTEGER_SHIFT = 2
"""Residual integer-CFO search in the header stage (safety net; the ambiguity is resolved
by the matched-filter hypothesis test)."""
CFO_HYPOTHESES = range(-3, 4)
"""Multiples of 2·Δf (80 Hz) tested against the matched filter: ±240 Hz + fractional."""


@dataclass(frozen=True)
class FrameSync:
    start: int
    """Sample index (in the analysed buffer) where the first preamble symbol period begins."""
    cfo_hz: float
    header: FrameHeader
    coarse_metric: float
    """Peak Schmidl–Cox metric (≈ (SNR/(1+SNR))²)."""
    header_confidence: float
    """Best differential-correlation metric divided by the runner-up."""
    timing_peak: float
    """Normalised matched-filter peak (1.0 = perfect match)."""


def _moving_sum(x: NDArray, length: int) -> NDArray:
    c = np.cumsum(np.concatenate(([0.0], x)))
    return c[length:] - c[:-length]


class FrameDetector:
    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        *,
        threshold: float = 0.06,
        min_timing_peak: float = 0.33,
        min_header_confidence: float = 1.3,
        max_candidates: int = 12,
        min_gap_samples: int | None = None,
    ) -> None:
        """Thresholds, from noise-only measurements on band-limited noise (10 s buffers):
        the coarse metric's noise floor peaks around 0.10 (correlated noise inflates the
        Schmidl–Cox statistic), so it only *nominates* candidates (``threshold`` 0.06 keeps
        preambles down to ≈ −6 dB in the running); acceptance needs the 496-sample
        matched-filter peak (noise ≤ 0.26; preamble ≥ 0.42 at −4 dB) and the header
        confidence (noise ≤ 1.23; preamble ≥ 1.9 at 0 dB, ≥ 1.25 at −4 dB)."""
        self.p = params
        self.min_timing_peak = min_timing_peak
        self.min_header_confidence = min_header_confidence
        self.max_candidates = max_candidates
        self._band_taps = band_limit_taps(params)
        self.pre = preamble(params)
        self.mod = OfdmModulator(params)
        self.dem = OfdmDemodulator(params)
        self.threshold = threshold
        self.n = params.fft_size
        self.period = params.symbol_samples
        self.min_gap = min_gap_samples or 4 * self.period
        # Known two-symbol Schmidl–Cox waveform for the matched filter (no trailing taper).
        sc2 = self.mod.modulate([self.pre.sc_values, self.pre.sc_values])[: 2 * self.period]
        self._sc_ref = sc2 / np.sqrt(np.sum(np.abs(sc2) ** 2))
        # Known statistics for every header code: SC2→UW ratio (even carriers), UW
        # differential, UW→pilot ratio — each row normalised.
        cands = self.pre.uw_candidates()
        self._codes = sorted(cands)
        pilot = self.dem.cmap.pilot_sequence
        even = self.pre.even
        sc = self.pre.sc_values

        def rows(fn: Callable[[ComplexArray], ComplexArray]) -> ComplexArray:
            r = np.stack([fn(cands[c]) for c in self._codes])
            return r / np.linalg.norm(r, axis=1, keepdims=True)

        self._even = even
        self._ref_sc_uw = rows(lambda u: np.conj(sc[even]) * u[even])
        self._ref_diff = rows(lambda u: u[1:] * np.conj(u[:-1]))
        self._ref_uw_pilot = rows(lambda u: np.conj(u) * pilot)

    # ── stage 0: conditioning ─────────────────────────────────────────

    def condition(self, x: ComplexArray) -> ComplexArray:
        """Band-limit to the signal (removes out-of-band noise that would depress every
        metric); output is aligned with the input (group delay compensated, tail zero-padded).
        Idempotent enough to apply after :class:`AudioToBaseband` as well."""
        x = np.asarray(x, dtype=np.complex128)
        y = signal.lfilter(
            self._band_taps, [1.0], np.concatenate((x, np.zeros(len(self._band_taps))))
        )
        delay = (len(self._band_taps) - 1) // 2
        return np.asarray(y[delay : delay + len(x)], dtype=np.complex128)

    # ── stage 1: coarse metric ────────────────────────────────────────

    def coarse_metric(self, x: ComplexArray) -> FloatArray:
        """M(d) for every start position d where a full two-symbol span fits."""
        lag = self.period
        if len(x) < 2 * lag:
            return np.zeros(0)
        prod = np.conj(x[:-lag]) * x[lag:]
        p = _moving_sum(prod, lag)
        energy = _moving_sum(np.abs(x) ** 2, lag)  # window energy at every position
        r = 0.5 * (energy[:-lag] + energy[lag:])  # mean energy of both windows → M ≤ 1
        floor = max(1e-3 * float(np.mean(np.abs(x) ** 2)) * lag, 1e-30)  # ignore silence
        return np.abs(p) ** 2 / np.maximum(r, floor) ** 2

    # ── stage 2: fractional CFO ───────────────────────────────────────

    def coarse_cfo(self, x: ComplexArray, d: int) -> float:
        """Half-symbol-lag estimate at coarse position ``d`` (unambiguous modulo 80 Hz)."""
        half = self.n // 2
        fs = self.p.fs_baseband
        a = d + self.p.cp_samples  # inside the first SC symbol's useful part
        seg = x[a : a + self.n]
        c1 = np.vdot(seg[:half], seg[half:])  # Σ conj(x[m]) x[m+half]
        return float(np.angle(c1) * fs / (2 * np.pi * half))

    def refine_cfo(self, x: ComplexArray, start: int, cfo_hz: float) -> float:
        """Full-symbol-lag refinement (±16 Hz range) at the fine timing ``start``."""
        fs = self.p.fs_baseband
        t = (np.arange(2 * self.period) + start) / fs
        y = x[start : start + 2 * self.period] * np.exp(-2j * np.pi * cfo_hz * t)
        c2 = np.vdot(y[: self.period], y[self.period :])
        return float(cfo_hz + np.angle(c2) * fs / (2 * np.pi * self.period))

    # ── stage 3: fine timing ──────────────────────────────────────────

    def fine_timing(self, x: ComplexArray, d: int, cfo_hz: float, span: int) -> tuple[int, float]:
        fs = self.p.fs_baseband
        lo = max(0, d - span)
        hi = min(len(x) - 2 * self.period, d + span)
        if hi <= lo:
            return d, 0.0
        t = np.arange(lo, hi + 2 * self.period) / fs
        y = x[lo : hi + 2 * self.period] * np.exp(-2j * np.pi * cfo_hz * t)
        corr = np.abs(np.correlate(y, self._sc_ref, mode="valid"))  # positions lo … hi
        energy = np.sqrt(_moving_sum(np.abs(y) ** 2, 2 * self.period))
        norm = corr / np.maximum(energy[: len(corr)], 1e-12)
        peak = float(norm.max())
        return lo + int(np.argmax(norm)), peak

    # ── stage 4: header and integer CFO ───────────────────────────────

    def header_and_integer_cfo(
        self, x: ComplexArray, start: int, cfo_hz: float
    ) -> tuple[int, int, float]:
        """Returns ``(header code, integer bin shift, confidence)``; the code may be a reserved
        value on noise, which the caller treats as a rejection."""
        fs = self.p.fs_baseband

        def spectrum_of(symbol_index: int) -> ComplexArray:
            seg_start = start + symbol_index * self.period + self.dem.fft_offset
            seg = x[seg_start : seg_start + self.n]
            if len(seg) < self.n:
                raise ValueError("preamble runs past the end of the buffer")
            t = (np.arange(self.n) + seg_start) / fs
            return np.fft.fft(seg * np.exp(-2j * np.pi * cfo_hz * t))

        s_sc2, s_uw, s_pilot = spectrum_of(1), spectrum_of(2), spectrum_of(3)
        bins = self.dem.cmap.bins
        best = (-1.0, 0, 0)
        second = 0.0

        def corr(ref: ComplexArray, z: ComplexArray) -> FloatArray:
            return np.abs(ref @ np.conj(z)) / max(np.linalg.norm(z), 1e-12)

        for m in range(-MAX_INTEGER_SHIFT, MAX_INTEGER_SHIFT + 1):
            idx = (bins + m) % self.n
            y_uw = s_uw[idx]
            metrics = (
                corr(self._ref_sc_uw, np.conj(s_sc2[idx][self._even]) * y_uw[self._even])
                + corr(self._ref_diff, y_uw[1:] * np.conj(y_uw[:-1]))
                + corr(self._ref_uw_pilot, np.conj(y_uw) * s_pilot[idx])
            ) / 3.0
            k = int(np.argmax(metrics))
            val = float(metrics[k])
            if val > best[0]:
                second = max(second, best[0])
                best = (val, k, m)
            else:
                second = max(second, val)
        val, k, m = best
        confidence = val / max(second, 1e-12)
        return self._codes[k], m, confidence

    # ── full acquisition ──────────────────────────────────────────────

    def detect(self, x: ComplexArray, max_frames: int = 1) -> list[FrameSync]:
        """Find up to ``max_frames`` preambles in ``x`` (offline, whole-buffer).

        Candidates are the strongest coarse-metric plateaus above ``threshold``, examined in
        order of metric value; each is accepted or rejected on the matched-filter peak and
        header confidence, so periodic interferers (tones, CW) and noise peaks are dropped
        without stopping the search."""
        x = np.asarray(x, dtype=np.complex128)
        metric = self.coarse_metric(x)
        found: list[FrameSync] = []
        mask = metric >= self.threshold
        two_df = 2 * self.p.subcarrier_spacing_hz
        for _ in range(self.max_candidates):
            if len(found) >= max_frames:
                break
            idx = np.flatnonzero(mask)
            if len(idx) == 0:
                break
            d_peak = int(idx[np.argmax(metric[idx])])
            run_lo = d_peak
            while run_lo > 0 and mask[run_lo - 1]:
                run_lo -= 1
            run_hi = d_peak
            while run_hi + 1 < len(mask) and mask[run_hi + 1]:
                run_hi += 1
            d0 = (run_lo + run_hi) // 2
            reject_lo, reject_hi = max(0, d0 - self.period), min(len(mask), d0 + self.period)
            f_coarse = self.coarse_cfo(x, d0)
            start, tpeak, f_hyp = d0, -1.0, f_coarse
            for k in CFO_HYPOTHESES:
                f_k = f_coarse + k * two_df
                s_k, p_k = self.fine_timing(x, d0, f_k, span=self.period)
                if p_k > tpeak:
                    start, tpeak, f_hyp = s_k, p_k, f_k
            if tpeak < self.min_timing_peak:
                mask[reject_lo:reject_hi] = False
                continue
            f_frac = self.refine_cfo(x, start, f_hyp)
            try:
                code, m, conf = self.header_and_integer_cfo(x, start, f_frac)
            except ValueError:  # preamble would run past the end of the buffer
                mask[reject_lo:reject_hi] = False
                continue
            if code > 16 or conf < self.min_header_confidence:
                mask[reject_lo:reject_hi] = False
                continue
            cfo = f_frac + m * self.p.subcarrier_spacing_hz
            header = FrameHeader.from_code(code)
            found.append(FrameSync(start, cfo, header, float(metric[d_peak]), conf, tpeak))
            mask[max(0, start - self.min_gap) : min(len(mask), start + self.min_gap)] = False
        return sorted(found, key=lambda f: f.start)
