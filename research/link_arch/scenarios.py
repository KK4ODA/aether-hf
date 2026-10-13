"""Deterministic adversarial scenarios, run against every arm.

    uv run python -m research.link_arch.scenarios --arms legacy,stated,stated6,gs,fc \
        --trials 8 --out research/link_arch/results/scenarios.csv

Each scenario is a workload, a channel and an adversary that decides the fate of chosen
frames (``advsim.FrameCtx``): a lost acknowledgement, a lost handover, a late decode, a
detected frame that does not decode. The same adversary is applied to every arm, keyed by
what the frame *is* in each protocol — an answer is the receiving station's control frame
that opens its transmission, data is data — so a scenario means the same event everywhere.
"""

from __future__ import annotations

import argparse
import csv
import statistics
from collections import defaultdict
from collections.abc import Callable
from dataclasses import dataclass, replace
from multiprocessing import Pool
from typing import Any

from research.link_arch.advsim import AdvSim, FrameCtx
from research.link_arch.setups import builder
from research.link_arch.workload import run

CONTROL_KINDS = {"ack", "poll", "turn", "disc", "disc_ack", "hdr", "ans", "control"}


def opens_with_control(ctx: FrameCtx) -> bool:
    return bool(ctx.kinds) and ctx.kinds[0] in CONTROL_KINDS


class Ordinals:
    """Counts each station's transmissions that carry data: the n-th burst, whatever the
    protocol puts around it."""

    def __init__(self) -> None:
        self.seen: dict[tuple[int, int], int] = {}
        self.count = [0, 0]

    def __call__(self, ctx: FrameCtx) -> int | None:
        if "data" not in ctx.kinds:
            return None
        key = (ctx.tx, ctx.tx_n)
        if key not in self.seen:
            self.seen[key] = self.count[ctx.tx]
            self.count[ctx.tx] += 1
        return self.seen[key]


@dataclass(frozen=True)
class Scenario:
    name: str
    kind: str
    """Workload: bulk, exchange or chat."""
    describe: str
    adversary: Callable[[], Callable[[FrameCtx], object]] | None = None
    profile: str = "field"
    asym_db: float = 0.0
    schedule: Callable[[float], float] | None = None
    tweak: dict[str, Any] | None = None
    """StationCfg fields to change for station B (the receiver of a bulk transfer)."""
    nbytes: int = 4000


def _ack_lost() -> Callable[[FrameCtx], object]:
    def fate(ctx: FrameCtx) -> object:
        # every third transmission of the receiving station that opens with a control frame
        # (its acknowledgement): A hears none of it
        if ctx.tx == 1 and opens_with_control(ctx) and ctx.tx_n % 3 == 1:
            return "unheard"
        return None

    return fate


def _data_lost() -> Callable[[FrameCtx], object]:
    nth = Ordinals()

    def fate(ctx: FrameCtx) -> object:
        n = nth(ctx)
        if ctx.tx == 0 and n in (1, 4) and ctx.kind == "data":
            return "unheard"  # the 2nd and 5th bursts' data, every frame
        return None

    return fate


def _handover_lost(whole: bool) -> Callable[[], Callable[[FrameCtx], object]]:
    def make() -> Callable[[FrameCtx], object]:
        nth = Ordinals()

        def fate(ctx: FrameCtx) -> object:
            # the called station's first transmission that carries data — where the
            # direction changes — goes unheard: all of it, or its opening control frame
            if (
                ctx.tx == 1
                and nth(ctx) == 0
                and (whole or (ctx.i == 0 and opens_with_control(ctx)))
            ):
                return "unheard"
            return None

        return fate

    return make


def _late_decode() -> Callable[[FrameCtx], object]:
    def fate(ctx: FrameCtx) -> object:
        # B's receiver decodes every fourth transmission of A's 1.5 s late
        if ctx.tx == 0 and ctx.tx_n % 4 == 2:
            return ("late", 1.5)
        return None

    return fate


def _data_index(ctx: FrameCtx) -> int:
    """The frame's position among the data frames of its transmission: the same event in
    every protocol, whatever control frame opens the transmission."""
    return sum(1 for k in ctx.kinds[: ctx.i] if k == "data")


def _last_undecodable() -> Callable[[FrameCtx], object]:
    nth = Ordinals()

    def fate(ctx: FrameCtx) -> object:
        # the last data frame of each of A's first eight bursts is detected and does not
        # decode: where the burst ends has to come from the frames before it
        n = nth(ctx)
        last = max((j for j, k in enumerate(ctx.kinds) if k == "data"), default=-1)
        if ctx.tx == 0 and n is not None and n < 8 and ctx.i == last:
            return "undecodable"
        return None

    return fate


def _partial() -> Callable[[FrameCtx], object]:
    nth = Ordinals()

    def fate(ctx: FrameCtx) -> object:
        # in each of A's first eight bursts every other data frame (the 2nd, 4th, ...) is
        # detected and does not decode
        n = nth(ctx)
        if ctx.tx == 0 and ctx.kind == "data" and n is not None and n < 8 and _data_index(ctx) % 2:
            return "undecodable"
        return None

    return fate


def _chat_handover_lossy() -> Callable[[FrameCtx], object]:
    import random

    r = random.Random(77)

    def fate(ctx: FrameCtx) -> object:
        # either station's control frame that opens a transmission with data, lost 30 %
        if ctx.i == 0 and opens_with_control(ctx) and "data" in ctx.kinds and r.random() < 0.3:
            return "unheard"
        return None

    return fate


