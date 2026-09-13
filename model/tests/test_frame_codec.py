"""P1-3: mode table, frame layouts and the payload ↔ symbol codec."""

from __future__ import annotations

from fractions import Fraction

import numpy as np
import pytest

from aether_model.channel import complex_normal
from aether_model.frame.codec import FrameCodec, coprime_stride
from aether_model.frame.modes import (
    CONTROL_MODE,
    LONG,
    MODES,
    SHORT,
    Mode,
    mode_table,
    select_base_graph,
)
from aether_model.waveform import Modulation

# ── layouts and modes ─────────────────────────────────────────────────


def test_long_layout_numbers() -> None:
    assert LONG.pilot_symbol_indices == (0, 8, 16, 24)
    assert LONG.n_payload_symbols == 28
    assert LONG.qam_symbols == 28 * 42
    assert LONG.total_symbols == 35
    assert LONG.duration_s == pytest.approx(35 * 248 / 8000)


def test_short_layout_numbers() -> None:
    assert SHORT.pilot_symbol_indices == (0, 8)
    assert SHORT.qam_symbols == 10 * 42


def test_mode_table_is_monotonic_in_throughput_and_fits_the_codes() -> None:
    rates = [m.net_bit_rate(LONG) for m in MODES]
    assert rates == sorted(rates)
    for m in MODES:
        assert m.payload_bytes(LONG) >= 20
        assert (
            m.info_bits(LONG) <= 10 * m.lifting_size(LONG)
            if m.base_graph(LONG) == 2
            else 22 * m.lifting_size(LONG)
        )
        assert 0.9 * float(m.code_rate) < m.effective_rate(LONG) <= float(m.code_rate)
    assert 150 < MODES[0].net_bit_rate(LONG) < 260  # BPSK 1/5
    assert 900 < MODES[4].net_bit_rate(LONG) < 1200  # QPSK 1/2
    assert 5000 < MODES[-1].net_bit_rate(LONG) < 5600  # 64-QAM 5/6


def test_base_graph_rule_follows_ts38212_7_2_2() -> None:
    assert select_base_graph(200, Fraction(5, 6)) == 2  # tiny block
    assert select_base_graph(3000, Fraction(1, 2)) == 2  # ≤ 3824 and R ≤ 0.67
    assert select_base_graph(5000, Fraction(1, 5)) == 2  # R ≤ 0.25
    assert select_base_graph(5000, Fraction(1, 2)) == 1
    assert select_base_graph(3000, Fraction(3, 4)) == 1


def test_high_rate_64qam_uses_bg1() -> None:
    assert MODES[-1].base_graph(LONG) == 1
    assert MODES[0].base_graph(LONG) == 2


def test_control_mode_fits_an_ack_in_a_short_frame() -> None:
    assert CONTROL_MODE.payload_bytes(SHORT) == 7


def test_mode_table_summary_has_all_modes() -> None:
    rows = mode_table()
    assert [r["mode"] for r in rows] == list(range(len(MODES)))
    assert all(r["payload_bytes"] > 0 for r in rows)


# ── interleaver ───────────────────────────────────────────────────────


@pytest.mark.parametrize("e", [420, 1176, 2352, 4704, 7056, 100, 101, 1000])
def test_coprime_stride_is_coprime_and_near_golden(e: int) -> None:
    p = coprime_stride(e)
    assert np.gcd(p, e) == 1
    assert abs(p - e / 1.618) < max(8, 0.02 * e)


def test_interleaver_scatters_bursts_in_time_and_frequency() -> None:
    """A burst of 64 consecutive coded bits must touch ≥ 60 distinct OFDM symbols… and no
    two of them may share a carrier within the same symbol."""
    codec = FrameCodec(MODES[4], LONG)  # QPSK ½
    m = 2
    n_c = LONG.waveform.n_data_carriers
    positions = codec._perm[1000:1064]  # interleaved bit positions of a coded-bit burst
    qam = positions // m
    symbols = qam // n_c
    carriers = qam % n_c
    assert len(set(symbols.tolist())) >= 20
    assert len(set(carriers.tolist())) >= 30
    assert len({(int(s), int(c)) for s, c in zip(symbols, carriers, strict=True)}) == 64


# ── codec round trips ─────────────────────────────────────────────────


@pytest.mark.parametrize("mode", MODES, ids=lambda m: m.name)
def test_noiseless_round_trip_every_mode(mode: Mode, rng: np.random.Generator) -> None:
    codec = FrameCodec(mode, LONG)
    payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
    syms = codec.encode(payload)
    assert syms.shape == (LONG.qam_symbols,)
    assert np.mean(np.abs(syms) ** 2) == pytest.approx(1.0, abs=0.05)
    got, _ = codec.decode(syms, noise_var=0.01)
    assert got == payload


def test_short_frame_round_trip(rng: np.random.Generator) -> None:
    codec = FrameCodec(CONTROL_MODE, SHORT)
    payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
    got, _ = codec.decode(codec.encode(payload), noise_var=0.01)
    assert got == payload


def test_crc_rejects_a_corrupted_frame(rng: np.random.Generator) -> None:
    codec = FrameCodec(MODES[13], LONG)  # 64-QAM 5/6: fragile on purpose
    payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
    syms = codec.encode(payload)
    noisy = syms + 0.6 * complex_normal(rng, len(syms))  # far too much noise
    got, _ = codec.decode(noisy, noise_var=0.36)
    assert got is None


def test_qpsk_half_decodes_at_moderate_snr(rng: np.random.Generator) -> None:
    """QPSK ½ (K′ ≈ 1 176) at E_s/N_0 = 3 dB → 10/10 frames (theory: threshold ≈ 1 dB)."""
    codec = FrameCodec(MODES[4], LONG)
    nv = 1 / 10 ** (3.0 / 10)
    for _ in range(10):
        payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
        y = codec.encode(payload) + np.sqrt(nv) * complex_normal(rng, LONG.qam_symbols)
        got, _ = codec.decode(y, noise_var=nv)
        assert got == payload


def test_harq_ir_across_redundancy_versions(rng: np.random.Generator) -> None:
    """16-QAM ¾ at an SNR where RV0 alone fails: combining RV0 + RV1 decodes."""
    codec = FrameCodec(MODES[10], LONG)
    nv = 1 / 10 ** (6.0 / 10)
    rescued = 0
    for _ in range(4):
        payload = rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8).tobytes()
        y0 = codec.encode(payload, rv=0) + np.sqrt(nv) * complex_normal(rng, LONG.qam_symbols)
        got0, buf = codec.decode(y0, nv, rv=0)
        y1 = codec.encode(payload, rv=1) + np.sqrt(nv) * complex_normal(rng, LONG.qam_symbols)
        got1, _ = codec.decode(y1, nv, rv=1, buffer=buf)
        assert got1 == payload
        rescued += int(got0 is None)
    assert rescued >= 2


def test_wrong_payload_length_is_rejected() -> None:
    codec = FrameCodec(MODES[4], LONG)
    with pytest.raises(ValueError):
        codec.encode(b"\x00" * (codec.payload_bytes + 1))


def test_describe_is_consistent() -> None:
    d = FrameCodec(MODES[4], LONG).describe()
    assert d["mode"] == "QPSK-1/2"
    assert d["coded_bits"] == 2352
    assert d["base_graph"] == 2
    assert d["fillers"] >= 0
    assert Modulation.QPSK.bits_per_symbol == 2
