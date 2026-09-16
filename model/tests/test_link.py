"""Link layer: frame formats, ARQ engine and session FSM over the lossy-pipe simulator
(roadmap P2-1). The bit-exact-over-the-real-PHY check lives in ``test_link_harness.py``."""

from __future__ import annotations

from itertools import pairwise

import pytest

from aether_model.frame.modes import LONG, MODES, SHORT
from aether_model.link.engine import LinkConfig, LinkEngine, Role, State, Transmit
from aether_model.link.frames import (
    CAP_COMPRESSION,
    CONNECT_BODY_BYTES,
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    ProbeBody,
    bandwidth_code,
    decode_data,
    encode_data,
    pack_callsign,
    unpack_callsign,
    with_bandwidth,
)
from aether_model.link.phy import PhyTiming
from aether_model.link.rate import AWGN_THRESHOLD_DB, RateController, usable_modes
from aether_model.link.sim import TwoStationSim


@pytest.fixture(scope="module")
def timing() -> PhyTiming:
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    return PhyTiming(
        data_frame_s=LONG.duration_s, control_frame_s=SHORT.duration_s, data_capacity=caps
    )


def _pair(timing: PhyTiming, config: LinkConfig | None = None) -> tuple[LinkEngine, LinkEngine]:
    return (
        LinkEngine("W4ODA", timing, config, seed=1),
        LinkEngine("KK4XYZ", timing, config, seed=2),
    )


# ── frame formats ─────────────────────────────────────────────────────


@pytest.mark.parametrize("call", ["W4ODA", "KK4XYZ", "M0ABC", "VK2DEF/P", "A1", "9Z9ZZZ9ZZ"])
def test_callsign_round_trip(call: str) -> None:
    assert unpack_callsign(pack_callsign(call)) == call
    assert len(pack_callsign(call)) == 7


def test_callsign_rejects_bad_input() -> None:
    with pytest.raises(ValueError):
        pack_callsign("")
    with pytest.raises(ValueError):
        pack_callsign("TOOLONGCALL")
    with pytest.raises(ValueError):
        pack_callsign("W4ODA!")


@pytest.mark.parametrize("mode_idx", [0, 4, 13])
def test_data_frame_round_trip_full_and_partial(mode_idx: int) -> None:
    cap = MODES[mode_idx].payload_bytes(LONG)
    for body_len in (0, 5, cap - 3, cap - 5):
        if body_len < 0:
            continue
        body = bytes(range(256))[:body_len] if body_len <= 256 else bytes(body_len)
        header = DataHeader(DataKind.DATA, seq=200, session=42)
        frame = encode_data(header, body, cap)
        assert len(frame) == cap
        got_header, got_body = decode_data(frame)
        assert got_header == header
        assert got_body == body


def test_data_frame_is_identical_across_retransmissions() -> None:
    """HARQ combines identical codewords: the encoded bytes must not depend on anything
    outside (header, body, capacity)."""
    cap = MODES[4].payload_bytes(LONG)
    body = bytes(range(100))
    h = DataHeader(DataKind.DATA, seq=7, session=3)
    assert encode_data(h, body, cap) == encode_data(h, body, cap)


def test_connect_body_round_trip() -> None:
    body = ConnectBody("W4ODA", "KK4XYZ", caps=0b101, version=1)
    assert ConnectBody.decode(body.encode()) == body


def test_a_connect_body_carries_the_snr_the_request_arrived_at() -> None:
    # the acceptance says how the request was heard (P9-2); a request says nothing, and a
    # body from an earlier version, which stops at the version byte, reads as nothing
    ack = ConnectBody("KK4XYZ", "W4ODA", caps=0b010, snr_db=12.5)
    assert len(ack.encode()) == CONNECT_BODY_BYTES == 17
    assert ConnectBody.decode(ack.encode()).snr_db == 12.0  # whole dB, ties to even
    req = ConnectBody("W4ODA", "KK4XYZ")
    assert ConnectBody.decode(req.encode()).snr_db is None
    old = req.encode()[:16]
    assert ConnectBody.decode(old) == req


def test_a_session_starts_at_the_mode_the_connect_frames_measured(timing: PhyTiming) -> None:
    """P9-2: the first burst is not sent at the slowest mode and climbed from; the
    acceptance carries the SNR the request arrived at, and the caller starts one step
    below what that supports. Both controllers start from the connect frame they
    decoded, so the called station's first recommendation is not mode 0 either."""
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=31)
    first: list[int] = []
    original = a._send_burst

    def wrapped() -> None:
        first.append(min(a._recommended, a.cfg.max_mode))
        original()

    a._send_burst = wrapped  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    a.send(bytes(600))
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == bytes(600)
    expected = RateController().first_mode(15.0)
    assert expected > 0
    assert first[0] == expected, first
    # and the climb is still allowed from there
    assert max(first) >= expected


