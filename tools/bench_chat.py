"""Keyboard-to-keyboard chat on the link bench: how long a typed line takes to arrive.

    python tools/bench_chat.py [--policies base,handover,request] [--bandwidth 2300,500]
                               [--channels awgn,good,moderate,poor] [--snr -6,0,6,12]
                               [--trials 20] [--jobs 4] [--out bench/baselines/chat_handover.csv]

``bench_link.py`` measures a transfer: one station sends, the other acknowledges, and what
matters is goodput. A keyboard-to-keyboard contact is the other shape of traffic — short
lines both ways, with the operators reading and typing between them — and what matters is
how long a line takes from the moment it is sent to the moment it is on the other screen.
Little of that is air time: a line typed at the station that does not hold the turn waits for
the sender's next poll, which an idle link sends every ``keepalive_s``, and then for a poll,
an acknowledgement and a TURN before its own burst can go (ADR-0023's turn-taking).

**The workload** is seeded by the trial alone, so every policy, channel and SNR runs the same
script: A calls B, and once both are connected the two exchange 10–20 lines of 20–120 bytes.
Each line is typed 5–30 s after the line before it was delivered, by the other station — or,
one time in five, by the same station again, before the reply. A line is typed only once the
one before has arrived, so no line waits behind another and a line's latency is its own: from
``send()`` to the delivery of its last byte at the other station. The call is not part of it
(it is the same for every policy); the conversation starts when both stations are up.

**The cost** is keyed time: both transmitters' air time, every frame, from the first line
typed to the last one delivered — what polling more often, or handing the turn over when
nobody asked for it, puts on the air. A session that ends before its last line arrives (the
link gives up) is a drop, and its undelivered lines are lost; a line not delivered within
``LINE_S`` ends the session the same way.

**The channel** is ``bench_link.py``'s: the fading pipe (P9-6, ``bench/baselines/
fading_pipe.csv``) with the tone floor's SNR reading capped at its class's ceiling (ADR-0016)
— ``--logistic`` and ``--no-floor-cap`` for the older pipe and uncapped readings — and a burst
is held to the daemon's key-time limit (ADR-0017, ``--max-burst-s``).

**Policies** are ``LinkConfig`` overrides by name (:data:`POLICIES`) — ``chat`` is what
ADR-0027 adopted — and ``handover`` and ``hold``, which run what it did not adopt
(:class:`Candidate`); ``--policy name:key=value,…`` adds one. What was measured, and what was
decided, is ADR-0027.

Output: one row per (policy, bandwidth, channel, SNR) to ``--out`` — sessions, lines sent,
delivered and lost, drops, median and 90th-percentile latency (every line, replies,
follow-ups), keyed seconds per line and the transmitters' duty cycle, turns, polls and
requests per line; ``--sessions`` writes one row per session.
"""

from __future__ import annotations

import argparse
import csv
import random
import statistics
import sys
import time
from collections.abc import Sequence
from dataclasses import dataclass
from multiprocessing import Pool
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import bench_link as bench
from aether_model.frame.modes import NARROW, WIDE, AirInterface
from aether_model.link.engine import LinkConfig, LinkEngine, Role, State, Transmit
from aether_model.link.frames import ControlKind, DataKind
from aether_model.link.harness import phy_timing
from aether_model.link.phy import Container, TxFrame
from aether_model.link.sim import TwoStationSim, control_thresholds_for

HERE = Path(__file__).resolve().parents[1]

Setting = float | int | bool | str | None

POLICIES: dict[str, dict[str, Setting]] = {
    "base": {},
    "keepalive5": {"keepalive_s": 5.0},
    "handover": {"handover": "always"},
    "handover+keepalive5": {"handover": "always", "keepalive_s": 5.0},
    "request": {"chat": True},
    "request-without-hold": {"chat": True, "hold": False},
    "handover+request": {"chat": True, "handover": "always"},
    "ordinary-handover+request": {"chat": True, "handover": "ordinary"},
    "request+keepalive20": {"chat": True, "keepalive_s": 20.0},
    "handover+request+keepalive20": {"chat": True, "handover": "always", "keepalive_s": 20.0},
}
"""The turn policies measured: today's (``base``); a shorter idle poll (``keepalive5``); the
sender handing the turn over as soon as its line is acknowledged (``handover``, the
:class:`Candidate` engine); the receiving station asking for the turn unasked (``request``:
``LinkConfig.chat``, what ADR-0027 adopted), and the same without the idle sender's hold over
a frame arriving (``hold``); and combinations — with a longer idle poll too, which asking makes
possible: a poll no longer carries the other station's wish to send, only the news that the
link is alive. Every key but ``handover`` and ``hold`` is a ``LinkConfig`` field."""

