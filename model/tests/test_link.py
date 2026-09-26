"""Link layer: frame formats, ARQ engine and session FSM over the lossy-pipe simulator
(roadmap P2-1). The bit-exact-over-the-real-PHY check lives in ``test_link_harness.py``."""

from __future__ import annotations

import math
from collections.abc import Callable
from itertools import pairwise

import pytest

from aether_model.frame.modes import LONG, MODES, SHORT, WIDE
from aether_model.link.engine import LinkConfig, LinkEngine, ProbeResult, Role, State, Transmit
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
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import AWGN_THRESHOLD_DB, RateController, usable_modes
from aether_model.link.sim import TwoStationSim


@pytest.fixture(scope="module")
def timing() -> PhyTiming:
    """The wide air's ladder — the tone floor's six rungs, then the OFDM modes — as the
    model's PHY reports it, but without preamble reports (the engine's safe fallback)."""
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import WIDE_2300

    return phy_timing(WIDE_2300, start_of_frame=False)


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


def _first_acceptance_unheard() -> Callable[[int, Container, float], bool]:
    """A pipe's ``unheard``: the caller (station 0) never detects the first DATA frame sent to
    it — the called station's first acceptance."""
    lost: list[float] = []

    def unheard(rx: int, container: Container, t0: float) -> bool:
        if rx != 0 or container is not Container.DATA:
            return False
        if not lost:
            lost.append(t0)
        return t0 == lost[0]

    return unheard


def test_a_repeated_acceptance_says_how_its_request_arrived(timing: PhyTiming) -> None:
    """A called station whose acceptance was lost is connected, and answers the caller's next
    try with the acceptance again — carrying the SNR that try arrived at, as the first carried
    the first's. A caller that hears only the repeat starts its first burst where that
    measurement puts it (P9-2). The repeat said "not measured", and such a session started on
    the ladder's first rung, the tone floor, whatever the path: on the link bench's fading
    pipe, a 2 kB session at +24 dB whose first acceptance was lost took 73 s, against 21 s
    for one whose acceptance arrived."""
    a, b = _pair(timing)
    answers: list[float | None] = []
    transmit = b._transmit

    def record(frames: list[TxFrame]) -> None:
        for f in frames:
            if f.container is Container.DATA:
                header, body = decode_data(f.payload)
                if header.kind is DataKind.CONNECT_ACK:
                    answers.append(ConnectBody.decode(body).snr_db)
        transmit(frames)

    b._transmit = record  # type: ignore[method-assign]
    first: list[int] = []
    send_burst = a._send_burst

    def wrapped() -> None:
        first.append(min(a._recommended, a.cfg.max_mode))
        send_burst()

    a._send_burst = wrapped  # type: ignore[method-assign]
    # the first try arrives in a fade, the second on a strong path
    sim = TwoStationSim(
        a,
        b,
        seed=31,
        snr_schedule=lambda t: 3.0 if t < 10.0 else 18.0,
        unheard=_first_acceptance_unheard(),
    )
    a.connect("KK4XYZ")
    a.send(bytes(600))
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == bytes(600)
    assert a._connect_tries == 2
    assert answers == [3.0, 18.0], answers
    fresh = LinkEngine("N0CALL", timing, None, seed=3).rate
    assert fresh.first_mode(18.0) > fresh.first_mode(3.0)
    assert first[0] == fresh.first_mode(18.0), first


def test_a_request_heard_again_is_answered_and_nothing_else(timing: PhyTiming) -> None:
    """The acceptance again is the whole answer to a request heard again. The request had also
    armed an acknowledgement, which went out a burst's quiet after it — over the caller's first
    burst, which follows the acceptance at once. On a weak path the repeat and that
    acknowledgement are floor frames, and the first five-second frame of the burst was lost
    under them and sent again."""
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=31, unheard=_first_acceptance_unheard())
    a.connect("KK4XYZ")
    a.send(bytes(600))
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == bytes(600)
    assert a._connect_tries == 2
    # one burst, one acknowledgement, and nothing of the burst sent twice
    assert b.stats.acks_sent == 1, b.stats
    assert a.stats.frames_resent == 0, a.stats


def test_the_first_mode_keeps_a_step_in_hand() -> None:
    rc = RateController()
    # far below every mode but the slowest: the slowest
    assert rc.first_mode(-20.0) == usable_modes()[0]
    # steps below the fastest that fits with margin and hysteresis — within the family it is
    # in: a fit on the OFDM rungs does not start on the floor
    ordinary = next(i for i, m in enumerate(rc.modes) if m >= rc.floor_modes)
    for snr in (4.0, 9.0, 15.0, 20.0):
        modes = rc.modes
        fits = [
            m for m in modes if AWGN_THRESHOLD_DB[m] + rc.margin_db + rc.up_hysteresis_db <= snr
        ]
        top = modes.index(fits[-1])
        lowest = ordinary if top >= ordinary else 0
        assert rc.first_mode(snr) == modes[max(lowest, top - rc.first_mode_back)], snr
    # seeding places the controller there and takes the measurement, once
    rc.seed(15.0)
    assert rc.recommend() == rc.first_mode(15.0)
    assert rc.snr_db == 15.0
    rc.seed(2.0)
    assert rc.snr_db == 15.0  # a second seed changes nothing


def test_a_seed_from_the_tone_floor_is_a_lower_bound() -> None:
    """ADR-0016: calls start on the tone floor, whose SNR estimate reads low on a strong path
    (it saturates near +17 dB on AWGN and reads a few decibels on a dispersive path). A
    controller seeded from it seeds again from the first clean burst measured on an ordinary
    frame — upward, past the estimates' own spread, and once; a failed burst or a floor
    burst does not."""
    rc = RateController()
    ordinary = rc.modes[rc._first_ordinary()]
    rc.seed(4.0, lower_bound=True)
    assert rc.recommend() == rc.first_mode(4.0)
    rc.observe(20.0, ok=6, failed=0, mode=0)  # a floor burst: a lower bound again
    assert rc.snr_db == pytest.approx(0.7 * 4.0 + 0.3 * 20.0)
    rc.observe(20.0, ok=3, failed=3, mode=ordinary)  # a failure says nothing of the path
    assert rc.recommend() < rc.first_mode(20.0)
    rc.observe(20.0, ok=6, failed=0, mode=ordinary)
    assert rc.snr_db == 20.0
    assert rc.recommend() == rc.first_mode(20.0)
    rc.observe(10.0, ok=6, failed=0, mode=ordinary)  # once: then smoothed as ever
    assert rc.snr_db == pytest.approx(0.7 * 20.0 + 0.3 * 10.0)

    high = RateController()  # upward only
    high.seed(15.0, lower_bound=True)
    high.observe(10.0, ok=6, failed=0, mode=ordinary)
    assert high.snr_db == pytest.approx(0.7 * 15.0 + 0.3 * 10.0)
    near = RateController()  # within the estimates' own spread: smoothed, not replaced
    near.seed(10.0, lower_bound=True)
    near.observe(10.0 + near.reseed_margin_db, ok=6, failed=0, mode=ordinary)
    assert near.snr_db == pytest.approx(0.7 * 10.0 + 0.3 * (10.0 + near.reseed_margin_db))
    plain = RateController()  # an ordinary connect frame's measurement is not a lower bound
    plain.seed(4.0)
    plain.observe(20.0, ok=6, failed=0, mode=ordinary)
    assert plain.snr_db == pytest.approx(0.7 * 4.0 + 0.3 * 20.0)


