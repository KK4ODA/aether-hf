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
KINDS = (*FLOOR_KINDS, *M.TONE_FAST)


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
    assert M.WIDE.tone_data == ladder and M.NARROW.tone_data == M.TONE_DATA


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
    for kind in (M.TONE_DATA[0], *M.TONE_FAST):
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


@pytest.mark.parametrize("kind", [M.TONE_DATA[0], M.TONE_FAST[3]], ids=lambda k: k.name)
def test_the_snr_is_reported_in_the_ofdm_reference(kind: M.ToneKind) -> None:
    rng = np.random.default_rng(7)
    for snr in (-18.0, -12.0, -6.0):
        y = _received(kind, _payload(rng, kind), snr_db=snr, lead=800, cfo_hz=20.0, seed=int(-snr))
        s = T.ToneDetector().detect(y)[0]
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
    is not taken."""
    kind = M.TONE_FAST[1]
    det = T.ToneDetector()
    y, start, payload = _after_silence(310, kind=kind)
    early = start - kind.block_offsets[1] * kind.num.symbol_samples
    reading = det.refine(y, kind, 0, early, 0.0)
    assert det.block_hits(y, kind, 0, reading.start, reading.cfo_hz) == [0, 8, 4]
    assert det.confirmed(y, reading)
    for found in (det.detect(y), _streamed(y)):
        assert [(s.kind, s.rv) for s in found] == [(kind, 0)], found
        assert abs(found[0].start - start) <= 4
        frame = T.demodulate(y, kind, 0, found[0].start, found[0].cfo_hz)
        assert frame.decode()[0] == payload


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
