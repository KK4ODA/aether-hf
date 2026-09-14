"""Link layer: frame formats, ARQ engine and session FSM over the lossy-pipe simulator
(roadmap P2-1). The bit-exact-over-the-real-PHY check lives in ``test_link_harness.py``."""

from __future__ import annotations

from itertools import pairwise

import pytest

from aether_model.frame.modes import LONG, MODES, SHORT
from aether_model.link.engine import LinkConfig, LinkEngine, Role, State, Transmit
from aether_model.link.frames import (
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    decode_data,
    encode_data,
    pack_callsign,
    unpack_callsign,
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
    a = LinkEngine("W4ODA", timing, LinkConfig(capabilities=0b101), seed=1)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(capabilities=0b011), seed=2)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=3)
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert a.connected and b.connected
    assert a.peer_capabilities == 0b011
    assert b.peer_capabilities == 0b101
    # what both offered is the intersection; nothing is negotiated on one side's word alone
    assert a.peer_capabilities & a.cfg.capabilities == 0b001


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
