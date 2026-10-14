"""Granted slots, link protocol 8 (ADR-0069): the model engine's strict tests
(``docs/research/link-architecture/07-protocol-8.md`` §9).

* **I1 — the transmitters never radiate together**, whatever the stations' keying lead, decode
  latency, receiver recovery and false detections, as long as each stays inside what the
  protocol's constants allow — and past them the keying gate drops a transmission rather than
  send it late.
* **I2–I4 — exact, once-only, in-order delivery** under loss, late decodes and repeats; a frame
  of a slot already closed changes no timing.
* **L1–L5 — every session delivers or ends**; a slow station is reached and climbs its gap
  level.
* **The interface** the daemon uses (07-protocol-8.md §8).
"""

from __future__ import annotations

import heapq
import math
import random
from dataclasses import replace

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.engine import LinkConfig, LinkEngine, State
from aether_model.link.frames import (
    CONNECT_BODY_8_BYTES,
    CONNECT_BODY_BYTES,
    PROTOCOL_VERSION,
    TURNS_VERSION,
    ConnectBody,
    GrantFrame,
    GrantKind,
    level_of,
    with_level,
)
from aether_model.link.granted import (
    DECODE_BUDGET_S,
    KEY_S,
    SLACK_S,
    GrantConfig,
    GrantedEngine,
    Role,
    SlotTransmit,
)
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container, PhyTiming, TxFrame
from aether_model.link.sim import TwoStationSim

AIRS = {"2300": WIDE, "500": NARROW}
WIDE_WINDOW = GrantConfig().open_window_s[0] * 2


def _timing(air: AirInterface, latency: float = 0.0) -> PhyTiming:
    return replace(phy_timing(air.params), tx_latency_s=latency)


def _pair(
    air: AirInterface, latency: float = 0.0, **cfg: object
) -> tuple[GrantedEngine, GrantedEngine]:
    timing = _timing(air, latency)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(air.params),
            max_mode=air.n_rungs - 1,
            max_burst_s=26.0,
            **cfg,  # type: ignore[arg-type]
        )

    return (
        GrantedEngine("W4ODA", timing, config(), seed=11),
        GrantedEngine("KK4XYZ", timing, config(), seed=12),
    )


def _family(timing: PhyTiming, frame: TxFrame) -> bool:
    return frame.floor if frame.container is Container.CONTROL else timing.is_floor(frame.mode)


def _station_model(
    rng: random.Random, timing: PhyTiming, *, tail_s: float = 0.08, lead_s: float = SLACK_S
) -> dict[str, object]:
    """A field station, as the review's simulator draws one: a keying lead up to the slack past
    the engine's own, a decode latency inside the budget with an exponential tail past it (the
    harness measured 0.12 s typical for both families, 08 §1.3), a receiver deaf for a moment
    after transmitting, false detections now and then."""

    def latency(_rx: int, _frame: TxFrame, _end: float) -> float:
        return 0.12 + rng.expovariate(1.0 / tail_s)

    return {
        "key_lead": lambda _who: rng.uniform(0.0, lead_s),
        "decode_latency": latency,
        "rx_recovery_s": (0.1, 0.1),
        "phantoms_per_min": 3.0,
        "phantom_trusted": 0.1,
    }


def _exchange(
    sim: TwoStationSim,
    a: GrantedEngine,
    b: GrantedEngine,
    to_b: bytes,
    to_a: bytes,
    until: float = 1500.0,
) -> float | None:
    """Call, carry both messages, disconnect. Returns when both had arrived, or None."""
    a.connect(b.my_call)
    a.send(to_b)
    b.send(to_a)
    done = None
    t = 0.0
    while t < until:
        t += 5.0
        sim.run(until=t)
        if done is None and sim.delivered(1) == to_b and sim.delivered(0) == to_a:
            done = t
            a.disconnect()
        if done is not None and a.state is State.IDLE and b.state is State.IDLE:
            break
    return done


def _message(rng: random.Random, n: int) -> bytes:
    return bytes(rng.randrange(256) for _ in range(n))


# ── frames ────────────────────────────────────────────────────────────


def test_the_control_frame_carries_every_field() -> None:
    frame = GrantFrame(
        GrantKind.HEADER,
        session=0x5A,
        count=6,
        data_floor=True,
        level=3,
        fam=True,
        flag=True,
        ident=True,
        base=200,
        bitmap=0xA5C3,
        snr_db=-12.4,
        rung=19,
        n=7,
    )
    raw = frame.encode()
    assert len(raw) == 7
    assert GrantFrame.decode(raw) == replace(frame, snr_db=-12.0)
    # the SNR is six bits: −31 … +31, and "not measured" is its own code
    assert GrantFrame.decode(replace(frame, snr_db=40.0).encode()).snr_db == 31.0
    assert GrantFrame.decode(replace(frame, snr_db=-50.0).encode()).snr_db == -31.0
    assert GrantFrame.decode(replace(frame, snr_db=None).encode()).snr_db is None
    for kind in GrantKind:
        assert GrantFrame.decode(GrantFrame(kind, 1).encode()).kind is kind


