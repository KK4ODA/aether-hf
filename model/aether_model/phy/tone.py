"""The tone floor (P9-8, ADR-0013): a steady-envelope M-FSK frame family.

Below the ordinary tables an OFDM frame spends its energy on things a weak signal cannot
afford: a peak-to-average ratio of 5.6–5.9 dB that a peak-limited transmitter takes out of
the average power (``bench/baselines/peak_to_average.csv``), pilots and a cyclic prefix, and
channel estimates that fail before the code does. The tone floor sends **one tone at a
time** with a continuous phase, so its envelope is constant and it goes out at the OFDM
frames' *peak* amplitude (:data:`TONE_GAIN_DB` above their average power) — the transmitter
turns its whole peak power into average power — and it is detected **non-coherently**, from
the energy in each tone, so it needs no channel estimate at all. It is the classic
weak-signal design, taken from textbooks and public sources, not from any other modem:

* **Orthogonal M-FSK detected by energy** (Proakis & Salehi, *Digital Communications*,
  non-coherent orthogonal signalling): sixteen tones spaced at the symbol rate, each symbol
  carrying four Gray-labelled coded bits; a bit's soft decision is the log-sum of the tone
  metrics labelled 0 against those labelled 1.
* **Costas-array synchronisation** (J. P. Costas, *Proc. IEEE*, 1984): sync blocks whose
  tones have distinct time–frequency displacements, so a shifted copy of a block matches it
  in at most one symbol — timing and carrier offset are found together — and three blocks a
  frame (start, middle, end) survive a fade that swallows one. The same idea synchronises
  MIL-STD-188-141's link establishment and the published FT4/FT8 design; the patterns here
  are Aether's own (:data:`SYNC_PATTERNS`), and the pattern also names the frame's kind and
  redundancy version.
* **Aether's own code**: the CRC-24 and the TS 38.212 LDPC with its rate matching and
  redundancy versions, and the golden-ratio interleaver of :mod:`aether_model.frame.codec`,
  so a fade in time is spread over the whole codeword and a retransmission adds parity.

The family sits at the centre of the passband and inside 500 Hz, so the same frames serve
both air interfaces. What it costs is rate: tens of bits per second, the bottom of a table
rather than a replacement for it.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from functools import cached_property

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.codec import gray_labels, tone_codec
from aether_model.frame.modes import (
    SYNC_SYMBOLS,
    TONE_CONTROL,
    TONE_DATA,
    TONE_NUMEROLOGY,
    ToneKind,
    ToneNumerology,
)

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]
IntArray = NDArray[np.int64]

TONE_GAIN_DB = 5.5
"""Power of a tone-floor frame above an OFDM frame's average, both at the same transmit
level: the tone goes out at the OFDM frames' peak amplitude. The OFDM PSK modes peak
5.6–5.9 dB above their average (QAM 7.4), so 5.5 dB keeps the tone at or under every OFDM
frame's peak — equal peak power, which is what an ALC-limited transmitter delivers.
Every SNR the tone floor reports is taken back to the OFDM reference by this much, so the
link layer compares the two families in one currency."""


# ── modulation ─────────────────────────────────────────────────────────


def modulate(
    tones: IntArray, num: ToneNumerology = TONE_NUMEROLOGY, gain_db: float = 0.0
) -> ComplexArray:
    """Continuous-phase FSK: the frequency glides between tones over ``ramp_samples`` on a
    raised cosine, the phase accumulates, and only the frame's own fade in and out touches
    the amplitude — a constant envelope, at ``gain_db`` above unit power."""
    t = np.asarray(tones, dtype=np.int64)
    n = num.symbol_samples
    hz = num.tone_hz(t)
    freq = np.repeat(hz, n)
    ramp = num.ramp_samples
    if ramp > 0 and len(t) > 1:
        shape = 0.5 - 0.5 * np.cos(np.pi * (np.arange(ramp) + 0.5) / ramp)
        for i in np.flatnonzero(np.diff(t)) + 1:
            start = i * n - ramp // 2
            freq[start : start + ramp] = hz[i - 1] + (hz[i] - hz[i - 1]) * shape
    # the phase at the *start* of each sample, so the first sample has phase zero
    phase = (2.0 * np.pi / num.fs) * np.concatenate(([0.0], np.cumsum(freq[:-1])))
    x = np.exp(1j * phase) * 10.0 ** (gain_db / 20.0)
    edge = num.edge_samples
    if edge > 0:
        win = 0.5 - 0.5 * np.cos(np.pi * (np.arange(edge) + 0.5) / edge)
        x[:edge] *= win
        x[-edge:] *= win[::-1]
    return np.asarray(x, dtype=np.complex128)


def frame_tones(kind: ToneKind, data: IntArray, rv: int = 0) -> IntArray:
    """The whole frame's tones: the sync blocks with the data between them."""
    out = kind.layout(rv)
    out[out < 0] = data
    return out


