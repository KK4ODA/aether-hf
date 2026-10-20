"""The tone floor (P9-8, ADR-0013) and its fast kinds (P9-9, ADR-0014): numerology, sync
patterns, codec, modulator, detector.

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

FLOOR_KINDS = (M.TONE_CONTROL, *M.TONE_DATA)
KINDS = (*FLOOR_KINDS, *M.TONE_FAST, *M.TONE_NARROW)
NARROW_DETECTOR_KINDS = (M.NARROW.tone_control, *M.NARROW.tone_data)


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
    follows: int = 0,
) -> np.ndarray:
    x = np.concatenate(
        (np.zeros(lead, complex), T.burst(kind, payload, rv, follows), np.zeros(2000, complex))
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


def test_the_first_sync_patterns_are_the_pinned_search() -> None:
    # the floor's and the fast kinds' 25 are 77 thousand draws in; the rest, millions
    assert M.search_sync_patterns(25) == M.SYNC_PATTERNS[:25]


@pytest.mark.slow
def test_every_sync_pattern_is_the_pinned_search() -> None:
    assert M.search_sync_patterns(len(M.SYNC_PATTERNS)) == M.SYNC_PATTERNS


def test_the_sync_patterns_are_costas_and_mutually_distinct() -> None:
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


def test_the_fast_kinds_are_the_floors_frame_with_faster_data() -> None:
    """ADR-0014: the same sync blocks at the same places, the same 5.36 s, the floor's two
    code rates — and two or four data symbols a slot, on tones 50 or 100 Hz apart. The 2 300
    Hz air's ladder runs through them in order of rate; the 500 Hz air has no room for them."""
    assert [k.payload_bytes for k in M.TONE_FAST] == [51, 75, 105, 153]
    assert [k.speed for k in M.TONE_FAST] == [2, 2, 4, 4]
    assert [k.data.spacing_hz for k in M.TONE_FAST] == [50.0, 50.0, 100.0, 100.0]
    floor = M.TONE_DATA[0]
    for k in M.TONE_FAST:
        assert k.num == M.TONE_NUMEROLOGY and not k.control and len(k.patterns) == 4
        assert k.data_slots == floor.data_slots and k.symbols == floor.symbols
        assert k.block_offsets == floor.block_offsets and k.samples == floor.samples
        assert k.data_symbols == floor.data_symbols * k.speed
        # the floor's rates: 24 and 36 bytes' information per 440 coded bits, scaled
        slow = M.TONE_DATA[M.TONE_FAST.index(k) % 2]
        assert k.info_bits == slow.info_bits * k.speed
        assert k.rate == pytest.approx(slow.rate)
    ladder = (*M.TONE_DATA, *M.TONE_FAST)
    assert [round(k.net_bps) for k in ladder] == [36, 54, 76, 112, 157, 228]
    assert M.WIDE.tone_data == ladder


def test_the_narrow_middle_kinds_are_the_floors_frame_on_four_tones() -> None:
    """ADR-0015: the floor's frame again — its sync blocks, its 5.36 s, its two code rates —
    with four data symbols a slot on four tones 100 Hz apart, the floor's own 400 Hz: 880
    coded bits a frame, twice the floor's. The 500 Hz air's ladder runs through them between
    the floor's kinds and the OFDM modes; the 2 300 Hz air never looks for them. The third,
    tone4x100-93 (ADR-0081), is the same frame at rate 0.87."""
    assert [k.payload_bytes for k in M.TONE_NARROW] == [51, 75, 93]
    floor = M.TONE_DATA[0]
    for k in M.TONE_NARROW:
        assert k.num == M.TONE_NUMEROLOGY and not k.control and len(k.patterns) == 4
        assert (k.data.tones, k.data.spacing_hz, k.data.span_hz, k.speed) == (4, 100.0, 400.0, 4)
        assert k.data.span_hz == k.num.span_hz
        assert k.data_slots == floor.data_slots and k.symbols == floor.symbols
        assert k.block_offsets == floor.block_offsets and k.samples == floor.samples
        assert k.coded_bits == 2 * floor.coded_bits == 880
    for k, slow in zip(M.TONE_NARROW[:2], M.TONE_DATA, strict=True):
        assert k.info_bits == 2 * slow.info_bits and k.rate == pytest.approx(slow.rate)
    assert M.TONE_NARROW[2].rate == pytest.approx(0.87, abs=0.005)
    ladder = (*M.TONE_DATA, *M.TONE_NARROW)
    assert [round(k.net_bps) for k in ladder] == [36, 54, 76, 112, 139]
    assert M.NARROW.tone_data == ladder
    assert not set(M.TONE_NARROW) & set(M.WIDE.tone_data)


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
    for kind in (M.TONE_DATA[0], *M.TONE_FAST, *M.TONE_NARROW):
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


