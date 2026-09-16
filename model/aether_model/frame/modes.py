"""Frame layouts and the mode table — everything derives from :mod:`aether_model.waveform`.

A *frame* is a preamble (two Schmidl–Cox symbols whose PN sequence encodes the frame
type) followed by ``data_symbols`` OFDM symbols, every ``pilot_symbol_period``-th of which
(starting with the first) is a full pilot symbol; in DATA frames the data carriers of the
full pilot symbols carry the mode index as PN chips. A *mode* is a (modulation, code rate)
pair; together with a layout it fixes the number of coded bits, information bits and
payload bytes per frame.

Base-graph choice follows the public 5G rule (TS 38.212 §7.2.2): BG2 for small blocks or
low rates, BG1 otherwise. One code block per frame.

Two air interfaces share this module (P7-0). The **wide** one is the 2 300 Hz waveform with
its fourteen modes; the **narrow** one is the 500 Hz waveform of ADR-0002 — twelve carriers,
the same symbol timing and the same frame layouts, so the link layer's clocks do not
change — with its own thirteen-mode table. A 500 Hz signal puts its power into a fifth of
the band, ≈ 6.8 dB more per carrier at the same 3 kHz-referenced SNR, so its control mode
can be QPSK ½ where the wide table starts at BPSK ⅕ and still reach the same SNR floor;
it has to be, because with eight data carriers a control frame's seven bytes do not fit a
SHORT frame at anything slower. Below it the narrow air has a **floor family**
(ADR-0009): two more layouts with an eight-symbol preamble and two or four times the data
symbols, on which a tenth-rate and a fifth-rate QPSK mode carry a few bytes at −12 and
−10 dB, and a control frame at the same SNR as the data. A frame's family is told from
the length of its preamble, so the ordinary frames are untouched. :class:`AirInterface`
bundles a waveform with its layouts and modes; :func:`air_interface` finds the one for a
:class:`WaveformParams`.
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction

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
    """Identical Schmidl–Cox symbols ahead of the data symbols: two on the ordinary
    layouts; eight on the floor layouts (ADR-0009), which is what lets the detector find
    them 6 dB deeper and is also how a receiver tells the two families apart."""
    pilot_smoothing: int = 1
    """Symbols either side over which the receiver averages its comb-pilot channel
    estimate: ±1 on the ordinary layouts; ±3 on the floor layouts, where every pilot
    arrives at a fifth of the power and the channel is slow enough to allow it."""

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
"""Every control frame (ACK, connect, ping) uses the most robust mode on the SHORT layout."""


NARROW_LONG = FrameLayout("long", data_symbols=32, waveform=NARROW_500)
"""500 Hz data frames: the same 34 symbols ≈ 1.05 s; 28 payload symbols × 8 carriers =
224 QAM symbols."""
NARROW_SHORT = FrameLayout("short", data_symbols=12, waveform=NARROW_500)
"""500 Hz control frames: the same 14 symbols; 10 × 8 = 80 QAM symbols → 7 payload bytes
at the narrow control mode (QPSK ½), exactly the wide control frame's capacity."""
FLOOR_PREAMBLE_SYMBOLS = 8
"""Schmidl–Cox symbols of a floor frame (ADR-0009): four times the ordinary preamble, which
the detector integrates for 6 dB, and a run of seven full two-symbol peaks that no ordinary
frame produces."""
NARROW_FLOOR_LONG = FrameLayout(
    "floor-long",
    data_symbols=128,
    waveform=NARROW_500,
    preamble_symbols=FLOOR_PREAMBLE_SYMBOLS,
    pilot_smoothing=3,
)
"""500 Hz floor data frames: 136 symbols ≈ 4.2 s; 112 payload symbols × 8 carriers = 896
QAM symbols, so a tenth-rate QPSK mode still carries 19 bytes and a fifth-rate one 41 — a
connect request. Sixteen full pilot symbols carry the mode chips."""
NARROW_FLOOR_SHORT = FrameLayout(
    "floor-short",
    data_symbols=64,
    waveform=NARROW_500,
    preamble_symbols=FLOOR_PREAMBLE_SYMBOLS,
    pilot_smoothing=3,
)
"""500 Hz floor control frames: 72 symbols ≈ 2.2 s; 56 × 8 = 448 QAM symbols → 8 payload
bytes at the floor control mode (QPSK 1/10), one more than a control frame needs."""

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
"""The 500 Hz mode table, most robust first. Modes 0 and 1 are the floor family's
(ADR-0009): QPSK 1/10 and QPSK ⅕ on :data:`NARROW_FLOOR_LONG`, 19 and 41 bytes a frame at
about −12 and −10 dB. Mode 2, QPSK ⅓ on the ordinary frame, is the rung between them and
mode 3. Mode 3 is QPSK ½: the slowest mode whose SHORT frame carries a control frame and
whose LONG frame carries a connect request, and — with the narrow waveform's per-carrier
advantage — one that reaches about the same 3 kHz SNR as the wide table's BPSK ⅕; control
frames, connect requests, beacons and probes go out at it. Thirteen modes is what the
32-chip sequence set holds at |ρ| ≤ 0.25."""

