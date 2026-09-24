"""The tone floor (P9-8, ADR-0013): numerology, sync patterns, codec, modulator, detector.

The tone floor's value is its envelope — a constant one, sent at the OFDM frames' peak — and
a detector that needs no channel estimate. These tests pin the frame kinds and patterns that
are part of the air interface, prove the waveform is what the design says (steady, narrow,
phase-continuous), and that the detector finds frames by kind, redundancy version, start and
carrier offset while ignoring noise, OFDM traffic and a strong carrier.
"""

from __future__ import annotations

import math

import numpy as np
import pytest

from aether_model.channel import make_channel
from aether_model.frame import modes as M
from aether_model.frame.codec import tone_codec
from aether_model.frame.modes import air_interface
from aether_model.link.frames import CONNECT_BODY_BYTES, CONTROL_BYTES, DATA_HEADER
from aether_model.phy import tone as T
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.waveform import NARROW_500, WIDE_2300

KINDS = (M.TONE_CONTROL, *M.TONE_DATA)


def _payload(rng: np.random.Generator, kind: M.ToneKind) -> bytes:
    return bytes(rng.integers(0, 256, kind.payload_bytes, dtype=np.uint8))


def _received(
    kind: M.ToneKind,
    payload: bytes,
    *,
    snr_db: float,
    lead: int,
    cfo_hz: float,
    seed: int,
    rv: int = 0,
    channel: str = "awgn",
) -> np.ndarray:
    x = np.concatenate(
        (np.zeros(lead, complex), T.burst(kind, payload, rv), np.zeros(2000, complex))
    )
    ch = make_channel(
        channel, snr_db=snr_db, fs=kind.num.fs, seed=seed, signal_power=1.0, cfo_hz=cfo_hz
    )
    return ch.process(x)


# ── the air interface's numbers ────────────────────────────────────────


def test_the_numerology_is_sixteen_tones_at_twenty_five_baud_inside_500_hz() -> None:
    num = M.TONE_NUMEROLOGY
    assert (num.tones, num.symbol_samples, num.fs) == (16, 320, 8000.0)
    assert num.spacing_hz == 25.0
    assert num.span_hz == 400.0
    assert num.tone_hz(np.arange(16))[[0, -1]].tolist() == [-187.5, 187.5]


def test_the_sync_patterns_are_the_pinned_search_and_costas_and_mutually_distinct() -> None:
    assert M.search_sync_patterns(len(M.SYNC_PATTERNS)) == M.SYNC_PATTERNS
    for i, a in enumerate(M.SYNC_PATTERNS):
        assert len(a) == M.SYNC_SYMBOLS
        assert M.is_costas(a)
        # a pattern against itself: one symbol at most under any nonzero offset or shift
        assert (
            max(
                sum(1 for j in range(8) if 0 <= j - d < 8 and a[j] == a[j - d] + s)
                for d in range(-7, 8)
                for s in range(-8, 9)
                if (d, s) != (0, 0)
            )
            <= 1
        )
        for b in M.SYNC_PATTERNS[i + 1 :]:
            assert M.cross_hits(a, b, 7) <= M.PATTERN_CROSS
    used = [p for k in KINDS for p in k.patterns]
    assert sorted(used) == list(range(len(M.SYNC_PATTERNS)))


def test_the_kinds_carry_what_the_link_puts_on_the_floor() -> None:
    assert M.TONE_CONTROL.payload_bytes == CONTROL_BYTES
    assert M.TONE_CONTROL.control and len(M.TONE_CONTROL.patterns) == 1
    slowest = M.TONE_DATA[0]
    # a connect request (and its acceptance) fits the slowest data kind: body, DATA header
    # and the partial frame's two length bytes
    assert slowest.payload_bytes >= CONNECT_BODY_BYTES + DATA_HEADER + 2
    assert [k.payload_bytes for k in M.TONE_DATA] == [24, 36]
    assert {k.symbols for k in M.TONE_DATA} == {134}
    assert M.TONE_CONTROL.symbols == 80
    assert M.TONE_CONTROL.duration_s == pytest.approx(3.2)
    assert M.TONE_DATA[0].duration_s == pytest.approx(5.36)
    for k in M.TONE_DATA:
        assert len(k.patterns) == 4
    # slowest first, and the control frame more robust (a lower rate) than any data kind
    assert M.TONE_DATA[0].net_bps < M.TONE_DATA[1].net_bps
    assert M.TONE_CONTROL.rate < min(k.rate for k in M.TONE_DATA)


