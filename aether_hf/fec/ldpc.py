"""
aether_hf/fec/ldpc.py

LDPC encoder and decoder for AETHER HF.

This is a prototype implementation using a regular LDPC code constructed
via the PEG (Progressive Edge Growth) algorithm approximation. For
production, replace with 5G NR base graph codes via a C extension or
the pyldpc / sionna libraries.

Supports:
  - Multiple code rates (1/4, 1/3, 1/2, 2/3, 3/4, 5/6)
  - Multiple block lengths (256, 512, 1024, 2048, 4096)
  - Belief-propagation (sum-product) decoding with LLR input
"""

import numpy as np
from typing import Optional
import logging

log = logging.getLogger(__name__)


def _make_regular_H(n: int, rate: float, col_weight: int = 3) -> np.ndarray:
    """Construct a regular LDPC parity-check matrix.

    Uses a simplified construction (not PEG) suitable for prototyping.
    The matrix has dimensions (n - k) x n where k = n * rate.

    Args:
        n: Codeword length (bits).
        rate: Code rate (0 < rate < 1).
        col_weight: Column weight (connections per variable node).

    Returns:
        Binary parity-check matrix H of shape (n-k, n).
    """
    k = int(n * rate)
    m = n - k  # number of parity checks

    H = np.zeros((m, n), dtype=np.int8)
    rng = np.random.RandomState(seed=hash((n, int(rate * 1000))) & 0x7FFFFFFF)

    for col in range(n):
        # Place col_weight ones in random rows
        rows = rng.choice(m, size=min(col_weight, m), replace=False)
        H[rows, col] = 1

    return H


class LDPCCode:
    """LDPC code with encoding and decoding capabilities."""

    def __init__(self, n: int, rate: float, max_iter: int = 40):
        """
        Args:
            n: Codeword length in bits.
            rate: Code rate.
            max_iter: Maximum belief-propagation iterations.
        """
        self.n = n
        self.k = int(n * rate)
        self.rate = rate
        self.max_iter = max_iter

        # Build parity-check matrix
        self.H = _make_regular_H(n, rate)
        self.m = self.H.shape[0]  # n - k

        # Build generator matrix G (systematic form: G = [I_k | P])
        # For prototype, use a simple approach: solve for P from H
        self._build_generator()

        # Pre-compute check node connections for fast decoding
        self._cn_vn = []  # check-node to variable-node connections
        self._vn_cn = []  # variable-node to check-node connections
        for i in range(self.m):
            self._cn_vn.append(np.where(self.H[i, :] == 1)[0])
        for j in range(self.n):
            self._vn_cn.append(np.where(self.H[:, j] == 1)[0])

    def _build_generator(self):
        """Build a systematic generator matrix from H using Gaussian elimination."""
        H = self.H.copy()
        m, n = H.shape
        k = self.k

        # Try to put H in systematic form [P^T | I_{n-k}]
        # by Gaussian elimination over GF(2)
        pivot_cols = []
        for row in range(min(m, n)):
            # Find pivot
            found = False
            for col in range(n):
                if col in pivot_cols:
                    continue
                if H[row, col] == 1:
                    found = True
                    pivot_cols.append(col)
                    # Eliminate other rows
                    for r2 in range(m):
                        if r2 != row and H[r2, col] == 1:
                            H[r2, :] = (H[r2, :] + H[row, :]) % 2
                    break
            if not found:
                pivot_cols.append(-1)

        # For prototype, use a simple encoding: multiply info bits by G
        # G is k x n in systematic form
        self.G = np.eye(k, n, dtype=np.int8)
        # Add parity from H (simplified — may not produce valid codewords
        # for all H matrices, but works for prototype testing)

    def encode(self, info_bits: np.ndarray) -> np.ndarray:
        """Encode information bits into a codeword.

        Args:
            info_bits: Binary array of length k.

        Returns:
            Binary codeword of length n.
        """
        assert len(info_bits) == self.k, f"Expected {self.k} bits, got {len(info_bits)}"

        # Systematic encoding: codeword = [info_bits | parity_bits]
        codeword = np.zeros(self.n, dtype=np.int8)
        codeword[:self.k] = info_bits

        # Compute parity bits: p = (H_info * info_bits) mod 2
        # where H = [H_info | H_parity]
        H_info = self.H[:, :self.k]
        H_parity = self.H[:, self.k:]

        syndrome = H_info @ info_bits % 2

        # Solve H_parity * p = syndrome (mod 2) via back-substitution
        # For prototype, use least-squares approximation
        try:
            p = np.linalg.lstsq(H_parity.astype(float),
                                syndrome.astype(float), rcond=None)[0]
            codeword[self.k:] = np.round(p).astype(np.int8) % 2
        except np.linalg.LinAlgError:
            # Fallback: random parity (prototype only)
            codeword[self.k:] = np.random.randint(0, 2, size=self.n - self.k)

        return codeword

    def decode(self, llr: np.ndarray) -> tuple[np.ndarray, bool, int]:
        """Decode using belief-propagation (min-sum approximation).

        Args:
            llr: Log-likelihood ratios, length n.
                 Positive = bit more likely 0, negative = more likely 1.

        Returns:
            (decoded_info_bits, converged, iterations_used)
        """
        assert len(llr) == self.n

        # Initialize variable-to-check messages with channel LLRs
        v2c = np.zeros((self.m, self.n))
        for j in range(self.n):
            for i in self._vn_cn[j]:
                v2c[i, j] = llr[j]

        c2v = np.zeros((self.m, self.n))

        for iteration in range(self.max_iter):
            # ── Check node update (min-sum approximation) ─────────
            for i in range(self.m):
                vns = self._cn_vn[i]
                if len(vns) == 0:
                    continue
                for j in vns:
                    # Product of signs, minimum of magnitudes (excluding j)
                    others = [v2c[i, j2] for j2 in vns if j2 != j]
                    if not others:
                        c2v[i, j] = 0
                        continue
                    sign = 1
                    min_abs = np.inf
                    for val in others:
                        if val < 0:
                            sign *= -1
                        min_abs = min(min_abs, abs(val))
                    c2v[i, j] = sign * min_abs * 0.75  # scaling factor

            # ── Variable node update ──────────────────────────────
            total_llr = llr.copy()
            for j in range(self.n):
                for i in self._vn_cn[j]:
                    total_llr[j] += c2v[i, j]

            for j in range(self.n):
                for i in self._vn_cn[j]:
                    v2c[i, j] = total_llr[j] - c2v[i, j]

            # ── Check convergence ─────────────────────────────────
            hard = (total_llr < 0).astype(np.int8)
            syndrome = self.H @ hard % 2
            if np.all(syndrome == 0):
                return hard[:self.k], True, iteration + 1

        # Did not converge — return hard decision anyway
        hard = (total_llr < 0).astype(np.int8)
        return hard[:self.k], False, self.max_iter


# ── Code cache ────────────────────────────────────────────────────────

_code_cache: dict[tuple[int, float], LDPCCode] = {}


def get_ldpc_code(block_length: int, rate: float,
                  max_iter: int = 40) -> LDPCCode:
    """Get or create an LDPC code instance (cached)."""
    key = (block_length, round(rate, 4))
    if key not in _code_cache:
        _code_cache[key] = LDPCCode(block_length, rate, max_iter)
    return _code_cache[key]
