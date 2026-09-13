"""Frame preamble and in-frame header signalling (P1-5, revised in P2-3).

Preamble: two identical Schmidl–Cox symbols — a PN sequence on the *even* carriers only
(odd carriers zero), so each symbol's useful part consists of two identical halves and the
two symbols repeat each other. The receiver detects the frame, estimates timing and carrier
offset from that structure without knowing anything else (Schmidl & Cox, IEEE Trans.
Commun. 1997). **The frame type is carried by the choice of PN sequence**: DATA frames and
CONTROL frames use different sequences, and the matched-filter bank correlates against
both. With the full processing gain of the preamble behind it, this two-way decision is
reliable wherever the preamble is detectable at all — far below where a separate header
symbol could be read.

Mode signalling: DATA frames carry their mode index as PN chips on the *data* carriers of
the full pilot symbols (4 × 42 = 168 chips in a LONG frame). The comb pilots on those
symbols stay known, so the receiver estimates the channel from them, correlates the chips
coherently against all 14 mode sequences, and only then treats the pilot symbols as fully
known. CONTROL frames always use the control mode, so their pilot symbols carry the plain
pilot sequence.

PN rather than Zadoff–Chu everywhere in the preamble: a ZC chirp shifted in frequency is
(up to phase) the same chirp shifted in time, so a matched filter could not tell a CFO
error from a timing error; a PN symbol decorrelates under either.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from enum import Enum
from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.phy.ofdm import carrier_map
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]

SC_SEEDS = {0: 4649, 1: 7919}
"""PN seeds of the Schmidl–Cox sequence per frame type (fixed by the air-interface spec)."""
MODE_CHIP_SEED = 20260913
N_MODES = 14
MAX_PILOT_SYMBOLS = 4


class FrameType(Enum):
    DATA = 0
    CONTROL = 1


@dataclass(frozen=True)
class FrameHeader:
    frame_type: FrameType
    mode: int = 0
    """Mode index for DATA frames; ignored (0) for CONTROL frames."""

    def __post_init__(self) -> None:
        if not 0 <= self.mode < N_MODES:
            raise ValueError(f"mode index must be 0 … {N_MODES - 1}")


def _pn(seed: int, n: int) -> ComplexArray:
    return (1.0 - 2.0 * np.random.default_rng(seed).integers(0, 2, n)).astype(np.complex128)


def _shifted_self_correlation(x: ComplexArray, max_shift: int = 3) -> float:
    n = len(x)
    worst = 0.0
    for m in range(1, max_shift + 1):
        worst = max(worst, float(abs(np.vdot(x[m:], x[:-m]))) / (n - m))
    return worst


def _select_pn_set(length: int, count: int, seed: int, max_corr: float) -> list[ComplexArray]:
    """``count`` PN sequences with pairwise |correlation| ≤ max_corr (deterministic)."""
    rng = np.random.default_rng(seed)
    chosen: list[ComplexArray] = []
    for _ in range(200 * count):
        cand = (1.0 - 2.0 * rng.integers(0, 2, length)).astype(np.complex128)
        if all(abs(np.vdot(cand, c)) / length <= max_corr for c in chosen):
            chosen.append(cand)
            if len(chosen) == count:
                return chosen
    raise RuntimeError("could not find enough PN sequences")


@cache
def mode_chip_sequences(n_chips: int) -> tuple[ComplexArray, ...]:
    """One ±1 sequence of ``n_chips`` per mode, pairwise |correlation| ≤ 0.2."""
    return tuple(_select_pn_set(n_chips, N_MODES, MODE_CHIP_SEED, 0.2))


class Preamble:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        n = self.cmap.n_carriers
        self.even = np.flatnonzero(self.cmap.bins % 2 == 0)
        scale = math.sqrt(n / len(self.even))
        self._sc: dict[FrameType, ComplexArray] = {}
        for ft in FrameType:
            sc = np.zeros(n, dtype=np.complex128)
            sc[self.even] = _pn(SC_SEEDS[ft.value], len(self.even)) * scale
            self._sc[ft] = sc
        # ensure the two type sequences are well separated
        a, b = self._sc[FrameType.DATA][self.even], self._sc[FrameType.CONTROL][self.even]
        if abs(np.vdot(a, b)) / np.vdot(a, a).real > 0.3:
            raise RuntimeError("frame-type PN sequences correlate too strongly")
        self.n_data = len(self.cmap.data_carriers)
        self.n_chips = MAX_PILOT_SYMBOLS * self.n_data

    def sc_values(self, frame_type: FrameType = FrameType.DATA) -> ComplexArray:
        """Carrier values of each Schmidl–Cox symbol (unit mean power over active carriers)."""
        return self._sc[frame_type].copy()

    def symbols(self, header: FrameHeader) -> list[ComplexArray]:
        sc = self.sc_values(header.frame_type)
        return [sc, sc.copy()]

    def mode_chips(self, mode: int, pilot_symbol_index: int) -> ComplexArray:
        """Chips (±1) for the data carriers of the given full pilot symbol of a DATA frame."""
        seq = mode_chip_sequences(self.n_chips)[mode]
        a = pilot_symbol_index * self.n_data
        if a + self.n_data > len(seq):
            raise ValueError("more pilot symbols than the chip sequence covers")
        return seq[a : a + self.n_data]


@cache
def preamble(params: WaveformParams = WIDE_2300) -> Preamble:
    return Preamble(params)
