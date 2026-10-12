"""Stated answers (ADR-0068): every frame says how many frames of its burst follow it, and a
station answers a burst at the time a frame it read stated — never because of silence, an
announcement, or a frame it could not read. These run with ``LinkConfig.stated_answers`` on,
whatever the default."""

from __future__ import annotations

import pytest

from aether_model.frame.modes import NARROW, WIDE
from aether_model.link.engine import LinkConfig, LinkEngine, Role, State, Transmit
from aether_model.link.frames import (
    MAX_FOLLOWS,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    decode_data,
    encode_data,
)
from aether_model.link.harness import phy_timing
from aether_model.link.phy import Container, PhyTiming, TxFrame
from aether_model.link.sim import SimFrame, TwoStationSim

STATED = LinkConfig(stated_answers=True)


def _timing(air: str) -> PhyTiming:
    return phy_timing((WIDE if air == "2300" else NARROW).params)


@pytest.fixture(scope="module", params=["2300", "500"])
def air(request: pytest.FixtureRequest) -> PhyTiming:
    return _timing(request.param)


def _receiving(timing: PhyTiming) -> tuple[LinkEngine, list[TxFrame]]:
    """A session up with stated answers, the called station receiving and nothing due; what it
    transmits from then on is kept instead of sent."""
    a = LinkEngine("W4ODA", timing, STATED, seed=1)
    b = LinkEngine("KK4XYZ", timing, STATED, seed=2)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=40)
    assert b.state is State.CONNECTED and b.role is Role.IRS
    sent: list[TxFrame] = []
    b._transmit = sent.extend  # type: ignore[method-assign]
    for timer in ("ack", "close"):
        b._deadlines.pop(timer, None)
    b.drain()
    return b, sent


def _data(
    b: LinkEngine,
    seq: int,
    mode: int,
    t0: float,
    follows: int | None,
    *,
    decoded: bool = True,
    trusted: bool = True,
) -> SimFrame:
    """A DATA frame of ``b``'s session that began at ``t0``: read (decoded), or failed with its
    acquisition trusted or not."""
    payload = encode_data(
        DataHeader(DataKind.DATA, seq, b.session), b"x" * 8, b.timing.capacity(mode)
    )
    t1 = t0 + b.timing.data_frame_s_for(mode)
    return SimFrame(
        Container.DATA,
        mode,
        0,
        20.0 if decoded else -2.0,
        t0,
        t1,
        payload,
        0.0 if decoded else 2.0,
        floor=b.timing.is_floor(mode),
        trusted=trusted,
        follows=follows,
    )


def _ordinary(timing: PhyTiming) -> int:
    """An OFDM rung the 15 dB session carries."""
    return next(m for m in range(len(timing.data_capacity or {})) if not timing.is_floor(m)) + 2


# ── the answer at the stated time ─────────────────────────────────────


def test_the_answer_is_due_when_the_frame_said(air: PhyTiming) -> None:
    """A frame that says two follow puts the answer two slots and a turnaround after its end;
    the next frame says the same time; an announcement moves nothing; and the answer goes
    then, once."""
    b, sent = _receiving(air)
    mode = _ordinary(air)
    slot = air.data_frame_s_for(mode)
    t0 = b.now + 1.0
    first = _data(b, b._rx_base, mode, t0, 2)
    b.on_frame(first, first.t_end + 0.05)
    due = first.t_end + 2 * slot + air.turnaround_s
    assert b._deadlines["ack"] == pytest.approx(due)
    b.on_preamble(first.t_end + 0.3, first.t_end + 0.35, 3 * slot)
    assert b._deadlines["ack"] == pytest.approx(due), "an announcement moved the answer"
    second = _data(b, (b._rx_base + 1) % 256, mode, first.t_end, 1)
    b.on_frame(second, second.t_end + 0.05)
    assert b._deadlines["ack"] == pytest.approx(due)
    b.tick(due - 0.01)
    assert not sent
    b.tick(due + 0.01)
    assert len(sent) == 1
    assert ControlFrame.decode(sent[0].payload).kind is ControlKind.ACK


