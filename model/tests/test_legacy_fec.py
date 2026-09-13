"""Legacy "5G NR-inspired" LDPC and interleaver — roadmap P1-1/P1-3 replace both.

``fec/ldpc.py`` (random-H, least-squares parity) was deleted in Phase 0; only the QC
variant remains until the real TS 38.212 BG2 code lands.
"""

from __future__ import annotations

import numpy as np
import pytest
from conftest import audit_xfail

from aether_model.fec.interleaver import FrequencyInterleaver, TimeInterleaver
from aether_model.fec.ldpc_5gnr import LDPC5GNR

# ── LDPC ──────────────────────────────────────────────────────────────


def test_decoder_accepts_a_valid_all_zero_codeword() -> None:
    code = LDPC5GNR(288, 0.5)
    info, converged, iters = code.decode(np.full(code.n, 8.0))
    assert converged
    assert iters == 1
    assert not info.any()


def test_code_dimensions_are_consistent() -> None:
    code = LDPC5GNR(288, 0.5)
    assert code.H.shape == (code.m, code.n)
    assert code.k + code.m == code.n


@pytest.mark.audit
@audit_xfail(
    "§2 fec/ldpc_5gnr.py",
    "encoder ignores the super-diagonal: every codeword violates ~50 % of checks",
)
@pytest.mark.parametrize(("n", "rate"), [(288, 0.5), (1152, 0.25), (1152, 0.75)])
def test_encoder_produces_valid_codewords(n: int, rate: float, rng: np.random.Generator) -> None:
    code = LDPC5GNR(n, rate)
    for _ in range(10):
        cw = code.encode(rng.integers(0, 2, code.k).astype(np.int8))
        assert not ((code.H @ cw) % 2).any()


@pytest.mark.audit
@audit_xfail("§2 fec/ldpc_5gnr.py", "decoder fails on noiseless LLRs of the encoder's own output")
def test_noiseless_decode_recovers_information_bits(rng: np.random.Generator) -> None:
    code = LDPC5GNR(288, 0.5)
    info = rng.integers(0, 2, code.k).astype(np.int8)
    llr = 8.0 * (1 - 2 * code.encode(info).astype(float))
    decoded, converged, _ = code.decode(llr)
    assert converged
    assert np.array_equal(decoded, info)


@pytest.mark.audit
@audit_xfail(
    "§2 fec/ldpc_5gnr.py", "40 % of check nodes have degree ≤ 2 at R = 1/3 (parity chains)"
)
def test_low_rate_code_has_no_degenerate_check_nodes() -> None:
    code = LDPC5GNR(1152, 1 / 3)
    assert int(code.H.sum(axis=1).min()) >= 3


@pytest.mark.audit
@audit_xfail("§2 fec/ldpc_5gnr.py", "dict-keyed pure-Python BP: ~0.8 s per 288-bit block")
def test_decoder_throughput_is_usable() -> None:
    """Ten BP iterations on a 288-bit block must take < 20 ms (≈ 2 ms/iteration) to keep up
    with the lowest v1 modes. Random-sign LLRs never converge, so exactly 10 iterations run."""
    import time

    code = LDPC5GNR(288, 0.5, max_iter=10)
    llr = np.random.default_rng(0).choice([-3.0, 3.0], size=code.n)
    t0 = time.perf_counter()
    _, converged, iters = code.decode(llr)
    elapsed = time.perf_counter() - t0
    assert not converged and iters == 10
    assert elapsed < 0.02


# ── Interleaver ───────────────────────────────────────────────────────


def test_interleavers_are_bijections(rng: np.random.Generator) -> None:
    bits = rng.integers(0, 2, 4096).astype(np.int8)
    ti = TimeInterleaver(4096, depth=8)
    assert np.array_equal(ti.deinterleave(ti.interleave(bits)), bits)
    fi = FrequencyInterleaver(52)
    b52 = bits[:52]
    assert np.array_equal(fi.deinterleave(fi.interleave(b52)), b52)


@pytest.mark.audit
@audit_xfail(
    "§2 fec/interleaver.py",
    "time 'interleaver' permutes contiguous 512-bit blocks: zero burst spreading",
)
def test_time_interleaver_spreads_a_burst() -> None:
    """A burst of 64 consecutive channel errors must not land on more than 8 adjacent
    coded bits anywhere in the codeword."""
    n = 4096
    ti = TimeInterleaver(n, depth=8)
    positions = ti.deinterleave(np.arange(n))  # channel index → codeword index
    burst = np.sort(positions[1000:1064])
    longest_run = int(
        np.max(np.diff(np.flatnonzero(np.diff(burst) != 1), prepend=-1, append=len(burst) - 1))
    )
    assert longest_run <= 8
