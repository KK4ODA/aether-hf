"""Frame layouts, the mode tables and the ladder — everything derives from
:mod:`aether_model.waveform`.

An OFDM *frame* is a preamble (two Schmidl–Cox symbols whose PN sequence encodes the frame
type) followed by ``data_symbols`` OFDM symbols, every ``pilot_symbol_period``-th of which
(starting with the first) is a full pilot symbol; in DATA frames the data carriers of the
full pilot symbols carry the mode index as PN chips. A *mode* is a (modulation, code rate)
pair; together with a layout it fixes the number of coded bits, information bits and
payload bytes per frame.

Base-graph choice follows the public 5G rule (TS 38.212 §7.2.2): BG2 for small blocks or
low rates, BG1 otherwise. One code block per frame.

Two air interfaces share this module (P7-0). The **wide** one is the 2 300 Hz waveform with
its fourteen OFDM modes; the **narrow** one is the 500 Hz waveform of ADR-0002 — twelve
carriers, the same symbol timing and the same frame layouts, so the link layer's clocks do
not change — with its own table. A 500 Hz signal puts its power into a fifth of the band,
≈ 6.8 dB more per carrier at the same 3 kHz-referenced SNR, so its control mode can be QPSK
½ where the wide table starts at BPSK ⅕ and still reach the same SNR floor; it has to be,
because with eight data carriers a control frame's seven bytes do not fit a SHORT frame at
anything slower.

Below both tables is the **tone floor** (ADR-0013): a steady-envelope sixteen-tone FSK
family, sent at the OFDM frames' peak amplitude and detected by energy, whose frames are
named by Costas-sequence sync blocks rather than a preamble and chips. Its frame
definitions — numerology, sync patterns, kinds — are here beside the OFDM layouts; the
signal processing is :mod:`aether_model.phy.tone`. What the link layer calls "mode N" is
a rung of the air's **ladder**: the tone floor's data kinds — its own two, then the air's
middle kinds, the fast ones at 2 300 Hz (ADR-0014) and the four-tone ones at 500 Hz
(ADR-0015) — then the air's OFDM modes, most robust first (:class:`Rung`,
:attr:`AirInterface.ladder`). An OFDM frame's chips carry its OFDM mode index, which is not
its rung: the wide ladder puts OFDM mode 0 at rung 6, the narrow one puts OFDM mode 2 at
rung 4 and skips the OFDM modes the floor replaced. :class:`AirInterface` bundles a
waveform with its layouts, modes and ladder; :func:`air_interface` finds the one for a
:class:`WaveformParams`.
"""

from __future__ import annotations

import itertools
import math
from dataclasses import dataclass, field
from fractions import Fraction
from functools import cached_property

import numpy as np
from numpy.typing import NDArray

from aether_model.fec.crc import CRC24A, Crc
from aether_model.fec.nr_ldpc import select_lifting_size
from aether_model.waveform import NARROW_500, WIDE_2300, Bandwidth, Modulation, WaveformParams

PREAMBLE_SYMBOLS = 2
PAYLOAD_CRC: Crc = CRC24A


@dataclass(frozen=True)
class FrameLayout:
    name: str
    data_symbols: int
    waveform: WaveformParams = WIDE_2300
    preamble_symbols: int = PREAMBLE_SYMBOLS
    """Identical Schmidl–Cox symbols ahead of the data symbols: two on every layout since
    the OFDM floor family (ADR-0009, eight) gave way to the tone floor (ADR-0013)."""
    pilot_smoothing: int = 1
    """Symbols either side over which the receiver averages its comb-pilot channel
    estimate: ±1."""

    @property
    def pilot_symbol_indices(self) -> tuple[int, ...]:
        p = self.waveform.pilot_symbol_period
        return tuple(range(0, self.data_symbols, p)) if p else ()

    @property
    def n_pilot_symbols(self) -> int:
        return len(self.pilot_symbol_indices)

    @property
    def n_payload_symbols(self) -> int:
        return self.data_symbols - self.n_pilot_symbols

    @property
    def qam_symbols(self) -> int:
        """Data-carrier slots available for coded bits in one frame."""
        return self.n_payload_symbols * self.waveform.n_data_carriers

    @property
    def total_symbols(self) -> int:
        return self.preamble_symbols + self.data_symbols

    @property
    def duration_s(self) -> float:
        return self.total_symbols * self.waveform.symbol_period_s

    @property
    def samples(self) -> int:
        return self.total_symbols * self.waveform.symbol_samples


