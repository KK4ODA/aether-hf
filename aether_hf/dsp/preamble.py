"""
aether_hf/dsp/preamble.py

Preamble generation and detection for AETHER HF.

Parts:
  A — Zadoff-Chu detection sequence (2 symbols)
  B — Fine sync OFDM symbol (BPSK PN on all subcarriers)
  C — Channel estimation OFDM symbol (QPSK training pattern)
"""

import numpy as np
from aether_hf.constants import (
    ZC_LENGTH, ZC_ROOT, BASEBAND_RATE, FFT_SIZE_W,
    CP_DEFAULT_SAMPLES, CFO_SWEEP_RANGE_HZ, CFO_SWEEP_STEP_HZ,
)


def zadoff_chu(length: int, root: int) -> np.ndarray:
    """Generate a Zadoff-Chu sequence of given length and root index."""
    n = np.arange(length)
    if length % 2 == 0:
        zc = np.exp(-1j * np.pi * root * n * (n + 1) / length)
    else:
        zc = np.exp(-1j * np.pi * root * n * (n + 2) / (length + 1))
    return zc


class PreambleGenerator:
    """Generates the AETHER HF preamble for transmission."""

    def __init__(self, mode: str = "wide"):
        self.mode = mode
        self.fft_size = FFT_SIZE_W
        self.cp_len = CP_DEFAULT_SAMPLES

        # Part A: Zadoff-Chu sequence (repeated twice)
        self.zc = zadoff_chu(ZC_LENGTH, ZC_ROOT)

        # Part B: BPSK PN sequence for fine sync
        rng = np.random.RandomState(seed=123)
        self.sync_pn = 2 * rng.randint(0, 2, size=self.fft_size) - 1

        # Part C: QPSK training pattern for channel estimation
        rng2 = np.random.RandomState(seed=456)
        bits = rng2.randint(0, 4, size=self.fft_size)
        angles = bits * np.pi / 2 + np.pi / 4
        self.train_qpsk = np.exp(1j * angles)

    def generate(self) -> np.ndarray:
        """Generate the full 4-symbol preamble.

        Returns time-domain baseband samples.
        """
        parts = []

        # Part A: ZC sequence, zero-padded to symbol length, repeated
        zc_padded = np.zeros(self.fft_size, dtype=np.complex128)
        zc_padded[:ZC_LENGTH] = self.zc
        zc_td = np.fft.ifft(zc_padded) * np.sqrt(self.fft_size)
        zc_cp = np.concatenate([zc_td[-self.cp_len:], zc_td])
        parts.append(zc_cp)  # symbol 1
        parts.append(zc_cp)  # symbol 2 (repeat)

        # Part B: Fine sync OFDM symbol
        X_sync = np.zeros(self.fft_size, dtype=np.complex128)
        X_sync[:] = self.sync_pn
        sync_td = np.fft.ifft(X_sync) * np.sqrt(self.fft_size)
        sync_cp = np.concatenate([sync_td[-self.cp_len:], sync_td])
        parts.append(sync_cp)

        # Part C: Channel estimation OFDM symbol
        X_train = np.zeros(self.fft_size, dtype=np.complex128)
        X_train[:] = self.train_qpsk
        train_td = np.fft.ifft(X_train) * np.sqrt(self.fft_size)
        train_cp = np.concatenate([train_td[-self.cp_len:], train_td])
        parts.append(train_cp)

        return np.concatenate(parts)

    def generate_short(self) -> np.ndarray:
        """Generate shortened 2-symbol preamble for ACK frames (Part A only)."""
        zc_padded = np.zeros(self.fft_size, dtype=np.complex128)
        zc_padded[:ZC_LENGTH] = self.zc
        zc_td = np.fft.ifft(zc_padded) * np.sqrt(self.fft_size)
        zc_cp = np.concatenate([zc_td[-self.cp_len:], zc_td])
        return np.concatenate([zc_cp, zc_cp])


