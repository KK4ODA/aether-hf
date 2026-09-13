"""Wiener (MMSE) channel interpolation (roadmap P2-6).

The Phase 1 estimator interpolates the comb pilots *linearly* across frequency and averages
±1 symbol in time. Linear interpolation is the MMSE solution only if the channel really is a
straight line between pilots, which it is not: a multipath channel's frequency response is
the Fourier transform of its delay profile, and its time evolution is a Bessel function of
the Doppler spread. Using those two correlations instead is the textbook improvement, and it
is cheap here because the pilot grid is fixed — the filters depend only on the geometry and
the SNR, so they are built once and cached.

The estimator is **separable**: Wiener across frequency (15 pilot carriers → 57 carriers),
then Wiener along time (a short symbol window). A jointly-optimal 2-D filter would need a
510 x 510 inverse per frame for a fraction of a dB more; separable is the standard
engineering answer and is what is implemented.

**The statistics are estimated, not assumed.** A single worst-case design was tried first and
measured: designing for 3 ms of delay spread costs 2-5 dB of interpolation accuracy on the
channels that are actually flatter than that, which is most of them, and loses to plain
linear interpolation nearly everywhere. Matched to the delay spread actually present, the
same filter beats linear by 1.3 dB (ITU Poor) to 5.3 dB (AWGN). So the spread is measured
per frame — cheaply, from the pilots that are already there.

:func:`estimate_channel_statistics` takes the inverse FFT of the comb-pilot estimates across
frequency, which is the channel impulse response sampled at the pilot spacing, and reads two
numbers off it: how far the energy extends (the delay spread) and how much sits in the tail
beyond any plausible echo (the noise floor, hence the SNR). Both feed the filter design, and
both are snapped to a small grid so the filters stay cached rather than rebuilt per frame.

Note the hard limit the pilot grid imposes: pilots every 160 Hz sample the frequency response
at 160 Hz, so delays beyond 1/160 Hz = 6.25 ms alias and cannot be represented at all, no
matter how the filter is designed.

Correlations used:

* frequency, uniform delay profile on ``[0, tau_max]``:
  ``R(df) = exp(-j pi df tau) sinc(df tau)``
* time, uniform Doppler spectrum on ``[-f_d, f_d]``: ``R(dt) = sinc(2 f_d dt)``

A uniform Doppler spectrum is used rather than Jakes' ``J0(2 pi f_d dt)`` deliberately: it is
the more conservative of the two (it decorrelates faster), which is what robust design wants.
"""

from __future__ import annotations

from functools import cache

import numpy as np
from numpy.typing import NDArray

from aether_model.phy.ofdm import carrier_map
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]

DEFAULT_TAU_MAX_S = 3e-3
"""Design delay spread. Over-designing this is *not* free: the comb pilots sit 160 Hz apart
(every 4th of 40 Hz), so the pilot grid samples the frequency response at 160 Hz and cannot
represent a delay spread beyond 1/160 Hz = 6.25 ms at all — at that point adjacent pilots are
formally uncorrelated, the Wiener solution concludes it cannot interpolate, and the estimate
collapses toward zero between pilots. 3 ms covers ITU Poor's 2 ms while staying comfortably
inside the grid's unambiguous range (adjacent-pilot correlation 0.77 rather than 0.01)."""
DEFAULT_DOPPLER_MAX_HZ = 1.5
"""Design Doppler spread. ITU Poor is 1 Hz (2-sigma, F.1487); 1.5 Hz leaves headroom."""
DEFAULT_SNR_DB = 10.0
"""Design SNR used when the caller does not supply a measurement."""

TAU_GRID_S: tuple[float, ...] = (0.5e-3, 1.0e-3, 2.0e-3, 3.0e-3, 4.5e-3)
"""Delay spreads the filters are built for. Estimates snap to the nearest, so the cache holds
a handful of filters instead of one per frame."""
SNR_GRID_DB: tuple[float, ...] = (0.0, 5.0, 10.0, 15.0, 20.0, 25.0)
"""Design SNRs, likewise snapped."""


