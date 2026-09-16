"""The 500 Hz waveform (roadmap P7-0): its numerology, its mode table and its floor.

The narrow air interface shares the wide one's symbol timing and frame layouts, so the
link layer's clocks are untouched; what changes is the carrier count, the mode table and
the chip sequences that signal the mode. These tests pin the numbers ADR-0002 anticipated,
prove every narrow mode decodes, and put the floor where the design says it is: the
narrow control mode (QPSK ½) reaching about the same 3 kHz-referenced SNR as the wide
table's BPSK ⅕, because the same power sits in a fifth of the band."""

from __future__ import annotations

import numpy as np
import pytest

from aether_model.channel import make_channel
from aether_model.frame.modes import (
    LONG,
    MODES,
    NARROW,
    NARROW_FLOOR_LONG,
    NARROW_FLOOR_SHORT,
    NARROW_LONG,
    NARROW_MODES,
    NARROW_SHORT,
    SHORT,
    WIDE,
    air_interface,
)
from aether_model.link.engine import LinkConfig, LinkEngine, State
from aether_model.link.frames import CONNECT_BODY_BYTES, CONTROL_BYTES, bandwidth_code
from aether_model.link.harness import bandwidth_capabilities, phy_timing, two_modem_sim
from aether_model.link.rate import (
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_FRAME_S,
    NARROW_PAYLOAD_BYTES,
    usable_modes,
)
from aether_model.phy.passband import AudioToBaseband
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType, mode_chip_sequences, preamble
from aether_model.waveform import NARROW_500, WIDE_2300, Modulation

P = NARROW_500
FS = P.fs_baseband


@pytest.fixture(scope="module")
def modem() -> Modem:
    return Modem(P)


def _buffer(burst: np.ndarray, lead: int = 1200, tail: int = 1200) -> np.ndarray:
    return np.concatenate((np.zeros(lead), burst, np.zeros(tail)))


# ── numerology ────────────────────────────────────────────────────────


def test_narrow_numerology_matches_adr_0002() -> None:
    assert P.n_carriers == 12
    assert P.occupied_bandwidth_hz == pytest.approx(480.0)
    assert P.n_pilot_carriers == 4 and P.n_data_carriers == 8
    # the same clock as the wide waveform: the link layer's timers do not change
    assert P.symbol_samples == WIDE_2300.symbol_samples
    assert NARROW_LONG.duration_s == LONG.duration_s
    assert NARROW_SHORT.duration_s == SHORT.duration_s
    assert NARROW_LONG.samples == LONG.samples
    assert NARROW_LONG.qam_symbols == 28 * 8
    assert NARROW_SHORT.qam_symbols == 10 * 8
    assert air_interface(NARROW_500) is NARROW
    assert air_interface(WIDE_2300) is WIDE


def test_narrow_preamble_keeps_the_frame_types_apart() -> None:
    pre = preamble(P)
    a = pre.sc_values(FrameType.DATA)[pre.even]
    b = pre.sc_values(FrameType.CONTROL)[pre.even]
    assert len(pre.even) == 6
    # the same seeds as the wide waveform, drawn to six chips, come out orthogonal
    assert abs(np.vdot(a, b)) == pytest.approx(0.0)


def test_narrow_chip_set_is_its_own_and_the_wide_set_is_untouched() -> None:
    pre = preamble(P)
    assert pre.n_chips == 32
    assert len(pre.sequences) == 4 * len(NARROW_MODES) == 52
    seqs = np.array(pre.sequences)
    gram = np.abs(seqs @ seqs.conj().T) / pre.n_chips
    np.fill_diagonal(gram, 0.0)
    assert gram.max() <= NARROW.chip_correlation_bound + 1e-12
    # (mode, rv) round-trips through this air interface's indexing
    for mode in range(len(NARROW_MODES)):
        for rv in range(4):
            assert pre.chip_hypothesis(pre.chip_index(mode, rv)) == (mode, rv)
    with pytest.raises(ValueError):
        pre.chip_index(len(NARROW_MODES), 0)
    # the wide waveform's 56 sequences of 168 chips are the P2-3 set, bit for bit
    wide = preamble(WIDE_2300)
    assert wide.n_chips == 168 and len(wide.sequences) == 56
    assert all(
        np.array_equal(x, y) for x, y in zip(wide.sequences, mode_chip_sequences(168), strict=True)
    )