@pytest.mark.parametrize("kind", [M.TONE_DATA[1], *M.TONE_NARROW], ids=lambda k: k.name)
def test_the_phase_is_continuous_and_the_spectrum_stays_inside_500_hz(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(2)
    x = T.burst(kind, _payload(rng, kind))
    step = np.angle(x[1:] / x[:-1])
    # no sample-to-sample phase step larger than the highest tone's advance
    assert np.abs(step).max() <= 2 * np.pi * 190.0 / kind.num.fs
    spec = np.abs(np.fft.fftshift(np.fft.fft(x, 1 << 18))) ** 2
    f = np.fft.fftshift(np.fft.fftfreq(1 << 18, 1 / kind.num.fs))
    # 99.9 % inside the 500 Hz channel; −35 dB past ±300 Hz, −50 dB past ±500 Hz (measured
    # −32, −39 and −56 over the floor's three kinds; −30.5, −35.1 and −54 over the narrow
    # middle kinds, whose 100 Bd data glides over the floor's 32 samples to stay inside)
    inside = spec[np.abs(f) <= 250.0].sum() / spec.sum()
    assert inside > 0.999
    assert spec[np.abs(f) > 300.0].sum() / spec.sum() < 10 ** (-35 / 10)
    assert spec[np.abs(f) > 500.0].sum() / spec.sum() < 10 ** (-50 / 10)


@pytest.mark.parametrize(
    ("kind", "near_db", "edge_db"),
    [(M.TONE_FAST[1], -40.0, -65.0), (M.TONE_FAST[3], -34.0, -37.0)],
    ids=["50Bd", "100Bd"],
)
def test_a_fast_kind_is_phase_continuous_and_inside_the_wide_air(
    kind: M.ToneKind, near_db: float, edge_db: float
) -> None:
    """Its data glides between tones up to 1 500 Hz apart in a tenth of a symbol, as the
    floor's do: continuous phase, and what spills past the data's span a quarter-kilohertz
    out stays below ``near_db``, past the 2 300 Hz air's band edge (±1 150 Hz) below
    ``edge_db`` (measured −43.8 / −68 dB at 50 Bd, −35.7 / −39.2 dB at 100 Bd)."""
    rng = np.random.default_rng(12)
    x = T.burst(kind, _payload(rng, kind))
    step = np.angle(x[1:] / x[:-1])
    top = kind.data.tone_hz(kind.data.tones - 1)
    assert np.abs(step).max() <= 2 * np.pi * (float(top) + 1.0) / kind.num.fs
    spec = np.abs(np.fft.fftshift(np.fft.fft(x, 1 << 18))) ** 2
    f = np.fft.fftshift(np.fft.fftfreq(1 << 18, 1 / kind.num.fs))
    near = kind.data.span_hz / 2 + 250.0
    assert spec[np.abs(f) > near].sum() / spec.sum() < 10 ** (near_db / 10)
    assert spec[np.abs(f) > 1150.0].sum() / spec.sum() < 10 ** (edge_db / 10)


# ── the codec ──────────────────────────────────────────────────────────


@pytest.mark.parametrize("kind", KINDS, ids=lambda k: k.name)
def test_every_kind_round_trips_clean(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(3)
    payload = _payload(rng, kind)
    y = _received(kind, payload, snr_db=10.0, lead=500, cfo_hz=0.0, seed=4)
    frame = T.demodulate(y, kind, 0, 500, 0.0)
    out, _ = frame.decode()
    assert out == payload


@pytest.mark.parametrize(
    ("kind", "snr_db"), [(M.TONE_DATA[1], -18.5), (M.TONE_FAST[3], -12.5)], ids=["25Bd", "100Bd"]
)
def test_a_retransmission_at_another_redundancy_version_combines(
    kind: M.ToneKind, snr_db: float
) -> None:
    rng = np.random.default_rng(5)
    rescued = 0
    for trial in range(12):
        payload = _payload(rng, kind)
        buffer = None
        for i, rv in enumerate((0, 2)):
            y = _received(
                kind, payload, snr_db=snr_db, lead=400, cfo_hz=0.0, seed=100 + 2 * trial + i, rv=rv
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


@pytest.mark.parametrize("kind", FLOOR_KINDS, ids=lambda k: k.name)
def test_the_detector_finds_a_frame_up_to_150_hz_off(kind: M.ToneKind) -> None:
    """ADR-0064: a rig 70 Hz off and warming 0.3 Hz/s passed ±100 Hz three minutes into a
    500 Hz session, and the floor's DISC and its answer went unheard. The detector searches
    ±150 Hz: a frame between 100 and 150 Hz off either way is found, named and placed."""
    rng = np.random.default_rng(64)
    det = T.ToneDetector()
    for i, cfo in enumerate((-148.0, -121.0, 112.0, 139.0)):
        lead = int(rng.integers(0, 4000))
        y = _received(kind, _payload(rng, kind), snr_db=-12.0, lead=lead, cfo_hz=cfo, seed=640 + i)
        syncs = det.detect(y)
        assert len(syncs) == 1, cfo
        s = syncs[0]
        assert s.kind == kind
        assert abs(s.start - lead) <= kind.num.symbol_samples // 16
        assert abs(s.cfo_hz - cfo) < 1.0, (cfo, s.cfo_hz)


def test_a_receiver_centred_on_its_peer_hears_a_rig_past_the_search() -> None:
    """ADR-0079 (ADR-0064's open step): a peer 230 Hz off is past the floor's ±150 Hz search and
    half out of the 500 Hz band filter. With the receive mixer moved to the offset the session's
    frames measured, the frame sits near the centre again and is found, and its offset reads
    from the mixer: the true one is the mixer's plus the detector's."""
    from aether_model.phy.passband import AudioToBaseband, BasebandToAudio

    kind = M.NARROW.tone_control
    rng = np.random.default_rng(79)
    burst = T.burst(kind, _payload(rng, kind), 0, 0)
    lead = 3000
    x = np.concatenate((np.zeros(lead, complex), burst, np.zeros(4000, complex)))
    fs = NARROW_500.fs_baseband
    off = 230.0
    x = x * np.exp(2j * np.pi * off * np.arange(len(x)) / fs)
    x = x + 0.05 * (rng.standard_normal(len(x)) + 1j * rng.standard_normal(len(x)))
    audio = BasebandToAudio(NARROW_500).process(x)
    det = T.ToneDetector((kind,))

    plain = AudioToBaseband(NARROW_500)
    assert det.detect(plain.process(audio)) == []

    followed = AudioToBaseband(NARROW_500)
    followed.offset_hz = 200.0
    syncs = det.detect(followed.process(audio))
    assert len(syncs) == 1
    assert syncs[0].kind == kind
    assert abs(followed.offset_hz + syncs[0].cfo_hz - off) < 1.0, syncs[0].cfo_hz


def test_the_receive_mixer_at_no_offset_is_the_mixer_it_was() -> None:
    """The offset is added to the centre; at none, the output is the same to the bit, so the
    vectors and every recording replay as before."""
    from aether_model.phy.passband import AudioToBaseband, BasebandToAudio

    rng = np.random.default_rng(1)
    x = rng.standard_normal(4000) + 1j * rng.standard_normal(4000)
    audio = BasebandToAudio(WIDE_2300).process(x)
    a = AudioToBaseband(WIDE_2300)
    b = AudioToBaseband(WIDE_2300)
    b.offset_hz = 0.0
    assert np.array_equal(a.process(audio), b.process(audio))


@pytest.mark.parametrize("kind", FLOOR_KINDS, ids=lambda k: k.name)
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


DATA_KINDS = (*M.TONE_DATA, *M.TONE_FAST, *M.TONE_NARROW)


def _detector_for(kind: M.ToneKind) -> T.ToneDetector:
    air = M.NARROW if kind in M.TONE_NARROW else M.WIDE
    return T.ToneDetector((air.tone_control, *air.tone_data))


def test_a_frame_with_nothing_following_is_the_frame_before() -> None:
    """The count rides in the middle sync block (ADR-0068): a frame that says none follow is
    the frame sent before it, and a control frame — one pattern — has nowhere to carry one."""
    rng = np.random.default_rng(3)
    for kind in DATA_KINDS:
        for rv in range(len(kind.patterns)):
            assert np.array_equal(kind.layout(rv, 0), kind.layout(rv))
            for f in range(1, 4):
                layout = kind.layout(rv, f)
                start, mid, end = kind.block_offsets
                assert np.array_equal(layout[start : start + 8], kind.sync(rv))
                assert np.array_equal(layout[end : end + 8], kind.sync(rv))
                assert np.array_equal(layout[mid : mid + 8], kind.sync((rv + f) % 4))
    payload = _payload(rng, M.TONE_CONTROL)
    assert np.array_equal(
        T.burst(M.TONE_CONTROL, payload, 0, 3), T.burst(M.TONE_CONTROL, payload, 0)
    )


@pytest.mark.parametrize("kind", DATA_KINDS, ids=lambda k: k.name)
def test_a_frame_says_how_many_follow_it(kind: M.ToneKind) -> None:
    """Every data kind, every redundancy version, every count: the detector names the kind
    and the redundancy version from the start and end blocks as before, reads the count from
    the middle one, and a copy at RV 0 decodes."""
    rng = np.random.default_rng(11)
    det = _detector_for(kind)
    for rv in range(len(kind.patterns)):
        for f in range(4):
            payload = _payload(rng, kind)
            lead = int(rng.integers(0, 3000))
            y = _received(
                kind,
                payload,
                snr_db=-4.0,
                lead=lead,
                cfo_hz=float(rng.uniform(-80, 80)),
                seed=10 * rv + f,
                rv=rv,
                follows=f,
            )
            syncs = det.detect(y)
            assert len(syncs) == 1, (rv, f)
            s = syncs[0]
            assert (s.kind, s.rv, s.middle, s.follows) == (kind, rv, f, f)
            if rv == 0:
                out, _ = T.demodulate(y, kind, rv, s.start, s.cfo_hz, middle=s.middle).decode()
                assert out == payload


def test_a_middle_block_that_is_gone_states_nothing() -> None:
    """A fade over the middle block takes the count with it, not the frame: it is still found
    on its start and end blocks and still decodes, and says nothing of what follows — a guess
    would put the answer at the wrong time."""
    rng = np.random.default_rng(5)
    kind = M.TONE_DATA[1]
    det = _detector_for(kind)
    payload = _payload(rng, kind)
    lead = 1500
    y = _received(kind, payload, snr_db=-2.0, lead=lead, cfo_hz=20.0, seed=4, follows=2)
    n = kind.num.symbol_samples
    mid = kind.block_offsets[1]
    noise = make_channel("awgn", snr_db=-2.0, fs=kind.num.fs, seed=9, signal_power=1.0)
    span = slice(lead + mid * n, lead + (mid + 8) * n)
    y[span] = noise.process(np.zeros(span.stop - span.start, complex))
    syncs = det.detect(y)
    assert len(syncs) == 1
    s = syncs[0]
    assert (s.kind, s.rv, s.follows) == (kind, 0, None)
    out, _ = T.demodulate(y, kind, 0, s.start, s.cfo_hz, middle=s.middle).decode()
    assert out == payload


@pytest.mark.parametrize("kind", M.TONE_FAST, ids=lambda k: k.name)
def test_the_detector_finds_a_fast_kind_as_well_as_the_floors(kind: M.ToneKind) -> None:
    """A fast kind's sync blocks are the floor's, so the detector must name it as surely
    and place it as well: at −14 dB, measured over thirty frames a kind, the floor's kinds
    and the fast ones alike come within 0.46–0.72 Hz rms and 2.4–4.7 samples rms, with the
    worst of thirty 1.0–1.8 Hz — and another kind's hypothesis never wins."""
    rng = np.random.default_rng(13)
    det = T.ToneDetector()
    cfo_err, start_err = [], []
    for i in range(12):
        rv = i % len(kind.patterns)
        lead = int(rng.integers(0, 4000))
        cfo = float(rng.uniform(-100, 100))
        y = _received(
            kind, _payload(rng, kind), snr_db=-14.0, lead=lead, cfo_hz=cfo, seed=60 + i, rv=rv
        )
        syncs = det.detect(y)
        assert [(s.kind, s.rv) for s in syncs] == [(kind, rv)]
        cfo_err.append(syncs[0].cfo_hz - cfo)
        start_err.append(syncs[0].start - lead)
    assert max(abs(e) for e in start_err) <= kind.num.symbol_samples // 16
    assert max(abs(e) for e in cfo_err) < 2.0
    assert math.sqrt(np.mean(np.square(cfo_err))) < 0.9


@pytest.mark.parametrize("kind", M.TONE_NARROW, ids=lambda k: k.name)
def test_the_500_hz_detector_finds_a_middle_kind_and_no_other_air_does(kind: M.ToneKind) -> None:
    """A middle kind's sync blocks are the floor's, so the 500 Hz air's detector names and
    places it as surely as a floor kind; the 2 300 Hz air's does not look for it and finds
    nothing in a strong one, and the 500 Hz air's finds nothing in a strong fast kind — the
    patterns that name them are the search's last eight, mutually distinct from every other."""
    rng = np.random.default_rng(14)
    narrow = _detector(kind)
    cfo_err = []
    for i in range(8):
        rv = i % len(kind.patterns)
        lead = int(rng.integers(0, 4000))
        cfo = float(rng.uniform(-100, 100))
        y = _received(
            kind, _payload(rng, kind), snr_db=-14.0, lead=lead, cfo_hz=cfo, seed=80 + i, rv=rv
        )
        syncs = narrow.detect(y)
        assert [(s.kind, s.rv) for s in syncs] == [(kind, rv)]
        assert abs(syncs[0].start - lead) <= kind.num.symbol_samples // 16
        cfo_err.append(syncs[0].cfo_hz - cfo)
    assert max(abs(e) for e in cfo_err) < 2.0
    strong = _received(kind, _payload(rng, kind), snr_db=10.0, lead=1000, cfo_hz=0.0, seed=90)
    assert T.ToneDetector().detect(strong) == []
    fast = M.TONE_FAST[3]
    strong = _received(fast, _payload(rng, fast), snr_db=10.0, lead=1000, cfo_hz=0.0, seed=91)
    assert narrow.detect(strong) == []


def _detector(kind: M.ToneKind) -> T.ToneDetector:
    """The detector of the air ``kind`` is on: the 500 Hz air's for its middle kinds."""
    return T.ToneDetector(NARROW_DETECTOR_KINDS) if kind in M.TONE_NARROW else T.ToneDetector()


@pytest.mark.parametrize(
    "kind", [M.TONE_DATA[0], M.TONE_FAST[3], M.TONE_NARROW[1]], ids=lambda k: k.name
)
def test_the_snr_is_reported_in_the_ofdm_reference(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(7)
    for snr in (-18.0, -12.0, -6.0):
        y = _received(kind, _payload(rng, kind), snr_db=snr, lead=800, cfo_hz=20.0, seed=int(-snr))
        s = _detector(kind).detect(y)[0]
        frame = T.demodulate(y, kind, s.rv, s.start, s.cfo_hz)
        assert frame.snr_db == pytest.approx(snr, abs=1.0)


@pytest.mark.parametrize(
    ("kind", "snr_db"),
    [(M.TONE_DATA[0], -17.0), (M.TONE_FAST[1], -13.0), (M.TONE_FAST[3], -10.0)],
    ids=lambda v: v.name if isinstance(v, M.ToneKind) else str(v),
)
def test_frames_decode_through_the_detector_near_the_floor(kind: M.ToneKind, snr_db: float) -> None:
    """A decibel or two above each kind's 10 % point on AWGN."""
    rng = np.random.default_rng(8)
    det = T.ToneDetector()
    ok = 0
    for i in range(10):
        payload = _payload(rng, kind)
        lead = int(rng.integers(0, 3000))
        y = _received(
            kind,
            payload,
            snr_db=snr_db,
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
@pytest.mark.parametrize("narrow", [False, True], ids=["2300", "500"])
def test_a_minute_of_noise_is_never_a_frame(narrow: bool) -> None:
    """Each air's detector, with every pattern it looks for: the 500 Hz air's four for
    tone4x100-93 (ADR-0081) left its noise maximum where it was, 2.82–2.92 against 3.0."""
    det = T.ToneDetector(NARROW_DETECTOR_KINDS) if narrow else T.ToneDetector()
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


def _after_silence(
    seed: int, lead_zero: int = 30000, kind: M.ToneKind = M.TONE_DATA[1]
) -> tuple[np.ndarray, int, bytes]:
    """A frame (tone-36 unless ``kind`` says otherwise) straight after a span of exact
    silence — what a receiver holds while its own station transmits — with a little noise
    after it, and where it starts."""
    rng = np.random.default_rng(seed)
    payload = _payload(rng, kind)
    x = T.burst(kind, payload, 0)
    y = np.concatenate((np.zeros(lead_zero, complex), x, np.zeros(4000, complex)))
    noise = rng.normal(0, 0.03, len(y)) + 1j * rng.normal(0, 0.03, len(y))
    noise[:lead_zero] = 0
    return y + noise, lead_zero, payload


def test_a_frame_after_the_receivers_own_silence_is_not_read_a_block_early() -> None:
    """Found by two daemons over ``[sim]``: a station mutes its receiver while it transmits,
    and the peer's burst follows at once. A hypothesis whose first block lies in that
    silence and whose middle block sits on the frame's first had eight hits there, one
    "hit" from the silence — every tone ties at zero, and the argmax is tone 0, which this
    pattern holds — and three from the data: twelve. Taken as frames end, it came first and
    blocked the real frame; the receiver's arrivals and its acknowledgement's timing went
    with it. A silent symbol is no evidence, and a frame needs it in two blocks."""
    kind = M.TONE_DATA[1]
    det = T.ToneDetector()
    n = kind.num.symbol_samples
    y, start, payload = _after_silence(192)  # a payload whose data made the three
    early = start - (kind.block_offsets[1]) * n
    assert det.block_hits(y, kind, 0, early, 0.0) == [0, 8, 3]
    assert not det.confirmed(y, T.ToneSync(early, 0.0, kind, 0, 0.0))
    # offline and streaming alike, the frame itself — at its own start, decoded
    for found in (det.detect(y), _streamed(y)):
        assert [(s.kind, s.rv) for s in found] == [(kind, 0)], found
        assert abs(found[0].start - start) <= 4
        frame = T.demodulate(y, kind, 0, found[0].start, found[0].cfo_hz)
        assert frame.decode()[0] == payload


def _streamed(y: np.ndarray, block: int = 160) -> list[T.ToneSync]:
    stream = T.ToneStream()
    found: list[T.ToneSync] = []
    for i in range(0, len(y), block):
        found += stream.feed(y[: i + block], 0)
    return found


def test_a_fast_frame_after_silence_is_not_taken_for_its_early_reading() -> None:
    """ADR-0014: the hypothesis read a block-spacing early — its first block in the silence a
    station keeps while it transmits, its middle block on the frame's first — has its end
    block over the frame's data, and a fast kind's data can put four chance hits there: [0, 8,
    4] passes :meth:`ToneDetector.confirmed`. A receiver taking frames as they end took it
    first, and the real frame, overlapping it, was lost — once in 160 strong bursts. The real
    frame is announced as arriving long before the early reading ends, and a candidate that
    an announced frame starts inside, with a first block as strong as the candidate's whole,
    is not taken. Since ADR-0015 the reading fails confirmation as well: its end block lies
    over the frame's data, whose strong tones stand where the pattern wants its own —
    contradictions — and the stream's rule stays as the second defence."""
    kind = M.TONE_FAST[1]
    det = T.ToneDetector()
    y, start, payload = _after_silence(310, kind=kind)
    early = start - kind.block_offsets[1] * kind.num.symbol_samples
    reading = det.refine(y, kind, 0, early, 0.0)
    assert det.block_hits(y, kind, 0, reading.start, reading.cfo_hz) == [0, 8, 4]
    assert det.contradictions(y, reading) > det.MAX_CONTRADICTIONS
    assert not det.confirmed(y, reading)
    # the stream announces the real frame inside the reading's span, stronger than its whole
    stream = T.ToneStream(det)
    for i in range(0, start + 12 * kind.num.symbol_samples, 160):
        stream.feed(y[: i + 160], 0)
    arrival = next(a for a in stream.arriving if abs(a.start - start) <= 160)
    assert stream._announced_inside(early, kind, arrival.statistic)
    assert not stream._announced_inside(early, kind, arrival.statistic + 1e-9)
    for found in (det.detect(y), _streamed(y)):
        assert [(s.kind, s.rv) for s in found] == [(kind, 0)], found
        assert abs(found[0].start - start) <= 4
        frame = T.demodulate(y, kind, 0, found[0].start, found[0].cfo_hz)
        assert frame.decode()[0] == payload


def test_a_frame_that_slipped_a_symbol_stands_on_its_two_clean_blocks() -> None:
    """WC4Y's audio of 2026-10-05 (ADR-0049): an acceptance that reached him clean had 40 ms
    more audio in it than was sent, after its middle block, so its end block sat a symbol late
    — eight contradictions, each the frame's own next tone — and the detector refused a frame
    that decodes. Two whole uncontradicted blocks are a frame's alone."""
    kind = M.TONE_DATA[0]
    det = T.ToneDetector()
    n = kind.num.symbol_samples
    rng = np.random.default_rng(3)
    payload = _payload(rng, kind)
    x = T.burst(kind, payload, 0)
    at = 110  # past the middle block: the end block slips, the data mostly survives
    x = np.concatenate((x[: at * n], x[at * n - n : at * n], x[at * n :]))
    start = 3000
    y = np.concatenate((np.zeros(start, complex), x, np.zeros(3000, complex)))
    y = make_channel("awgn", snr_db=-4.0, fs=kind.num.fs, seed=3, signal_power=1.0).process(y)
    reading = det.refine(y, kind, 0, start, 0.0)
    assert det.block_hits(y, kind, 0, reading.start, reading.cfo_hz) == [8, 8, 0]
    assert det.contradictions(y, reading) > det.MAX_CONTRADICTIONS
    for found in (det.detect(y), _streamed(y)):
        assert [(f.kind, f.rv) for f in found] == [(kind, 0)], found
        assert abs(found[0].start - start) <= det.hop
        assert T.demodulate(y, kind, 0, found[0].start, found[0].cfo_hz).decode()[0] == payload


def test_a_burst_whose_first_frame_began_under_the_mute_is_not_lost_with_it() -> None:
    """WC4Y's Test of 2026-10-10 (ADR-0060): a peer's burst that begins while the receiver is
    still muted after its own transmission has no first block for its first frame. That
    frame's middle block was then announced as a frame of its own; the second frame's first
    block, inside that false arrival, was refused as weaker; and the second frame itself was
    refused because an arrival — its own middle block — started inside it. Each frame's
    middle block refused the next, and five tone frames at -2 dB were lost to the end of the
    burst. A candidate whose own first block stands is not refused by its own blocks — and,
    since ADR-0070, neither is one with two clean blocks, so the first frame,
    its first block silent, is found and decodes too."""
    kind = M.TONE_DATA[1]
    rng = np.random.default_rng(0)
    payloads = [_payload(rng, kind) for _ in range(3)]
    frames = [T.burst(kind, p, 0) for p in payloads]
    lead = 8000
    y = np.concatenate([np.zeros(lead, complex), *frames, np.zeros(8000, complex)])
    y += 0.3 * (rng.normal(size=len(y)) + 1j * rng.normal(size=len(y)))
    y[: lead + 4000] = 0  # the first half-second of the burst under the receiver's mute
    starts = [lead + i * kind.samples for i in range(3)]
    found = _streamed(y)
    assert [any(abs(f.start - x) <= 8 for f in found) for x in starts] == [True, True, True]
    first = min(found, key=lambda f: f.start)
    assert T.demodulate(y, kind, 0, first.start, first.cfo_hz).decode()[0] == payloads[0]


def test_a_frame_whose_first_block_faded_is_not_refused_by_its_own_middle_block() -> None:
    """WC4Y's probe of 2026-10-14, protocol 8's first session on the air: KK4ODA-1's audio held
    it clean, as strong as the call twelve seconds later that decoded at -6.2 dB, and the
    offline detector confirms it (blocks 3/8, 8/8, 7/8) — but its first 0.3 s arrived faded, so
    its first block did not stand. The stream announced its middle block as a frame arriving;
    that arrival, a first block stronger than the probe's whole, refused the probe (ADR-0014's
    rule; ADR-0060 excused a candidate's own blocks only when its first block stood). The
    arrival was never a frame and the probe was never looked at again: unanswered, live and on
    replay (11-step-4-first-air.md §6). A frame with two clean blocks of its own is a frame
    (ADR-0049), and its own later block does not refuse it."""
    kind = M.TONE_DATA[0]
    n = kind.num.symbol_samples
    rng = np.random.default_rng(0)
    payload = _payload(rng, kind)
    x = T.burst(kind, payload, 0)
    x[: T.SYNC_SYMBOLS * n] *= 10 ** (-16.0 / 20)  # the first sync block faded
    lead = 8000
    y = np.concatenate((np.zeros(lead, complex), x, np.zeros(8000, complex)))
    y = make_channel("awgn", snr_db=-8.0, fs=kind.num.fs, seed=0, signal_power=1.0).process(y)
    det = T.ToneDetector()
    offline = det.detect(y)
    assert [(s.kind, s.rv) for s in offline] == [(kind, 0)], offline
    hits = det.block_hits(y, kind, 0, offline[0].start, offline[0].cfo_hz)
    assert hits[0] < det.MIN_FIRST_HITS and min(hits[1:]) >= det.CLEAN_BLOCK_HITS, hits
    streamed = _streamed(y)
    assert [(s.kind, s.rv) for s in streamed] == [(kind, 0)], streamed
    frame = T.demodulate(y, kind, 0, streamed[0].start, streamed[0].cfo_hz)
    assert frame.decode()[0] == payload


def test_a_frame_taken_by_one_block_of_a_stronger_one_gives_way() -> None:
    """ADR-0060: a weak hypothesis whose last block is a real frame's first — its first and
    middle blocks on noise and on the silence of the receiver's own transmission — was taken
    before that frame's first block was even announced, and the real frame, overlapping it by
    that block, was kept out. A stronger frame overlapping a taken one by no more than a sync
    block is not overlapping it; by more, or weaker, it still is."""
    kind = M.TONE_NARROW[1]
    stream = T.ToneStream()
    n = kind.num.symbol_samples
    stream._taken = [(0, kind.samples, 4.4)]
    block = T.SYNC_SYMBOLS * n
    after = (kind.samples - block, 2 * kind.samples - block)
    assert not stream._overlaps(after, 10.0)
    assert stream._overlaps(after, 4.0)
    assert stream._overlaps((kind.samples - 3 * block, 2 * kind.samples), 10.0)


def test_a_false_arrival_gives_way_to_a_frame_announced_inside_it() -> None:
    """ADR-0014: with 25 patterns in the search a first block of noise, or of noise and a
    symbol or two of a strong frame's first block, now and then passes the announcement's
    threshold; announced, it covered the real frame's start, whose own announcement — "inside
    a frame already arriving" — was then refused as one of its middle blocks. A first block
    inside an arrival that is not one of its blocks, and stronger, replaces it."""
    kind = M.TONE_FAST[3]
    rng = np.random.default_rng(21)
    n = kind.num.symbol_samples
    # a lone first block of another kind's pattern, weak: announced, and no frame
    decoy = M.TONE_DATA[0]
    block = T.modulate(np.asarray(decoy.sync(1)), kind.num) * 10 ** (-28.0 / 20)
    frame = T.burst(kind, _payload(rng, kind), 0)
    lead, gap = 4000, 20 * n
    y = np.concatenate(
        (np.zeros(lead, complex), block, np.zeros(gap, complex), frame, np.zeros(6000, complex))
    )
    y += 0.2 * (rng.normal(size=len(y)) + 1j * rng.normal(size=len(y)))
    start = lead + len(block) + gap
    stream = T.ToneStream()
    announced: dict[int, M.ToneKind] = {}
    found: list[T.ToneSync] = []
    for i in range(0, len(y), 160):
        found += stream.feed(y[: i + 160], 0)
        for a in stream.arriving:
            announced.setdefault(a.start, a.kind)
        if found:
            break
    # the decoy was announced — and the frame, inside its span, was announced too
    assert any(abs(a - lead) <= 160 and k is decoy for a, k in announced.items()), announced
    assert any(abs(a - start) <= 160 and k is kind for a, k in announced.items()), announced
    assert [(s.kind, abs(s.start - start) <= 4) for s in found] == [(kind, True)]


@pytest.mark.parametrize("kind", [M.TONE_DATA[0], M.TONE_CONTROL])
def test_an_unconfirmed_frames_end_block_is_not_announced_as_a_frame(kind: M.ToneKind) -> None:
    """The scenario harness, 20 m across the auroral zone (ADR-0067): a tone frame the
    Doppler spread kept from being confirmed was announced at its start, and its end block —
    its first block repeated — was announced as a frame of its own as the frame ended, the
    first announcement having lapsed with it. The receiving station held its acknowledgement
    a whole frame for a frame that never came, past the sender's wait, and the two keyed at
    once. A first block at an announced frame's own middle or end block is that frame's,
    read after the frame's end as before it."""
    rng = np.random.default_rng(3)
    frame = T.burst(kind, _payload(rng, kind), 0)
    lead = 4000
    y = np.concatenate((np.zeros(lead, complex), frame, np.zeros(kind.samples, complex)))
    y += 0.05 * (rng.normal(size=len(y)) + 1j * rng.normal(size=len(y)))
    stream = T.ToneStream()
    stream.det.confirmed = lambda buf, sync: False  # type: ignore[method-assign]
    announced: dict[int, M.ToneKind] = {}
    for i in range(0, len(y), 160):
        assert not stream.feed(y[: i + 160], 0)
        for a in stream.arriving:
            announced.setdefault(a.start, a.kind)
    assert len(announced) == 1 and abs(next(iter(announced)) - lead) <= 160, announced


def test_the_sync_blocks_tell_a_steady_channel_from_flutter() -> None:
    """ADR-0080 §2.4: the wobble of the sync tones about a straight line through each block is
    about 0 where the channel holds or fades smoothly over a block, and about 1 on ITU flutter
    (10 Hz), where the levels read from the data are only a window's mean."""
    kind = M.TONE_NARROW[2]
    rng = np.random.default_rng(31)
    layout = kind.layout(0, 0)

    def wobble(channel: str, seed: int) -> float:
        y = _received(
            kind, _payload(rng, kind), snr_db=6.0, lead=700, cfo_hz=0.0, seed=seed, channel=channel
        )
        e = T.tone_energies(y, 700, 0.0, kind.symbols, kind.num)
        return T.fade_wobble(e / T.sync_noise(e, layout, kind), layout)

    steady = [wobble("awgn", 40 + i) for i in range(4)]
    flutter = [wobble("flutter", 50 + i) for i in range(4)]
    assert max(abs(w) for w in steady) < 0.1, steady
    assert min(flutter) > 0.4, flutter
    # a silent symbol in a block leaves the block out; a muted symbol is no evidence
    e = np.zeros((kind.symbols, M.TONE_NUMEROLOGY.tones))
    assert T.fade_wobble(e, layout) == 0.0
    levels = np.full((3, 4), 2.0)
    data = np.array([[0.0, 0.0, 0.0, 0.0], [5.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]])
    out = T.tracked_metrics(data, levels, 1.0)
    assert (out[0] == 0).all() and (out[2] == 0).all() and out[1, 0] > out[1, 1]


def test_a_four_tone_frame_on_flutter_reads_as_well_as_the_sync_blocks_level() -> None:
    """ITU flutter turns the channel over within the quarter second the levels are read over.
    With the known level's metric there, a tone in an instant's fade under a high mean was a
    confident error — tone4x100-93 decoded 12 frames in 60 at 0 dB where the sync blocks'
    common level decoded 60. The Nakagami metric at the frame's measured wobble reads it as
    the common level did."""
    kind = M.TONE_NARROW[2]
    rng = np.random.default_rng(32)
    ok = {0: 0, 4: 0}
    try:
        for i in range(12):
            payload = _payload(rng, kind)
            y = _received(
                kind, payload, snr_db=3.0, lead=900, cfo_hz=0.0, seed=60 + i, channel="flutter"
            )
            for tracked in ok:
                T.TRACKED_TONES = tracked
                ok[tracked] += T.demodulate(y, kind, 0, 900, 0.0).decode()[0] == payload
    finally:
        T.TRACKED_TONES = 4
    assert ok[4] >= ok[0] - 1 and ok[4] >= 10, ok


def test_silence_under_a_frame_is_left_out_of_its_noise() -> None:
    """A frame half under the receiver's own transmission — the station keyed over it —
    read its noise as zero from the silent symbols and its SNR as 290 dB, which a rate
    controller would take at its word."""
    kind = M.TONE_DATA[0]
    rng = np.random.default_rng(4)
    payload = _payload(rng, kind)
    lead = 3000
    y = _received(kind, payload, snr_db=10.0, lead=lead, cfo_hz=0.0, seed=4)
    whole = T.demodulate(y, kind, 0, lead, 0.0)
    muted = y.copy()
    muted[lead + kind.samples // 2 : lead + kind.samples] = 0
    half = T.demodulate(muted, kind, 0, lead, 0.0)
    assert abs(half.snr_db - whole.snr_db) < 4.0, (half.snr_db, whole.snr_db)