def _snap(value: float, grid: tuple[float, ...]) -> float:
    return min(grid, key=lambda g: abs(g - value))


def estimate_channel_statistics(
    pilot_estimates: ComplexArray, params: WaveformParams = WIDE_2300
) -> tuple[float, float]:
    """``(tau_max_s, snr_db)`` chosen for these pilots by leave-one-out cross-validation.

    The obvious approach — read the delay spread off the pilot impulse response — was tried
    and measured: with only 15 pilots the transform leaks badly enough to overestimate the
    spread by two to four times on fading channels, and a Wiener filter designed for four
    times too much delay is worse than no Wiener filter at all.

    So the spread is not estimated at all. Instead each candidate design is *scored* on the
    job it will actually do: predict each pilot from the others, and keep the design whose
    held-out prediction error is lowest. That is a direct measurement of interpolation
    accuracy on this frame's channel, it needs no assumption about the delay profile, and
    leakage cannot fool it. The SNR still comes from the impulse response, where the far taps
    hold noise and nothing else.
    """
    obs = np.atleast_2d(np.asarray(pilot_estimates, dtype=np.complex128))
    if obs.size == 0 or obs.shape[1] < 3:
        return DEFAULT_TAU_MAX_S, DEFAULT_SNR_DB
    snr_db = _estimate_snr_db(obs)
    best_tau, best_error = TAU_GRID_S[0], float("inf")
    for tau in TAU_GRID_S:
        predicted = obs @ loo_filter(params, tau, snr_db).T
        error = float(np.mean(np.abs(predicted - obs) ** 2))
        if error < best_error:
            best_tau, best_error = tau, error
    return best_tau, snr_db


