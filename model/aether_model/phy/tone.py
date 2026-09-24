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

The **fast kinds** (P9-9, ADR-0014) fill the 2 300 Hz table's gap between the floor and the
OFDM modes with the same frame: its sync blocks, its length, its code — and two or four data
symbols a slot, at 50 or 100 Bd on tones 50 or 100 Hz apart, so the data spreads over 800 or
1 600 Hz and the same detector finds every kind, far below where any of them decodes.
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
    TONE_FAST,
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
    """Continuous-phase FSK at one numerology (:func:`cpfsk`)."""
    t = np.asarray(tones, dtype=np.int64)
    return cpfsk(
        num.tone_hz(t),
        np.full(len(t), num.symbol_samples),
        np.full(len(t), num.ramp_samples),
        num.fs,
        edge=num.edge_samples,
        gain_db=gain_db,
    )


def cpfsk(
    hz: FloatArray,
    lengths: IntArray,
    ramps: IntArray,
    fs: float,
    *,
    edge: int,
    gain_db: float = 0.0,
) -> ComplexArray:
    """Continuous-phase FSK: symbol ``i`` holds ``hz[i]`` for ``lengths[i]`` samples, the
    frequency glides into it over ``ramps[i]`` samples on a raised cosine centred on its
    first sample, the phase accumulates, and only the frame's own fade in and out over
    ``edge`` samples touches the amplitude — a constant envelope, at ``gain_db`` above unit
    power."""
    hz = np.asarray(hz, dtype=np.float64)
    lengths = np.asarray(lengths, dtype=np.int64)
    starts = np.concatenate(([0], np.cumsum(lengths)[:-1]))
    freq = np.repeat(hz, lengths)
    for i in np.flatnonzero(np.diff(hz)) + 1:
        ramp = int(ramps[i])
        if ramp <= 0:
            continue
        shape = 0.5 - 0.5 * np.cos(np.pi * (np.arange(ramp) + 0.5) / ramp)
        start = int(starts[i]) - ramp // 2
        freq[start : start + ramp] = hz[i - 1] + (hz[i] - hz[i - 1]) * shape
    # the phase at the *start* of each sample, so the first sample has phase zero
    phase = (2.0 * np.pi / fs) * np.concatenate(([0.0], np.cumsum(freq[:-1])))
    x = np.exp(1j * phase) * 10.0 ** (gain_db / 20.0)
    if edge > 0:
        win = 0.5 - 0.5 * np.cos(np.pi * (np.arange(edge) + 0.5) / edge)
        x[:edge] *= win
        x[-edge:] *= win[::-1]
    return np.asarray(x, dtype=np.complex128)


def frame_symbols(
    kind: ToneKind, data: IntArray, rv: int = 0
) -> tuple[IntArray, NDArray[np.bool_]]:
    """The whole frame's tones in time order, and which of them are sync symbols: a sync
    slot is one symbol at the sync numerology, a data slot :attr:`ToneKind.speed` symbols at
    the data's."""
    layout = kind.layout(rv)
    sync = layout >= 0
    is_sync = np.repeat(sync, np.where(sync, 1, kind.speed))
    tones = np.empty(len(is_sync), dtype=np.int64)
    tones[is_sync] = layout[sync]
    tones[~is_sync] = data
    return tones, is_sync


def frame_tones(kind: ToneKind, data: IntArray, rv: int = 0) -> IntArray:
    """The whole frame's tones: the sync blocks with the data between them."""
    return frame_symbols(kind, data, rv)[0]


def modulate_frame(
    kind: ToneKind, tones: IntArray, is_sync: NDArray[np.bool_], gain_db: float = 0.0
) -> ComplexArray:
    """A frame's tones on the air: each at its own numerology, a glide between two symbols
    as short as the shorter of their own, the frame's edges the sync numerology's."""
    s, d = kind.num, kind.data
    hz = np.where(is_sync, s.tone_hz(tones), d.tone_hz(tones))
    lengths = np.where(is_sync, s.symbol_samples, d.symbol_samples)
    own = np.where(is_sync, s.ramp_samples, d.ramp_samples)
    ramps = np.minimum(own, np.concatenate((own[:1], own[:-1])))
    return cpfsk(hz, lengths, ramps, s.fs, edge=s.edge_samples, gain_db=gain_db)


