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
4. **The floor family (ADR-0009).** A floor frame's preamble is eight symbols of the
   floor family's own Schmidl–Cox sequence for its type, so its two-symbol statistic on
   the floor reference is a run of seven full peaks a symbol apart, and neither family's
   references fire on the other's frames. On an air with a floor family the bank also runs
   the two floor references and averages their normalised statistic over those seven
   windows — non-coherently, which the prototype measured within half a decibel of
   coherent combining on AWGN and which does not care about phase drift across a quarter
   second on a fading channel — and that *floor statistic* has its own, lower noise floor
   and threshold: a floor frame is found about 4.5 dB below where an ordinary one is. A
   floor candidate's start is refined within a symbol and a half by combining four
   two-symbol windows coherently on a fine frequency sub-grid (the averaged statistic is
   a symbol wide at its top), its CFO is estimated from all seven symbol lags, and the
   candidate is checked for the preamble's repetition over its seven lags (noise does
   not repeat). With twelve carriers, and the bank's search over frequency, a strong
   frame's *body* of either family scores 0.4–0.75 on the other family's references —
   so both passes run on the full statistics and, where a candidate of one family lies
   inside the other's frame, the contest is settled by evidence: a genuine floor frame
   scores the signal's share of the power on its own statistic and at most three
   quarters of it on the ordinary peak, a genuine ordinary frame its share on its peak
   and at most half on the floor statistic. The ordinary path itself is untouched.

