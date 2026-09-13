"""PAPR measurement and peak reduction (roadmap P2-4, ADR-0004)."""

from __future__ import annotations

import numpy as np
import pytest

from aether_model.frame.modes import LONG, MODES
from aether_model.phy.papr import (
    CLIP_TARGET_DB,
    CLIP_TARGET_DENSE_DB,
    ClipAndFilter,
    ToneReservation,
    clip_target_db,
    evm_db,
    out_of_band_db,
    papr_db,
)
from aether_model.phy.pipeline import Modem
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import WIDE_2300 as P


@pytest.fixture(scope="module")
def burst() -> np.ndarray:
    """Several raw (unclipped) frames end to end."""
    modem = Modem(P)
    modem.tx = FrameTransmitter(P, papr_reduction=False)
    rng = np.random.default_rng(7)
    out = []
    for _ in range(4):
        n = modem.payload_bytes(MODES[4])
        payload = rng.integers(0, 256, n, dtype=np.uint8).tobytes()
        out.append(modem.data_burst(payload, MODES[4]))
    return np.concatenate(out)


def test_papr_of_raw_ofdm_is_about_ten_db(burst: np.ndarray) -> None:
    assert 9.0 < papr_db(burst) < 11.5


def test_clip_and_filter_hits_its_target_without_splattering(burst: np.ndarray) -> None:
    reference = ClipAndFilter(P, target_papr_db=99.0, iterations=1).process(burst)
    for target in (7.0, 6.0, 5.0):
        y = ClipAndFilter(P, target_papr_db=target, iterations=4).process(burst)
        assert papr_db(y) < target + 1.2, target  # filtering regrows the peak a little
        assert papr_db(y) < papr_db(reference) - 2.0, target
        # the whole point of the *filter* half: no more splatter than the clean waveform
        assert out_of_band_db(y, P) <= out_of_band_db(reference, P) + 1.0, target
        # average power is preserved, so this is a fair peak-for-peak comparison
        assert abs(np.mean(np.abs(y) ** 2) - np.mean(np.abs(reference) ** 2)) < 0.02


def test_harder_clipping_costs_more_evm(burst: np.ndarray) -> None:
    reference = ClipAndFilter(P, target_papr_db=99.0, iterations=1).process(burst)
    evms = [
        evm_db(reference, ClipAndFilter(P, target_papr_db=t, iterations=4).process(burst))
        for t in (7.0, 6.0, 5.0, 4.0)
    ]
    assert evms == sorted(evms)  # lower target → more in-band error (less negative dB)
    assert evms[0] < -28.0 and evms[-1] > -25.0


def test_clip_target_policy_protects_the_amplitude_modulated_modes() -> None:
    """PSK absorbs the clipping distortion; QAM carries information in amplitude and does
    not, so 16-QAM and 64-QAM get the gentler target (ADR-0004)."""
    for mode in MODES:
        bits = mode.modulation.bits_per_symbol
        expected = CLIP_TARGET_DENSE_DB if bits >= 4 else CLIP_TARGET_DB
        assert clip_target_db(bits) == expected
    assert clip_target_db(1) == CLIP_TARGET_DB  # BPSK
    assert clip_target_db(3) == CLIP_TARGET_DB  # 8-PSK
    assert clip_target_db(4) == CLIP_TARGET_DENSE_DB  # 16-QAM


@pytest.mark.parametrize("mode_idx", [0, 4, 10, 13])
def test_transmitter_meets_the_adr_target(mode_idx: int) -> None:
    modem = Modem(P)
    rng = np.random.default_rng(11)
    payload = rng.integers(0, 256, modem.payload_bytes(MODES[mode_idx]), dtype=np.uint8).tobytes()
    bb = modem.data_burst(payload, MODES[mode_idx])
    target = clip_target_db(MODES[mode_idx].modulation.bits_per_symbol)
    assert papr_db(bb) < target + 1.5, (mode_idx, papr_db(bb))
    assert abs(float(np.mean(np.abs(bb) ** 2)) - 1.0) < 0.1  # unit average power preserved


def test_peak_reduction_can_be_switched_off() -> None:
    rng = np.random.default_rng(5)
    plain = Modem(P)
    plain.tx = FrameTransmitter(P, papr_reduction=False)
    clipped = Modem(P)
    payload = rng.integers(0, 256, plain.payload_bytes(MODES[4]), dtype=np.uint8).tobytes()
    assert (
        papr_db(plain.data_burst(payload, MODES[4]))
        > papr_db(clipped.data_burst(payload, MODES[4])) + 2.0
    )


def test_tone_reservation_buys_little_for_what_it_costs() -> None:
    """Recorded because ADR-0004 rejects it on these numbers: reserving carriers takes
    payload away and barely moves the peak."""
    tr = ToneReservation(P, n_reserved=8, target_papr_db=5.0, iterations=10)
    assert tr.payload_cost > 0.15  # 8 of 42 data carriers
    mod = Modem(P).tx.mod
    rng = np.random.default_rng(3)
    before, after = [], []
    for _ in range(40):
        vals = mod.symbol_values(rng.standard_normal(42) + 1j * rng.standard_normal(42))
        body = mod.to_time(vals)[P.cp_samples : P.cp_samples + P.fft_size]
        before.append(papr_db(body))
        after.append(papr_db(tr.process_symbol(body)))
    assert 0.0 < np.mean(before) - np.mean(after) < 1.5


def test_frame_layout_unchanged_by_peak_reduction() -> None:
    """Peak reduction must not alter the air interface: same length, same timing."""
    rng = np.random.default_rng(2)
    modem = Modem(P)
    payload = rng.integers(0, 256, modem.payload_bytes(MODES[4]), dtype=np.uint8).tobytes()
    assert len(modem.data_burst(payload, MODES[4])) == LONG.samples + P.taper_samples
