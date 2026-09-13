"""Frame acquisition: timing, carrier-frequency offset and header detection (P1-5, P2-3).

Stages, on band-limited complex baseband at ``fs_baseband``:

1. **Matched-filter bank (PMF-FFT).** The known two-symbol Schmidl–Cox waveform (496
   samples) is split into 62 segments of 8 samples. Each segment is correlated with the
   signal at every timing position; an FFT across the 62 segment outputs (zero-padded to
   256 bins) evaluates the full 496-sample matched filter for every carrier-offset
   hypothesis on a 3.9 Hz grid over ±300 Hz, in one vectorised pass. The statistic at
   position ``d`` is the best bin's normalised correlation (1.0 = perfect match). This is
   the classic partial-matched-filter/FFT acquisition used in GNSS and burst modems: the
   full 27 dB processing gain of the preamble with no time/frequency ambiguity (the SC
   sequence is PN, not a chirp) and less than ~2 dB of segment/scallop loss at the edge
   of the range.
2. **Fine CFO.** At the found position the segmented matched filter is evaluated on a
   0.24 Hz grid (a longer FFT over the same 62 segment outputs), then refined with the
   full-symbol-lag phase (±16 Hz range, well inside the fine-grid residual). The
   half-symbol Schmidl–Cox estimate is not used: at low SNR its error occasionally exceeds
   the refinement's range and aliases by 32 Hz.
3. **Frame type.** The bank is run for both Schmidl–Cox sequences (DATA / CONTROL); the
   type is whichever reference wins at the candidate position, with the ratio of the two
   peaks as the confidence. Because this decision rides on the full preamble gain it is
   essentially error-free wherever the frame is detectable. The mode of a DATA frame is
   read later by the receiver from the pilot-symbol chips (see :mod:`preamble`).

Candidates are the local maxima of the bank statistic above ``min_timing_peak``. The
statistic's noise maximum over 60 s of band-limited noise is ≈ 0.32; the default threshold
of 0.36 gave no false alarms in that test. The occasional false alarm at the margin costs
only a failed CRC, which is why the threshold is set for sensitivity rather than purity.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.frame.modes import LONG, SHORT
from aether_model.phy.ofdm import OfdmDemodulator, OfdmModulator
from aether_model.phy.passband import band_limit_taps
from aether_model.phy.preamble import FrameHeader, FrameType, preamble
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

SEGMENT_LEN = 8
FFT_LEN = 256


@dataclass(frozen=True)
class FrameSync:
    start: int
    """Sample index (in the analysed buffer) where the first preamble symbol period begins."""
    cfo_hz: float
    header: FrameHeader
    coarse_metric: float
    """Bank statistic's raw bin frequency estimate (Hz) — kept for diagnostics."""
    header_confidence: float
    """Bank peak of the winning frame-type reference divided by the other type's peak."""
    timing_peak: float
    """Normalised matched-filter-bank peak (1.0 = perfect match)."""


def _moving_sum(x: NDArray, length: int) -> NDArray:
    c = np.cumsum(np.concatenate(([0.0], x)))
    return c[length:] - c[:-length]