NARROW_FLOOR_MODES = 2
"""Leading modes of the narrow table that go out on the floor layouts."""
NARROW_CONTROL_MODE_INDEX = 3
NARROW_CONTROL_MODE = NARROW_MODES[NARROW_CONTROL_MODE_INDEX]
NARROW_FLOOR_ACQUISITION_THRESHOLD = 0.32
"""The detector's threshold on its floor statistic at 500 Hz — four two-symbol windows of
an eight-symbol preamble combined coherently — set like the ordinary one, just above the
statistic's maximum over 60 s of band-limited noise (0.314)."""


@dataclass(frozen=True)
class AirInterface:
    """One waveform with the layouts and modes that go with it — what a transmitter,
    receiver, detector or link harness needs to know about the air it is on."""

    params: WaveformParams
    long: FrameLayout
    short: FrameLayout
    modes: tuple[Mode, ...]
    chip_correlation_bound: float
    """Largest pairwise correlation allowed between the (mode, RV) chip sequences. The
    wide waveform has 168 chips and holds 56 sequences at 0.2; the narrow one has 32 and
    holds its 52 at 0.25 — the metric loses a little separation and keeps a 15 dB
    processing gain, which at the narrow floor is still a reliable decision."""
    acquisition_threshold: float
    """The detector's normalised matched-filter peak above which a preamble is declared.
    Set just above the statistic's maximum over 60 s of band-limited noise: 0.348 at
    2 300 Hz, so 0.36; 0.549 at 500 Hz, so 0.56. The narrow waveform's noise maximum is
    higher because its band-limited noise has a fifth of the degrees of freedom in a
    preamble's span, and its signal peaks are higher by about as much (0.57–0.68 at
    −5 dB against 0.42–0.50), so the two floors land in the same place."""
    floor_long: FrameLayout | None = None
    """The floor family's data layout (ADR-0009), when this air has one: the frame the
    modes below :attr:`control_mode` go out on, and a connect request once the ordinary
    frame has gone unanswered."""
    floor_short: FrameLayout | None = None
    """The floor family's control layout: acknowledgements and the other control frames
    while the link runs a floor mode."""
    floor_modes: int = 0
    """How many of the leading modes are floor modes — the slowest of the table and the
    only ones on the floor layouts. Mode ``floor_modes`` is the first ordinary one."""
    control_mode_index: int = 0
    """The mode control frames, connect requests, beacons and probes go out at: the
    slowest whose SHORT frame carries a control frame — mode 0 on the wide air, mode 3
    (QPSK ½) on the narrow one, where mode 2 (QPSK ⅓) is an ordinary data mode whose
    SHORT frame would carry three bytes."""
    floor_acquisition_threshold: float = 1.0
    """The detector's threshold on its floor statistic (the two-symbol bank's outputs at
    four positions two symbols apart, combined coherently), set like
    :attr:`acquisition_threshold`; 1.0 — never — on an air without a floor family."""

    @property
    def control_mode(self) -> Mode:
        """The mode control frames, connect requests, beacons and probes go out at."""
        return self.modes[self.control_mode_index]

    @property
    def floor_control_mode(self) -> Mode:
        """The mode floor control frames use on :attr:`floor_short` — the slowest of all."""
        if not self.floor_modes:
            raise ValueError(f"{self.name} has no floor family")
        return self.modes[0]

    def control_mode_for(self, floor: bool) -> Mode:
        return self.floor_control_mode if floor else self.control_mode

    @property
    def n_modes(self) -> int:
        return len(self.modes)

    @property
    def name(self) -> str:
        return self.params.bandwidth.name

    def is_floor(self, mode_index: int) -> bool:
        return mode_index < self.floor_modes

    def layout_for(self, data: bool, floor: bool = False) -> FrameLayout:
        if floor:
            layout = self.floor_long if data else self.floor_short
            if layout is None:
                raise ValueError(f"{self.name} has no floor family")
            return layout
        return self.long if data else self.short

    def data_layout(self, mode_index: int) -> FrameLayout:
        """The layout a DATA frame at this mode goes out on."""
        return self.layout_for(True, self.is_floor(mode_index))

    @property
    def layouts(self) -> tuple[FrameLayout, ...]:
        """Every layout of this air, the ordinary two first."""
        return tuple(
            x for x in (self.long, self.short, self.floor_long, self.floor_short) if x is not None
        )


WIDE = AirInterface(WIDE_2300, LONG, SHORT, MODES, 0.2, 0.36)
NARROW = AirInterface(
    NARROW_500,
    NARROW_LONG,
    NARROW_SHORT,
    NARROW_MODES,
    0.25,
    0.56,
    floor_long=NARROW_FLOOR_LONG,
    floor_short=NARROW_FLOOR_SHORT,
    floor_modes=NARROW_FLOOR_MODES,
    control_mode_index=NARROW_CONTROL_MODE_INDEX,
    floor_acquisition_threshold=NARROW_FLOOR_ACQUISITION_THRESHOLD,
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
    """Human-readable summary, e.g. for docs/spec and the GUI. With ``air`` every mode is
    tabulated on the layout it actually goes out on (the floor modes on the floor
    layout); otherwise on ``layout``."""
    rows: list[dict[str, float | int | str]] = []
    if air is None:
        air = air_interface(layout.waveform)
        per_mode = False
    else:
        per_mode = True
    for m in modes if modes is not None else air.modes:
        lay = air.data_layout(m.index) if per_mode else layout
        rows.append(
            {
                "mode": m.index,
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
        )
    return rows