LONG = FrameLayout("long", data_symbols=32)
"""Data frames: 34 symbols ≈ 1.05 s; 28 payload symbols × 42 carriers = 1 176 QAM symbols."""
SHORT = FrameLayout("short", data_symbols=12)
"""Control frames (ACK, connect, ping): 14 symbols ≈ 0.43 s; 10 × 42 = 420 QAM symbols →
7 payload bytes at the control mode (BPSK 1/5)."""


def select_base_graph(payload_bits: int, rate: Fraction) -> int:
    """TS 38.212 §7.2.2 base-graph selection (A = payload bits before CRC)."""
    r = float(rate)
    if payload_bits <= 292 or (payload_bits <= 3824 and r <= 0.67) or r <= 0.25:
        return 2
    return 1


@dataclass(frozen=True)
class Mode:
    index: int
    modulation: Modulation
    code_rate: Fraction

    @property
    def name(self) -> str:
        return f"{self.modulation.name}-{self.code_rate}"

    def coded_bits(self, layout: FrameLayout) -> int:
        return layout.qam_symbols * self.modulation.bits_per_symbol

    def info_bits(self, layout: FrameLayout) -> int:
        """K′ = payload + CRC, rounded down to a whole payload byte count."""
        raw = int(self.coded_bits(layout) * self.code_rate)
        payload_bytes = (raw - PAYLOAD_CRC.width) // 8
        return payload_bytes * 8 + PAYLOAD_CRC.width

    def payload_bytes(self, layout: FrameLayout) -> int:
        return (self.info_bits(layout) - PAYLOAD_CRC.width) // 8

    def base_graph(self, layout: FrameLayout) -> int:
        return select_base_graph(self.payload_bytes(layout) * 8, self.code_rate)

    def lifting_size(self, layout: FrameLayout) -> int:
        return select_lifting_size(self.base_graph(layout), self.info_bits(layout))

    def effective_rate(self, layout: FrameLayout) -> float:
        return self.info_bits(layout) / self.coded_bits(layout)

    def net_bit_rate(self, layout: FrameLayout) -> float:
        """Payload bits per second of frame air time (no ACK turnaround included)."""
        return 8 * self.payload_bytes(layout) / layout.duration_s


def _f(n: int, d: int) -> Fraction:
    return Fraction(n, d)


MODES: tuple[Mode, ...] = (
    Mode(0, Modulation.BPSK, _f(1, 5)),
    Mode(1, Modulation.BPSK, _f(1, 3)),
    Mode(2, Modulation.BPSK, _f(1, 2)),
    Mode(3, Modulation.QPSK, _f(1, 3)),
    Mode(4, Modulation.QPSK, _f(1, 2)),
    Mode(5, Modulation.QPSK, _f(2, 3)),
    Mode(6, Modulation.PSK8, _f(1, 2)),
    Mode(7, Modulation.PSK8, _f(2, 3)),
    Mode(8, Modulation.QAM16, _f(1, 2)),
    Mode(9, Modulation.QAM16, _f(2, 3)),
    Mode(10, Modulation.QAM16, _f(3, 4)),
    Mode(11, Modulation.QAM64, _f(2, 3)),
    Mode(12, Modulation.QAM64, _f(3, 4)),
    Mode(13, Modulation.QAM64, _f(5, 6)),
)
"""Ordered from most robust to fastest; the rate controller steps along this list."""

CONTROL_MODE = MODES[0]
"""The 2 300 Hz air's OFDM control frames (ACK, POLL, TURN, DISC) use the most robust mode
on the SHORT layout; connect requests, probes and beacons are DATA-container frames on LONG or
the tone floor (ADR-0016)."""