def test_the_floor_boundary_is_crossed_by_what_the_rungs_are_worth() -> None:
    """ADR-0013 §4, ADR-0014: the floor's frames are five times as long as an OFDM frame. A
    session the SNR puts on the first OFDM rungs does not start on the floor; on an air that
    caps the first OFDM rung's margin the rung is held against the floor to its threshold
    plus the cap, however wide the learned margin, and one failed burst there does not leave
    for the floor — a second in a row does; the climb back asks for the cap and the
    hysteresis. The first OFDM rung is the first *usable* one: an OFDM mode the floor beats
    on rate and threshold both is on the ladder and never recommended."""
    rc = RateController()  # the wide ladder: rungs 0–5 are the tone floor
    assert rc.floor_modes == 6 and rc.modes[:6] == list(range(6))
    first = rc.modes[rc._first_ordinary()]
    second = rc.modes[rc._first_ordinary() + 1]
    fits_second = AWGN_THRESHOLD_DB[second] + rc.margin_db + rc.up_hysteresis_db
    assert rc.first_mode(fits_second) == first  # not two steps down, on the floor
    assert rc.first_mode(-12.0) < rc.floor_modes  # nothing above the floor fits: the floor

    capped = RateController(floor_margin_db=1.0)
    capped.seed(AWGN_THRESHOLD_DB[first] + 1.5)
    capped._index = capped.modes.index(first)
    capped.margin_db = 8.0  # a fading channel's learned margin
    snr = capped.snr_db
    capped.observe(snr, ok=0, failed=6, mode=first)
    assert capped.recommend() == first  # one lost burst on the capped rung: held
    capped.observe(snr, ok=0, failed=6, mode=first)
    assert capped.recommend() < first  # two in a row: the floor
    for _ in range(3):
        capped.observe(snr, ok=6, failed=0)
    assert capped.recommend() < first  # the cap and the hysteresis are not met at that SNR
    for _ in range(6):
        capped.observe(
            AWGN_THRESHOLD_DB[first] + 1.0 + capped.up_hysteresis_db + 0.5, ok=6, failed=0
        )
    assert capped.recommend() >= first  # they are here, whatever the learned margin

    # uncapped the learned margin decides, as between any two rungs
    plain = RateController(floor_margin_db=None)
    plain.seed(AWGN_THRESHOLD_DB[first] + 1.5)
    plain._index = plain.modes.index(first)
    plain.margin_db = 8.0
    plain.observe(plain.snr_db, ok=0, failed=6, mode=first)
    assert plain.recommend() < first


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
    # 8-PSK 2/3 is dominated by 16-QAM 1/2: more payload, lower threshold
    assert WIDE.rung_of(7) not in modes and WIDE.rung_of(8) in modes
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
    one mode at a time for half a minute. From the bottom of the ladder two things pace it
    and nothing else: the climb, :attr:`RateController.max_up_step` usable modes a burst,
    and the margin, which gives up :attr:`RateController.down_step_db` a clean burst before
    the first failure — a mode that fits only at the least margin waits for it. One burst
    more for the first measurement. (With the ladder of ADR-0013, sixteen rungs, the bound
    at 20 dB was the eight bursts this test once stated.)"""
    for snr in (0.0, 8.0, 14.0, 20.0):
        rc = RateController()
        climb = -(-rc.modes.index(_settle(RateController(), snr)) // rc.max_up_step)
        decay = math.ceil((rc.margin_db - rc.min_margin_db) / rc.down_step_db)
        final = _settle(RateController(), snr)
        track = [rc.observe(snr, ok=6, failed=0) or rc.recommend() for _ in range(30)]
        assert track.index(final) + 1 <= max(climb, decay) + 1, (snr, track)


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
    """Mode pinned to BPSK ½ (threshold ≈ −1.8 dB) and driven at −3 dB: the transfer only
    completes because retransmissions are soft-combined."""
    rung = WIDE.rung_of(2)
    cfg = LinkConfig(initial_mode=rung, max_mode=rung, max_retries=60)
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


def test_a_station_that_only_received_learns_how_it_was_heard(timing: PhyTiming) -> None:
    """ADR-0021: ND1J called and sent, KK4ODA-1 only acknowledged, and its history could not
    say how ND1J heard it — a station says that in its acknowledgements, and the one that only
    receives is never acknowledged. Every control frame now carries how its sender hears the
    other station, and the disconnect brings it to the station that only received; kept, once
    the session has ended, for the account written after it."""
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=9)
    a.connect("KK4XYZ")
    a.send(bytes([0x5A]) * 400)
    a.disconnect()
    sim.run(until=300)
    assert b.state is State.IDLE
    assert b.peer_snr_db is None, "the session's own report goes with it"
    assert b.ended_peer_snr_db is not None
    assert abs(b.ended_peer_snr_db - 15.0) < 3.0


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
    assert a.last_probe == ProbeResult("KK4XYZ", 15.0, 15.0) and not a.probing
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
    assert a.probing and a.last_probe is None
    sim.run(until=60)
    assert "probe:N0BODY: no answer" in sim.events(0)
    assert not a.probing and a.last_probe is None
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


def test_a_datagram_numbered_like_the_session_is_not_session_data(timing: PhyTiming) -> None:
    # a datagram's session byte holds its own number (ADR-0019); one that happens to equal
    # the session's must not enter the session's stream
    from aether_model.link.datagram import fragments
    from aether_model.link.phy import Container
    from aether_model.link.sim import SimFrame

    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=29)
    a.connect("KK4XYZ")
    sim.run(until=40)
    assert b.connected
    received = b.stats.frames_received
    for payload in fragments(bytes(60), b.session, timing.capacity(0)):
        b.on_frame(SimFrame(Container.DATA, 0, 0, 12.0, 0.0, 1.0, payload, 0.0, floor=True), b.now)
    assert b.stats.frames_received == received
    message = bytes(range(200))
    a.send(message)
    sim.run(until=200)
    assert sim.delivered(1) == message


# ── calls, probes and their answers on the tone floor (ADR-0016) ─────────


def _modes_sent(engine: LinkEngine) -> list[int]:
    """Record the mode of every frame ``engine`` puts on the air from now on."""
    sent: list[int] = []
    original = engine._transmit

    def record(frames: list[TxFrame]) -> None:
        sent.extend(f.mode for f in frames)
        original(frames)

    engine._transmit = record  # type: ignore[method-assign]
    return sent


def test_a_strong_path_called_on_the_floor_climbs_from_its_first_ordinary_burst(
    timing: PhyTiming,
) -> None:
    """ADR-0016: the tone floor's SNR estimate reads a few decibels on a strong dispersive
    path whatever the SNR. The session's first burst goes out where that reading puts it; the
    called station's controller, seeded from it, starts again from the first clean burst it
    measures on an ordinary frame — so the second burst goes out where an ordinary connect
    frame would have started the session, not two rungs a burst up from the floor's reading."""
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=20.0, seed=41, floor_reading_cap_db=4.0)
    bursts: list[int] = []
    original = a._send_burst

    def wrapped() -> None:
        bursts.append(min(a._recommended, a.cfg.max_mode))
        original()

    a._send_burst = wrapped  # type: ignore[method-assign]
    fresh = LinkEngine("N0CALL", timing, None, seed=3).rate
    a.connect("KK4XYZ")
    a.send(bytes(20000))
    a.disconnect()
    sim.run(until=900)
    assert sim.delivered(1) == bytes(20000)
    assert bursts[0] == fresh.first_mode(4.0), bursts
    assert bursts[1] == fresh.first_mode(20.0), bursts
    assert bursts[1] > bursts[0] + fresh.max_up_step, bursts


def test_a_burst_fits_the_transmitters_key_time(timing: PhyTiming) -> None:
    """ADR-0017: six tone frames are 32 s, and the daemon's 30 s key watchdog cut the last
    one of every full tone burst on the air. The receiver acquired the cut frame and could
    not decode it, which its rate controller counts as the channel's loss: the link stepped
    down the tone floor, where every burst was full and cut again (ND1J, 2026-09-25). A
    burst carries no more frames than fit in :attr:`LinkConfig.max_burst_s`."""
    tone = timing.data_frame_s_for(0)
    ofdm = timing.data_frame_s_for(timing.floor_modes)
    assert 6 * tone > 30.0 > 5 * tone, "the case this exists for"
    capped = LinkEngine("W4ODA", timing, LinkConfig(max_burst_s=29.0))
    assert capped._burst_capacity(tone) == 5
    assert capped._burst_capacity(ofdm) == 6, "a second a frame: six fit either way"
    assert LinkEngine("W4ODA", timing, LinkConfig())._burst_capacity(tone) == 6

    def session(max_burst_s: float | None) -> tuple[float, int, list[int]]:
        cfg = LinkConfig(max_burst_s=max_burst_s)
        a, b = _pair(timing, cfg)
        sizes: list[int] = []
        original = a._transmit

        def record(frames: list[TxFrame]) -> None:
            if frames and frames[0].container is Container.DATA and timing.is_floor(frames[0].mode):
                sizes.append(len(frames))
            original(frames)

        a._transmit = record  # type: ignore[method-assign]
        # −4 dB: the fast tones' range, and a transmitter that unkeys at 30 s as the
        # daemon's watchdog does
        sim = TwoStationSim(a, b, snr_db=-4.0, seed=3, key_limit_s=30.0)
        a.connect("KK4XYZ")
        a.send(bytes(2000))
        a.disconnect()
        took = sim.run(until=4000)
        assert sim.delivered(1) == bytes(2000)
        return took, a.stats.frames_resent, sizes

    cut_took, cut_resent, cut_sizes = session(None)
    fit_took, fit_resent, fit_sizes = session(30.0 - 1.0)
    assert max(cut_sizes) == 6
    assert max(fit_sizes) <= 5
    assert fit_resent == 0, "a channel that carries the rung resends nothing"
    assert cut_resent >= 10, "every full burst lost a frame, and the link fell with it"
    assert fit_took < 0.5 * cut_took, (fit_took, cut_took)


