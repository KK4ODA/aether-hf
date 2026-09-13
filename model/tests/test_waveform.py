"""ADR-0002 waveform numerology: derived quantities must match the recorded decision."""

from __future__ import annotations

from fractions import Fraction

import pytest

from aether_model.waveform import (
    NARROW_500,
    WIDE_2300,
    WIDE_2750,
    Bandwidth,
    Modulation,
    WaveformParams,
)


def test_wide_2300_numerology_matches_adr_0002() -> None:
    w = WIDE_2300
    assert w.subcarrier_spacing_hz == 40.0
    assert w.useful_symbol_s == pytest.approx(0.025)
    assert w.cp_s == pytest.approx(0.006)
    assert w.symbol_samples == 248
    assert w.symbol_rate_bd == pytest.approx(32.258, abs=1e-3)
    assert w.n_carriers == 57
    assert w.occupied_bandwidth_hz == 2280.0
    assert w.n_pilot_carriers == 15  # 0, 4, …, 56 — both edges are pilots
    assert w.n_data_carriers == 42
    assert w.resample_factor == 6


def test_other_bandwidths() -> None:
    assert WIDE_2750.n_carriers == 68
    assert WIDE_2750.occupied_bandwidth_hz <= 2750.0
    assert NARROW_500.n_carriers == 12
    assert NARROW_500.n_data_carriers == 8


def test_cp_covers_itu_poor_delay_spread() -> None:
    assert WIDE_2300.effective_cp_s >= 2.0e-3 * 2  # 2 ms delay spread with 2× margin
    assert WIDE_2300.effective_cp_s == pytest.approx(5e-3)


def test_raw_rates_are_in_varas_class() -> None:
    w = WIDE_2300
    assert w.raw_bit_rate(Modulation.QPSK, Fraction(1, 2)) == pytest.approx(1185, abs=5)
    assert w.raw_bit_rate(Modulation.QAM16, Fraction(3, 4)) == pytest.approx(3556, abs=5)
    assert w.raw_bit_rate(Modulation.QAM64, Fraction(5, 6)) == pytest.approx(5927, abs=5)
    assert 200 < w.raw_bit_rate(Modulation.BPSK, Fraction(1, 5)) < 260


def test_upper_edge_pilot_is_added_when_off_grid() -> None:
    w = WaveformParams(Bandwidth.WIDE_2300, pilot_carrier_spacing=5)
    # carriers 0..56; grid pilots at 0,5,…,55 (12) plus the edge 56
    assert w.n_pilot_carriers == 13


def test_non_integer_resample_factor_is_rejected() -> None:
    with pytest.raises(ValueError):
        _ = WaveformParams(fs_baseband=9000.0).resample_factor