NARROW_LONG = FrameLayout("long", data_symbols=32, waveform=NARROW_500)
"""500 Hz data frames: the same 34 symbols ≈ 1.05 s; 28 payload symbols × 8 carriers =
224 QAM symbols."""
NARROW_SHORT = FrameLayout("short", data_symbols=12, waveform=NARROW_500)
"""500 Hz control frames: the same 14 symbols; 10 × 8 = 80 QAM symbols → 7 payload bytes
at the narrow control mode (QPSK ½), exactly the wide control frame's capacity."""
NARROW_MODES: tuple[Mode, ...] = (
    Mode(0, Modulation.QPSK, _f(1, 10)),
    Mode(1, Modulation.QPSK, _f(1, 5)),
    Mode(2, Modulation.QPSK, _f(1, 3)),
    Mode(3, Modulation.QPSK, _f(1, 2)),
    Mode(4, Modulation.QPSK, _f(2, 3)),
    Mode(5, Modulation.PSK8, _f(1, 2)),
    Mode(6, Modulation.PSK8, _f(2, 3)),
    Mode(7, Modulation.QAM16, _f(1, 2)),
    Mode(8, Modulation.QAM16, _f(2, 3)),
    Mode(9, Modulation.QAM16, _f(3, 4)),
    Mode(10, Modulation.QAM64, _f(2, 3)),
    Mode(11, Modulation.QAM64, _f(3, 4)),
    Mode(12, Modulation.QAM64, _f(5, 6)),
)
"""The 500 Hz OFDM mode table, most robust first. Modes 0 and 1 were the OFDM floor family's
(ADR-0009): QPSK 1/10 and ⅕ on a frame four times as long, until the tone floor (ADR-0013)
replaced them — they stay in the table only because an OFDM frame's chip sequence is indexed
by its position in it, and are on no rung of the ladder. Mode 2, QPSK ⅓, is the ladder's
first OFDM rung. Mode 3 is QPSK ½: the slowest mode whose SHORT frame carries a control
frame and whose LONG frame carries a connect request, and — with the narrow waveform's
per-carrier advantage — one that reaches about the same 3 kHz SNR as the wide table's BPSK
⅕; ordinary control frames go out at it, and a connect request's ordinary tries (beacons,
probes and a request's first try go out on the tone floor, ADR-0016). Thirteen modes is what
the 32-chip sequence set holds at |ρ| ≤ 0.25."""

NARROW_CONTROL_MODE_INDEX = 3
NARROW_CONTROL_MODE = NARROW_MODES[NARROW_CONTROL_MODE_INDEX]


# ── the tone floor (ADR-0013) ─────────────────────────────────────────


@dataclass(frozen=True)
class ToneNumerology:
    """M tones spaced at the symbol rate, centred on the passband's centre."""

    fs: float = 8000.0
    symbol_samples: int = 320
    """40 ms: twenty times ITU Poor's 2 ms delay spread, and a tone spacing of 25 Hz that is
    twenty-five times its 1 Hz Doppler spread."""
    tones: int = 16
    ramp_samples: int = 32
    """Each change of tone glides over this many samples on a raised-cosine frequency
    trajectory, which keeps the spectrum inside 500 Hz without touching the envelope."""
    edge_samples: int = 16
    """The frame fades in and out over this many samples, so keying it does not click."""

    @property
    def bits_per_symbol(self) -> int:
        return int(math.log2(self.tones))

    @property
    def spacing_hz(self) -> float:
        return self.fs / self.symbol_samples

    @property
    def symbol_s(self) -> float:
        return self.symbol_samples / self.fs

    @property
    def span_hz(self) -> float:
        """Lowest tone to highest, plus one spacing: 400 Hz."""
        return self.tones * self.spacing_hz

    def tone_hz(self, tone: int | NDArray[np.int64]) -> NDArray[np.float64]:
        return np.asarray(
            (np.asarray(tone, dtype=np.float64) - (self.tones - 1) / 2.0) * self.spacing_hz,
            dtype=np.float64,
        )

    def snr_scale(self) -> float:
        """Symbol energy over noise density per unit 3 kHz SNR: a tone of power ``P`` in
        noise whose power in 3 kHz is ``N`` puts ``P/N · 3000 · T`` into its bin."""
        return 3000.0 * self.symbol_s


TONE_NUMEROLOGY = ToneNumerology()

FAST50 = ToneNumerology(symbol_samples=160, ramp_samples=16, edge_samples=8)
"""The fast kinds' 50 Bd data (ADR-0014): the floor's sixteen tones at twice the rate and
twice the spacing — 800 Hz, the 2 300 Hz air only. Its glide keeps the floor's proportion of
a symbol; its edges are never used, because every frame starts and ends on a sync block."""
FAST100 = ToneNumerology(symbol_samples=80, ramp_samples=8, edge_samples=4)
"""The fast kinds' 100 Bd data (ADR-0014): 10 ms symbols, five times ITU Poor's delay spread,
on tones 100 Hz apart — 1 600 Hz."""
QUAD100 = ToneNumerology(symbol_samples=80, tones=4, ramp_samples=32, edge_samples=4)
"""The 500 Hz air's middle kinds' data (ADR-0015): four tones at 100 Bd, 100 Hz apart — the
floor's own 400 Hz, so the frame stays inside 500 Hz. Two bits a symbol where the floor's
sixteen tones carry four, at four times the rate: twice the floor's bits a second in the
same span. Eight tones at 50 Bd, and the floor's sixteen with a lighter code, were measured
too: on the fading classes both reach less far at the same rate. The glide is the floor's
own 32 samples, two fifths of a symbol here: against a tenth (8 samples, the fast kinds'
proportion) it puts 5 dB less power outside ±250 Hz (−30.5 dB of the frame's after the
transmit filter, where the 500 Hz OFDM frames put −15) and costs nothing measurable — 100
frames a point on AWGN and the three ITU classes, genie timing."""

