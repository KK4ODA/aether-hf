"""Calls and probes on the fading pipe: how often a call connects and a probe is answered.

A session's throughput (`bench_link.py`) says little about the start of a contact: a call that
takes three tries to connect and a probe that is never answered both vanish into a session's
total. This runs the two engines of `LinkEngine` through the link bench's fading pipe
(`bench_link.py --fading`, `bench/baselines/fading_pipe.csv`) and records, per air, channel
class and SNR, one call — connected or not within 400 s, when, after how many tries — and one
probe — answered or not within 120 s — per trial (ADR-0016).

`--tree` runs another checkout's model and link bench against this calibration: that is how the
"before" rows of `bench/baselines/calls.csv` come from a worktree of an earlier release, which
has no copy of this tool.

    python tools/bench_calls.py --variant "calls on the floor" --out calls.csv
    python tools/bench_calls.py --tree C:/Dev/aether-before --variant beta.54 --out before.csv
"""

from __future__ import annotations

import argparse
import csv
import statistics
import sys
from multiprocessing import Pool
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
CALIBRATION = HERE / "bench" / "baselines" / "fading_pipe.csv"
CALL_S = 400
PROBE_S = 120


def point(job: tuple[str, int, str, float, int]) -> list[dict[str, object]]:
    """Every trial of one (air, class, SNR): a call and a probe, each on a fresh pair."""
    tree, bandwidth, channel, snr, trials = job
    sys.path.insert(0, str(Path(tree) / "model"))
    sys.path.insert(0, str(Path(tree) / "tools"))
    import bench_link as bench
    from aether_model.frame.modes import NARROW, WIDE
    from aether_model.link.engine import LinkConfig, LinkEngine
    from aether_model.link.harness import phy_timing
    from aether_model.link.sim import TwoStationSim, control_thresholds_for

    air = WIDE if bandwidth == 2300 else NARROW
    timing = phy_timing(air.params)
    rows: list[dict[str, object]] = []
    for trial in range(trials):
        seed = 1000 * trial + 17
        for what in ("call", "probe"):
            cfg = LinkConfig(max_mode=air.n_rungs - 1)
            a = LinkEngine("W4ODA", timing, cfg, seed=seed)
            b = LinkEngine("KK4XYZ", timing, cfg, seed=seed + 1)
            sim = TwoStationSim(
                a,
                b,
                snr_db=snr,
                seed=seed,
                thresholds=bench.table_for(air)[0],
                control_thresholds=control_thresholds_for(timing),
                fading=bench.fading_pipe(CALIBRATION, air, channel, seed),
            )
            ok, took, tries = False, None, None
            if what == "call":
                a.connect("KK4XYZ")
                for t in range(1, CALL_S + 1):
                    sim.run(until=float(t))
                    if a.connected and b.connected:
                        ok, took, tries = True, float(t), a._connect_tries
                        break
                    if not a.connected and a.state.name == "IDLE":
                        break
            else:
                a.probe("KK4XYZ")
                for t in range(1, PROBE_S + 1):
                    sim.run(until=float(t))
                    if not a.probing:
                        break
                ok = a.last_probe is not None
                took = float(t) if ok else None
            rows.append(
                {
                    "bandwidth_hz": bandwidth,
                    "channel": channel,
                    "snr_db": snr,
                    "what": what,
                    "trial": trial,
                    "ok": int(ok),
                    "seconds": "" if took is None else took,
                    "tries": "" if tries is None else tries,
                }
            )
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--tree", default=str(HERE), help="the checkout whose model to run")
    ap.add_argument("--variant", required=True, help="the label every row carries")
    ap.add_argument("--out", required=True)
    ap.add_argument("--bandwidth", default="2300,500")
    ap.add_argument("--channels", default="awgn,good,moderate,poor")
    ap.add_argument("--snr", default="-16,-12,-8,-4,0,4,8")
    ap.add_argument("--trials", type=int, default=30)
    ap.add_argument("--jobs", type=int, default=2)
    args = ap.parse_args()
    jobs = [
        (args.tree, int(bw), channel, float(snr), args.trials)
        for bw in args.bandwidth.split(",")
        for channel in args.channels.split(",")
        for snr in args.snr.split(",")
    ]
    out: list[dict[str, object]] = []
    with Pool(args.jobs) as pool:
        for rows in pool.imap(point, jobs):
            out += [{"variant": args.variant, **row} for row in rows]
            calls = [r for r in rows if r["what"] == "call"]
            took = [float(str(r["seconds"])) for r in calls if r["ok"]]
            answered = sum(int(str(r["ok"])) for r in rows if r["what"] == "probe")
            first = rows[0]
            median = statistics.median(took) if took else float("nan")
            print(
                f"{first['bandwidth_hz']:>4} {first['channel']:8s} {float(str(first['snr_db'])):+5.0f}"
                f"  call {len(took):2d}/{len(calls)} median {median:4.0f} s"
                f"  probe {answered:2d}/{len(calls)}",
                flush=True,
            )
    with Path(args.out).open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(out[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