def test_a_burst_nothing_of_which_decoded_is_not_answered(air: PhyTiming) -> None:
    """Frames that failed with a trusted acquisition are the burst's — counted against the
    path — but say nothing of its end: the burst closes a frame after the last one, unanswered,
    and the rate controller hears of it then."""
    b, sent = _receiving(air)
    mode = _ordinary(air)
    before = b.rate.recommend()
    t0 = b.now + 1.0
    first = _data(b, b._rx_base, mode, t0, 1, decoded=False)
    b.on_frame(first, first.t_end + 0.05)
    second = _data(b, (b._rx_base + 1) % 256, mode, first.t_end, 0, decoded=False)
    b.on_frame(second, second.t_end + 0.05)
    assert "ack" not in b._deadlines and "close" in b._deadlines
    b.tick(second.t_end + 30.0)
    assert not sent, "answered a burst it could not read"
    assert not b._burst
    assert b.rate.recommend() < before


def test_an_untrusted_frame_that_did_not_decode_is_nothing(air: PhyTiming) -> None:
    """A detection whose acquisition the receiver does not trust, and which does not decode, is
    no part of anything: no burst, no answer, no close (ADR-0065's phantoms)."""
    b, sent = _receiving(air)
    phantom = _data(b, b._rx_base, _ordinary(air), b.now + 1.0, 0, decoded=False, trusted=False)
    b.on_frame(phantom, phantom.t_end + 0.05)
    assert not b._burst and "ack" not in b._deadlines and "close" not in b._deadlines
    b.tick(b.now + 30.0)
    assert not sent


def test_a_frame_whose_count_is_lost_states_nothing(air: PhyTiming) -> None:
    """A frame that decoded with no count — a tone frame whose middle block faded — is kept and
    delivered, and states nothing: the burst closes unanswered, and the next answer covers it.
    Answered as if three followed, its answer went out over the burst the sender sent again."""
    b, sent = _receiving(air)
    received = b.stats.frames_received
    frame = _data(b, b._rx_base, _ordinary(air), b.now + 1.0, None)
    b.on_frame(frame, frame.t_end + 0.05)
    assert b.stats.frames_received == received + 1
    assert "ack" not in b._deadlines and "close" in b._deadlines
    b.tick(frame.t_end + 30.0)
    assert not sent and not b._burst


def test_a_frame_of_a_burst_already_answered_is_no_burst(air: PhyTiming) -> None:
    """A frame read after this station's answer went, whose burst's end it states before that
    answer, belongs to the burst already answered: what it carried is kept, and no second
    answer goes."""
    b, sent = _receiving(air)
    mode = _ordinary(air)
    t0 = b.now + 1.0
    frame = _data(b, b._rx_base, mode, t0, 0)
    b._tx_started = frame.t_end + air.turnaround_s + 0.5
    b.now = b._tx_started + 0.1
    b.on_frame(frame, b.now)
    assert not b._burst and "ack" not in b._deadlines
    assert not sent


def test_an_acknowledgement_that_takes_the_turn_states_its_burst() -> None:
    """The acknowledgement that takes an offered turn says how many frames follow it, in slots
    of the burst behind it: its sender's answer is due at their end whether any of the burst is
    read or not."""
    timing = _timing("2300")
    a = LinkEngine("W4ODA", timing, STATED, seed=1)
    a.state, a.role, a.session, a.now = State.CONNECTED, Role.ISS, 7, 100.0
    a._recommended = _ordinary(timing)  # a floor burst offers nothing: its TURN is 3.2 s
    a.send(b"hello")
    a._maybe_start_burst()
    frames = [f for x in a.drain() if isinstance(x, Transmit) for f in x.frames]
    kinds = [f.container for f in frames]
    assert kinds == [Container.DATA, Container.CONTROL], kinds
    offer = ControlFrame.decode(frames[-1].payload)
    assert offer.kind is ControlKind.TURN and offer.flags & ControlFlags.OFFER
    assert frames[-1].floor == timing.is_floor(frames[0].mode), "the offer left its burst's family"
    assert frames[0].follows == 1, "the offer takes a slot"
    a.on_tx_done(a._tx_busy_until)
    t_end = a._tx_busy_until + timing.turnaround_s + timing.control_frame_s
    payload = ControlFrame(
        ControlKind.ACK,
        7,
        flags=ControlFlags.TAKEN | ControlFlags.WANT_TX,
        base=1,
        bitmap=1,
        follows=2,
        burst_floor=False,
    ).encode()
    ack = SimFrame(
        Container.CONTROL, 0, 0, 20.0, t_end - timing.control_frame_s, t_end, payload, 0.0
    )
    a.on_frame(ack, t_end + 0.05)
    assert a.role is Role.IRS
    due = t_end + 2 * timing.data_frame_s + timing.turnaround_s
    assert a._deadlines["ack"] == pytest.approx(due)