def burst(kind: ToneKind, payload: bytes, rv: int = 0) -> ComplexArray:
    """A frame as the transmitter sends it: at :data:`TONE_GAIN_DB` above an OFDM frame."""
    tones = frame_tones(kind, tone_codec(kind).encode(payload, rv), rv)
    return modulate(tones, kind.num, TONE_GAIN_DB)


# ── demodulation ───────────────────────────────────────────────────────


def tone_energies(
    x: ComplexArray, start: int, cfo_hz: float, symbols: int, num: ToneNumerology = TONE_NUMEROLOGY
) -> FloatArray:
    """Energy of every tone in every symbol of a frame starting at ``start`` with carrier
    offset ``cfo_hz`` (symbols × tones); white noise of unit power per sample gives one."""
    n = num.symbol_samples
    if start < 0 or start + symbols * n > len(x):
        raise ValueError("the frame runs past the buffer")
    seg = np.asarray(x[start : start + symbols * n], dtype=np.complex128)
    t = np.arange(n) / num.fs
    ref = np.exp(-2j * np.pi * (num.tone_hz(np.arange(num.tones))[:, None] + cfo_hz) * t)
    return np.asarray(np.abs(seg.reshape(symbols, n) @ ref.T) ** 2 / n, dtype=np.float64)


def noise_level(energies: FloatArray, layout: IntArray) -> float:
    """The noise energy per bin: the median of the bins that should hold only noise — every
    tone but the sync tone in a sync symbol, every tone but the strongest in a data symbol —
    over ln 2, the median of an exponential."""
    e = np.array(energies, dtype=np.float64)
    known = layout >= 0
    rows = np.arange(len(e))
    top = np.where(known, layout, e.argmax(axis=1))
    mask = np.ones_like(e, dtype=bool)
    mask[rows, top] = False
    return max(float(np.median(e[mask])) / math.log(2.0), 1e-30)


def symbol_metrics(energies: FloatArray, layout: IntArray, noise: float) -> FloatArray:
    """Log-likelihood of each tone in each data symbol (data symbols × tones), up to a
    per-symbol constant: ``log I0(2·sqrt(E·s)/σ)`` — the non-coherent detector's own metric,
    which trusts a strong tone by its amplitude rather than its energy — with ``s`` the
    symbol SNR the sync blocks measure, interpolated between the three blocks so a slow fade
    sets the weight of the symbols it covers."""
    e = energies / noise
    known = layout >= 0
    idx = np.flatnonzero(known)
    held = np.clip(e[idx, layout[idx]] - 1.0, 0.0, None)
    # one estimate per sync block, at the block's middle; linear between, flat outside
    blocks = idx.reshape(-1, SYNC_SYMBOLS)
    centres = blocks.mean(axis=1)
    levels = held.reshape(-1, SYNC_SYMBOLS).mean(axis=1)
    data = np.flatnonzero(~known)
    s = np.maximum(np.interp(data, centres, levels), 0.1)
    arg = 2.0 * np.sqrt(e[data] * s[:, None])
    big = np.maximum(arg, 3.0)
    return np.asarray(
        np.where(
            arg > 3.0,
            big - 0.5 * np.log(2.0 * np.pi * big) + 1.0 / (8.0 * big),
            np.log(np.i0(np.minimum(arg, 3.0))),
        ),
        dtype=np.float64,
    )


