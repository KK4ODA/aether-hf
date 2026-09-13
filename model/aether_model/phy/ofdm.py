"""OFDM symbol construction and parsing for the v1 waveform (roadmap P1-4 / P1-6).

Carrier map
-----------
The ``n_carriers`` active carriers occupy FFT bins ``c − n_carriers//2`` for carrier index
``c = 0 … n_carriers−1`` (DC-centred; the passband stage moves DC to ``centre_hz``). Comb
pilots sit on every ``pilot_carrier_spacing``-th carrier including both band edges; the
remaining carriers carry data.

Known sequences
---------------
All pilot values come from one frequency-domain Zadoff–Chu sequence of length
``n_carriers`` (root :data:`PILOT_ROOT`): unit magnitude on every carrier, and a full pilot
symbol built from it has a low-PAPR time waveform (≈ 3.5 dB, versus ≈ 10 dB for data
symbols), which keeps the transmit envelope steady across pilot and data symbols.

Symbol shaping
--------------
Each symbol is ``CP + N`` samples plus a ``taper``-sample cyclic *suffix*; a raised-cosine
ramp is applied to the first and last ``taper`` samples and consecutive symbols overlap-add
by ``taper`` samples. The symbol period stays ``CP + N``; the effective CP shrinks by the
taper. This is the standard windowed-OFDM construction and, unlike the legacy code, adds no
ICI: the FFT window never sees a tapered sample.
"""

from __future__ import annotations

import math
from dataclasses import dataclass
from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

PILOT_ROOT = 7
"""Zadoff–Chu root of the pilot/full-pilot sequence (coprime with 57, 68 and 12)."""


def zadoff_chu(length: int, root: int) -> ComplexArray:
    """Zadoff–Chu sequence ``exp(−jπ·u·n(n+c_f)/N)`` with ``c_f = N mod 2`` (u coprime to N)."""
    if math.gcd(length, root) != 1:
        raise ValueError(f"root {root} is not coprime with length {length}")
    n = np.arange(length)
    cf = length % 2
    return np.exp(-1j * np.pi * root * n * (n + cf) / length).astype(np.complex128)


@dataclass(frozen=True)
class CarrierMap:
    params: WaveformParams

    @property
    def n_carriers(self) -> int:
        return self.params.n_carriers

    @property
    def bins(self) -> NDArray[np.int64]:
        """FFT bin (may be negative) of each carrier index."""
        return np.arange(self.n_carriers) - self.n_carriers // 2

    @property
    def pilot_carriers(self) -> NDArray[np.int64]:
        s = self.params.pilot_carrier_spacing
        last = self.n_carriers - 1
        idx = list(range(0, self.n_carriers, s))
        if idx[-1] != last:
            idx.append(last)
        return np.array(idx, dtype=np.int64)

    @property
    def data_carriers(self) -> NDArray[np.int64]:
        mask = np.ones(self.n_carriers, dtype=bool)
        mask[self.pilot_carriers] = False
        return np.flatnonzero(mask).astype(np.int64)

    @property
    def pilot_sequence(self) -> ComplexArray:
        """Known unit-magnitude value for every carrier index."""
        return zadoff_chu(self.n_carriers, PILOT_ROOT)


@cache
def carrier_map(params: WaveformParams = WIDE_2300) -> CarrierMap:
    return CarrierMap(params)


class OfdmModulator:
    """Frequency-domain carrier values → windowed, overlap-added time-domain samples."""

    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        n, cp, w = params.fft_size, params.cp_samples, params.taper_samples
        self.n, self.cp, self.w = n, cp, w
        ramp = 0.5 * (1 - np.cos(np.pi * (np.arange(w) + 0.5) / w))  # 0 → 1, symmetric power
        self._ramp_up = ramp
        self._ramp_down = ramp[::-1]
        # Scale so that a symbol with unit-power carriers has unit mean power per sample.
        self._scale = n / math.sqrt(self.cmap.n_carriers)

    def symbol_values(
        self,
        data: ComplexArray | None,
        full_pilot: bool = False,
        chips: ComplexArray | None = None,
    ) -> ComplexArray:
        """Carrier values (length n_carriers) for a data symbol with comb pilots, or a full
        pilot symbol (optionally with ``chips`` — ±1 header chips — on its data carriers)."""
        cm = self.cmap
        x = np.zeros(cm.n_carriers, dtype=np.complex128)
        seq = cm.pilot_sequence
        if full_pilot:
            x[:] = seq
            if chips is not None:
                if len(chips) != len(cm.data_carriers):
                    raise ValueError(f"need {len(cm.data_carriers)} chips")
                x[cm.data_carriers] = chips
            return x
        if data is None or len(data) != len(cm.data_carriers):
            raise ValueError(f"need {len(cm.data_carriers)} data-carrier values")
        x[cm.pilot_carriers] = seq[cm.pilot_carriers]
        x[cm.data_carriers] = data
        return x

    def to_time(self, carrier_values: ComplexArray) -> ComplexArray:
        """One extended symbol: CP + N + taper suffix, edges tapered (length CP+N+taper)."""
        spectrum = np.zeros(self.n, dtype=np.complex128)
        spectrum[self.cmap.bins % self.n] = carrier_values
        body = np.fft.ifft(spectrum) * self._scale
        ext = np.concatenate((body[-self.cp :], body, body[: self.w]))
        ext[: self.w] *= self._ramp_up
        ext[-self.w :] *= self._ramp_down
        return ext

    def modulate(self, symbols: list[ComplexArray]) -> ComplexArray:
        """Overlap-add a sequence of carrier-value vectors into a contiguous waveform.

        Output length is ``len(symbols) · (CP+N) + taper``: the trailing taper is the last
        symbol's ramp-down (so consecutive calls can be overlap-added by the caller)."""
        period = self.cp + self.n
        out = np.zeros(len(symbols) * period + self.w, dtype=np.complex128)
        for i, vals in enumerate(symbols):
            out[i * period : i * period + period + self.w] += self.to_time(vals)
        return out


class OfdmDemodulator:
    """Time-domain samples → raw (un-equalized) carrier values."""

    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.cmap = carrier_map(params)
        self.n, self.cp, self.w = params.fft_size, params.cp_samples, params.taper_samples
        self.period = self.n + self.cp
        self._scale = math.sqrt(self.cmap.n_carriers) / self.n

    @property
    def fft_offset(self) -> int:
        """Where the FFT window starts inside a symbol period: past the tapered part of the
        CP, but with margin before the useful part so late multipath stays inside the CP."""
        return self.w + (self.cp - self.w) // 2

    def carriers(self, samples: ComplexArray, symbol_start: int) -> ComplexArray:
        """Carrier values of the symbol whose period begins at ``symbol_start``.

        The FFT window is taken ``fft_offset`` samples into the period; the resulting
        linear phase ramp across carriers is removed so that a distortion-free channel yields
        the transmitted carrier values exactly."""
        start = symbol_start + self.fft_offset
        seg = samples[start : start + self.n]
        if len(seg) != self.n:
            raise ValueError("symbol runs past the end of the buffer")
        spectrum = np.fft.fft(seg) * self._scale
        vals = spectrum[self.cmap.bins % self.n]
        # window starts `shift` samples before the useful part: W[k] = X[k]·e^{+j2πk·shift/N}
        shift = self.fft_offset - self.cp
        return vals * np.exp(-2j * np.pi * self.cmap.bins * shift / self.n)