def _ramp(t: float) -> float:
    """+12 dB for a minute, down to −8 dB over a minute, a minute there, and back."""
    period = 240.0
    x = t % period
    if x < 60:
        return 12.0
    if x < 120:
        return 12.0 - 20.0 * (x - 60) / 60
    if x < 180:
        return -8.0
    return -8.0 + 20.0 * (x - 180) / 60


SCENARIOS = [
    Scenario("ack-lost", "bulk", "every 3rd acknowledgement from B unheard at A", _ack_lost),
    Scenario("data-lost", "bulk", "A's 2nd and 5th bursts entirely unheard", _data_lost),
    Scenario(
        "handover-lost",
        "exchange",
        "B's first transmission with data (the change of direction) unheard",
        _handover_lost(True),
    ),
    Scenario(
        "handover-ctl-lost",
        "exchange",
        "only the control frame opening B's first data transmission unheard",
        _handover_lost(False),
    ),
    Scenario(
        "late-decode", "bulk", "B decodes every 4th transmission of A 1.5 s late", _late_decode
    ),
    Scenario(
        "last-undecodable",
        "bulk",
        "the last data frame of A's first 8 bursts detected, never decoded",
        _last_undecodable,
    ),
    Scenario(
        "partial-bursts",
        "bulk",
        "every other data frame of A's first 8 bursts undecodable",
        _partial,
    ),
    Scenario(
        "chat-handover-lossy",
        "chat",
        "30 % of the control frames that open a data transmission lost",
        _chat_handover_lossy,
    ),
    Scenario(
        "phantom-storm",
        "exchange",
        "30 false detections a minute at both stations",
        tweak={"phantom_per_min": 30.0, "phantom_trusted": 0.2},
    ),
    Scenario("asymmetric-10dB", "exchange", "A hears B 10 dB worse than B hears A", asym_db=10.0),
    Scenario(
        "slow-computer",
        "exchange",
        "B's decodes 1.2 s later than any budget",
        tweak={"decode_latency_s": 1.35},
    ),
    Scenario(
        "ptt-jitter",
        "exchange",
        "B's keying 0-1.0 s later than it was told",
        tweak={"lead_jitter_s": 1.0},
    ),
    Scenario(
        "fade-ramp",
        "bulk",
        "+12 dB to -8 dB and back, every 4 minutes",
        schedule=_ramp,
        nbytes=12000,
    ),
    Scenario("outage", "exchange", "the band dead from 60 s to 100 s"),
]


def one(job: tuple[str, str, int, int, str]) -> dict[str, Any]:
    name, arm, bw, trial, channel = job
    sc = next(s for s in SCENARIOS if s.name == name)
    snr = 6.0 if bw == 2300 else 3.0
    make = builder(
        arm, bandwidth=bw, channel=channel, snr_db=snr, asym_db=sc.asym_db, profile=sc.profile
    )

    def make2(t: int) -> tuple[Any, Any, AdvSim]:
        a, b, sim = make(t)
        if sc.adversary is not None:
            sim.scenario = sc.adversary()
        if sc.schedule is not None:
            sim.snr_schedule = sc.schedule
        if sc.tweak:
            sim.st[1].cfg = replace(sim.st[1].cfg, **sc.tweak)
            if "phantom_per_min" in sc.tweak:
                sim.st[0].cfg = replace(sim.st[0].cfg, **sc.tweak)
                for who in (0, 1):
                    sim._next_phantom(who, 0.0)
        if name == "outage":
            sim.ch.outage = (60.0, 100.0)
        return a, b, sim

    row = run(make2, sc.kind, trial, nbytes=sc.nbytes, until=2400.0)
    over, classes = row.pop("_overlaps"), row.pop("_classes")
    del over
    out = {"scenario": name, "arm": arm, "bw": bw, "channel": channel, **row}
    for c in classes:
        out[f"c:{c}"] = out.get(f"c:{c}", 0) + 1
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arms", default="legacy,stated,stated6,gs,fc")
    ap.add_argument("--bandwidth", default="2300,500")
    ap.add_argument("--channel", default="good")
    ap.add_argument("--trials", type=int, default=8)
    ap.add_argument("--only", default="")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    names = [s.name for s in SCENARIOS if not args.only or s.name in args.only.split(",")]
    jobs = [
        (n, arm, int(bw), t, args.channel)
        for n in names
        for arm in args.arms.split(",")
        for bw in args.bandwidth.split(",")
        for t in range(args.trials)
    ]
    with Pool(args.jobs) as pool:
        rows = pool.map(one, jobs, chunksize=2)
    keys: list[str] = []
    for r in rows:
        keys.extend(k for k in r if k not in keys)
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=keys)
        w.writeheader()
        w.writerows(rows)
    table: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for r in rows:
        table[(r["scenario"], r["arm"])].append(r)
    arms = args.arms.split(",")
    print("scenario | " + " | ".join(f"{a}: ok, overlaps/run, median s" for a in arms))
    for n in names:
        cells = []
        for a in arms:
            rs = table[(n, a)]
            ok = sum(int(r["ok"]) for r in rs)
            ov = statistics.fmean(float(r.get("rf_overlaps") or 0) for r in rs)
            done = [float(r["done_s"]) for r in rs if int(r["ok"])]
            med = statistics.median(done) if done else float("nan")
            cells.append(f"{ok}/{len(rs)}, {ov:.2f}, {med:.0f}")
        print(f"{n} | " + " | ".join(cells))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