LINES = (10, 20)
"""Lines per conversation, inclusive."""
LINE_BYTES = (20, 120)
"""A chat line's length, inclusive: a short sentence to a long one."""
THINK_S = (5.0, 30.0)
"""Reading the last line and typing the next, seconds."""
FOLLOW_UP = 0.2
"""How often the station that sent the last line sends another before the reply."""
TEXT = b"abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789 .,?!"
"""What the lines are made of: text, as a chat is (the pipe does not look at it)."""
CONVERSATION_SEED = 27_000
"""The script's seed is this plus the trial: the same conversation on every policy, channel
and SNR, so a difference between two policies is theirs and not the script's."""

CONNECT_S = 400.0
"""How long the call may take before the session is given up as never connected."""
LINE_S = 900.0
"""How long a line may take to arrive before the session is given up as stalled."""
STEP_S = 1.0
"""The simulator is run in steps of this while a line is on its way; delivery times are
exact whatever the step (the simulator notes them)."""

MAX_KEY_S = 30.0
"""The daemon's default key-time limit (``[radio] max_key_s``)."""
KEY_LEAD_S = 0.1
KEY_TAIL_S = 0.05
KEY_TIME_MARGIN_S = 1.0
"""The daemon's keying lead and tail and its margin (``station.rs``): a burst is held to
``burst_limit_s`` = the limit less the three (ADR-0017)."""
MAX_BURST_S = MAX_KEY_S - KEY_LEAD_S - KEY_TAIL_S - KEY_TIME_MARGIN_S


@dataclass(frozen=True)
class Line:
    sender: int
    """0 the calling station, 1 the called one."""
    body: bytes
    think_s: float
    """Seconds between the delivery of the line before and this one's ``send()``."""
    follow_up: bool
    """The same station sent the line before (no reply came between)."""