def test_the_first_mode_keeps_a_step_in_hand() -> None:
    rc = RateController()
    # far below every mode but the slowest: the slowest
    assert rc.first_mode(-10.0) == usable_modes()[0]
    # one step below the fastest that fits with margin and hysteresis
    for snr in (4.0, 9.0, 15.0, 20.0):
        modes = rc.modes
        fits = [
            m for m in modes if AWGN_THRESHOLD_DB[m] + rc.margin_db + rc.up_hysteresis_db <= snr
        ]
        top = modes.index(fits[-1])
        assert rc.first_mode(snr) == modes[max(0, top - rc.first_mode_back)], snr
    # seeding places the controller there and takes the measurement, once
    rc.seed(15.0)
    assert rc.recommend() == rc.first_mode(15.0)
    assert rc.snr_db == 15.0
    rc.seed(2.0)
    assert rc.snr_db == 15.0  # a second seed changes nothing


def test_probe_body_round_trips_and_clamps_its_snr() -> None:
    probe = ProbeBody("W4ODA", "KK4XYZ", None, caps=0b10)
    assert ProbeBody.decode(probe.encode()) == probe
    assert len(probe.encode()) == 16
    for snr, expect in [(-7.4, -7.0), (12.5, 12.0), (13.5, 14.0), (200.0, 40.0), (-200.0, -40.0)]:
        ack = ProbeBody("KK4XYZ", "W4ODA", snr, caps=0)
        assert ProbeBody.decode(ack.encode()).snr_db == expect, snr
    # a whole-decibel byte, 0x7F meaning "not measured", as the control frame's SNR byte
    assert ProbeBody.decode(b"\0" * 14 + bytes([0x7F, 0])).snr_db is None


def test_control_frame_round_trip() -> None:
    ctl = ControlFrame(
        ControlKind.ACK,
        session=9,
        flags=ControlFlags.WANT_TX,
        base=100,
        bitmap=0b1011,
        snr_db=-7.0,
        recommended_mode=6,
        counter=5,
    )
    got = ControlFrame.decode(ctl.encode())
    assert (got.kind, got.session, got.flags, got.base, got.bitmap) == (
        ctl.kind,
        ctl.session,
        ctl.flags,
        ctl.base,
        ctl.bitmap,
    )
    assert got.snr_db == -7.0 and got.recommended_mode == 6 and got.counter == 5


def test_ack_received_semantics() -> None:
    ack = ControlFrame(ControlKind.ACK, session=0, base=10, bitmap=0b0101)  # 10 and 12 present
    assert ack.received(10) and ack.received(12)
    assert not ack.received(11) and not ack.received(13)
    assert ack.received(9) and ack.received(5)  # below base = already acknowledged


# ── rate controller ───────────────────────────────────────────────────


def test_usable_modes_are_pareto_and_sorted() -> None:
    modes = usable_modes()
    assert modes == sorted(modes)
    # mode 7 (8-PSK 2/3) is dominated by mode 8 (16-QAM 1/2): more payload, lower threshold
    assert 7 not in modes
    for m in modes:
        assert not any(
            AWGN_THRESHOLD_DB[o] <= AWGN_THRESHOLD_DB[m] and o != m and o in modes
            for o in modes
            if o > m
        )


def _settle(rc: RateController, snr_db: float, bursts: int = 20) -> int:
    """Feed clean bursts at a fixed SNR until the recommendation stops moving."""
    for _ in range(bursts):
        rc.observe(snr_db, ok=6, failed=0)
    return rc.recommend()


def test_rate_recommendation_climbs_with_snr() -> None:
    floor = usable_modes()[0]
    prev = -1
    for snr in range(-6, 22, 2):
        m = _settle(RateController(), float(snr))
        assert m >= prev, (snr, m, prev)
        # never recommends a mode the SNR cannot carry — unless already on the slowest one
        assert m == floor or AWGN_THRESHOLD_DB[m] <= snr, (snr, m)
        prev = m


