"""Generate the trace vectors that check the port of protocol 8's engine (ADR-0069 step 2).

    python tools/make_granted_vectors.py [--out core/aether-link/tests/data/granted_vectors.json]

The model's :class:`~aether_model.link.granted.GrantedEngine` runs sessions through
:class:`~aether_model.link.sim.TwoStationSim` — clean and weak, both airs, fading, outages,
false detections, late decodes and late keying, aborts, probes, identifiers, the Test's ladder —
and every input each engine is handed is recorded: the commands, the ticks, every frame with
every decode the engine asked of it, and every action it drained. ``tests/granted_vectors.rs``
hands the Rust engine the same inputs, answers its decodes from the record, and compares every
action and, after every input, what the engine says of itself. The engine's random draws are
the same in both (``granted.Draws``), so nothing is left to chance.

Floats are written as Python writes them, the shortest text that reads back as the same double,
and the Rust side parses them exactly (``serde_json``'s ``float_roundtrip``): an input a hair off
could tip a comparison the engine makes to a microsecond. Every payload is written once, in a
table, and referred to by its place in it. Regenerate only for a deliberate change to the model
engine; a mismatch otherwise is a bug in the port. ``tools/check_vectors.py`` compares a fresh
file with the committed one within a float tolerance: the fading pipe and the station model's
latencies go through a C library's ``log`` and ``exp``, whose last bit differs between platforms.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
from collections.abc import Callable
from dataclasses import replace
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))

from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.engine import Deliver, Event, Transmit
from aether_model.link.fading import FadingPipe
from aether_model.link.frames import GrantFrame, GrantKind
from aether_model.link.granted import GrantConfig, GrantedEngine
from aether_model.link.harness import bandwidth_capabilities, phy_timing
from aether_model.link.phy import Container, PhyTiming, TxFrame
from aether_model.link.sim import TwoStationSim


def bits(x: float | None) -> float | None:
    """A float as the file holds it; ``None`` stays ``None``."""
    return None if x is None else float(x)


class Blobs:
    """Every payload once: a transmitted frame's bytes come back as the other station's decode,
    and a session's data as its delivery."""

    def __init__(self) -> None:
        self.table: list[str] = []
        self._index: dict[bytes, int] = {}

    def __call__(self, data: bytes) -> int:
        data = bytes(data)
        if data not in self._index:
            self._index[data] = len(self.table)
            self.table.append(data.hex())
        return self._index[data]


BLOBS = Blobs()


def timing_doc(t: PhyTiming) -> dict[str, Any]:
    caps = t.data_capacity or {}
    thresholds = t.mode_threshold_db or {}
    controls = t.control_threshold_db
    return {
        "data_frame_s": bits(t.data_frame_s),
        "control_frame_s": bits(t.control_frame_s),
        "turnaround_s": bits(t.turnaround_s),
        "detect_latency_s": bits(t.detect_latency_s),
        "tx_latency_s": bits(t.tx_latency_s),
        "preamble_detect_s": bits(t.preamble_detect_s),
        "data_capacity": [caps[m] for m in sorted(caps)],
        "mode_threshold_db": [bits(thresholds[m]) for m in sorted(thresholds)],
        "floor_data_frame_s": bits(t.floor_data_frame_s),
        "floor_control_frame_s": bits(t.floor_control_frame_s),
        "floor_modes": t.floor_modes,
        "control_threshold_db": None
        if controls is None
        else [bits(controls[False]), bits(controls[True])],
        "floor_margin_db": bits(t.floor_margin_db),
        "floor_preamble_detect_s": bits(t.floor_preamble_detect_s),
    }