def _estimate_snr_db(obs: ComplexArray) -> float:
    """SNR from the pilot impulse response: no plausible HF echo reaches the far taps, so
    what sits there is noise."""
    n_pilots = obs.shape[1]
    profile = np.mean(np.abs(np.fft.ifft(obs, axis=1)) ** 2, axis=0)
    floor = float(np.median(profile[n_pilots // 2 :]))
    signal = float(np.maximum(profile - floor, 0.0).sum())
    noise = floor * n_pilots
    if signal <= 0 or noise <= 0:
        return DEFAULT_SNR_DB
    return _snap(10.0 * np.log10(signal / noise), SNR_GRID_DB)


@cache
def loo_filter(
    params: WaveformParams = WIDE_2300,
    tau_max_s: float = DEFAULT_TAU_MAX_S,
    snr_db: float = DEFAULT_SNR_DB,
) -> ComplexArray:
    """``(n_pilots, n_pilots)`` leave-one-out predictor: row *i* reconstructs pilot *i* from
    every pilot but itself (its own column is zero). Used only to score candidate designs."""
    cmap = carrier_map(params)
    pilots = np.asarray(cmap.pilot_carriers, dtype=np.float64) * params.subcarrier_spacing_hz
    n = len(pilots)
    noise = 10.0 ** (-snr_db / 10.0)
    out = np.zeros((n, n), dtype=np.complex128)
    for i in range(n):
        others = np.delete(np.arange(n), i)
        r_pp = _freq_correlation(pilots[others][:, None] - pilots[others][None, :], tau_max_s)
        r_tp = _freq_correlation(pilots[i] - pilots[others], tau_max_s)[None, :]
        out[i, others] = _wiener(r_tp, r_pp, noise)[0]
    return out


def _freq_correlation(delta_hz: NDArray[np.float64], tau_max_s: float) -> ComplexArray:
    """Channel correlation across frequency for a uniform delay profile on [0, tau]."""
    x = delta_hz * tau_max_s
    return np.asarray(np.exp(-1j * np.pi * x) * np.sinc(x), dtype=np.complex128)


def _time_correlation(delta_s: NDArray[np.float64], doppler_max_hz: float) -> ComplexArray:
    """Channel correlation across time for a uniform Doppler spectrum on [-f_d, f_d]."""
    return np.asarray(np.sinc(2.0 * doppler_max_hz * delta_s), dtype=np.complex128)


def _wiener(
    r_target_pilot: ComplexArray, r_pilot_pilot: ComplexArray, noise: float
) -> ComplexArray:
    """W = R_tp (R_pp + noise I)^-1 — one row per target position."""
    n = r_pilot_pilot.shape[0]
    a = r_pilot_pilot + noise * np.eye(n)
    return np.asarray(np.linalg.solve(a.T, r_target_pilot.T).T, dtype=np.complex128)


@cache
def frequency_filter(
    params: WaveformParams = WIDE_2300,
    tau_max_s: float = DEFAULT_TAU_MAX_S,
    snr_db: float = DEFAULT_SNR_DB,
) -> ComplexArray:
    """``(n_carriers, n_pilot_carriers)`` Wiener interpolator across frequency."""
    cmap = carrier_map(params)
    spacing = params.subcarrier_spacing_hz
    pilots = np.asarray(cmap.pilot_carriers, dtype=np.float64)
    targets = np.arange(cmap.n_carriers, dtype=np.float64)
    r_pp = _freq_correlation((pilots[:, None] - pilots[None, :]) * spacing, tau_max_s)
    r_tp = _freq_correlation((targets[:, None] - pilots[None, :]) * spacing, tau_max_s)
    weights = _wiener(r_tp, r_pp, 10.0 ** (-snr_db / 10.0))
    # De-bias. The MMSE estimate is deliberately shrunk toward zero — optimal for minimising
    # E|H - H_hat|^2, but this estimate is then *divided into* the received symbols, and a
    # systematic 9 % shrink at the design SNR rescales the whole constellation and corrupts
    # every LLR. Normalising each row so a flat unit channel maps to unity removes the bias
    # while keeping the shape of the interpolation, which is what actually carries the gain.
    row_sums = weights.sum(axis=1, keepdims=True)
    return np.asarray(
        weights / np.where(np.abs(row_sums) < 1e-12, 1.0, row_sums), dtype=np.complex128
    )


@cache
def time_filter(
    params: WaveformParams = WIDE_2300,
    doppler_max_hz: float = DEFAULT_DOPPLER_MAX_HZ,
    snr_db: float = DEFAULT_SNR_DB,
    half_width: int = 2,
) -> ComplexArray:
    """``(2*half_width+1,)`` symmetric Wiener smoother along time, for a symbol at the centre
    of a window of neighbours. Edge symbols reuse the nearest full window."""
    offsets = np.arange(-half_width, half_width + 1, dtype=np.float64)
    period = params.symbol_period_s
    r_pp = _time_correlation((offsets[:, None] - offsets[None, :]) * period, doppler_max_hz)
    r_tp = _time_correlation(offsets[None, :] * period, doppler_max_hz)
    return _wiener(r_tp, r_pp, 10.0 ** (-snr_db / 10.0))[0]


def smooth_time(comb: ComplexArray, first: int, taps: ComplexArray) -> ComplexArray:
    """Apply the symmetric time smoother down the symbol axis of ``comb[first:]``, holding
    the window inside the valid range at the edges."""
    out = comb.copy()
    n = comb.shape[0]
    half = (len(taps) - 1) // 2
    for s in range(first, n):
        lo = max(first, s - half)
        hi = min(n - 1, s + half)
        window = comb[lo : hi + 1]
        w = taps[(lo - s + half) : (hi - s + half + 1)]
        total = w.sum()
        if abs(total) < 1e-12:
            continue
        out[s] = (w[:, None] * window).sum(axis=0) / total
    return out