def test_rate_controller_converges_quickly() -> None:
    """A clean link must reach its final mode in a handful of bursts, not crawl up the table
    one mode at a time for half a minute."""
    for snr in (0.0, 8.0, 14.0, 20.0):
        rc = RateController()
        final = _settle(RateController(), snr)
        track = [rc.observe(snr, ok=6, failed=0) or rc.recommend() for _ in range(20)]
        assert track.index(final) + 1 <= 8, (snr, track)


def _boundary_track(rc: RateController, snr_db: float, bursts: int) -> list[int]:
    """Drive the controller against a channel that fails any mode needing more than
    ``snr_db − 1`` dB, and return the sequence of recommendations."""
    track = []
    for _ in range(bursts):
        m = rc.recommend()
        track.append(m)
        failed = 3 if AWGN_THRESHOLD_DB[m] > snr_db - 1.0 else 0
        rc.observe(snr_db, ok=6 - failed, failed=failed)
    return track


def test_rate_controller_does_not_oscillate_at_a_mode_boundary() -> None:
    """The point of the hysteresis: parked where two modes are nearly equally plausible, the
    controller must settle instead of flapping and losing a burst to every change."""
    track = _boundary_track(RateController(), 9.0, bursts=40)
    assert len(set(track[-20:])) == 1, track  # perfectly steady once settled
    assert AWGN_THRESHOLD_DB[track[-1]] <= 9.0


def test_rate_controller_backs_off_fast_when_the_channel_collapses() -> None:
    """Fast down, slow up: a 14 dB collapse must be absorbed in a couple of bursts."""
    rc = RateController()
    high = _settle(rc, 18.0)
    track = _boundary_track(rc, 4.0, bursts=12)
    assert track[0] == high
    sustainable = next(i for i, m in enumerate(track) if AWGN_THRESHOLD_DB[m] <= 3.0)
    assert sustainable <= 4, track
    assert len(set(track[-5:])) == 1, track  # and it stays there


def test_rate_margin_backs_off_on_failures() -> None:
    rc = RateController()
    rc.observe(10.0, ok=6, failed=0)
    good_mode = rc.recommend()
    base = rc.margin_db
    for _ in range(3):
        rc.observe(10.0, ok=0, failed=6)
    assert rc.margin_db > base  # failures widen the margin
    assert rc.recommend() <= good_mode  # a wider margin never picks a faster mode


def _track_burst_modes(engine: LinkEngine) -> list[int]:
    """Record the mode of every burst the engine sends (the controller's live decisions —
    ``engine.rate`` itself is reset when the session ends)."""
    track: list[int] = []
    original = engine._send_burst

    def wrapped() -> None:
        track.append(min(engine._recommended, engine.cfg.max_mode))
        original()

    engine._send_burst = wrapped  # type: ignore[method-assign]
    return track


# ── session FSM over the simulator ────────────────────────────────────


def _run_transfer(timing: PhyTiming, msg: bytes, snr_db: float, seed: int) -> TwoStationSim:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=seed)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=2000)
    return sim


def test_connect_transfer_disconnect_clean_channel(timing: PhyTiming) -> None:
    msg = b"The quick brown fox jumps over the lazy dog. " * 20
    sim = _run_transfer(timing, msg, snr_db=15.0, seed=7)
    a, b = sim.st[0].engine, sim.st[1].engine
    assert sim.delivered(1) == msg
    # what the sender saw acknowledged is what the receiver delivered
    assert a.stats.bytes_acked == len(msg)
    assert b.stats.bytes_delivered == len(msg)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert "connected:KK4XYZ (iss)" in sim.events(0)
    assert any(e.startswith("disconnected") for e in sim.events(0))
    assert any(e.startswith("disconnected") for e in sim.events(1))


@pytest.mark.parametrize("snr_db", [20.0, 12.0, 6.0, 3.0])
def test_transfer_completes_across_snr(timing: PhyTiming, snr_db: float) -> None:
    msg = bytes((i * 37) % 256 for i in range(1200))
    sim = _run_transfer(timing, msg, snr_db=snr_db, seed=int(snr_db) + 3)
    assert sim.delivered(1) == msg


def test_throughput_increases_with_snr(timing: PhyTiming) -> None:
    msg = bytes(4000)
    times = {}
    for snr in (4.0, 18.0):
        a, b = _pair(timing)
        sim = TwoStationSim(a, b, snr_db=snr, seed=11)
        a.connect("KK4XYZ")
        a.send(msg)
        a.disconnect()
        times[snr] = sim.run(until=3000)
        assert sim.delivered(1) == msg
    assert times[18.0] < 0.6 * times[4.0]