SYNC_SYMBOLS = 8
SYNC_PATTERNS: tuple[tuple[int, ...], ...] = (
    (11, 5, 8, 1, 15, 2, 4, 10),
    (0, 15, 1, 13, 14, 4, 7, 12),
    (6, 15, 8, 3, 13, 4, 11, 1),
    (2, 11, 10, 15, 3, 1, 12, 7),
    (12, 3, 4, 11, 14, 13, 5, 1),
    (14, 2, 7, 13, 4, 1, 5, 0),
    (2, 0, 12, 15, 1, 9, 13, 6),
    (11, 14, 1, 15, 10, 6, 8, 0),
    (8, 1, 14, 4, 6, 15, 10, 13),
    (11, 4, 2, 3, 6, 14, 1, 10),
    (12, 9, 5, 15, 1, 0, 6, 13),
    (15, 13, 4, 3, 0, 10, 6, 14),
    (13, 3, 11, 7, 6, 8, 9, 1),
    (7, 2, 13, 11, 15, 5, 14, 3),
    (12, 7, 13, 2, 0, 14, 8, 10),
    (0, 3, 14, 1, 2, 11, 8, 7),
    (13, 8, 12, 11, 14, 5, 3, 0),
    (9, 2, 4, 12, 3, 13, 1, 15),
    (12, 8, 0, 3, 9, 14, 15, 6),
    (1, 4, 3, 13, 2, 14, 8, 12),
    (2, 7, 11, 5, 15, 10, 9, 0),
    (0, 13, 6, 7, 15, 12, 4, 9),
    (3, 9, 6, 13, 14, 10, 0, 15),
    (8, 13, 15, 14, 11, 0, 10, 2),
    (15, 0, 4, 6, 1, 14, 5, 10),
    (3, 9, 1, 14, 15, 13, 10, 0),
    (3, 14, 15, 10, 2, 1, 13, 6),
    (10, 2, 13, 15, 0, 7, 5, 14),
    (7, 6, 0, 15, 8, 14, 2, 5),
    (2, 9, 5, 0, 14, 13, 1, 10),
    (6, 13, 5, 14, 10, 4, 2, 15),
    (0, 3, 1, 5, 11, 12, 14, 4),
    (4, 13, 11, 0, 8, 15, 2, 14),
)
"""The tone floor's sync blocks: eight symbols over sixteen tones, each a *Costas sequence* —
every displacement (Δsymbol, Δtone) between two of its symbols occurs once, so a shifted
copy matches it in at most one symbol — and no two sharing more than two symbols under any
time offset within a block and any tone shift up to ±8 (±200 Hz, twice the carrier-offset
search). A bound at zero offset alone is not enough: every block of a frame repeats its
pattern at the same places, so two patterns that overlap in three symbols three symbols
apart put nine of a frame's 24 sync symbols on a hypothesis of the other kind read three
symbols early — which, with the data's coincidences on top, passed for a frame. The first
is the control frame's; then four for each data kind, one per redundancy version, in the
order of :data:`TONE_DATA`, :data:`TONE_FAST` and :data:`TONE_NARROW` — each family's are
the same search carried on (ADR-0014, ADR-0015), so the earlier ones never change. Drawn by
:func:`search_sync_patterns` and pinned: they are part of the air interface."""
PATTERN_SHIFT = 8
"""Tone shifts over which :data:`SYNC_PATTERNS` are mutually distinct."""
PATTERN_CROSS = 2
"""The most symbols two of :data:`SYNC_PATTERNS` share under any offset and shift."""


def is_costas(seq: tuple[int, ...]) -> bool:
    """Every displacement between two symbols of ``seq`` is distinct (and no tone repeats)."""
    seen: set[tuple[int, int]] = set()
    for j in range(len(seq)):
        for i in range(j):
            d = (j - i, seq[j] - seq[i])
            if d in seen:
                return False
            seen.add(d)
    return len(set(seq)) == len(seq)


def cross_hits(
    a: tuple[int, ...], b: tuple[int, ...], max_offset: int, max_shift: int = PATTERN_SHIFT
) -> int:
    """The most symbols block ``a`` shares with block ``b`` read up to ``max_offset``
    symbols earlier or later and up to ``max_shift`` tones higher or lower — how easily a
    frame of one kind passes for another."""
    n = len(a)
    return max(
        sum(1 for j in range(n) if 0 <= j - d < n and a[j] == b[j - d] + shift)
        for d in range(-max_offset, max_offset + 1)
        for shift in range(-max_shift, max_shift + 1)
    )