def burst(kind: ToneKind, payload: bytes, rv: int = 0) -> ComplexArray:
    """A frame as the transmitter sends it: at :data:`TONE_GAIN_DB` above an OFDM frame."""
    tones, is_sync = frame_symbols(kind, tone_codec(kind).encode(payload, rv), rv)
    return modulate_frame(kind, tones, is_sync, TONE_GAIN_DB)


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
    over ln 2, the median of an exponential. A silent symbol (the receiver muted while its
    station transmitted) measures nothing and is left out: counted, a frame half under the
    station's own transmission read its noise as zero and its SNR as 290 dB."""
    e = np.array(energies, dtype=np.float64)
    known = layout >= 0
    rows = np.arange(len(e))
    top = np.where(known, layout, e.argmax(axis=1))
    mask = np.ones_like(e, dtype=bool)
    mask[rows, top] = False
    mask[e.max(axis=1) <= 0] = False
    if not mask.any():
        return 1e-30
    return max(float(np.median(e[mask])) / math.log(2.0), 1e-30)


def symbol_metrics(energies: FloatArray, layout: IntArray, noise: float) -> FloatArray:
    """Log-likelihood of each tone in each data symbol (data symbols × tones), up to a
    per-symbol constant: ``log I0(2·sqrt(E·s)/σ)`` — the non-coherent detector's own metric,
    which trusts a strong tone by its amplitude rather than its energy — with ``s`` the
    symbol SNR the sync blocks measure, interpolated between the three blocks so a slow fade
    sets the weight of the symbols it covers."""
    e = energies / noise
    known = layout >= 0
    centres, levels = block_levels(e, layout)
    data = np.flatnonzero(~known)
    s = np.maximum(np.interp(data, centres, levels), 0.1)
    return _log_i0(2.0 * np.sqrt(e[data] * s[:, None]))


def block_levels(e: FloatArray, layout: IntArray) -> tuple[FloatArray, FloatArray]:
    """The symbol SNR each sync block measures (its sync tones' normalised energy less the
    noise's one), at the block's middle slot — one estimate per block, from slot energies
    already divided by the noise."""
    idx = np.flatnonzero(layout >= 0)
    held = np.clip(e[idx, layout[idx]] - 1.0, 0.0, None)
    centres = idx.reshape(-1, SYNC_SYMBOLS).mean(axis=1)
    return centres, held.reshape(-1, SYNC_SYMBOLS).mean(axis=1)


def fast_metrics(
    kind: ToneKind, energies: FloatArray, layout: IntArray, noise: float, data: FloatArray
) -> FloatArray:
    """:func:`symbol_metrics` for a fast kind (ADR-0014): the sync blocks' levels come from
    the slot energies at the sync numerology, the data symbols' energies at their own. A
    data symbol is ``speed`` times shorter than a slot and carries that much less energy, so
    the level interpolated at its middle is scaled down by the same factor; each family is
    divided by its own noise, measured on its own bins."""
    centres, levels = block_levels(energies / noise, layout)
    speed = kind.speed
    slots = np.flatnonzero(layout < 0)
    at = np.repeat(slots, speed) + (np.tile(np.arange(speed), len(slots)) + 0.5) / speed - 0.5
    s = np.maximum(np.interp(at, centres, levels) / speed, 0.1)
    e = data / noise_level(data, np.full(len(data), -1, dtype=np.int64))
    return _log_i0(2.0 * np.sqrt(e * s[:, None]))


def _log_i0(arg: FloatArray) -> FloatArray:
    """``log I0``, exact below 3 and by its asymptotic series above."""
    big = np.maximum(arg, 3.0)
    return np.asarray(
        np.where(
            arg > 3.0,
            big - 0.5 * np.log(2.0 * np.pi * big) + 1.0 / (8.0 * big),
            np.log(np.i0(np.minimum(arg, 3.0))),
        ),
        dtype=np.float64,
    )


