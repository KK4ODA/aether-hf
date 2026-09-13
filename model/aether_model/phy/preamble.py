"""Frame preamble: two Schmidl–Cox symbols and a unique word that carries the frame header.

Symbol 1–2 (``SC``): a pseudo-random BPSK sequence on the *even* carriers only (odd carriers
zero), so each symbol's useful part consists of two identical halves. The receiver detects
the frame and estimates fractional CFO from that repetition without knowing anything else
(Schmidl & Cox, IEEE Trans. Commun. 1997). The two symbols are identical, which also gives
a full-symbol repetition for a finer CFO estimate and a two-symbol matched filter for
sample-accurate timing. The sequence is PN rather than Zadoff–Chu on purpose: a ZC chirp
shifted in frequency is (up to phase) the same chirp shifted in time, so a matched filter
could not tell a CFO error from a timing error; a PN symbol decorrelates under either.

Symbol 3 (``UW``): a pseudo-random BPSK sequence on all carriers, one sequence per header
code (frame type and mode), chosen for low mutual and shifted correlation. Three
channel-blind correlations identify the code and the integer part of the CFO: the ratio
of consecutive symbols on shared carriers (SC2 → UW, UW → first pilot symbol) and the
UW's own adjacent-carrier differential. A PN sequence is used rather than a Zadoff–Chu
sequence because a ZC chirp's adjacent-carrier differential is a pure tone, which a bin
shift merely rotates — it carries no integer-CFO information.

All three symbols have the same mean power as data symbols and near-constant envelopes.
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

SC_SEED = 4649
"""Seed of the Schmidl–Cox PN sequence (fixed by the air-interface specification)."""
N_HEADER_CODES = 32


class FrameType(Enum):
    DATA = 0
    CONTROL = 1


@dataclass(frozen=True)
class FrameHeader:
    frame_type: FrameType
    mode: int = 0
    """Mode index for DATA frames; ignored (0) for CONTROL frames."""

    @property
    def code(self) -> int:
        if self.frame_type is FrameType.CONTROL:
            return 16
        if not 0 <= self.mode < 16:
            raise ValueError("mode index must be 0 … 15")
        return self.mode

    @classmethod
    def from_code(cls, code: int) -> FrameHeader:
        if code == 16:
            return cls(FrameType.CONTROL, 0)
        if 0 <= code < 16:
            return cls(FrameType.DATA, code)
        raise ValueError(f"reserved header code {code}")


def _pn_candidates(n_carriers: int, count: int, seed: int = 20260913) -> list[ComplexArray]:
    rng = np.random.default_rng(seed)
    return [
        (1.0 - 2.0 * rng.integers(0, 2, n_carriers)).astype(np.complex128) for _ in range(count)
    ]


def _shifted_self_correlation(x: ComplexArray, max_shift: int = 3) -> float:
    n = len(x)
    worst = 0.0
    for m in range(1, max_shift + 1):
        worst = max(worst, float(abs(np.vdot(x[m:], x[:-m]))) / (n - m))
    return worst


def unique_words(n_carriers: int, count: int = N_HEADER_CODES) -> list[ComplexArray]:
    """``count`` PN sequences of length ``n_carriers`` with pairwise |correlation| ≤ 0.3 and
    low correlation with their own shifted copies (deterministic selection)."""
    chosen: list[ComplexArray] = []
    for cand in _pn_candidates(n_carriers, 40 * count):
        if _shifted_self_correlation(cand) > 0.3:
            continue
        if all(abs(np.vdot(cand, c)) / n_carriers <= 0.3 for c in chosen):
            chosen.append(cand)
            if len(chosen) == count:
                return chosen
    raise RuntimeError("could not find enough unique-word sequences")


class Preamble:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        n = self.cmap.n_carriers
        self.even = np.flatnonzero(self.cmap.bins % 2 == 0)
        sc = np.zeros(n, dtype=np.complex128)
        pn = 1.0 - 2.0 * np.random.default_rng(SC_SEED).integers(0, 2, len(self.even))
        sc[self.even] = pn * math.sqrt(n / len(self.even))
        self._sc = sc
        self._uw = dict(enumerate(unique_words(n)))

    @property
    def sc_values(self) -> ComplexArray:
        """Carrier values of each Schmidl–Cox symbol (unit mean power over active carriers)."""
        return self._sc.copy()

    def uw_values(self, header: FrameHeader) -> ComplexArray:
        return self._uw[header.code].copy()

    def uw_candidates(self) -> dict[int, ComplexArray]:
        """All unique-word carrier vectors keyed by header code (for the detector)."""
        return {k: v.copy() for k, v in self._uw.items()}

    def symbols(self, header: FrameHeader) -> list[ComplexArray]:
        return [self.sc_values, self.sc_values, self.uw_values(header)]


@cache
def preamble(params: WaveformParams = WIDE_2300) -> Preamble:
    return Preamble(params)
