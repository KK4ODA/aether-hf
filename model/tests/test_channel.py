"""Calibration tests for the HF channel simulator (roadmap P0-6).

These tests are the guarantee behind every benchmark number in the project: if one of them
fails, no FER/SNR curve produced afterwards can be trusted.
"""

from __future__ import annotations

import numpy as np
import pytest
from conftest import db, phase_slope_frequency, tone
from scipy import signal

from aether_model.channel import (
    PROFILES,
    SNR_REFERENCE_BANDWIDTH_HZ,
    Awgn,
    ChannelConfig,
    ChannelProfile,
    FrequencyOffset,
    HfChannel,
    Interferer,
    InterfererConfig,
    RayleighFadingProcess,
    SampleRateOffset,
    SaturatingPa,
    WattersonChannel,
    make_channel,
    noise_power_for_snr,
)

FS = 8000.0


# ── Profiles ──────────────────────────────────────────────────────────


@pytest.mark.parametrize(
    ("name", "delay_ms", "spread_hz"),
    [
        ("good", 0.5, 0.1),
        ("moderate", 1.0, 0.5),
        ("poor", 2.0, 1.0),
        ("nvis", 7.0, 1.0),
        ("flutter", 0.5, 10.0),
    ],
)
def test_profiles_match_itu_r_f1487(name: str, delay_ms: float, spread_hz: float) -> None:
    p = PROFILES[name]
    assert p.delays_s == (0.0, delay_ms * 1e-3)
    assert p.doppler_spread_hz == (spread_hz, spread_hz)
    assert p.tap_powers_db == (0.0, 0.0)


def test_profile_validation() -> None:
    with pytest.raises(ValueError):
        ChannelProfile("bad", (0.0, 1e-3), (1.0,), (0.0, 0.0))
    with pytest.raises(ValueError):
        make_channel("no-such-profile")


# ── Fading process ────────────────────────────────────────────────────


def test_fading_has_unit_mean_power_and_rayleigh_statistics() -> None:
    """E|g|² = 1 and the envelope is Rayleigh: P(|g|² < -10 dB) = 1 - e^-0.1 = 9.52 %."""
    proc = RayleighFadingProcess(FS, doppler_spread_hz=1.0, rng=np.random.default_rng(7))
    g = proc.lowrate(1_000_000)  # 10 000 s at fs_low = 100 Hz → ~10⁴ independent fades
    p = np.abs(g) ** 2
    assert abs(p.mean() - 1.0) < 0.04
    assert abs(np.mean(p < 0.1) - (1 - np.exp(-0.1))) < 0.01
    assert abs(np.mean(p < 0.01) - (1 - np.exp(-0.01))) < 0.004


def test_fading_doppler_spectrum_width_is_half_the_f1487_spread() -> None:
    """A '1 Hz' F.1487 channel has a Gaussian Doppler spectrum with σ = 0.5 Hz."""
    proc = RayleighFadingProcess(FS, doppler_spread_hz=1.0, rng=np.random.default_rng(3))
    g = proc.lowrate(400_000)
    f, psd = signal.welch(g, fs=proc.fs_low, nperseg=4096, return_onesided=False)
    sigma = np.sqrt(np.sum(f**2 * psd) / np.sum(psd))
    assert abs(sigma - 0.5) < 0.05


def test_fading_interpolated_stream_keeps_unit_power() -> None:
    proc = RayleighFadingProcess(FS, doppler_spread_hz=30.0, rng=np.random.default_rng(11))
    g = proc.next(2_000_000)  # 250 s of a 30 Hz-spread process → thousands of fades
    assert abs(np.mean(np.abs(g) ** 2) - 1.0) < 0.06


def test_fading_is_continuous_across_blocks() -> None:
    a = RayleighFadingProcess(FS, 1.0, np.random.default_rng(5))
    b = RayleighFadingProcess(FS, 1.0, np.random.default_rng(5))
    whole = a.next(5000)
    split = np.concatenate([b.next(1234), b.next(1), b.next(3765)])
    np.testing.assert_allclose(split, whole, rtol=0, atol=1e-12)


def test_static_tap_is_constant_unity() -> None:
    proc = RayleighFadingProcess(FS, 0.0, np.random.default_rng(0))
    assert np.all(proc.next(100) == 1.0)


# ── Watterson ─────────────────────────────────────────────────────────


def test_static_two_ray_channel_is_exact_delay_sum(rng: np.random.Generator) -> None:
    profile = ChannelProfile("static 2-ray", (0.0, 1e-3), (0.0, 0.0), (0.0, 0.0))
    ch = WattersonChannel(profile, FS)
    x = rng.standard_normal(500) + 1j * rng.standard_normal(500)
    y = ch.process(x)
    delayed = np.concatenate([np.zeros(8), x[:-8]])  # 1 ms at 8 kHz = 8 samples
    np.testing.assert_allclose(y, (x + delayed) / np.sqrt(2), atol=1e-12)


