"""Where a session's time goes: radiated air, keying overhead and dead air, per arm.

    uv run python -m research.link_arch.where_time_goes --out research/link_arch/results/time.csv

The completion-time ratios of the main campaign say *how much* slower an arm is; this says
*why*. A session's time up to its last delivery is split into

* radiated air — some transmitter radiating a frame (both stations, never both at once
  for the prototypes),
* keying overhead — a transmitter keyed but not yet (or no longer) radiating: the lead,
  the card's queue and the tail,
* dead air — nobody keyed: decode latency, the waits each protocol reserves, holds.

Dead air per change of direction is the number that separates "answers when it decodes"
from "answers in a slot reserved for the worst decode". The ``gs0`` arm is granted slots
told the truth about an ideal station (no decode budget, no slack): what the structure
costs with nothing reserved. It is only safe where the timing really is ideal, and is run
only there.
"""

from __future__ import annotations

import argparse
import csv
import itertools
import statistics
from multiprocessing import Pool
from typing import Any

from research.link_arch import setups
from research.link_arch.setups import builder
from research.link_arch.workload import Driver, script


def _union(spans: list[tuple[float, float]], end: float) -> float:
    total, cur_a, cur_b = 0.0, None, None
    for a, b in sorted(spans):
        a, b = min(a, end), min(b, end)
        if b <= a:
            continue
        if cur_b is None or a > cur_b:
            if cur_b is not None and cur_a is not None:
                total += cur_b - cur_a
            cur_a, cur_b = a, b
        else:
            cur_b = max(cur_b, b)
    if cur_a is not None and cur_b is not None:
        total += cur_b - cur_a
    return total


def one(job: tuple[Any, ...]) -> dict[str, Any]:
    arm, bw, ch, snr, kind, prof, trial = job
    if arm == "gs0":
        setups.KNOBS["zero"] = (0.0, 0.0, 0.0, 0.0)
        knobs_profile, real_arm = "zero", "gs"
    else:
        knobs_profile, real_arm = prof, arm
    saved = setups.KNOBS.get(prof)
    if knobs_profile != prof:
        setups.KNOBS[prof] = setups.KNOBS[knobs_profile]
    try:
        make = builder(real_arm, bandwidth=bw, channel=ch, snr_db=snr, profile=prof)
        _a, b, sim = make(trial)
    finally:
        if saved is not None:
            setups.KNOBS[prof] = saved
    msgs = script(kind, trial, 4000)
    drv = Driver(sim, msgs, "B2BBB")
    if msgs and msgs[0].sender == 1:

        def poll_up() -> None:
            if b.state.value == "connected" and not drv.started:
                drv.started = True
                drv._send(sim.t)
            elif not drv.started:
                sim.schedule_app(sim.t + 0.25, 1, poll_up)

        sim.schedule_app(0.25, 1, poll_up)
    else:
        drv.started = True
    drv.start()
    end = sim.run(until=1500.0)
    want = [b"".join(m.body for m in msgs if m.sender == 1 - w) for w in (0, 1)]
    ok = all(sim.delivered(w) == want[w] for w in (0, 1))
    done = max((x for x in drv.arrived_at if x is not None), default=end)
    rad = _union([r.rad for r in sim.txs], done)
    keyed = _union([r.keyed for r in sim.txs], done)
    txs = [r for r in sim.txs if r.keyed[0] < done]
    turns = sum(1 for x, y in itertools.pairwise(txs) if x.who != y.who)
    return {
        "arm": arm,
        "bw": bw,
        "channel": ch,
        "snr": snr,
        "kind": kind,
        "profile": prof,
        "trial": trial,
        "ok": int(ok),
        "done_s": round(done, 2),
        "radiated_s": round(rad, 2),
        "keying_s": round(keyed - rad, 2),
        "dead_s": round(done - keyed, 2),
        "transmissions": len(txs),
        "turns": turns,
        "rf_overlaps": sum(1 for *_, r in sim.overlaps() if r > 0),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--arms", default="legacy,stated,gs,gs0")
    ap.add_argument("--profiles", default="ideal,field")
    ap.add_argument("--kinds", default="bulk,exchange")
    ap.add_argument("--trials", type=int, default=3)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    jobs = [
        j
        for j in itertools.product(
            args.arms.split(","),
            (500, 2300),
            ("awgn", "good", "moderate", "poor"),
            (-6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0),
            args.kinds.split(","),
            args.profiles.split(","),
            range(args.trials),
        )
        if not (j[0] == "gs0" and j[5] != "ideal")
    ]
    with Pool(args.jobs) as pool:
        rows = list(pool.imap_unordered(one, jobs, chunksize=4))
    with open(args.out, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)
    # the summary: medians of each share over runs every arm completed, per profile
    key = ("bw", "channel", "snr", "kind", "profile", "trial")
    by: dict[tuple[Any, ...], dict[str, dict[str, Any]]] = {}
    for r in rows:
        by.setdefault(tuple(r[k] for k in key), {})[r["arm"]] = r
    for prof in args.profiles.split(","):
        arms = [a for a in args.arms.split(",") if not (a == "gs0" and prof != "ideal")]
        print(f"\nprofile {prof}: per run, median over runs all arms completed")
        print("arm      done/legacy  radiated/legacy  keying/turn s  dead/turn s  turns")
        sets = [
            v for k, v in by.items() if k[4] == prof and all(a in v and v[a]["ok"] for a in arms)
        ]
        for a in arms:
            ratio = [v[a]["done_s"] / v["legacy"]["done_s"] for v in sets]
            rr = [v[a]["radiated_s"] / v["legacy"]["radiated_s"] for v in sets]
            kt = [v[a]["keying_s"] / max(1, v[a]["transmissions"]) for v in sets]
            dt = [v[a]["dead_s"] / max(1, v[a]["turns"]) for v in sets]
            tn = [v[a]["turns"] for v in sets]
            print(
                f"{a:8} {statistics.median(ratio):11.3f} {statistics.median(rr):16.3f} "
                f"{statistics.median(kt):14.2f} {statistics.median(dt):12.2f} "
                f"{statistics.median(tn):6.0f}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