def search_sync_patterns(
    count: int, symbols: int = SYNC_SYMBOLS, tones: int = 16, draws: int = 5_000_000
) -> tuple[tuple[int, ...], ...]:
    """How :data:`SYNC_PATTERNS` were drawn: random selections of ``symbols`` distinct tones
    from a seeded generator, kept if Costas (:func:`is_costas`) and within
    :data:`PATTERN_CROSS` of every earlier one (:func:`cross_hits`, over every offset).

    The later patterns lie millions of draws in — the 33rd at 4.45 million — so the search
    runs a batch at a time and never one pair at a time: ``permuted`` shuffles each row of a
    batch exactly as successive ``permutation`` calls would, from the same generator, and
    both tests are vectorised. A block is Costas when no two of its displacements *the same
    distance apart* are equal: one bit per tone difference, the sum of a distance's bits
    equals their OR only if none repeats. And two Costas blocks share three symbols under
    some offset and shift — one more than :data:`PATTERN_CROSS` — exactly when three symbols
    of one, a *triangle*, are a translate of a triangle of the other with a shift within
    :data:`PATTERN_SHIFT` (any offset is within a block's own length); so every chosen block's
    56 triangles go into a table of shapes, and a candidate's are looked up in it."""
    if PATTERN_CROSS != 2:
        raise ValueError("the triangle table is the bound at two shared symbols")
    batch = 1 << 14
    rng = np.random.default_rng(20260924)
    lo, hi = np.triu_indices(symbols, 1)
    order = np.argsort(hi - lo, kind="stable")
    lo, hi = lo[order], hi[order]
    distances = np.searchsorted(hi - lo, np.arange(1, symbols))
    span = 2 * tones - 1
    vectors = (symbols - 1) * span
    a, b, c = np.array(list(itertools.combinations(range(symbols), 3)), dtype=np.int64).T

    def shapes(blocks: NDArray[np.int64]) -> NDArray[np.int64]:
        """Each triangle's two displacements from its first symbol, as one code."""
        first = (b - a - 1) * span + blocks[:, b] - blocks[:, a] + tones - 1
        second = (c - a - 1) * span + blocks[:, c] - blocks[:, a] + tones - 1
        return np.asarray(first * vectors + second, dtype=np.int64)

    # the first symbol's tone of every chosen triangle, by shape; far off where there is none
    table: dict[int, list[int]] = {}
    held = np.full((vectors * vectors, 1), -4 * tones, dtype=np.int64)

    def clear(blocks: NDArray[np.int64]) -> NDArray[np.bool_]:
        near = np.abs(blocks[:, a, None] - held[shapes(blocks)]) <= PATTERN_SHIFT
        return np.asarray(~near.any(axis=(1, 2)))

    chosen: list[tuple[int, ...]] = []
    for _ in range(0, draws, batch):
        rows = np.tile(np.arange(tones, dtype=np.int64), (batch, 1))
        cand = rng.permuted(rows, axis=1)[:, :symbols]
        bits = np.left_shift(np.int64(1), cand[:, hi] - cand[:, lo] + tones - 1)
        costas = np.add.reduceat(bits, distances, axis=1) == np.bitwise_or.reduceat(
            bits, distances, axis=1
        )
        cand = cand[costas.all(axis=1)]
        cand = cand[clear(cand)]
        while len(cand):
            new = cand[0]
            chosen.append(tuple(int(v) for v in new))
            if len(chosen) == count:
                return tuple(chosen)
            for code, tone in zip(shapes(new[None, :])[0], new[a], strict=True):
                table.setdefault(int(code), []).append(int(tone))
            width = max(map(len, table.values()))
            held = np.full((vectors * vectors, width), -4 * tones, dtype=np.int64)
            for code, held_tones in table.items():
                held[code, : len(held_tones)] = held_tones
            cand = cand[1:][clear(cand[1:])]
    raise ValueError(f"only {len(chosen)} patterns found")


