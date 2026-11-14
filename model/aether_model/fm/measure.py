"""The one measure used on recordings and on the model's output (ADR-0115 §2.5).

Gate 2 compares a recorded radio with the model through these functions only, so both sides
are measured the same way:

* :func:`delay` and :func:`clock_ppm` align what was heard with what was sent;
* :func:`sndr` is the magnitude-squared coherence between them, SNDR(f) = C/(1 − C), with
  distortion counted as noise, and the effective SNDR over a band: the flat SNDR that gives
  the same information rate;
* :func:`response` is the linear response S_xy/S_xx, normalised at 1 kHz.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray
from scipy import signal

from aether_model.fm.channel import FS

FloatArray = NDArray[np.float64]

SEGMENT = 4096
"""Welch segment length: about 12 Hz resolution at 48 kHz."""


def delay(sent: FloatArray, heard: FloatArray, max_lag: int) -> int:
    """How many samples ``heard`` lags ``sent``, from the cross-correlation's peak within
    ``0 … max_lag``."""
    n = min(len(sent), len(heard) - max_lag)
    if n <= 0:
        raise ValueError("the recording is shorter than the search")
    c = signal.correlate(heard[: n + max_lag], sent[:n], mode="valid", method="fft")
    return int(np.argmax(np.abs(c)))


def clock_ppm(
    marker: FloatArray, heard: FloatArray, first: int, second: int, max_lag: int
) -> tuple[float, int]:
    """The heard clock against the sent one, from a marker sent at sample ``first`` and again
    at ``second``: ``(ppm, delay of the first)``. Positive ppm: the heard samples run fast
    against the sent ones (the recording is longer)."""
    d0 = _marker_at(marker, heard, first, max_lag)
    d1 = _marker_at(marker, heard, second, max_lag)
    span = second - first
    return ((d1 - d0) / span) * 1e6, round(d0)


def _marker_at(marker: FloatArray, heard: FloatArray, at: int, max_lag: int) -> float:
    lo = max(0, at - max_lag)
    seg = heard[lo : at + max_lag + len(marker)]
    c = np.abs(signal.correlate(seg, marker, mode="valid", method="fft"))
    k = int(np.argmax(c))
    # a parabola through the peak and its neighbours, for a sub-sample position
    if 0 < k < len(c) - 1:
        y0, y1, y2 = float(c[k - 1]), float(c[k]), float(c[k + 1])
        den = y0 - 2.0 * y1 + y2
        frac = 0.5 * (y0 - y2) / den if den != 0.0 else 0.0
    else:
        frac = 0.0
    return lo + k + frac - at


def resample(x: FloatArray, ppm: float) -> FloatArray:
    """Undo a clock ``ppm`` fast: ``x`` is played back at the sent clock (polyphase, exact for
    the long recordings a few hundred ppm off that the bench gives)."""
    if ppm == 0.0:
        return np.asarray(x, dtype=np.float64)
    n_out = round(len(x) / (1.0 + ppm * 1e-6))
    pos = np.arange(n_out) * (1.0 + ppm * 1e-6)
    return np.asarray(np.interp(pos, np.arange(len(x)), x), dtype=np.float64)


@dataclass(frozen=True)
class SndrReading:
    """SNDR against frequency over a band, and what it adds up to."""

    freqs_hz: FloatArray
    sndr_db: FloatArray
    rate_bps: float
    effective_db: float


def sndr(
    sent: FloatArray, heard: FloatArray, band: tuple[float, float], segment: int = SEGMENT
) -> SndrReading:
    """SNDR(f) = C/(1 − C) from the coherence of aligned ``sent`` and ``heard``, the information
    rate it allows over ``band``, and the effective SNDR: the flat SNDR with the same rate."""
    n = min(len(sent), len(heard))
    f, c = signal.coherence(sent[:n], heard[:n], fs=FS, nperseg=segment, noverlap=segment // 2)
    sel = (f >= band[0]) & (f <= band[1])
    c = np.clip(c[sel], 0.0, 1.0 - 1e-9)
    s = c / (1.0 - c)
    rate = float(np.trapezoid(np.log2(1.0 + s), f[sel]))
    width = band[1] - band[0]
    eff = 10.0 * math.log10(2.0 ** (rate / width) - 1.0)
    return SndrReading(
        np.asarray(f[sel], dtype=np.float64),
        np.asarray(10.0 * np.log10(s), dtype=np.float64),
        rate,
        eff,
    )


def response(
    sent: FloatArray,
    heard: FloatArray,
    band: tuple[float, float],
    segment: int = SEGMENT,
    reference_hz: float = 1000.0,
) -> tuple[FloatArray, FloatArray]:
    """The linear response S_xy/S_xx over ``band`` in dB, 0 dB at ``reference_hz``."""
    n = min(len(sent), len(heard))
    f, sxy = signal.csd(sent[:n], heard[:n], fs=FS, nperseg=segment, noverlap=segment // 2)
    _, sxx = signal.welch(sent[:n], fs=FS, nperseg=segment, noverlap=segment // 2)
    h = np.abs(sxy) / np.maximum(sxx, 1e-30)
    ref = float(np.interp(reference_hz, f, h))
    sel = (f >= band[0]) & (f <= band[1])
    return (
        np.asarray(f[sel], dtype=np.float64),
        np.asarray(20.0 * np.log10(h[sel] / ref), dtype=np.float64),
    )


def level_db(x: FloatArray, reference_rms: float = 1.0 / math.sqrt(2.0)) -> float:
    """RMS of ``x`` in dB against ``reference_rms`` (by default a full-scale sine)."""
    return 10.0 * math.log10(float(np.mean(np.asarray(x) ** 2)) / reference_rms**2)
