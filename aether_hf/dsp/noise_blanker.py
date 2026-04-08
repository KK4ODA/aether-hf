"""
aether_hf/dsp/noise_blanker.py

Three-layer impulsive noise defense (Section 6.1 of the spec).

Layer 1: Time-domain blanking — zero samples exceeding adaptive threshold
Layer 2: Iterative clipping and filtering — suppress residual impulses
Layer 3: Erasure marking — flag corrupted subcarriers for the LDPC decoder

Operates on complex baseband samples before the OFDM FFT.
"""

import numpy as np
from typing import Optional

from aether_hf.constants import BASEBAND_RATE, FFT_SIZE_W


class NoiseBlanker:
    """Three-layer impulsive noise defense for OFDM reception."""

    def __init__(
        self,
        threshold_factor: float = 3.0,
        rms_window_ms: float = 100.0,
        clip_iterations: int = 3,
        erasure_threshold_db: float = 10.0,
        sample_rate: float = BASEBAND_RATE,
    ):
        """
        Args:
            threshold_factor: Blanking threshold as multiple of RMS (default 3x).
            rms_window_ms: Sliding window for RMS estimation (ms).
            clip_iterations: Number of clip-and-filter iterations (Layer 2).
            erasure_threshold_db: Per-subcarrier power threshold for erasure
                marking (dB above local average).
            sample_rate: Baseband sample rate.
        """
        self._threshold = threshold_factor
        self._rms_window = int(rms_window_ms * sample_rate / 1000)
        self._clip_iters = clip_iterations
        self._erasure_thresh_db = erasure_threshold_db
        self._fs = sample_rate

        # Statistics
        self.blanked_count = 0
        self.erasure_count = 0

    def process_time_domain(self, samples: np.ndarray) -> np.ndarray:
        """Apply Layer 1 (blanking) and Layer 2 (clip-and-filter).

        Args:
            samples: Complex baseband samples (before FFT).

        Returns:
            Cleaned samples.
        """
        out = samples.copy()

        # ── Layer 1: Time-domain blanking ─────────────────────────────
        out = self._blank(out)

        # ── Layer 2: Iterative clipping and filtering ─────────────────
        out = self._clip_and_filter(out)

        return out

    def mark_erasures(self, subcarrier_powers: np.ndarray,
                      n_neighbors: int = 5) -> np.ndarray:
        """Layer 3: Mark corrupted subcarriers as erasures.

        Args:
            subcarrier_powers: Power (|X[k]|^2) of each subcarrier after FFT.
            n_neighbors: Number of neighbors for local average.

        Returns:
            Boolean mask — True = erasure (unreliable subcarrier).
        """
        n = len(subcarrier_powers)
        erasures = np.zeros(n, dtype=bool)

        # Compute local average power for each subcarrier
        half_win = n_neighbors // 2
        for i in range(n):
            lo = max(0, i - half_win)
            hi = min(n, i + half_win + 1)
            neighbors = np.concatenate([
                subcarrier_powers[lo:i],
                subcarrier_powers[i+1:hi],
            ])
            if len(neighbors) == 0:
                continue
            local_avg = np.mean(neighbors)
            if local_avg <= 0:
                continue

            ratio_db = 10 * np.log10(
                subcarrier_powers[i] / local_avg + 1e-30
            )
            if ratio_db > self._erasure_thresh_db:
                erasures[i] = True

        self.erasure_count = int(np.sum(erasures))
        return erasures

    # ── Layer 1: Blanking ─────────────────────────────────────────────

    def _blank(self, samples: np.ndarray) -> np.ndarray:
        """Zero samples that exceed threshold_factor * RMS."""
        n = len(samples)
        amplitudes = np.abs(samples)

        # Compute adaptive RMS using sliding window
        window = min(self._rms_window, n)
        if window < 1:
            return samples

        # Use cumulative sum for efficient windowed RMS
        amp_sq = amplitudes ** 2
        cumsum = np.cumsum(amp_sq)
        # Windowed mean of squared amplitudes
        windowed_mean = np.zeros(n)
        windowed_mean[:window] = cumsum[:window] / np.arange(1, window + 1)
        windowed_mean[window:] = (
            cumsum[window:] - cumsum[:-window]
        ) / window
        rms = np.sqrt(windowed_mean + 1e-30)

        # Blank samples exceeding threshold
        threshold = self._threshold * rms
        mask = amplitudes > threshold
        samples[mask] = 0.0
        self.blanked_count = int(np.sum(mask))

        return samples

    # ── Layer 2: Iterative clip and filter ────────────────────────────

    def _clip_and_filter(self, samples: np.ndarray) -> np.ndarray:
        """Alternate between time-domain clipping and frequency-domain
        filtering to suppress residual impulsive energy.
        """
        n = len(samples)

        for _ in range(self._clip_iters):
            # Clip: limit amplitude to threshold * RMS
            rms = np.sqrt(np.mean(np.abs(samples) ** 2) + 1e-30)
            clip_level = self._threshold * rms
            amplitudes = np.abs(samples)
            over = amplitudes > clip_level
            if np.any(over):
                # Soft clip: scale down rather than hard zero
                scale = np.ones(n)
                scale[over] = clip_level / amplitudes[over]
                samples = samples * scale

            # Filter: zero out-of-band components in frequency domain
            # This removes energy splattered outside the passband by clipping
            X = np.fft.fft(samples)
            # Keep only the subcarriers that should have signal
            # (center ±half_carriers)
            half_bw_bins = FFT_SIZE_W // 4  # roughly half the used bandwidth
            mask = np.zeros(n, dtype=bool)
            if n >= FFT_SIZE_W:
                mask[:half_bw_bins] = True
                mask[-half_bw_bins:] = True
            else:
                mask[:] = True  # short segment, keep all
            X[~mask] = 0
            samples = np.fft.ifft(X)

        return samples
