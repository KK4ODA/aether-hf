"""P1-1: 3GPP TS 38.212 LDPC (BG1/BG2), rate matching, CRC."""

from __future__ import annotations

import time

import numpy as np
import pytest

from aether_model.channel import complex_normal
from aether_model.fec.crc import CRC6, CRC11, CRC16, CRC24A, CRC24B, CRC24C
from aether_model.fec.nr_ldpc import (
    ALL_LIFTING_SIZES,
    FILLER_LLR,
    LIFTING_SETS,
    NrLdpcCode,
    RateMatcher,
    base_graph,
    kb_for,
    lifting_set_index,
    nr_bit_deinterleave,
    nr_bit_interleave,
    nr_ldpc_code,
    select_lifting_size,
)
from aether_model.phy.constellation import constellation
from aether_model.waveform import Modulation

# ── tables ────────────────────────────────────────────────────────────


def test_base_graph_tables_have_the_standard_dimensions() -> None:
    bg1, bg2 = base_graph(1), base_graph(2)
    assert (bg1.rows, bg1.cols, len(bg1.entries)) == (46, 68, 316)
    assert (bg2.rows, bg2.cols, len(bg2.entries)) == (42, 52, 197)
    for bg in (bg1, bg2):
        assert all(len(v) == 8 for _, _, v in bg.entries)
        # extension part is an identity: block row i ≥ 4 has one entry at column kb+i, shift 0
        ext = [(i, j, v) for i, j, v in bg.entries if i >= 4 and j >= bg.kb_max + 4]
        assert all(j == bg.kb_max + i for i, j, _ in ext)
        assert all(all(x == 0 for x in v) for _, _, v in ext)
        assert len(ext) == bg.rows - 4


def test_lifting_sets() -> None:
    assert len(ALL_LIFTING_SIZES) == 51
    assert ALL_LIFTING_SIZES[0] == 2 and ALL_LIFTING_SIZES[-1] == 384
    assert lifting_set_index(384) == 1 and lifting_set_index(240) == 7
    with pytest.raises(ValueError):
        lifting_set_index(100)
    assert sum(len(s) for s in LIFTING_SETS) == 51


@pytest.mark.parametrize(
    ("bg", "info_len", "kb", "z"),
    [
        (2, 100, 6, 18),
        (2, 200, 8, 26),
        (2, 600, 9, 72),
        (2, 1000, 10, 104),
        (2, 3840, 10, 384),
        (1, 8448, 22, 384),
        (1, 1000, 22, 48),
    ],
)
def test_kb_and_lifting_selection(bg: int, info_len: int, kb: int, z: int) -> None:
    assert kb_for(bg, info_len) == kb
    assert select_lifting_size(bg, info_len) == z


# ── encoder ───────────────────────────────────────────────────────────


@pytest.mark.parametrize("z", ALL_LIFTING_SIZES, ids=lambda z: f"Z{z}")
def test_bg2_encoder_is_systematic_and_valid_for_every_lifting_size(
    z: int, rng: np.random.Generator
) -> None:
    code = NrLdpcCode(2, z)
    for _ in range(3):
        info = rng.integers(0, 2, code.k).astype(np.uint8)
        cw = code.encode(info)
        assert cw.shape == (52 * z,)
        assert np.array_equal(cw[: code.k], info)
        assert code.syndrome_ok(cw)


@pytest.mark.parametrize("z", [2, 3, 5, 7, 9, 11, 13, 15, 48, 208, 384], ids=lambda z: f"Z{z}")
def test_bg1_encoder_is_systematic_and_valid(z: int, rng: np.random.Generator) -> None:
    code = NrLdpcCode(1, z)
    info = rng.integers(0, 2, code.k).astype(np.uint8)
    cw = code.encode(info)
    assert cw.shape == (68 * z,)
    assert np.array_equal(cw[: code.k], info)
    assert code.syndrome_ok(cw)


def test_syndrome_detects_a_single_flipped_bit(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 24)
    cw = code.encode(rng.integers(0, 2, code.k).astype(np.uint8))
    cw[rng.integers(0, code.n_full)] ^= 1
    assert not code.syndrome_ok(cw)


def test_encoder_rejects_wrong_length() -> None:
    with pytest.raises(ValueError):
        nr_ldpc_code(2, 8).encode(np.zeros(10, dtype=np.uint8))


# ── decoder ───────────────────────────────────────────────────────────


def test_decoder_converges_immediately_on_a_clean_codeword(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 48)
    info = rng.integers(0, 2, code.k).astype(np.uint8)
    cw = code.encode(info)
    llr = 6.0 * (1.0 - 2.0 * cw)
    llr[: 2 * code.z] = 0.0  # punctured systematic bits are never received
    hard, converged, iters = code.decode(llr)
    assert converged.all() and iters[0] == 1
    assert np.array_equal(hard[: code.k], info)