def test_harq_ir_rescues_below_threshold(timing: PhyTiming) -> None:
    """Mode pinned to QPSK ½ (threshold ≈ +1 dB) and driven at −3 dB: the transfer only
    completes because retransmissions are soft-combined."""
    cfg = LinkConfig(initial_mode=4, max_mode=4, max_retries=60)
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    sim = TwoStationSim(a, b, snr_db=-3.0, seed=99)
    msg = bytes(1500)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=6000)
    assert sim.delivered(1) == msg
    assert b.stats.harq_rescues > 0


def test_break_hands_over_the_channel(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=5)
    reply = b"REPLY FROM KK4XYZ. " * 8
    a.connect("KK4XYZ")
    a.send(bytes(3000))
    b.send(reply)
    b.request_break()
    sim.run(until=600)
    assert sim.delivered(1) == bytes(3000)  # A → B still completes
    assert sim.delivered(0) == reply  # B → A after the handover
    assert a.stats.turns == 1


def test_connect_fails_when_peer_is_absent(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    b.state = State.IDLE  # B never listens: give it a session that ignores everything
    sim = TwoStationSim(a, b, snr_db=15.0, seed=1)
    # deafen B by dropping every frame to it
    sim._busy = lambda who, t0, t1: who == 1 or TwoStationSim._busy(sim, who, t0, t1)  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert a.state is State.IDLE
    assert any(e == "disconnected:no answer" for e in sim.events(0))


def test_simultaneous_connect_resolves(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=2)
    a.connect("KK4XYZ")
    b.connect("W4ODA")
    a.send(b"data from A" * 3)
    sim.run(until=300)
    # exactly one becomes ISS, one IRS, and they agree on a session
    assert {a.role, b.role} == {Role.ISS, Role.IRS}
    assert a.session == b.session
    assert sim.delivered(1) == b"data from A" * 3


def test_a_station_answers_to_every_callsign_it_was_given(timing: PhyTiming) -> None:
    # a host program owns the operator's callsign (MYCALL), and sends several when the
    # station also answers to a club or tactical call; the station answers as the one that
    # was called, so the caller sees the callsign it asked for
    a, b = _pair(timing)
    b.set_callsigns(["KK4XYZ", "KK4XYZ-T"])
    sim = TwoStationSim(a, b, snr_db=15.0, seed=5)
    a.connect("KK4XYZ-T")
    a.send(b"to the tactical call")
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == b"to the tactical call"
    assert b.my_call == "KK4XYZ-T"
    assert "connected:W4ODA (irs)" in sim.events(1)
    assert "connected:KK4XYZ-T (iss)" in sim.events(0)


def test_a_message_one_byte_short_of_a_full_frame_still_crosses(timing: PhyTiming) -> None:
    # the DATA container carries a full body with no length, or a partial body with two
    # length bytes; a body one byte short of full is neither, and the sender must not try
    # to build it — the modem raised on exactly this length mid-session
    from aether_model.link.frames import data_capacity

    a, b = _pair(timing)
    cap = data_capacity(timing.capacity(LinkConfig().initial_mode))
    for length in (cap - 1, cap, cap + 1, 2 * cap - 1):
        a, b = _pair(timing)
        message = bytes(range(256)) * (length // 256 + 1)
        message = message[:length]
        sim = TwoStationSim(a, b, snr_db=15.0, seed=11)
        a.connect("KK4XYZ")
        a.send(message)
        a.disconnect()
        sim.run(until=600)
        assert sim.delivered(1) == message, length


def test_the_sender_learns_how_the_other_station_hears_it(timing: PhyTiming) -> None:
    # every acknowledgement carries the SNR the receiver measured on the burst, and the
    # sender keeps the last one: it is the one number an operator cannot read off their
    # own receiver, and the one a panel shows as "they hear you at"
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=9)
    a.connect("KK4XYZ")
    a.send(bytes([0x5A]) * 400)
    sim.run(until=40)
    assert a.state is State.CONNECTED
    assert a.peer_snr_db is not None
    assert abs(a.peer_snr_db - 15.0) < 3.0
    # and the report belongs to the session: it is gone when the session is
    a.disconnect()
    sim.run(until=300)
    assert a.state is State.IDLE
    assert a.peer_snr_db is None


def test_a_probe_is_answered_with_the_snr_it_arrived_at(timing: PhyTiming) -> None:
    # "can you hear me, and how well?" without a session: the probed station answers with
    # the SNR the probe arrived at, and the prober reports both directions of the path
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=21)
    a.probe("KK4XYZ")
    sim.run(until=60)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert "probed:W4ODA at 15.0 dB" in sim.events(1)
    assert "probe:KK4XYZ hears us at 15 dB, heard at 15.0 dB" in sim.events(0)
    assert (a.stats.probes_sent, a.stats.probe_replies, b.stats.probes_answered) == (1, 1, 1)
    # the question can be asked again, and a session can follow
    a.probe("KK4XYZ")
    sim.run(until=120)
    assert a.stats.probe_replies == 2
    a.connect("KK4XYZ")
    a.send(b"after the probe")
    a.disconnect()
    sim.run(until=400)
    assert sim.delivered(1) == b"after the probe"