def config_doc(c: GrantConfig) -> dict[str, Any]:
    if c.rate:
        raise ValueError("the vectors use the rate controller's defaults")
    return {
        "max_data_frames": c.max_data_frames,
        "max_burst_s": bits(c.max_burst_s),
        "max_retries": c.max_retries,
        "connect_retries": c.connect_retries,
        "disc_retries": c.disc_retries,
        "disc_patience_exchanges": bits(c.disc_patience_exchanges),
        "disc_patience_s": bits(c.disc_patience_s),
        "leave_repeats": c.leave_repeats,
        "floor_answer_margin_db": bits(c.floor_answer_margin_db),
        "answer_floor_margin_db": bits(c.answer_floor_margin_db),
        "family_readings": c.family_readings,
        "link_timeout_s": bits(c.link_timeout_s),
        "link_timeout_exchanges": bits(c.link_timeout_exchanges),
        "initial_mode": c.initial_mode,
        "max_mode": c.max_mode,
        "ceiling": c.ceiling,
        "silence_step": c.silence_step,
        "max_combines": c.max_combines,
        "reencode_early_after": c.reencode_early_after,
        "reencode_hopeless_db": bits(c.reencode_hopeless_db),
        "capabilities": c.capabilities,
        "first_grant": c.first_grant,
        "window_doublings": c.window_doublings,
        "elide": c.elide,
        "turnaround_s": bits(c.turnaround_s),
        "identifier_s": bits(c.identifier_s),
        "answer_gap_s": bits(c.answer_gap_s),
        "floor_answer_memory": c.floor_answer_memory,
        "decode_budget_s": [bits(x) for x in c.decode_budget_s],
        "key_s": bits(c.key_s),
        "guard_s": bits(c.guard_s),
        "slack_s": bits(c.slack_s),
        "levels_s": [bits(x) for x in c.levels_s],
        "open_window_s": [bits(x) for x in c.open_window_s],
        "keepalive_s": bits(c.keepalive_s),
        "silence_levels_after": c.silence_levels_after,
        "on_time_to_lower": c.on_time_to_lower,
        "need_ease_after": c.need_ease_after,
    }


def tx_frame_doc(f: TxFrame) -> dict[str, Any]:
    return {
        "control": f.container is Container.CONTROL,
        "payload": BLOBS(f.payload),
        "mode": f.mode,
        "rv": f.rv,
        "floor": f.floor,
        "follows": f.follows,
    }


def action_doc(a: object) -> dict[str, Any]:
    if isinstance(a, Transmit):
        return {
            "transmit": [tx_frame_doc(f) for f in a.frames],
            "duration_s": bits(a.duration_s),
            "cause": getattr(a, "cause", ""),
            "not_after": bits(getattr(a, "not_after", None)),
            "not_before": bits(getattr(a, "not_before", None)),
            "identifier_s": bits(getattr(a, "identifier_s", 0.0)),
        }
    if isinstance(a, Deliver):
        return {"deliver": BLOBS(a.data)}
    if isinstance(a, Event):
        return {"event": a.name, "detail": a.detail}
    raise TypeError(f"unknown action {a!r}")


def digest(e: GrantedEngine) -> dict[str, Any]:
    """What the engine says of itself — compared after every input, so a port that goes wrong
    is caught at the input it went wrong on."""
    return {
        "state": e.state.value,
        "role": e.role.value,
        "session": e.session,
        "level": e.level,
        "current_mode": e.current_mode,
        "tx_pending": e.tx_pending_bytes,
        "tx_undelivered": e.tx_undelivered_bytes,
        "next_deadline": bits(e.next_deadline()),
        "connect_tries": e.connect_tries,
        "probing": e.probing,
        "peer_snr_db": bits(e.peer_snr_db),
        "stats": dict(vars(e.stats)),
    }


class _Proxy:
    """A frame handed to the engine, its every decode recorded."""

    def __init__(self, frame: Any, calls: list[dict[str, Any]]) -> None:
        self._frame = frame
        self._calls = calls
        for name in ("container", "mode", "rv", "snr_db", "trusted", "t_start", "t_end", "floor"):
            setattr(self, name, getattr(frame, name))
        self.follows = getattr(frame, "follows", None)

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        payload, out = self._frame.decode(buffer)
        self._calls.append(
            {
                "in": None if buffer is None else bits(float(buffer)),  # type: ignore[arg-type]
                "payload": None if payload is None else BLOBS(payload),
                "out": bits(float(out)),  # type: ignore[arg-type]
            }
        )
        return payload, out


