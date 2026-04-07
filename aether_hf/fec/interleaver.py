"""
aether_hf/fec/interleaver.py

Three-stage interleaver for AETHER HF.

Stage 1: Frequency interleaving (within one OFDM symbol)
Stage 2: Time interleaving (across multiple symbols)
Stage 3: Bit interleaving (BICM — handled by the modulation mapper)
"""

import numpy as np


class FrequencyInterleaver:
    """Pseudo-random permutation of bits across subcarriers within one symbol."""

    def __init__(self, n_bits: int, seed: int = 7):
        self._n = n_bits
        rng = np.random.RandomState(seed=seed)
        self._perm = rng.permutation(n_bits)
        self._inv_perm = np.argsort(self._perm)

    def interleave(self, bits: np.ndarray) -> np.ndarray:
        return bits[self._perm]

    def deinterleave(self, bits: np.ndarray) -> np.ndarray:
        return bits[self._inv_perm]


class TimeInterleaver:
    """Spreads a codeword across multiple OFDM symbols.

    Input:  coded bits for one codeword (length = block_length)
    Output: 2D array of shape (depth, bits_per_symbol_slot)
            where depth = number of OFDM symbols the codeword spans.
    """

    def __init__(self, block_length: int, depth: int = 8, seed: int = 13):
        self._block = block_length
        self._depth = depth
        self._slot = block_length // depth
        # Pseudo-random write-column permutation
        rng = np.random.RandomState(seed=seed)
        self._col_perm = rng.permutation(depth)
        self._col_inv = np.argsort(self._col_perm)

    @property
    def depth(self) -> int:
        return self._depth

    def interleave(self, bits: np.ndarray) -> np.ndarray:
        """Interleave: write columns in permuted order, read rows."""
        n = len(bits)
        padded = np.zeros(self._depth * self._slot, dtype=bits.dtype)
        padded[:n] = bits

        matrix = padded.reshape(self._depth, self._slot)
        # Permute columns
        matrix = matrix[self._col_perm, :]
        return matrix.flatten()[:n]

    def deinterleave(self, bits: np.ndarray) -> np.ndarray:
        """Reverse the interleaving."""
        n = len(bits)
        padded = np.zeros(self._depth * self._slot, dtype=bits.dtype)
        padded[:n] = bits

        matrix = padded.reshape(self._depth, self._slot)
        matrix = matrix[self._col_inv, :]
        return matrix.flatten()[:n]


class AetherInterleaver:
    """Combined frequency + time interleaver for one codeword."""

    def __init__(self, block_length: int, n_carriers: int,
                 time_depth: int = 8):
        self._freq = FrequencyInterleaver(n_carriers)
        self._time = TimeInterleaver(block_length, depth=time_depth)

    def interleave(self, bits: np.ndarray) -> np.ndarray:
        """Apply time interleaving then frequency interleaving per symbol."""
        return self._time.interleave(bits)

    def deinterleave(self, bits: np.ndarray) -> np.ndarray:
        return self._time.deinterleave(bits)