# ── the mode table ────────────────────────────────────────────────────


def test_narrow_mode_table() -> None:
    assert len(NARROW_MODES) == 13
    assert NARROW.floor_modes == 2 and NARROW.control_mode is NARROW_MODES[3]
    assert NARROW.control_mode.modulation is Modulation.QPSK
    assert NARROW.control_mode.code_rate == 1 / 2
    # a control frame fits the SHORT layout at the control mode, exactly as at 2 300 Hz
    assert NARROW.control_mode.payload_bytes(NARROW_SHORT) == CONTROL_BYTES == 7
    assert MODES[0].payload_bytes(SHORT) == 7
    # and a connect request fits the LONG layout at the control mode (header, length and
    # body), and the floor layout at mode 1 — but not mode 0, whose frame is for data
    assert NARROW.control_mode.payload_bytes(NARROW_LONG) >= CONNECT_BODY_BYTES + 5
    assert NARROW_MODES[1].payload_bytes(NARROW_FLOOR_LONG) >= CONNECT_BODY_BYTES + 5
    assert NARROW_MODES[0].payload_bytes(NARROW_FLOOR_LONG) < CONNECT_BODY_BYTES + 5
    # the ordinary modes each carry more than the one before; the floor modes carry
    # 19 and 41 bytes in a frame four times as long; the fastest reaches a kilobit
    payloads = [m.payload_bytes(NARROW.data_layout(m.index)) for m in NARROW_MODES]
    assert payloads[2:] == sorted(payloads[2:])
    assert payloads[:4] == [19, 41, 15, 25] and payloads[-1] == 137
    assert NARROW_FLOOR_LONG.duration_s == pytest.approx(4 * NARROW_LONG.duration_s, abs=0.2)
    # a floor control frame carries a control frame with a byte over
    assert NARROW.floor_control_mode.payload_bytes(NARROW_FLOOR_SHORT) == CONTROL_BYTES + 1
    assert {
        m.index: pytest.approx(NARROW.data_layout(m.index).duration_s, abs=1e-3)
        for m in NARROW_MODES
    } == NARROW_FRAME_S
    assert NARROW_MODES[-1].net_bit_rate(NARROW_LONG) == pytest.approx(1040, abs=1)
    # the rate controller's copies agree with the table
    assert {
        m.index: m.payload_bytes(NARROW.data_layout(m.index)) for m in NARROW_MODES
    } == NARROW_PAYLOAD_BYTES
    assert set(NARROW_AWGN_THRESHOLD_DB) == {m.index for m in NARROW_MODES}
    assert usable_modes(NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S)[:4] == [
        0,
        1,
        2,
        3,
    ]


# ── the air ───────────────────────────────────────────────────────────


def test_every_narrow_mode_decodes_at_20_db(modem: Modem) -> None:
    rng = np.random.default_rng(1)
    for mode in modem.modes:
        payload = rng.integers(0, 256, modem.payload_bytes(mode), dtype=np.uint8).tobytes()
        ch = make_channel(
            "awgn",
            snr_db=20.0,
            fs=FS,
            seed=3 + mode.index,
            signal_power=1.0,
            cfo_hz=37.0,
            sro_ppm=20.0,
        )
        frames = modem.decode_buffer(ch.process(_buffer(modem.data_burst(payload, mode))))
        assert len(frames) == 1, mode.name
        assert frames[0].payload == payload, mode.name
        assert frames[0].frame.mode == mode.index
        assert frames[0].frame.cfo_hz == pytest.approx(37.0, abs=1.0)