def test_what_has_reached_the_other_station_is_counted_in_order(timing: PhyTiming) -> None:
    """A message is delivered when the other station's application has it: every byte of it
    and every byte before it acknowledged. :attr:`tx_undelivered_bytes` counts what is still
    short of that — the queue, and each frame from the lowest unacknowledged one up, a frame
    acknowledged past a hole included — where :attr:`tx_pending_bytes` counts only what is
    unacknowledged. The panel's check mark on a sent message comes from it."""
    a, b = _pair(timing)
    readings: list[tuple[int, int, int]] = []
    sim = TwoStationSim(a, b, snr_db=0.0, seed=3)  # a lossy path: frames acked past holes
    original = a.on_frame

    def observe(frame: SoftFrame, now: float) -> None:
        original(frame, now)
        readings.append((a.tx_pending_bytes, a.tx_undelivered_bytes, len(sim.delivered(1))))

    a.on_frame = observe  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    a.send(bytes(3000))
    assert a.tx_undelivered_bytes == 3000
    sim.run(until=3000)
    assert sim.delivered(1) == bytes(3000)
    assert a.tx_undelivered_bytes == a.tx_pending_bytes == 0
    arrived = [3000 - undelivered for _, undelivered, _ in readings]
    # what the sender counts as arrived only grows, never runs ahead of what the other
    # station's application actually has, and never exceeds what is acknowledged
    assert arrived == sorted(arrived)
    assert all(3000 - u <= got for _, u, got in readings), readings
    assert all(u >= p for p, u, _ in readings)
    assert any(u > p for p, u, _ in readings), "a frame acknowledged past a hole waits for it"


def test_a_regulatory_ceiling_holds_every_frame_the_station_sends(timing: PhyTiming) -> None:
    """ADR-0018: the rules outrank link adaptation. A station whose ceiling admits only the
    tone floor — an automatically controlled station answering outside the §97.221(b)
    segments — sends every data frame at a floor rung and every control frame, answer and
    probe answer on the floor, however good the path and whatever family the other station
    uses; the other station, which has no ceiling, climbs as it always did."""
    floor_top = timing.floor_modes - 1
    ceiling = 1
    assert timing.is_floor(ceiling) and ceiling < floor_top, "a ceiling inside the floor"
    a, b = _pair(timing)
    b.set_ceiling(ceiling)
    sent: dict[str, list[TxFrame]] = {"a": [], "b": []}
    for name, engine in (("a", a), ("b", b)):
        original = engine._transmit

        def record(frames: list[TxFrame], name: str = name, original=original) -> None:  # type: ignore[no-untyped-def]
            sent[name].extend(frames)
            original(frames)

        engine._transmit = record  # type: ignore[method-assign]
    sim = TwoStationSim(a, b, snr_db=24.0, seed=31)
    a.connect("KK4XYZ")
    sim.run(until=120)
    assert a.connected and b.connected
    a.send(bytes(3000))
    b.send(bytes(3000))
    sim.run(until=3000)
    assert sim.delivered(1) == bytes(3000) and sim.delivered(0) == bytes(3000)
    b_data = [f.mode for f in sent["b"] if f.container is Container.DATA]
    b_control = [f.floor for f in sent["b"] if f.container is Container.CONTROL]
    assert b_data and max(b_data) <= ceiling, b_data
    assert b_control and all(b_control), "every control frame on the floor"
    a_data = [f.mode for f in sent["a"] if f.container is Container.DATA]
    assert max(a_data) > floor_top, "the path was good: the other station climbed"

    # a ceiling set mid-session holds from the next frame on
    a.set_ceiling(ceiling)
    before = len(sent["a"])
    a.send(bytes(1500))
    sim.run(until=6000)
    later = [f for f in sent["a"][before:]]
    assert later
    assert all(f.mode <= ceiling for f in later if f.container is Container.DATA)
    assert all(f.floor for f in later if f.container is Container.CONTROL)


def test_a_call_starts_on_the_tone_floor_and_alternates(timing: PhyTiming) -> None:
    """A call is made before anything is known of the path, so it goes where the path most
    likely carries it: the tone floor, 14 dB below the ordinary family's control rung, on its
    first try and every other one after, the ordinary family between (ADR-0016). ADR-0009 had
    the first two tries ordinary, and a weak path's first two were wasted."""
    a, b = _pair(timing, LinkConfig(connect_retries=4))
    sent = _modes_sent(a)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=24)
    a.connect("N0BODY")
    sim.run(until=900)
    assert "disconnected:no answer" in sim.events(0)
    assert (a._robust_mode(True), a._robust_mode(False)) == (0, WIDE.control_rung)
    assert sent == [0, WIDE.control_rung, 0, WIDE.control_rung], sent


def test_a_call_answered_on_the_floor_connects_on_its_first_try(timing: PhyTiming) -> None:
    """At −14 dB only the floor carries a frame: the first try reaches the station, the
    acceptance comes back on the floor, and the session needs no second try."""
    a, b = _pair(timing)
    sent = _modes_sent(a)
    sim = TwoStationSim(a, b, snr_db=-14.0, seed=25)
    a.connect("KK4XYZ")
    sim.run(until=120)
    assert a.connected and b.connected
    assert sent[0] == 0 and a._connect_tries == 1, sent


def test_a_probe_goes_out_on_the_floor_and_is_answered_in_its_family(
    timing: PhyTiming,
) -> None:
    """A probe exists to measure a weak path, so it goes out on the tone floor, and it is
    answered in the family it arrived in, as an acceptance is — a station of an earlier
    version probes in the ordinary family and hears its answer there (ADR-0016)."""
    from aether_model.link.frames import DataHeader, encode_data
    from aether_model.link.phy import Container
    from aether_model.link.sim import SimFrame

    a, b = _pair(timing)
    sent_a, sent_b = _modes_sent(a), _modes_sent(b)
    sim = TwoStationSim(a, b, snr_db=-14.0, seed=26)
    a.probe("KK4XYZ")
    sim.run(until=120)
    assert a.stats.probe_replies == 1, sim.events(0)
    assert sent_a == [0] and sent_b == [0], (sent_a, sent_b)
    # an ordinary-family probe: answered in the ordinary family
    robust = WIDE.control_rung
    body = ProbeBody("N0CALL", "KK4XYZ", None).encode()
    payload = encode_data(DataHeader(DataKind.PROBE, 0, 0), body, timing.capacity(robust))
    b.on_frame(SimFrame(Container.DATA, robust, 0, 12.0, 0.0, 1.0, payload, 0.0), b.now)
    assert b.stats.probes_answered == 2
    assert sent_b == [0, robust], sent_b


def test_the_iss_waits_for_an_answer_in_the_family_the_irs_last_heard(
    timing: PhyTiming,
) -> None:
    """The IRS answers in the family it last heard from the ISS: a first OFDM burst after a
    call on the floor that it decodes none of is answered on the floor. The ISS waits for
    that floor acknowledgement, not only for the OFDM one its burst would bring — waiting for
    its own family's gave up a second into it, and the recommendation it carried was lost
    (ADR-0016)."""

    def wait_after_burst(peer_floor: bool) -> float:
        a, _ = _pair(timing)
        a.state, a.role, a.session = State.CONNECTED, Role.ISS, 7
        a._peer_floor = peer_floor
        a._recommended = WIDE.rung_of(1)  # BPSK 1/3, an OFDM rung
        a.send(bytes(100))
        assert a._waiting_for == "ack"
        return a._deadlines["wait"] - a._tx_busy_until

    ordinary, after_floor = wait_after_burst(False), wait_after_burst(True)
    longer = timing.control_frame_s_for(True) - timing.control_frame_s_for(False)
    assert after_floor >= ordinary + longer - 1e-9, (ordinary, after_floor)


def test_a_caller_does_not_call_over_a_frame_it_hears_arriving(timing: PhyTiming) -> None:
    """A called station whose acceptance was lost is connected, and answers the undecodable
    preamble of the caller's next try with an acknowledgement on the floor; the caller's try
    after that ran into it, again and again, until the caller gave up — two of thirty calls
    at −14 dB on ITU Moderate. A caller waits out a frame it hears arriving, and a prober its
    answer (ADR-0016)."""
    floor_control = timing.control_frame_s_for(True)
    a, _ = _pair(timing)
    a.connect("KK4XYZ")
    due = a._deadlines["connect"]
    a.on_preamble(due - 1.0, due - 0.5, floor_control)
    assert a._deadlines["connect"] >= due - 1.0 + floor_control + timing.turnaround_s
    # a frame that ends before the next try was due changes nothing
    a2, _ = _pair(timing)
    a2.connect("KK4XYZ")
    due2 = a2._deadlines["connect"]
    a2.on_preamble(due2 - 10.0, due2 - 9.5, 1.0)
    assert a2._deadlines["connect"] == due2
    # a probe's answer heard arriving is waited for
    p, _ = _pair(timing)
    p.probe("KK4XYZ")
    due3 = p._deadlines["probe"]
    p.on_preamble(due3 - 1.0, due3 - 0.5, floor_control)
    assert p._deadlines["probe"] >= due3 - 1.0 + floor_control


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


