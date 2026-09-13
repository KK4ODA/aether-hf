"""Legacy constellation mapper/demapper (``dsp/modulation.py``) — roadmap P1-2 rewrites it."""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.dsp.modulation import CONSTELLATIONS, Demapper, Mapper
from aether_model.speed_levels import BITS_PER_SYMBOL, Modulation

ALL = [m for m in Modulation if CONSTELLATIONS.get(m) is not None]


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.value)
def test_hard_round_trip_is_lossless(mod: Modulation, rng: np.random.Generator) -> None:
    bps = BITS_PER_SYMBOL[mod]
    bits = rng.integers(0, 2, 200 * bps).astype(np.int8)
    assert np.array_equal(Demapper(mod).hard_demap(Mapper(mod).map(bits)), bits)


@pytest.mark.parametrize("mod", ALL, ids=lambda m: m.value)
def test_constellation_has_unit_average_power(mod: Modulation) -> None:
    c = CONSTELLATIONS[mod]
    assert np.mean(np.abs(c) ** 2) == pytest.approx(1.0, abs=1e-9)
    assert len(c) == 2 ** BITS_PER_SYMBOL[mod]


def test_llr_sign_convention_positive_means_zero() -> None:
    mod = Modulation.QPSK
    bits = np.array([0, 0, 0, 1, 1, 0, 1, 1], dtype=np.int8)
    llr = Demapper(mod).soft_demap(Mapper(mod).map(bits), noise_var=0.1)
    assert np.array_equal((llr < 0).astype(np.int8), bits)
    assert np.all(np.abs(llr) > 1.0)


def _nearest_neighbour_hamming_violations(c: np.ndarray) -> int:
    d = np.abs(c[:, None] - c[None, :])
    np.fill_diagonal(d, np.inf)
    dmin = d.min()
    i, j = np.where(np.isclose(d, dmin))
    return int(sum(bin(a ^ b).count("1") != 1 for a, b in zip(i, j, strict=True) if a < b))


@pytest.mark.parametrize("mod", [Modulation.BPSK, Modulation.QPSK], ids=lambda m: m.value)
def test_psk_labelling_is_gray(mod: Modulation) -> None:
    assert _nearest_neighbour_hamming_violations(CONSTELLATIONS[mod]) == 0


@pytest.mark.audit
@audit_xfail("§2 dsp/modulation.py", "1-D Gray code applied to a 2-D grid; 8-PSK natural binary")
@pytest.mark.parametrize(
    "mod",
    [Modulation.PSK8, Modulation.QAM16, Modulation.QAM32, Modulation.QAM64, Modulation.QAM128],
    ids=lambda m: m.value,
)
def test_higher_order_labelling_is_gray(mod: Modulation) -> None:
    """Every pair of nearest neighbours must differ in exactly one bit (BICM requirement)."""
    assert _nearest_neighbour_hamming_violations(CONSTELLATIONS[mod]) == 0


@pytest.mark.audit
@audit_xfail("§2 dsp/modulation.py", "soft demapper is a pure-Python O(M·bps) loop per symbol")
def test_soft_demapper_is_fast_enough_for_real_time() -> None:
    """One second of 64-QAM at the v1 symbol rate (≈1 350 symbols) must demap in < 50 ms."""
    import time

    mod = Modulation.QAM64
    syms = Mapper(mod).map(np.random.default_rng(0).integers(0, 2, 6 * 1350).astype(np.int8))
    t0 = time.perf_counter()
    Demapper(mod).soft_demap(syms, noise_var=0.1)
    assert time.perf_counter() - t0 < 0.05