def test_a_probe_to_nobody_reports_no_answer(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=22)
    a.probe("N0BODY")
    with pytest.raises(RuntimeError):
        a.probe("KK4XYZ")  # one at a time
    sim.run(until=60)
    assert "probe:N0BODY: no answer" in sim.events(0)
    assert not any(e.startswith("probed") for e in sim.events(1))
    assert a.state is State.IDLE
    # and once it is answered or timed out, another may go
    a.probe("KK4XYZ")
    sim.run(until=120)
    assert a.stats.probe_replies == 1


def test_a_probe_is_not_answered_during_a_session_or_in_another_bandwidth(
    timing: PhyTiming,
) -> None:
    from aether_model.link.frames import DataHeader, encode_data
    from aether_model.link.phy import Container
    from aether_model.link.sim import SimFrame

    def probe_from(call: str, to: str, caps: int = 0) -> SimFrame:
        body = ProbeBody(call, to, None, caps=caps).encode()
        payload = encode_data(DataHeader(DataKind.PROBE, 0, 0), body, timing.capacity(0))
        return SimFrame(Container.DATA, 0, 0, 12.0, 0.0, 1.0, payload, 0.0)

    a, b = _pair(timing)
    # a session up: a third station's probe is left alone
    sim = TwoStationSim(a, b, snr_db=15.0, seed=23)
    a.connect("KK4XYZ")
    sim.run(until=40)
    assert b.connected
    before = len(b.actions)
    b.on_frame(probe_from("N0CALL", "KK4XYZ"), b.now)
    assert b.stats.probes_answered == 0
    assert len(b.actions) == before
    # idle, but the probe claims another bandwidth: ignored, and said so
    a.disconnect()
    sim.run(until=300)
    assert b.state is State.IDLE
    b.on_frame(probe_from("N0CALL", "KK4XYZ", caps=with_bandwidth(0, 500)), b.now)
    assert b.stats.probes_answered == 0
    assert any(e.name == "ignored" and "N0CALL" in e.detail for e in b.actions)
    # and one for somebody else is nobody's business
    b.on_frame(probe_from("N0CALL", "W1AW"), b.now)
    assert b.stats.probes_answered == 0
    # while one addressed to it, in its bandwidth, is answered
    b.on_frame(probe_from("N0CALL", "KK4XYZ"), b.now)
    assert b.stats.probes_answered == 1
    assert any(isinstance(x, Transmit) for x in b.actions)
    # a station in a session may not probe; one that is probing may not either
    with pytest.raises(RuntimeError):
        a.connect("KK4XYZ")
        a.probe("KK4XYZ")


def test_a_call_stating_another_bandwidth_is_not_answered(timing: PhyTiming) -> None:
    # the bandwidth bits are a statement of the waveform the frame was sent in; a station
    # set up for 2 300 Hz that is called by a frame claiming 500 Hz leaves it alone
    assert bandwidth_code(with_bandwidth(0, 500)) == 1
    assert bandwidth_code(with_bandwidth(CAP_COMPRESSION, 2300)) == 0
    assert with_bandwidth(CAP_COMPRESSION, 500) == CAP_COMPRESSION | 0x02
    a = LinkEngine("W4ODA", timing, LinkConfig(capabilities=with_bandwidth(0, 500)), seed=1)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(capabilities=with_bandwidth(0, 2300)), seed=2)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=6)
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert any(e.startswith("ignored:W4ODA calls in another bandwidth") for e in sim.events(1))
    assert "disconnected:no answer" in sim.events(0)
    # and two stations that agree connect as before
    a, b = _pair(timing, LinkConfig(capabilities=with_bandwidth(0, 500)))
    sim = TwoStationSim(a, b, snr_db=15.0, seed=6)
    a.connect("KK4XYZ")
    a.send(b"at five hundred hertz")
    sim.run(until=40)
    assert a.connected and b.connected
    assert bandwidth_code(b.peer_capabilities) == 1
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == b"at five hundred hertz"


