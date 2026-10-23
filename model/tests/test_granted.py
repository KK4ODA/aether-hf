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
from collections.abc import Callable
from dataclasses import replace

import pytest

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link import granted as granted_module
from aether_model.link.frames import (
    CONNECT_BODY_8_BYTES,
    CONNECT_BODY_BYTES,
    PROTOCOL_VERSION,
    REPORT,
    ConnectBody,
    DataHeader,
    DataKind,
    GrantFrame,
    GrantKind,
    data_capacity,
    decode_data,
    encode_data,
    level_of,
    pieces,
    with_level,
)
from aether_model.link.granted import (
    DECODE_BUDGET_S,
    KEY_S,
    READABLE_ALONE_RVS,
    SLACK_S,
    Event,
    GrantConfig,
    GrantedEngine,
    LadderRung,
    Role,
    SlotTransmit,
    State,
    Transmit,
)
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container, PhyTiming, TxFrame
from aether_model.link.sim import SimFrame, TwoStationSim

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
        measure=True,
        base=200,
        bitmap=0xA5C2,
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
    # one bit, two meanings by kind (protocol 9): a header's or an answer's measurement, a
    # DISC's or a DISC_ACK's closing identifier
    for kind in GrantKind:
        closing = kind in (GrantKind.DISC, GrantKind.DISC_ACK)
        read = GrantFrame.decode(GrantFrame(kind, 1, ident=True, measure=True).encode())
        assert (read.ident, read.measure) == (closing, not closing)


def test_a_report_acknowledges_nothing_at_or_above_its_base() -> None:
    # a plain acknowledgement's bit 0 is always clear: base is the first frame not received
    plain = GrantFrame(GrantKind.ANSWER, 1, base=10, bitmap=0b0110)
    assert not plain.reports
    assert [plain.received(s) for s in (9, 10, 11, 12, 13)] == [True, False, True, True, False]
    # with REPORT set the bitmap says which of the last measurement's frames arrived
    report = replace(plain, bitmap=REPORT | 0b10100)  # frames 1 and 3
    assert report.reports
    assert [report.measured(i) for i in range(4)] == [False, True, False, True]
    assert [report.received(s) for s in (9, 10, 11, 12, 13)] == [True, False, False, False, False]
    assert not plain.measured(0)


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
    old = ConnectBody("W4ODA", "KK4XYZ", version=7).encode()
    assert len(old) == CONNECT_BODY_BYTES
    assert ConnectBody.decode(old).version == 7
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


def test_a_disconnect_on_a_slow_path_closes_within_its_minute_and_says_what_it_left() -> None:
    """ADR-0072, from the nightly soak of 2026-10-15: on three slow paths (27–49 bit/s) with
    6–15 kB queued, Disconnect waited for the queue while every exchange still acknowledged a
    little of it, so the patience that ends a stalled drain (ADR-0039) never ran out — the
    sessions would have closed 30–40 minutes later. The drain now ends a minute after the
    disconnect was asked, at the master's next opportunity, and the master says what it left."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=0.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    assert a.state is State.CONNECTED
    message = bytes(range(256)) * 60
    a.send(message)
    sim.run(until=80.0)
    a.disconnect()
    asked = sim.t
    drain = GrantConfig().disc_drain_s
    # the engine counts from its own clock: the last moment it was given
    assert a.leave_by == pytest.approx(a.now + drain)
    sim.run(until=asked + drain + 90.0)
    assert a.state is State.IDLE and b.state is State.IDLE
    left = [e for e in sim.events(0) if e.startswith("disconnect:")]
    assert len(left) == 1, sim.events(0)
    assert "bytes not delivered" in left[0] and "after the disconnect was asked" in left[0]
    assert sim.events(0)[-1] == "disconnected:closed"
    assert 0 < len(sim.delivered(1)) < len(message)
    assert a.leave_by is None


def test_a_disconnect_with_a_small_queue_still_delivers_it_and_says_nothing() -> None:
    """The limit is for a queue that will not drain in a minute; one that does goes whole, as
    before, and nothing is said to have been left."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=6.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    a.send(b"seventy-three" * 4)
    a.disconnect()
    sim.run(until=sim.t + 120.0)
    assert sim.delivered(1) == b"seventy-three" * 4
    assert not [e for e in sim.events(0) if e.startswith("disconnect:")], sim.events(0)
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

    # eight seeds: one transfer is a draw of the bursts, and either rule can be the luckier.
    # Four were enough until ADR-0093 let the 2 300 Hz link leave rung 7: then seeds 0-3 tied
    # to the second at 2 300 Hz (held slower in two), while twelve still gave held 20 % less
    # time and a quarter fewer frames
    blind = [run(False, seed) for seed in range(8)]
    held = [run(True, seed) for seed in range(8)]
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