def test_the_connect_body_of_protocol_8_declares_the_radio() -> None:
    body = ConnectBody(
        "W4ODA",
        "KK4XYZ",
        caps=with_level(0x03, 2),
        version=PROTOCOL_VERSION,
        snr_db=-7.0,
        turnaround_s=0.23,
        identifier_s=6.2,
    )
    raw = body.encode()
    assert len(raw) == CONNECT_BODY_8_BYTES == 19
    back = ConnectBody.decode(raw)
    # rounded up, never down: a station declares at least what its radio takes
    assert back.turnaround_s == pytest.approx(0.3)
    assert back.identifier_s == pytest.approx(6.5)
    assert level_of(back.caps) == 2 and back.caps & 0x07 == 0x03
    # protocol 7's body is two bytes shorter and reads as protocol 7
    old = ConnectBody("W4ODA", "KK4XYZ", version=TURNS_VERSION).encode()
    assert len(old) == CONNECT_BODY_BYTES
    assert ConnectBody.decode(old).version == TURNS_VERSION
    assert ConnectBody.decode(old).turnaround_s == 0.0


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_connect_body_fits_the_robust_frame_of_both_families(air: str) -> None:
    a, _ = _pair(AIRS[air])
    for floor in (True, False):
        mode = a.robust_mode(floor)
        assert a.timing.capacity(mode) >= CONNECT_BODY_8_BYTES + 5


def test_no_session_number_makes_an_all_zero_control_frame() -> None:
    a, _ = _pair(WIDE)
    for _ in range(2000):
        a.state = State.IDLE
        a.connect("KK4XYZ")
        assert a.session & 0x7F
        a.abort()


# ── I1: the transmitters never radiate together ──────────────────────


@pytest.mark.parametrize("air", ["2300", "500"])
@pytest.mark.parametrize("snr", [-6.0, 2.0, 12.0])
def test_no_two_transmitters_radiate_together(air: str, snr: float) -> None:
    rng = random.Random(f"{air}{snr}")
    a, b = _pair(AIRS[air])
    sim = TwoStationSim(a, b, snr_db=snr, seed=3, **_station_model(rng, a.timing))  # type: ignore[arg-type]
    done = _exchange(sim, a, b, _message(rng, 2500), _message(rng, 700))
    assert sim.overlaps() == [], "the two stations radiated at once"
    assert done is not None, f"not delivered: {sim.events(0)} {sim.events(1)}"


@pytest.mark.slow
@pytest.mark.parametrize("air", ["2300", "500"])
def test_no_two_transmitters_radiate_together_over_a_sweep(air: str) -> None:
    """A randomised sweep: SNR, a slow fade, loss bursts, the stations' latencies."""
    bad: list[str] = []
    for trial in range(24):
        rng = random.Random(trial)
        a, b = _pair(AIRS[air])
        base = rng.uniform(-10.0, 14.0)
        period = rng.uniform(40.0, 160.0)
        depth = rng.uniform(0.0, 14.0)

        def schedule(
            t: float, base: float = base, period: float = period, depth: float = depth
        ) -> float:
            return base - depth * (0.5 + 0.5 * math.sin(t / period * 2 * math.pi))

        model = _station_model(rng, a.timing, tail_s=rng.uniform(0.02, 0.4))
        sim = TwoStationSim(a, b, snr_schedule=schedule, seed=trial, **model)  # type: ignore[arg-type]
        _exchange(
            sim, a, b, _message(rng, rng.randint(200, 6000)), _message(rng, rng.randint(0, 2000))
        )
        if sim.overlaps():
            bad.append(f"trial {trial}: {sim.overlaps()[:3]}")
    assert not bad


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_transmission_held_past_its_slot_is_dropped_not_sent(air: str) -> None:
    """A keying lead past the slack: the gate refuses the late answer, the engine counts it,
    and nothing radiates over the master's next transmission."""
    rng = random.Random(7)
    a, b = _pair(AIRS[air])

    def lead(who: int) -> float:
        return rng.choice([0.0, 0.0, 0.0, SLACK_S + 0.4]) if who == 1 else 0.0

    sim = TwoStationSim(a, b, snr_db=6.0, seed=4, key_lead=lead)
    done = _exchange(sim, a, b, _message(rng, 1500), _message(rng, 300))
    assert sim.overlaps() == []
    assert b.stats.dropped > 0 and sim.st[1].dropped
    assert done is not None


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_grant_decoded_too_late_is_forfeited(air: str) -> None:
    """A receiver that reports control frames 4 s late for a while — more than the highest gap
    level covers — forfeits the slots of transmissions that are a header alone: it never
    answers late. The session still completes. (A header read late with data after it costs
    nothing: the slot starts after the last data frame, whose count anchors it; and lateness the
    levels cover is asked for and granted, not forfeited.)"""
    rng = random.Random(9)
    a, b = _pair(AIRS[air])

    def latency(rx: int, frame: TxFrame, end: float) -> float:
        # a stretch in which the called station reports control frames 4 s late
        late = rx == 1 and frame.container is Container.CONTROL and 12.0 < end < 80.0
        return 4.0 if late else 0.05

    sim = TwoStationSim(a, b, snr_db=8.0, seed=5, decode_latency=latency)
    done = _exchange(sim, a, b, _message(rng, 1500), _message(rng, 300))
    assert sim.overlaps() == []
    assert b.stats.forfeits > 0
    assert done is not None


# ── I2–I4: exact delivery, and old frames change no timing ───────────