def test_a_call_to_somebody_else_is_not_answered(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    b.set_callsigns(["KK4XYZ"])
    sim = TwoStationSim(a, b, snr_db=15.0, seed=6)
    a.connect("KK4ABC")
    sim.run(until=200)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert "disconnected:no answer" in sim.events(0)
    assert not any(e.startswith("connected") for e in sim.events(1))


def test_a_station_calls_as_whichever_of_its_callsigns_the_host_named(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    a.set_callsigns(["W4ODA", "W4ODA-1"])
    sim = TwoStationSim(a, b, snr_db=15.0, seed=8)
    a.connect("KK4XYZ", as_call="w4oda-1")
    a.disconnect()
    sim.run(until=200)
    assert "connected:W4ODA-1 (irs)" in sim.events(1)
    with pytest.raises(ValueError):
        a.connect("KK4XYZ", as_call="N0CALL")
    # without a choice, the first callsign is the station's name
    a.connect("KK4XYZ")
    assert a.my_call == "W4ODA"


def test_callsigns_cannot_change_under_a_session(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=9)
    a.connect("KK4XYZ")
    sim.run(until=30)
    assert a.connected
    with pytest.raises(RuntimeError):
        a.set_callsigns(["W4ODA-2"])
    assert a.my_call == "W4ODA"
    idle = LinkEngine("N0CALL", timing, seed=3)
    with pytest.raises(ValueError):
        idle.set_callsigns([])
    with pytest.raises(ValueError):
        idle.set_callsigns(["TOOLONGCALL"])
    assert idle.callsigns == ["N0CALL"]


def test_peer_disconnect_is_observed(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=4)
    a.connect("KK4XYZ")
    a.send(b"short message")
    a.disconnect()
    sim.run(until=200)
    assert any("disconnected" in e for e in sim.events(1))
    assert b.state is State.IDLE


def test_transfer_survives_a_slow_snr_ramp(timing: PhyTiming) -> None:
    """P2-2: the channel fades from +18 dB to +2 dB and back over the transfer. The rate
    controller must follow it down and up without losing the session or the data."""
    a, b = _pair(timing)

    def schedule(t: float) -> float:
        """Triangular fade, 60 s period: +18 dB down to +2 and back."""
        phase = t % 60.0
        return 18.0 - 0.533 * phase if phase < 30.0 else 2.0 + 0.533 * (phase - 30.0)

    sim = TwoStationSim(a, b, snr_db=18.0, seed=13, snr_schedule=schedule)
    msg = bytes((i * 17) % 256 for i in range(12000))
    track = _track_burst_modes(a)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=3000)
    assert sim.delivered(1) == msg
    assert len(set(track)) >= 4, track  # it really did move around the mode table
    assert max(track) >= 8, track  # and exploited the good half of the fade
    steps = [m for i, m in enumerate(track) if i == 0 or m != track[i - 1]]  # drop plateaus
    deltas = [b_ - a_ for a_, b_ in pairwise(steps)]
    reversals = sum(1 for x, y in pairwise(deltas) if x * y < 0)
    assert reversals >= 2, track  # followed the channel down and back up


def test_rate_control_beats_a_fixed_conservative_mode(timing: PhyTiming) -> None:
    """Adaptation has to pay for itself: on a good channel the controller must finish a
    transfer far sooner than a link pinned to the most robust mode."""
    msg = bytes(6000)
    times = {}
    for name, cfg in (("adaptive", LinkConfig()), ("pinned", LinkConfig(max_mode=0))):
        a = LinkEngine("W4ODA", timing, cfg, seed=1)
        b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
        sim = TwoStationSim(a, b, snr_db=16.0, seed=21)
        a.connect("KK4XYZ")
        a.send(msg)
        a.disconnect()
        times[name] = sim.run(until=6000)
        assert sim.delivered(1) == msg
    assert times["adaptive"] < 0.25 * times["pinned"], times


# ── P2-2a / P2-2b ─────────────────────────────────────────────────────


def _latent_transfer(prop_s: float, tx_latency_s: float) -> tuple[bool, int]:
    """A transfer with real transport latency, the sender told (or not) about its own."""
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    t = PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        data_capacity=caps,
        tx_latency_s=tx_latency_s,
    )
    a, b = LinkEngine("W4ODA", t, None, seed=1), LinkEngine("KK4XYZ", t, None, seed=2)
    sim = TwoStationSim(a, b, snr_db=20.0, seed=3, prop_s=prop_s)
    msg = b"The quick brown fox jumps over the lazy dog. " * 10
    a.connect("KK4XYZ")
    sim.run(until=30)
    a.send(msg)  # once connected, as a host does
    sim.run(until=400)
    return sim.delivered(1) == msg, a.stats.ack_timeouts