def bit_llrs(metrics: FloatArray, bits: int) -> FloatArray:
    """Soft bits (positive = 0) from the per-tone metrics: log-sum over the tones labelled 0
    less the log-sum over those labelled 1, bit by bit, in transmission order."""
    labels = gray_labels(bits)
    out = np.empty((metrics.shape[0], bits))
    for b in range(bits):
        zero = labels[:, b] == 0
        out[:, b] = _logsumexp(metrics[:, zero]) - _logsumexp(metrics[:, ~zero])
    return out.reshape(-1)


def _logsumexp(a: FloatArray) -> FloatArray:
    top = a.max(axis=1)
    return np.asarray(top + np.log(np.exp(a - top[:, None]).sum(axis=1)), dtype=np.float64)


@dataclass(frozen=True)
class ToneFrame:
    """A detected tone frame's soft information, ready to decode or to combine."""

    kind: ToneKind
    rv: int
    llr: FloatArray
    snr_db: float
    """SNR (3 kHz) in the OFDM reference: what this path gives an ordinary frame."""

    def decode(self, buffer: FloatArray | None = None) -> tuple[bytes | None, FloatArray]:
        return tone_codec(self.kind).decode(self.llr, self.rv, buffer)


def demodulate(x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo_hz: float) -> ToneFrame:
    """Soft bits and SNR of a frame of ``kind`` at a known start and carrier offset."""
    layout = kind.layout(rv)
    energies = tone_energies(x, start, cfo_hz, kind.symbols, kind.num)
    noise = noise_level(energies, layout)
    llr = bit_llrs(symbol_metrics(energies, layout, noise), kind.num.bits_per_symbol)
    idx = np.flatnonzero(layout >= 0)
    es = float(np.mean(energies[idx, layout[idx]])) / noise - 1.0
    return ToneFrame(kind, rv, llr, snr_from_es(es, kind.num))


def snr_from_es(es_over_n0: float, num: ToneNumerology = TONE_NUMEROLOGY) -> float:
    """SNR (3 kHz, OFDM reference) from a measured symbol energy over noise density."""
    return 10.0 * math.log10(max(es_over_n0, 1e-3) / num.snr_scale()) - TONE_GAIN_DB


# ── acquisition ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class ToneSync:
    """A tone frame found in a buffer."""

    start: int
    cfo_hz: float
    kind: ToneKind
    rv: int
    statistic: float
    """Mean normalised energy of the 24 sync tones: 1 on noise, 1 + Es/N0 on a frame."""


