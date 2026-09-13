"""Shared fixtures and helpers for the Aether reference-model tests.

Two marker conventions (see ``pyproject.toml``):

* ``@pytest.mark.audit`` + ``xfail(strict=True)`` documents a defect from ``docs/AUDIT.md``.
  The test encodes the *correct* behaviour; it fails today and must be un-marked in the
  same PR that fixes the defect (strict mode makes an unexpected pass a failure, so the
  marker cannot silently outlive the bug).
* ``@pytest.mark.slow`` for anything over ~10 s; CI runs ``-m "not slow"`` on every push
  and the full set nightly.
"""

from __future__ import annotations

import numpy as np
import pytest


@pytest.fixture
def rng() -> np.random.Generator:
    return np.random.default_rng(2026)


def tone(freq_hz: float, fs: float, n: int, power: float = 1.0) -> np.ndarray:
    """Complex tone of the given mean power."""
    t = np.arange(n) / fs
    return np.sqrt(power) * np.exp(2j * np.pi * freq_hz * t)


def phase_slope_frequency(x: np.ndarray, fs: float) -> float:
    """Frequency of a (nearly) pure complex tone from the mean phase increment."""
    return float(np.angle(np.sum(x[1:] * np.conj(x[:-1]))) * fs / (2 * np.pi))


def db(x: float) -> float:
    return 10.0 * np.log10(x)


def audit_xfail(section: str, reason: str) -> pytest.MarkDecorator:
    """``xfail(strict=True)`` that names the audit finding it tracks."""
    return pytest.mark.xfail(strict=True, reason=f"AUDIT {section}: {reason}")