@pytest.mark.parametrize("air", ["2300", "500"])
def test_delivery_is_exact_once_and_in_order_through_outages(air: str) -> None:
    rng = random.Random(21)
    a, b = _pair(AIRS[air])

    def schedule(t: float) -> float:
        return -40.0 if 30.0 < t % 120.0 < 55.0 else 4.0

    sim = TwoStationSim(a, b, snr_schedule=schedule, seed=6, **_station_model(rng, a.timing))  # type: ignore[arg-type]
    to_b, to_a = _message(rng, 5000), _message(rng, 1800)
    done = _exchange(sim, a, b, to_b, to_a, until=2500.0)
    assert done is not None
    assert sim.delivered(1) == to_b and sim.delivered(0) == to_a
    assert sim.overlaps() == []


def test_a_frame_of_a_closed_slot_moves_no_deadline() -> None:
    """An answer read late — it began before the master's last transmission ended — has its
    timing ignored: the master's next key stays where the open slot put it."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=1)
    a.connect("KK4XYZ")
    a.send(b"x" * 600)
    t = 0.0
    while not (a.role is Role.MASTER and a._slot is not None and a._slot.final) and t < 60:
        t += 0.2
        sim.run(until=t)
    slot = a._slot
    assert slot is not None
    before = dict(a._deadlines)
    old = GrantFrame(GrantKind.ANSWER, a.session & 0x7F, n=5, rung=3).encode()

    class Old:
        container = Container.CONTROL
        mode = 0
        rv = 0
        snr_db = 10.0
        trusted = True
        floor = False
        follows = None
        t_start = slot.tx_end - 3.0
        t_end = slot.tx_end - 2.5

        def decode(self, buffer: object = None) -> tuple[bytes, object]:
            return old, None

    a.on_frame(Old(), sim.t)  # type: ignore[arg-type]
    assert a._deadlines.get("proceed") == before.get("proceed")
    assert a._slave_want != 5


def test_a_frame_heard_arriving_changes_nothing_in_a_session() -> None:
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=6.0, seed=2)
    a.connect("KK4XYZ")
    a.send(b"y" * 3000)
    sim.run(until=40.0)
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    for engine in (a, b):
        before = dict(engine._deadlines)
        engine.on_preamble(engine.now + 0.1, engine.now + 0.2, 5.36)
        assert engine._deadlines == before


# ── L1–L5: every session delivers or ends; slow stations are reached ──


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_dead_path_ends_the_session_at_both_ends(air: str) -> None:
    a, b = _pair(AIRS[air])

    def schedule(t: float) -> float:
        return 8.0 if t < 60.0 else -60.0

    sim = TwoStationSim(a, b, snr_schedule=schedule, seed=3)
    a.connect("KK4XYZ")
    a.send(b"z" * 20000)
    sim.run(until=1500.0)
    assert a.state is State.IDLE and b.state is State.IDLE
    ended = {"disconnected:no response", "disconnected:link timeout"}
    assert ended & set(sim.events(0))
    assert any(e.startswith("disconnected:link timeout") for e in sim.events(1))
    assert sim.overlaps() == []


def test_the_link_timeout_spans_exchanges_of_the_family_in_use_when_it_falls_due() -> None:
    """The four whole exchanges (ADR-0012) are of the family the link runs in when the timer
    falls due. An answer that moved the link to the floor armed the timer by the ordinary
    exchange it was heard in, a third of a floor one, and on the review's ack-lost scenario the
    master ended a live session as the answer it was waiting for arrived (08 §2.6)."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=20.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    assert a.state is State.CONNECTED and not a.timing.is_floor(a._burst_mode())
    heard = a._heard_at
    ordinary = a._link_timeout()
    a.pin_mode(0)  # the link moves to the floor and the other station falls silent
    floor = a._link_timeout()
    assert floor > ordinary + 60.0
    a.tick(heard + ordinary)
    assert a.state is State.CONNECTED
    assert a.next_deadline() is not None and a.next_deadline() <= heard + floor
    a.tick(heard + floor)
    assert a.state is State.IDLE


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_slow_slave_asks_for_a_level_and_completes(air: str) -> None:
    """The called station's receiver reports every frame a second late: its answers ask for a
    gap level that covers it, the master grants it, and nothing goes late or collides."""
    rng = random.Random(4)
    a, b = _pair(AIRS[air])

    def latency(rx: int, frame: TxFrame, _end: float) -> float:
        return 1.0 if rx == 1 else 0.1

    sim = TwoStationSim(a, b, snr_db=8.0, seed=8, decode_latency=latency)
    done = _exchange(sim, a, b, _message(rng, 2000), _message(rng, 400))
    assert done is not None
    assert max(a._level_asked, b._need_level) >= 2 or b.stats.forfeits == 0
    assert sim.overlaps() == []


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_call_reaches_a_station_that_decodes_late(air: str) -> None:
    """A called station whose receiver reports 1.35 s after a frame ends, keying as late as
    the protocol allows, misses the acceptance slots at level 0; the call's level rises every
    second try and reaches it."""
    a, b = _pair(AIRS[air], latency=KEY_S)

    def latency(rx: int, frame: TxFrame, _end: float) -> float:
        return 1.35 if rx == 1 else 0.1

    sim = TwoStationSim(
        a, b, snr_db=6.0, seed=9, decode_latency=latency, key_lead=lambda _who: KEY_S
    )
    a.connect("KK4XYZ")
    sim.run(until=200.0)
    assert a.state is State.CONNECTED and a.role is Role.MASTER
    assert a.connect_tries > 1
    assert b.stats.forfeits > 0
    assert sim.overlaps() == []


