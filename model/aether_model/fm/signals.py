"""Known signals for measuring an FM radio, on the model and on the bench (ADR-0115 §2.5).

The test signal must have the data's spectrum and peak statistics: a tone would mislead the
limiter (proposal §E.2), so the drive is measured with a single carrier of QAM symbols, the
shape the proposal recommends. Chirps mark the start and end so a recording can be aligned and
its clock offset read.
"""

from __future__ import annotations

import math

import numpy as np
from numpy.typing import NDArray

from aether_model.fm.channel import FS

FloatArray = NDArray[np.float64]
ComplexArray = NDArray[np.complex128]


def qam_symbols(order: int, n: int, rng: np.random.Generator) -> ComplexArray:
    """``n`` square-QAM symbols of unit mean power (``order`` 4, 16, 64 or 256)."""
    k = math.isqrt(order)
    if k * k != order:
        raise ValueError(f"square QAM needs a square order, not {order}")
    levels = np.arange(k) * 2 - (k - 1)
    s = rng.choice(levels, n) + 1j * rng.choice(levels, n)
    return np.asarray(s / math.sqrt((order - 1) * 2.0 / 3.0), dtype=np.complex128)


def rrc_taps(alpha: float, sps: int, span: int) -> FloatArray:
    """A root-raised-cosine filter of roll-off ``alpha``, ``sps`` samples a symbol, ``span``
    symbols each side, unit energy."""
    t = np.arange(-span * sps, span * sps + 1) / sps
    h = np.zeros_like(t)
    for i, ti in enumerate(t):
        if abs(ti) < 1e-12:
            h[i] = 1.0 - alpha + 4.0 * alpha / math.pi
        elif alpha > 0 and abs(abs(4.0 * alpha * ti) - 1.0) < 1e-9:
            h[i] = (alpha / math.sqrt(2.0)) * (
                (1.0 + 2.0 / math.pi) * math.sin(math.pi / (4.0 * alpha))
                + (1.0 - 2.0 / math.pi) * math.cos(math.pi / (4.0 * alpha))
            )
        else:
            h[i] = (
                math.sin(math.pi * ti * (1.0 - alpha))
                + 4.0 * alpha * ti * math.cos(math.pi * ti * (1.0 + alpha))
            ) / (math.pi * ti * (1.0 - (4.0 * alpha * ti) ** 2))
    return np.asarray(h / math.sqrt(float(np.sum(h**2))), dtype=np.float64)


def single_carrier(
    symbol_rate: float,
    centre_hz: float,
    seconds: float,
    rng: np.random.Generator,
    *,
    order: int = 16,
    alpha: float = 0.2,
) -> FloatArray:
    """Real audio of QAM symbols on one carrier, root-raised-cosine shaped, RMS 1.

    The voice path's shape is 2 000 Bd at 1 650 Hz (450–2 850 Hz); the flat port's 4 000 Bd at
    2 700 Hz (300–5 100 Hz). ``FS / symbol_rate`` must be a whole number."""
    sps = round(FS / symbol_rate)
    if abs(sps * symbol_rate - FS) > 1e-6:
        raise ValueError("the sample rate must be a whole number of samples a symbol")
    n = math.ceil(seconds * symbol_rate)
    up = np.zeros(n * sps, dtype=np.complex128)
    up[::sps] = qam_symbols(order, n, rng)
    bb = np.convolve(up, rrc_taps(alpha, sps, 10), mode="same")
    t = np.arange(len(bb)) / FS
    x = np.real(bb * np.exp(2j * math.pi * centre_hz * t))
    return np.asarray(x / math.sqrt(float(np.mean(x**2))), dtype=np.float64)


def chirp(f0: float, f1: float, seconds: float) -> FloatArray:
    """A linear sweep from ``f0`` to ``f1``, RMS 1, with 5 ms raised-cosine ends: a marker
    whose cross-correlation has one sharp peak."""
    n = round(seconds * FS)
    t = np.arange(n) / FS
    x = np.sin(2.0 * math.pi * (f0 * t + 0.5 * (f1 - f0) / seconds * t**2))
    ramp = round(0.005 * FS)
    w = np.ones(n)
    edge = 0.5 * (1.0 - np.cos(np.pi * np.arange(ramp) / ramp))
    w[:ramp] = edge
    w[n - ramp :] = edge[::-1]
    x = x * w
    return np.asarray(x / math.sqrt(float(np.mean(x**2))), dtype=np.float64)


def tone(freq_hz: float, seconds: float, amplitude: float = 1.0) -> FloatArray:
    """A sine of the given peak amplitude."""
    t = np.arange(round(seconds * FS)) / FS
    return np.asarray(amplitude * np.sin(2.0 * math.pi * freq_hz * t), dtype=np.float64)
