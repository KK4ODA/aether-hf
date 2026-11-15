"""Calls and probes on the fading pipe: how often a call connects and a probe is answered.

A session's throughput (`bench_link.py`) says little about the start of a contact: a call that
takes three tries to connect and a probe that is never answered both vanish into a session's
total. This runs two link engines (`GrantedEngine`, ADR-0069) through the link bench's fading
pipe
(`bench_link.py --fading`, `bench/baselines/fading_pipe.csv`) and records, per air, channel
class and SNR, one call — connected or not within 400 s, when, after how many tries — and one
probe — answered or not within 120 s — per trial (ADR-0016).

`--tree` runs another checkout's model and link bench against this calibration: that is how the
"before" rows of `bench/baselines/calls.csv` come from a worktree of an earlier release, which
has no copy of this tool. The rows there are protocol 7's turn engine, retired in ADR-0069's
Step 5; a tree run now needs the granted engine (2026-10-13 on).

    python tools/bench_calls.py --variant "calls on the floor" --out calls.csv
    python tools/bench_calls.py --tree C:/Dev/aether-before --variant beta.54 --out before.csv

`--wait` sets the random wait after a call's try (`docs/research/calling/`): `growing` (today's,
`1 + tries` floor frames), `short` (one floor frame), `none`, or a number of floor frames;
`--calls floor` puts every try of a call on the tone floor (today's: `alternate`).

`--calls capped` is the capped call (ADR-0126, proposed): its tries chosen by `--heard` — `none`
(not heard lately: two both-ways tries), `true` (heard at the cell's SNR) or a number of dB — and
a call counts as connected once the called station has the caller's details too. `--again`
calls a second time when the first call gives up, and records the pair's outcome beside the
first's. `--step` is the bench's clock (1 s by default); `--what call` leaves the probes out;
`ended` is when a call that did not connect gave up, `up` when both stations were in the session.
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


WAITS = {"growing": None, "short": 1.0, "none": 0.0}


def point(
    job: tuple[str, int, str, float, int, float | None, str, str, bool, float, str],
) -> list[dict[str, object]]:
    """Every trial of one (air, class, SNR): a call and a probe, each on a fresh pair."""
    tree, bandwidth, channel, snr, trials, wait, calls, heard, again, step, what_ = job
    floor_only = calls == "floor"
    capped = calls == "capped"
    sys.path.insert(0, str(Path(tree) / "model"))
    sys.path.insert(0, str(Path(tree) / "tools"))
    import bench_link as bench
    from aether_model.frame.modes import NARROW, WIDE
    from aether_model.link.granted import GrantConfig, GrantedEngine
    from aether_model.link.harness import bandwidth_capabilities, phy_timing
    from aether_model.link.sim import TwoStationSim, control_thresholds_for

    air = WIDE if bandwidth == 2300 else NARROW
    timing = phy_timing(air.params)
    rows: list[dict[str, object]] = []

    def config() -> GrantConfig:
        """Each station its own, stating its air's capability byte: the default reads as
        2300 Hz, and both stations of a 500 Hz point had run as such."""
        cfg = GrantConfig(max_mode=air.n_rungs - 1, capabilities=bandwidth_capabilities(air.params))
        if wait is not None:
            cfg.call_wait_frames = wait
        if floor_only:
            cfg.call_floor_only = True
        if capped:
            cfg.capped_call = True
        return cfg

    heard_db = None if heard == "none" else (snr if heard == "true" else float(heard))

    def keyed(sim: TwoStationSim, until: float) -> float:
        return sum(max(0.0, min(b, until) - a) for a, b in sim.radiated(0) if a < until)

    for trial in range(trials):
        seed = 1000 * trial + 17
        for what in ("call", "probe") if what_ == "both" else (what_,):
            a = GrantedEngine("W4ODA", timing, config(), seed=seed)
            b = GrantedEngine("KK4XYZ", timing, config(), seed=seed + 1)
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
            ended = up = ok2 = took2 = plan = None
            spent = 0.0
            if what == "call":
                if capped:
                    a.connect("KK4XYZ", heard_db=heard_db)
                else:
                    a.connect("KK4XYZ")
                plan = a._call_plan or None
                calls_made = 1
                for k in range(1, round(CALL_S / step) + 1):
                    t = round(k * step, 6)
                    sim.run(until=t)
                    if up is None and a.connected and b.connected:
                        up = t
                    if a.connected and b.connected and b.identified:
                        if calls_made == 1:
                            ok, took, tries = True, t, a._connect_tries
                            spent = keyed(sim, t)
                        ok2, took2 = True, t
                        break
                    if not a.connected and a.state.name == "IDLE":
                        if calls_made == 1:
                            ended = t
                            spent = keyed(sim, t)
                        if again and calls_made == 1:
                            calls_made = 2
                            up = None
                            if capped:
                                a.connect("KK4XYZ", heard_db=heard_db)
                            else:
                                a.connect("KK4XYZ")
                            continue
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
                    **(
                        {
                            "plan": plan or "",
                            "up": "" if up is None or not ok else up,
                            "ended": "" if ended is None else ended,
                            "keyed_s": round(spent, 2),
                            "ok2": int(bool(ok2)) if again else "",
                            "seconds2": "" if took2 is None or not again else took2,
                        }
                        if what == "call" and (capped or again or step != 1.0)
                        else {}
                    ),
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
    ap.add_argument("--wait", default="growing", help="growing, short, none, or floor frames")
    ap.add_argument("--calls", default="alternate", choices=("alternate", "floor", "capped"))
    ap.add_argument("--heard", default="none", help="capped: none, true, or dB")
    ap.add_argument("--again", action="store_true", help="a second call when the first fails")
    ap.add_argument("--step", type=float, default=1.0, help="the bench's clock, seconds")
    ap.add_argument("--what", default="both", choices=("both", "call", "probe"))
    args = ap.parse_args()
    wait = WAITS[args.wait] if args.wait in WAITS else float(args.wait)
    jobs = [
        (
            args.tree,
            int(bw),
            channel,
            float(snr),
            args.trials,
            wait,
            args.calls,
            args.heard,
            args.again,
            args.step,
            args.what,
        )
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
                f"  call {len(took):2d}/{len(calls)} median {median:5.1f} s"
                f"  probe {answered:2d}/{len(calls)}",
                flush=True,
            )
    fields = list(dict.fromkeys(k for row in out for k in row))
    with Path(args.out).open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