def test_answers_go_on_the_floor_while_the_other_station_hears_this_one_weakly(
    air: PhyTiming,
) -> None:
    """An answer goes on the tone floor while the other station's report of this one is under
    the ordinary control frame's threshold or above it by less than the margin: WC4Y heard
    KK4ODA-1 at −7.8 and −9.2 dB, and its one ordinary acknowledgement in each of three Tests
    was lost. Above the margin it is ordinary, as before."""
    b, _ = _receiving(air)
    b._peer_floor = False
    threshold = b._ordinary_control_threshold()
    b.peer_snr_db = threshold + b.cfg.answer_floor_margin_db - 0.5
    assert b._control(ControlKind.ACK).floor
    b.peer_snr_db = threshold + b.cfg.answer_floor_margin_db + 0.5
    assert not b._control(ControlKind.ACK).floor


# ── the sender ────────────────────────────────────────────────────────


def test_an_unanswered_burst_goes_again_after_a_widening_random_backoff() -> None:
    """A first silence repeats the burst at once; from the second the repeat waits a random
    time up to a slot per silence, so two stations that both believe they hold the turn are not
    keyed over each other on every slot (ADR-0068)."""
    timing = _timing("500")
    a = LinkEngine("W4ODA", timing, STATED, seed=1)
    a.state, a.role, a.session, a.now = State.CONNECTED, Role.ISS, 7, 100.0
    a.send(bytes(400))
    a._maybe_start_burst()
    assert a._waiting_for == "ack"
    a.drain()
    a.on_tx_done(a._tx_busy_until)
    a.tick(a._deadlines["wait"] + 0.01)
    assert [x for x in a.drain() if isinstance(x, Transmit)], "the first silence repeats at once"
    a.on_tx_done(a._tx_busy_until)
    a.tick(a._deadlines["wait"] + 0.01)
    assert not [x for x in a.drain() if isinstance(x, Transmit)]
    assert "resend" in a._deadlines
    slot = a._slot_s(timing.is_floor(a._burst_mode()))
    assert a._deadlines["resend"] - a.now <= slot + 1e-9
    a.tick(a._deadlines["resend"] + 0.01)
    assert [x for x in a.drain() if isinstance(x, Transmit)]


# ── sessions ──────────────────────────────────────────────────────────


def _collisions(sim: TwoStationSim) -> int:
    a, b = sim.st[0].busy, sim.st[1].busy
    return sum(any(x < b1 - 1e-9 and b0 + 1e-9 < y for x, y in a) for b0, b1 in b)


@pytest.mark.parametrize("snr_db", [15.0, 3.0, -8.0])
def test_bursts_state_their_counts_and_a_session_both_ways_completes(
    air: PhyTiming, snr_db: float
) -> None:
    """Every transmission that carries data is at most four frames, each stating exactly how
    many follow it — an acknowledgement leading the other station's burst too — and a session
    with traffic both ways completes and closes without one station keyed over the other."""
    a = LinkEngine("W4ODA", air, STATED, seed=1)
    b = LinkEngine("KK4XYZ", air, STATED, seed=2)
    sent: list[list[TxFrame]] = []
    for e in (a, b):
        original = e._transmit

        def wrapped(frames: list[TxFrame], original=original) -> None:  # type: ignore[no-untyped-def]
            sent.append(list(frames))
            original(frames)

        e._transmit = wrapped  # type: ignore[method-assign]
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=7)
    there, back = bytes(range(256)) * 6, bytes(range(200))
    a.connect("KK4XYZ")
    a.send(there)
    b.send(back)
    t = 0.0
    while t < 1500 and (sim.delivered(1) != there or sim.delivered(0) != back):
        t += 5.0
        sim.run(until=t)
    assert sim.delivered(1) == there and sim.delivered(0) == back
    a.disconnect()
    sim.run(until=t + 300)
    assert "disconnected:closed" in sim.events(0), sim.events(0)
    assert _collisions(sim) == 0
    bursts = [f for f in sent if any(x.container is Container.DATA for x in f)]
    assert bursts
    for frames in bursts:
        data = [x for x in frames if x.container is Container.DATA]
        if decode_data(data[0].payload)[0].kind is not DataKind.DATA:
            continue  # a call, an acceptance: one frame, nothing follows
        assert len(frames) <= MAX_FOLLOWS + 1, frames
        for i, f in enumerate(frames):
            follows = (
                ControlFrame.decode(f.payload).follows
                if f.container is Container.CONTROL
                else f.follows
            )
            assert follows == len(frames) - 1 - i, (i, frames)


