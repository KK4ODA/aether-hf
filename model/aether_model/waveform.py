"""Aether HF v1 waveform parameters (ADR-0002) and the quantities derived from them.

This module is the single place where the OFDM numerology lives. Everything downstream —
mode table, frame timing, benchmark labels, the air-interface specification — derives from a
:class:`WaveformParams` instance rather than restating numbers, so a change here propagates
everywhere and the spec can never disagree with the code.

The defaults are the ADR-0002 *starting point*; Phase 1 simulation (P1-4 … P1-7) may adjust
them, in which case the ADR is amended and this file is the record.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from fractions import Fraction


class Modulation(Enum):
    """Constellations the v1 waveform supports (Gray-labelled, BICM)."""

    BPSK = 1
    QPSK = 2
    PSK8 = 3
    QAM16 = 4
    QAM64 = 6

    @property
    def bits_per_symbol(self) -> int:
        return self.value


class Bandwidth(Enum):
    """Occupied-bandwidth options, mirroring what host applications already know how to ask
    for (VARA's BW500 / BW2300 / BW2750)."""

    NARROW_500 = 500
    WIDE_2300 = 2300
    WIDE_2750 = 2750

    @property
    def hz(self) -> int:
        return self.value


@dataclass(frozen=True)
class WaveformParams:
    """OFDM numerology for one bandwidth option."""

    bandwidth: Bandwidth = Bandwidth.WIDE_2300
    fs_baseband: float = 8000.0
    """Complex baseband sample rate (Hz); 48 kHz audio ÷ 6."""
    fft_size: int = 200
    """FFT length → subcarrier spacing 40 Hz, useful-symbol length 25 ms."""
    cp_samples: int = 48
    """Cyclic prefix, 6 ms: covers ITU Poor (2 ms) with margin; extended CP is a mode option."""
    centre_hz: float = 1500.0
    """Audio centre frequency of the passband signal."""
    pilot_carrier_spacing: int = 4
    """Comb pilots on every symbol: every N-th carrier, both band edges always pilots."""
    pilot_symbol_period: int = 8
    """Every N-th OFDM symbol is a full pilot symbol (time-direction channel tracking)."""
    taper_samples: int = 8
    """Raised-cosine taper on each symbol edge (1 ms); adjacent symbols overlap-add by this
    much, so the effective cyclic prefix is ``cp_samples − taper_samples`` = 5 ms."""
    audio_rate: int = 48000

    # ── derived ───────────────────────────────────────────────────────

    @property
    def subcarrier_spacing_hz(self) -> float:
        return self.fs_baseband / self.fft_size

    @property
    def useful_symbol_s(self) -> float:
        return self.fft_size / self.fs_baseband

    @property
    def cp_s(self) -> float:
        return self.cp_samples / self.fs_baseband

    @property
    def symbol_samples(self) -> int:
        return self.fft_size + self.cp_samples

    @property
    def symbol_period_s(self) -> float:
        return self.symbol_samples / self.fs_baseband

    @property
    def symbol_rate_bd(self) -> float:
        return 1.0 / self.symbol_period_s

    @property
    def effective_cp_s(self) -> float:
        return (self.cp_samples - self.taper_samples) / self.fs_baseband

    @property
    def n_carriers(self) -> int:
        """Total active carriers that fit inside the nominal bandwidth."""
        return int(self.bandwidth.hz // self.subcarrier_spacing_hz)

    @property
    def occupied_bandwidth_hz(self) -> float:
        return self.n_carriers * self.subcarrier_spacing_hz

    @property
    def n_pilot_carriers(self) -> int:
        """Comb pilots at indices 0, s, 2s, … plus the upper edge if it is not on the grid."""
        last = self.n_carriers - 1
        on_grid = last // self.pilot_carrier_spacing + 1
        return on_grid + (0 if last % self.pilot_carrier_spacing == 0 else 1)

    @property
    def n_data_carriers(self) -> int:
        return self.n_carriers - self.n_pilot_carriers

    @property
    def pilot_symbol_fraction(self) -> float:
        return 1.0 / self.pilot_symbol_period if self.pilot_symbol_period else 0.0

    @property
    def resample_factor(self) -> int:
        ratio = Fraction(self.audio_rate, 1) / Fraction(self.fs_baseband).limit_denominator()
        if ratio.denominator != 1:
            raise ValueError("audio rate must be an integer multiple of the baseband rate")
        return int(ratio)

    def raw_bit_rate(self, modulation: Modulation, code_rate: Fraction | float) -> float:
        """Coded-payload bit rate before framing/ARQ overhead (bits per second)."""
        data_symbols_per_s = (
            self.n_data_carriers * self.symbol_rate_bd * (1.0 - self.pilot_symbol_fraction)
        )
        return data_symbols_per_s * modulation.bits_per_symbol * float(code_rate)

    def summary(self) -> dict[str, float | int | str]:
        return {
            "bandwidth": self.bandwidth.name,
            "fs_baseband_hz": self.fs_baseband,
            "fft_size": self.fft_size,
            "subcarrier_spacing_hz": self.subcarrier_spacing_hz,
            "useful_symbol_ms": 1e3 * self.useful_symbol_s,
            "cp_ms": 1e3 * self.cp_s,
            "symbol_rate_bd": self.symbol_rate_bd,
            "n_carriers": self.n_carriers,
            "n_pilot_carriers": self.n_pilot_carriers,
            "n_data_carriers": self.n_data_carriers,
            "occupied_bandwidth_hz": self.occupied_bandwidth_hz,
        }


WIDE_2300 = WaveformParams(Bandwidth.WIDE_2300)
WIDE_2750 = WaveformParams(Bandwidth.WIDE_2750)
NARROW_500 = WaveformParams(Bandwidth.NARROW_500)

WAVEFORMS: dict[Bandwidth, WaveformParams] = {
    Bandwidth.WIDE_2300: WIDE_2300,
    Bandwidth.WIDE_2750: WIDE_2750,
    Bandwidth.NARROW_500: NARROW_500,
}