def test_a_sender_that_knows_its_own_latency_waits_long_enough() -> None:
    """Two real daemons over a socket stalled on their first session: each burst left the
    sound card a quarter of a second after the engine handed it over, so every reply was
    waited for from too early a moment and the acknowledgement of the first poll arrived
    just after the engine had given up on it. With ``tx_latency_s`` set to what the daemon
    knows about itself, half a second of one-way latency costs no timeouts at all; without
    it, the same link limps on retries."""
    delivered, timeouts = _latent_transfer(prop_s=0.45, tx_latency_s=0.4)
    assert delivered and timeouts == 0
    delivered_blind, timeouts_blind = _latent_transfer(prop_s=0.45, tx_latency_s=0.0)
    assert delivered_blind and timeouts_blind > 10


def test_an_ack_that_arrives_during_a_repoll_is_acted_on_when_the_poll_ends() -> None:
    """The other half of the same stall: the late acknowledgement was accepted while the
    re-poll was on the air, and the burst it should have started was dropped because the
    transmitter was busy — and nothing tried again. ``on_tx_done`` now does."""
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    t = PhyTiming(
        data_frame_s=LONG.duration_s, control_frame_s=SHORT.duration_s, data_capacity=caps
    )
    a, b = LinkEngine("W4ODA", t, None, seed=1), LinkEngine("KK4XYZ", t, None, seed=2)
    sim = TwoStationSim(a, b, snr_db=20.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=30)
    assert a.state is State.CONNECTED
    # queue data while the transmitter is busy with the post-connect poll and no reply is
    # awaited: the only thing that can start the burst is the end of that transmission
    a._waiting_for = None  # the race the field showed, reproduced by hand
    a._tx_busy_until = a.now + 1.0
    a.send(b"queued while keyed")
    assert not any(isinstance(x, Transmit) for x in a.drain())
    a.on_tx_done(a.now + 1.0)
    assert any(isinstance(x, Transmit) for x in a.drain()), "the queued data never went out"


def test_a_burst_held_back_by_a_busy_channel_moves_the_timers_with_it(timing: PhyTiming) -> None:
    """On the air, connect requests went out in pairs inside one keying: the busy detector
    held the first back, the retry timer — set as if it had gone out — fired meanwhile,
    and both left together when the channel cleared. A physical layer that holds a burst
    now says so, and the engine's deadlines move by the same amount."""
    a = LinkEngine("W4ODA", timing, seed=1)
    a.connect("KK4XYZ")
    assert sum(isinstance(x, Transmit) for x in a.drain()) == 1
    # held for eight seconds, reported a block at a time as the station does
    t = 0.0
    while t < 8.0:
        t += 0.02
        a.on_tx_delayed(0.02)
        a.tick(t)
    assert not any(isinstance(x, Transmit) for x in a.drain()), "a retry was queued while held"
    assert a.state is State.CONNECTING and a._connect_tries == 1
    # once it has gone, the retry comes when it would have without the delay: after the
    # burst, the reply wait and the backoff, all of it measured from the real departure
    a.on_tx_done(t + timing.tx_latency_s + timing.data_frame_s)
    retry_at = None
    while t < 30.0:
        t += 0.02
        a.tick(t)
        if any(isinstance(x, Transmit) for x in a.drain()):
            retry_at = t
            break
    assert retry_at is not None, "no retry ever came"
    assert retry_at - 8.0 > timing.data_frame_s * 2, (
        f"the retry came too soon after departure: {retry_at - 8.0:.2f} s"
    )


def _timing(*, start_of_frame: bool) -> PhyTiming:
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    return PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        preamble_detect_s=4 * LONG.waveform.symbol_period_s if start_of_frame else None,
        data_capacity=caps,
    )


def test_start_of_frame_signal_raises_throughput() -> None:
    """P2-2a: told when a frame *starts*, the receiver no longer has to wait a whole frame of
    silence to know a burst has ended, which is about a quarter of the air time."""
    msg = bytes(16000)
    times = {}
    for sof in (False, True):
        timing = _timing(start_of_frame=sof)
        a = LinkEngine("W4ODA", timing, seed=1)
        b = LinkEngine("KK4XYZ", timing, seed=2)
        sim = TwoStationSim(a, b, snr_db=14.0, seed=5)
        a.connect("KK4XYZ")
        a.send(msg)
        a.disconnect()
        times[sof] = sim.run(until=6000)
        assert sim.delivered(1) == msg
    assert times[True] < 0.92 * times[False], times