class Recorder:
    """Every input of one engine, and every action it drained, in order."""

    def __init__(self, engine: GrantedEngine, seed: int) -> None:
        self.engine = engine
        self.seed = seed
        self.call = engine.my_call
        self.timing = timing_doc(engine.timing)
        self.config = config_doc(engine.cfg)
        self.trace: list[dict[str, Any]] = []
        self._said: dict[str, Any] | None = None
        for name, encode in INPUTS.items():
            self._wrap(name, encode)
        drain = engine.drain

        def drained() -> list[Any]:
            out = drain()
            if out:
                self.trace.append({"drain": [action_doc(a) for a in out]})
            return out

        engine.drain = drained  # type: ignore[method-assign]

    def _wrap(self, name: str, encode: Callable[..., dict[str, Any]]) -> None:
        method = getattr(self.engine, name)

        def wrapped(*args: Any, **kw: Any) -> Any:
            entry: dict[str, Any] = {"in": name}
            if name == "on_frame":
                frame, now = args
                calls: list[dict[str, Any]] = []
                entry["frame"] = frame_doc(frame, calls)
                entry["now"] = bits(now)
                result = method(_Proxy(frame, calls), now)
            else:
                entry.update(encode(*args, **kw))
                result = method(*args, **kw)
            if name == "take_ladder":
                entry["rungs"] = [
                    {
                        "mode": r.mode,
                        "frames": r.frames,
                        "decoded": r.decoded,
                        "snr_db": bits(r.snr_db),
                    }
                    for r in result
                ]
            # what the engine says of itself, written when it changes: the replay compares it
            # after every input, the last one written standing until the next
            said = digest(self.engine)
            if said != self._said:
                entry["after"] = said
                self._said = said
            self.trace.append(entry)
            return result

        setattr(self.engine, name, wrapped)

    def doc(self) -> dict[str, Any]:
        return {
            "call": self.call,
            "seed": self.seed,
            "timing": self.timing,
            "config": self.config,
            "trace": self.trace,
        }


def frame_doc(frame: Any, calls: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "control": frame.container is Container.CONTROL,
        "mode": frame.mode,
        "floor": frame.floor,
        "rv": frame.rv,
        "snr_db": bits(frame.snr_db),
        "trusted": frame.trusted,
        "follows": getattr(frame, "follows", None),
        "t_start": bits(frame.t_start),
        "t_end": bits(frame.t_end),
        "decodes": calls,
    }


INPUTS: dict[str, Callable[..., dict[str, Any]]] = {
    "connect": lambda remote, as_call=None: {"remote": remote, "as": as_call},
    "probe": lambda remote, as_call=None: {"remote": remote, "as": as_call},
    "disconnect": lambda: {},
    "abort": lambda: {},
    "send": lambda data: {"data": BLOBS(data)},
    "set_identifier_due": lambda due=True: {"due": due},
    "set_answer_gap": lambda seconds: {"seconds": bits(seconds)},
    "pin_mode": lambda mode, body_bytes=None: {"mode": mode, "body": body_bytes},
    "take_ladder": lambda: {},
    "set_ceiling": lambda rung: {"rung": rung},
    "set_max_burst_s": lambda seconds: {"seconds": bits(seconds)},
    "set_max_mode": lambda rung: {"rung": rung},
    "tick": lambda now: {"now": bits(now)},
    "on_frame": lambda frame, now: {},  # handled in the wrapper
    "on_preamble": lambda t_start, now, frame_s=None: {
        "t_start": bits(t_start),
        "now": bits(now),
        "frame_s": bits(frame_s),
    },
    "on_tx_done": lambda now, frames_end=None: {"now": bits(now), "frames_end": bits(frames_end)},
    "on_tx_dropped": lambda now: {"now": bits(now)},
    "on_tx_cut_unread": lambda now: {"now": bits(now)},
    "on_tx_delayed": lambda seconds: {"seconds": bits(seconds)},
}


# ── the sessions ──────────────────────────────────────────────────────


def _pair(
    air: AirInterface, *, latency: float = 0.0, seeds: tuple[int, int] = (11, 12), **cfg: Any
) -> tuple[GrantedEngine, GrantedEngine, Recorder, Recorder]:
    timing = replace(phy_timing(air.params), tx_latency_s=latency)

    def config() -> GrantConfig:
        return GrantConfig(
            capabilities=bandwidth_capabilities(air.params),
            max_mode=air.n_rungs - 1,
            max_burst_s=26.0,
            **cfg,
        )

    a = GrantedEngine("W4ODA", timing, config(), seed=seeds[0])
    b = GrantedEngine("KK4XYZ", timing, config(), seed=seeds[1])
    return a, b, Recorder(a, seeds[0]), Recorder(b, seeds[1])