def test_watterson_tap_powers_normalise_to_unity() -> None:
    profile = ChannelProfile("unequal", (0.0, 1e-3), (0.0, 0.0), (0.0, -6.0))
    ch = WattersonChannel(profile, FS)
    assert abs(float(np.sum(ch._amps**2)) - 1.0) < 1e-12


def test_faded_channel_preserves_mean_power_long_term() -> None:
    ch = WattersonChannel("poor", FS, seed=1)
    x = tone(500.0, FS, 4_000_000)  # 500 s
    y = ch.process(x)
    assert abs(db(float(np.mean(np.abs(y) ** 2)))) < 0.5


# ── Noise and SNR calibration ─────────────────────────────────────────


def test_noise_power_formula_3khz_reference() -> None:
    # 0 dB SNR, unit signal power, fs = 8 kHz → total noise power = 8000/3000
    assert noise_power_for_snr(0.0, 1.0, FS) == pytest.approx(FS / SNR_REFERENCE_BANDWIDTH_HZ)


def test_awgn_snr_is_calibrated_in_3khz() -> None:
    """Noise power inside ±1.5 kHz must equal P_signal / SNR to within 0.2 dB."""
    n = 1 << 16
    x = tone(700.0, FS, n)
    awgn = Awgn(FS, snr_db=10.0, seed=0)
    noise = awgn.process(x, signal_power=1.0) - x
    spectrum = np.abs(np.fft.fft(noise)) ** 2 / n**2  # power per bin
    f = np.fft.fftfreq(n, 1 / FS)
    in_band = spectrum[np.abs(f) <= SNR_REFERENCE_BANDWIDTH_HZ / 2].sum()
    assert abs(db(in_band) - (-10.0)) < 0.2


def test_awgn_is_block_invariant() -> None:
    x = np.ones(3000, dtype=complex)
    a = Awgn(FS, 5.0, seed=9, impulsive_probability=0.05)
    b = Awgn(FS, 5.0, seed=9, impulsive_probability=0.05)
    whole = a.process(x, 1.0)
    split = np.concatenate([b.process(x[:1000], 1.0), b.process(x[1000:], 1.0)])
    np.testing.assert_array_equal(split, whole)


def test_impulsive_noise_hits_at_the_requested_rate() -> None:
    n = 200_000
    awgn = Awgn(FS, snr_db=20.0, seed=4, impulsive_probability=0.05, impulsive_db_above_noise=20.0)
    noise = awgn.process(np.zeros(n, dtype=complex), signal_power=1.0)
    floor = awgn.noise_power(1.0)
    hits = np.mean(np.abs(noise) ** 2 > 10 * floor)
    assert abs(hits - 0.05) < 0.01


# ── Receiver imperfections ────────────────────────────────────────────


def test_frequency_offset_shifts_a_tone_exactly() -> None:
    x = tone(1000.0, FS, 8000)
    y = FrequencyOffset(FS, 37.5).process(x)
    assert phase_slope_frequency(y, FS) == pytest.approx(1037.5, abs=1e-6)


def test_frequency_drift_is_phase_continuous_across_blocks() -> None:
    x = tone(1000.0, FS, 16000)
    a = FrequencyOffset(FS, 10.0, drift_hz_per_s=5.0)
    b = FrequencyOffset(FS, 10.0, drift_hz_per_s=5.0)
    whole = a.process(x)
    split = np.concatenate([b.process(x[:7000]), b.process(x[7000:])])
    np.testing.assert_allclose(split, whole, atol=1e-12)
    # mean instantaneous frequency over 2 s with 5 Hz/s drift: 1000 + 10 + 5
    assert phase_slope_frequency(whole, FS) == pytest.approx(1015.0, abs=0.01)


@pytest.mark.parametrize("ppm", [100.0, -100.0, 0.0])
def test_sample_rate_offset_scales_frequency_by_ppm(ppm: float) -> None:
    x = tone(1000.0, FS, 16000)
    y = SampleRateOffset(FS, ppm).process(x)[8:]
    assert phase_slope_frequency(y, FS) == pytest.approx(1000.0 * (1 + ppm * 1e-6), abs=1e-4)
    # the interpolator withholds 2 tail samples until the next block arrives
    assert abs(len(y) + 8 - 16000 / (1 + ppm * 1e-6)) <= 4


def test_sample_rate_offset_cubic_interpolation_is_accurate() -> None:
    n = 16000
    x = tone(1000.0, FS, n)
    y = SampleRateOffset(FS, 100.0).process(x)
    r = 1 + 100e-6
    expected = np.exp(2j * np.pi * 1000.0 * (np.arange(len(y)) * r) / FS)
    err = np.mean(np.abs(y[8:] - expected[8:]) ** 2)
    assert db(err) < -40.0