@dataclass(frozen=True)
class ToneKind:
    """One kind of tone-floor frame: what it carries, how long it is, and the sync patterns
    that name it — one per redundancy version (a control frame has one).

    A frame is a row of *slots*, each one sync symbol long: three sync blocks of
    :data:`SYNC_SYMBOLS` slots and the data slots between them. The floor's own kinds send
    one data symbol a slot; a fast kind (ADR-0014) sends two or four, at its
    :attr:`data_num`, under the same sync blocks — so one detector finds every kind, and
    every data kind's frame is as long as every other's."""

    name: str
    payload_bytes: int
    data_symbols: int
    patterns: tuple[int, ...]
    """Indices into :data:`SYNC_PATTERNS`, by redundancy version."""
    control: bool = False
    num: ToneNumerology = field(default=TONE_NUMEROLOGY)
    """The sync blocks' numerology: the floor's, for every kind."""
    data_num: ToneNumerology | None = None
    """The data symbols' numerology; ``None`` is :attr:`num`'s own."""

    @property
    def data(self) -> ToneNumerology:
        """The numerology the data symbols go out at."""
        return self.num if self.data_num is None else self.data_num

    @property
    def speed(self) -> int:
        """Data symbols a slot: 1, 2 or 4."""
        return self.num.symbol_samples // self.data.symbol_samples

    @property
    def data_slots(self) -> int:
        return self.data_symbols // self.speed

    @property
    def info_bits(self) -> int:
        return self.payload_bytes * 8 + PAYLOAD_CRC.width

    @property
    def coded_bits(self) -> int:
        return self.data_symbols * self.data.bits_per_symbol

    @property
    def rate(self) -> float:
        return self.info_bits / self.coded_bits

    @property
    def base_graph(self) -> int:
        return select_base_graph(self.payload_bytes * 8, Fraction(self.rate).limit_denominator(64))

    @property
    def lifting_size(self) -> int:
        return select_lifting_size(self.base_graph, self.info_bits)

    @property
    def symbols(self) -> int:
        """The frame's length in slots (sync symbols)."""
        return self.data_slots + 3 * SYNC_SYMBOLS

    @property
    def samples(self) -> int:
        return self.symbols * self.num.symbol_samples

    @property
    def duration_s(self) -> float:
        return self.symbols * self.num.symbol_s

    @property
    def net_bps(self) -> float:
        return 8 * self.payload_bytes / self.duration_s

    @property
    def block_offsets(self) -> tuple[int, int, int]:
        """First slot of each sync block: start, middle, end. The data is split unevenly
        between them — 45 % before the middle block — so the three distances between blocks
        all differ and no shift of a frame lines up more than one of its blocks with
        another's: with an even split a frame read one block-spacing early has its middle
        and end blocks on the true frame's first and middle, sixteen of 24 sync symbols."""
        first = self.data_slots * 9 // 20
        return (0, SYNC_SYMBOLS + first, self.symbols - SYNC_SYMBOLS)

    def sync(self, rv: int = 0) -> tuple[int, ...]:
        return SYNC_PATTERNS[self.patterns[rv]]

    def layout(self, rv: int = 0) -> NDArray[np.int64]:
        """Each slot's role: ``-1`` a data slot, otherwise the sync tone."""
        out = np.full(self.symbols, -1, dtype=np.int64)
        for o in self.block_offsets:
            out[o : o + SYNC_SYMBOLS] = self.sync(rv)
        return out

    def data_segments(self) -> tuple[tuple[int, int], ...]:
        """The data's runs of slots between the sync blocks, as ``(first slot, slots)``."""
        b = self.block_offsets
        return tuple(
            (b[i] + SYNC_SYMBOLS, b[i + 1] - b[i] - SYNC_SYMBOLS) for i in range(len(b) - 1)
        )


TONE_CONTROL = ToneKind("tone-control", 7, 56, (0,), control=True)
"""Acknowledgements and the other control frames on the floor: 80 symbols, 3.2 s; seven
bytes at rate 0.36, more robust than the data it answers."""
TONE_DATA: tuple[ToneKind, ...] = (
    ToneKind("tone-24", 24, 110, (1, 2, 3, 4)),
    ToneKind("tone-36", 36, 110, (5, 6, 7, 8)),
)
"""The data kinds, slowest first, on one 134-symbol frame (5.36 s): 24 bytes at rate 0.49
(36 bit/s) — enough for a connect request — and 36 at rate 0.71 (54 bit/s)."""
TONE_FAST: tuple[ToneKind, ...] = (
    ToneKind("tone50-51", 51, 220, (9, 10, 11, 12), data_num=FAST50),
    ToneKind("tone50-75", 75, 220, (13, 14, 15, 16), data_num=FAST50),
    ToneKind("tone100-105", 105, 440, (17, 18, 19, 20), data_num=FAST100),
    ToneKind("tone100-153", 153, 440, (21, 22, 23, 24), data_num=FAST100),
)
"""The fast kinds (ADR-0014), the 2 300 Hz air's middle rungs: the floor's frame — its sync
blocks, its 5.36 s — with two or four data symbols a slot, at the floor's two code rates:
51 and 75 bytes at 50 Bd (76 and 112 bit/s), 105 and 153 at 100 Bd (157 and 228 bit/s)."""
TONE_NARROW: tuple[ToneKind, ...] = (
    ToneKind("tone4x100-51", 51, 440, (25, 26, 27, 28), data_num=QUAD100),
    ToneKind("tone4x100-75", 75, 440, (29, 30, 31, 32), data_num=QUAD100),
)
"""The 500 Hz air's middle rungs (ADR-0015): the floor's frame with four data symbols a slot
on :data:`QUAD100`'s four tones — 880 coded bits, as the 50 Bd fast kinds have, at the
floor's two code rates: 51 and 75 bytes (76 and 112 bit/s)."""