def _message(rng: random.Random, n: int) -> bytes:
    return bytes(rng.randrange(256) for _ in range(n))


def _station_model(rng: random.Random, timing: PhyTiming) -> dict[str, Any]:
    def latency(_rx: int, _frame: TxFrame, _end: float) -> float:
        return 0.12 + rng.expovariate(1.0 / 0.08)

    return {
        "key_lead": lambda _who: rng.uniform(0.0, 0.35),
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
    until: float = 900.0,
) -> None:
    a.connect(b.my_call)
    a.send(to_b)
    b.send(to_a)
    done = False
    t = 0.0
    while t < until:
        t += 5.0
        sim.run(until=t)
        if not done and sim.delivered(1) == to_b and sim.delivered(0) == to_a:
            done = True
            a.disconnect()
        if done and a.state.value == "idle" and b.state.value == "idle":
            break


def _fading(air: AirInterface, channel: str, seed: int) -> FadingPipe:
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from bench_link import FADING_CSV, fading_pipe

    return fading_pipe(Path(FADING_CSV), air, channel, seed)


def clean_wide() -> list[Recorder]:
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(1)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=1)
    _exchange(sim, a, b, _message(rng, 3000), _message(rng, 600))
    return [ra, rb]


def weak_narrow() -> list[Recorder]:
    a, b, ra, rb = _pair(NARROW)
    rng = random.Random(2)
    sim = TwoStationSim(a, b, snr_db=-3.0, seed=2)
    _exchange(sim, a, b, _message(rng, 600), _message(rng, 150))
    return [ra, rb]