def test_margin_learns_the_channel_penalty_in_one_step() -> None:
    """P2-2b: a failure is a measurement. Mode 4 dying at +12 dB says this channel wants
    ~11 dB more than the AWGN table predicts, so the margin jumps toward that instead of
    creeping up in fixed steps and overshooting the mode for several bursts first."""
    rc = RateController()
    start = rc.margin_db
    rc.observe(12.0, ok=0, failed=4, mode=4)
    assert rc.margin_db - start > rc.up_step_db  # targeted, not a fixed nudge
    assert rc.margin_db - start <= rc.max_jump_db  # but a single fade cannot strand the link


def test_learned_margin_is_sticky_then_decays() -> None:
    rc = RateController()
    rc.observe(12.0, ok=0, failed=4, mode=4)
    learned = rc.margin_db
    for _ in range(rc.decay_every - 1):
        rc.observe(12.0, ok=6, failed=0, mode=4)
    assert rc.margin_db == learned  # a couple of clean bursts do not give it up
    rc.observe(12.0, ok=6, failed=0, mode=4)
    assert rc.margin_db < learned  # but it is not permanent either


def test_a_learned_margin_is_given_back_faster_the_longer_bursts_stay_clean() -> None:
    """P9-2 (the faster climb): after the sticky bursts, every further clean burst gives back
    more of a learned margin than the one before, so a fade that has passed — or a collision
    that never was one — costs a handful of bursts, not twenty. Found on the VarAC bench,
    where one lost burst held a transfer a mode and a half below what the SNR carried."""
    rc = RateController()
    rc.observe(12.0, ok=0, failed=4, mode=4)
    learned = rc.margin_db
    steps: list[float] = []
    before = learned
    for _ in range(8):
        rc.observe(12.0, ok=6, failed=0, mode=4)
        steps.append(before - rc.margin_db)
        before = rc.margin_db
    assert steps[: rc.decay_every - 1] == [0.0] * (rc.decay_every - 1)  # sticky first
    taken = [s for s in steps if s > 0]
    assert taken[0] == rc.down_step_db
    # never slower, until the floor cuts the last step short
    assert all(b >= a for a, b in pairwise(taken[:-1]))
    assert taken[1] > taken[0]  # and faster from the second step on
    assert learned - rc.margin_db >= 2.0, steps  # most of it back within eight bursts
    assert max(taken) <= rc.max_down_step_db


def test_margin_decays_freely_before_anything_is_learned() -> None:
    """Stickiness protects a *learned* penalty; on a link that has never failed there is
    nothing to protect, so a clean channel must still reach its mode quickly."""
    rc = RateController()
    for _ in range(3):
        rc.observe(10.0, ok=6, failed=0, mode=0)
    assert rc.margin_db < RateController().margin_db - 2 * rc.down_step_db


# ── capability negotiation (P3-6) ─────────────────────────────────────


def test_capabilities_are_exchanged_in_the_connect_handshake(timing: PhyTiming) -> None:
    """The handshake carries a capability byte both ways, which is how compression is agreed.

    The link layer does not interpret the bits — that is the caller's business — but it has
    to carry them, and each station has to be able to read what the other offered."""
    # bits 1–2 are the bandwidth and have to agree (see the test above); the rest are
    # whatever the caller means by them
    a = LinkEngine("W4ODA", timing, LinkConfig(capabilities=0b01001), seed=1)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(capabilities=0b10001), seed=2)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert a.connected and b.connected
    assert a.peer_capabilities == 0b10001
    assert b.peer_capabilities == 0b01001
    # what both offered is the intersection; nothing is negotiated on one side's word alone
    assert a.peer_capabilities & a.cfg.capabilities == 0b00001


def test_capabilities_are_forgotten_when_a_session_ends(timing: PhyTiming) -> None:
    """A station that has not said what it can do must be assumed to do nothing: carrying a
    previous peer's capabilities into the next session would be exactly the wrong default."""
    a, b = _pair(timing, LinkConfig(capabilities=0b111))
    sim = TwoStationSim(a, b, snr_db=15.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert b.peer_capabilities == 0b111
    b.abort()
    assert b.peer_capabilities == 0