def test_each_sync_block_starts_where_the_layout_says() -> None:
    for kind in KINDS:
        layout = kind.layout()
        assert (layout >= 0).sum() == 3 * M.SYNC_SYMBOLS
        for o in kind.block_offsets:
            assert tuple(layout[o : o + M.SYNC_SYMBOLS]) == kind.sync()
        assert kind.block_offsets[-1] + M.SYNC_SYMBOLS == kind.symbols


# ── the waveform ───────────────────────────────────────────────────────


def test_the_envelope_is_constant_at_the_ofdm_peak() -> None:
    rng = np.random.default_rng(1)
    kind = M.TONE_DATA[0]
    x = T.burst(kind, _payload(rng, kind))
    edge = kind.num.edge_samples
    mag = np.abs(x[edge:-edge])
    gain = 10 ** (T.TONE_GAIN_DB / 20)
    assert np.allclose(mag, gain, rtol=1e-12)
    assert len(x) == kind.samples
    # the OFDM frames of both airs peak above the tone: equal peak power at most
    for params in (WIDE_2300, NARROW_500):
        modem = Modem(params)
        air = air_interface(params)
        codec = modem.codec(air.control_mode, air.short)
        burst = modem.tx.baseband(
            FrameHeader(FrameType.CONTROL), air.short, codec.encode(bytes(codec.payload_bytes))
        )
        power = np.abs(burst) ** 2
        papr_db = 10 * math.log10(power.max() / power.mean())
        assert papr_db >= T.TONE_GAIN_DB


def test_the_phase_is_continuous_and_the_spectrum_stays_inside_500_hz() -> None:
    rng = np.random.default_rng(2)
    kind = M.TONE_DATA[1]
    x = T.burst(kind, _payload(rng, kind))
    step = np.angle(x[1:] / x[:-1])
    # no sample-to-sample phase step larger than the highest tone's advance
    assert np.abs(step).max() <= 2 * np.pi * 190.0 / kind.num.fs
    spec = np.abs(np.fft.fftshift(np.fft.fft(x, 1 << 18))) ** 2
    f = np.fft.fftshift(np.fft.fftfreq(1 << 18, 1 / kind.num.fs))
    # 99.9 % inside the 500 Hz channel; −35 dB past ±300 Hz, −50 dB past ±500 Hz (measured
    # −32, −39 and −56 over the three kinds)
    inside = spec[np.abs(f) <= 250.0].sum() / spec.sum()
    assert inside > 0.999
    assert spec[np.abs(f) > 300.0].sum() / spec.sum() < 10 ** (-35 / 10)
    assert spec[np.abs(f) > 500.0].sum() / spec.sum() < 10 ** (-50 / 10)


# ── the codec ──────────────────────────────────────────────────────────


