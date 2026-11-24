"""Hand-overs and endings (ADR-0135), from three Winlink Express P2P sessions on 2026-11-24
(``docs/research/link-architecture/21-wle-p2p-40m-beta174.md``):

* **the slave's wants, made exact**: its answer counts the frames it sends in that very
  transmission among those it still has, and the master no longer grants them again once they
  have arrived;
* **the DISC carries the acknowledgement**, said by its ``flag``, so a slave that asks to leave
  with its last bytes is not kept one exchange more;
* **a drain that cannot finish gives up**: a disconnect whose queue could not go within the
  drain at the session's rate leaves it at once;
* **a hand-over waits for the program's reply**: a program answers 0.17–0.26 s after it is
  handed data, which used to be after the station had decided its next transmission.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import replace

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.frames import GrantFrame, GrantKind
from aether_model.link.granted import GrantConfig, GrantedEngine, State, Transmit
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container, TxFrame
from aether_model.link.sim import TwoStationSim


def _pair(air: AirInterface, **cfg: object) -> tuple[GrantedEngine, GrantedEngine]:
    timing = replace(phy_timing(air.params), tx_latency_s=0.0)

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


def _spy(sim: TwoStationSim) -> list[tuple[int, float, list[TxFrame]]]:
    """Every transmission launched: who, when, its frames."""
    sent: list[tuple[int, float, list[TxFrame]]] = []
    orig = sim._launch

    def spy(who: int, at: float, tx: Transmit) -> None:
        sent.append((who, at, list(tx.frames)))
        orig(who, at, tx)

    sim._launch = spy  # type: ignore[method-assign]
    return sent


def _control(frames: list[TxFrame]) -> GrantFrame | None:
    for f in frames:
        if f.container is Container.CONTROL:
            try:
                return GrantFrame.decode(f.payload)
            except ValueError:
                return None
    return None


def _connect(sim: TwoStationSim, a: GrantedEngine, b: GrantedEngine, until: float = 60.0) -> None:
    a.connect(b.my_call)
    t = 0.0
    while t < until and not (a.state is State.CONNECTED and b.state is State.CONNECTED):
        t += 1.0
        sim.run(until=t)
    assert a.state is State.CONNECTED and b.state is State.CONNECTED


def _run_until(sim: TwoStationSim, done: Callable[[], bool], until: float) -> float | None:
    t = sim.t
    while t < until:
        t += 0.5
        sim.run(until=t)
        if done():
            return sim.t
    return None


# ── the slave's wants ──────────────────────────────────────────────────


def test_the_slaves_last_data_is_not_granted_again() -> None:
    """Session 2 of 2026-11-24: home (the slave) sent its message's last four frames, all
    decoded, and the portable's next header granted it four frames again; home answered with
    nothing, a whole exchange. Its answer had counted those four frames, sent in that very
    transmission, among the ones it still had."""
    a, b = _pair(NARROW, reply_wait_s=0.0)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=3)
    _connect(sim, a, b)
    sent = _spy(sim)
    message = bytes(range(256)) * 2
    b.send(message)
    assert _run_until(sim, lambda: sim.delivered(0) == message, 200.0) is not None
    delivered_at = sim.t
    sim.run(until=sim.t + 15.0)
    headers = [
        ctl
        for who, at, frames in sent
        if who == 0 and at >= delivered_at - 1.0
        if (ctl := _control(frames)) is not None and ctl.kind is GrantKind.HEADER
    ]
    assert headers, "the master went on"
    first = headers[0]
    assert first.flag or first.n == 0, f"granted the delivered frames again: {first}"


# ── the DISC's acknowledgement ─────────────────────────────────────────


def _slave_leaves_with_fq(**cfg: object) -> tuple[TwoStationSim, GrantedEngine, list]:
    a, b = _pair(NARROW, **cfg)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=5)
    _connect(sim, a, b)
    sent = _spy(sim)
    b.send(b"FQ\r")
    b.disconnect()
    sim.run(until=sim.t + 60.0)
    return sim, b, sent


def test_a_disc_acknowledges_the_slaves_last_bytes() -> None:
    """Sessions 1 and 2: Winlink Express at home wrote FQ and asked to disconnect; the FQ went
    with the request to leave, and the portable sent a header acknowledging it before it could
    send its DISC — about 5 s at 500 Hz. The DISC now goes next, and acknowledges the FQ."""
    sim, b, sent = _slave_leaves_with_fq()
    assert sim.st[0].engine.state is State.IDLE and b.state is State.IDLE
    assert sim.delivered(0) == b"FQ\r"
    master = [ctl for who, _, frames in sent if who == 0 if (ctl := _control(frames)) is not None]
    kinds = [ctl.kind for ctl in master]
    assert GrantKind.DISC in kinds
    before = kinds[: kinds.index(GrantKind.DISC)]
    # the transmission granting the slave's answer, and then the DISC: no header in between
    assert before.count(GrantKind.HEADER) <= 1, kinds
    disc = master[kinds.index(GrantKind.DISC)]
    assert disc.flag, "the DISC does not say it acknowledges"
    assert b.stats.bytes_acked >= 3, "the FQ was left unacknowledged"
    assert not any("not delivered" in e for e in sim.events(1))


def test_a_disc_without_its_flag_acknowledges_nothing() -> None:
    """A DISC from beta.174 carries zero acknowledgement fields and no flag. Read as an
    acknowledgement, a base of 0 would acknowledge every sequence number half a window behind
    it, so it is not read at all: the slave's last bytes stay unacknowledged, as before."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=5)
    _connect(sim, a, b)
    orig = sim._launch

    def old_disc(who: int, at: float, tx: Transmit) -> None:
        frames = []
        for f in tx.frames:
            ctl = _control([f]) if f.container is Container.CONTROL else None
            if ctl is not None and ctl.kind is GrantKind.DISC:
                ctl = replace(ctl, flag=False, base=0, bitmap=0)
                f = replace(f, payload=ctl.encode())
            frames.append(f)
        orig(who, at, replace(tx, frames=frames))

    sim._launch = old_disc  # type: ignore[method-assign]
    b.send(b"FQ\r")
    b.disconnect()
    acked_before = b.stats.bytes_acked
    sim.run(until=sim.t + 60.0)
    assert b.state is State.IDLE
    assert sim.delivered(0) == b"FQ\r"
    assert b.stats.bytes_acked == acked_before, "an old DISC's zeros were read as acknowledging"