def conversation(trial: int, file_bytes: int = 0) -> list[Line]:
    """The trial's script: who types what, and after how long. With ``file_bytes``, the line
    in the middle is a file of that size instead — a transfer inside a chat session."""
    rng = random.Random(CONVERSATION_SEED + trial)
    count = rng.randint(*LINES)
    sender = rng.randrange(2)
    lines: list[Line] = []
    for i in range(count):
        follow_up = i > 0 and rng.random() < FOLLOW_UP
        if i > 0 and not follow_up:
            sender = 1 - sender
        size = rng.randint(*LINE_BYTES)
        body = bytes(rng.choice(TEXT) for _ in range(size))
        lines.append(Line(sender, body, rng.uniform(*THINK_S), follow_up))
    if file_bytes:
        middle = lines[count // 2]
        body = bytes(rng.randrange(256) for _ in range(file_bytes))
        lines[count // 2] = Line(middle.sender, body, middle.think_s, middle.follow_up)
    return lines


class Candidate(LinkEngine):
    """The model's engine with what ADR-0027 measured and did not adopt, which lives here and
    not in the model so the bench can still run it.

    ``handover``: a sender whose bursts of this turn are all acknowledged, with nothing more to
    send, hands the turn over at once (TURN) — to the station that has just read its line and is
    likely to answer it — on every family (``always``), or only while its control frames are
    ordinary ones (``ordinary``: on the tone floor a TURN and its confirmation are three frames
    of 3.2 s). It lost sessions where the request did not.

    ``hold=False``: the adopted chat without the idle sender's hold — its poll keyed on time
    even over a frame it hears arriving, which may be the other station's request."""

    def __init__(
        self, *args: object, handover: str = "", hold: bool = True, **kwargs: object
    ) -> None:
        super().__init__(*args, **kwargs)  # type: ignore[arg-type]
        if handover not in ("", "always", "ordinary"):
            raise ValueError(f"handover={handover!r}: always or ordinary")
        self.handover = handover
        self.hold = hold

    def _maybe_start_burst(self) -> None:
        if self._hands_over():
            self._turn_tries = 0
            self._send_turn()
            return
        super()._maybe_start_burst()

    def on_preamble(self, t_start: float, now: float, frame_s: float | None = None) -> None:
        keepalive = self._deadlines.get("keepalive")
        super().on_preamble(t_start, now, frame_s)
        if not self.hold and keepalive is not None and "keepalive" in self._deadlines:
            self._deadlines["keepalive"] = keepalive

    def _hands_over(self) -> bool:
        return (
            self.handover != ""
            and self.state is State.CONNECTED
            and self.role is Role.ISS
            and self._waiting_for is None
            and not self._tx_busy()
            and not self._disc_requested
            and not self._has_work()
            and self._bursts_since_turn > 0
            and (self.handover == "always" or not self.timing.is_floor(self._burst_mode()))
        )


def frame_kind(frame: TxFrame) -> str:
    """What a frame is, from its first byte: a DATA container's kind or a control frame's."""
    if frame.container is Container.DATA:
        return DataKind(frame.payload[0] >> 5).name.lower()
    return ControlKind(frame.payload[0] >> 4).name.lower()


class ChatSim(TwoStationSim):
    """The link bench's simulator, noting when each station's delivered stream grew and
    every frame each transmitter keyed."""

    def __init__(self, *args: object, **kwargs: object) -> None:
        super().__init__(*args, **kwargs)  # type: ignore[arg-type]
        self.growth: list[list[tuple[float, int]]] = [[], []]
        """Per station: (time, bytes delivered so far) each time the stream grew."""
        self.keyed: list[tuple[int, float, float, str]] = []
        """(station, start, end, kind) of every frame put on the air."""

    def _pump(self, who: int, at: float) -> None:
        before = len(self.st[who].delivered)
        super()._pump(who, at)
        after = len(self.st[who].delivered)
        if after > before:
            self.growth[who].append((at, after))

    def _launch(self, who: int, at: float, tx: Transmit) -> None:
        t = max(at, self.st[who].tx_end)
        timing = self.st[who].engine.timing
        for frame in tx.frames:
            dur = timing.frame_s(frame)
            self.keyed.append((who, t, t + dur, frame_kind(frame)))
            t += dur
        super()._launch(who, at, tx)

    def delivered_at(self, who: int, length: int) -> float | None:
        """When station ``who`` had received ``length`` bytes of the stream, if it has."""
        for at, have in self.growth[who]:
            if have >= length:
                return at
        return None

    def keyed_between(self, t0: float, t1: float) -> dict[str, float]:
        """Seconds keyed by both stations between two times, by frame kind."""
        out: dict[str, float] = {}
        for _, start, end, kind in self.keyed:
            overlap = min(end, t1) - max(start, t0)
            if overlap > 0:
                out[kind] = out.get(kind, 0.0) + overlap
        return out

    def count_between(self, t0: float, t1: float, kind: str) -> int:
        """Frames of one kind that started between two times."""
        return sum(1 for _, start, _, k in self.keyed if k == kind and t0 <= start < t1)


@dataclass(frozen=True)
class Point:
    policy: str
    overrides: tuple[tuple[str, Setting], ...]
    bandwidth: int
    channel: str
    snr_db: float
    trials: int
    fading: bool
    floor_cap: bool
    max_burst_s: float | None
    file_bytes: int = 0
    first_trial: int = 0


def make_sim(point: Point, air: AirInterface, a: LinkEngine, b: LinkEngine, seed: int) -> ChatSim:
    """Two engines on the link bench's channel for this point."""
    timing = a.timing
    cap = bench.FLOOR_READING_CAP_DB.get(point.channel) if point.floor_cap else None
    if point.fading:
        return ChatSim(
            a,
            b,
            snr_db=point.snr_db,
            seed=seed,
            thresholds=bench.table_for(air)[0],
            control_thresholds=control_thresholds_for(timing),
            fading=bench.fading_pipe(HERE / bench.FADING_CSV, air, point.channel, seed),
            floor_reading_cap_db=cap,
        )
    awgn = bench.table_for(air)[0]
    fer = HERE / "bench" / "baselines" / ("phy_fer_500.csv" if air is NARROW else "phy_fer.csv")
    tone = HERE / bench.TONE_CSV
    tables = bench.channel_thresholds(fer, awgn=awgn, air=air, tone_csv=tone)
    awgn_controls = control_thresholds_for(timing)
    controls = bench.control_thresholds(
        HERE / bench.CONTROL_CSV[air], awgn_controls, tables, awgn, tone_csv=tone, air=air
    )
    return ChatSim(
        a,
        b,
        snr_db=point.snr_db,
        seed=seed,
        thresholds=tables.get(point.channel, awgn),
        control_thresholds=controls.get(point.channel, awgn_controls),
        floor_reading_cap_db=cap,
    )


def run_session(point: Point, trial: int) -> dict[str, object]:
    """One call and one conversation; what it cost and how long each line took."""
    air = WIDE if point.bandwidth == 2300 else NARROW
    timing = phy_timing(air.params)
    seed = 1000 * trial + 17

    def engine(call: str, engine_seed: int) -> LinkEngine:
        settings = dict(point.overrides)
        handover = str(settings.pop("handover", "") or "")
        hold = bool(settings.pop("hold", True))
        cfg = LinkConfig(
            max_mode=air.n_rungs - 1,
            max_burst_s=point.max_burst_s,
            **settings,  # type: ignore[arg-type]
        )
        if handover or not hold:
            return Candidate(call, timing, cfg, seed=engine_seed, handover=handover, hold=hold)
        return LinkEngine(call, timing, cfg, seed=engine_seed)

    a, b = engine("W4ODA", seed), engine("KK4XYZ", seed + 1)
    sim = make_sim(point, air, a, b, seed)
    lines = conversation(trial, point.file_bytes)
    file_line = len(lines) // 2 if point.file_bytes else None
    row: dict[str, object] = {
        "policy": point.policy,
        "bandwidth_hz": point.bandwidth,
        "channel": point.channel,
        "snr_db": point.snr_db,
        "trial": trial,
        "lines": len(lines),
    }

    def down() -> bool:
        return a.state is State.IDLE or b.state is State.IDLE

    a.connect("KK4XYZ")
    clock = 0.0
    while not (a.connected and b.connected):
        clock += STEP_S
        sim.run(until=clock)
        if a.state is State.IDLE or clock >= CONNECT_S:
            return {**row, "connected": 0, "connect_s": ""}
    row.update(connected=1, connect_s=clock)

    sent = [0, 0]  # bytes each station has handed to send()
    latency: list[float] = []
    replies: list[float] = []
    follow_ups: list[float] = []
    file_s: float | str = ""
    typed = clock + lines[0].think_s
    first_typed = typed
    end = typed
    dropped = stalled = False
    for i, line in enumerate(lines):
        sim.run(until=typed)
        if down():
            dropped = True
            end = max(first_typed, sim.t)
            break
        sender = sim.st[line.sender].engine
        sender.tick(typed)
        sender.send(line.body)
        sim._pump(line.sender, typed)
        sent[line.sender] += len(line.body)
        receiver = 1 - line.sender
        clock = typed
        arrived = None
        while True:
            arrived = sim.delivered_at(receiver, sent[line.sender])
            if arrived is not None or down() or clock >= typed + LINE_S:
                break
            clock = min(typed + LINE_S, clock + STEP_S)
            sim.run(until=clock)
        if arrived is None:
            dropped = down()
            stalled = not dropped
            end = clock
            break
        took = arrived - typed
        if i == file_line:
            file_s = round(took, 2)  # a file is no chat line: its time is its own
        else:
            latency.append(took)
            (follow_ups if line.follow_up else replies).append(took)
        end = arrived
        if i + 1 < len(lines):
            typed = arrived + lines[i + 1].think_s
    keyed = sim.keyed_between(first_typed, end)
    reasons = [e for e in sim.events(0) + sim.events(1) if e.startswith("disconnected")]
    arrived_lines = len(latency) + (file_s != "")
    row.update(
        delivered=arrived_lines,
        lost=len(lines) - arrived_lines,
        file_s=file_s,
        dropped=int(dropped),
        stalled=int(stalled),
        drop_reason=reasons[0].split(":", 1)[1] if reasons else "",
        window_s=round(end - first_typed, 2),
        keyed_s=round(sum(keyed.values()), 2),
        keyed_data_s=round(keyed.get("data", 0.0), 2),
        keyed_control_s=round(sum(v for k, v in keyed.items() if k != "data"), 2),
        polls=sim.count_between(first_typed, end, "poll"),
        turns=sim.count_between(first_typed, end, "turn"),
        acks=sim.count_between(first_typed, end, "ack"),
        requests=a.stats.turn_requests + b.stats.turn_requests,
        latency=latency,
        replies=replies,
        follow_ups=follow_ups,
    )
    return row


def run_point(point: Point) -> list[dict[str, object]]:
    """Every trial of one point."""
    first = point.first_trial
    return [run_session(point, trial) for trial in range(first, first + point.trials)]


def _quantile(values: Sequence[float], q: float) -> float | str:
    return round(float(np.percentile(values, q)), 1) if values else ""


def summarise(point: Point, rows: list[dict[str, object]], wall_s: float) -> dict[str, object]:
    """One row for a point: the lines of every session pooled."""
    up = [r for r in rows if r.get("connected")]
    every = [x for r in up for x in r["latency"]]  # type: ignore[attr-defined]
    replies = [x for r in up for x in r["replies"]]  # type: ignore[attr-defined]
    follow_ups = [x for r in up for x in r["follow_ups"]]  # type: ignore[attr-defined]
    files = [float(r["file_s"]) for r in up if r.get("file_s") not in ("", None)]  # type: ignore[arg-type]
    typed = sum(int(r["lines"]) for r in up)  # type: ignore[call-overload]
    keyed = sum(float(r["keyed_s"]) for r in up)  # type: ignore[arg-type]
    window = sum(float(r["window_s"]) for r in up)  # type: ignore[arg-type]

    def per_line(field: str) -> float | str:
        total = sum(float(r[field]) for r in up)  # type: ignore[arg-type]
        return round(total / typed, 3) if typed else ""

    return {
        "policy": point.policy,
        "link": ",".join(f"{k}={v}" for k, v in point.overrides),
        "bandwidth_hz": point.bandwidth,
        "channel": point.channel,
        "snr_db": point.snr_db,
        "pipe": "fading" if point.fading else "logistic",
        "floor_cap": int(point.floor_cap),
        "first_trial": point.first_trial,
        "sessions": len(rows),
        "connected": len(up),
        "lines": typed,
        "delivered": len(every),
        "lost": sum(int(r["lost"]) for r in up),  # type: ignore[call-overload]
        "drops": sum(int(r["dropped"]) for r in up),  # type: ignore[call-overload]
        "stalls": sum(int(r["stalled"]) for r in up),  # type: ignore[call-overload]
        "median_s": _quantile(every, 50),
        "p90_s": _quantile(every, 90),
        "mean_s": round(statistics.fmean(every), 1) if every else "",
        "reply_median_s": _quantile(replies, 50),
        "reply_p90_s": _quantile(replies, 90),
        "follow_up_median_s": _quantile(follow_ups, 50),
        "follow_up_p90_s": _quantile(follow_ups, 90),
        "file_bytes": point.file_bytes,
        "file_median_s": _quantile(files, 50),
        "keyed_per_line_s": round(keyed / typed, 2) if typed else "",
        "duty": round(keyed / window, 3) if window else "",
        "polls_per_line": per_line("polls"),
        "turns_per_line": per_line("turns"),
        "requests_per_line": per_line("requests"),
        "wall_s": round(wall_s, 1),
    }


def _timed(point: Point) -> tuple[Point, list[dict[str, object]], float]:
    wall = time.perf_counter()
    rows = run_point(point)
    return point, rows, time.perf_counter() - wall


def parse_policy(text: str) -> tuple[str, dict[str, Setting]]:
    """``name:key=value,…`` — a policy of one's own: ``LinkConfig`` fields (numbers, ``true``,
    ``false``, ``none``), ``handover=always|ordinary`` and ``hold=false``."""
    name, _, spec = text.partition(":")
    settings: dict[str, Setting] = {}
    for item in spec.split(","):
        if not item.strip():
            continue
        key, value = (x.strip() for x in item.split("=", 1))
        lowered = value.lower()
        if lowered in ("true", "false"):
            settings[key] = lowered == "true"
        elif lowered == "none":
            settings[key] = None
        elif key == "handover":
            settings[key] = value
        else:
            settings[key] = int(value) if value.lstrip("-").isdigit() else float(value)
    return name, settings


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--policies", default=",".join(POLICIES), help="names from POLICIES")
    ap.add_argument(
        "--policy",
        action="append",
        default=[],
        help="another policy, name:key=value,... (LinkConfig fields); may repeat",
    )
    ap.add_argument("--bandwidth", default="2300,500")
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--snr", default="-6,0,6,12")
    ap.add_argument("--trials", type=int, default=30)
    ap.add_argument(
        "--first-trial", type=int, default=0, help="the first trial's number (seeds and script)"
    )
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--logistic", action="store_true", help="the per-class logistic pipe")
    ap.add_argument(
        "--no-floor-cap", action="store_true", help="tone-floor readings as the channel gives"
    )
    ap.add_argument(
        "--max-burst-s",
        type=float,
        default=MAX_BURST_S,
        help="the longest burst (the daemon's key-time limit less lead, tail and margin); "
        "0 for none",
    )
    ap.add_argument(
        "--file",
        type=int,
        default=0,
        help="bytes of a file sent as the conversation's middle line (0: none) — a transfer "
        "inside a chat session, timed on its own",
    )
    ap.add_argument("--out", default="", help="one row per point")
    ap.add_argument("--sessions", default="", help="one row per session")
    args = ap.parse_args()

    policies = {name: POLICIES[name] for name in args.policies.split(",") if name}
    policies.update(parse_policy(p) for p in args.policy)
    points = [
        Point(
            name,
            tuple(sorted(overrides.items())),
            int(bw),
            channel,
            float(snr),
            args.trials,
            not args.logistic,
            not args.no_floor_cap,
            args.max_burst_s or None,
            args.file,
            args.first_trial,
        )
        for bw in args.bandwidth.split(",")
        for channel in args.channels.split(",")
        for snr in args.snr.split(",")
        for name, overrides in policies.items()
    ]
    summary: list[dict[str, object]] = []
    sessions: list[dict[str, object]] = []
    with Pool(args.jobs) as pool:
        for point, rows, wall in pool.imap(_timed, points):
            row = summarise(point, rows, wall)
            summary.append(row)
            sessions += [
                {
                    k: (" ".join(f"{x:.1f}" for x in v) if isinstance(v, list) else v)
                    for k, v in r.items()
                }
                for r in rows
            ]
            carried = f"  file {row['file_median_s']} s" if point.file_bytes else ""
            print(
                f"{point.bandwidth:>4} {point.channel:8s} {point.snr_db:+5.0f} "
                f"{point.policy:22s} median {row['median_s']!s:>5} p90 {row['p90_s']!s:>5} "
                f"(reply {row['reply_median_s']!s:>5}, follow-up {row['follow_up_median_s']!s:>5}) "
                f"keyed/line {row['keyed_per_line_s']!s:>5} s  lost {row['lost']} "
                f"drops {row['drops']}{carried}  [{wall:.0f} s]",
                flush=True,
            )
    for path, rows in ((args.out, summary), (args.sessions, sessions)):
        if path and rows:
            target = Path(path)
            target.parent.mkdir(parents=True, exist_ok=True)
            fields = list(dict.fromkeys(k for r in rows for k in r))
            with target.open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
                writer.writeheader()
                writer.writerows(rows)
            print(f"wrote {target} ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