def data_energies(x: ComplexArray, kind: ToneKind, start: int, cfo_hz: float) -> FloatArray:
    """Energy of every tone in every data symbol of a fast kind's frame, at the data's
    numerology (data symbols × tones), segment by segment between the sync blocks."""
    slot = kind.num.symbol_samples
    return np.concatenate(
        [
            tone_energies(x, start + first * slot, cfo_hz, slots * kind.speed, kind.data)
            for first, slots in kind.data_segments()
        ]
    )


def sync_noise(energies: FloatArray, layout: IntArray, kind: ToneKind) -> float:
    """The noise per bin at the sync numerology: :func:`noise_level` over the whole frame for
    the floor's own kinds, over the sync slots alone for a fast one — whose data slots, read
    at a sync symbol's length, hold its data's energy smeared across the bins."""
    if kind.speed == 1:
        return noise_level(energies, layout)
    known = layout >= 0
    return noise_level(energies[known], layout[known])


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
    noise = sync_noise(energies, layout, kind)
    if kind.speed == 1:
        metrics = symbol_metrics(energies, layout, noise)
    else:
        data = data_energies(x, kind, start, cfo_hz)
        metrics = fast_metrics(kind, energies, layout, noise, data)
    llr = bit_llrs(metrics, kind.data.bits_per_symbol)
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
    MIN_BLOCK_HITS = 4
    """Hits the second-best sync block of a confirmed frame must have: evidence in two of the
    three blocks, which is what three blocks are for — a fade may swallow one. A hypothesis
    read a block-spacing early against a strong frame has one block on the frame's own and the
    others over its data or silence; with eight hits there and the data's coincidences it
    reached twelve, and a receiver taking frames as they end took it first — found by two
    daemons over ``[sim]``, where it was the silence a station keeps while it transmits."""
    MIN_FIRST_HITS = 5

    def __init__(
        self,
        kinds: tuple[ToneKind, ...] = (TONE_CONTROL, *TONE_DATA, *TONE_FAST),
        max_cfo_hz: float = 100.0,
        threshold: float = 3.0,
    ) -> None:
        self.kinds = kinds
        self.num = kinds[0].num
        if any(k.num != self.num for k in kinds):
            raise ValueError("one sync numerology per detector")
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

    def block_hits(
        self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float
    ) -> list[int]:
        """Per sync block, the symbols of a frame placed at ``start``/``cfo`` whose own tone
        is the strongest of the sixteen. A symbol with no energy at all is no evidence: a
        receiver hears exact silence only while it is muted — its own transmission — and
        there every tone ties."""
        layout = kind.layout(rv)
        e = tone_energies(x, start, cfo, kind.symbols, kind.num)
        out = []
        for o in kind.block_offsets:
            rows = e[o : o + SYNC_SYMBOLS]
            hit = (rows.argmax(axis=1) == layout[o : o + SYNC_SYMBOLS]) & (rows.max(axis=1) > 0)
            out.append(int(np.sum(hit)))
        return out

    def sync_hits(self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float) -> int:
        """Sync symbols of a frame placed at ``start``/``cfo`` whose own tone is the strongest
        of the sixteen (:meth:`block_hits`, summed)."""
        return sum(self.block_hits(x, kind, rv, start, cfo))

    def confirmed(self, x: ComplexArray, sync: ToneSync) -> bool:
        """Whether a refined candidate is a frame: at least :attr:`MIN_HITS` of its 24 sync
        symbols have their own tone strongest, and at least :attr:`MIN_BLOCK_HITS` of them in
        a second block. Inside a strong frame the data symbols line up with some pattern at
        some offset on a handful of positions — about seven of 24, each clipped at the top —
        which can lift the statistic over the threshold; they do not make the pattern's tones
        the strongest at half the positions, which a frame at its decode threshold does 999
        times in 1000. A hypothesis a block-spacing off a strong frame has eight hits in one
        block and chance in the others, which the second condition refuses."""
        hits = sorted(self.block_hits(x, sync.kind, sync.rv, sync.start, sync.cfo_hz))
        return sum(hits) >= self.MIN_HITS and hits[-2] >= self.MIN_BLOCK_HITS

    def sync_statistic(
        self, x: ComplexArray, kind: ToneKind, rv: int, start: int, cfo: float
    ) -> float:
        """Mean normalised energy of the sync tones of a frame placed at ``start``/``cfo``."""
        layout = kind.layout(rv)
        e = tone_energies(x, start, cfo, kind.symbols, kind.num)
        idx = np.flatnonzero(layout >= 0)
        return float(np.mean(e[idx, layout[idx]])) / sync_noise(e, layout, kind)

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
    receiving station needs to hold its acknowledgement while a burst is still coming.

    Two frames of a half-duplex burst never overlap, and two rules act on it (ADR-0014). A
    first block inside a frame already arriving is that frame's own middle or end block and
    is not announced — unless it is somewhere else, and stronger: then the arrival it falls
    in was the false one, and it gives way. And a candidate is not taken while a frame
    announced as arriving starts inside it with a first block as strong as the candidate's
    whole: that is the hypothesis read a block-spacing early, its middle block on the
    frame's first and its first in the silence before the burst, which a fast kind's data
    under its end block can carry past :meth:`ToneDetector.confirmed`."""

    LOOKAHEAD = 4
    ANNOUNCE_LOOKAHEAD = 12
    """Hops either side a first block must beat to be announced: three symbols, because a
    block read two or three symbols early against the true one can put five of eight of
    another pattern's tones on top — only at part-symbol offsets, only on a strong frame,
    and always below the true start's eight. A hypothesis further off that still shares a
    symbol or two with the true block and passes on noise is announced, and gives way to the
    true block when that is announced inside it."""

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
                # a silent symbol (the receiver muted) ties every tone and is no evidence
                hits = sum(
                    int(
                        np.argmax(self._rows[h + q * j][:, c]) == tone
                        and self._rows[h + q * j][tone, c] > 0
                    )
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
            if self._announced_inside(start, start + kind.samples, v):
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
            inside = [a for a in self.arriving if a.start - hop < start < a.end - hop]
            if any(self._own_block(a, start) or v <= a.statistic for a in inside):
                # inside a frame already arriving: its own middle or end sync block, which
                # repeats the first, or a weaker reading than it — never a frame of its own,
                # in a half-duplex burst
                continue
            # stronger, and not one of its blocks: the arrival it falls in was the false one
            self.arriving = [a for a in self.arriving if a not in inside]
            self.arriving.append(ToneArrival(start, kind, rv, v))
        self._firsts = keep
        # an announcement lasts until its frame is taken or its end has passed
        now = self._next_hop * hop
        self.arriving = [
            a
            for a in self.arriving
            if a.end > now - hop and not self._overlaps((a.start + hop, a.end - hop))
        ]

    def _own_block(self, arrival: ToneArrival, start: int) -> bool:
        """Whether a first block at ``start`` is ``arrival``'s own middle or end block, to a
        symbol."""
        n = self.det.num.symbol_samples
        return any(
            abs(start - (arrival.start + o * n)) <= n for o in arrival.kind.block_offsets[1:]
        )

    def _announced_inside(self, start: int, end: int, statistic: float) -> bool:
        """Whether a frame announced as arriving starts inside ``start``–``end`` — more than a
        symbol after its start — with a first block at least ``statistic``."""
        n = self.det.num.symbol_samples
        return any(
            start + n < a.start < end - n and a.statistic >= statistic for a in self.arriving
        )

    def _overlaps(self, span: tuple[int, int]) -> bool:
        """Whether ``span`` overlaps a frame taken, by more than the hop or two a coarse
        start is off by — back-to-back frames of a burst touch."""
        tol = self.det.HOP_DIV // 2 * self.det.hop
        return any(span[0] < b - tol and a + tol < span[1] for a, b in self._taken)