# ── the ladder ────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Rung:
    """One step of an air's ladder — what the link layer, the rate controller and the
    operator call "mode N": a tone-floor kind, or an OFDM mode on the layout it goes out on."""

    index: int
    tone: ToneKind | None = None
    mode: Mode | None = None
    layout: FrameLayout | None = None

    @property
    def floor(self) -> bool:
        return self.tone is not None

    @property
    def name(self) -> str:
        if self.tone is not None:
            return self.tone.name
        assert self.mode is not None
        return self.mode.name

    @property
    def payload_bytes(self) -> int:
        if self.tone is not None:
            return self.tone.payload_bytes
        assert self.mode is not None and self.layout is not None
        return self.mode.payload_bytes(self.layout)

    @property
    def duration_s(self) -> float:
        if self.tone is not None:
            return self.tone.duration_s
        assert self.layout is not None
        return self.layout.duration_s

    @property
    def net_bps(self) -> float:
        return 8 * self.payload_bytes / self.duration_s


@dataclass(frozen=True)
class AirInterface:
    """One waveform with the layouts, modes and ladder that go with it — what a transmitter,
    receiver, detector or link harness needs to know about the air it is on."""

    params: WaveformParams
    long: FrameLayout
    short: FrameLayout
    modes: tuple[Mode, ...]
    """The OFDM mode table: what an OFDM frame's chips index. The link runs the ladder."""
    chip_correlation_bound: float
    """Largest pairwise correlation allowed between the (mode, RV) chip sequences. The
    wide waveform has 168 chips and holds 56 sequences at 0.2; the narrow one has 32 and
    holds its 52 at 0.25 — the metric loses a little separation and keeps a 15 dB
    processing gain."""
    acquisition_threshold: float
    """The detector's normalised matched-filter peak above which a preamble is declared.
    Set just above the statistic's maximum over 60 s of band-limited noise: 0.348 at
    2 300 Hz, so 0.36; 0.549 at 500 Hz, so 0.56. The narrow waveform's noise maximum is
    higher because its band-limited noise has a fifth of the degrees of freedom in a
    preamble's span, and its signal peaks are higher by about as much (0.57–0.68 at
    −5 dB against 0.42–0.50), so the two floors land in the same place."""
    ofdm_ladder: tuple[int, ...] = ()
    """The OFDM modes on the ladder, ascending, above the tone floor's rungs."""
    control_mode_index: int = 0
    """The OFDM mode ordinary control frames and a connect request's ordinary tries go out
    at: the slowest whose SHORT frame carries a control frame — mode 0 on the wide air,
    mode 3 (QPSK ½) on the narrow one, where mode 2 (QPSK ⅓) is a data mode whose SHORT
    frame would carry three bytes."""
    tone_data: tuple[ToneKind, ...] = TONE_DATA
    """The tone floor's data kinds (ADR-0013): the ladder's first rungs."""
    tone_control: ToneKind = TONE_CONTROL
    """The control frame while the link runs the floor."""

    @property
    def control_mode(self) -> Mode:
        """The OFDM mode ordinary control frames and a connect request's ordinary tries go
        out at."""
        return self.modes[self.control_mode_index]

    @cached_property
    def ladder(self) -> tuple[Rung, ...]:
        """Every rung, most robust first: the tone floor's data kinds, then the OFDM modes
        of :attr:`ofdm_ladder` on the LONG layout."""
        tones = [Rung(i, tone=k) for i, k in enumerate(self.tone_data)]
        base = len(tones)
        ofdm = [
            Rung(base + j, mode=self.modes[m], layout=self.long)
            for j, m in enumerate(self.ofdm_ladder)
        ]
        return (*tones, *ofdm)

    @property
    def floor_modes(self) -> int:
        """How many of the ladder's leading rungs are the floor's: rung ``floor_modes`` is
        the first OFDM one."""
        return len(self.tone_data)

    @property
    def n_modes(self) -> int:
        """OFDM modes in the table — what the chip sequences are indexed by."""
        return len(self.modes)

    @property
    def n_rungs(self) -> int:
        """Rungs on the ladder — the link layer's mode count."""
        return len(self.ladder)

    @property
    def control_rung(self) -> int:
        """The rung of :attr:`control_mode`."""
        return self.rung_of(self.control_mode_index)

    @property
    def name(self) -> str:
        return self.params.bandwidth.name

    def is_floor(self, rung: int) -> bool:
        return rung < self.floor_modes

    def rung_of(self, ofdm_mode: int) -> int:
        """The rung an OFDM mode sits on (``ValueError`` if it is on none)."""
        return self.floor_modes + self.ofdm_ladder.index(ofdm_mode)

    def data_layout(self, rung: int) -> FrameLayout:
        """The OFDM layout a DATA frame at an OFDM rung goes out on."""
        layout = self.ladder[rung].layout
        if layout is None:
            raise ValueError(f"rung {rung} is the tone floor's; it has no OFDM layout")
        return layout

    def layout_for(self, data: bool) -> FrameLayout:
        return self.long if data else self.short

    @property
    def layouts(self) -> tuple[FrameLayout, ...]:
        """Every OFDM layout of this air."""
        return (self.long, self.short)