@pytest.mark.parametrize("kind", KINDS, ids=lambda k: k.name)
def test_every_kind_round_trips_clean(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(3)
    payload = _payload(rng, kind)
    y = _received(kind, payload, snr_db=10.0, lead=500, cfo_hz=0.0, seed=4)
    frame = T.demodulate(y, kind, 0, 500, 0.0)
    out, _ = frame.decode()
    assert out == payload


def test_a_retransmission_at_another_redundancy_version_combines() -> None:
    rng = np.random.default_rng(5)
    kind = M.TONE_DATA[1]
    rescued = 0
    for trial in range(12):
        payload = _payload(rng, kind)
        buffer = None
        for i, rv in enumerate((0, 2)):
            y = _received(
                kind, payload, snr_db=-18.5, lead=400, cfo_hz=0.0, seed=100 + 2 * trial + i, rv=rv
            )
            out, buffer = T.demodulate(y, kind, rv, 400, 0.0).decode(buffer)
            if out is not None:
                assert out == payload
                rescued += i == 1
                break
        else:
            pytest.fail("neither transmission nor both together decoded")
    assert rescued > 0


def test_the_all_zero_block_is_refused() -> None:
    kind = M.TONE_CONTROL
    codec = tone_codec(kind)
    llr = np.full(kind.coded_bits, 20.0)  # every bit a confident zero
    out, _ = codec.decode(llr)
    assert out is None


# ── the detector ───────────────────────────────────────────────────────


@pytest.mark.parametrize("kind", KINDS, ids=lambda k: k.name)
def test_the_detector_names_kind_rv_start_and_offset(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(6)
    det = T.ToneDetector()
    for rv in range(len(kind.patterns)):
        lead = int(rng.integers(0, 4000))
        cfo = float(rng.uniform(-100, 100))
        y = _received(
            kind, _payload(rng, kind), snr_db=-14.0, lead=lead, cfo_hz=cfo, seed=rv, rv=rv
        )
        syncs = det.detect(y)
        assert len(syncs) == 1
        s = syncs[0]
        assert (s.kind, s.rv) == (kind, rv)
        assert abs(s.start - lead) <= kind.num.symbol_samples // 16
        assert abs(s.cfo_hz - cfo) < 1.0


def test_the_snr_is_reported_in_the_ofdm_reference() -> None:
    rng = np.random.default_rng(7)
    kind = M.TONE_DATA[0]
    for snr in (-18.0, -12.0, -6.0):
        y = _received(kind, _payload(rng, kind), snr_db=snr, lead=800, cfo_hz=20.0, seed=int(-snr))
        s = T.ToneDetector().detect(y)[0]
        frame = T.demodulate(y, kind, s.rv, s.start, s.cfo_hz)
        assert frame.snr_db == pytest.approx(snr, abs=1.0)


def test_frames_decode_through_the_detector_near_the_floor() -> None:
    rng = np.random.default_rng(8)
    kind = M.TONE_DATA[0]
    det = T.ToneDetector()
    ok = 0
    for i in range(10):
        payload = _payload(rng, kind)
        lead = int(rng.integers(0, 3000))
        y = _received(
            kind,
            payload,
            snr_db=-17.0,
            lead=lead,
            cfo_hz=float(rng.uniform(-100, 100)),
            seed=40 + i,
        )
        syncs = det.detect(y)
        if syncs:
            out, _ = T.demodulate(y, kind, syncs[0].rv, syncs[0].start, syncs[0].cfo_hz).decode()
            ok += out == payload
    assert ok >= 9


@pytest.mark.slow
def test_a_minute_of_noise_is_never_a_frame() -> None:
    det = T.ToneDetector()
    for seed in range(3):
        y = make_channel("awgn", snr_db=0.0, fs=8000.0, seed=300 + seed, signal_power=1.0).process(
            np.zeros(8000 * 60, complex)
        )
        stat, _, _ = det.statistics(det.spectrogram(y))
        assert stat.max() < det.threshold


@pytest.mark.parametrize("params", [WIDE_2300, NARROW_500], ids=["2300", "500"])
def test_ofdm_traffic_is_never_a_tone_frame(params: object) -> None:
    rng = np.random.default_rng(9)
    modem = Modem(params)  # type: ignore[arg-type]
    air = air_interface(params)  # type: ignore[arg-type]
    frames = []
    for _ in range(8):
        m = int(rng.integers(0, len(air.modes)))
        layout = air.long
        codec = modem.codec(air.modes[m], layout)
        payload = bytes(rng.integers(0, 256, codec.payload_bytes, dtype=np.uint8))
        frames.append(
            modem.tx.baseband(FrameHeader(FrameType.DATA, m), layout, codec.encode(payload, 0))
        )
    x = np.concatenate([np.zeros(4000, complex), *frames, np.zeros(4000, complex)])
    y = make_channel("awgn", snr_db=30.0, fs=8000.0, seed=10, signal_power=1.0).process(x)
    assert T.ToneDetector().detect(y) == []


def test_a_strong_carrier_on_a_tone_is_not_a_frame() -> None:
    t = np.arange(8000 * 20) / 8000
    carrier = 30.0 * np.exp(2j * np.pi * 62.5 * t)
    y = make_channel("awgn", snr_db=0.0, fs=8000.0, seed=11, signal_power=1.0).process(carrier)
    assert T.ToneDetector().detect(y) == []
