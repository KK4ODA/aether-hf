"""Frame preamble: two Schmidl–Cox symbols and a unique word that carries the frame header.

Symbol 1–2 (``SC``): a Zadoff–Chu sequence on the *even* carriers only (odd carriers zero),
so each symbol's useful part consists of two identical halves. The receiver detects the
frame and estimates fractional CFO from that repetition without knowing anything else
(Schmidl & Cox, IEEE Trans. Commun. 1997). The two symbols are identical, which also gives
a full-symbol repetition for a finer CFO estimate and a two-symbol matched filter for
sample-accurate timing.

Symbol 3 (``UW``): a Zadoff–Chu sequence of the largest prime length ≤ n_carriers,
cyclically extended over all carriers (the LTE construction), whose root encodes the frame
header (frame type and mode). Prime length matters: for composite lengths, roots whose
difference shares a factor with N correlate strongly. The receiver identifies the root —
and the integer part of the CFO — by a differential correlation across carriers, which is
insensitive to the unknown channel phase. Distinct roots have cross-correlation ≈ 1/√N, so
32 hypotheses are separated by ~17 dB of processing gain over a single carrier's SNR.

All three symbols have the same mean power as data symbols and near-constant envelopes.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from enum import Enum
from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.phy.ofdm import PILOT_ROOT, carrier_map, zadoff_chu
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]

SC_ROOT = 3
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


def largest_prime_at_most(n: int) -> int:
    for cand in range(n, 1, -1):
        if all(cand % d for d in range(2, math.isqrt(cand) + 1)):
            return cand
    raise ValueError("no prime ≤ n")


def header_roots(n_carriers: int) -> tuple[int, ...]:
    """32 roots of the prime-length unique-word sequence (all non-zero residues are coprime)."""
    n_zc = largest_prime_at_most(n_carriers)
    roots = [u for u in range(1, n_zc) if u != PILOT_ROOT]
    if len(roots) < N_HEADER_CODES:
        raise ValueError(f"only {len(roots)} usable roots for {n_carriers} carriers")
    # spread the chosen roots over the available range rather than taking the first 32
    step = len(roots) / N_HEADER_CODES
    return tuple(roots[int(i * step)] for i in range(N_HEADER_CODES))


def unique_word(n_carriers: int, root: int) -> ComplexArray:
    """Prime-length Zadoff–Chu cyclically extended to ``n_carriers`` (unit magnitude)."""
    n_zc = largest_prime_at_most(n_carriers)
    return zadoff_chu(n_zc, root)[np.arange(n_carriers) % n_zc]


class Preamble:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        n = self.cmap.n_carriers
        self.even = np.flatnonzero(self.cmap.bins % 2 == 0)
        self._roots = header_roots(n)
        sc = np.zeros(n, dtype=np.complex128)
        sc[self.even] = zadoff_chu(len(self.even), SC_ROOT) * math.sqrt(n / len(self.even))
        self._sc = sc
        self._uw = {code: unique_word(n, root) for code, root in enumerate(self._roots)}

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