def test_a_receiver_hearing_frames_it_cannot_read_keeps_the_link() -> None:
    """ADR-0033's case with stated answers: the path falls below the ordinary rungs while the
    called station holds the turn. Its bursts are not answered — nothing of them is read — and
    the receiving station, which no longer transmits, keeps the session for the frames it hears
    arriving, sized for their family, until the floor bursts are read."""
    timing = _timing("500")
    cfg = LinkConfig(max_mode=4, max_burst_s=28.85, stated_answers=True)
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    fade = 25.0
    sim = TwoStationSim(
        a, b, snr_db=10.0, seed=5, snr_schedule=lambda t: -16.0 if t >= fade else 10.0
    )
    message = bytes(range(240)) * 2
    a.connect("KK4XYZ")
    a.send(b"hello")
    b.send(message)
    sim.run(until=fade)
    assert b.role is Role.ISS and 0 < len(sim.delivered(0)) < len(message), "not the case"
    sim.run(until=1500)
    assert sim.delivered(0) == message, (sim.events(0), sim.events(1))
    assert not any("link timeout" in e for e in sim.events(0) + sim.events(1))


def test_a_disconnect_leaves_once_the_floor_went_unanswered() -> None:
    """ADR-0039 with stated answers: a path that carries none of the queue sends nothing back,
    so the sender asked to leave goes once the other station has been heard not at all for its
    patience and every frame left has gone on the floor unanswered — before the link timeout,
    and with the DISC answered."""
    timing = _timing("500")
    a = LinkEngine("ND1J", timing, STATED, seed=1)
    b = LinkEngine("KK4ODA", timing, STATED, seed=2)
    sim = TwoStationSim(a, b, snr_db=6.0, seed=3)
    a.connect("KK4ODA")
    sim.run(until=60)
    assert a.state is State.CONNECTED
    sim.thresholds = {m: 99.0 for m in range(len(timing.data_capacity or {}))}
    a.send(bytes(400))
    asked = sim.t
    a.disconnect()
    sim.run(until=asked + 600)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert any(e.startswith("disconnect:") and "not acknowledged" in e for e in sim.events(0))
    assert "disconnected:closed" in sim.events(0), sim.events(0)
    assert sim.t - asked < a._link_timeout()


def test_lopsided_answers_do_not_send_a_patient_sender_away() -> None:
    """The other side of that rule: on WC4Y's lopsided path (ADR-0059) the receiver's ordinary
    acknowledgements are lost while its floor ones arrive. A sender asked to leave that hears
    them now and then is making progress, and stays to deliver everything."""
    timing = _timing("500")

    def lopsided(frame: TxFrame) -> float:
        return -8.0 if frame.container is Container.CONTROL and not frame.floor else 0.0

    a = LinkEngine("W4ODA", timing, STATED, seed=1)
    b = LinkEngine("KK4XYZ", timing, STATED, seed=2)
    b._note_ack_lost = lambda decoded: None  # type: ignore[method-assign]
    sim = TwoStationSim(a, b, snr_db=0.0, seed=59, frame_snr_offset=lopsided)
    msg = bytes(range(256)) * 8
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=3000)
    assert sim.delivered(1) == msg
    assert not any("not acknowledged" in e for e in sim.events(0)), sim.events(0)