Candidates are the local maxima of the bank statistic above ``min_timing_peak``, which
defaults to the air interface's ``acquisition_threshold``:
the statistic's noise maximum over 60 s of band-limited noise is ≈ 0.35 at 2 300 Hz and
≈ 0.55 at 500 Hz (a fifth of the degrees of freedom in a preamble's span), and the
thresholds 0.36 and 0.56 gave no false alarms in that test. The occasional false alarm at
the margin costs only a failed CRC, which is why they are set for sensitivity rather than
purity.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.frame.modes import PREAMBLE_SYMBOLS, air_interface
from aether_model.phy.ofdm import OfdmDemodulator, OfdmModulator
from aether_model.phy.passband import band_limit_taps
from aether_model.phy.preamble import FrameHeader, FrameType, preamble
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

SEGMENT_LEN = 8
FFT_LEN = 256
FLOOR_REPETITION_MIN = 0.1
FLOOR_OVER_ORDINARY = 0.85
"""When a candidate of one family lies inside the other family's frame, the floor one is
kept if its statistic is at least this fraction of the ordinary one's peak. A genuine
floor frame scores about the signal's share of the power on its own statistic and at
most three quarters of that on the ordinary references; a genuine ordinary frame scores
its share on its own peak and at most half of it on the floor statistic — so the ratio
sits at 1.3 or above for the one and 0.5 or below for the other."""
"""The least a floor candidate's eight symbols may repeat one another: the magnitude of
the summed one-symbol-lag correlation over the energy the lags span. A floor preamble
shows the signal's share of the power (0.2 at −13 dB); noise shows a few hundredths."""
FLOOR_SUB_GRID_HZ = (-1.5625, -0.78125, 0.0, 0.78125, 1.5625)
"""Frequency sub-steps inside one 3.9 Hz bank bin for the coherent timing refinement of a
floor candidate: four windows two symbols apart drift 180° over a bin's half-width, so the
combination is tried on this grid and the best kept."""


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
    """Normalised matched-filter-bank peak (1.0 = perfect match); for a floor frame, the
    floor statistic at the candidate."""
    floor: bool = False
    """The frame is of the floor family (ADR-0009): an eight-symbol preamble of the floor
    sequences and the floor layouts; ``start`` is then the first of the eight."""


def _moving_sum(x: NDArray, length: int) -> NDArray:
    c = np.cumsum(np.concatenate(([0.0], x)))
    return c[length:] - c[:-length]


class FrameDetector:
    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        *,
        min_timing_peak: float | None = None,
        max_cfo_hz: float = 300.0,
        max_candidates: int = 16,
        min_gap_samples: int | None = None,
    ) -> None:
        self.p = params
        self.air = air_interface(params)
        self.min_timing_peak = (
            self.air.acquisition_threshold if min_timing_peak is None else min_timing_peak
        )
        self.max_cfo_hz = max_cfo_hz
        self.max_candidates = max_candidates
        self._band_taps = band_limit_taps(params)
        self.pre = preamble(params)
        self.mod = OfdmModulator(params)
        self.dem = OfdmDemodulator(params)
        self.n = params.fft_size
        self.period = params.symbol_samples
        self.min_gap = min_gap_samples or 4 * self.period
        # The floor family (ADR-0009): its preamble length and the threshold of the
        # averaged statistic that finds it.
        self.has_floor = self.air.floor_long is not None
        self.floor_symbols = max(layout.preamble_symbols for layout in self.air.layouts)
        self.floor_windows = self.floor_symbols - 1
        """Two-symbol windows, a symbol apart, that a floor preamble fills."""
        self.min_floor_peak = self.air.floor_acquisition_threshold
        # Known two-symbol Schmidl–Cox waveforms (unit energy), one per frame type and
        # family, segmented for the bank.
        self._types = [FrameType.DATA, FrameType.CONTROL]
        self._families = [False, True] if self.has_floor else [False]
        self._ref_segs: dict[tuple[FrameType, bool], ComplexArray] = {}
        for floor in self._families:
            for ft in self._types:
                sc = self.pre.sc_values(ft, floor)
                wave = self.mod.modulate([sc, sc])[: 2 * self.period]
                wave = wave / np.sqrt(np.sum(np.abs(wave) ** 2))
                n_seg = len(wave) // SEGMENT_LEN
                self._ref_segs[ft, floor] = wave[: n_seg * SEGMENT_LEN].reshape(n_seg, SEGMENT_LEN)
        self.n_seg = self._ref_segs[FrameType.DATA, False].shape[0]
        self._ref_len = self.n_seg * SEGMENT_LEN
        # Bank bin → frequency, and the bins inside the search range.
        fs = params.fs_baseband
        self._bin_hz = np.fft.fftfreq(FFT_LEN, d=SEGMENT_LEN / fs)
        self._bins_ok = np.flatnonzero(np.abs(self._bin_hz) <= max_cfo_hz)
        self._floor_stat = np.zeros(0)
        self._floor_other = np.zeros(0)
        self._floor_cfo = np.zeros(0)
        self._floor_type = np.zeros(0, dtype=np.intp)

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

    def _spectra(self, x: ComplexArray, p0: int, p1: int, ref: ComplexArray) -> ComplexArray:
        """The segmented matched filter's FFT outputs (complex, positions ``p0 … p1`` ×
        search bins) for one reference; positions past the last valid one are dropped."""
        n_pos = len(x) - self._ref_len + 1
        m = min(p1, n_pos) - p0
        if m <= 0:
            return np.zeros((0, len(self._bins_ok)), dtype=np.complex128)
        parts = np.empty((m, self.n_seg), dtype=np.complex128)
        for k in range(self.n_seg):
            a = p0 + k * SEGMENT_LEN
            seg = x[a : a + m + SEGMENT_LEN - 1]
            parts[:, k] = np.correlate(seg, ref[k], mode="valid")
        return np.asarray(np.fft.fft(parts, n=FFT_LEN, axis=1)[:, self._bins_ok])

    def bank(self, x: ComplexArray, chunk: int = 8192) -> tuple[FloatArray, FloatArray, FloatArray]:
        """For every start position: (best normalised peak over both frame types, CFO of
        the best bin in Hz, peak of the *other* type at that position). On an air with a
        floor family the floor statistic — the floor references' normalised peak averaged
        over the seven windows a floor preamble fills — is kept alongside for
        :meth:`detect`."""
        x = np.asarray(x, dtype=np.complex128)
        n_pos = len(x) - self._ref_len + 1
        if n_pos <= 0:
            self._floor_stat = self._floor_other = self._floor_cfo = np.zeros(0)
            self._floor_type = np.zeros(0, dtype=np.intp)
            return np.zeros(0), np.zeros(0), np.zeros(0)
        energy = np.sqrt(np.maximum(_moving_sum(np.abs(x) ** 2, self._ref_len), 1e-30))
        floor = 1e-3 * float(np.sqrt(np.mean(np.abs(x) ** 2))) * np.sqrt(self._ref_len)
        norm = np.maximum(energy, floor)
        peaks = np.zeros((len(self._types), n_pos))
        cfos = np.zeros((len(self._types), n_pos))
        fstats = np.zeros((len(self._types), n_pos))
        fcfos = np.zeros((len(self._types), n_pos))
        bins = self._bin_hz[self._bins_ok]
        ext = (self.floor_windows - 1) * self.period if self.has_floor else 0
        for p0 in range(0, n_pos, chunk):
            p1 = min(n_pos, p0 + chunk)
            m = p1 - p0
            for ti, ft in enumerate(self._types):
                spec = self._spectra(x, p0, p1, self._ref_segs[ft, False])
                mag = np.abs(spec)
                best = np.argmax(mag, axis=1)
                peaks[ti, p0:p1] = mag[np.arange(m), best] / norm[p0:p1]
                cfos[ti, p0:p1] = bins[best]
                if not self.has_floor:
                    continue
                spec = self._spectra(x, p0, p1 + ext, self._ref_segs[ft, True])
                mag = np.abs(spec) / norm[p0 : p0 + len(spec), None]
                # positions whose seven windows all lie inside the buffer
                m_f = len(spec) - ext
                if m_f <= 0:
                    continue
                acc = mag[:m_f].copy()
                for k in range(1, self.floor_windows):
                    acc += mag[k * self.period : k * self.period + m_f]
                acc /= self.floor_windows
                fbest = np.argmax(acc, axis=1)
                fstats[ti, p0 : p0 + m_f] = acc[np.arange(m_f), fbest]
                fcfos[ti, p0 : p0 + m_f] = bins[fbest]
        win = np.argmax(peaks, axis=0)
        cols = np.arange(n_pos)
        self._last_type = win
        fwin = np.argmax(fstats, axis=0)
        self._floor_type = fwin
        self._floor_stat = fstats[fwin, cols]
        self._floor_other = fstats[1 - fwin, cols]
        self._floor_cfo = fcfos[fwin, cols]
        return peaks[win, cols], cfos[win, cols], peaks[1 - win, cols]

    # ── stage 2: CFO refinement ───────────────────────────────────────

    def fine_cfo(
        self,
        x: ComplexArray,
        start: int,
        frame_type: FrameType,
        preamble_symbols: int = PREAMBLE_SYMBOLS,
        floor: bool = False,
    ) -> float:
        """CFO at a known preamble position: the segmented matched filter evaluated on a
        fine frequency grid (0.24 Hz), then the full-symbol-lag refinement (±16 Hz range —
        safe because the fine-grid estimate leaves a residual of a hertz or two). A floor
        preamble (``preamble_symbols`` = 8, ``floor``) is matched whole — its two-symbol
        reference tiled — and the lag phase is averaged over all seven symbol pairs."""
        fs = self.p.fs_baseband
        ref = self._ref_segs[frame_type, floor]
        reps = preamble_symbols // 2
        seg = x[start : start + reps * self._ref_len].reshape(reps * self.n_seg, SEGMENT_LEN)
        parts = np.sum(seg * np.conj(np.tile(ref, (reps, 1))), axis=1)
        n_fine = 16 * FFT_LEN
        spectrum = np.fft.fft(parts, n=n_fine)
        freqs = np.fft.fftfreq(n_fine, d=SEGMENT_LEN / fs)
        ok = np.abs(freqs) <= self.max_cfo_hz
        f0 = float(freqs[ok][np.argmax(np.abs(spectrum[ok]))])
        n = preamble_symbols * self.period
        t = (np.arange(n) + start) / fs
        y = x[start : start + n] * np.exp(-2j * np.pi * f0 * t)
        per = self.period
        c2: complex = 0j
        for k in range(preamble_symbols - 1):
            c2 += complex(np.vdot(y[k * per : (k + 1) * per], y[(k + 1) * per : (k + 2) * per]))
        return f0 + float(np.angle(c2) * fs / (2 * np.pi * per))

    # ── the floor family ──────────────────────────────────────────────

    def _refine_floor(
        self, x: ComplexArray, lo: int, hi: int, frame_type: FrameType, f0: float
    ) -> int:
        """The start of a floor preamble within ``lo … hi``: four two-symbol windows two
        symbols apart combined coherently at the bank's bin ``f0`` and the sub-grid around
        it, each window normalised by its own energy."""
        ref = self._ref_segs[frame_type, True]
        n_win = self.floor_symbols // 2
        step = 2 * self.period
        spec = self._spectra(x, lo, hi + (n_win - 1) * step, ref)
        n = len(spec) - (n_win - 1) * step
        if n <= 0:
            return lo
        b = int(np.argmin(np.abs(self._bin_hz[self._bins_ok] - f0)))
        energy = np.sqrt(np.maximum(_moving_sum(np.abs(x) ** 2, self._ref_len), 1e-30))
        best = np.zeros(n)
        fs = self.p.fs_baseband
        for delta in FLOOR_SUB_GRID_HZ:
            acc = np.zeros(n, dtype=np.complex128)
            for k in range(n_win):
                a = lo + k * step
                rot = np.exp(-2j * np.pi * (f0 + delta) * (k * step) / fs)
                acc += spec[k * step : k * step + n, b] * rot / energy[a : a + n]
            best = np.maximum(best, np.abs(acc) / n_win)
        return lo + int(np.argmax(best))

    def _repetition(self, x: ComplexArray, start: int, symbols: int) -> float:
        """How much ``symbols`` symbol periods from ``start`` repeat one another: the
        magnitude of the summed one-symbol-lag correlation over the energy the lags
        span, 1.0 for identical noiseless symbols (Schmidl & Cox's timing metric, summed
        over a run). Carrier offset only rotates every lag by the same phase."""
        per = self.period
        n = symbols * per
        if start + n > len(x):
            return 0.0
        y = x[start : start + n].reshape(symbols, per)
        lags = np.sum(np.conj(y[:-1]) * y[1:])
        energy = float(np.sum(np.abs(y) ** 2)) * (symbols - 1) / symbols
        return float(abs(lags)) / max(energy, 1e-30)

    def _detect_floor(self, x: ComplexArray, max_frames: int) -> list[FrameSync]:
        """Floor-family candidates, best first; each accepted one masks its whole span and
        the seven symbols before it (where the averaged statistic still sees part of its
        preamble). The contest with the ordinary pass is settled in :meth:`detect`."""
        found: list[FrameSync] = []
        fstat, fother = self._floor_stat, self._floor_other
        fcfo, ftype = self._floor_cfo, self._floor_type
        fmask = fstat >= self.min_floor_peak
        half = 3 * self.period // 2
        skirt = self.floor_windows * self.period
        for _ in range(self.max_candidates):
            if len(found) >= max_frames:
                break
            idx = np.flatnonzero(fmask)
            if len(idx) == 0:
                break
            c = int(idx[np.argmax(fstat[idx])])
            lo, hi = max(0, c - half), min(len(fstat), c + half + 1)
            frame_type = self._types[int(ftype[c])]
            d = self._refine_floor(x, lo, hi, frame_type, float(fcfo[c]))
            span = self.air.layout_for(frame_type is FrameType.DATA, floor=True).samples
            if d + span + self.dem.fft_offset > len(x):
                fmask[lo:hi] = False  # too close to the end to be usable yet
                continue
            cfo = self.fine_cfo(x, d, frame_type, self.floor_symbols, True)
            if self._repetition(x, d, self.floor_symbols) < FLOOR_REPETITION_MIN:
                fmask[lo:hi] = False  # the bank liked it; the symbols do not repeat
                continue
            confidence = float(fstat[c] / max(fother[c], 1e-12))
            header = FrameHeader(frame_type)
            found.append(
                FrameSync(d, cfo, header, float(fcfo[c]), confidence, float(fstat[c]), True)
            )
            a, b = max(0, d - max(self.min_gap, skirt)), min(len(fmask), d + span)
            fmask[a:b] = False
        return found

    # ── full acquisition ──────────────────────────────────────────────

    def detect(self, x: ComplexArray, max_frames: int = 1) -> list[FrameSync]:
        """Find up to ``max_frames`` preambles in ``x`` (offline, whole-buffer)."""
        x = np.asarray(x, dtype=np.complex128)
        peak, bin_cfo, other = self.bank(x)
        ordinary: list[FrameSync] = []
        if len(peak) == 0:
            return ordinary
        mask = peak >= self.min_timing_peak
        # The two SC symbols are identical, so a preamble preceded by silence also produces a
        # ≈ 0.7 sidelobe one symbol early. Never accept a peak until the statistic one symbol
        # later exists (a stronger one there wins); in streaming use the caller re-scans.
        mask[max(0, len(mask) - self.period) :] = False
        # Both passes look for up to max_frames of their own on the full statistics — a
        # strong frame's body scores on either family's references, so each pass sees the
        # other's frames — and the contest is settled by evidence below.
        for _ in range(self.max_candidates):
            if len(ordinary) >= max_frames:
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
            ordinary.append(
                FrameSync(start, cfo, header, float(bin_cfo[start]), confidence, float(peak[start]))
            )
            # Nothing else can start inside this frame (strong data symbols correlate with
            # the reference at ≈ 0.3–0.4, which the threshold does not exclude).
            span = self.air.layout_for(header.frame_type is FrameType.DATA).samples
            mask[max(0, start - self.min_gap) : min(len(mask), start + span)] = False
        found = self._detect_floor(x, max_frames) if self.has_floor else []
        if found:
            found, ordinary = self._settle_families(found, ordinary)
        return sorted(found + ordinary, key=lambda f: f.start)[:max_frames]

    def _span(self, sync: FrameSync) -> tuple[int, int]:
        layout = self.air.layout_for(sync.header.frame_type is FrameType.DATA, sync.floor)
        return sync.start, sync.start + layout.samples

    def _settle_families(
        self, floor: list[FrameSync], ordinary: list[FrameSync]
    ) -> tuple[list[FrameSync], list[FrameSync]]:
        """Where a candidate of one family lies inside the other's frame, one of them is the
        other's body or preamble scoring on the wrong references: the floor one stays if
        its statistic is at least ``FLOOR_OVER_ORDINARY`` of the ordinary peak, else the
        ordinary one does. Strongest ordinary candidates are settled first."""
        kept: list[FrameSync] = []
        for o in sorted(ordinary, key=lambda f: -f.timing_peak):
            o_start, o_end = self._span(o)
            clash = [f for f in floor if o_start < self._span(f)[1] and f.start < o_end]
            if any(f.timing_peak >= FLOOR_OVER_ORDINARY * o.timing_peak for f in clash):
                continue  # a floor frame explains it
            floor = [f for f in floor if f not in clash]
            kept.append(o)
        return floor, kept
