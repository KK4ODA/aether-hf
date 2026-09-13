"""Frame layouts and the mode table — everything derives from :mod:`aether_model.waveform`.

A *frame* is a preamble (two Schmidl–Cox symbols whose PN sequence encodes the frame
type) followed by ``data_symbols`` OFDM symbols, every ``pilot_symbol_period``-th of which
(starting with the first) is a full pilot symbol; in DATA frames the data carriers of the
full pilot symbols carry the mode index as PN chips. A *mode* is a (modulation, code rate)
pair; together with a layout it fixes the number of coded bits, information bits and
payload bytes per frame.

Base-graph choice follows the public 5G rule (TS 38.212 §7.2.2): BG2 for small blocks or
low rates, BG1 otherwise. One code block per frame.
"""

from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction

from aether_model.fec.crc import CRC24A, Crc
from aether_model.fec.nr_ldpc import select_lifting_size
from aether_model.waveform import WIDE_2300, Modulation, WaveformParams

PREAMBLE_SYMBOLS = 2
PAYLOAD_CRC: Crc = CRC24A


@dataclass(frozen=True)
class FrameLayout:
    name: str
    data_symbols: int
    waveform: WaveformParams = WIDE_2300

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
        return PREAMBLE_SYMBOLS + self.data_symbols

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


def mode_table(layout: FrameLayout = LONG) -> list[dict[str, float | int | str]]:
    """Human-readable summary, e.g. for docs/spec and the GUI."""
    rows: list[dict[str, float | int | str]] = []
    for m in MODES:
        rows.append(
            {
                "mode": m.index,
                "name": m.name,
                "bits_per_symbol": m.modulation.bits_per_symbol,
                "code_rate": str(m.code_rate),
                "coded_bits": m.coded_bits(layout),
                "payload_bytes": m.payload_bytes(layout),
                "base_graph": m.base_graph(layout),
                "z": m.lifting_size(layout),
                "net_bps": round(m.net_bit_rate(layout)),
            }
        )
    return rows