# ── the interface ─────────────────────────────────────────────────────


def test_a_session_comes_up_with_its_roles_and_ends_in_order() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=1)
    done = _exchange(sim, a, b, b"hello there" * 50, b"and you" * 30)
    assert done is not None
    assert sim.events(0)[:2] == ["connected:KK4XYZ (master)", "role:master"]
    assert sim.events(1)[:2] == ["connected:W4ODA (slave)", "role:slave"]
    assert sim.events(0)[-1] == "disconnected:closed"
    assert sim.events(1)[-1] == "disconnected:peer disconnected"
    assert a.all_acknowledged() is True or a.state is State.IDLE


def test_the_slave_asks_to_leave_and_the_master_closes() -> None:
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=2)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    b.send(b"last words")
    b.disconnect()
    assert b.disconnect_requested
    sim.run(until=200.0)
    assert sim.delivered(0) == b"last words"
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:closed"


def test_the_slave_asking_to_leave_stops_the_masters_own_data() -> None:
    """The called station's Disconnect closes the session at the master's next opportunity,
    with what the called station still had delivered first and the master's own queue left
    unsent: its operator has stopped receiving (ADR-0023, protocol 7's receiving station). The
    harness's `80m-leave-together-500` (Step 2): both operators pressed Disconnect in the
    middle of a 6 kB transfer on a weak 500 Hz path, and the master went on sending its queue —
    minutes of tone frames — before it would close."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=0.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    assert a.state is State.CONNECTED
    a.send(bytes(range(256)) * 24)
    sim.run(until=80.0)
    b.send(b"last words")
    b.disconnect()
    asked = sim.t
    sim.run(until=asked + 120.0)
    assert sim.delivered(0) == b"last words"
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:closed"
    assert len(sim.delivered(1)) < 256 * 24, "the master sent its whole queue first"


def _gap_session(gaps: tuple[float, float], hold: float) -> tuple[list[tuple[float, float]], int]:
    """Both radios hold their transmitter ``hold`` after the audio stops and declare nothing;
    each station's answer gap is ``gaps``. Returns every transmission that began while the
    other station was still deaf after its own, once the session was up, and the slave's level."""
    a, b = _pair(NARROW)
    for e, gap in zip((a, b), gaps, strict=True):
        e.set_answer_gap(gap)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=9, rx_recovery_s=(hold, hold))
    a.connect("KK4XYZ")
    t = 0.0
    while a.state is not State.CONNECTED and t < 200.0:
        t += 1.0
        sim.run(until=t)
    assert a.state is State.CONNECTED
    up = sim.t
    a.send(b"x" * 400)
    b.send(b"y" * 200)
    sim.run(until=up + 300.0)
    assert sim.delivered(1) == b"x" * 400 and sim.delivered(0) == b"y" * 200
    into = [
        (end, begin)
        for who in (0, 1)
        for _, end in sim.radiated(who)
        for begin, _ in sim.radiated(1 - who)
        if begin > up and end - 1e-9 <= begin < end + hold
    ]
    return into, b.level


def test_the_answer_gap_holds_a_station_off_the_others_frames() -> None:
    """A peer slower to hear again than it declared — a VOX interface's hold left at 0 — is
    waited for by the answer gap (ADR-0036): as the master, by keying no sooner than the gap
    after the slave's frames; as the slave, by asking for the level that starts its slots the
    gap after the master's. The acceptance is the call's slot, which the caller's own declared
    turnaround and its stepping level govern, so only the session after it is judged."""
    hold = 1.1
    # the master's gap covers the slave's radio, the slave's gap the master's
    into, level = _gap_session((1.3, 1.3), hold)
    assert into == [], f"a station radiated into the other's hold: {into}"
    assert level >= 2, "the slave did not ask for a level that covers its gap"
    # without them the same radios radiate into each other's holds
    into, _ = _gap_session((0.0, 0.0), hold)
    assert into, "the radios' hold did not show without the gap"


@pytest.mark.parametrize("air", ["2300", "500"])
def test_an_aborting_master_leaves_and_the_slave_hears_it(air: str) -> None:
    a, b = _pair(AIRS[air])
    sim = TwoStationSim(a, b, snr_db=8.0, seed=3)
    a.connect("KK4XYZ")
    a.send(b"q" * 4000)
    sim.run(until=30.0)
    a.abort()
    assert a.state is State.IDLE
    sim.run(until=sim.t + 120.0)
    assert b.state is State.IDLE
    assert sim.events(1)[-1] == "disconnected:peer disconnected"
    assert sim.overlaps() == []