class PreambleDetector:
    """Detects AETHER HF preambles in received audio with coarse CFO sweep."""

    def __init__(self, mode: str = "wide"):
        self.fft_size = FFT_SIZE_W
        self.cp_len = CP_DEFAULT_SAMPLES
        self.symbol_len = self.fft_size + self.cp_len
        self.zc = zadoff_chu(ZC_LENGTH, ZC_ROOT)

        # Build matched filter (conjugate of ZC in frequency domain)
        zc_padded = np.zeros(self.fft_size, dtype=np.complex128)
        zc_padded[:ZC_LENGTH] = self.zc
        self._zc_td = np.fft.ifft(zc_padded) * np.sqrt(self.fft_size)

        # Pre-compute frequency shift vectors for CFO sweep
        self._cfo_hypotheses = np.arange(
            -CFO_SWEEP_RANGE_HZ,
            CFO_SWEEP_RANGE_HZ + CFO_SWEEP_STEP_HZ / 2,
            CFO_SWEEP_STEP_HZ,
        )
        t = np.arange(self.symbol_len) / BASEBAND_RATE
        self._shift_vectors = [
            np.exp(-2j * np.pi * f * t) for f in self._cfo_hypotheses
        ]

    def detect(self, samples: np.ndarray,
               threshold: float = 0.5) -> tuple[bool, int, float]:
        """Detect preamble in received samples with CFO sweep.

        Args:
            samples: Received baseband samples.
            threshold: Normalized correlation threshold (0-1).

        Returns:
            (detected, sample_offset, estimated_cfo_hz)
        """
        if len(samples) < 2 * self.symbol_len:
            return False, 0, 0.0

        best_corr = 0.0
        best_offset = 0
        best_cfo = 0.0

        # Slide over the input and try each CFO hypothesis
        search_len = len(samples) - 2 * self.symbol_len
        step = max(1, self.symbol_len // 4)  # coarse search step

        for cfo_idx, shift_vec in enumerate(self._shift_vectors):
            for offset in range(0, search_len, step):
                # Extract two consecutive symbol-length chunks
                seg1 = samples[offset:offset + self.symbol_len]

                # Apply CFO correction
                seg1_corrected = seg1 * shift_vec[:len(seg1)]

                # Correlate with known ZC
                corr = np.abs(np.correlate(
                    seg1_corrected[self.cp_len:self.cp_len + self.fft_size],
                    self._zc_td,
                    mode='valid',
                ))
                if len(corr) == 0:
                    continue

                peak = np.max(corr)
                # Normalize
                sig_power = np.sqrt(np.sum(np.abs(seg1_corrected) ** 2))
                ref_power = np.sqrt(np.sum(np.abs(self._zc_td) ** 2))
                norm_corr = peak / max(sig_power * ref_power, 1e-10) * self.fft_size

                if norm_corr > best_corr:
                    best_corr = norm_corr
                    best_offset = offset
                    best_cfo = self._cfo_hypotheses[cfo_idx]

        detected = best_corr >= threshold
        return detected, best_offset, best_cfo

    def estimate_fine_cfo(self, samples: np.ndarray,
                          offset: int, coarse_cfo: float) -> float:
        """Estimate fine CFO from the repeated ZC symbols (Part A).

        Uses the phase difference between the two ZC repetitions.
        """
        sym1_start = offset + self.cp_len
        sym2_start = offset + self.symbol_len + self.cp_len

        seg1 = samples[sym1_start:sym1_start + self.fft_size]
        seg2 = samples[sym2_start:sym2_start + self.fft_size]

        # Correct coarse CFO
        t1 = np.arange(len(seg1)) / BASEBAND_RATE
        t2 = np.arange(len(seg2)) / BASEBAND_RATE + self.symbol_len / BASEBAND_RATE
        seg1 *= np.exp(-2j * np.pi * coarse_cfo * t1)
        seg2 *= np.exp(-2j * np.pi * coarse_cfo * t2)

        # Phase difference between repeated symbols
        correlation = np.sum(seg2 * np.conj(seg1))
        phase_diff = np.angle(correlation)

        # CFO = phase_diff / (2 * pi * T_symbol)
        T_symbol = self.symbol_len / BASEBAND_RATE
        fine_cfo = phase_diff / (2 * np.pi * T_symbol)

        return coarse_cfo + fine_cfo