@pytest.mark.parametrize("side", ["slave", "master"])
@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_count_rescued_with_its_block_places_no_end(
    air: str, side: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    """10-step-3.md §6: on the scenario harness the first data frame of a transmission without
    its header reached the slave too weak to decode alone. HARQ rescued its block, and its count,
    read off that copy's chips outside the codeword, said none to follow where three did; the
    slave answered over them. A count is believed only from a copy that decoded alone, by either
    station. The sender's data is pinned a little above the path, so frames are rescued, and
    every rescued copy's count reads as none to follow (``count_misread``). The master's case
    needs the slave's answer lost — its stated count is the end otherwise — so half are."""
    snr = {"2300": 8.0, "500": 4.0}[air]
    table = _timing(AIRS[air]).mode_threshold_db
    # the rung whose threshold is the nearest over the path by 1.5 dB or more
    pin = min((th, mode) for mode, th in table.items() if th >= snr + 1.5)[1]

    def overlapping_runs(header_above: bool = True) -> int:
        runs = 0
        for seed in range(4):
            a, b = _pair(AIRS[air], header_above_recommendation=header_above)
            sender, receiver = (a, 1) if side == "slave" else (b, 0)
            hits = random.Random(900 + seed)

            def answers_lost(frame: TxFrame, hits: random.Random = hits) -> float:
                if side == "slave" or frame.container is not Container.CONTROL:
                    return 0.0
                if GrantFrame.decode(frame.payload).kind is not GrantKind.ANSWER:
                    return 0.0
                return -40.0 if hits.random() < 0.5 else 0.0

            sim = TwoStationSim(
                a, b, snr_db=snr, seed=seed, count_misread=1.0, frame_snr_offset=answers_lost
            )
            message = _message(random.Random(seed), 600)
            a.connect(b.my_call)
            sim.run(until=30.0)
            sender.pin_mode(pin)
            sender.send(message)
            if side == "master":
                a.send(b"x" * 200)  # the master's headers grant the slave its room
            t = sim.t
            while t < 900.0 and len(sim.st[receiver].delivered) < len(message):
                t += 5.0
                sim.run(until=t)
            assert bytes(sim.st[receiver].delivered) == message, "not all of it, in order"
            runs += bool(sim.overlaps())
        return runs

    assert overlapping_runs() == 0, "a station radiated over the other's frames"
    taken = GrantedEngine._take_data

    def believed(self: GrantedEngine, frame: object) -> object:
        rec = taken(self, frame)  # type: ignore[arg-type]
        if rec.payload is not None:
            rec.combined = False  # a rescued copy's count believed, as before the rule
        return rec

    monkeypatch.setattr(GrantedEngine, "_take_data", believed)
    if (air, side) == ("500", "slave"):
        # ADR-0073 guards this case as well: here every transmission whose counts only rescued
        # copies could carry was of RV 2 and 3 copies, and keeps its header now (16 seeds, no
        # overlap with the rescued count believed). Without both rules the count misleads
        monkeypatch.setattr(granted_module, "READABLE_ALONE_RVS", frozenset({0, 1, 2, 3}))
    # and ADR-0076 guards it where the header is ordinary: data pinned above the recommendation
    # keeps its header, whose count the slave reads first. The hazard is the headerless case
    assert overlapping_runs(header_above=False) > 0, (
        "no rescued count misled anyone: the test does not tell"
    )


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


def test_a_session_closed_without_its_disc_ack_is_left_unheard() -> None:
    """A master whose DISCs all went unanswered closes "closed (no DISC_ACK)", which leaves the
    other station perhaps still in the session: like an abort, that is a session left unheard,
    and a DISC of it heard afterwards is answered (ADR-0052). The model listed the reason as
    "closed (no disc ack)", which it never ended a session with, and the port did not: found
    when protocol 7's engine, which held the list, was deleted (ADR-0069 Step 5)."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(
        a,
        b,
        snr_db=10.0,
        seed=7,
        unheard=lambda rx, container, t0: rx == 0 and sim.t > closing and t0 > closing,
    )
    closing = math.inf
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    assert a.connected
    closing = sim.t
    a.disconnect()
    sim.run(until=sim.t + 120.0)
    assert "disconnected:closed (no DISC_ACK)" in sim.events(0), sim.events(0)
    # the other station's DISC of that session, heard late, is answered with a DISC_ACK
    disc = GrantFrame(GrantKind.DISC, a.session & 0x7F, fam=True).encode()
    length = a.timing.control_frame_s_for(True)
    frame = SimFrame(Container.CONTROL, 0, 0, 10.0, sim.t, sim.t + length, disc, 0.0, floor=True)
    a.on_frame(frame, sim.t + length)
    a.tick(sim.t + 30.0)
    sent = [f for x in a.drain() if isinstance(x, Transmit) for f in x.frames]
    assert [GrantFrame.decode(f.payload).kind for f in sent] == [GrantKind.DISC_ACK]


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


@pytest.mark.parametrize("version", [7, 8])
def test_a_call_in_another_link_protocol_is_ignored_and_said_so(version: int) -> None:
    """A call from a station of protocol 7 (the turn engine, retired in ADR-0069's Step 5) or 8
    is read, not answered, and said to be another protocol: the daemon puts that on the
    panel's mismatch banner, and the operator knows to update rather than to call again."""
    timing = _timing(WIDE)
    caps = bandwidth_capabilities(WIDE.params)
    b = GrantedEngine("W4ODA", timing, GrantConfig(capabilities=caps), seed=1)
    body = ConnectBody("KK4XYZ", "W4ODA", caps=caps, version=version).encode()
    mode = b.robust_mode(True)
    payload = encode_data(DataHeader(DataKind.CONNECT_REQ, 0, 77), body, timing.capacity(mode))
    length = timing.data_frame_s_for(mode)
    call = SimFrame(Container.DATA, mode, 0, 10.0, 0.0, length, payload, _draw=0.0, floor=True)
    b.on_frame(call, length)
    actions = b.drain()
    assert Event("ignored", f"KK4XYZ calls with link protocol {version}") in actions
    assert not any(isinstance(a, Transmit) for a in actions)
    assert b.state is State.IDLE


def test_a_call_in_another_bandwidth_is_ignored_and_said() -> None:
    a, _ = _pair(NARROW)
    _, b = _pair(WIDE)
    b.timing = a.timing
    sim = TwoStationSim(a, b, snr_db=10.0, seed=4)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    assert any(e.startswith("ignored:W4ODA calls in another bandwidth") for e in sim.events(1))


def test_the_ladder_reports_each_measurement() -> None:
    """A measurement is filler at its rung, reported and never delivered: the session's data
    and its counts are untouched by it (ADR-0077)."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=5)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    rungs = []
    for mode in (8, 10, 12):
        a.measure(mode, 4)
        assert a.measuring
        sim.run(until=sim.t + 40.0)
        assert not a.measuring
        rungs += a.take_ladder()
    assert [r.mode for r in rungs] == [8, 10, 12]
    assert all(r.frames == 4 and r.decoded == r.frames for r in rungs[:2])
    assert all(r.snr_db is not None for r in rungs)
    assert sim.delivered(1) == b"" and b.stats.bytes_delivered == 0
    assert a.top_acked_mode is None and a.stats.bytes_acked == 0
    assert a.all_acknowledged() and a.tx_undelivered_bytes == 0
    with pytest.raises(ValueError):
        a.measure(99, 4)
    with pytest.raises(ValueError):
        a.measure(8, 0)


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


# ── the closing identifier (ADR-0075) ─────────────────────────────────


def _on_air(sim: TwoStationSim) -> list[list[SlotTransmit]]:
    """Every transmission each station radiated, in order — the keying gate's drops left out."""
    sent: list[list[SlotTransmit]] = [[], []]
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        dropped = len(sim.st[who].dropped)
        orig(who, at, tx)  # type: ignore[arg-type]
        if len(sim.st[who].dropped) == dropped:
            sent[who].append(tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    return sent


def _kind(tx: SlotTransmit) -> str:
    """A transmission by what it is: ``id`` alone, a control frame's kind, ``data``."""
    if not tx.frames:
        return "id"
    first = tx.frames[0]
    if first.container is Container.CONTROL:
        return GrantFrame.decode(first.payload).kind.name.lower()
    return "data"


def _ends_with_identifiers(sim: TwoStationSim, sent: list[list[SlotTransmit]]) -> None:
    """Each station's last transmission ends with its closing identifier, nothing of either
    went over the other's, and no identifier went anywhere but at a session's end: behind a
    DISC_ACK, or alone."""
    for who in (0, 1):
        assert sent[who], f"station {who} sent nothing"
        last = sent[who][-1]
        assert last.identifier, (who, [_kind(t) for t in sent[who]])
        for tx in sent[who]:
            if tx.identifier:
                assert _kind(tx) in ("disc_ack", "id"), (who, _kind(tx))
    assert sim.overlaps() == []


def _lost(kind: GrantKind, count: int | None = None):  # type: ignore[no-untyped-def]
    """A frame offset that loses control frames of one kind: the first ``count``, or all."""
    seen = [0]

    def offset(frame: TxFrame) -> float:
        if frame.container is not Container.CONTROL:
            return 0.0
        if GrantFrame.decode(frame.payload).kind is not kind:
            return 0.0
        seen[0] += 1
        return -60.0 if count is None or seen[0] <= count else 0.0

    return offset


@pytest.mark.parametrize("air", ["2300", "500"])
@pytest.mark.parametrize("closing", ["master", "slave", "both"])
def test_a_session_ends_with_both_identifiers_last(air: str, closing: str) -> None:
    """ADR-0075: the DISC, then the slave's DISC_ACK with its identifier right behind it in the
    time the DISC granted, then the master's identifier on its own once that slot is over —
    each station's identifier the last thing it sends, whoever asked to close."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=21)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    a.send(b"m" * 800)
    b.send(b"s" * 300)
    sim.run(until=150.0)
    assert sim.delivered(1) == b"m" * 800 and sim.delivered(0) == b"s" * 300
    if closing in ("master", "both"):
        a.disconnect()
    if closing in ("slave", "both"):
        b.disconnect()
    sim.run(until=sim.t + 120.0)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:closed"
    _ends_with_identifiers(sim, sent)
    assert [_kind(t) for t in sent[1][-1:]] == ["disc_ack"]
    assert [_kind(t) for t in sent[0][-2:]] == ["disc", "id"]
    # one identifier each, and none before the close
    assert [sum(t.identifier for t in sent[w]) for w in (0, 1)] == [1, 1]
    # the master's went after the slave's had ended
    slave_end = sim.radiated(1)[-1][1]
    assert sim.radiated(0)[-1][0] >= slave_end
    assert not a.identifier_owed and not b.identifier_owed
    # in time the closing exchange left it: listening would only wait out the other's hangover
    assert not sent[0][-1].listen_first


@pytest.mark.parametrize("air", ["2300", "500"])
def test_the_slave_starts_nothing_until_the_masters_identifier_is_over(air: str) -> None:
    """A probe the slave makes the moment its DISC_ACK and identifier are handed over waits for
    the master's identifier: nothing it starts on its own goes over it (ADR-0075)."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=30)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    a.disconnect()
    t = sim.t
    while b.state is not State.IDLE and t < 200.0:
        t += 0.05
        sim.run(until=t)
    b.probe("W4ODA")
    sim.run(until=sim.t + 120.0)
    kinds = [_kind(tx) for tx in sent[1]]
    assert kinds[-2:] == ["disc_ack", "data"], kinds
    master_id = next(
        span for tx, span in zip(sent[0], sim.radiated(0), strict=True) if not tx.frames
    )
    assert sim.radiated(1)[-1][0] >= master_id[1]
    assert sim.overlaps() == []


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_lost_disc_ack_is_answered_again_identifier_and_all(air: str) -> None:
    """The master missed the first DISC_ACK and said DISC again: the slave answers each one with
    its DISC_ACK and its identifier, so its identifier stays the last thing it sent."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=22, frame_snr_offset=_lost(GrantKind.DISC_ACK, 1))
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    a.disconnect()
    sim.run(until=sim.t + 150.0)
    assert sim.events(0)[-1] == "disconnected:closed"
    _ends_with_identifiers(sim, sent)
    assert [_kind(t) for t in sent[1]].count("disc_ack") == 2
    assert [_kind(t) for t in sent[0]].count("disc") == 2


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_master_that_gives_up_on_the_disc_ack_identifies_after_the_last_slot(air: str) -> None:
    """Every DISC_ACK lost: the master leaves after three DISCs and identifies once the last
    one's slot can no longer be in use — which covers the slave's identifier behind it — and,
    having read nothing of the other station at the end, listens first."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=23, frame_snr_offset=_lost(GrantKind.DISC_ACK))
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    a.disconnect()
    sim.run(until=sim.t + 200.0)
    assert sim.events(0)[-1] == "disconnected:closed (no DISC_ACK)"
    _ends_with_identifiers(sim, sent)
    assert [_kind(t) for t in sent[1]].count("disc_ack") == 3
    assert sent[0][-1].listen_first and _kind(sent[0][-1]) == "id"


@pytest.mark.parametrize("air", ["2300", "500"])
def test_an_aborting_master_identifies_once_it_has_left(air: str) -> None:
    """ADR-0052's DISC on the floor, answered by the slave's DISC_ACK and identifier; the
    master's identifier after it."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=24)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    a.send(b"q" * 4000)
    sim.run(until=30.0)
    a.abort()
    sim.run(until=sim.t + 150.0)
    assert b.state is State.IDLE
    _ends_with_identifiers(sim, sent)


@pytest.mark.parametrize("air", ["2300", "500"])
def test_an_aborting_slave_is_closed_by_the_master_so_both_identify(air: str) -> None:
    """The slave left: it answers the next header with a DISC, the master closes with a DISC of
    its own, and the slave's DISC_ACK carries its identifier — both stations' identifiers come
    last, in time both can count on."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=25)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    a.send(b"q" * 4000)
    sim.run(until=30.0)
    b.abort()
    sim.run(until=sim.t + 150.0)
    assert a.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:peer disconnected"
    _ends_with_identifiers(sim, sent)
    kinds = [_kind(t) for t in sent[1]]
    assert kinds[-2:] == ["disc", "disc_ack"]


@pytest.mark.parametrize("air", ["2300", "500"])
def test_a_dead_path_ends_with_each_identifier_on_its_own(air: str) -> None:
    """The path dies mid-session: the master ends "no response" and identifies after its last
    slot; the slave times out and identifies on its own. Each listens first (ADR-0075): the
    other may have given up first and be sending its own."""
    a, b = _pair(AIRS[air], identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_schedule=lambda t: 8.0 if t < 30.0 else -60.0, seed=26)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    a.send(b"z" * 6000)
    sim.run(until=1200.0)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.events(0)[-1] == "disconnected:no response"
    assert sim.events(1)[-1] == "disconnected:link timeout"
    _ends_with_identifiers(sim, sent)
    assert sent[0][-1].listen_first and _kind(sent[0][-1]) == "id"
    assert sent[1][-1].listen_first and _kind(sent[1][-1]) == "id"


@pytest.mark.parametrize("air", ["2300", "500"])
@pytest.mark.parametrize("close_at", [60.0, 140.0])
def test_a_band_that_closes_ends_with_the_identifiers_apart(air: str, close_at: float) -> None:
    """The band closes under a transfer (35 dB in 20 s): the master keeps trying until "no
    response" and the slave times out. A timed-out slave cannot hear the master, so it waits
    until the master can have given up and identified: an exchange and the master's declared
    identifier (ADR-0075). Sent at once, it went over the master's last tries in 24 of 96 such
    sessions, and in the harness's `80m-band-closes-2300` the two identifiers met 0.6 s apart."""
    a, b = _pair(AIRS[air], identifier_s=4.5)
    sim = TwoStationSim(
        a,
        b,
        snr_schedule=lambda t: 10.0 if t < close_at else 10 - 35 * min(1, (t - close_at) / 20),
        seed=41,
    )
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    a.send(b"z" * 16000)
    sim.run(until=1500.0)
    assert sim.events(1)[-1] == "disconnected:link timeout"
    _ends_with_identifiers(sim, sent)
    assert sim.overlaps() == []
    assert sim.radiated(1)[-1][0] > sim.radiated(0)[-1][1]


def test_a_peer_that_calls_again_gets_no_identifier_between_its_sessions() -> None:
    """ADR-0055 §4 kept: a session the caller ended by calling again goes on in the new one — no
    identifier from the called station between them, and the caller's own, owed for the old
    session, is not sent when it calls the same station back."""
    a, b = _pair(WIDE, identifier_s=5.5)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=27)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    a.abort()
    a._left = None  # say nothing about leaving: the call is the only news
    a._disarm("leave")
    a.connect("KK4XYZ")
    sim.run(until=sim.t + 60.0)
    assert any(e == "disconnected:W4ODA called again" for e in sim.events(1))
    assert a.state is State.CONNECTED and b.state is State.CONNECTED
    assert not any(t.identifier for t in sent[0] + sent[1])
    a.disconnect()
    sim.run(until=sim.t + 120.0)
    _ends_with_identifiers(sim, sent)


def test_a_station_that_sends_no_identifier_ends_as_before() -> None:
    """A station that declared none (a non-US callsign with the setting off) ends with its
    DISC_ACK; the other's identifier still comes last of all."""
    a, b = _pair(WIDE, identifier_s=5.5)
    b.set_identifiers({"KK4XYZ": 0.0})
    sim = TwoStationSim(a, b, snr_db=10.0, seed=28)
    sent = _on_air(sim)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    a.disconnect()
    sim.run(until=sim.t + 120.0)
    assert [_kind(t) for t in sent[1][-1:]] == ["disc_ack"]
    assert not any(t.identifier for t in sent[1])
    assert sent[0][-1].identifier and _kind(sent[0][-1]) == "id"
    assert sim.overlaps() == []


def test_the_identifier_declared_is_the_callsign_the_session_runs_under() -> None:
    """The identifier sends the session's callsign, so its length — and whether there is one —
    follows it: a call made as another of the station's callsigns declares that one's."""
    a, b = _pair(WIDE)
    a.set_callsigns(["W4ODA", "W4ODA-12"])
    a.set_identifiers({"W4ODA": 4.0, "W4ODA-12": 6.5})
    calls: list[ConnectBody] = []
    sim = TwoStationSim(a, b, snr_db=10.0, seed=29)
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        frames = tx.frames  # type: ignore[attr-defined]
        if who == 0 and frames and frames[0].container is Container.DATA:
            body = frames[0].payload
            if body[0] >> 5 == 1:
                from aether_model.link.frames import decode_data

                calls.append(ConnectBody.decode(decode_data(body)[1]))
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    a.connect("KK4XYZ", as_call="W4ODA-12")
    sim.run(until=20.0)
    assert calls and calls[0].identifier_s == 6.5
    assert b._peer_identifier_s == 6.5


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


def test_a_transmission_no_frame_of_which_can_be_read_alone_keeps_its_header() -> None:
    """ND1J's Tests of 2026-10-15: frames that had failed went again at RV 2 and 3 without their
    header, and the slave, which believes a count only from a copy that decoded alone (R2), could
    grant itself nothing: eight silent slots. R7 now leaves a header out only when an RV 0 copy
    goes with it (ADR-0073). Here the master's data fades out mid-transfer, as a path does, and
    the frames on the air when it went are sent again at their rung until re-encoded."""
    a, b = _pair(NARROW)
    fade = {"on": True}

    def faded(frame: TxFrame) -> float:
        # the session's data only: the call and every control frame get through
        if not fade["on"] or frame.container is not Container.DATA:
            return 0.0
        return -30.0 if decode_data(frame.payload)[0].kind is DataKind.DATA else 0.0

    sim = TwoStationSim(a, b, snr_db=8.0, seed=21, frame_snr_offset=faded)
    sent: list[list[TxFrame]] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: object) -> None:
        if who == 0 and isinstance(tx, SlotTransmit):
            sent.append(list(tx.frames))
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    # a few frames, all of them on the air when the path goes: nothing new to send beside them
    message = _message(random.Random(21), 100)
    a.connect("KK4XYZ")
    a.send(message)
    sim.run(until=150.0)
    fade["on"] = False
    sim.run(until=sim.t + 300.0)
    copies = [
        tx
        for tx in sent
        if any(f.container is Container.DATA for f in tx)
        and all(f.rv not in READABLE_ALONE_RVS for f in tx if f.container is Container.DATA)
    ]
    assert copies, "no frame was sent again past its RV 0 copies"
    assert all(tx[0].container is Container.CONTROL for tx in copies)
    # what R7 still leaves out is a transmission with a copy the slave can read a count from
    assert a.stats.headers_elided > 0
    assert sim.delivered(1) == message
    assert sim.overlaps() == []


def test_a_stranded_frame_goes_at_the_most_robust_rung_of_its_new_family() -> None:
    """80m-agc-carrier-outside-500 on ITU NVIS: frames stranded at the first OFDM rung were
    re-encoded at the recommendation, the floor's top rung — tone4x100-93, whose 10 ms symbols a
    7 ms echo smears — and spent four more copies there, 86 s of a 300 s budget, where tone-36
    carried them at once. Every data frame of a family is as long as the others, so the most
    robust rung the body fits costs the frame no air (ADR-0085)."""
    a, _ = _pair(NARROW)
    timing = a.timing

    def body(mode: int) -> int:
        return data_capacity(timing.capacity(mode))

    floor_top, first_ordinary = timing.floor_modes - 1, timing.floor_modes
    # a full first-OFDM-rung frame, the recommendation at the floor's top: tone-36, not rung 4
    target = a._reencode_target(body(first_ordinary), first_ordinary, floor_top)
    assert target == 1
    assert timing.data_frame_s_for(target) == timing.data_frame_s_for(floor_top)
    # among the OFDM rungs, as low as the body fits, whatever rung was recommended
    assert a._reencode_target(body(first_ordinary), 9, 7) == first_ordinary
    # never into the floor, five times as long, while the recommendation is an OFDM rung
    assert a._reencode_target(body(9), 9, 7) is None
    # and on the wide air the same: tone-36 under the fast tones, and among the OFDM rungs the
    # lowest on the usable ladder (BPSK 1/5, rung 6, is beaten on both counts and is not)
    w, _ = _pair(WIDE)
    assert w._reencode_target(body_len=20, from_mode=w.timing.floor_modes, to_mode=4) == 1
    assert w.timing.data_frame_s_for(1) == w.timing.data_frame_s_for(4)
    assert w._reencode_target(body_len=20, from_mode=9, to_mode=7) == 7


def test_a_body_that_fits_no_rung_up_to_the_recommendation_goes_in_the_fewest_pieces() -> None:
    """A stranded frame is re-encoded whole only at a rung up to the recommendation: one above
    it strands it again. A body that fits none goes in pieces at the rung up to it, in its
    family, that cuts it into the fewest — every frame of a family is as long as the others, so
    the pieces are the air — and of those the most robust (ADR-0086)."""
    a, _ = _pair(NARROW)
    timing = a.timing

    def body(mode: int) -> int:
        return data_capacity(timing.capacity(mode))

    # 72 bytes at rung 3 under a recommendation of rung 2: nothing below it fits them whole;
    # two pieces at rung 2, three at rung 1
    assert a._reencode_target(body(3), 3, 2) is None
    assert a._split_rung(body(3), 3, 2) == 2
    # 50 bytes of an OFDM rung fit rung 3 whole, above the recommendation; two pieces at rung 2
    # and two at rung 1, which is the more robust
    assert a._reencode_target(body(9), 9, 2) is None
    assert a._split_rung(body(9), 9, 2) == 1
    # under an OFDM recommendation among the OFDM rungs: 106 bytes are four pieces at rungs 6
    # and 7, six at rung 5; below rung 0, nowhere
    assert a._split_rung(body(12), 12, 7) == 6
    assert a._split_rung(10, 0, 0) is None


def test_a_stuck_frame_goes_in_pieces_and_its_block_arrives() -> None:
    """80m-agc-carrier-outside-500, seed 31, on CI's runner: ITU NVIS at 500 Hz carried the
    floor's rungs 0-2 and nothing faster, and frames stranded at rung 3 with 72 bytes, or at an
    OFDM rung with 50, fit no rung below: they went on at their own rung until the session timed
    out. Now such a frame goes in pieces at the recommendation, in transmissions of their own,
    each at RV 0, and its sequence number is acknowledged once all of it is in. Here the path
    loses a block's first piece too, at any rung but the slowest: the block is split again,
    slower and at other boundaries, and still arrives (ADR-0086)."""
    a, b = _pair(NARROW)

    def refused(frame: SimFrame) -> float:
        if frame.container is not Container.DATA:
            return 0.0
        header = decode_data(frame.payload)[0]
        first_piece = header.fragment and header.offset == 0 and frame.mode >= 1
        return -40.0 if frame.mode >= 3 or first_piece else 0.0

    sim = TwoStationSim(a, b, snr_db=10.0, seed=30, frame_snr_offset=refused)
    sent = _spy_data(sim, 0)
    message = bytes(random.Random(1).randrange(256) for _ in range(1000))
    a.connect("KK4XYZ")
    a.send(message)
    while sim.delivered(1) != message and sim.t < 900.0:
        sim.run(until=sim.t + 5.0)
    assert sim.delivered(1) == message
    pieced = [tx for tx in sent if any(decode_data(f.payload)[0].fragment for f in tx)]
    assert pieced
    rungs: dict[int, set[int]] = {}
    for tx in pieced:
        assert all(decode_data(f.payload)[0].fragment for f in tx)  # pieces go alone
        assert all(f.rv == 0 for f in tx)  # and are never combined
        for f in tx:
            rungs.setdefault(decode_data(f.payload)[0].seq, set()).add(f.mode)
    assert any(len(modes) > 1 for modes in rungs.values())  # split again, and made whole
    assert sim.overlaps() == []


def test_a_frame_goes_in_pieces_only_after_the_copy_its_receiver_combines_last() -> None:
    """The receiver tries a frame's fifth copy with the four it holds before it starts over: a
    frame split after four leaves that behind, and on 80m-agc-carrier-outside-500 a rung-4
    frame whose fifth copy would have decoded never got it (ADR-0086). Here no report says the
    rungs refused are out of combining's reach (``reencode_hopeless_db`` off), so every block
    goes whole at its last rung one copy past ``max_combines`` before it goes in pieces."""
    a, b = _pair(NARROW, reencode_hopeless_db=100.0)

    def refused(frame: SimFrame) -> float:
        return -40.0 if frame.container is Container.DATA and frame.mode >= 3 else 0.0

    sim = TwoStationSim(a, b, snr_db=10.0, seed=30, frame_snr_offset=refused)
    sent = _spy_data(sim, 0)
    message = bytes(random.Random(1).randrange(256) for _ in range(1000))
    a.connect("KK4XYZ")
    a.send(message)
    while sim.delivered(1) != message and sim.t < 900.0:
        sim.run(until=sim.t + 5.0)
    assert sim.delivered(1) == message
    copies: dict[int, list[tuple[int, bool]]] = {}
    for tx in sent:
        for f in tx:
            header = decode_data(f.payload)[0]
            if header.kind is DataKind.DATA:
                copies.setdefault(header.seq, []).append((f.mode, header.fragment))
    split = {seq: c for seq, c in copies.items() if any(piece for _, piece in c)}
    assert split
    for seq, history in split.items():
        whole = [mode for mode, piece in history[: [p for _, p in history].index(True)]]
        at_last_rung = next(
            (i for i, mode in enumerate(reversed(whole)) if mode != whole[-1]), len(whole)
        )
        assert at_last_rung >= a.cfg.max_combines + 1, (seq, history)
    assert sim.overlaps() == []


def test_pieces_of_two_splits_make_their_block() -> None:
    """The receiver places pieces by their offsets: a block split at one rung and again at a
    slower one, a piece of each heard, is whole once every byte of it is (ADR-0086)."""
    block = bytes(range(72))
    held = granted_module._Pieces()
    for offset, piece, last in pieces(block, 51)[:1]:  # rung 2's first piece, 46 bytes
        held.place(offset, piece, last)
    assert held.body() is None
    for offset, piece, last in pieces(block, 24)[2:]:  # rung 0's pieces from byte 38 on
        held.place(offset, piece, last)
    assert held.body() == block


def _spy_data(sim: TwoStationSim, who: int) -> list[list[TxFrame]]:
    """Every transmission of station ``who``'s that carries data frames, as launched."""
    sent: list[list[TxFrame]] = []
    orig = sim._launch

    def spy(station: int, at: float, tx: object) -> None:
        if station == who and isinstance(tx, SlotTransmit):
            data = [f for f in tx.frames if f.container is Container.DATA]
            if data:
                sent.append(data)
        orig(station, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    return sent


def _data_kind(frame: TxFrame) -> DataKind:
    return decode_data(frame.payload)[0].kind


def test_a_measurement_frame_goes_once_and_its_losses_cost_nothing() -> None:
    """The Test's ladder had pinned the session's own data at each rung, so a frame that failed
    there was data still to deliver: on WC4Y's Test of 2026-10-17 the leftovers took 93 s of 435
    (13-wc4y-beta111.md §10). A measurement frame takes no sequence number: sent once, alone in
    its transmission, reported, never sent again — and the data queued behind it waits for the
    report, then goes as if there had been none (ADR-0077)."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=-5.0, seed=22)
    sent = _spy_data(sim, 0)
    a.connect("KK4XYZ")
    sim.run(until=60.0)
    measured = 9
    a.measure(measured, 4)
    message = _message(random.Random(22), 40)
    a.send(message)
    sim.run(until=sim.t + 300.0)
    rungs = a.take_ladder()
    assert [r.mode for r in rungs] == [measured] and rungs[0].decoded < rungs[0].frames
    with_measurement = [tx for tx in sent if any(_data_kind(f) is DataKind.MEASURE for f in tx)]
    assert len(with_measurement) == 1, "a measurement is sent once"
    assert all(_data_kind(f) is DataKind.MEASURE and f.rv == 0 for f in with_measurement[0])
    assert [decode_data(f.payload)[0].seq for f in with_measurement[0]] == list(range(4))
    first = sent.index(with_measurement[0])
    assert all(_data_kind(f) is DataKind.DATA for tx in sent[first + 1 :] for f in tx)
    assert sim.delivered(1) == message, "the data behind it, and nothing of it"
    assert sim.overlaps() == []


def test_a_measurement_leaves_nothing_to_combine() -> None:
    """A measurement frame that does not decode is never sent again, and the data that follows
    takes the sequence numbers a receiver would infer for it: a buffer kept for one would be
    combined with another codeword. The receiver keeps none (ADR-0077)."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=4.0, seed=23)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    failed = b.stats.frames_failed
    a.measure(WIDE.n_rungs - 1, 4)
    sim.run(until=sim.t + 60.0)
    rungs = a.take_ladder()
    assert rungs and rungs[0].decoded == 0
    assert b.stats.frames_failed >= failed + rungs[0].frames
    assert b._harq == {}


def test_an_unread_measurement_leaves_nothing_to_combine() -> None:
    """Its header lost and its frames failed, a measurement looks like data to the slave, which
    keeps the frames to combine under the sequence numbers the master's next data takes. The
    master's poll says it still waits for the report, and the slave drops them before any data
    comes (ADR-0077)."""
    a, b = _pair(WIDE)
    masters: set[int] = set()
    alive: list[TxFrame] = []  # so no other frame is given one of their ids
    lost = [False]

    def offset(frame: TxFrame) -> float:
        if id(frame) not in masters:
            return 0.0
        if frame.container is Container.CONTROL:
            ctl = GrantFrame.decode(frame.payload)
            if not lost[0] and ctl.kind is GrantKind.HEADER and ctl.measure and ctl.count:
                lost[0] = True
                return -60.0
            return 0.0
        return -8.0 if _data_kind(frame) is DataKind.MEASURE else 0.0

    sim = TwoStationSim(a, b, snr_db=10.0, seed=55, frame_snr_offset=offset)
    launch = sim._launch

    def launched(who: int, at: float, tx: object) -> None:
        if who == 0:
            alive.extend(tx.frames)  # type: ignore[attr-defined]
            masters.update(id(f) for f in tx.frames)  # type: ignore[attr-defined]
        launch(who, at, tx)  # type: ignore[arg-type]

    sim._launch = launched  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    a.send(_message(random.Random(55), 600))
    sim.run(until=60.0)
    b.send(_message(random.Random(56), 6000))  # the slave keeps answering
    sim.run(until=75.0)
    a.measure(a.current_mode, 4)
    kept = False
    t = sim.t
    while a.measuring and t < 200.0:
        t += 0.1
        sim.run(until=t)
        kept = kept or bool(b._harq)
    assert lost[0] and kept, "the unread frames were kept to combine"
    assert not a.measuring and a.take_ladder()
    assert b._harq == {}, "and dropped before the master's data comes"


def test_data_waits_for_the_measurements_report() -> None:
    """The report rides in the bitmap the other station's acknowledgements would carry for
    this station's data, so nothing of it goes until the report is read (ADR-0077)."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=24)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    a.measure(10, 3)
    a.send(b"d" * 300)
    sent = _spy_data(sim, 0)
    t = sim.t
    while a.measuring and t < 120.0:
        t += 0.5
        sim.run(until=t)
        assert all(_data_kind(f) is DataKind.MEASURE for tx in sent for f in tx)
    assert not a.measuring and a.take_ladder()
    sim.run(until=sim.t + 60.0)
    assert sim.delivered(1) == b"d" * 300


def test_the_slave_measures_too() -> None:
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=25)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    b.measure(10, 3)
    b.send(b"s" * 200)
    sim.run(until=sim.t + 90.0)
    rungs = b.take_ladder()
    assert [(r.mode, r.frames, r.decoded) for r in rungs] == [(10, 3, 3)]
    assert sim.delivered(0) == b"s" * 200
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


def _ladder_rung(
    rung: int,
    *,
    snr_db: float,
    seed: int,
    pinned: bool = False,
    lose: Callable[[TxFrame], float] | None = None,
    **cfg: object,
) -> tuple[GrantedEngine, list[list[TxFrame]], int, list[object]]:
    """A session on the 2 300 Hz air, then one rung of the Test's ladder at ``rung`` — measured,
    or with ``pinned`` the session's own data pinned there, as the ladder did before ADR-0077:
    the master, its transmissions since, the silent slots they drew, and what the ladder
    reported. ``lose``: decibels off a frame, as the simulator's ``frame_snr_offset``."""
    a, b = _pair(WIDE, **cfg)
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=seed, frame_snr_offset=lose)
    sent: list[list[TxFrame]] = []
    orig = sim._launch
    recording = [False]

    def spy(who: int, at: float, tx: object) -> None:
        if who == 0 and recording[0]:
            sent.append(list(tx.frames))  # type: ignore[attr-defined]
        orig(who, at, tx)  # type: ignore[arg-type]

    sim._launch = spy  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    a.send(b"m" * 600)
    sim.run(until=90.0)
    assert sim.delivered(1) == b"m" * 600
    silent = a.stats.silent_slots
    recording[0] = True
    if pinned:
        a.pin_mode(rung, body_bytes=10)
        a.send(b"L" * 40)
    else:
        a.measure(rung, 4)
    sim.run(until=sim.t + 60.0)
    rungs: list[object] = list(a.take_ladder())
    a.pin_mode(None)
    return a, sent, a.stats.silent_slots - silent, rungs


def _at(sent: list[list[TxFrame]], rung: int) -> list[list[TxFrame]]:
    return [tx for tx in sent if any(f.mode == rung for f in tx if f.container is Container.DATA)]


def test_a_pinned_transmission_above_the_recommendation_keeps_a_cheap_header() -> None:
    """WC4Y's Test of 2026-10-17 pinned rungs 8 and 9 above the rung asked for, without their
    header; none of their frames decoded, so each transmission granted nothing — a wait and a
    floor poll — where an ordinary header, 0.43 s, would have been read (ADR-0076)."""
    pinned = WIDE.n_rungs - 1
    _, sent, silent, _ = _ladder_rung(pinned, snr_db=6.0, seed=31, pinned=True)
    with_pinned = _at(sent, pinned)
    assert with_pinned, "the pinned rung went out"
    assert with_pinned[0][0].container is Container.CONTROL and not with_pinned[0][0].floor
    assert silent == 0
    _, sent, silent, _ = _ladder_rung(
        pinned, snr_db=6.0, seed=31, pinned=True, header_above_recommendation=False
    )
    assert _at(sent, pinned)[0][0].container is Container.DATA, "elided without the rule"
    assert silent >= 1


def test_a_measurement_keeps_its_header_whatever_the_rule() -> None:
    """Its header says the frames are measurement frames, so it always goes: what came of the
    frames is reported in the measurement's own slot, with the rung's SNR."""
    top = WIDE.n_rungs - 1
    for rule in (True, False):
        _, sent, silent, rungs = _ladder_rung(
            top, snr_db=6.0, seed=31, header_above_recommendation=rule
        )
        measured = _at(sent, top)
        assert len(measured) == 1 and measured[0][0].container is Container.CONTROL
        assert GrantFrame.decode(measured[0][0].payload).measure
        assert silent == 0
        assert rungs and rungs[0].decoded < rungs[0].frames  # type: ignore[attr-defined]
        assert rungs[0].snr_db is not None  # type: ignore[attr-defined]


def test_a_ladder_rung_whose_answer_was_lost_is_measured_again() -> None:
    """The measurement's answer lost, its slot is silent. Until 2026-10-23 the master polled and
    took the answer's report with no SNR (ADR-0076): the report was the measurement's, but the
    SNR the poll's. A silent slot cannot tell that case from a slave that read nothing of the
    measurement and reports the one before (15-wc4y-2300-beta120.md §6), so the rung is
    measured again, and reported from the repeat with the rung's own SNR."""
    lost = [False, False]  # a measurement frame launched; its answer lost

    def lose(frame: TxFrame) -> float:
        if frame.container is Container.DATA and _data_kind(frame) is DataKind.MEASURE:
            lost[0] = True
        elif (
            frame.container is Container.CONTROL
            and lost[0]
            and not lost[1]
            and GrantFrame.decode(frame.payload).kind is GrantKind.ANSWER
        ):
            lost[1] = True
            return -60.0
        return 0.0

    _, sent, silent, rungs = _ladder_rung(10, snr_db=12.0, seed=32, lose=lose)
    assert lost == [True, True] and silent >= 1
    assert len(_at(sent, 10)) == 2, "measured again"
    assert len(rungs) == 1, "one rung, reported once"
    assert rungs[0].decoded == rungs[0].frames == 4  # type: ignore[attr-defined]
    assert rungs[0].snr_db is not None  # type: ignore[attr-defined]


def _two_rungs(lose: Callable[[TxFrame], float]) -> tuple[GrantedEngine, list[LadderRung]]:
    """Rung 8 measured, then rung 12, on the 2 300 Hz air at 14 dB with ``lose``."""
    a, b = _pair(WIDE)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=5, frame_snr_offset=lose)
    a.connect("KK4XYZ")
    sim.run(until=20.0)
    rungs: list[LadderRung] = []
    for mode in (8, 12):
        a.measure(mode, 4)
        sim.run(until=sim.t + 90.0)
        assert not a.measuring
        rungs += a.take_ladder()
    assert a.state is State.CONNECTED
    assert sim.overlaps() == []
    return a, rungs