# ── a drain that cannot finish ─────────────────────────────────────────


def test_a_drain_that_cannot_finish_closes_at_once() -> None:
    """Session 3: Winlink Express gave up on a 52 kB file after five minutes, 43.7 kB still
    queued at 224 bit/s, and the drain sent on for 70 s. A queue that could not go within the
    drain at the session's rate is now left, and the session closes at its next opportunity."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=2.0, seed=7)
    _connect(sim, a, b)
    a.send(bytes(range(256)) * 80)  # 20 kB
    sim.run(until=sim.t + 90.0)
    assert a.state is State.CONNECTED
    asked = sim.t
    a.disconnect()
    ended = _run_until(sim, lambda: a.state is State.IDLE and b.state is State.IDLE, asked + 60.0)
    assert ended is not None and ended - asked < 30.0, "the drain went on"
    said = [e for e in sim.events(0) if e.startswith("disconnect:")]
    assert any("closing without them" in e for e in said), said
    assert len(sim.delivered(1)) < 20480


def test_a_queue_that_would_go_in_time_still_drains() -> None:
    """The FQ a program writes before it asks to leave still goes first (ADR-0072)."""
    a, b = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=7)
    _connect(sim, a, b)
    message = bytes(range(256)) * 8
    a.send(message)
    assert _run_until(sim, lambda: sim.delivered(1) == message, sim.t + 200.0) is not None
    a.send(b"FQ\r")
    a.disconnect()
    sim.run(until=sim.t + 60.0)
    assert sim.delivered(1) == message + b"FQ\r"
    assert not any("closing without them" in e for e in sim.events(0))


# ── the reply at a hand-over ───────────────────────────────────────────


def _reply_delay(replier: int, wait_s: float, reply_after: float = 0.4) -> float:
    """Station ``1 - replier`` sends a message; ``replier``'s program answers it ``reply_after``
    seconds after the simulator hands it the last of it. How long from that answer being written
    to its arrival at the other station. The simulator hands a frame over as it ends, where a
    station decodes it about 0.15 s later (08-steps-0-and-1.md §1.3): 0.4 s is the field's
    0.15 s and Winlink Express's 0.22–0.26 s."""
    a, b = _pair(NARROW, reply_wait_s=wait_s)
    message = bytes(range(200))
    reply = b"FF\r"
    written: list[float] = []
    arrived: list[float] = []
    got = {0: b"", 1: b""}

    def on_deliver(who: int, data: bytes, at: float) -> None:
        got[who] += data
        if who == replier and got[who] == message and not written:
            written.append(at + reply_after)
            sim.send_at(who, at + reply_after, reply)
        if who != replier and got[who] == reply:
            arrived.append(at)

    sim = TwoStationSim(a, b, snr_db=12.0, seed=9, on_deliver=on_deliver)
    _connect(sim, a, b)
    (b if replier == 0 else a).send(message)
    assert _run_until(sim, lambda: bool(arrived), sim.t + 200.0) is not None
    return arrived[0] - written[0]


def test_the_masters_program_reply_rides_its_next_transmission() -> None:
    """Session 2: the portable (the master) was handed home's message and its Winlink Express
    wrote FF 0.22 s later — just after the master had keyed a header with nothing in it. The
    master now holds its next transmission for the reply."""
    waited = _reply_delay(replier=0, wait_s=0.35)
    hurried = _reply_delay(replier=0, wait_s=0.0)
    # an exchange of ordinary control frames, here; on the floor's headers of 2026-11-24, 5 s
    assert waited + 1.0 < hurried, (waited, hurried)


def test_the_slaves_program_reply_rides_its_answer() -> None:
    """Sessions 1 and 2: home (the slave) was handed the portable's last data and its Winlink
    Express wrote FQ 0.17–0.26 s later, after home's answer had been decided. The last of the
    master's data now gives the answer a frame of room and a later slot, and the slave decides
    it at the last moment."""
    waited = _reply_delay(replier=1, wait_s=0.35)
    hurried = _reply_delay(replier=1, wait_s=0.0)
    assert waited + 1.0 < hurried, (waited, hurried)


def test_a_reply_that_does_not_come_costs_little() -> None:
    """A program with nothing to say: the master waits its reply time once, and an answer
    states its own length, so the room it was given and did not use costs nothing."""

    def session(wait_s: float) -> float:
        a, b = _pair(WIDE, reply_wait_s=wait_s)
        sim = TwoStationSim(a, b, snr_db=12.0, seed=13)
        _connect(sim, a, b)
        start = sim.t
        a.send(bytes(range(256)) * 4)
        b.send(bytes(range(256)) * 4)
        done = _run_until(
            sim, lambda: len(sim.delivered(0)) == 1024 and len(sim.delivered(1)) == 1024, 300.0
        )
        assert done is not None
        return done - start

    assert session(0.35) <= session(0.0) + 3.0
