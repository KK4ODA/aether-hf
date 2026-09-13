"""
aether_model/dsp/modulation.py

Constellation mappers and demappers for all AETHER HF modulation schemes.
Supports both hard and soft (LLR) demapping.
"""

import numpy as np

from aether_model.speed_levels import BITS_PER_SYMBOL, Modulation


def _gray_code(n_bits: int) -> np.ndarray:
    """Generate Gray-coded bit patterns for n_bits."""
    n = 2**n_bits
    return np.array([i ^ (i >> 1) for i in range(n)])


# ── Constellation generation ──────────────────────────────────────────


def _bpsk_constellation() -> np.ndarray:
    return np.array([-1.0 + 0j, 1.0 + 0j])


def _qpsk_constellation() -> np.ndarray:
    s = 1.0 / np.sqrt(2)
    return np.array([-s - s * 1j, -s + s * 1j, s - s * 1j, s + s * 1j])


def _psk8_constellation() -> np.ndarray:
    angles = np.pi / 8 + np.arange(8) * np.pi / 4
    return np.exp(1j * angles)


def _qam_constellation(order: int) -> np.ndarray:
    """Generate a square QAM constellation (Gray-coded)."""
    side = int(np.sqrt(order))
    assert side * side == order, f"{order}-QAM requires perfect square"

    # Generate grid points
    coords = np.arange(side) - (side - 1) / 2.0
    points = []
    for q in coords:
        for i in coords:
            points.append(i + 1j * q)
    constellation = np.array(points)

    # Normalize to unit average power
    power = np.mean(np.abs(constellation) ** 2)
    constellation /= np.sqrt(power)

    # Gray-code reordering
    gray = _gray_code(int(np.log2(order)))
    reordered = np.zeros_like(constellation)
    for i, g in enumerate(gray):
        if g < len(constellation):
            reordered[g] = constellation[i]
    return reordered


# Pre-compute all constellations
CONSTELLATIONS = {
    Modulation.BPSK: _bpsk_constellation(),
    Modulation.QPSK: _qpsk_constellation(),
    Modulation.PSK8: _psk8_constellation(),
    Modulation.QAM16: _qam_constellation(16),
    Modulation.QAM32: _qam_constellation(32) if int(np.sqrt(32)) ** 2 == 32 else None,
    Modulation.QAM64: _qam_constellation(64),
    Modulation.QAM128: None,  # Cross-QAM, handled separately
    Modulation.QAM256: _qam_constellation(256),
}


# 32-QAM is cross-shaped, not square. Use a custom constellation.
def _qam32_constellation() -> np.ndarray:
    """Generate 32-QAM cross constellation."""
    # 32-QAM uses a cross pattern: 6x6 grid minus 4 corners
    side = 6
    coords = np.arange(side) - (side - 1) / 2.0
    points = []
    for q in coords:
        for i in coords:
            # Skip corner points to get 32 from 36
            if abs(i) > 1.5 and abs(q) > 1.5:
                continue
            points.append(i + 1j * q)
    constellation = np.array(points[:32])
    power = np.mean(np.abs(constellation) ** 2)
    constellation /= np.sqrt(power)
    return constellation


CONSTELLATIONS[Modulation.QAM32] = _qam32_constellation()


# 128-QAM: use cross pattern from 12x12 grid
def _qam128_constellation() -> np.ndarray:
    side = 12
    coords = np.arange(side) - (side - 1) / 2.0
    points = []
    for q in coords:
        for i in coords:
            if abs(i) > 4.5 and abs(q) > 4.5:
                continue
            points.append(i + 1j * q)
    constellation = np.array(sorted(points, key=lambda x: abs(x))[:128])
    power = np.mean(np.abs(constellation) ** 2)
    constellation /= np.sqrt(power)
    return constellation


CONSTELLATIONS[Modulation.QAM128] = _qam128_constellation()


# ── Mapper ────────────────────────────────────────────────────────────


class Mapper:
    """Maps bit sequences to constellation symbols."""

    def __init__(self, modulation: Modulation):
        self.mod = modulation
        self.bps = BITS_PER_SYMBOL[modulation]
        self.constellation = CONSTELLATIONS.get(modulation)
        if self.constellation is None:
            raise ValueError(f"No constellation defined for {modulation}")

    def map(self, bits: np.ndarray) -> np.ndarray:
        """Map bits to complex symbols.

        Args:
            bits: Binary array, length must be multiple of bits_per_symbol.

        Returns:
            Complex symbol array.
        """
        assert len(bits) % self.bps == 0
        n_symbols = len(bits) // self.bps

        symbols = np.zeros(n_symbols, dtype=np.complex128)
        for i in range(n_symbols):
            idx = 0
            for b in range(self.bps):
                idx = (idx << 1) | int(bits[i * self.bps + b])
            symbols[i] = self.constellation[idx % len(self.constellation)]

        return symbols


# ── Demapper ──────────────────────────────────────────────────────────


class Demapper:
    """Demaps received symbols to bits or soft LLR values."""

    def __init__(self, modulation: Modulation):
        self.mod = modulation
        self.bps = BITS_PER_SYMBOL[modulation]
        self.constellation = CONSTELLATIONS.get(modulation)
        if self.constellation is None:
            raise ValueError(f"No constellation defined for {modulation}")

    def hard_demap(self, symbols: np.ndarray) -> np.ndarray:
        """Hard decision: find nearest constellation point, return bits."""
        bits = []
        for sym in symbols:
            distances = np.abs(sym - self.constellation)
            idx = np.argmin(distances)
            for b in range(self.bps - 1, -1, -1):
                bits.append((idx >> b) & 1)
        return np.array(bits, dtype=np.int8)

    def soft_demap(self, symbols: np.ndarray, noise_var: float = 1.0) -> np.ndarray:
        """Soft demapping: compute log-likelihood ratios (LLRs).

        Positive LLR = bit more likely 0.
        Negative LLR = bit more likely 1.
        """
        n_points = len(self.constellation)
        llrs = []

        for sym in symbols:
            distances = np.abs(sym - self.constellation) ** 2

            for bit_pos in range(self.bps):
                # Find min distance for bit=0 and bit=1 at this position
                mask_bit = 1 << (self.bps - 1 - bit_pos)
                d_min_0 = np.inf
                d_min_1 = np.inf

                for idx in range(n_points):
                    if idx & mask_bit:
                        d_min_1 = min(d_min_1, distances[idx])
                    else:
                        d_min_0 = min(d_min_0, distances[idx])

                # LLR = (d_min_1 - d_min_0) / noise_var
                llr = (d_min_1 - d_min_0) / max(noise_var, 1e-10)
                llrs.append(llr)

        return np.array(llrs)
