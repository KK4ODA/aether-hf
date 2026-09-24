"""A fading channel for the lossy pipe (P9-6): what a frame sees, and what it is judged at.

The logistic pipe of :mod:`aether_model.link.sim` gives every frame the channel's SNR and a
frame error rate read off the per-class curve at that SNR, independently of every other
frame. That is the average over the fading and nothing else: no frame ever arrives in a
fade that also swallows the next three, no connect frame is ever measured on a lucky peak,
and the acknowledgement never fades with the burst it answers. Those are exactly the things
a session's start and its rate control have to live with on the air.

This module models them from the standard channel (ITU-R F.1487 two-path Watterson, as the
modem's channel simulator runs it) and a textbook link-to-system mapping:

* **One channel for both directions.** HF is reciprocal: the two stations share the same
  pair of Rayleigh taps, sampled on the simulation clock, so a burst and the
  acknowledgement after it see the same fade.
* **Per frame, per resource element.** Over a frame's air time and across its carriers the
  instantaneous SNR is the mean SNR times ``|H(f, t)|²`` of the two-ray response.
* **Exponential effective SNR mapping** (EESM, the 3GPP link-to-system method):
  ``γ_eff = −β ln( mean(exp(−γ_i / β)) )``, and the frame decodes with the probability the
  modem's AWGN waterfall gives at ``γ_eff``: a steep logistic through the mode's measured
  10 % point. β is calibrated per frame type and channel class so that the ensemble's 10 %
  point is the one the real modem measured on that class (``tools/calibrate_fading.py``,
  ``bench/baselines/fading_pipe.csv``) — the pipe reproduces the measured averages and adds
  the correlation they average away.
* **The SNR the receiver reports** is the frame's mean ``γ`` with a little estimation noise,
  which is what the rate controller and the connect handshake act on.
"""

from __future__ import annotations

import math
from collections.abc import Callable
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.channel import RayleighFadingProcess, get_profile
from aether_model.frame.modes import AirInterface
from aether_model.link.phy import Container, TxFrame
from aether_model.phy.ofdm import CarrierMap

FloatArray = NDArray[np.float64]

RATE_HZ = 50.0
"""The taps' sampling rate on the simulation clock: forty times the fastest Doppler spread
of the four classes' σ, far finer than any frame."""
STEP_S = 0.05
"""Spacing of the time samples taken across a frame: the channel moves far less in 50 ms
than the frame's own symbols resolve, even on ITU Poor."""
STEEP = 3.5
"""Logistic steepness of the modem's frame error rate against SNR on AWGN, per dB — the
median of the measured waterfalls (``phy_fer*.csv``): 90 % to 10 % in about a decibel and
a quarter. The old pipe's 1.2 was a fading average applied to every frame."""
ESTIMATE_SIGMA_DB = 0.5
"""Spread of the receiver's SNR estimate about a frame's true mean."""


@dataclass(frozen=True)
class FrameShape:
    """Where a frame's resource elements lie: its air time and its data carriers' offsets
    from the band centre, in hertz (subsampled — the channel is smooth across them)."""

    duration_s: float
    carriers_hz: tuple[float, ...]


class SharedFading:
    """The two-ray channel both stations share, sampled lazily on the simulation clock."""

    def __init__(self, profile: str, seed: int = 0) -> None:
        prof = get_profile(profile)
        powers = 10.0 ** (np.asarray(prof.tap_powers_db) / 10.0)
        self.amps = np.sqrt(powers / powers.sum())
        self.delays_s = np.asarray(prof.delays_s, dtype=np.float64)
        seeds = np.random.SeedSequence(seed).spawn(len(prof.delays_s))
        self._taps = [
            RayleighFadingProcess(RATE_HZ, spread, np.random.default_rng(s))
            for spread, s in zip(prof.doppler_spread_hz, seeds, strict=True)
        ]
        self._gains = [np.zeros(0, dtype=np.complex128) for _ in self._taps]
        self.static = all(sp <= 0.0 for sp in prof.doppler_spread_hz) and len(self._taps) == 1

    def _extend(self, n: int) -> None:
        have = len(self._gains[0])
        if n <= have:
            return
        more = n - have + int(10 * RATE_HZ)
        self._gains = [
            np.concatenate((g, tap.next(more)))
            for g, tap in zip(self._gains, self._taps, strict=True)
        ]

    def power(self, t0: float, t1: float, carriers_hz: tuple[float, ...]) -> FloatArray:
        """``|H(f, t)|²`` over a frame (time × carrier), unit mean power on average."""
        if self.static:
            return np.ones((1, len(carriers_hz)))
        n = max(1, math.ceil((t1 - t0) / STEP_S))
        times = t0 + STEP_S * (0.5 + np.arange(n))
        idx = np.maximum(0, np.round(times * RATE_HZ).astype(int))
        self._extend(int(idx.max()) + 2)
        f = np.asarray(carriers_hz, dtype=np.float64)
        h = np.zeros((n, len(f)), dtype=np.complex128)
        for amp, delay, g in zip(self.amps, self.delays_s, self._gains, strict=True):
            h += amp * g[idx][:, None] * np.exp(-2j * np.pi * f[None, :] * delay)
        return np.abs(h) ** 2


