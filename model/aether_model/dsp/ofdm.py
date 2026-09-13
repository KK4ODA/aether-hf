"""
aether_model/dsp/ofdm.py

OFDM modulator and demodulator for AETHER HF.

Handles:
  - Subcarrier mapping (data + pilot placement)
  - IFFT/FFT with cyclic prefix
  - Raised-cosine windowing for spectral containment
  - Frequency-domain equalization
"""

import numpy as np

from aether_model.constants import (
    CP_DEFAULT_SAMPLES,
    CP_EXTENDED_SAMPLES,
    DATA_CARRIERS_N,
    DATA_CARRIERS_W,
    FFT_SIZE_W,
    PILOT_CARRIERS_N,
    PILOT_CARRIERS_W,
    RAISED_COSINE_BETA,
    TOTAL_CARRIERS_N,
    TOTAL_CARRIERS_W,
)


class SubcarrierMap:
    """Maps data and pilot symbols to FFT bins.

    The center of the passband is at FFT bin 0 (DC).  Useful subcarriers
    are placed symmetrically around the center, excluding DC.

    For wide mode (52 data + 12 pilot = 64 carriers):
      Bins: -32 to +31 (relative to center), excluding DC.
      Every 4th bin is a pilot; the rest are data.

    For narrow mode (8 data + 2 pilot = 10 carriers):
      Bins: -5 to +5, excluding DC.
    """

    def __init__(self, mode: str = "wide"):
        if mode == "wide":
            n_data = DATA_CARRIERS_W
            n_pilot = PILOT_CARRIERS_W
            n_total = TOTAL_CARRIERS_W
        else:
            n_data = DATA_CARRIERS_N
            n_pilot = PILOT_CARRIERS_N
            n_total = TOTAL_CARRIERS_N

        self.mode = mode
        self.fft_size = FFT_SIZE_W
        self.n_data = n_data
        self.n_pilot = n_pilot

        # Generate carrier indices (centered around DC)
        # We need exactly n_data + n_pilot carriers total
        half = n_total // 2
        carriers = list(range(-half, 0)) + list(range(1, half + 1))
        # Ensure we have exactly n_total carriers
        carriers = carriers[:n_total]

        # Assign pilots: pick n_pilot evenly-spaced positions
        pilot_positions = set()
        spacing = max(1, len(carriers) // n_pilot)
        for i in range(n_pilot):
            pilot_positions.add(min(i * spacing, len(carriers) - 1))

        self.pilot_indices = []
        self.data_indices = []
        for i, c in enumerate(carriers):
            if i in pilot_positions:
                self.pilot_indices.append(c)
            else:
                self.data_indices.append(c)

        # Safety trim (should be exact but just in case)
        self.pilot_indices = self.pilot_indices[:n_pilot]
        self.data_indices = self.data_indices[:n_data]

        # Map to FFT bin indices (positive representation)
        self.pilot_bins = [c % self.fft_size for c in self.pilot_indices]
        self.data_bins = [c % self.fft_size for c in self.data_indices]

        # Known pilot values (BPSK PN sequence, fixed per the spec)
        rng = np.random.RandomState(seed=42)  # deterministic
        self.pilot_values = (2 * rng.randint(0, 2, size=n_pilot) - 1).astype(np.complex128)


class OFDMModulator:
    """Generates OFDM symbols from data and pilot symbols."""

    def __init__(self, mode: str = "wide", extended_cp: bool = False):
        self.smap = SubcarrierMap(mode)
        self.fft_size = self.smap.fft_size
        self.cp_len = CP_EXTENDED_SAMPLES if extended_cp else CP_DEFAULT_SAMPLES

        # Raised-cosine window for spectral shaping
        beta = RAISED_COSINE_BETA
        n_taper = max(1, int(beta * self.fft_size))
        self._window = np.ones(self.fft_size + self.cp_len)
        taper = 0.5 * (1 - np.cos(np.pi * np.arange(n_taper) / n_taper))
        self._window[:n_taper] = taper
        self._window[-n_taper:] = taper[::-1]

    def modulate(self, data_symbols: np.ndarray, symbol_index: int = 0) -> np.ndarray:
        """Map data symbols to subcarriers and produce one OFDM symbol.

        Args:
            data_symbols: Complex array of length n_data (one QAM/PSK symbol per carrier).
            symbol_index: Used to determine pilot pattern in time.

        Returns:
            Time-domain samples (complex, length = fft_size + cp_len).
        """
        assert len(data_symbols) == self.smap.n_data, (
            f"Expected {self.smap.n_data} data symbols, got {len(data_symbols)}"
        )

        # Build frequency-domain OFDM symbol
        X = np.zeros(self.fft_size, dtype=np.complex128)

        # Place data
        for i, bin_idx in enumerate(self.smap.data_bins):
            X[bin_idx] = data_symbols[i]

        # Place pilots
        for i, bin_idx in enumerate(self.smap.pilot_bins):
            X[bin_idx] = self.smap.pilot_values[i]

        # IFFT → time domain
        x = np.fft.ifft(X) * np.sqrt(self.fft_size)  # normalize power

        # Add cyclic prefix
        cp = x[-self.cp_len :]
        x_cp = np.concatenate([cp, x])

        # Apply raised-cosine window
        x_cp *= self._window

        return x_cp

    def modulate_frame(self, data_symbols_2d: np.ndarray) -> np.ndarray:
        """Modulate multiple OFDM symbols into a continuous frame.

        Args:
            data_symbols_2d: Array of shape (n_symbols, n_data).

        Returns:
            Concatenated time-domain samples.
        """
        parts = []
        for sym_idx in range(data_symbols_2d.shape[0]):
            parts.append(self.modulate(data_symbols_2d[sym_idx], sym_idx))
        return np.concatenate(parts)


class OFDMDemodulator:
    """Demodulates received OFDM symbols."""

    def __init__(
        self,
        mode: str = "wide",
        extended_cp: bool = False,
        use_wiener: bool = False,
        use_blanker: bool = False,
        snr_est_db: float = 10.0,
    ):
        self.smap = SubcarrierMap(mode)
        self.fft_size = self.smap.fft_size
        self.cp_len = CP_EXTENDED_SAMPLES if extended_cp else CP_DEFAULT_SAMPLES
        self.symbol_len = self.fft_size + self.cp_len

        # Channel estimate (updated per symbol from pilots)
        self._H: np.ndarray | None = None

        # Optional Wiener channel estimator
        self._wiener = None
        if use_wiener:
            from aether_model.dsp.wiener import WienerEstimator

            self._wiener = WienerEstimator(
                pilot_freq_indices=np.array(self.smap.pilot_indices),
                data_freq_indices=np.array(self.smap.data_indices),
                fft_size=self.fft_size,
                snr_est_db=snr_est_db,
            )

        # Optional noise blanker
        self._blanker = None
        if use_blanker:
            from aether_model.dsp.noise_blanker import NoiseBlanker

            self._blanker = NoiseBlanker()

    def demodulate(self, samples: np.ndarray, symbol_index: int = 0) -> np.ndarray:
        """Demodulate one OFDM symbol.

        Args:
            samples: Time-domain samples of length symbol_len.
            symbol_index: For pilot pattern tracking.

        Returns:
            Complex data symbols (length n_data), equalized.
        """
        assert len(samples) == self.symbol_len

        # Apply noise blanker (Layer 1+2) before FFT
        if self._blanker:
            samples = self._blanker.process_time_domain(samples.copy())

        # Strip cyclic prefix
        x = samples[self.cp_len :]

        # FFT → frequency domain
        X = np.fft.fft(x) / np.sqrt(self.fft_size)

        # Extract pilots and estimate channel
        H_pilot = np.zeros(len(self.smap.pilot_bins), dtype=np.complex128)
        for i, bin_idx in enumerate(self.smap.pilot_bins):
            H_pilot[i] = X[bin_idx] / self.smap.pilot_values[i]

        # Channel estimation: Wiener or linear interpolation
        if self._wiener:
            self._H = self._wiener.estimate(H_pilot, symbol_index)
        else:
            self._H = self._interpolate_channel(H_pilot)

        # Erasure marking (Layer 3) — flag corrupted subcarriers
        erasure_mask = None
        if self._blanker:
            powers = np.array([np.abs(X[b]) ** 2 for b in self.smap.data_bins])
            erasure_mask = self._blanker.mark_erasures(powers)

        # Extract and equalize data subcarriers
        data_syms = np.zeros(self.smap.n_data, dtype=np.complex128)
        for i, bin_idx in enumerate(self.smap.data_bins):
            if erasure_mask is not None and erasure_mask[i]:
                # Erased — set to zero (LLR will be 0 for this position)
                data_syms[i] = 0.0
            elif self._H is not None and abs(self._H[i]) > 1e-10:
                data_syms[i] = X[bin_idx] / self._H[i]
            else:
                data_syms[i] = X[bin_idx]

        return data_syms

    def demodulate_frame(self, samples: np.ndarray, n_symbols: int) -> np.ndarray:
        """Demodulate multiple OFDM symbols.

        Returns:
            Array of shape (n_symbols, n_data).
        """
        result = []
        for i in range(n_symbols):
            start = i * self.symbol_len
            end = start + self.symbol_len
            result.append(self.demodulate(samples[start:end], i))
        return np.array(result)

    def _interpolate_channel(self, H_pilot: np.ndarray) -> np.ndarray:
        """Interpolate pilot channel estimates to data subcarrier positions.

        Simple linear interpolation for the prototype.
        Production would use 2D Wiener filtering.
        """
        n_data = self.smap.n_data
        n_pilot = len(H_pilot)

        if n_pilot < 2:
            # Not enough pilots — use mean
            return np.full(n_data, np.mean(H_pilot))

        # Build interpolated H for each data carrier
        # Use pilot positions as reference points
        pilot_freqs = np.array(self.smap.pilot_indices, dtype=float)
        data_freqs = np.array(self.smap.data_indices, dtype=float)

        H_data_real = np.interp(data_freqs, pilot_freqs, H_pilot.real)
        H_data_imag = np.interp(data_freqs, pilot_freqs, H_pilot.imag)

        return H_data_real + 1j * H_data_imag