class ToneDetector:
    """Finds tone frames: a spectrogram at a quarter-symbol hop with bins a quarter of the
    tone spacing, and for every start, carrier-offset bin and pattern the mean of the 24 sync
    bins' energies. The best hypothesis above the threshold is refined to a sixteenth of a
    symbol and a fraction of a hertz by direct correlation.

    What is summed is not the sync tone's energy but its *ratio* to the mean of the other
    fifteen tones in the same symbol (as the published FT8 decoders weigh their Costas
    arrays): noise gives about one, a frame ``1 + Es/N0``. A wideband burst — an OFDM frame,
    a static crash — lifts every tone together and leaves the ratio at one; a strong carrier
    or another narrowband signal on one of our tones scores in the few symbols whose sync tone
    it happens to sit on and costs every other symbol its ratio, and :attr:`CLIP` keeps those
    few from carrying the rest."""

    HOP_DIV = 4
    BIN_DIV = 4
    CLIP = 10.0
    MIN_HITS = 12
    MIN_FIRST_HITS = 5

    def __init__(
        self,
        kinds: tuple[ToneKind, ...] = (TONE_CONTROL, *TONE_DATA),
        max_cfo_hz: float = 100.0,
        threshold: float = 3.0,
    ) -> None:
        self.kinds = kinds
        self.num = kinds[0].num
        if any(k.num != self.num for k in kinds):
            raise ValueError("one numerology per detector")
        self.hop = self.num.symbol_samples // self.HOP_DIV
        self.nfft = self.num.symbol_samples * self.BIN_DIV
        self.bin_hz = self.num.fs / self.nfft
        self.cfo_bins = round(max_cfo_hz / self.bin_hz)
        self.threshold = threshold
        # tone k sits at bin (k − 7.5)·BIN_DIV = BIN_DIV·k − 30
        centre = (self.num.tones - 1) * self.BIN_DIV // 2
        self._tone_bin = np.arange(self.num.tones) * self.BIN_DIV - centre
        self._lo = int(self._tone_bin[0]) - self.cfo_bins
        self._hi = int(self._tone_bin[-1]) + self.cfo_bins

    @cached_property
    def hypotheses(self) -> list[tuple[ToneKind, int]]:
        return [(k, rv) for k in self.kinds for rv in range(len(k.patterns))]

    def spectrogram(self, x: ComplexArray) -> FloatArray:
        """``(hops, bins)``: energy at every quarter symbol of every bin from the lowest
        tone less the offset search to the highest plus it, unit for unit noise power."""
        n = self.num.symbol_samples
        hops = max(0, (len(x) - n) // self.hop + 1)
        if hops == 0:
            return np.zeros((0, self._hi - self._lo + 1))
        frames = np.lib.stride_tricks.sliding_window_view(np.asarray(x, np.complex128), n)[
            :: self.hop
        ][:hops]
        spec = np.fft.fft(frames, self.nfft, axis=1)
        cols = np.arange(self._lo, self._hi + 1) % self.nfft
        return np.asarray(np.abs(spec[:, cols]) ** 2 / n, dtype=np.float64)

    def ratios(self, spec: FloatArray) -> FloatArray:
        """``(hops, tones, offsets)``: every tone's energy at every hop and carrier-offset bin
        over the mean of the other fifteen tones there, clipped at :attr:`CLIP`."""
        width = 2 * self.cfo_bins + 1
        cols = (self._tone_bin - self.cfo_bins - self._lo)[:, None] + np.arange(width)
        e = spec[:, cols]  # (hops, tones, offsets)
        others = (e.sum(axis=1, keepdims=True) - e) / (self.num.tones - 1)
        return np.asarray(np.minimum(e / np.maximum(others, 1e-30), self.CLIP), np.float64)

    def statistics(self, spec: FloatArray) -> tuple[FloatArray, IntArray, IntArray]:
        """For every start hop: the best statistic over hypotheses and offsets, the
        hypothesis and the offset bin it came from."""
        r = self.ratios(spec)
        q = self.HOP_DIV
        width = 2 * self.cfo_bins + 1
        best = np.full(len(r), -np.inf)
        best_h = np.zeros(len(r), dtype=np.int64)
        best_c = np.zeros(len(r), dtype=np.int64)
        for h, (kind, rv) in enumerate(self.hypotheses):
            size = len(r) - q * (kind.symbols - 1)
            if size <= 0:
                continue
            acc = np.zeros((size, width))
            for o in kind.block_offsets:
                for i, tone in enumerate(kind.sync(rv)):
                    row = q * (o + i)
                    acc += r[row : row + size, tone, :]
            acc /= 3 * SYNC_SYMBOLS
            c = acc.argmax(axis=1)
            v = acc[np.arange(size), c]
            better = v > best[:size]
            best[:size] = np.where(better, v, best[:size])
            best_h[:size] = np.where(better, h, best_h[:size])
            best_c[:size] = np.where(better, c - self.cfo_bins, best_c[:size])
        return best, best_h, best_c

    def detect(self, x: ComplexArray, max_frames: int = 8) -> list[ToneSync]:
        """Frames in ``x``, earliest first: peaks of the statistic above the threshold,
        strongest first, each refined and kept if it passes :meth:`confirmed`; a kept frame
        rules out every other start inside its span."""
        spec = self.spectrogram(x)
        stat, hyp, cbin = self.statistics(spec)
        found: list[ToneSync] = []
        taken = np.zeros(len(stat), dtype=bool)
        for p in np.argsort(-stat):
            if stat[p] < self.threshold or len(found) >= max_frames:
                break
            if taken[p]:
                continue
            kind, rv = self.hypotheses[int(hyp[p])]
            sync = self.refine(x, kind, rv, int(p) * self.hop, float(cbin[p]) * self.bin_hz)
            if not self.confirmed(x, sync):
                # a coincidence of data symbols with a pattern: rule out its neighbourhood
                taken[max(0, p - self.HOP_DIV) : p + self.HOP_DIV + 1] = True
                continue
            # nothing else starts inside it; a frame right after it may start a hop or two
            # before its nominal end, where the peak's rounding puts it
            span = self.HOP_DIV * kind.symbols - self.HOP_DIV // 2
            taken[max(0, p - span + 1) : min(len(stat), p + span)] = True
            found.append(sync)
        return sorted(found, key=lambda s: s.start)

    def sync_hits(self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float) -> int:
        """Sync symbols of a frame placed at ``start``/``cfo`` whose own tone is the strongest
        of the sixteen."""
        layout = kind.layout(rv)
        idx = np.flatnonzero(layout >= 0)
        e = tone_energies(x, start, cfo, kind.symbols, kind.num)[idx]
        return int(np.sum(e.argmax(axis=1) == layout[idx]))

    def confirmed(self, x: ComplexArray, sync: ToneSync) -> bool:
        """Whether a refined candidate is a frame: at least :attr:`MIN_HITS` of its 24 sync
        symbols have their own tone strongest. Inside a strong frame the data symbols line
        up with some pattern at some offset on a handful of positions — about seven of 24,
        each clipped at the top — which can lift the statistic over the threshold; they do
        not make the pattern's tones the strongest at half the positions, which a frame at
        its decode threshold does 999 times in 1000."""
        return self.sync_hits(x, sync.kind, sync.rv, sync.start, sync.cfo_hz) >= self.MIN_HITS

    def sync_statistic(
        self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float
    ) -> float:
        """Mean normalised energy of the sync tones of a frame placed at ``start``/``cfo``."""
        layout = kind.layout(rv)
        e = tone_energies(x, start, cfo, kind.symbols, kind.num)
        idx = np.flatnonzero(layout >= 0)
        return float(np.mean(e[idx, layout[idx]])) / noise_level(e, layout)

    def sync_energies(
        self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfos: FloatArray
    ) -> FloatArray:
        """Mean energy of the 24 sync tones alone at ``start`` for each offset in ``cfos`` —
        unclipped and un-normalised, which is what a refinement at one noise level
        compares; ``-inf`` where the frame would leave the buffer."""
        cfos = np.atleast_1d(np.asarray(cfos, dtype=np.float64))
        if start < 0 or start + kind.samples > len(x):
            return np.full(len(cfos), -np.inf)
        n = self.num.symbol_samples
        layout = kind.layout(rv)
        idx = np.flatnonzero(layout >= 0)
        seg = np.asarray(x[start : start + kind.samples], np.complex128).reshape(kind.symbols, n)
        t = np.arange(n) / self.num.fs
        base = seg[idx] * np.exp(-2j * np.pi * self.num.tone_hz(layout[idx])[:, None] * t)
        shift = np.exp(-2j * np.pi * t[:, None] * cfos[None, :])  # (n, offsets)
        return np.asarray(np.mean(np.abs(base @ shift) ** 2, axis=0) / n, dtype=np.float64)

    def refine(self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float) -> ToneSync:
        """Timing and offset on the sync tones' own energy: a grid of two hops and two bins
        either side at half a hop and half a bin — the clipped statistic the search
        maximised is flat across a strong frame's neighbourhood, so its peak is only good to
        that — then to the sample and a fraction of a hertz. A strong frame read a few
        samples off smears the neighbouring symbol's tone over every bin, and the noise
        estimate with it."""
        half = self.hop // 2
        cfos = cfo + self.bin_hz * np.arange(-2.0, 2.01, 0.5)
        starts = [start + k * half for k in range(-4, 5)]
        grid = np.array([self.sync_energies(x, kind, rv, s, cfos) for s in starts])
        i, j = np.unravel_index(int(np.argmax(grid)), grid.shape)
        start, cfo = starts[int(i)], float(cfos[int(j)])
        starts = list(range(start - half, start + half + 1, 4))
        values = [float(self.sync_energies(x, kind, rv, s, np.array([cfo]))[0]) for s in starts]
        start = starts[int(np.argmax(values))]
        starts = list(range(start - 3, start + 4))
        values = [float(self.sync_energies(x, kind, rv, s, np.array([cfo]))[0]) for s in starts]
        start = starts[int(np.argmax(values))]
        offsets = cfo + self.bin_hz * np.linspace(-0.5, 0.5, 21)
        energy = self.sync_energies(x, kind, rv, start, offsets)
        k = int(np.argmax(energy))
        cfo = float(offsets[k])
        if 0 < k < len(offsets) - 1:
            a, b, c = energy[k - 1], energy[k], energy[k + 1]
            denom = a - 2 * b + c
            if denom < 0:
                cfo += 0.5 * (a - c) / denom * (offsets[1] - offsets[0])
        return ToneSync(start, cfo, kind, rv, self.sync_statistic(x, kind, rv, start, cfo))


# ── streaming ──────────────────────────────────────────────────────────

ANNOUNCE_THRESHOLD = 4.8
"""The first sync block's statistic (eight ratios, averaged) above which a frame is announced
as arriving. Its noise maximum is 4.0–4.6 a minute; at the slowest data kind's decode
threshold a first block reaches 4.8 five times in six, and a decibel above it every time."""


def announce_delay_s(num: ToneNumerology = TONE_NUMEROLOGY, receiver_s: float = 0.1) -> float:
    """How long after a tone frame starts a :class:`ToneStream` has announced it: its first
    sync block and the neighbourhood it must beat (:attr:`ToneStream.ANNOUNCE_LOOKAHEAD`),
    eleven symbols, plus ``receiver_s`` for the stream's own lateness — the impulse
    blanker's 57 ms, the band filter's 9 ms and a 20 ms block: 0.54 s."""
    lookahead = ToneStream.ANNOUNCE_LOOKAHEAD // ToneDetector.HOP_DIV
    return (SYNC_SYMBOLS + lookahead) * num.symbol_s + receiver_s


@dataclass(frozen=True)
class ToneArrival:
    """A tone frame whose first sync block is in and whose end is not: evidence that a frame
    of ``kind`` is arriving, ``start`` to a hop (absolute samples)."""

    start: int
    kind: ToneKind
    rv: int
    statistic: float

    @property
    def end(self) -> int:
        return self.start + self.kind.samples


class ToneStream:
    """The streaming counterpart of :meth:`ToneDetector.detect`: fed the receiver's rolling
    buffer, it keeps the spectrogram's ratio rows by absolute hop — each computed once — and
    evaluates every hypothesis at every start once, when the rows it needs are in.

    A frame is final when its statistic is the largest within :attr:`LOOKAHEAD` hops either
    side (the statistic is a symbol wide at its top), it passes
    :meth:`ToneDetector.confirmed`, and no frame already taken overlaps it — a symbol and a
    hop after its end. A frame whose *first* sync block is in, above
    :data:`ANNOUNCE_THRESHOLD` and the largest within :attr:`ANNOUNCE_LOOKAHEAD` hops, is
    announced as arriving meanwhile (:attr:`arriving`), 0.44 s after it starts: what a
    receiving station needs to hold its acknowledgement while a burst is still coming."""

    LOOKAHEAD = 4
    ANNOUNCE_LOOKAHEAD = 12
    """Hops either side a first block must beat to be announced: three symbols, because a
    block read two or three symbols early against the true one can put five of eight of
    another pattern's tones on top — only at part-symbol offsets, only on a strong frame,
    and always below the true start's eight."""

    def __init__(
        self, detector: ToneDetector | None = None, announce_threshold: float = ANNOUNCE_THRESHOLD
    ) -> None:
        self.det = detector or ToneDetector()
        self.announce_threshold = announce_threshold
        q = self.det.HOP_DIV
        self._groups: dict[int, list[int]] = {}
        """Hops from a frame's first sync hop to its last → the hypotheses of that length:
        each length is evaluated as soon as its own frames are in, so a control frame is not
        held back for a data frame's worth of rows."""
        for i, (kind, _) in enumerate(self.det.hypotheses):
            self._groups.setdefault(q * (kind.symbols - 1), []).append(i)
        self._first_hops = q * (SYNC_SYMBOLS - 1)
        self._rows: dict[int, FloatArray] = {}
        self._next_hop = 0
        self._next_start = dict.fromkeys(self._groups, 0)
        """Per length: starts before this hop have been evaluated in full."""
        self._next_first = 0
        """Starts before this hop have had their first block evaluated."""
        self._candidates: list[tuple[int, float, int, int]] = []
        self._firsts: list[tuple[int, float, int, int]] = []
        self._taken: list[tuple[int, int]] = []
        self.arriving: list[ToneArrival] = []

    def feed(self, buf: ComplexArray, abs0: int) -> list[ToneSync]:
        """Frames final in ``buf`` (whose first sample is absolute index ``abs0``), with
        absolute starts; :attr:`arriving` holds the frames announced and not yet final."""
        det = self.det
        n, hop = det.num.symbol_samples, det.hop
        end = abs0 + len(buf)
        self._next_hop = max(self._next_hop, -(-abs0 // hop))
        while self._next_hop * hop + n <= end:
            a = self._next_hop * hop - abs0
            self._rows[self._next_hop] = det.ratios(det.spectrogram(buf[a : a + n]))[0]
            self._next_hop += 1
        top = self._next_hop
        oldest = min(self._rows, default=top)
        # every hypothesis at every start whose rows are all in
        for length, hyps in self._groups.items():
            last_full = top - 1 - length
            for h in range(max(self._next_start[length], oldest), last_full + 1):
                v, i, c = self._best(h, hyps, first_only=False)
                if v >= det.threshold:
                    self._candidates.append((h, v, i, c))
            self._next_start[length] = max(self._next_start[length], last_full + 1)
        # the first blocks, for the frames arriving
        last_first = top - 1 - self._first_hops
        for h in range(max(self._next_first, oldest), last_first + 1):
            v, i, c = self._best(h, range(len(det.hypotheses)), first_only=True)
            if v >= self.announce_threshold:
                self._firsts.append((h, v, i, c))
        self._next_first = max(self._next_first, last_first + 1)
        found = self._settle(buf, abs0)
        self._announce()
        # drop the rows no start still to be evaluated needs
        need = min(
            [*self._next_start.values(), self._next_first]
            + [x[0] for x in self._candidates]
            + [x[0] for x in self._firsts]
        )
        for h in [h for h in self._rows if h < need]:
            del self._rows[h]
        return found

    def _best(
        self, h: int, hypotheses: list[int] | range, *, first_only: bool
    ) -> tuple[float, int, int]:
        """The best hypothesis at start hop ``h``: statistic, hypothesis, offset bin. A first
        block counts only if :attr:`ToneDetector.MIN_FIRST_HITS` of its eight tones are the
        strongest in their symbols (see :meth:`ToneDetector.confirmed`)."""
        q = self.det.HOP_DIV
        best = (-np.inf, 0, 0)
        for i in hypotheses:
            kind, rv = self.det.hypotheses[i]
            blocks = kind.block_offsets[:1] if first_only else kind.block_offsets
            acc = np.zeros(2 * self.det.cfo_bins + 1)
            for o in blocks:
                for j, tone in enumerate(kind.sync(rv)):
                    acc += self._rows[h + q * (o + j)][tone]
            acc /= len(blocks) * SYNC_SYMBOLS
            c = int(np.argmax(acc))
            if acc[c] <= best[0]:
                continue
            if first_only:
                hits = sum(
                    int(np.argmax(self._rows[h + q * j][:, c]) == tone)
                    for j, tone in enumerate(kind.sync(rv))
                )
                if hits < self.det.MIN_FIRST_HITS:
                    continue
            best = (float(acc[c]), i, c - self.det.cfo_bins)
        return best

    def _settle(self, buf: ComplexArray, abs0: int) -> list[ToneSync]:
        """The candidates now final, each refined, with absolute starts."""
        found: list[ToneSync] = []
        keep: list[tuple[int, float, int, int]] = []
        for cand in sorted(self._candidates):
            h, v, i, c = cand
            kind, rv = self.det.hypotheses[i]
            if h + self.LOOKAHEAD >= self._next_start[self.det.HOP_DIV * (kind.symbols - 1)]:
                keep.append(cand)  # its neighbourhood is not all evaluated yet
                continue
            start = h * self.det.hop
            near = [x[1] for x in self._candidates if abs(x[0] - h) <= self.LOOKAHEAD]
            if v < max(near) or self._overlaps((start, start + kind.samples)):
                continue
            sync = self.det.refine(buf, kind, rv, start - abs0, c * self.det.bin_hz)
            if not self.det.confirmed(buf, sync):
                continue
            sync = ToneSync(sync.start + abs0, sync.cfo_hz, kind, rv, sync.statistic)
            self._taken = [*self._taken, (sync.start, sync.start + kind.samples)][-16:]
            found.append(sync)
        self._candidates = keep
        return found

    def _announce(self) -> None:
        hop = self.det.hop
        keep: list[tuple[int, float, int, int]] = []
        for first in sorted(self._firsts):
            h, v, i, _ = first
            if h + self.ANNOUNCE_LOOKAHEAD >= self._next_first:
                keep.append(first)
                continue
            kind, rv = self.det.hypotheses[i]
            start = h * hop
            near = [x[1] for x in self._firsts if abs(x[0] - h) <= self.ANNOUNCE_LOOKAHEAD]
            if v < max(near) or self._overlaps((start, start + kind.samples)):
                continue
            if any(a.start - hop < start < a.end - hop for a in self.arriving):
                # inside a frame already arriving: its own middle or end sync block, which
                # repeats the first — never a frame of its own, in a half-duplex burst
                continue
            self.arriving.append(ToneArrival(start, kind, rv, v))
        self._firsts = keep
        # an announcement lasts until its frame is taken or its end has passed
        now = self._next_hop * hop
        self.arriving = [
            a
            for a in self.arriving
            if a.end > now - hop and not self._overlaps((a.start + hop, a.end - hop))
        ]

    def _overlaps(self, span: tuple[int, int]) -> bool:
        """Whether ``span`` overlaps a frame taken, by more than the hop or two a coarse
        start is off by — back-to-back frames of a burst touch."""
        tol = self.det.HOP_DIV // 2 * self.det.hop
        return any(span[0] < b - tol and a + tol < span[1] for a, b in self._taken)