@pytest.mark.parametrize("air", ["2300", "500"])
def test_answers_go_on_the_floor_while_recent_slots_went_silent(air: str) -> None:
    """10-step-3.md §3.4: on `80m-poor-500-pactor-signalink` a PACTOR station's bursts killed the
    slave's short ordinary answers (4 read of 15) and spared its floor ones (6 of 6), and the
    readings, from decoded frames alone, kept asking for ordinary ones; half the seeds ran out of
    time. With ``floor_answer_memory`` the master asks for floor answers while any recent slot
    went silent: fewer slots go silent and the transfer is quicker."""

    def run(memory: int) -> tuple[int, float]:
        a, b = _pair(AIRS[air], floor_answer_memory=memory, hold_unknown=False)
        rng = random.Random(7)

        def bursts(frame: TxFrame) -> float:
            # two in three of the slave's ordinary answers lost; nothing else touched
            if frame.container is not Container.CONTROL or frame.floor:
                return 0.0
            if GrantFrame.decode(frame.payload).kind is not GrantKind.ANSWER:
                return 0.0
            return -40.0 if rng.random() < 2 / 3 else 0.0

        sim = TwoStationSim(a, b, snr_db=8.0, seed=7, frame_snr_offset=bursts)
        a.connect("KK4XYZ")
        a.send(bytes(rng.randrange(256) for _ in range(3000)))
        t = 0.0
        while len(sim.st[1].delivered) < 3000 and t < 900.0:
            t += 5.0
            sim.run(until=t)
        assert len(sim.st[1].delivered) == 3000, "the transfer did not finish"
        return a.stats.silent_slots, t

    silent_before, t_before = run(0)
    silent_after, t_after = run(4)
    assert silent_after < silent_before, (silent_before, silent_after)
    assert t_after < t_before, (t_before, t_after)


@pytest.mark.parametrize("air", ["2300", "500"])
def test_frames_whose_answer_went_unheard_are_not_resent_blind(air: str) -> None:
    """10-step-3.md §3.4: on the PACTOR path protocol 8's slave decoded more frames than protocol
    7's and still finished later — the master resent every frame of a transmission whose answer
    it had not heard, and the slave had most of them. With ``hold_unknown`` those frames wait for
    an answer to say what arrived: fewer frames for the same bytes, and sooner."""

    def run(hold: bool, seed: int) -> tuple[int, float]:
        a, b = _pair(AIRS[air], floor_answer_memory=4, hold_unknown=hold)
        rng = random.Random(seed)
        hits = random.Random(100 + seed)

        def bursts(frame: TxFrame) -> float:
            if (
                frame.container is Container.CONTROL
                and not frame.floor
                and GrantFrame.decode(frame.payload).kind is GrantKind.ANSWER
            ):
                return -40.0 if hits.random() < 2 / 3 else 0.0
            return -40.0 if hits.random() < 1 / 3 else 0.0

        sim = TwoStationSim(a, b, snr_db=8.0, seed=seed, frame_snr_offset=bursts)
        message = bytes(rng.randrange(256) for _ in range(3000))
        a.connect("KK4XYZ")
        a.send(message)
        t = 0.0
        while len(sim.st[1].delivered) < len(message) and t < 1500.0:
            t += 5.0
            sim.run(until=t)
        assert bytes(sim.st[1].delivered) == message, "not all of it, in order"
        return a.stats.frames_sent, t

    # four seeds: one transfer is a draw of the bursts, and either rule can be the luckier
    blind = [run(False, seed) for seed in range(4)]
    held = [run(True, seed) for seed in range(4)]
    sent = (sum(n for n, _ in blind), sum(n for n, _ in held))
    took = (sum(t for _, t in blind), sum(t for _, t in held))
    assert sent[1] < sent[0], sent
    assert took[1] < took[0], took


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_header_cut_before_it_could_be_read_grants_nothing(air: str) -> None:
    """10-step-3.md §3.6: a Test's abort cut the master's floor header 1 s into its 3.2 s; the
    master waited out the slot that header would have granted, 29 s, and its DISC went unheard.
    Cut before it could be read, a header grants nothing: the DISC follows the cut."""

    def run(read_rule: bool) -> tuple[float, bool, list[tuple[float, float]]]:
        a, b = _pair(AIRS[air])
        sim = TwoStationSim(a, b, snr_db=8.0, seed=3)
        a.connect("KK4XYZ")
        a.send(b"q" * 4000)
        st = sim.st[0]
        # a long transmission, a header first, its first tenth out; the simulator stops at its
        # last event, so the steps keep a clock of their own
        t = 0.0
        while t < 120.0:
            t += 0.02
            sim.run(until=t)
            sim.t = max(sim.t, t)
            first = st.first_frame
            if (
                first is not None
                and first[2]
                and first[0] <= t < first[0] + 0.1 * (first[1] - first[0])
                and st.tx_end - t > 3.0
            ):
                break
        else:
            pytest.fail("no long transmission with a header first")
        cut_at = sim.t
        if read_rule:
            sim.cut(0)
        else:
            # the rule as it was: reported as ending where its frames would have
            whole = st.tx_end
            sim._q = [ev for ev in sim._q if not (ev.kind == "tx_done" and ev.who == 0)]
            heapq.heapify(sim._q)
            st.tx_end = cut_at
            st.busy[-1] = (st.busy[-1][0], cut_at)
            a.on_tx_done(cut_at, whole)
        a.abort()
        sim.run(until=sim.t + 120.0)
        disc = next((t0 for t0, _ in st.busy if t0 >= cut_at - 1e-9), math.inf)
        return disc - cut_at, b.state is State.IDLE, sim.overlaps()

    after, left, overlaps = run(True)
    assert after < 3.0, f"the DISC waited {after:.1f} s for the slot of an unread header"
    assert left, "the slave did not hear the master leave"
    assert overlaps == []
    before, _, _ = run(False)
    assert before > after + 2.0, "the old rule waited no longer: the test does not tell"


