"""Complex baseband (8 kHz) ↔ real audio (48 kHz) at the configured centre frequency.

Transmit: baseband ─► band-limiting FIR ─► mix to ``centre_hz`` ─► Re{·} ─► ×6 interpolation
FIR ─► float32 audio. Receive is the mirror image: ÷6 decimation FIR ─► mix down ─► the
same band-limiting FIR (which also removes the image at ``−2·centre``) ─► baseband.

Both directions are streaming objects with FIR state and phase-continuous mixers, so
arbitrary block sizes give the same output as one big block (checked in tests). Group
delays are fixed and known (``tx_delay_samples`` / ``rx_delay_samples`` at the audio rate);
frame synchronization absorbs them.
"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]


def band_limit_taps(params: WaveformParams, transition_hz: float = 240.0) -> FloatArray:
    """Linear-phase low-pass for the complex baseband: flat over all active carriers."""
    edge = params.occupied_bandwidth_hz / 2 + params.subcarrier_spacing_hz  # ≈ 1 180 Hz
    cutoff = edge + transition_hz / 2
    beta = signal.kaiser_beta(70.0)
    numtaps = int(np.ceil((70.0 - 8.0) / (2.285 * 2 * np.pi * transition_hz / params.fs_baseband)))
    numtaps |= 1  # odd → symmetric, integer group delay
    return np.asarray(
        signal.firwin(numtaps, cutoff, fs=params.fs_baseband, window=("kaiser", beta))
    )


def interpolation_taps(params: WaveformParams) -> FloatArray:
    """Low-pass at the audio rate for ×R interpolation / ÷R decimation.

    Passband reaches the top of the occupied audio band (centre + half bandwidth + one
    spacing); the stopband starts at the first zero-stuffing image. Its length is 12k+1 so
    the group delay is a multiple of the resample factor (integer delay at baseband)."""
    r = params.resample_factor
    fa = params.audio_rate
    fs_bb = params.fs_baseband
    beta = signal.kaiser_beta(70.0)
    top = params.centre_hz + params.occupied_bandwidth_hz / 2 + params.subcarrier_spacing_hz
    transition = 2 * (fs_bb / 2 - top)  # symmetric around baseband Nyquist
    numtaps = int(np.ceil((70.0 - 8.0) / (2.285 * 2 * np.pi * transition / fa)))
    numtaps = ((numtaps + 2 * r - 1) // (2 * r)) * 2 * r + 1
    return np.asarray(signal.firwin(numtaps, fs_bb / 2, fs=fa, window=("kaiser", beta))) * r


class _Fir:
    def __init__(self, taps: FloatArray, dtype: type = np.complex128) -> None:
        self.taps = taps
        self._zi: NDArray = np.zeros(len(taps) - 1, dtype=dtype)

    def __call__(self, x: NDArray) -> NDArray:
        y, self._zi = signal.lfilter(self.taps, [1.0], x, zi=self._zi)
        return np.asarray(y)

    @property
    def delay(self) -> int:
        return (len(self.taps) - 1) // 2


class BasebandToAudio:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.r = params.resample_factor
        self._band = _Fir(band_limit_taps(params))
        self._interp = _Fir(interpolation_taps(params), dtype=np.float64)
        self._n0 = 0

    @property
    def tx_delay_samples(self) -> int:
        """Group delay at the audio rate."""
        return self._band.delay * self.r + self._interp.delay

    def process(self, baseband: ComplexArray) -> NDArray[np.float32]:
        x = self._band(np.asarray(baseband, dtype=np.complex128))
        n = len(x)
        t = (self._n0 + np.arange(n)) / self.p.fs_baseband
        self._n0 += n
        passband = np.real(x * np.exp(2j * np.pi * self.p.centre_hz * t))
        stuffed = np.zeros(n * self.r)
        stuffed[:: self.r] = passband
        return self._interp(stuffed).astype(np.float32)


class AudioToBaseband:
    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.r = params.resample_factor
        self._decim = _Fir(interpolation_taps(params) / params.resample_factor, dtype=np.float64)
        self._band = _Fir(band_limit_taps(params))
        self._phase = 0  # decimation phase carried across blocks
        self._n0 = 0

    @property
    def rx_delay_samples(self) -> int:
        """Group delay at the audio rate."""
        return self._decim.delay + self._band.delay * self.r

    def process(self, audio: NDArray) -> ComplexArray:
        y = self._decim(np.asarray(audio, dtype=np.float64))
        idx = np.arange(self._phase, len(y), self.r)
        self._phase = (self._phase + self.r * len(idx)) - len(y)
        low = y[idx]
        n = len(low)
        t = (self._n0 + np.arange(n)) / self.p.fs_baseband
        self._n0 += n
        mixed = low * np.exp(-2j * np.pi * self.p.centre_hz * t)
        return np.asarray(self._band(mixed) * 2.0, dtype=np.complex128)  # ×2 undoes Re{}'s halving