def test_a_station_moved_to_another_air_answers_calls_in_it(timing: PhyTiming) -> None:
    # ADR-0026: the daemon moves the engine between sessions — to the bandwidth a host
    # program asked for, or to the narrower one a call to it came in — and the engine keeps
    # its callsigns, its counters and its session numbering across the move
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import NARROW_500

    narrow = phy_timing(NARROW_500, start_of_frame=False)
    assert narrow.mode_threshold_db is not None and timing.mode_threshold_db is not None
    a = LinkEngine("W4ODA", narrow, LinkConfig(capabilities=with_bandwidth(0, 500)), seed=1)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(capabilities=with_bandwidth(0, 2300)), seed=2)
    b.set_callsigns(["KK4XYZ", "KK4XYZ-1"])
    b.stats.probes_sent = 3
    top = len(narrow.mode_threshold_db) - 1
    b.set_air(narrow, with_bandwidth(0, 500), max_mode=top)
    assert b.timing is narrow and bandwidth_code(b.cfg.capabilities) == 1
    assert b.cfg.max_mode == top
    assert b.rate.thresholds == dict(narrow.mode_threshold_db)
    assert b.callsigns == ["KK4XYZ", "KK4XYZ-1"] and b.stats.probes_sent == 3
    sim = TwoStationSim(a, b, snr_db=15.0, seed=6)
    a.connect("KK4XYZ-1")
    a.send(b"a narrow call answered")
    sim.run(until=40)
    assert a.connected and b.connected
    assert b.my_call == "KK4XYZ-1"
    assert bandwidth_code(a.peer_capabilities) == 1 and bandwidth_code(b.peer_capabilities) == 1
    # nothing moves while the session runs: it ends on the air it started on
    with pytest.raises(RuntimeError):
        b.set_air(timing, with_bandwidth(0, 2300))
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == b"a narrow call answered"
    assert a.state is State.IDLE and b.state is State.IDLE
    # and back, once it is over
    b.set_air(timing, with_bandwidth(0, 2300), max_mode=len(timing.mode_threshold_db) - 1)
    assert b.timing is timing and bandwidth_code(b.cfg.capabilities) == 0
    assert b.rate.thresholds == dict(timing.mode_threshold_db)
    # a call or a probe under way holds the air too
    c, _ = _pair(timing)
    c.connect("KK4ABC")
    with pytest.raises(RuntimeError):
        c.set_air(narrow, with_bandwidth(0, 500))
    p, _ = _pair(timing)
    p.probe("KK4ABC")
    with pytest.raises(RuntimeError):
        p.set_air(narrow, with_bandwidth(0, 500))


def test_a_new_fastest_rung_reaches_the_link(timing: PhyTiming) -> None:
    # `max_mode` is a live setting of the station's: a change reaches the link's
    # recommendations from the next burst on, under the rules' ceiling as before
    a, _ = _pair(timing)
    assert a._cap() == LinkConfig().max_mode
    a.set_max_mode(8)
    assert a._cap() == 8
    a.set_ceiling(5)
    assert a._cap() == 5
    a.set_max_mode(3)
    assert a._cap() == 3


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


def _ofdm_ladder() -> tuple[dict[int, int], dict[int, float]]:
    """A ladder of the wide air's OFDM modes alone, numbered by mode: the capacities and the
    AWGN thresholds of the rungs they sit on — what the harness tests below run, without the
    tone floor's frames."""
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    thresholds = {m.index: AWGN_THRESHOLD_DB[WIDE.rung_of(m.index)] for m in MODES}
    return caps, thresholds


def _latent_transfer(prop_s: float, tx_latency_s: float) -> tuple[bool, int]:
    """A transfer with real transport latency, the sender told (or not) about its own."""
    caps, thresholds = _ofdm_ladder()
    t = PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        data_capacity=caps,
        mode_threshold_db=thresholds,
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
    caps, thresholds = _ofdm_ladder()
    t = PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        data_capacity=caps,
        mode_threshold_db=thresholds,
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
    caps, thresholds = _ofdm_ladder()
    return PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        preamble_detect_s=4 * LONG.waveform.symbol_period_s if start_of_frame else None,
        data_capacity=caps,
        mode_threshold_db=thresholds,
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


def test_a_pinned_mode_goes_out_whatever_the_peer_recommends(timing: PhyTiming) -> None:
    """P6-7's ladder: while a mode is pinned every new frame goes out at it, the peer's
    recommendation notwithstanding, and each pinned burst leaves a rung saying how many
    of its frames the peer acknowledged at what SNR."""
    from aether_model.link.phy import Container

    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=31)
    sent: list[tuple[int, int]] = []
    original = a._transmit

    def wrapped(frames: list[TxFrame]) -> None:
        if a.state is State.CONNECTED and a.role is Role.ISS:
            sent.extend((f.mode, f.rv) for f in frames if f.container is Container.DATA)
        original(frames)

    a._transmit = wrapped  # type: ignore[method-assign]
    with pytest.raises(ValueError):
        a.pin_mode(99)
    a.pin_mode(2, body_bytes=16)
    a.connect("KK4XYZ")
    a.send(bytes(range(200)))
    a.disconnect()
    sim.run(until=300)
    assert sim.delivered(1) == bytes(range(200))
    assert sent and all(mode == 2 for mode, _ in sent), sent
    # 16-byte bodies: 200 bytes are 13 frames, six to a burst
    rungs = a.take_ladder()
    assert [r.frames for r in rungs] == [6, 6, 1], rungs
    assert all(r.mode == 2 and r.decoded == r.frames for r in rungs), rungs
    assert all(r.snr_db is not None and abs(r.snr_db - 15.0) < 1.0 for r in rungs), rungs
    assert a.take_ladder() == []
    assert a.all_acknowledged()
    # unpinned, the recommendation is followed again
    a.pin_mode(None)
    assert a._burst_mode() == min(a._recommended, a.cfg.max_mode)


def test_a_stranded_frame_is_re_encoded_at_a_mode_that_carries_it(timing: PhyTiming) -> None:
    """A frame that has gone max_combines transmissions unacknowledged at a mode the
    channel cannot carry is given another codeword at the slowest mode down to the
    recommendation that fits its body — so a ladder rung above the channel, or a link
    that drops into the floor, does not strand the session."""
    from aether_model.link.frames import data_capacity

    a, b = _pair(timing)
    top = usable_modes()[-1]
    sim = TwoStationSim(a, b, snr_db=0.0, seed=7)
    a.pin_mode(top, body_bytes=16)
    a.connect("KK4XYZ")
    a.send(bytes(range(48)))
    a.disconnect()
    sim.run(until=600)
    assert sim.delivered(1) == bytes(range(48))
    rungs = a.take_ladder()
    assert rungs and rungs[0].mode == top and rungs[0].decoded == 0, rungs
    assert a.stats.frames_reencoded >= 1
    assert sim.t < 600
    # the chooser: the slowest mode down to the recommendation that carries the body, and
    # nothing for a body only the faster mode can hold
    small = a._reencode_target(16, top, 0)
    assert small == 0
    assert a._reencode_target(16, top, 5) == 5
    full = data_capacity(timing.capacity(top))
    assert a._reencode_target(full, top, 0) is None


# ── rate control on a real path (ADR-0020) ────────────────────────────


def _connected_irs(timing: PhyTiming) -> tuple[LinkEngine, LinkEngine, list[TxFrame]]:
    """A session up, the called station receiving; what it transmits from then on is kept
    instead of sent."""
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=40)
    assert b.state is State.CONNECTED and b.role is Role.IRS
    sent: list[TxFrame] = []
    b._transmit = sent.extend  # type: ignore[method-assign]
    return a, b, sent


def _heard(
    mode: int,
    snr_db: float,
    *,
    decoded: bool,
    trusted: bool = True,
    rv: int = 0,
    combined: bool = False,
) -> object:
    from aether_model.link.engine import _RxRecord
    from aether_model.link.sim import SimFrame

    frame = SimFrame(Container.DATA, mode, rv, snr_db, 0.0, 1.0, b"", 0.0, trusted=trusted)
    return _RxRecord(frame, 0, payload=b"x" if decoded else None, combined=combined)


