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

Mode and redundancy-version signalling: DATA frames carry their mode index and HARQ
redundancy version as PN chips on the *data* carriers of the full pilot symbols (4 × 42 =
168 chips in a LONG frame); one sequence per (RV, mode) pair, 4 × 14 = 56 in all. The comb
pilots on those symbols stay known, so the receiver estimates the channel from them,
correlates the chips coherently against all 56 sequences, and only then treats the pilot
symbols as fully known. Carrying the RV here — outside the LDPC codeword, like the DCI of a
cellular downlink — is what lets the link layer soft-combine retransmissions of a frame
whose payload (and therefore sequence number) it could not decode. CONTROL frames always
use the control mode and RV 0, so their pilot symbols carry the plain pilot sequence.

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

from aether_model.frame.modes import PREAMBLE_SYMBOLS, FrameLayout, air_interface
from aether_model.phy.ofdm import carrier_map
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]

SC_SEEDS = {0: 4649, 1: 7919}
"""PN seeds of the Schmidl–Cox sequence per frame type (fixed by the air-interface spec).
The same seeds serve every bandwidth: the sequence is drawn to the length of the even
carriers — 29 at 2 300 Hz, 6 at 500 Hz — and the two frame types stay separated at both
(orthogonal at length 6; the constructor checks)."""
SC_SEPARATION = 0.3
"""Largest cosine allowed between any two preamble sequences of one waveform."""
MODE_CHIP_SEED = 20260913
N_MODES = 14
"""Most modes any air interface may signal; the wide table uses all fourteen, the narrow
one its first ten. Each air interface's chip sequences are indexed by *its* mode count
(:meth:`Preamble.chip_index`)."""
N_RV = 4
"""Redundancy versions signalled per frame (TS 38.212 rate matching has four)."""
MAX_PILOT_SYMBOLS = 4


class FrameType(Enum):
    DATA = 0
    CONTROL = 1


@dataclass(frozen=True)
class FrameHeader:
    frame_type: FrameType
    mode: int = 0
    """Mode index for DATA frames; ignored (0) for CONTROL frames."""
    rv: int = 0
    """HARQ redundancy version for DATA frames; ignored (0) for CONTROL frames."""

    def __post_init__(self) -> None:
        if not 0 <= self.mode < N_MODES:
            raise ValueError(f"mode index must be 0 … {N_MODES - 1}")
        if not 0 <= self.rv < N_RV:
            raise ValueError(f"redundancy version must be 0 … {N_RV - 1}")


def chip_index(mode: int, rv: int, n_modes: int = N_MODES) -> int:
    """Index of the chip sequence carrying (mode, rv). RV 0 uses the first ``n_modes``
    sequences, so RV-0 frames are unchanged from the P2-3 air interface (golden vectors
    hold)."""
    return rv * n_modes + mode


def chip_hypothesis(index: int, n_modes: int = N_MODES) -> tuple[int, int]:
    """Inverse of :func:`chip_index`: ``(mode, rv)``."""
    return index % n_modes, index // n_modes


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
def mode_chip_sequences(
    n_chips: int, count: int = N_RV * N_MODES, max_corr: float = 0.2
) -> tuple[ComplexArray, ...]:
    """``count`` ±1 sequences of ``n_chips``, one per (rv, mode) pair (:func:`chip_index`
    order), pairwise |correlation| ≤ ``max_corr``. The wide waveform's 56 sequences of
    168 chips at 0.2 are the P2-3 set, unchanged; the narrow waveform's 40 of 32 chips
    hold at 0.25 (56 of 32 do not, at any bound worth having)."""
    return tuple(_select_pn_set(n_chips, count, MODE_CHIP_SEED, max_corr))


class Preamble:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.air = air_interface(params)
        self.n_modes = self.air.n_modes
        self.cmap = carrier_map(params)
        n = self.cmap.n_carriers
        self.even = np.flatnonzero(self.cmap.bins % 2 == 0)
        scale = math.sqrt(n / len(self.even))
        self._sc: dict[FrameType, ComplexArray] = {}
        for ft in FrameType:
            sc = np.zeros(n, dtype=np.complex128)
            sc[self.even] = _pn(SC_SEEDS[ft.value], len(self.even)) * scale
            self._sc[ft] = sc
        # ensure every pair of sequences is well separated
        keys = list(self._sc)
        for i, k1 in enumerate(keys):
            for k2 in keys[i + 1 :]:
                a, b = self._sc[k1], self._sc[k2]
                cosine = abs(np.vdot(a, b)) / np.sqrt(np.vdot(a, a).real * np.vdot(b, b).real)
                if cosine > SC_SEPARATION:
                    raise RuntimeError("preamble PN sequences correlate too strongly")
        self.n_data = len(self.cmap.data_carriers)
        self.n_chips = MAX_PILOT_SYMBOLS * self.n_data

    @property
    def sequences(self) -> tuple[ComplexArray, ...]:
        """This air interface's (rv, mode) chip sequences, :meth:`chip_index` order."""
        return mode_chip_sequences(
            self.n_chips, N_RV * self.n_modes, self.air.chip_correlation_bound
        )

    def chip_index(self, mode: int, rv: int) -> int:
        if not 0 <= mode < self.n_modes:
            raise ValueError(f"mode index must be 0 … {self.n_modes - 1} at {self.air.name}")
        return chip_index(mode, rv, self.n_modes)

    def chip_hypothesis(self, index: int) -> tuple[int, int]:
        return chip_hypothesis(index, self.n_modes)

    def sc_values(self, frame_type: FrameType = FrameType.DATA) -> ComplexArray:
        """Carrier values of each Schmidl–Cox symbol (unit mean power over active carriers)."""
        return self._sc[frame_type].copy()

    def symbols(self, header: FrameHeader, layout: FrameLayout | None = None) -> list[ComplexArray]:
        """The preamble: ``layout.preamble_symbols`` copies of the type's SC symbol (two)."""
        n = PREAMBLE_SYMBOLS if layout is None else layout.preamble_symbols
        sc = self.sc_values(header.frame_type)
        return [sc.copy() for _ in range(n)]

    def sequences_for(self, layout: FrameLayout | None) -> tuple[ComplexArray, ...]:
        """The chip set of a layout: :attr:`sequences` on every layout."""
        del layout
        return self.sequences

    def mode_chips(
        self,
        mode: int,
        pilot_symbol_index: int,
        rv: int = 0,
        layout: FrameLayout | None = None,
    ) -> ComplexArray:
        """Chips (±1) for the data carriers of the given full pilot symbol of a DATA frame."""
        seq = self.sequences_for(layout)[self.chip_index(mode, rv)]
        a = pilot_symbol_index * self.n_data
        if a + self.n_data > len(seq):
            raise ValueError("more pilot symbols than the chip sequence covers")
        return seq[a : a + self.n_data]


@cache
def preamble(params: WaveformParams = WIDE_2300) -> Preamble:
    return Preamble(params)
