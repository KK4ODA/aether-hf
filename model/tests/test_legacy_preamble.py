"""Legacy preamble generator/detector (``dsp/preamble.py``) — roadmap P1-5 redesigns it."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.constants import BASEBAND_RATE, CP_DEFAULT_SAMPLES, FFT_SIZE_W
from aether_model.dsp.preamble import PreambleDetector, PreambleGenerator, zadoff_chu

SYMBOL = FFT_SIZE_W + CP_DEFAULT_SAMPLES


def test_zadoff_chu_has_constant_amplitude() -> None:
    np.testing.assert_allclose(np.abs(zadoff_chu(128, 29)), 1.0)


@pytest.mark.audit
@audit_xfail(
    "§2 dsp/preamble.py",
    "zadoff_chu() uses n(n+1)/N for even N and n(n+2)/(N+1) for odd N; neither is a ZC sequence",
)
@pytest.mark.parametrize("length", [128, 127])
def test_zadoff_chu_has_ideal_periodic_autocorrelation(length: int) -> None:
    """A true Zadoff–Chu sequence (u coprime with N) has zero periodic autocorrelation off-peak."""
    zc = zadoff_chu(length, 29)
    acf = np.abs(np.fft.ifft(np.fft.fft(zc) * np.conj(np.fft.fft(zc))))
    assert acf[0] == pytest.approx(length)
    assert acf[1:].max() < 1e-9


def test_preamble_has_the_documented_length() -> None:
    gen = PreambleGenerator("wide")
    assert len(gen.generate()) == 4 * SYMBOL
    assert len(gen.generate_short()) == 2 * SYMBOL


def _padded(pre: np.ndarray, before: int = 500, after: int = 500) -> np.ndarray:
    return np.concatenate([np.zeros(before), pre, np.zeros(after)])


@pytest.mark.audit
@audit_xfail(
    "§2 dsp/preamble.py",
    "ZC time/frequency ambiguity and 73-sample search step: wrong offset with no noise",
)
def test_detector_finds_the_true_start_without_noise() -> None:
    rx = _padded(PreambleGenerator("wide").generate())
    detected, offset, _ = PreambleDetector("wide").detect(rx, threshold=0.3)
    assert detected
    assert abs(offset - 500) <= CP_DEFAULT_SAMPLES // 2


@pytest.mark.audit
@audit_xfail("§2 dsp/preamble.py", "coarse CFO estimate is off by 150–425 Hz even without noise")
@pytest.mark.parametrize("cfo_hz", [0.0, -100.0, 200.0])
def test_detector_estimates_cfo_within_one_sweep_step(cfo_hz: float) -> None:
    rx = _padded(PreambleGenerator("wide").generate())
    t = np.arange(len(rx)) / BASEBAND_RATE
    _, _, est = PreambleDetector("wide").detect(rx * np.exp(2j * np.pi * cfo_hz * t), threshold=0.3)
    assert abs(est - cfo_hz) <= 25.0


@pytest.mark.audit
@audit_xfail(
    "§2 dsp/preamble.py", "preamble occupies 6–12 kHz; only ~25 % of its energy is inside ±1.5 kHz"
)
def test_preamble_energy_is_inside_the_channel() -> None:
    pre = PreambleGenerator("wide").generate()
    spectrum = np.abs(np.fft.fft(pre)) ** 2
    f = np.fft.fftfreq(len(pre), 1 / BASEBAND_RATE)
    assert spectrum[np.abs(f) <= 1500.0].sum() / spectrum.sum() > 0.95


@pytest.mark.audit
@audit_xfail(
    "§2 dsp/preamble.py", "estimate_fine_cfo multiplies views of the caller's buffer in place"
)
def test_fine_cfo_estimator_does_not_mutate_its_input() -> None:
    rx = _padded(PreambleGenerator("wide").generate())
    before = rx.copy()
    PreambleDetector("wide").estimate_fine_cfo(rx, offset=500, coarse_cfo=25.0)
    np.testing.assert_array_equal(rx, before)