def test_a_burst_reports_the_snr_of_the_frames_that_were_there(timing: PhyTiming) -> None:
    """ND1J's 40 m path, 2026-09-25: a burst's failed frames read -11 dB between frames
    decoding at +5 to +8 — detections barely over their threshold, the correlator on noise —
    and the burst's mean took the recommendation from rung 4 to rung 1. The SNR a burst
    reports is now that of the frames that decoded and of those the PHY trusts: a real frame
    that failed still says how the path was."""
    _, b, sent = _connected_irs(timing)
    b._burst = [
        _heard(8, 6.0, decoded=True),
        _heard(8, -12.0, decoded=False, trusted=False),
        _heard(8, 2.0, decoded=False),
    ]
    b._send_ack()
    ack = ControlFrame.decode(sent[-1].payload)
    assert ack.snr_db == 4, ack
    # nothing that was really there: no SNR at all, and the controller's reading stands
    reading = b.rate.snr_db
    b._burst = [_heard(8, -12.0, decoded=False, trusted=False)] * 3
    b._send_ack()
    assert ControlFrame.decode(sent[-1].payload).snr_db is None
    assert b.rate.snr_db == reading


def test_a_burst_faster_than_this_station_ever_asked_for_is_the_senders_choice(
    timing: PhyTiming,
) -> None:
    """The Test's ladder pins rungs past what the path carries; their failures are no news
    to the receiving station's controller, which never asked for them — on ND1J's test they
    held its margin at the ceiling for the whole file that followed. A burst of rungs this
    station has asked for is the path's, retransmissions of them included."""
    _, b, _ = _connected_irs(timing)
    b.rate = b._rate_controller()
    b.rate.seed(0.0)
    b._asked = b.rate.recommend()
    faster = b._asked + 3
    before = (b.rate.margin_db, b.rate.recommend())
    b._burst = [_heard(faster, 1.0, decoded=False) for _ in range(4)]
    b._send_ack()
    assert (b.rate.margin_db, b.rate.recommend()) == before
    assert b.rate.snr_db == pytest.approx(0.7 * 0.0 + 0.3 * 1.0), "what it measured still counts"
    # a rung it asked for, failing: the path's news
    b._burst = [_heard(b._asked, 1.0, decoded=False) for _ in range(4)]
    b._send_ack()
    assert b.rate.margin_db > before[0] or b.rate.recommend() < before[1]


def test_a_retransmission_that_could_not_decode_alone_is_no_news(timing: PhyTiming) -> None:
    """RV 1 and 2 are mostly parity: alone they do not decode at any SNR (6 and 10 % on ND1J's
    path against 75 % at RV 0), and alone is how the retransmission of a frame this station
    already has arrives — after an acknowledgement the sender missed. Their failure taught the
    margin 3 dB for every lost acknowledgement. Combined with an earlier transmission, or at
    RV 3, a failure is the path's again, and so is a first transmission's."""
    _, b, _ = _connected_irs(timing)
    b.rate = b._rate_controller()
    b.rate.seed(6.0)
    b._asked = b.rate.recommend()
    mode = b._asked
    before = (b.rate.margin_db, b.rate.recommend())
    b._burst = [_heard(mode, 6.0, decoded=False, rv=rv) for rv in (1, 2, 1, 2)]
    b._send_ack()
    assert (b.rate.margin_db, b.rate.recommend()) == before
    for telling in (
        _heard(mode, 6.0, decoded=False, rv=1, combined=True),
        _heard(mode, 6.0, decoded=False, rv=3),
        _heard(mode, 6.0, decoded=False, rv=0),
    ):
        b.rate = b._rate_controller()
        b.rate.seed(6.0)
        b._burst = [telling]
        b._send_ack()
        assert (b.rate.margin_db, b.rate.recommend()) != before, telling


def test_the_tests_ladder_does_not_teach_the_receiver_a_margin(timing: PhyTiming) -> None:
    """End to end: a session settles, the sender pins the fastest rung for a while — far
    past the path, every frame failing until it is re-encoded — and unpins. The receiving
    station's learned margin is no wider for it."""
    a, b = _pair(timing)
    top = usable_modes()[-1]
    sim = TwoStationSim(a, b, snr_db=6.0, seed=11)
    a.connect("KK4XYZ")
    a.send(bytes(400))
    sim.run(until=120)
    assert b.state is State.CONNECTED
    margin = b.rate.margin_db
    a.pin_mode(top, body_bytes=16)
    a.send(bytes(64))
    sim.run(until=sim.t + 240)
    a.pin_mode(None)
    rungs = a.take_ladder()
    assert rungs and all(r.mode == top and r.decoded == 0 for r in rungs if r.mode == top)
    assert b.rate.margin_db <= margin + 1e-9, (margin, b.rate.margin_db)


# ── the lossy pipe's families (P9-6) ──────────────────────────────────


def test_a_narrow_session_at_the_floor_completes_through_the_pipe() -> None:
    """At −8 dB on AWGN only the 500 Hz floor family decodes (ADR-0009): its data modes and
    its control frame. Until P9-6 the pipe delivered every frame as ordinary, the engine
    rightly dropped a floor-mode frame whose flag disagreed with its mode, and every
    simulated session that reached the floor failed — the link bench's 500 Hz numbers below
    the ordinary table were a bench artefact."""
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import NARROW_500

    timing = phy_timing(NARROW_500)
    a, b = _pair(timing, LinkConfig(max_mode=14))
    sim = TwoStationSim(a, b, snr_db=-8.0, seed=3)
    message = bytes(range(60))
    a.connect("KK4XYZ")
    a.send(message)
    a.disconnect()
    sim.run(until=900)
    assert sim.delivered(1) == message, (sim.events(0), sim.events(1))
    assert b.stats.frames_received > 0


def test_a_control_frame_is_judged_at_its_own_family() -> None:
    """The pipe judges a control frame at its family's control frame, not at data mode 0 —
    on the 500 Hz air mode 0 is a floor mode at −12 dB, while the ordinary control frame
    needs −4.5: at −8 dB the floor acknowledgement gets through and the ordinary one does
    not."""
    from aether_model.link.harness import phy_timing
    from aether_model.link.phy import Container, TxFrame
    from aether_model.link.sim import control_thresholds_for
    from aether_model.waveform import NARROW_500

    timing = phy_timing(NARROW_500)
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=-8.0, seed=1)
    thresholds = control_thresholds_for(timing)
    assert thresholds[False] > -8.0 > thresholds[True]
    decoded = {False: 0, True: 0}
    for floor in (False, True):
        for _ in range(200):
            frame = sim._synthetic_frame(
                TxFrame(Container.CONTROL, b"\x01" * 7, floor=floor), -8.0, 0.0, 1.0
            )
            assert frame is not None and frame.floor is floor
            decoded[floor] += frame.decode(None)[0] is not None
    assert decoded[True] > 180 and decoded[False] < 20, decoded
    # a data frame's family is its mode's
    data = sim._synthetic_frame(TxFrame(Container.DATA, b"x", mode=1), -8.0, 0.0, 1.0)
    assert data is not None and data.floor is True


# ── holding the link on a fading path (P9-7) ──────────────────────────


@pytest.mark.parametrize("version", [1, 2, 3])
def test_a_call_in_another_link_protocol_is_ignored_and_said_so(
    timing: PhyTiming, version: int
) -> None:
    """Version 4 of the link protocol numbers the 500 Hz ladder's rungs with its middle kinds
    (ADR-0015), version 3 the 2 300 Hz one's with the fast kinds (ADR-0014), version 2 the
    ladders before them (ADR-0013), version 1 OFDM modes: a station of another version means
    other frames by the same numbers, so a call from one is not a session to start — it is
    ignored, with an event saying why."""
    from aether_model.link.frames import PROTOCOL_VERSION, ConnectBody, DataHeader, encode_data
    from aether_model.link.phy import Container, TxFrame

    assert PROTOCOL_VERSION == 4 and ConnectBody("A", "B").version == 4
    b = LinkEngine("KK4XYZ", timing, None, seed=2)
    body = ConnectBody("W4ODA", "KK4XYZ", version=version).encode()
    payload = encode_data(DataHeader(DataKind.CONNECT_REQ, 0, 7), body, timing.capacity(2))
    sim = TwoStationSim(LinkEngine("W4ODA", timing, None, seed=1), b, snr_db=10.0, seed=3)
    frame = sim._synthetic_frame(TxFrame(Container.DATA, payload, mode=2), 10.0, 0.0, 1.0)
    assert frame is not None
    b.on_frame(frame, 1.0)
    assert b.state is State.IDLE
    assert any(f"link protocol {version}" in str(e) for e in b.actions)