def test_decoder_corrects_awgn_errors_at_the_mother_rate(rng: np.random.Generator) -> None:
    """BG2 mother code is R = 10/50 = 1/5. BPSK at E_b/N_0 = 2.5 dB (E_s/N_0 = −4.5 dB) must
    decode 20/20 blocks — a working LDPC has ≈1 dB of margin here; the legacy code had none."""
    code = nr_ldpc_code(2, 48)
    bpsk = constellation(Modulation.BPSK)
    es_n0 = 10 ** ((2.5 + 10 * np.log10(0.2)) / 10)
    noise_var = 1.0 / es_n0
    ok = 0
    for _ in range(20):
        info = rng.integers(0, 2, code.k).astype(np.uint8)
        cw = code.encode(info)
        tx = cw[2 * code.z :]  # transmit everything but the punctured 2Z bits
        y = bpsk.map(tx) + np.sqrt(noise_var) * complex_normal(rng, len(tx))
        llr = np.concatenate((np.zeros(2 * code.z), bpsk.llr(y, noise_var)))
        hard, converged, _ = code.decode(llr, max_iter=30)
        ok += int(converged[0] and np.array_equal(hard[: code.k], info))
    assert ok == 20


def test_batched_decoding_matches_single(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 16)
    llrs = []
    for _ in range(4):
        cw = code.encode(rng.integers(0, 2, code.k).astype(np.uint8))
        llr = 2.0 * (1.0 - 2.0 * cw) + 1.5 * rng.standard_normal(code.n_full)
        llr[: 2 * code.z] = 0.0
        llrs.append(llr)
    batch = np.stack(llrs)
    hb, cb, ib = code.decode(batch, max_iter=10)
    for k in range(4):
        h, c, i = code.decode(llrs[k], max_iter=10)
        assert np.array_equal(h, hb[k]) and c[0] == cb[k] and i[0] == ib[k]


def test_decoder_speed() -> None:
    """A BG2 Z=48 codeword (2 400 bits) must run 25 iterations in well under a second in
    the model; the Rust core will be ~100× faster."""
    code = nr_ldpc_code(2, 48)
    llr = np.random.default_rng(1).choice([-2.0, 2.0], size=code.n_full)
    t0 = time.perf_counter()
    code.decode(llr, max_iter=25, early_stop=False)
    assert time.perf_counter() - t0 < 1.0


# ── rate matching ─────────────────────────────────────────────────────


@pytest.mark.parametrize(("bg", "expected"), [(1, (0, 17, 33, 56)), (2, (0, 13, 25, 43))])
def test_k0_follows_table_5_4_2_1_2(bg: int, expected: tuple[int, ...]) -> None:
    code = nr_ldpc_code(bg, 32)
    for rv, num in enumerate(expected):
        assert RateMatcher(code, code.k, e=100, rv=rv).k0 == num * 32


def test_rv0_full_buffer_is_the_identity(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 24)
    cw = code.encode(rng.integers(0, 2, code.k).astype(np.uint8))
    rm = RateMatcher(code, code.k, e=code.n_cb, rv=0)
    assert np.array_equal(rm.match(cw), cw[2 * code.z :])


def test_bit_selection_wraps_and_skips_fillers(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 8)  # K = 80, fillers from 60
    info = np.concatenate((rng.integers(0, 2, 60), np.zeros(20))).astype(np.uint8)
    cw = code.encode(info)
    rm = RateMatcher(code, info_len=60, e=code.n_cb + 50, rv=0)
    e = rm.match(cw)
    d = cw[2 * code.z :]
    fillers = np.arange(60 - 16, 80 - 16)
    assert not np.isin(rm.positions, fillers).any()
    assert len(e) == code.n_cb + 50
    # first n_cb − 20 outputs are the buffer without fillers, in order; then it wraps
    usable = np.setdiff1d(np.arange(code.n_cb), fillers)
    np.testing.assert_array_equal(rm.positions[: len(usable)], usable)
    np.testing.assert_array_equal(rm.positions[len(usable) : len(usable) + 50], usable[:50])
    assert np.array_equal(e, d[rm.positions])


def test_soft_recovery_inverts_selection_and_combines_rvs(rng: np.random.Generator) -> None:
    code = nr_ldpc_code(2, 8)
    info = np.concatenate((rng.integers(0, 2, 60), np.zeros(20))).astype(np.uint8)
    cw = code.encode(info)
    llr_full = np.zeros(code.n_full)
    for rv in (0, 1):
        rm = RateMatcher(code, info_len=60, e=200, rv=rv)
        soft = 3.0 * (1.0 - 2.0 * rm.match(cw).astype(float))
        llr_full = rm.recover(soft, buffer=llr_full if rv else None)
    assert np.all(llr_full[: 2 * code.z] == 0.0)  # punctured
    assert np.all(llr_full[60:80] == FILLER_LLR)  # fillers
    transmitted = llr_full[2 * code.z :] != 0
    assert np.array_equal(
        (llr_full[2 * code.z :][transmitted] < 0), cw[2 * code.z :][transmitted] == 1
    )
    hard, converged, _ = code.decode(llr_full)
    assert converged.all() and np.array_equal(hard[:60], info[:60])


