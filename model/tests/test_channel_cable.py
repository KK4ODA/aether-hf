"""The A/B bench's cable channel (P9-1): audio in, the model's channel out, block by block."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import channel_cable  # noqa: E402

RATE = channel_cable.AUDIO_RATE
BLOCK = 960  # 20 ms


def _blocks(audio: np.ndarray, cable: channel_cable.CableChannel) -> np.ndarray:
    out = [cable.process(audio[i : i + BLOCK]) for i in range(0, len(audio) - BLOCK + 1, BLOCK)]
    return np.concatenate(out)


def _band_power(audio: np.ndarray, low_hz: float, high_hz: float) -> float:
    """Mean power of what lies between two frequencies, from a plain periodogram."""
    spectrum = np.fft.rfft(audio * np.hanning(len(audio)))
    freqs = np.fft.rfftfreq(len(audio), 1.0 / RATE)
    window_gain = np.mean(np.hanning(len(audio)) ** 2)
    band = (freqs >= low_hz) & (freqs <= high_hz)
    return float(2.0 * np.sum(np.abs(spectrum[band]) ** 2) / (len(audio) ** 2 * window_gain))


def _multitone(seconds: float, rms: float, seed: int = 3) -> np.ndarray:
    """A signal that fills 600–2400 Hz like a modem's: forty tones at random phases."""
    rng = np.random.default_rng(seed)
    t = np.arange(int(seconds * RATE)) / RATE
    tones = np.linspace(600.0, 2400.0, 40)
    phases = rng.uniform(0, 2 * np.pi, len(tones))
    x = sum(np.cos(2 * np.pi * f * t + p) for f, p in zip(tones, phases, strict=True))
    return rms * x / np.sqrt(np.mean(x**2))


def test_silence_in_gives_the_noise_the_snr_says_out() -> None:
    # the reference is a −15 dBFS transmit level; at 10 dB the noise in a 3 kHz band is
    # −25 dBFS, whether or not anything is being sent
    cable = channel_cable.CableChannel("awgn", snr_db=10.0, signal_dbfs=-15.0, seed=5)
    out = _blocks(np.zeros(RATE * 4), cable)
    settled = out[RATE:]  # past the filters' start-up
    noise_db = 10 * np.log10(_band_power(settled, 300.0, 3300.0))
    assert abs(noise_db - (-25.0)) < 0.6, noise_db
    # and nothing above the passband a transceiver would pass
    above = 10 * np.log10(_band_power(settled, 4000.0, 8000.0) + 1e-30)
    assert above < noise_db - 30, above
    assert cable.noise_dbfs_3k == pytest.approx(-25.0)


def test_a_signal_comes_through_at_its_level_and_frequency() -> None:
    cable = channel_cable.CableChannel("awgn", snr_db=None, signal_dbfs=-15.0)
    rms = 10 ** (-15 / 20)
    t = np.arange(RATE * 2) / RATE
    tone = rms * np.sqrt(2) * np.cos(2 * np.pi * 1500.0 * t)
    out = _blocks(tone, cable)
    settled = out[RATE // 2 :]
    out_db = 10 * np.log10(np.mean(settled**2))
    assert abs(out_db - (-15.0)) < 0.3, out_db
    spectrum = np.abs(np.fft.rfft(settled))
    peak_hz = np.fft.rfftfreq(len(settled), 1.0 / RATE)[np.argmax(spectrum)]
    assert abs(peak_hz - 1500.0) < 2.0, peak_hz
    # a modem-shaped signal keeps its level too: the fading is off and the passband flat
    cable = channel_cable.CableChannel("awgn", snr_db=None, signal_dbfs=-15.0)
    out = _blocks(_multitone(2.0, rms), cable)
    out_db = 10 * np.log10(np.mean(out[RATE // 2 :] ** 2))
    assert abs(out_db - (-15.0)) < 0.5, out_db


def test_a_fading_profile_keeps_the_mean_level_and_the_snr() -> None:
    rms = 10 ** (-15 / 20)
    signal_in = _multitone(12.0, rms)
    quiet = channel_cable.CableChannel("poor", snr_db=None, signal_dbfs=-15.0, seed=11)
    faded = _blocks(signal_in, quiet)
    out_db = 10 * np.log10(np.mean(faded[RATE:] ** 2))
    assert abs(out_db - (-15.0)) < 1.5, out_db
    # the same seed with noise on: what was added is the noise, at the level the SNR says
    noisy = channel_cable.CableChannel("poor", snr_db=6.0, signal_dbfs=-15.0, seed=11)
    with_noise = _blocks(signal_in, noisy)
    added = with_noise - faded
    noise_db = 10 * np.log10(_band_power(added[RATE:], 300.0, 3300.0))
    assert abs(noise_db - (-21.0)) < 0.8, noise_db


def test_auto_level_follows_the_burst_it_is_given() -> None:
    cable = channel_cable.CableChannel("awgn", snr_db=10.0, signal_dbfs=-15.0, auto_level=True)
    assert cable.reference_dbfs == pytest.approx(-15.0)
    _blocks(_multitone(3.0, 10 ** (-21 / 20)), cable)
    # a burst six decibels lower moved the reference most of the way there
    assert -21.5 < cable.reference_dbfs < -19.0, cable.reference_dbfs
    assert cable.active_blocks > 100
    # silence leaves it be, bar the burst's filter tail in the first quiet block
    before = cable.reference_dbfs
    _blocks(np.zeros(RATE), cable)
    assert abs(cable.reference_dbfs - before) < 0.5, (before, cable.reference_dbfs)
    # and blocks must be whole decimation steps
    with pytest.raises(ValueError):
        cable.process(np.zeros(961))