def effective_snr_db(snr_linear: FloatArray, beta: float) -> float:
    """EESM: ``−β ln(mean(exp(−γ/β)))``, computed stably, in dB."""
    x = -np.asarray(snr_linear, dtype=np.float64).ravel() / beta
    top = float(x.max())
    # log of the mean, the largest term factored out, so nothing underflows however far
    # the elements sit above β
    log_mean = top + math.log(float(np.mean(np.exp(x - top))))
    return 10.0 * math.log10(max(-beta * log_mean, 1e-30))


def success_probability(decode_snr_db: float, threshold_db: float) -> float:
    """The modem's AWGN waterfall: a steep logistic through the 10 % frame error point."""
    z = STEEP * (decode_snr_db - threshold_db) + math.log(9.0)
    return 1.0 / (1.0 + math.exp(-max(-60.0, min(60.0, z))))


@dataclass
class FadingPipe:
    """Everything the lossy pipe needs to put a frame through a fading channel.

    ``shape`` and ``beta`` answer per frame (``beta`` from the calibration table for the
    channel class being modelled); ``fading`` is the shared channel."""

    fading: SharedFading
    shape: Callable[[TxFrame], FrameShape]
    beta: Callable[[TxFrame], float]
    rng: np.random.Generator

    def judge(
        self, frame: TxFrame, mean_snr_db: float, t0: float, t1: float
    ) -> tuple[float, float]:
        """(the SNR the receiver reports, the SNR the frame decodes at) for one frame."""
        shape = self.shape(frame)
        gain = self.fading.power(t0, t1, shape.carriers_hz)
        snr = 10.0 ** (mean_snr_db / 10.0) * gain
        reported = 10.0 * math.log10(max(float(np.mean(snr)), 1e-30))
        reported += float(self.rng.normal(0.0, ESTIMATE_SIGMA_DB))
        if self.fading.static:
            return reported, mean_snr_db
        return reported, effective_snr_db(snr, self.beta(frame))


def frame_key(frame: TxFrame) -> str:
    """The name a frame goes by in the calibration table: ``mode N``, ``control short`` or
    ``control floor`` — the labels of ``bench_floor.py`` and ``bench_peak.py``."""
    if frame.container is Container.CONTROL:
        return "control floor" if frame.floor else "control short"
    return f"mode {frame.mode}"


CARRIER_STEP_HZ = 150.0
"""Spacing of the carriers a frame is sampled at: the two-ray response's notches are 500 Hz
apart on ITU Poor, 2 kHz on Good, so a few samples per notch period resolve them."""


def shapes_for(air: AirInterface) -> Callable[[TxFrame], FrameShape]:
    """Each frame's shape on an air: the layout it goes out on and the air's data carriers,
    as offsets from the band centre."""
    params = air.params
    data = CarrierMap(params).data_carriers
    step = max(1, round(CARRIER_STEP_HZ / params.subcarrier_spacing_hz))
    centre = params.n_carriers // 2
    carriers = tuple(float((k - centre) * params.subcarrier_spacing_hz) for k in data[::step])

    def shape(frame: TxFrame) -> FrameShape:
        if frame.container is Container.CONTROL:
            floor = frame.floor and air.floor_short is not None
            layout = air.floor_short if floor else air.short
        else:
            layout = air.data_layout(frame.mode)
        assert layout is not None
        return FrameShape(layout.duration_s, carriers)

    return shape
