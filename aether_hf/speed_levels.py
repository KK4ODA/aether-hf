"""
aether_hf/speed_levels.py

Speed level definitions for AETHER HF Wide and Narrow modes.
Each level specifies modulation, code rate, block length, and performance targets.
"""

from dataclasses import dataclass
from enum import Enum


class Modulation(Enum):
    FSK2   = "2-FSK"
    FSK4   = "4-FSK"
    BPSK   = "BPSK"
    QPSK   = "QPSK"
    PSK8   = "8-PSK"
    QAM16  = "16-QAM"
    QAM32  = "32-QAM"
    QAM64  = "64-QAM"
    QAM128 = "128-QAM"
    QAM256 = "256-QAM"


# Bits per symbol for each modulation
BITS_PER_SYMBOL = {
    Modulation.FSK2:   1,
    Modulation.FSK4:   2,
    Modulation.BPSK:   1,
    Modulation.QPSK:   2,
    Modulation.PSK8:   3,
    Modulation.QAM16:  4,
    Modulation.QAM32:  5,
    Modulation.QAM64:  6,
    Modulation.QAM128: 7,
    Modulation.QAM256: 8,
}


@dataclass(frozen=True)
class SpeedLevel:
    """Definition of one AETHER HF speed level."""
    level:        int
    modulation:   Modulation
    code_rate:    float       # e.g. 0.25, 0.5, 0.75
    block_length: int         # LDPC block length in bits
    carriers:     int         # number of data subcarriers
    net_rate_bps: int         # approximate net data rate
    min_snr_db:   float       # minimum required SNR
    coherent:     bool        # True = coherent detection, False = non-coherent


# ── Wide Mode (2,300 Hz) Speed Levels ─────────────────────────────────

WIDE_LEVELS = [
    SpeedLevel( 1, Modulation.FSK2,   1/8,   256, 32, 12,   -10, False),
    SpeedLevel( 2, Modulation.FSK2,   1/4,   256, 32, 24,    -7, False),
    SpeedLevel( 3, Modulation.FSK4,   1/4,   512, 16, 48,    -4, False),
    SpeedLevel( 4, Modulation.BPSK,   1/4,  1024, 52, 210,   -1, False),
    SpeedLevel( 5, Modulation.BPSK,   1/2,  2048, 52, 425,    2, True),
    SpeedLevel( 6, Modulation.QPSK,   1/3,  2048, 52, 565,    4, True),
    SpeedLevel( 7, Modulation.QPSK,   1/2,  4096, 52, 850,    6, True),
    SpeedLevel( 8, Modulation.QPSK,   2/3,  4096, 52, 1130,   8, True),
    SpeedLevel( 9, Modulation.QPSK,   3/4,  4096, 52, 1270,  10, True),
    SpeedLevel(10, Modulation.PSK8,   1/2,  4096, 52, 1270,  11, True),
    SpeedLevel(11, Modulation.PSK8,   2/3,  4096, 52, 1700,  13, True),
    SpeedLevel(12, Modulation.PSK8,   3/4,  4096, 52, 1910,  15, True),
    SpeedLevel(13, Modulation.QAM16,  1/2,  4096, 52, 1700,  14, True),
    SpeedLevel(14, Modulation.QAM16,  2/3,  4096, 52, 2265,  16, True),
    SpeedLevel(15, Modulation.QAM16,  3/4,  4096, 52, 2550,  18, True),
    SpeedLevel(16, Modulation.QAM32,  3/4,  4096, 52, 3185,  21, True),
    SpeedLevel(17, Modulation.QAM64,  2/3,  4096, 52, 3400,  23, True),
    SpeedLevel(18, Modulation.QAM64,  3/4,  4096, 52, 3825,  25, True),
    SpeedLevel(19, Modulation.QAM128, 3/4,  4096, 52, 4475,  28, True),
    SpeedLevel(20, Modulation.QAM256, 5/6,  4096, 52, 5700,  31, True),
]

# ── Narrow Mode (500 Hz) Speed Levels ─────────────────────────────────

NARROW_LEVELS = [
    SpeedLevel( 1, Modulation.FSK2,  1/8,  256,  8,   2,  -10, False),
    SpeedLevel( 2, Modulation.FSK2,  1/4,  256,  8,   4,   -7, False),
    SpeedLevel( 3, Modulation.BPSK,  1/4,  512,  8,  32,   -1, False),
    SpeedLevel( 4, Modulation.BPSK,  1/2, 1024,  8,  65,    2, True),
    SpeedLevel( 5, Modulation.QPSK,  1/3, 1024,  8,  87,    4, True),
    SpeedLevel( 6, Modulation.QPSK,  1/2, 2048,  8, 130,    6, True),
    SpeedLevel( 7, Modulation.QPSK,  2/3, 2048,  8, 174,    8, True),
    SpeedLevel( 8, Modulation.QPSK,  3/4, 2048,  8, 195,   10, True),
    SpeedLevel( 9, Modulation.PSK8,  1/2, 2048,  8, 195,   11, True),
    SpeedLevel(10, Modulation.PSK8,  2/3, 2048,  8, 260,   13, True),
    SpeedLevel(11, Modulation.PSK8,  3/4, 2048,  8, 293,   15, True),
    SpeedLevel(12, Modulation.QAM16, 1/2, 2048,  8, 260,   14, True),
    SpeedLevel(13, Modulation.QAM16, 2/3, 2048,  8, 347,   16, True),
    SpeedLevel(14, Modulation.QAM16, 3/4, 2048,  8, 390,   18, True),
]


def get_level(level_num: int, mode: str = "wide") -> SpeedLevel:
    """Look up a speed level by number and mode."""
    levels = WIDE_LEVELS if mode == "wide" else NARROW_LEVELS
    for sl in levels:
        if sl.level == level_num:
            return sl
    raise ValueError(f"Unknown speed level {level_num} for {mode} mode")