def _measurement_headers(lost: Callable[[int], bool]) -> Callable[[TxFrame], float]:
    """Rung 12's measurement frames never decode; the measurement headers ``lost`` says, by
    their order (rung 8's is the first), go unread."""
    seen = [0]

    def lose(frame: TxFrame) -> float:
        if frame.container is Container.DATA and frame.mode == 12:
            return -60.0
        if frame.container is Container.CONTROL:
            ctl = GrantFrame.decode(frame.payload)
            if ctl.kind is GrantKind.HEADER and ctl.measure and ctl.count > 0:
                seen[0] += 1
                return -60.0 if lost(seen[0]) else 0.0
        return 0.0

    return lose


def test_a_rung_the_slave_never_read_is_not_credited_with_the_rung_before() -> None:
    """WC4Y's Test of 2026-10-23: KK4ODA-1 missed rung 9's header and decoded none of its
    frames, the slot went silent, and the next answer repeated rung 8's report, which the master
    took for rung 9's — "3/4 decoded" for a rung that carried nothing, the Test's highest passed
    one too high and the ladder a rung too long (15-wc4y-2300-beta120.md §6). The rung is
    measured again, and its own report is what the ladder records."""
    _, rungs = _two_rungs(_measurement_headers(lambda n: n == 2))
    assert [(r.mode, r.frames, r.decoded) for r in rungs] == [(8, 4, 4), (12, 4, 0)]
    assert rungs[1].snr_db is not None, "the repeat's own report, with the rung's SNR"


