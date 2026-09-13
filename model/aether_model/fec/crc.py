"""Cyclic redundancy checks on bit arrays (MSB-first), using the public 3GPP polynomials.

TS 38.212 §5.1: CRC24A ``0x864CFB``, CRC24B ``0x800063``, CRC24C ``0xB2B117``, CRC16
``0x1021``, CRC11 ``0x621``, CRC6 ``0x21``. Initial register 0, no final XOR, no reflection
— i.e. the parity bits are the remainder of ``a(x)·x^L`` divided by ``g(x)``.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

BitArray = NDArray[np.uint8]


@dataclass(frozen=True)
class Crc:
    name: str
    poly: int
    """Generator polynomial without the leading x^L term."""
    width: int

    def remainder(self, bits: NDArray[np.integer]) -> BitArray:
        """CRC parity bits (``width`` of them) for ``bits``."""
        b = np.asarray(bits, dtype=np.uint8)
        reg = 0
        top = 1 << self.width
        mask = top - 1
        poly = self.poly
        for bit in b.tolist():  # ~1e6 bits/s; fine for HF frame sizes
            reg = ((reg << 1) | bit) & (mask | top)
            if reg & top:
                reg = (reg ^ top) ^ poly
        # flush width zeros
        for _ in range(self.width):
            reg = (reg << 1) & (mask | top)
            if reg & top:
                reg = (reg ^ top) ^ poly
        reg &= mask
        return np.array(
            [(reg >> (self.width - 1 - i)) & 1 for i in range(self.width)], dtype=np.uint8
        )

    def attach(self, bits: NDArray[np.integer]) -> BitArray:
        b = np.asarray(bits, dtype=np.uint8)
        return np.concatenate((b, self.remainder(b)))

    def check(self, bits_with_crc: NDArray[np.integer]) -> bool:
        b = np.asarray(bits_with_crc, dtype=np.uint8)
        if len(b) < self.width:
            return False
        return bool(np.array_equal(self.remainder(b[: -self.width]), b[-self.width :]))


CRC24A = Crc("CRC24A", 0x864CFB, 24)
CRC24B = Crc("CRC24B", 0x800063, 24)
CRC24C = Crc("CRC24C", 0xB2B117, 24)
CRC16 = Crc("CRC16", 0x1021, 16)
CRC11 = Crc("CRC11", 0x621, 11)
CRC6 = Crc("CRC6", 0x21, 6)