def fading_field(
    air: AirInterface, channel: str, snr: float, seed: int, size: int
) -> Callable[[], list[Recorder]]:
    def run() -> list[Recorder]:
        a, b, ra, rb = _pair(air, latency=0.35)
        rng = random.Random(seed)
        sim = TwoStationSim(
            a,
            b,
            snr_db=snr,
            seed=seed,
            fading=_fading(air, channel, seed),
            **_station_model(rng, a.timing),
        )
        _exchange(sim, a, b, _message(rng, size), _message(rng, size // 4))
        return [ra, rb]

    return run


def outage() -> list[Recorder]:
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(4)

    def schedule(t: float) -> float:
        return -60.0 if 30.0 <= t < 70.0 else 6.0

    sim = TwoStationSim(a, b, snr_schedule=schedule, seed=4)
    _exchange(sim, a, b, _message(rng, 4000), _message(rng, 800))
    return [ra, rb]


def dead_path() -> list[Recorder]:
    """A session whose path dies: no response at the master, a link timeout at the slave."""
    a, b, ra, rb = _pair(NARROW)

    def schedule(t: float) -> float:
        return 6.0 if t < 40.0 else -60.0

    sim = TwoStationSim(a, b, snr_schedule=schedule, seed=5)
    a.connect("KK4XYZ")
    a.send(b"z" * 3000)
    sim.run(until=1200.0)
    return [ra, rb]


def floor_memory() -> list[Recorder]:
    """The slave's short ordinary answers killed two times in three, its floor ones spared — a
    PACTOR station's bursts (10-step-3.md §3.4) — and the master asking for floor answers while
    any of its last four slots went silent."""
    a, b, ra, rb = _pair(NARROW, floor_answer_memory=4)
    rng = random.Random(23)
    hits = random.Random(24)

    def bursts(frame: TxFrame) -> float:
        if frame.container is not Container.CONTROL or frame.floor:
            return 0.0
        if GrantFrame.decode(frame.payload).kind is not GrantKind.ANSWER:
            return 0.0
        return -40.0 if hits.random() < 2 / 3 else 0.0

    sim = TwoStationSim(a, b, snr_db=8.0, seed=23, frame_snr_offset=bursts)
    _exchange(sim, a, b, _message(rng, 2000), _message(rng, 300))
    return [ra, rb]


def cut_header() -> list[Recorder]:
    """The master's transmission cut a tenth into its header (10-step-3.md §3.6), then the
    abort: nothing was granted, and the DISC follows the cut. Then a watchdog-like cut of a
    header in a session that goes on: the master keys again at once."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(21)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=21)
    a.connect("KK4XYZ")
    a.send(_message(rng, 6000))
    st = sim.st[0]

    def header_begun() -> None:
        t = start = sim.t
        while t < start + 200.0:
            t += 0.02
            sim.run(until=t)
            sim.t = max(sim.t, t)
            first = st.first_frame
            if (
                first is not None
                and first[2]
                and first[0] <= t < first[0] + 0.1 * (first[1] - first[0])
                and st.tx_end - t > 2.0
            ):
                return
        raise RuntimeError("no transmission with a header first")

    header_begun()
    sim.cut(0)
    sim.run(until=sim.t + 10.0)
    a.send(_message(rng, 6000))
    header_begun()
    sim.cut(0)
    a.abort()
    sim.run(until=sim.t + 60.0)
    return [ra, rb]


def aborts() -> list[Recorder]:
    """The master aborts mid-transfer and calls again; then the slave aborts."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(6)
    sim = TwoStationSim(a, b, snr_db=8.0, seed=6)
    a.connect("KK4XYZ")
    a.send(_message(rng, 6000))
    sim.run(until=25.0)
    a.abort()
    sim.run(until=60.0)
    a.connect("KK4XYZ")
    a.send(_message(rng, 1000))
    b.send(_message(rng, 4000))
    sim.run(until=110.0)
    b.abort()
    sim.run(until=400.0)
    return [ra, rb]


def leaving() -> list[Recorder]:
    """Mid-transfer, the slave asks to close: its last words go, the master's queue does not;
    then, in a second session, both ask at once."""
    a, b, ra, rb = _pair(NARROW)
    rng = random.Random(41)
    sim = TwoStationSim(a, b, snr_db=2.0, seed=41)
    a.connect("KK4XYZ")
    sim.run(until=40.0)
    a.send(_message(rng, 3000))
    sim.run(until=70.0)
    b.send(_message(rng, 30))
    b.disconnect()
    sim.run(until=200.0)
    a.connect("KK4XYZ")
    sim.run(until=240.0)
    a.send(_message(rng, 3000))
    sim.run(until=270.0)
    a.disconnect()
    b.disconnect()
    sim.run(until=400.0)
    return [ra, rb]


def probes() -> list[Recorder]:
    """Probes answered, across bandwidths, and unanswered; a call to nobody."""
    timing = phy_timing(WIDE.params)
    narrow = phy_timing(NARROW.params)
    a = GrantedEngine(
        "W4ODA",
        timing,
        GrantConfig(capabilities=bandwidth_capabilities(WIDE.params), max_mode=WIDE.n_rungs - 1),
        seed=21,
    )
    b = GrantedEngine(
        "KK4XYZ",
        narrow,
        GrantConfig(
            capabilities=bandwidth_capabilities(NARROW.params), max_mode=NARROW.n_rungs - 1
        ),
        seed=22,
    )
    ra, rb = Recorder(a, 21), Recorder(b, 22)
    sim = TwoStationSim(a, b, snr_db=4.0, seed=7)
    a.probe("KK4XYZ")
    sim.run(until=40.0)
    b.probe("W4ODA")
    sim.run(until=80.0)
    a.probe("N0BODY")
    sim.run(until=130.0)
    a.connect("KK4XYZ")  # another bandwidth: ignored, and said
    sim.run(until=400.0)
    return [ra, rb]


def identifiers() -> list[Recorder]:
    """Both stations identify; their identifiers are placed in the master's time and in slots
    granted with it, and close the session."""
    a, b, ra, rb = _pair(WIDE, identifier_s=4.5, turnaround_s=0.3)
    rng = random.Random(8)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=8)
    a.connect("KK4XYZ")
    a.send(_message(rng, 2500))
    b.send(_message(rng, 2500))
    t = 0.0
    while t < 500.0:
        t += 5.0
        if t in (20.0, 60.0):
            a.set_identifier_due(True)
            b.set_identifier_due(True)
        sim.run(until=t)
        if t == 120.0:
            a.disconnect()
    return [ra, rb]


def ladder() -> list[Recorder]:
    """The Test's ladder: the master's new data pinned rung by rung; a floor-only ceiling."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(9)
    sim = TwoStationSim(a, b, snr_db=14.0, seed=9)
    a.connect("KK4XYZ")
    sim.run(until=15.0)
    t = 15.0
    for rung in (4, 8, 12, 16):
        a.pin_mode(rung, 40)
        a.send(_message(rng, 120))
        t += 25.0
        sim.run(until=t)
        a.take_ladder()
    a.pin_mode(None)
    a.set_ceiling(1)
    b.send(_message(rng, 300))
    a.send(_message(rng, 300))
    t += 120.0
    sim.run(until=t)
    a.set_ceiling(None)
    a.disconnect()
    sim.run(until=t + 200.0)
    return [ra, rb]


def late_station() -> list[Recorder]:
    """A slave whose receiver reports every frame a second late and whose keying is often past
    the slack: levels asked for, slots forfeited, transmissions dropped by the gate."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(10)

    def latency(rx: int, _frame: TxFrame, _end: float) -> float:
        return 1.0 if rx == 1 else 0.1

    def lead(who: int) -> float:
        return rng.choice([0.0, 0.2, 0.6]) if who == 1 else 0.05

    sim = TwoStationSim(a, b, snr_db=9.0, seed=10, decode_latency=latency, key_lead=lead)
    _exchange(sim, a, b, _message(rng, 2000), _message(rng, 1500))
    return [ra, rb]


def simultaneous() -> list[Recorder]:
    """Both stations call each other at once."""
    a, b, ra, rb = _pair(NARROW)
    sim = TwoStationSim(a, b, snr_db=6.0, seed=11)
    a.connect("KK4XYZ")
    b.connect("W4ODA")
    a.send(b"a" * 300)
    sim.run(until=300.0)
    return [ra, rb]


def called_again() -> list[Recorder]:
    """The caller loses the session (aborts unheard) and calls again: the old session ends and
    the new call is answered; a delayed transmission moves the master's waits."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(12)

    def unheard(rx: int, container: Container, t0: float) -> bool:
        return rx == 1 and 30.0 <= t0 < 45.0  # the abort's DISCs never reach the slave

    sim = TwoStationSim(a, b, snr_db=7.0, seed=12, unheard=unheard)
    a.connect("KK4XYZ")
    a.send(_message(rng, 5000))
    sim.run(until=30.0)
    a.abort()
    sim.run(until=46.0)
    a.connect("KK4XYZ")
    a.send(_message(rng, 800))
    sim.run(until=60.0)
    a.on_tx_delayed(0.4)
    sim.run(until=300.0)
    return [ra, rb]


def phantom_storm() -> list[Recorder]:
    """Thirty false detections a minute at both stations, a fifth of them trusted: frames near
    the data's slots and between them."""
    a, b, ra, rb = _pair(WIDE, latency=0.35)
    rng = random.Random(13)
    model = _station_model(rng, a.timing)
    model["phantoms_per_min"] = 30.0
    model["phantom_trusted"] = 0.2
    sim = TwoStationSim(a, b, snr_db=6.0, seed=13, **model)
    _exchange(sim, a, b, _message(rng, 1500), _message(rng, 600))
    return [ra, rb]


def ramp() -> list[Recorder]:
    """A path that sinks from +14 to -4 dB and comes back: rungs mixed in one transmission,
    re-encoded frames, combining."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(14)

    def schedule(t: float) -> float:
        phase = t % 160.0
        return 14.0 - 18.0 * phase / 80.0 if phase < 80.0 else -4.0 + 18.0 * (phase - 80.0) / 80.0

    sim = TwoStationSim(a, b, snr_schedule=schedule, seed=14)
    _exchange(sim, a, b, _message(rng, 5000), _message(rng, 2500), until=700.0)
    return [ra, rb]


def stranded() -> list[Recorder]:
    """The master's data pinned far above what the path carries: every copy fails, the
    receiver combines up to its limit and starts over, the frames are re-encoded."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(15)
    sim = TwoStationSim(a, b, snr_db=4.0, seed=15)
    a.connect("KK4XYZ")
    sim.run(until=15.0)
    a.pin_mode(18, 60)
    a.send(_message(rng, 240))
    sim.run(until=120.0)
    a.pin_mode(None)
    sim.run(until=300.0)
    a.disconnect()
    sim.run(until=500.0)
    return [ra, rb]


def harq_limit() -> list[Recorder]:
    """Full frames pinned at the top rung, where nothing slower can carry them: no re-encoding,
    so a block goes again and again at its rung, combined until the receiver's limit and
    started over."""
    a, b, ra, rb = _pair(WIDE)
    rng = random.Random(18)
    sim = TwoStationSim(a, b, snr_db=9.0, seed=18)
    a.connect("KK4XYZ")
    sim.run(until=15.0)
    a.pin_mode(WIDE.n_rungs - 1)
    a.send(_message(rng, 3000))
    sim.run(until=200.0)
    a.abort()
    sim.run(until=300.0)
    return [ra, rb]


def vox() -> list[Recorder]:
    """Radios keyed by VOX: each station declares a turnaround longer than the decode budget,
    and keys 0.1 s after its engine asks."""
    a, b, ra, rb = _pair(NARROW, latency=0.1, turnaround_s=1.2)
    rng = random.Random(16)
    sim = TwoStationSim(a, b, snr_db=5.0, seed=16, key_lead=lambda _who: 0.1)
    _exchange(sim, a, b, _message(rng, 500), _message(rng, 300))
    return [ra, rb]


def answer_gap() -> list[Recorder]:
    """Radios that hold their transmitter after the audio and declare nothing, and an answer gap
    at each end that covers them: the master keys the gap after the slave's frames, the slave
    asks for the level that starts its slots the gap after the master's."""
    a, b, ra, rb = _pair(NARROW)
    a.set_answer_gap(1.3)
    b.set_answer_gap(1.3)
    rng = random.Random(17)
    sim = TwoStationSim(a, b, snr_db=6.0, seed=17, rx_recovery_s=(1.1, 1.1))
    _exchange(sim, a, b, _message(rng, 600), _message(rng, 300))
    return [ra, rb]


def idle_identifier() -> list[Recorder]:
    """An idle link: open windows, the slave's identifier falling due in one with nothing else
    to say, and a line typed into another."""
    a, b, ra, rb = _pair(WIDE, identifier_s=3.0)
    sim = TwoStationSim(a, b, snr_db=10.0, seed=17)
    a.connect("KK4XYZ")
    sim.run(until=30.0)
    b.set_identifier_due(True)
    sim.run(until=70.0)
    b.send(b"a line typed at the slave")
    sim.run(until=110.0)
    a.disconnect()
    sim.run(until=300.0)
    return [ra, rb]


SESSIONS: dict[str, Callable[[], list[Recorder]]] = {
    "clean-wide": clean_wide,
    "weak-narrow": weak_narrow,
    "good-wide-field": fading_field(WIDE, "good", 6.0, 31, 1500),
    "moderate-narrow-field": fading_field(NARROW, "moderate", 3.0, 32, 400),
    "poor-wide-field": fading_field(WIDE, "poor", 3.0, 33, 1500),
    "outage": outage,
    "dead-path": dead_path,
    "aborts": aborts,
    "leaving": leaving,
    "probes": probes,
    "identifiers": identifiers,
    "ladder": ladder,
    "late-station": late_station,
    "simultaneous": simultaneous,
    "called-again": called_again,
    "phantom-storm": phantom_storm,
    "ramp": ramp,
    "stranded": stranded,
    "harq-limit": harq_limit,
    "vox": vox,
    "cut-header": cut_header,
    "floor-memory": floor_memory,
    "answer-gap": answer_gap,
    "idle-identifier": idle_identifier,
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default="core/aether-link/tests/data/granted_vectors.json")
    args = ap.parse_args()
    sessions = []
    for name, run in SESSIONS.items():
        recorders = run()
        sessions.append({"name": name, "stations": [r.doc() for r in recorders]})
        steps = sum(len(r.trace) for r in recorders)
        print(f"{name}: {steps} steps")
    document = {
        "note": "Generated by tools/make_granted_vectors.py from the Python reference model: "
        "every input of protocol 8's engine through these sessions, and every action. Payloads "
        "are indices into blobs.",
        "blobs": BLOBS.table,
        "sessions": sessions,
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(document, separators=(",", ":")) + "\n", encoding="utf-8", newline="\n"
    )
    print(f"wrote {out} ({out.stat().st_size // 1024} kB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