def test_narrow_control_frame_is_recognised_and_decodes(modem: Modem) -> None:
    payload = bytes(range(7))
    y = make_channel("awgn", snr_db=0.0, fs=FS, seed=4, signal_power=1.0).process(
        _buffer(modem.control_burst(payload))
    )
    frames = modem.decode_buffer(y)
    assert len(frames) == 1
    assert frames[0].frame.sync.header.frame_type is FrameType.CONTROL
    assert frames[0].payload == payload


def test_narrow_control_mode_is_where_the_wide_floor_is(modem: Modem) -> None:
    """The control mode (QPSK ½) at −5 dB (3 kHz) with random offsets: the same SNR the
    wide table's BPSK ⅕ is held to (``test_low_snr_acquisition_and_decode``), reached on a
    fifth of the bandwidth."""
    rng = np.random.default_rng(11)
    mode = modem.air.control_mode
    ok = 0
    for trial in range(6):
        payload = rng.integers(0, 256, modem.payload_bytes(mode), dtype=np.uint8).tobytes()
        ch = make_channel(
            "awgn",
            snr_db=-5.0,
            fs=FS,
            seed=500 + trial,
            signal_power=1.0,
            cfo_hz=float(rng.uniform(-250, 250)),
            sro_ppm=float(rng.uniform(-80, 80)),
        )
        frames = modem.decode_buffer(ch.process(_buffer(modem.data_burst(payload, mode))))
        ok += int(len(frames) == 1 and frames[0].payload == payload)
    assert ok >= 5


def test_narrow_reported_snr_is_calibrated(modem: Modem) -> None:
    rng = np.random.default_rng(9)
    mode = modem.modes[4]
    payload = rng.integers(0, 256, modem.payload_bytes(mode), dtype=np.uint8).tobytes()
    y = make_channel("awgn", snr_db=10.0, fs=FS, seed=9, signal_power=1.0).process(
        _buffer(modem.data_burst(payload, mode))
    )
    f = modem.decode_buffer(y)[0]
    assert f.frame.snr_3k_db == pytest.approx(10.0, abs=1.0)


def test_narrow_full_audio_path_round_trip(modem: Modem) -> None:
    rng = np.random.default_rng(5)
    mode = modem.modes[6]
    payload = rng.integers(0, 256, modem.payload_bytes(mode), dtype=np.uint8).tobytes()
    audio = modem.tx.audio(
        np.concatenate((np.zeros(800), modem.data_burst(payload, mode), np.zeros(800)))
    )
    audio = (audio * 0.3).astype(np.float32)
    y = AudioToBaseband(P).process(audio.astype(np.float64))
    frames = modem.decode_buffer(y)
    assert len(frames) == 1 and frames[0].payload == payload


# ── the link over the narrow air ──────────────────────────────────────


def test_narrow_session_over_the_real_phy() -> None:
    """Two engines through the narrow modem: connect, transfer, disconnect, with the
    rate controller stepping the narrow table and never a wide index."""
    timing = phy_timing(P)
    assert timing.mode_threshold_db == NARROW_AWGN_THRESHOLD_DB
    assert timing.capacity(3) == 25 and timing.capacity(0) == 19
    assert timing.floor_modes == 2 and timing.floor_data_frame_s == pytest.approx(4.216, abs=1e-3)
    cfg = LinkConfig(capabilities=bandwidth_capabilities(P))
    assert bandwidth_code(cfg.capabilities) == 1
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    assert a.rate.modes == usable_modes(
        NARROW_AWGN_THRESHOLD_DB, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S
    )
    sim = two_modem_sim(a, b, params=P, channel="awgn", snr_db=8.0, seed=7)
    msg = b"Aether HF at 500 Hz: the bandwidth P2P contacts are made in. 73" * 2
    a.connect("KK4XYZ")
    a.send(msg)
    sim.run(until=30)
    assert a.connected and b.connected
    assert bandwidth_code(b.peer_capabilities) == 1
    a.disconnect()
    sim.run(until=600)
    assert sim.delivered(1) == msg
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.bridge.modes_sent and max(sim.bridge.modes_sent) < len(NARROW_MODES)