@pytest.mark.parametrize("reports", [False, True], ids=["no-preamble-reports", "reports"])
def test_the_iss_waits_out_the_irs_quiet_after_a_floor_burst(reports: bool) -> None:
    """The ISS sizes its wait for an ACK by the quiet the IRS keeps after a burst. It has to
    ask about the burst it sent — a floor burst straight after an ordinary acceptance — not
    the family it last heard: asked the other way it under-waited by the difference, a whole
    tone frame without preamble reports, and a floor session pinned at 16 dB died of ACK
    timeouts (ADR-0013)."""
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import WIDE_2300

    timing = phy_timing(WIDE_2300, start_of_frame=reports)
    cfg = LinkConfig(max_mode=0)
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    sim = TwoStationSim(a, b, snr_db=16.0, seed=21)
    msg = bytes(600)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=1200)
    assert sim.delivered(1) == msg
    assert a.stats.ack_timeouts == 0 and b.stats.ack_timeouts == 0


def test_the_link_timeout_spans_whole_exchanges_at_the_floor() -> None:
    """Silence ends a session after 45 s on the ordinary layouts, and only after four whole
    exchanges where the link runs the floor: one exchange there is over half a minute,
    and a slow fade outlasts one missed exchange far more often than a link really fails.
    """
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import NARROW_500, WIDE_2300

    # on the tone floor (ADR-0013) — the same frames on both airs
    exchange = 6 * 5.36 + 3.2 + 2 * 0.25 + 0.2
    for params in (WIDE_2300, NARROW_500):
        engine = LinkEngine("W4ODA", phy_timing(params), LinkConfig())
        engine._recommended = 0  # a floor mode
        assert engine._link_timeout() == pytest.approx(4 * exchange, rel=0.01)
        # the ordinary layouts on both sides: what the peer recommends and what it may send
        engine._recommended = 6
        engine.rate.seed(15.0)
        assert engine._link_timeout() == 45.0


def test_an_unanswered_burst_steps_the_recommendation_down(timing: PhyTiming) -> None:
    """A burst and its acknowledgement fade together, so a silence is evidence: the ISS steps
    down two usable modes a time, and the next acknowledgement puts the peer's own
    recommendation back."""
    a, _ = _pair(timing)
    modes = a.rate.modes
    a._recommended = modes[8]
    a._back_off()
    assert a._recommended == modes[6]
    a._recommended = modes[1]
    a._back_off()
    assert a._recommended == modes[0]
    a._back_off()
    assert a._recommended == modes[0]


def test_the_ack_waits_for_the_frame_the_preamble_announced() -> None:
    """An IRS that expects ordinary frames (the connect frames were) must not answer in the
    middle of a floor frame four times as long: the preamble names the frame's length and
    the acknowledgement waits for its end. Guessing from the peer's last mode, it fired
    inside every floor frame of a burst and trampled it until the link timed out."""
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import NARROW_500

    timing = phy_timing(NARROW_500)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(max_mode=14))
    b.role, b.state = Role.IRS, State.CONNECTED
    b._peer_mode = 5  # the narrow air's control rung, where the connect frames went
    b.rate.seed(10.0)  # and a recommendation on the ordinary layouts
    guessed = b._peer_data_frame_s()
    assert guessed < 2.0
    floor_frame = timing.data_frame_s_for(0)
    b.on_preamble(100.0, 100.3, floor_frame)
    assert b._deadlines["ack"] >= 100.0 + floor_frame
    b._deadlines.clear()
    b.on_preamble(100.0, 100.3)
    assert b._deadlines["ack"] < 100.0 + floor_frame  # the old guess, for a PHY that cannot say


def test_a_receiving_station_leaves_at_once_when_asked(timing: PhyTiming) -> None:
    """KE4QCM, 2026-09-25: five sessions ended with Abort. On the receiving side Disconnect
    only put the DISC in place of the next acknowledgement, and on a path where nothing
    decodable came there never was one. A receiving station leaves between bursts now
    (ADR-0023); the sender here stays quiet, polling nobody for five minutes."""
    a, b = _pair(timing, LinkConfig(keepalive_s=300.0))
    sim = TwoStationSim(a, b, snr_db=15.0, seed=6)
    a.connect("KK4XYZ")
    sim.run(until=30)
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    assert b.role is Role.IRS
    start = sim.t
    b.disconnect()
    sim.run(until=start + 30)
    assert b.state is State.IDLE and a.state is State.IDLE
    assert "disconnected:closed" in sim.events(1)
    assert "disconnected:peer disconnected" in sim.events(0)


def test_a_called_sender_yields_to_the_callers_poll(timing: PhyTiming) -> None:
    """KE4QCM, 2026-09-25 (23:30:58): the caller handed the turn over, heard none of the
    called station's data, took the turn back when its TURN tries ran out and polled; the
    called station, sure it held the turn, ignored the polls and gave up — "no response".
    The called station yields to the caller's poll now and asks for the turn back, so the
    session outlives a fade that lets control frames through and no data (ADR-0023)."""
    a, b = _pair(timing)
    fade: dict[str, float] = {"until": 0.0}
    # the caller detects nothing of the called station's data — not a preamble, not a frame —
    # while control frames get through both ways: no data flows the other way here, so every
    # DATA frame after the connect is the called one's
    sim = TwoStationSim(
        a,
        b,
        snr_db=15.0,
        seed=5,
        unheard=lambda rx, container, t0: (
            rx == 0 and container is Container.DATA and t0 < fade["until"]
        ),
    )
    a.connect("KK4XYZ")
    sim.run(until=30)
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    fade_until = sim.t + 150.0
    fade["until"] = fade_until
    message = b"sent while the path would carry no data"
    b.send(message)
    sim.run(until=fade_until + 300.0)
    assert sim.delivered(0) == message, sim.events(1)
    assert not any(e.startswith("disconnected") for e in sim.events(1))


def test_a_turn_waits_for_the_longest_first_frame(timing: PhyTiming) -> None:
    """KE4QCM, 2026-09-25 (23:30:58): the caller waited for the answer to its TURN as long as
    one OFDM frame, and the called station's first burst was a tone-floor frame five seconds
    long — the TURN went out again over it, three times, and the caller took the turn back.
    The wait covers the longest first frame there is, and a frame heard arriving (ADR-0023)."""
    a = LinkEngine("W4ODA", timing, LinkConfig())
    a.role, a.state, a.now = Role.ISS, State.CONNECTED, 100.0
    a.rate.seed(24.0)  # a strong path: the recommendation is a fast OFDM rung
    assert timing.data_frame_s_for(a.rate.recommend()) < timing.data_frame_s_for(0)
    a._peer_wants_tx = True
    a._maybe_start_burst()  # nothing of its own to send: it hands over
    assert a._waiting_for == "turn"
    assert a._deadlines["wait"] >= 100.0 + timing.data_frame_s_for(0)
    # a frame heard arriving late moves the next TURN past it
    t_start = a._deadlines["wait"] - 0.5
    a.on_preamble(t_start, t_start + 0.2, timing.data_frame_s_for(0))
    assert a._deadlines["wait"] >= t_start + timing.data_frame_s_for(0)


def _disconnecting(timing: PhyTiming) -> LinkEngine:
    """A sender that has just keyed its DISC and is waiting for the answer."""
    a = LinkEngine("W4ODA", timing, LinkConfig())
    a.role, a.state, a.now = Role.ISS, State.CONNECTED, 100.0
    a.disconnect()
    assert a.state is State.DISCONNECTING
    a.actions.clear()
    a.on_tx_done(101.0)
    return a


def test_a_station_waiting_for_the_answer_to_its_disc_acknowledges_nothing(
    timing: PhyTiming,
) -> None:
    """ND1J, 2026-09-25: waiting for the answer to its DISC, KK4ODA-1 took the other
    station's Morse identifier — a detection whose chips named data mode 12, undecodable — for
    a burst and armed an acknowledgement; a leaving station's acknowledgement is another DISC,
    and two went out in one keying, over the identifier."""
    from aether_model.link.sim import SimFrame

    a = _disconnecting(timing)
    retry_at = a._deadlines["wait"]
    junk = SimFrame(Container.DATA, 12, 0, -11.0, 100.9, 101.9, b"", 0.9999, trusted=False)
    a.on_frame(junk, 101.95)
    assert "ack" not in a._deadlines
    a.tick(retry_at + 5.0)
    discs = [x for x in a.actions if isinstance(x, Transmit)]
    assert len(discs) == 1, "one retry, and only one"
    assert a._disc_tries == 2


