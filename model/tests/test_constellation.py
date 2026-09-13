"""P1-2: Gray-labelled constellations and the vectorized max-log demapper."""

from __future__ import annotations

import time

import numpy as np
import pytest

from aether_model.channel import complex_normal
from aether_model.phy.constellation import Constellation, constellation
from aether_model.waveform import Modulation

ALL = list(Modulation)


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.name)
def test_unit_power_and_size(mod: Modulation) -> None:
    c = constellation(mod)
    assert len(c.points) == 2**mod.bits_per_symbol
    assert np.mean(np.abs(c.points) ** 2) == pytest.approx(1.0, abs=1e-12)
    assert len(set(np.round(c.points, 9))) == len(c.points)  # all distinct


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.name)
def test_labelling_is_gray(mod: Modulation) -> None:
    assert constellation(mod).is_gray()


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.name)
def test_map_hard_round_trip(mod: Modulation, rng: np.random.Generator) -> None:
    c = constellation(mod)
    bits = rng.integers(0, 2, 500 * mod.bits_per_symbol).astype(np.uint8)
    assert np.array_equal(c.hard(c.map(bits)), bits)


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.name)
def test_llr_sign_matches_bits_and_hard_decisions(
    mod: Modulation, rng: np.random.Generator
) -> None:
    c = constellation(mod)
    bits = rng.integers(0, 2, 400 * mod.bits_per_symbol).astype(np.uint8)
    y = c.map(bits) + 0.05 * complex_normal(rng, 400)
    llr = c.llr(y, noise_var=0.05**2)
    assert np.array_equal((llr < 0).astype(np.uint8), bits)
    assert np.array_equal((llr < 0).astype(np.uint8), c.hard(y))


def test_bpsk_llr_is_4_re_y_over_sigma2(rng: np.random.Generator) -> None:
    c = constellation(Modulation.BPSK)
    y = complex_normal(rng, 100)
    np.testing.assert_allclose(c.llr(y, 0.5), 4.0 * y.real / 0.5, atol=1e-12)


def test_qpsk_llr_separates_quadratures(rng: np.random.Generator) -> None:
    c = constellation(Modulation.QPSK)
    y = complex_normal(rng, 50)
    llr = c.llr(y, 0.3).reshape(-1, 2)
    np.testing.assert_allclose(llr[:, 0], 4.0 * y.real / np.sqrt(2) / 0.3, atol=1e-12)
    np.testing.assert_allclose(llr[:, 1], 4.0 * y.imag / np.sqrt(2) / 0.3, atol=1e-12)


def test_per_symbol_noise_variance_scales_llrs(rng: np.random.Generator) -> None:
    c = constellation(Modulation.QAM16)
    y = c.map(rng.integers(0, 2, 40).astype(np.uint8))
    base = c.llr(y, 1.0)
    scaled = c.llr(y, np.full(10, 0.25))
    np.testing.assert_allclose(scaled, 4.0 * base)


def test_qam16_matches_ts38211_example() -> None:
    """TS 38.211 §5.1.4: b = 0000 → (1+1j)·3/√10 … outermost corner; b = 0010 → (1+3j)/√10."""
    c = constellation(Modulation.QAM16)
    assert c.map(np.array([0, 0, 0, 0], dtype=np.uint8))[0] == pytest.approx((1 + 1j) / np.sqrt(10))
    assert c.map(np.array([0, 0, 1, 1], dtype=np.uint8))[0] == pytest.approx((3 + 3j) / np.sqrt(10))
    assert c.map(np.array([1, 0, 1, 0], dtype=np.uint8))[0] == pytest.approx(
        (-3 + 1j) / np.sqrt(10)
    )


def test_demapper_speed_is_real_time_capable() -> None:
    """One second of 64-QAM at the v1 symbol rate (≈1 350 symbols) demaps in < 50 ms."""
    c = constellation(Modulation.QAM64)
    y = c.map(np.random.default_rng(0).integers(0, 2, 6 * 1350).astype(np.uint8))
    t0 = time.perf_counter()
    c.llr(y, 0.1)
    assert time.perf_counter() - t0 < 0.05


def test_uncoded_bpsk_ber_matches_theory(rng: np.random.Generator) -> None:
    """Sanity anchor for the whole LLR/noise convention: BPSK at E_s/N_0 = 4 dB → BER ≈ 1.25 %."""
    from scipy.special import erfc

    c = constellation(Modulation.BPSK)
    n = 200_000
    bits = rng.integers(0, 2, n).astype(np.uint8)
    es_n0 = 10 ** (4 / 10)
    noise_var = 1.0 / es_n0
    y = c.map(bits) + np.sqrt(noise_var) * complex_normal(rng, n)
    ber = np.mean(c.hard(y) != bits)
    theory = 0.5 * erfc(np.sqrt(es_n0))
    assert abs(ber - theory) < 0.15 * theory


def test_constellation_cache_returns_same_instance() -> None:
    assert constellation(Modulation.QPSK) is constellation(Modulation.QPSK)
    assert isinstance(Constellation(Modulation.PSK8), Constellation)