def test_harq_ir_second_transmission_rescues_a_failed_first(rng: np.random.Generator) -> None:
    """RV0 at a punishing rate fails; adding RV1 (new parity bits) decodes. This is the
    mechanism the ARQ layer will use for NAK-near-miss retransmissions."""
    code = nr_ldpc_code(2, 48)  # K = 480
    bpsk = constellation(Modulation.BPSK)
    e = 560  # rate ≈ 0.86 on the first shot
    noise_var = 1.0 / 10 ** (1.5 / 10)  # E_s/N_0 = 1.5 dB: RV0 alone fails ~always
    rescued = 0
    for _ in range(6):
        info = rng.integers(0, 2, code.k).astype(np.uint8)
        cw = code.encode(info)
        rm0 = RateMatcher(code, code.k, e, rv=0)
        y0 = bpsk.map(rm0.match(cw)) + np.sqrt(noise_var) * complex_normal(rng, e)
        buf = rm0.recover(bpsk.llr(y0, noise_var))
        h0, c0, _ = code.decode(buf, max_iter=30)
        first_ok = bool(c0[0]) and np.array_equal(h0[: code.k], info)
        rm1 = RateMatcher(code, code.k, e, rv=1)
        y1 = bpsk.map(rm1.match(cw)) + np.sqrt(noise_var) * complex_normal(rng, e)
        buf = rm1.recover(bpsk.llr(y1, noise_var), buffer=buf)
        h1, c1, _ = code.decode(buf, max_iter=30)
        second_ok = bool(c1[0]) and np.array_equal(h1[: code.k], info)
        assert second_ok
        rescued += int(second_ok and not first_ok)
    assert rescued >= 4


def test_nr_bit_interleaver_round_trip() -> None:
    e = np.arange(24)
    f = nr_bit_interleave(e, 4)
    assert f[1] == 6 and f[4] == 1  # f[i + j·Q_m] = e[i·E/Q_m + j]
    assert np.array_equal(nr_bit_deinterleave(f, 4), e)


# ── CRC ───────────────────────────────────────────────────────────────


@pytest.mark.parametrize("crc", [CRC24A, CRC24B, CRC24C, CRC16, CRC11, CRC6], ids=lambda c: c.name)
def test_crc_attach_check_and_detect(crc, rng: np.random.Generator) -> None:  # type: ignore[no-untyped-def]
    data = rng.integers(0, 2, 200).astype(np.uint8)
    coded = crc.attach(data)
    assert len(coded) == 200 + crc.width
    assert crc.check(coded)
    bad = coded.copy()
    bad[rng.integers(0, len(bad))] ^= 1
    assert not crc.check(bad)


def test_crc16_ccitt_known_answer() -> None:
    """CRC-16/XMODEM (poly 0x1021, init 0, no reflection) of ASCII '123456789' is 0x31C3."""
    msg = np.unpackbits(np.frombuffer(b"123456789", dtype=np.uint8))
    rem = CRC16.remainder(msg)
    assert int("".join(map(str, rem)), 2) == 0x31C3


# ── independent oracle (optional) ─────────────────────────────────────


@pytest.mark.oracle
@pytest.mark.parametrize(
    ("bg", "z"),
    [(1, 2), (1, 3), (1, 24), (1, 44), (1, 208), (1, 384), (2, 72), (2, 104), (2, 240), (2, 384)],
    ids=lambda v: str(v),
)
def test_encoder_matches_py3gpp_oracle(bg: int, z: int, rng: np.random.Generator) -> None:
    """Bit-exact agreement with an independent implementation and table transcription.

    Run with ``uv run --with py3gpp pytest -m oracle``. Only configurations where py3gpp is
    itself standard-conformant are compared: every BG1 size, and BG2 with K_b = 10
    (Z ≥ 72). For BG2 with K_b < 10, py3gpp's output fails the standard's own parity check
    (verified 2026-09-13), so it cannot serve as an oracle there; our codewords satisfy
    H·c = 0 for every lifting size (see the per-Z encoder test above).
    """
    py3gpp = pytest.importorskip("py3gpp")
    code = NrLdpcCode(bg, z)
    info = rng.integers(0, 2, code.k).astype(np.uint8)
    ours = code.encode(info)[2 * z :]
    theirs = py3gpp.nrLDPCEncode(info.reshape(-1, 1).astype(int).copy(), bg, algo="thangaraj")[:, 0]
    assert np.array_equal(ours, theirs.astype(np.uint8))
