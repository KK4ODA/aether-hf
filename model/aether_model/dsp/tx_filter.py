"""
aether_model/dsp/tx_filter.py

Transmit bandpass filter for spectrum mask compliance.

Applies a steep FIR bandpass filter to the OFDM signal before
transmission to meet the out-of-band emission limits:
  -30 dBc at ±BW
  -50 dBc at ±2×BW
  -60 dBc at ±3×BW
"""

import numpy as np
from scipy.signal import firwin, lfilter

from aether_model.constants import BASEBAND_RATE


class TxFilter:
    """FIR bandpass filter for transmit spectrum shaping."""

    def __init__(
        self,
        bandwidth_hz: float = 2300.0,
        sample_rate: float = BASEBAND_RATE,
        num_taps: int = 127,
        guard_hz: float = 100.0,
    ):
        """
        Args:
            bandwidth_hz: Occupied signal bandwidth (Hz).
            sample_rate: Baseband sample rate.
            num_taps: FIR filter length (odd number, higher = steeper).
            guard_hz: Guard band between signal edge and filter cutoff.
        """
        self._fs = sample_rate
        self._bw = bandwidth_hz

        # Lowpass cutoff: half the bandwidth plus a small guard
        cutoff_hz = bandwidth_hz / 2.0 + guard_hz

        # Design a lowpass FIR filter
        # For complex baseband, we use a lowpass centered at DC
        nyquist = sample_rate / 2.0
        normalized_cutoff = cutoff_hz / nyquist

        # Clamp to valid range
        normalized_cutoff = min(normalized_cutoff, 0.99)

        self._taps = firwin(
            num_taps,
            normalized_cutoff,
            window="blackmanharris",  # steep rolloff
        )

        # Normalize to unity passband gain
        self._taps /= np.sum(self._taps)

    def filter(self, samples: np.ndarray) -> np.ndarray:
        """Apply the TX filter to baseband samples.

        Filters both real and imaginary parts independently.

        Returns filtered samples (same length, compensated for group delay).
        """
        # Filter real and imaginary parts separately
        filtered_real = lfilter(self._taps, 1.0, samples.real)
        filtered_imag = lfilter(self._taps, 1.0, samples.imag)
        filtered = filtered_real + 1j * filtered_imag

        # Compensate for group delay (half the filter length)
        delay = len(self._taps) // 2
        # Shift output to align with input
        result = np.zeros_like(samples)
        if delay < len(filtered):
            result[: len(filtered) - delay] = filtered[delay:]

        return result


class TxFilterNarrow(TxFilter):
    """TX filter for 500 Hz narrow mode."""

    def __init__(self, sample_rate: float = BASEBAND_RATE):
        super().__init__(
            bandwidth_hz=500.0,
            sample_rate=sample_rate,
            num_taps=255,  # more taps for narrower transition band
            guard_hz=50.0,
        )