class FrameDetector:
    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        *,
        min_timing_peak: float = 0.36,
        max_cfo_hz: float = 300.0,
        max_candidates: int = 16,
        min_gap_samples: int | None = None,
    ) -> None:
        self.p = params
        self.min_timing_peak = min_timing_peak
        self.max_cfo_hz = max_cfo_hz
        self.max_candidates = max_candidates
        self._band_taps = band_limit_taps(params)
        self.pre = preamble(params)
        self.mod = OfdmModulator(params)
        self.dem = OfdmDemodulator(params)
        self.n = params.fft_size
        self.period = params.symbol_samples
        self.min_gap = min_gap_samples or 4 * self.period
        # Known two-symbol Schmidl–Cox waveforms (unit energy), one per frame type,
        # segmented for the bank.
        self._types = [FrameType.DATA, FrameType.CONTROL]
        self._ref_segs = []
        for ft in self._types:
            sc = self.pre.sc_values(ft)
            wave = self.mod.modulate([sc, sc])[: 2 * self.period]
            wave = wave / np.sqrt(np.sum(np.abs(wave) ** 2))
            n_seg = len(wave) // SEGMENT_LEN
            self._ref_segs.append(wave[: n_seg * SEGMENT_LEN].reshape(n_seg, SEGMENT_LEN))
        self.n_seg = self._ref_segs[0].shape[0]
        self._ref_len = self.n_seg * SEGMENT_LEN
        # Bank bin → frequency, and the bins inside the search range.
        fs = params.fs_baseband
        self._bin_hz = np.fft.fftfreq(FFT_LEN, d=SEGMENT_LEN / fs)
        self._bins_ok = np.flatnonzero(np.abs(self._bin_hz) <= max_cfo_hz)

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

    # ── stage 1: matched-filter bank ──────────────────────────────────

    def bank(self, x: ComplexArray, chunk: int = 8192) -> tuple[FloatArray, FloatArray, FloatArray]:
        """For every start position: (best normalised peak over both frame types, CFO of
        the best bin in Hz, peak of the *other* type at that position)."""
        x = np.asarray(x, dtype=np.complex128)
        n_pos = len(x) - self._ref_len + 1
        if n_pos <= 0:
            return np.zeros(0), np.zeros(0), np.zeros(0)
        energy = np.sqrt(np.maximum(_moving_sum(np.abs(x) ** 2, self._ref_len), 1e-30))
        floor = 1e-3 * float(np.sqrt(np.mean(np.abs(x) ** 2))) * np.sqrt(self._ref_len)
        peaks = np.zeros((len(self._types), n_pos))
        cfos = np.zeros((len(self._types), n_pos))
        for p0 in range(0, n_pos, chunk):
            p1 = min(n_pos, p0 + chunk)
            m = p1 - p0
            norm = np.maximum(energy[p0:p1], floor)
            for ti, ref in enumerate(self._ref_segs):
                parts = np.empty((m, self.n_seg), dtype=np.complex128)
                for k in range(self.n_seg):
                    a = p0 + k * SEGMENT_LEN
                    seg = x[a : a + m + SEGMENT_LEN - 1]
                    parts[:, k] = np.correlate(seg, ref[k], mode="valid")
                mag = np.abs(np.fft.fft(parts, n=FFT_LEN, axis=1)[:, self._bins_ok])
                best = np.argmax(mag, axis=1)
                peaks[ti, p0:p1] = mag[np.arange(m), best] / norm
                cfos[ti, p0:p1] = self._bin_hz[self._bins_ok][best]
        win = np.argmax(peaks, axis=0)
        cols = np.arange(n_pos)
        self._last_type = win
        return peaks[win, cols], cfos[win, cols], peaks[1 - win, cols]

    # ── stage 2: CFO refinement ───────────────────────────────────────

    def fine_cfo(self, x: ComplexArray, start: int, frame_type: FrameType) -> float:
        """CFO at a known preamble position: the segmented matched filter evaluated on a
        fine frequency grid (0.24 Hz), then the full-symbol-lag refinement (±16 Hz range —
        safe because the fine-grid estimate leaves a residual of a hertz or two)."""
        fs = self.p.fs_baseband
        ref = self._ref_segs[self._types.index(frame_type)]
        seg = x[start : start + self._ref_len].reshape(self.n_seg, SEGMENT_LEN)
        parts = np.sum(seg * np.conj(ref), axis=1)
        n_fine = 16 * FFT_LEN
        spectrum = np.fft.fft(parts, n=n_fine)
        freqs = np.fft.fftfreq(n_fine, d=SEGMENT_LEN / fs)
        ok = np.abs(freqs) <= self.max_cfo_hz
        f0 = float(freqs[ok][np.argmax(np.abs(spectrum[ok]))])
        t = (np.arange(2 * self.period) + start) / fs
        y = x[start : start + 2 * self.period] * np.exp(-2j * np.pi * f0 * t)
        c2 = np.vdot(y[: self.period], y[self.period :])
        return f0 + float(np.angle(c2) * fs / (2 * np.pi * self.period))

    # ── full acquisition ──────────────────────────────────────────────

    def detect(self, x: ComplexArray, max_frames: int = 1) -> list[FrameSync]:
        """Find up to ``max_frames`` preambles in ``x`` (offline, whole-buffer)."""
        x = np.asarray(x, dtype=np.complex128)
        peak, bin_cfo, other = self.bank(x)
        found: list[FrameSync] = []
        if len(peak) == 0:
            return found
        mask = peak >= self.min_timing_peak
        # The two SC symbols are identical, so a preamble preceded by silence also produces a
        # ≈ 0.7 sidelobe one symbol early. Never accept a peak until the statistic one symbol
        # later exists (a stronger one there wins); in streaming use the caller re-scans.
        mask[max(0, len(mask) - self.period) :] = False
        for _ in range(self.max_candidates):
            if len(found) >= max_frames:
                break
            idx = np.flatnonzero(mask)
            if len(idx) == 0:
                break
            start = int(idx[np.argmax(peak[idx])])
            reject_lo, reject_hi = max(0, start - self.period), min(len(mask), start + self.period)
            later = start + self.period
            if later < len(peak) and peak[later] > peak[start]:
                mask[reject_lo:reject_hi] = False  # this was the early sidelobe
                continue
            if start + 4 * self.period + self.dem.fft_offset > len(x):
                mask[reject_lo:reject_hi] = False  # too close to the end to be usable yet
                continue
            header = FrameHeader(self._types[int(self._last_type[start])])
            cfo = self.fine_cfo(x, start, header.frame_type)
            confidence = float(peak[start] / max(other[start], 1e-12))
            found.append(
                FrameSync(start, cfo, header, float(bin_cfo[start]), confidence, float(peak[start]))
            )
            # Nothing else can start inside this frame (strong data symbols correlate with
            # the reference at ≈ 0.3–0.4, which the threshold does not exclude).
            span = (LONG if header.frame_type is FrameType.DATA else SHORT).samples
            mask[max(0, start - self.min_gap) : min(len(mask), start + span)] = False
        return sorted(found, key=lambda f: f.start)