@pytest.mark.parametrize("air", ["2300", "500"])
def test_an_aborting_slave_answers_the_next_header_with_a_disc(air: str) -> None:
    a, b = _pair(AIRS[air])
    sim = TwoStationSim(a, b, snr_db=8.0, seed=4)
    a.connect("KK4XYZ")
    a.send(b"q" * 4000)
    sim.run(until=30.0)
    b.abort()
    sim.run(until=sim.t + 120.0)
    assert a.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:peer disconnected"
    assert b.stats.left_answered >= 1
    assert sim.overlaps() == []


def test_a_peer_that_calls_again_ends_the_old_session() -> None:
    """ADR-0056: the master aborts and calls again under a new session number; the slave, still
    in the old session, ends it and answers the call."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    old = a.session
    a.abort()
    a._left = None  # say nothing about leaving: the call is the only news
    a._disarm("leave")
    a.connect("KK4XYZ")
    sim.run(until=sim.t + 60.0)
    assert a.session != old
    assert any(e == "disconnected:W4ODA called again" for e in sim.events(1))
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    assert b.session == a.session


def test_simultaneous_calls_resolve_to_one_session() -> None:
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=6)
    calls: list[tuple[float, float]] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        orig(who, at, tx)  # type: ignore[arg-type]
        frames = tx.frames  # type: ignore[attr-defined]
        if frames[0].container is Container.DATA and frames[0].payload[0] >> 5 == 1:
            calls.append(sim.st[who].busy[-1])

    sim._launch = spy  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    b.connect("W4ODA")
    sim.run(until=300.0)
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    assert a.session == b.session
    # the station whose callsign sorts lower answers
    assert b.role is Role.SLAVE if "KK4XYZ" < "W4ODA" else a.role is Role.SLAVE
    # two first calls keyed at once meet — carrier sense outside a session is the daemon's —
    # and nothing else does
    for lo, hi in sim.overlaps():
        assert sum(1 for c0, c1 in calls if c0 < hi and lo < c1) == 2


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_probe_is_answered_in_its_slot(air: str) -> None:
    a, b = _pair(AIRS[air])
    sim = TwoStationSim(a, b, snr_db=-4.0, seed=7)
    a.probe("KK4XYZ")
    assert a.probing
    sim.run(until=40.0)
    assert not a.probing and a.last_probe is not None
    assert a.last_probe.remote == "KK4XYZ"
    assert any(e.startswith("probe:KK4XYZ hears us at -4 dB") for e in sim.events(0))
    assert any(e.startswith("probed:W4ODA at -4.0 dB") for e in sim.events(1))
    assert sim.overlaps() == []


def test_a_probe_nobody_answers_says_so() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=-60.0, seed=8)
    a.probe("KK4XYZ")
    sim.run(until=60.0)
    assert "probe:KK4XYZ: no answer" in sim.events(0)
    assert a.last_probe is None


def test_a_probe_across_bandwidths_is_answered_and_says_so() -> None:
    a, _ = _pair(NARROW)
    _, b = _pair(WIDE)
    b.timing = a.timing  # the floor's frames are the same on both airs
    sim = TwoStationSim(a, b, snr_db=0.0, seed=9)
    a.probe("KK4XYZ")
    sim.run(until=40.0)
    assert a.last_probe is not None and a.last_probe.bandwidth_hz == 2300
    assert any("runs 2300 Hz, this station 500 Hz" in e for e in sim.events(0))


def test_protocols_7_and_8_ignore_each_other_and_say_so() -> None:
    timing = _timing(WIDE)
    caps = bandwidth_capabilities(WIDE.params)
    new = GrantedEngine("W4ODA", timing, GrantConfig(capabilities=caps), seed=1)
    old = LinkEngine("KK4XYZ", timing, LinkConfig(capabilities=caps), seed=2)
    sim = TwoStationSim(new, old, snr_db=10.0, seed=3)  # type: ignore[arg-type]
    new.connect("KK4XYZ")
    sim.run(until=30.0)
    assert "ignored:W4ODA calls with link protocol 8" in sim.events(1)
    new.abort()
    old.connect("W4ODA")
    sim.run(until=sim.t + 30.0)
    assert "ignored:KK4XYZ calls with link protocol 7" in sim.events(0)


def test_a_call_in_another_bandwidth_is_ignored_and_said() -> None:
    a, _ = _pair(NARROW)
    _, b = _pair(WIDE)
    b.timing = a.timing
    sim = TwoStationSim(a, b, snr_db=10.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    assert any(e.startswith("ignored:W4ODA calls in another bandwidth") for e in sim.events(1))


def test_the_ladder_reports_each_pinned_transmission() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    rungs = []
    for mode in (8, 10, 12):
        a.pin_mode(mode, body_bytes=20)
        a.send(b"L" * 80)
        sim.run(until=sim.t + 40.0)
        rungs += a.take_ladder()
    a.pin_mode(None)
    assert [r.mode for r in rungs][:3] == [8, 10, 12]
    assert all(r.frames > 0 and r.decoded == r.frames for r in rungs[:2])
    assert a.top_acked_mode is not None and a.top_acked_mode >= 10


def test_a_floor_only_ceiling_puts_every_frame_on_the_floor() -> None:
    a, b = _pair(NARROW)
    for engine in (a, b):
        engine.set_ceiling(1)
    assert a.ceiling == 1
    sent: list[TxFrame] = []
    sim = TwoStationSim(a, b, snr_db=12.0, seed=6)
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        sent.extend(tx.frames)  # type: ignore[attr-defined]
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    done = _exchange(sim, a, b, b"f" * 300, b"g" * 100)
    assert done is not None
    assert sent and all(_family(a.timing, f) for f in sent)


@pytest.mark.parametrize("air", ["2300", "500"])
def test_no_transmission_outlasts_the_key_limit(air: str) -> None:
    a, b = _pair(AIRS[air])
    for engine in (a, b):
        engine.set_max_burst_s(12.0)
    sim = TwoStationSim(a, b, snr_db=-2.0, seed=7)
    done = _exchange(sim, a, b, b"k" * 3000, b"j" * 2000)
    assert done is not None
    for who in (0, 1):
        assert max(e - s for s, e in sim.radiated(who)) <= 12.0 + 1e-9


def test_the_identifier_goes_where_the_engine_puts_it() -> None:
    """The master's ahead of its header; the slave's ahead of its answer, in a slot granted
    with its time once it said the identifier was due; neither ever over the other."""
    a, b = _pair(WIDE, identifier_s=6.0)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=8)
    keyed: list[tuple[int, SlotTransmit]] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        if isinstance(tx, SlotTransmit) and tx.identifier:
            keyed.append((who, tx))
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    a.send(b"i" * 2000)
    sim.run(until=20.0)
    a.set_identifier_due()
    b.set_identifier_due()
    sim.run(until=sim.t + 60.0)
    who = {w for w, _ in keyed}
    assert who == {0, 1}
    for _, tx in keyed:
        assert tx.identifier_s == 6.0
        assert tx.duration_s == pytest.approx(6.0 + sum(a.timing.frame_s(f) for f in tx.frames))
    assert sim.overlaps() == []


def test_a_line_typed_at_an_idle_slave_goes_in_the_open_window() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=9)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    typed = t = 60.0
    b.send(b"are you there?")
    while sim.delivered(0) != b"are you there?" and t < typed + 60.0:
        t += 0.5
        sim.run(until=t)
    # at most an open window and an exchange: never a turn to ask for
    assert t - typed < 2 * WIDE_WINDOW + 4.0
    assert sim.overlaps() == []


def test_a_bulk_transfer_leaves_out_headers_that_change_nothing() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=10)
    done = _exchange(sim, a, b, b"e" * 30000, b"")
    assert done is not None
    assert a.stats.headers_elided > 0
    assert sim.overlaps() == []


def test_the_byte_counts_follow_the_transfer() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=11)
    a.send(b"m" * 900)
    assert a.tx_pending_bytes == 900 and a.tx_undelivered_bytes == 900
    assert not a.all_acknowledged()
    a.connect("KK4XYZ")
    sim.run(until=120.0)
    assert sim.delivered(1) == b"m" * 900
    assert a.tx_pending_bytes == 0 and a.tx_undelivered_bytes == 0 and a.all_acknowledged()


def test_a_call_moves_to_another_air_and_keeps_its_tries() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=-60.0, seed=12)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    tries, due = a.connect_tries, a.connect_due
    assert tries >= 1 and due is not None
    narrow = _timing(NARROW)
    a.move_call(narrow, bandwidth_capabilities(NARROW.params), NARROW.n_rungs - 1)
    assert a.connect_tries == tries and a.connect_due == due
    assert a.timing is narrow
    with pytest.raises(RuntimeError):
        a.set_air(narrow, 0)
    a.abort()
    a.set_air(_timing(WIDE), bandwidth_capabilities(WIDE.params))
    with pytest.raises(RuntimeError):
        a.move_call(narrow, 0)


def test_callsigns_are_set_and_answered() -> None:
    a, b = _pair(WIDE)
    b.set_callsigns(["KK4XYZ", "KK4XYZ-7"])
    sim = TwoStationSim(a, b, snr_db=10.0, seed=13)
    a.connect("KK4XYZ-7")
    sim.run(until=30.0)
    assert b.state is State.CONNECTED and b.my_call == "KK4XYZ-7"
    with pytest.raises(RuntimeError):
        b.set_callsigns(["N0CALL"])
    with pytest.raises(ValueError):
        GrantedEngine("W4ODA", a.timing).connect("X", as_call="N0CALL")


# ── the constants ─────────────────────────────────────────────────────


def test_the_slot_both_stations_reckon_is_the_same() -> None:
    """The master places the slot from its own transmission's end, the slave from the frames it
    hears end: with the same constants and declared turnaround they agree to the propagation
    delay."""
    a, b = _pair(NARROW, turnaround_s=0.6)
    for floor in (False, True):
        for level in range(4):
            start = a._slot_start(100.0, floor, level, 0.6)
            assert start == b._slot_start(100.0, floor, level, 0.6)
            gap = max(DECODE_BUDGET_S[floor] + KEY_S, 0.6) + 0.05
            assert start == pytest.approx(100.0 + gap + (0.0, 0.5, 1.0, 2.0)[level])


def test_each_station_learns_how_it_is_heard_and_keeps_it_at_the_end() -> None:
    """ADR-0021: every control frame carries how its sender hears the other, so a station that
    only received still learns it; the session's last reading stays after it ends."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=7.0, seed=14)
    done = _exchange(sim, a, b, b"only one way" * 40, b"")
    assert done is not None
    for engine in (a, b):
        assert engine.ended_peer_snr_db == pytest.approx(7.0, abs=1.0)
        assert engine.peer_snr_db is None  # the session is over