def test_a_measurement_silent_twice_is_recorded_as_nothing_decoded() -> None:
    """Measured again and silent again: nothing came back from the rung twice, and whatever
    report follows may be another rung's. The rung is recorded with nothing decoded and no
    SNR, and the session goes on."""
    _, rungs = _two_rungs(_measurement_headers(lambda n: n >= 2))
    assert [(r.mode, r.frames, r.decoded, r.snr_db) for r in rungs] == [
        (8, 4, 4, rungs[0].snr_db),
        (12, 4, 0, None),
    ]


def test_a_disconnect_that_drains_nothing_leaves_at_the_next_opportunity() -> None:
    """The Test's last queued bytes are its ladder's filler, which nobody reads: it closes with
    no drain, and the master says what it left (ADR-0076)."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=-2.0, seed=33)
    message = _message(random.Random(33), 2000)
    a.connect("KK4XYZ")
    a.send(message)
    sim.run(until=40.0)
    assert a.state is State.CONNECTED and 0 < len(sim.delivered(1)) < len(message)
    asked = sim.t
    a.disconnect(drain_s=0.0)
    assert a.leave_by == pytest.approx(asked)
    sim.run(until=asked + 40.0)
    assert a.state is State.IDLE and b.state is State.IDLE
    assert len(sim.delivered(1)) < len(message)
    left = [e for e in sim.events(0) if e.startswith("disconnect:")]
    assert left and "bytes not delivered" in left[0], left
    # at the next opportunity: the slot of what was on the air when it was asked, no longer
    waited = float(left[0].split("closing ")[1].split(" s after")[0])
    assert waited < 15.0, left
    assert sim.overlaps() == []


def test_a_session_on_an_echoing_path_starts_below_the_rungs_the_echo_breaks() -> None:
    """ITU's disturbed NVIS (7 ms) at 500 Hz: the floor carries, rungs 3 and 4 and the OFDM
    rungs do not, and the floor's SNR reading sits near -3 dB whatever the SNR — what a quiet
    path at -3 dB reads, where rung 4 is the right start. The caller's first data went there and
    decoded only on its third copy. The floor's own frames read the path's echo: with it, the
    call's acceptance keeps the session below the rungs the echo breaks from its first burst
    on, and the message arrives sooner. Without it, the hazard (ADR-0087)."""

    def refused(frame: SimFrame) -> float:
        return -40.0 if frame.container is Container.DATA and frame.mode >= 3 else 0.0

    message = bytes(random.Random(5).randrange(256) for _ in range(1000))
    took = {}
    for echo in (None, 0.01):
        a, b = _pair(NARROW)
        sim = TwoStationSim(a, b, snr_db=-3.0, seed=8, frame_snr_offset=refused, echo=echo)
        sent = _spy_data(sim, 0)
        a.connect("KK4XYZ")
        a.send(message)
        while sim.delivered(1) != message and sim.t < 1200.0:
            sim.run(until=sim.t + 5.0)
        assert sim.delivered(1) == message
        took[echo] = sim.t
        data = [[f for f in tx if decode_data(f.payload)[0].kind is DataKind.DATA] for tx in sent]
        data = [tx for tx in data if tx]
        if echo is None:
            assert data[0][0].mode >= 3
        else:
            assert max(f.mode for tx in data for f in tx) <= 2
    assert took[0.01] < took[None]