def test_a_disc_is_not_repeated_over_a_frame_heard_arriving(timing: PhyTiming) -> None:
    """The answer to a DISC can be late — the other station's own queue, a busy hold — and a
    repeat keyed over it is heard by neither station. The retry waits for the frame's end."""
    a = _disconnecting(timing)
    retry_at = a._deadlines["wait"]
    t_start = retry_at - 0.2
    a.on_preamble(t_start, t_start + 0.2, timing.control_frame_s)
    assert a._deadlines["wait"] >= t_start + timing.control_frame_s
    a.tick(t_start + timing.control_frame_s)
    assert not a.actions, "repeated over the frame"
    # a PHY that cannot name the frame: the longest there is
    b = _disconnecting(timing)
    b.on_preamble(t_start, t_start + 0.2)
    assert b._deadlines["wait"] >= t_start + timing.data_frame_s_for(0)


@pytest.mark.parametrize("bandwidth", [2300, 500])
def test_a_poll_is_not_repeated_over_its_answer_arriving(bandwidth: int) -> None:
    """ADR-0027 §7, found by the chat bench: a receiving station that detects a poll and
    cannot decode it answers the preamble once its quiet after a frame has passed — 0.99 s
    when the last frame it decoded was a floor one, and the answer goes on the floor, 3.2 s —
    while the sender waited for an answer starting within a turnaround. The re-poll went out
    0.2 s before the answer ended, the sender heard none of it, the next answer met the next
    re-poll, and the sender gave up with the link up: "no response". A sender waits out a
    frame it hears arriving, as a caller, a leaving station and one that handed over the turn
    already did."""
    from aether_model.link.harness import phy_timing
    from aether_model.link.rate import TONE_CONTROL_THRESHOLD_DB
    from aether_model.waveform import NARROW_500, WIDE_2300

    # preamble reports, as the daemon gives them: the receiving station answers what it
    # detected and could not decode
    timing = phy_timing(WIDE_2300 if bandwidth == 2300 else NARROW_500)
    a, b = _pair(timing)
    # a path the tone floor's frames cross and the ordinary control frame does not: the call
    # goes on the floor and measures a strong path, so the sender polls in the ordinary family
    # of the rung it would send at, and the receiving station, which decoded only the floor's
    # call, answers every poll it detects on the floor
    sim = TwoStationSim(
        a,
        b,
        snr_db=20.0,
        seed=3,
        control_thresholds={False: 99.0, True: TONE_CONTROL_THRESHOLD_DB},
    )
    a.connect("KK4XYZ")
    sim.run(until=100)
    assert a.state is State.CONNECTED, sim.events(0)
    assert (a._control_floor(), b._control_floor()) == (False, True), "not the case measured"
    assert a.stats.ack_timeouts == 0
    assert a.stats.acks_received >= 5, "every poll's answer is heard"


def test_a_burst_is_not_repeated_over_its_acknowledgement_arriving(timing: PhyTiming) -> None:
    """A burst's acknowledgement can start late too — the receiving station's quiet stretched
    by a frame it heard arriving, a receiver running behind — and a burst sent again over it
    loses the acknowledgement and costs the burst's air time. The retry waits for the frame's
    end, and the acknowledgement is taken when it arrives."""
    from aether_model.link.sim import SimFrame

    a = LinkEngine("W4ODA", timing, LinkConfig())
    a.role, a.state, a.now, a.session = Role.ISS, State.CONNECTED, 100.0, 7
    a.send(b"a line typed at the keyboard")
    burst = [x for x in a.drain() if isinstance(x, Transmit)]
    assert len(burst) == 1 and a._waiting_for == "ack"
    a.on_tx_done(100.0 + burst[0].duration_s)
    retry_at = a._deadlines["wait"]
    control = timing.control_frame_s_for(a._control_floor())
    t_start = retry_at - 0.2
    a.on_preamble(t_start, t_start + 0.1, control)
    assert a._deadlines["wait"] >= t_start + control
    a.tick(t_start + control)
    assert not [x for x in a.drain() if isinstance(x, Transmit)], "repeated over the frame"
    sent = len(burst[0].frames)  # sequence numbers 0 … sent − 1, every one received
    ack = ControlFrame(ControlKind.ACK, session=7, base=sent, recommended_mode=a._recommended)
    frame = SimFrame(Container.CONTROL, 0, 0, 12.0, t_start, t_start + control, ack.encode(), 0.0)
    a.on_frame(frame, t_start + control + 0.01)
    assert a._waiting_for is None and a.all_acknowledged
    assert a.stats.ack_timeouts == 0


def test_a_narrow_session_rides_a_slow_fade_at_minus_four_db() -> None:
    """The three P9-7 changes together, on the fading pipe (P9-6): at −4 dB on ITU Good the
    500 Hz floor carries a 1 kB session that the fixed 45 s timeout dropped two times in
    three — the tone floor now (ADR-0013) and its middle kinds (ADR-0015), which carry it
    with room to spare."""
    import numpy as np

    from aether_model.frame.modes import NARROW
    from aether_model.link.fading import FadingPipe, SharedFading, frame_key, shapes_for
    from aether_model.link.harness import phy_timing
    from aether_model.link.rate import NARROW_AWGN_THRESHOLD_DB
    from aether_model.link.sim import control_thresholds_for

    # the calibrated constants of the frames this session uses (bench/baselines/fading_pipe.csv)
    beta = {
        "tone-24": 0.0122,
        "tone-36": 0.0111,
        "tone4x100-51": 0.0401,
        "tone4x100-75": 0.0226,
        "tone-control": 0.0087,
        "control short": 1000.0,
    }
    timing = phy_timing(NARROW.params)
    done = 0
    for seed in range(6):
        a = LinkEngine("W4ODA", timing, LinkConfig(max_mode=14), seed=seed)
        b = LinkEngine("KK4XYZ", timing, LinkConfig(max_mode=14), seed=seed + 1)
        pipe = FadingPipe(
            SharedFading("good", seed),
            shapes_for(NARROW),
            lambda f: beta.get(frame_key(f, NARROW), 1.0),
            np.random.default_rng(seed),
        )
        sim = TwoStationSim(
            a,
            b,
            snr_db=-4.0,
            seed=seed,
            thresholds=dict(NARROW_AWGN_THRESHOLD_DB),
            control_thresholds=control_thresholds_for(timing),
            fading=pipe,
        )
        message = bytes(range(250)) * 4
        a.connect("KK4XYZ")
        a.send(message)
        a.disconnect()
        sim.run(until=900)
        done += sim.delivered(1) == message
    assert done >= 5, done


# ── chat: asking for the turn (ADR-0027) ──────────────────────────────


@pytest.fixture(scope="module")
def live() -> PhyTiming:
    """The wide air as the daemon runs it: preambles reported, as the chat rules expect."""
    from aether_model.link.harness import phy_timing
    from aether_model.waveform import WIDE_2300

    return phy_timing(WIDE_2300)


def _chat_session(
    timing: PhyTiming, chat: tuple[bool, bool] = (True, True), seed: int = 5
) -> tuple[LinkEngine, LinkEngine, TwoStationSim]:
    """A session up on a clean path, the caller holding the turn with nothing to send, each
    station in chat or not."""
    a = LinkEngine("W4ODA", timing, LinkConfig(chat=chat[0]), seed=1)
    b = LinkEngine("KK4XYZ", timing, LinkConfig(chat=chat[1]), seed=2)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=seed)
    a.connect("KK4XYZ")
    sim.run(until=30)
    assert a.connected and b.connected and a.role is Role.ISS and b.role is Role.IRS
    return a, b, sim


def _between_polls(sim: TwoStationSim, iss: LinkEngine) -> float:
    """A quiet moment on an idle link: two seconds after the sender's last poll was answered,
    its next one still six or more away."""
    t = sim.t
    while t < sim.t + 120.0:
        t += 0.25
        sim.run(until=t)
        due = iss._deadlines.get("keepalive")
        if due is not None and due - t >= iss.cfg.keepalive_s - 2.0:
            return t + 2.0
    raise AssertionError("the sender never went idle")


def _type(sim: TwoStationSim, who: int, line: bytes, at: float) -> None:
    """``line`` handed to station ``who`` at ``at``, as a host program does when its operator
    presses Enter."""
    sim.run(until=at)
    engine = sim.st[who].engine
    engine.tick(at)
    engine.send(line)
    sim._pump(who, at)