def test_the_peer_capabilities_are_what_it_offered() -> None:
    a, b = _pair(WIDE)
    for engine in (a, b):
        engine.cfg.capabilities |= 0x01  # compression
    sim = TwoStationSim(a, b, snr_db=10.0, seed=15)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    assert a.peer_capabilities == b.cfg.capabilities
    assert b.peer_capabilities == a.cfg.capabilities  # the call's level bits are not a capability


def test_a_held_transmission_moves_only_what_waits_for_it() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=16)
    a.connect("KK4XYZ")
    a.send(b"h" * 4000)
    sim.run(until=20.0)
    a._deadlines["answer"] = 99.0
    before = dict(a._deadlines)
    a.on_tx_delayed(1.5)
    for name, at in before.items():
        moved = 0.0 if name in ("link", "answer") else 1.5
        assert a._deadlines[name] == pytest.approx(at + moved), name


def test_the_slot_is_measured_from_where_the_frames_ended() -> None:
    """``on_tx_done(frames_end)``: the daemon reports key-down a tail after the last frame; the
    slot is placed from the frames' end, where the other station measures it."""
    a, _ = _pair(WIDE)
    a.connect("KK4XYZ")
    slot = a._slot
    assert slot is not None and not slot.final
    a.on_tx_done(now=10.0, frames_end=9.75)
    assert slot.final and slot.tx_end == 9.75
    assert slot.start == pytest.approx(a._slot_start(9.75, slot.last_floor, 0, 0.0))


