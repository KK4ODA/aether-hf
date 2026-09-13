"""Gray-labelled constellations with a vectorized max-log LLR demapper (roadmap P1-2).

Mappings follow the public definitions in 3GPP TS 38.211 §5.1 for QPSK, 16-QAM and 64-QAM
(bit-to-symbol formulas reproduced below), real ±1 for BPSK, and a Gray-coded circle for
8-PSK. All constellations have unit mean power. Bit order within a symbol is MSB-first:
symbol index ``k = Σ b_i · 2^(m-1-i)``.

LLR convention (used everywhere in Aether): **positive LLR means bit 0 is more likely**,
``LLR = log P(b=0|y) / P(b=1|y)``. With max-log and complex noise variance ``σ²``
(total, both quadratures): ``LLR_i = (min_{s∈S₁ᵢ}|y−s|² − min_{s∈S₀ᵢ}|y−s|²) / σ²``.
For BPSK this is the familiar ``4·Re(y)/σ²``.

This replaces ``dsp/modulation.py`` (1-D Gray code on a 2-D grid; O(M·m) Python loops).
"""

from __future__ import annotations

from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.waveform import Modulation

ComplexArray = NDArray[np.complex128]
BitArray = NDArray[np.uint8]
FloatArray = NDArray[np.float64]


def _bits_of(index: NDArray[np.integer], m: int) -> BitArray:
    """Index → (n, m) bit matrix, MSB first."""
    shifts = np.arange(m - 1, -1, -1)
    return ((index[:, None] >> shifts[None, :]) & 1).astype(np.uint8)


def _points(modulation: Modulation) -> ComplexArray:
    """Constellation point for every index 0 … 2^m − 1, following TS 38.211 §5.1."""
    m = modulation.bits_per_symbol
    b = _bits_of(np.arange(2**m), m).astype(np.float64)
    s = 1.0 - 2.0 * b  # bit 0 → +1, bit 1 → −1
    if modulation is Modulation.BPSK:
        return s[:, 0].astype(np.complex128)
    if modulation is Modulation.QPSK:  # 38.211 5.1.3
        return ((s[:, 0] + 1j * s[:, 1]) / np.sqrt(2.0)).astype(np.complex128)
    if modulation is Modulation.PSK8:
        # Gray-coded circle: point k sits at angle 2πk/8 and carries label gray(k) = k ^ (k>>1).
        k = np.arange(8)
        gray = k ^ (k >> 1)
        pts = np.exp(2j * np.pi * k / 8.0)
        out = np.empty(8, dtype=np.complex128)
        out[gray] = pts
        return out
    if modulation is Modulation.QAM16:  # 38.211 5.1.4
        i = s[:, 0] * (2.0 - s[:, 2])
        q = s[:, 1] * (2.0 - s[:, 3])
        return ((i + 1j * q) / np.sqrt(10.0)).astype(np.complex128)
    if modulation is Modulation.QAM64:  # 38.211 5.1.5
        i = s[:, 0] * (4.0 - s[:, 2] * (2.0 - s[:, 4]))
        q = s[:, 1] * (4.0 - s[:, 3] * (2.0 - s[:, 5]))
        return ((i + 1j * q) / np.sqrt(42.0)).astype(np.complex128)
    raise ValueError(f"unsupported modulation {modulation}")


class Constellation:
    """Mapper / demapper for one modulation. Instances are cheap and cached per modulation."""

    def __init__(self, modulation: Modulation) -> None:
        self.modulation = modulation
        self.bits_per_symbol = modulation.bits_per_symbol
        self.points = _points(modulation)
        m = self.bits_per_symbol
        labels = _bits_of(np.arange(len(self.points)), m)  # (M, m)
        # For every bit position i, the indices of the points whose bit i is 0 / 1.
        self._zero_idx = [np.flatnonzero(labels[:, i] == 0) for i in range(m)]
        self._one_idx = [np.flatnonzero(labels[:, i] == 1) for i in range(m)]

    # ── mapping ───────────────────────────────────────────────────────

    def map(self, bits: NDArray[np.integer]) -> ComplexArray:
        bits = np.asarray(bits)
        m = self.bits_per_symbol
        if bits.ndim != 1 or bits.size % m:
            raise ValueError(f"need a flat bit array whose length is a multiple of {m}")
        b = bits.reshape(-1, m).astype(np.int64)
        idx = (b << np.arange(m - 1, -1, -1)[None, :]).sum(axis=1)
        return self.points[idx]

    # ── demapping ─────────────────────────────────────────────────────

    def hard(self, symbols: ComplexArray) -> BitArray:
        """Nearest-point decisions → flat bit array (MSB first per symbol)."""
        y = np.asarray(symbols, dtype=np.complex128)
        idx = np.argmin(np.abs(y[:, None] - self.points[None, :]), axis=1)
        return _bits_of(idx, self.bits_per_symbol).reshape(-1)

    def llr(self, symbols: ComplexArray, noise_var: float | FloatArray) -> FloatArray:
        """Max-log LLRs, flat, ``m`` per symbol, positive = bit 0.

        ``noise_var`` is the total complex noise variance per symbol (scalar or per-symbol
        array, e.g. after per-carrier equalisation).
        """
        y = np.asarray(symbols, dtype=np.complex128)
        diff = y[:, None] - self.points[None, :]
        d2 = diff.real**2 + diff.imag**2  # (n, M)
        m = self.bits_per_symbol
        out = np.empty((len(y), m), dtype=np.float64)
        for i in range(m):
            out[:, i] = d2[:, self._one_idx[i]].min(axis=1) - d2[:, self._zero_idx[i]].min(axis=1)
        nv = np.asarray(noise_var, dtype=np.float64)
        if nv.ndim == 1:
            nv = nv[:, None]
        return (out / nv).reshape(-1)

    # ── properties ────────────────────────────────────────────────────

    @property
    def min_distance(self) -> float:
        d = np.abs(self.points[:, None] - self.points[None, :])
        np.fill_diagonal(d, np.inf)
        return float(d.min())

    def is_gray(self) -> bool:
        """True if every pair of nearest neighbours differs in exactly one bit."""
        d = np.abs(self.points[:, None] - self.points[None, :])
        np.fill_diagonal(d, np.inf)
        i, j = np.where(np.isclose(d, d.min()))
        return all(bin(int(a) ^ int(b)).count("1") == 1 for a, b in zip(i, j, strict=True))


@cache
def constellation(modulation: Modulation) -> Constellation:
    return Constellation(modulation)