WIDE = AirInterface(
    WIDE_2300,
    LONG,
    SHORT,
    MODES,
    0.2,
    0.36,
    ofdm_ladder=tuple(range(len(MODES))),
    tone_data=(*TONE_DATA, *TONE_FAST),
)
NARROW = AirInterface(
    NARROW_500,
    NARROW_LONG,
    NARROW_SHORT,
    NARROW_MODES,
    0.25,
    0.56,
    ofdm_ladder=tuple(range(2, len(NARROW_MODES))),
    control_mode_index=NARROW_CONTROL_MODE_INDEX,
    tone_data=(*TONE_DATA, *TONE_NARROW),
)

AIR_INTERFACES: dict[Bandwidth, AirInterface] = {
    Bandwidth.WIDE_2300: WIDE,
    Bandwidth.NARROW_500: NARROW,
}


def air_interface(params: WaveformParams) -> AirInterface:
    """The air interface a waveform belongs to.

    Keyed by bandwidth: the numerology of a bandwidth is fixed by ADR-0002, and a
    :class:`WaveformParams` with the same bandwidth and different numbers is not a
    waveform this modem has (2 750 Hz is P9-3, not yet)."""
    try:
        air = AIR_INTERFACES[params.bandwidth]
    except KeyError as e:
        raise NotImplementedError(f"no air interface for {params.bandwidth.name}") from e
    if air.params != params:
        raise ValueError(f"{params.bandwidth.name} numerology differs from the air interface's")
    return air


def mode_table(
    layout: FrameLayout = LONG,
    modes: tuple[Mode, ...] | None = None,
    air: AirInterface | None = None,
) -> list[dict[str, float | int | str]]:
    """Human-readable summary, e.g. for docs/spec and the GUI. With ``air`` every rung of
    its ladder is tabulated — the tone floor's kinds and the OFDM modes on their layout;
    otherwise the OFDM ``modes`` (default: the layout's air's) on ``layout``."""
    if air is not None:
        rows: list[dict[str, float | int | str]] = []
        for r in air.ladder:
            if r.tone is not None:
                k = r.tone
                rows.append(
                    {
                        "mode": r.index,
                        "name": k.name,
                        "layout": f"tone, {k.symbols} symbols",
                        "bits_per_symbol": k.data.bits_per_symbol,
                        "code_rate": f"{k.rate:.2f}",
                        "coded_bits": k.coded_bits,
                        "payload_bytes": k.payload_bytes,
                        "base_graph": k.base_graph,
                        "z": k.lifting_size,
                        "net_bps": round(k.net_bps),
                    }
                )
                continue
            assert r.mode is not None and r.layout is not None
            rows.append(_ofdm_row(r.index, r.mode, r.layout))
        return rows
    table = modes if modes is not None else air_interface(layout.waveform).modes
    return [_ofdm_row(m.index, m, layout) for m in table]


def _ofdm_row(index: int, m: Mode, lay: FrameLayout) -> dict[str, float | int | str]:
    return {
        "mode": index,
        "name": m.name,
        "layout": lay.name,
        "bits_per_symbol": m.modulation.bits_per_symbol,
        "code_rate": str(m.code_rate),
        "coded_bits": m.coded_bits(lay),
        "payload_bytes": m.payload_bytes(lay),
        "base_graph": m.base_graph(lay),
        "z": m.lifting_size(lay),
        "net_bps": round(m.net_bit_rate(lay)),
    }