def test_the_fastest_rung_is_a_live_setting() -> None:
    a, b = _pair(WIDE)
    for engine in (a, b):
        engine.set_max_mode(8)
    sim = TwoStationSim(a, b, snr_db=30.0, seed=17)
    modes: list[int] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        frames = tx.frames  # type: ignore[attr-defined]
        modes.extend(f.mode for f in frames if f.container is Container.DATA)
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    done = _exchange(sim, a, b, b"r" * 6000, b"s" * 2000)
    assert done is not None
    assert max(modes) == 8  # the path carries far more: the setting is what holds it


def test_a_transmission_split_evenly_between_two_rungs_is_judged_at_the_slower() -> None:
    """The rate controller judges a transmission at the rung most of it went at; of two as
    common, the slower — a rule written out so the port takes the same one (the Rust engine's
    unit test says the same)."""
    from types import SimpleNamespace

    from aether_model.link.granted import _RxRecord

    timing = _timing(WIDE)
    engine = GrantedEngine("W4ODA", timing, GrantConfig(), seed=1)

    def record(mode: int, decoded: bool) -> _RxRecord:
        frame = SimpleNamespace(mode=mode, trusted=True, rv=0, snr_db=8.5, floor=False)
        return _RxRecord(frame, 0, b"" if decoded else None)  # type: ignore[arg-type]

    engine._observe([record(12, True), record(12, False), record(9, True), record(9, False)])

    def judged_at(mode: int) -> float:
        rate = GrantedEngine("W4ODA", timing, GrantConfig(), seed=1).rate
        rate.observe(8.5, 2, 2, mode)
        return rate.margin_db

    assert abs(judged_at(9) - judged_at(12)) > 0.1
    assert engine.rate.margin_db == judged_at(9)


@pytest.mark.parametrize("air", ["2300", "500"])
def test_an_answer_is_decided_early_and_radiates_at_its_slot(air: str) -> None:
    """A slot's answer is decided as soon as every frame its header stated has been reported,
    so the daemon renders it while the slot has yet to start, and it radiates at the slot's
    start all the same (``not_before``, 07-protocol-8.md §10)."""
    a, b = _pair(AIRS[air], latency=KEY_S)
    decided: list[tuple[float, SlotTransmit]] = []
    drain = b.drain

    def recorded() -> list[object]:
        out = drain()
        decided.extend((b.now, x) for x in out if isinstance(x, SlotTransmit))
        return out

    b.drain = recorded  # type: ignore[method-assign]
    sim = TwoStationSim(
        a, b, snr_db=12.0, seed=21, decode_latency=lambda *_: 0.12, key_lead=lambda _w: KEY_S
    )
    rng = random.Random(21)
    assert _exchange(sim, a, b, _message(rng, 1500), _message(rng, 300)) is not None
    answers = [(t, x) for t, x in decided if x.not_before is not None]
    assert answers, "the slave answered in slots"
    radiated = {round(t0, 6) for t0, _ in sim.radiated(1)}
    ahead = []
    for t, x in answers:
        assert x.not_before is not None and x.not_after is not None
        assert round(x.not_before, 6) in radiated, "it radiates at its slot's start"
        ahead.append(x.not_before - KEY_S - t)
    # decided a decode's spare budget ahead of the last moment: time the daemon renders in
    assert sorted(ahead)[len(ahead) // 2] > 0.1
    assert sim.overlaps() == []
