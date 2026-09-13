"""Peak-to-average power ratio: measurement and reduction (roadmap P2-4).

OFDM sums 57 independently modulated carriers, so its envelope is very nearly complex
Gaussian and its peaks are large: this waveform measures 9–10 dB PAPR. That matters because
an SSB transmitter's PA is driven at a fixed *peak* — the operator sets drive until ALC just
stops acting — so every dB of PAPR is a dB of average power, and therefore of link margin,
thrown away. It is also what the field reports as "ugly ALC spikes"
(``docs/COMMUNITY-CONCERNS.md`` #8).

Two reduction techniques are implemented, both of which keep the ADR-0002 air interface
exactly as it is (a receiver needs no knowledge of either):

* :class:`ClipAndFilter` — clip the envelope, then re-apply the band-limiting filter to put
  back what the clipping splattered out of band, and iterate because filtering regrows the
  peaks. Costs nothing in throughput; pays in in-band distortion (EVM), which sets a ceiling
  on the SNR the highest modes can reach.
* :class:`ToneReservation` — sacrifice a few carriers to carry a peak-cancelling signal.
  Costs those carriers' payload; pays nothing in EVM, because the data carriers are never
  touched and the receiver simply ignores the reserved ones.

Both operate on the complex baseband envelope, which is the right domain: an SSB
transmitter's nonlinearity is memoryless in the envelope (see
:class:`~aether_model.channel.SaturatingPa`).
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

from aether_model.phy.ofdm import carrier_map
from aether_model.phy.passband import band_limit_taps
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]


# ── measurement ───────────────────────────────────────────────────────


def papr_db(x: ComplexArray) -> float:
    """Peak-to-average power ratio of a burst, in dB."""
    p = np.abs(np.asarray(x)) ** 2
    mean = float(np.mean(p))
    if mean <= 0:
        return 0.0
    return float(10 * np.log10(np.max(p) / mean))


def ccdf(x: ComplexArray, levels_db: FloatArray | list[float]) -> FloatArray:
    """P(instantaneous power exceeds mean power by more than each level)."""
    p = np.abs(np.asarray(x)) ** 2
    rel = 10 * np.log10(np.maximum(p / np.mean(p), 1e-30))
    return np.array([float(np.mean(rel > lv)) for lv in np.asarray(levels_db)])


def out_of_band_db(
    x: ComplexArray, params: WaveformParams = WIDE_2300, guard_hz: float = 250.0
) -> float:
    """Power beyond the occupied band plus ``guard_hz``, relative to the power inside it.

    The guard keeps the filter's own transition skirt out of the measurement, so what is left
    is spectral regrowth — the thing that would land in a neighbour's passband.
    """
    x = np.asarray(x, dtype=np.complex128)
    spec = np.abs(np.fft.fft(x)) ** 2
    freq = np.fft.fftfreq(len(x), 1.0 / params.fs_baseband)
    edge = params.occupied_bandwidth_hz / 2 + guard_hz
    inside = float(np.sum(spec[np.abs(freq) <= params.occupied_bandwidth_hz / 2]))
    outside = float(np.sum(spec[np.abs(freq) > edge]))
    return float(10 * np.log10(max(outside, 1e-30) / max(inside, 1e-30)))


def evm_db(reference: ComplexArray, distorted: ComplexArray) -> float:
    """In-band error power after removing the best complex gain — a PA compresses, and a
    pure gain change is not distortion."""
    ref = np.asarray(reference, dtype=np.complex128)
    got = np.asarray(distorted, dtype=np.complex128)
    alpha = np.vdot(ref, got) / np.vdot(ref, ref)
    err = got - alpha * ref
    return float(10 * np.log10(np.mean(np.abs(err) ** 2) / np.mean(np.abs(alpha * ref) ** 2)))


def average_power_gain_db(x: ComplexArray, saturation_amplitude: float) -> float:
    """Average output power relative to the PA's saturation power — the number that actually
    sets link margin when drive is limited by peaks."""
    return float(10 * np.log10(np.mean(np.abs(x) ** 2) / saturation_amplitude**2))


CLIP_TARGET_DB = 5.0
"""Clip target for the constant-modulus modes (BPSK, QPSK, 8-PSK). Measured
(``bench/baselines/papr.csv``): EVM −22.8 dB, far more than they need — 8-PSK 1/2 decodes
20/20 at its threshold — and worth +1.0 … +1.7 dB of delivered power. These are the modes a
weak link actually runs on, so this is where the gain matters most."""
CLIP_TARGET_DENSE_DB = 7.0
"""Clip target for the amplitude-modulated constellations (16-QAM, 64-QAM). Their outer
points sit where the clipper works hardest: at the 5 dB target, 16-QAM 3/4 goes from 0 % to
45 % frame errors at its threshold. At 7 dB (EVM −31.9 dB) it is clean again, and the modes
still gain +0.25 … +0.50 dB."""


def clip_target_db(bits_per_symbol: int) -> float:
    """Clip target for a mode. Peak reduction is bought with in-band distortion, and how much
    a mode can absorb depends on whether its constellation carries information in amplitude:
    PSK rides through, QAM does not."""
    return CLIP_TARGET_DENSE_DB if bits_per_symbol >= 4 else CLIP_TARGET_DB


# ── reduction ─────────────────────────────────────────────────────────


def _clip(x: ComplexArray, threshold: float) -> ComplexArray:
    mag = np.abs(x)
    scale = np.ones_like(mag)
    hot = mag > threshold
    scale[hot] = threshold / mag[hot]
    return x * scale


class ClipAndFilter:
    """Iterative clip-and-filter (Armstrong, *Electron. Lett.* 2002).

    Each pass clips the envelope to ``target_papr_db`` above the mean and then runs the
    band-limiting filter, which removes the out-of-band splatter clipping created but also
    restores part of the peak — hence several passes. The filter is the same one the passband
    chain already applies, so nothing here can emit energy the transmitter would not.
    """

    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        target_papr_db: float = 6.0,
        iterations: int = 4,
    ) -> None:
        self.p = params
        self.target_papr_db = float(target_papr_db)
        self.iterations = int(iterations)
        self._taps = band_limit_taps(params)

    def _filter(self, x: ComplexArray) -> ComplexArray:
        # one pass of the *same* linear-phase FIR the passband chain uses, its constant group
        # delay taken out — exactly what a streaming transmitter does, so nothing here relies
        # on seeing the whole burst at once
        return np.asarray(np.convolve(x, self._taps, mode="same"), dtype=np.complex128)

    def process(self, x: ComplexArray) -> ComplexArray:
        y = np.asarray(x, dtype=np.complex128).copy()
        mean_power = float(np.mean(np.abs(y) ** 2))
        if mean_power <= 0:
            return y
        threshold = np.sqrt(mean_power * 10 ** (self.target_papr_db / 10))
        for _ in range(self.iterations):
            y = self._filter(_clip(y, threshold))
        # preserve average power so the comparison is like for like
        scale = np.sqrt(mean_power / max(float(np.mean(np.abs(y) ** 2)), 1e-30))
        return y * scale


class ToneReservation:
    """Peak cancellation on reserved carriers (Tellado, Stanford thesis, 1999).

    ``n_reserved`` data carriers are given up; the clipping noise is projected onto them and
    subtracted, so peaks come down without touching a single data symbol. The receiver needs
    no change — it already reads data only from the carriers the map assigns — but the
    reserved carriers no longer carry payload, which is the price.

    The reserved set is spread evenly across the band: peak cancellation needs the reserved
    tones to span the bandwidth, or the correction signal cannot be sharp in time.
    """

    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        n_reserved: int = 4,
        target_papr_db: float = 6.0,
        iterations: int = 8,
        step: float = 1.0,
    ) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        data = np.asarray(self.cmap.data_carriers)
        if not 1 <= n_reserved <= len(data):
            raise ValueError("n_reserved must fit inside the data carriers")
        pick = np.linspace(0, len(data) - 1, n_reserved).round().astype(int)
        self.reserved_carriers = data[np.unique(pick)]
        self.n_reserved = len(self.reserved_carriers)
        self.target_papr_db = float(target_papr_db)
        self.iterations = int(iterations)
        self.step = float(step)
        self._bins = np.asarray(self.cmap.bins)[self.reserved_carriers] % params.fft_size

    @property
    def payload_cost(self) -> float:
        """Fraction of the data carriers given up."""
        return self.n_reserved / len(self.cmap.data_carriers)

    def process_symbol(self, x: ComplexArray) -> ComplexArray:
        """Reduce the peaks of one time-domain OFDM symbol body (``fft_size`` samples)."""
        y = np.asarray(x, dtype=np.complex128).copy()
        n = self.p.fft_size
        if len(y) != n:
            raise ValueError(f"expected one symbol body of {n} samples, got {len(y)}")
        threshold = np.sqrt(np.mean(np.abs(y) ** 2) * 10 ** (self.target_papr_db / 10))
        for _ in range(self.iterations):
            excess = y - _clip(y, threshold)
            if not np.any(excess):
                break
            spec = np.fft.fft(excess)
            kept = np.zeros_like(spec)
            kept[self._bins] = spec[self._bins]
            y = y - self.step * np.fft.ifft(kept)
        return y