def _arrivals(sim: TwoStationSim, expected: dict[int, int], since: float) -> dict[int, float]:
    """When each station ``who`` of ``expected`` has that many bytes delivered, to a tenth of a
    second, running the simulation on from ``since`` (where it stands)."""
    got: dict[int, float] = {}
    t = since
    while len(got) < len(expected) and t < since + 300.0:
        t += 0.1
        sim.run(until=t)
        for who, length in expected.items():
            if who not in got and len(sim.delivered(who)) >= length:
                got[who] = t
    return got


def _arrival(sim: TwoStationSim, who: int, length: int, since: float) -> float | None:
    """When station ``who`` has ``length`` bytes delivered (:func:`_arrivals`)."""
    return _arrivals(sim, {who: length}, since).get(who)


def test_chat_is_off_unless_asked_for() -> None:
    assert LinkConfig().chat is False


def test_in_a_chat_the_receiving_station_asks_for_the_turn(live: PhyTiming) -> None:
    """A line typed at the station that does not hold the turn waited for the sender's next
    poll — up to ten seconds on an idle link, then a poll, its answer and a TURN before the
    line could go. In a chat (the host's CHAT ON) the station asks at once: an acknowledgement
    nobody asked for, with WANT_TX, and the idle sender hands over."""
    line = b"QSL on the report, 5 by 7 here in Atlanta"
    took = {}
    for chat in (False, True):
        a, b, sim = _chat_session(live, chat=(chat, chat))
        t = _between_polls(sim, a)
        _type(sim, 1, line, t)
        arrived = _arrival(sim, 0, len(line), t)
        assert arrived is not None, chat
        took[chat] = arrived - t
        assert b.stats.turn_requests == int(chat)
        assert a.stats.turns == 1, "one handover either way"
    assert took[False] > 6.0, took  # it waited for the poll
    assert took[True] < 4.0, took


def test_a_request_waits_until_an_answer_to_the_last_frame_could_have_begun(
    live: PhyTiming,
) -> None:
    """A sender answers an acknowledgement at once — its next burst, a TURN, a DISC — and a
    request keyed before that answer could be heard would be keyed over it. A line typed just
    after this station's acknowledgement waits out that reaction time; if a frame arrives in
    it, the request is not needed (the acknowledgement of that frame says WANT_TX)."""
    for arrives in (False, True):
        b = LinkEngine("KK4XYZ", live, LinkConfig(chat=True))
        b.role, b.state, b.now = Role.IRS, State.CONNECTED, 100.0
        b._tx_busy_until = 100.0  # its acknowledgement has just ended
        b.send(b"hello")
        assert not [x for x in b.actions if isinstance(x, Transmit)]
        due = b._deadlines["request"]
        assert due == pytest.approx(100.0 + b._reaction_s())
        if arrives:
            b.on_preamble(100.3, 100.3 + live.preamble_detect_s_for(False), live.data_frame_s)
        b.tick(due)
        sent = [f for x in b.actions if isinstance(x, Transmit) for f in x.frames]
        if arrives:
            assert not sent and b.stats.turn_requests == 0
            b.tick(b._deadlines["ack"])  # the frame's end: the acknowledgement asks
            sent = [f for x in b.actions if isinstance(x, Transmit) for f in x.frames]
        assert len(sent) == 1
        ack = ControlFrame.decode(sent[0].payload)
        assert ack.kind is ControlKind.ACK and ack.flags & ControlFlags.WANT_TX
        assert b.stats.turn_requests == int(not arrives)
        # asked once, and not again for more of the same
        b.send(b" and more")
        b.on_tx_done(b.now + 1.0)
        b.tick(b.now + 60.0)
        again = [f for x in b.actions if isinstance(x, Transmit) for f in x.frames]
        assert len(again) == 1, "asked twice"


def _request(session: int, t_end: float) -> SoftFrame:
    from aether_model.link.sim import SimFrame

    payload = ControlFrame(ControlKind.ACK, session, flags=ControlFlags.WANT_TX).encode()
    return SimFrame(Container.CONTROL, 0, 0, 10.0, t_end - SHORT.duration_s, t_end, payload, 0.0)


def test_an_idle_chat_sender_hands_over_on_a_request_and_does_not_poll_over_one(
    live: PhyTiming,
) -> None:
    """The sender's side: with nothing to send it answers a request with a TURN, and a frame it
    hears arriving just before its poll is due holds the poll back past the frame — a poll
    keyed over a request loses both. Without chat, neither: the request is no answer to
    anything, and it is ignored as before."""
    for chat in (False, True):
        a = LinkEngine("W4ODA", live, LinkConfig(chat=chat))
        a.role, a.state, a.now, a.session = Role.ISS, State.CONNECTED, 100.0, 7
        a._maybe_start_burst()  # nothing to send: the keepalive is armed
        due = a._deadlines["keepalive"]
        start = due - 0.1
        a.on_preamble(start, start + live.preamble_detect_s_for(False), SHORT.duration_s)
        held = a._deadlines["keepalive"]
        assert (held >= start + SHORT.duration_s) is chat
        a.on_frame(_request(7, start + SHORT.duration_s), start + SHORT.duration_s)
        kinds = [
            ControlFrame.decode(f.payload).kind
            for x in a.actions
            if isinstance(x, Transmit)
            for f in x.frames
        ]
        assert kinds == ([ControlKind.TURN] if chat else []), (chat, kinds)
        assert (a.role is Role.IRS) is chat


def test_chat_lines_typed_at_both_stations_at_once_both_arrive(live: PhyTiming) -> None:
    """The one risk in speaking unasked: the other station keys at the same moment — its
    operator typed too — and the two collide. Each side's usual recovery handles it: the burst
    is repeated when no acknowledgement comes, and its acknowledgement asks for the turn."""
    a, b, sim = _chat_session(live)
    t = _between_polls(sim, a)
    mine, theirs = b"going QRT for dinner shortly", b"same here, 73"
    _type(sim, 0, mine, t)
    _type(sim, 1, theirs, t)
    assert b.stats.turn_requests == 1
    sim.run(until=t + 120.0)
    assert sim.delivered(1) == mine and sim.delivered(0) == theirs
    assert not any(e.startswith("disconnected") for e in sim.events(0) + sim.events(1))


@pytest.mark.parametrize("chat", [(True, False), (False, True)], ids=["caller", "called"])
def test_a_chat_station_and_one_without_chat_still_talk(
    live: PhyTiming, chat: tuple[bool, bool]
) -> None:
    """Chat is each station's own setting, and a station that has it talks to one that has
    not — an older one included: a request nobody takes costs one control frame, and the line
    goes when the sender polls, as it always did."""
    a, b, sim = _chat_session(live, chat=chat)
    # the called station first, while the caller holds the turn; then the caller, while the
    # called station does — it took the turn to send its line
    for who, line, iss in ((1, b"first from the called station", a), (0, b"and an answer", b)):
        t = _between_polls(sim, iss)
        _type(sim, who, line, t)
        assert _arrival(sim, 1 - who, len(line), t) is not None, (chat, who)
    assert not any(e.startswith("disconnected") for e in sim.events(0) + sim.events(1))


def test_chat_can_be_switched_off_during_a_session(live: PhyTiming) -> None:
    """The host's CHAT OFF takes effect at once: a request waiting for its moment is not sent,
    and a line waits for the poll again."""
    b = LinkEngine("KK4XYZ", live, LinkConfig(chat=True))
    b.role, b.state, b.now = Role.IRS, State.CONNECTED, 100.0
    b._tx_busy_until = 100.0
    b.send(b"hello")
    assert "request" in b._deadlines
    b.set_chat(False)
    assert "request" not in b._deadlines
    b.tick(130.0)
    assert not [x for x in b.actions if isinstance(x, Transmit)]
    assert b.stats.turn_requests == 0
    b.set_chat(True)
    b.send(b" again")  # the next line asks
    assert b.stats.turn_requests == 1


def test_chat_leaves_a_transfer_alone(live: PhyTiming) -> None:
    """A file inside a chat session: the receiving station types a line while the file is
    arriving. Its acknowledgements ask for the turn as they always did — a request is never
    sent into a transfer — so the file and the line arrive exactly as without chat."""
    arrivals = {}
    for chat in (False, True):
        a, b, sim = _chat_session(live, chat=(chat, chat))
        t = _between_polls(sim, a)
        document = bytes((i * 37) % 256 for i in range(3000))
        _type(sim, 0, document, t)
        line = b"got the first part, looks good"
        _type(sim, 1, line, t + 4.0)
        got = _arrivals(sim, {1: len(document), 0: len(line)}, t + 4.0)
        assert sim.delivered(1) == document and sim.delivered(0) == line
        assert b.stats.turn_requests == 0
        arrivals[chat] = (got[1] - t, got[0] - t)
    assert arrivals[True] == arrivals[False], arrivals