def test_sample_rate_offset_is_block_invariant() -> None:
    x = tone(1000.0, FS, 10000) + 0.1 * tone(-2500.0, FS, 10000)
    a = SampleRateOffset(FS, 150.0)
    b = SampleRateOffset(FS, 150.0)
    whole = a.process(x)
    split = np.concatenate([b.process(x[:3333]), b.process(x[3333:3334]), b.process(x[3334:])])
    m = min(len(whole), len(split))
    assert len(whole) - m <= 3
    np.testing.assert_allclose(split[:m], whole[:m], atol=1e-12)


def test_zero_ppm_is_identity() -> None:
    x = tone(1234.5, FS, 4000)
    y = SampleRateOffset(FS, 0.0).process(x)
    np.testing.assert_allclose(y, x[: len(y)], atol=1e-12)


# ── Transmitter imperfection ──────────────────────────────────────────


def test_rapp_pa_is_linear_for_small_signals_and_saturates() -> None:
    pa = SaturatingPa.from_backoff(signal_power=1.0, input_backoff_db=6.0, smoothness=2.0)
    small = 0.1 * np.exp(1j * np.linspace(0, 6, 50))
    np.testing.assert_allclose(pa.process(small), small, rtol=1e-4)
    huge = 100.0 * np.exp(1j * np.linspace(0, 6, 50))
    assert np.all(np.abs(pa.process(huge)) <= pa.saturation_amplitude * (1 + 1e-9))
    assert pa.saturation_amplitude == pytest.approx(10 ** (6 / 20))
    hard = SaturatingPa(1.0, smoothness=50.0)
    assert abs(hard.process(np.array([1.5 + 0j]))[0]) == pytest.approx(1.0, abs=1e-3)


# ── Interferer ────────────────────────────────────────────────────────


def test_cw_interferer_power_and_offset() -> None:
    n = 8000
    intf = Interferer(FS, offset_hz=2000.0, power_db=-10.0, kind="cw")
    y = intf.next(n, signal_power=1.0)
    spectrum = np.abs(np.fft.fft(y)) ** 2 / n**2
    f = np.fft.fftfreq(n, 1 / FS)
    assert db(spectrum[np.argmin(np.abs(f - 2000.0))]) == pytest.approx(-10.0, abs=0.05)


def test_noise_interferer_power_and_bandwidth() -> None:
    n = 1 << 17
    intf = Interferer(
        FS,
        offset_hz=1000.0,
        power_db=0.0,
        kind="noise",
        bandwidth_hz=2400.0,
        rng=np.random.default_rng(1),
    )
    y = intf.next(n, signal_power=1.0)
    assert abs(db(float(np.mean(np.abs(y) ** 2)))) < 0.3
    f, psd = signal.welch(y, fs=FS, nperseg=2048, return_onesided=False)
    inside = psd[np.abs(f - 1000.0) <= 1200.0].sum()
    assert inside / psd.sum() > 0.9


# ── Composite ─────────────────────────────────────────────────────────


def test_full_channel_is_block_invariant() -> None:
    cfg = ChannelConfig(
        profile="poor",
        snr_db=8.0,
        fs=FS,
        signal_power=1.0,
        cfo_hz=30.0,
        cfo_drift_hz_per_s=1.0,
        sro_ppm=50.0,
        impulsive_probability=0.02,
        interferer=InterfererConfig(offset_hz=-2200.0, power_db=-3.0, kind="noise"),
        pa_input_backoff_db=8.0,
        seed=42,
    )
    x = tone(600.0, FS, 12000) + 0.5 * tone(-900.0, FS, 12000)
    whole = HfChannel(cfg).process(x)
    ch = HfChannel(cfg)
    split = np.concatenate([ch.process(x[:2500]), ch.process(x[2500:2501]), ch.process(x[2501:])])
    m = min(len(whole), len(split))
    assert len(whole) - m <= 3
    np.testing.assert_allclose(split[:m], whole[:m], atol=1e-9)


def test_full_channel_snr_under_awgn_profile() -> None:
    n = 1 << 16
    x = tone(400.0, FS, n)
    y = make_channel("awgn", snr_db=0.0, fs=FS, seed=3, signal_power=1.0).process(x)
    noise = y - x
    spectrum = np.abs(np.fft.fft(noise)) ** 2 / n**2
    f = np.fft.fftfreq(n, 1 / FS)
    in_band = spectrum[np.abs(f) <= 1500.0].sum()
    assert abs(db(in_band)) < 0.2


def test_silence_without_reference_power_is_an_error() -> None:
    with pytest.raises(ValueError):
        make_channel("awgn", snr_db=10.0, fs=FS).process(np.zeros(100, dtype=complex))


def test_noise_can_be_disabled() -> None:
    x = tone(300.0, FS, 1000)
    y = make_channel("awgn", snr_db=None, fs=FS).process(x)
    np.testing.assert_array_equal(y, x)
