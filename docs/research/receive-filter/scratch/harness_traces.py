"""Every station recording of a harness out dir: its live readings and each daemon's trace over
it (with its sidecar), as warnings on a radio that cuts nothing (H(a), 05-busy-band.md §7).

    python3 harness_traces.py RUNS_DIR DAEMON[,DAEMON...]
"""
import csv
import json
import subprocess
import sys
import tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

WARN = {"top", "bottom", "both"}


def summary(rows):
    warned = [r for r in rows if r[0] in WARN]
    drops = [d for r in warned for d in r[1:] if d is not None]
    return len(warned), len(rows), max(drops) if drops else None


def live(side):
    r = (side.get("passband") or {}).get("readings") or []
    return summary([(x[2], x[3], x[4]) for x in r])


def traced(daemon, wav):
    out = subprocess.run([daemon, "--passband-trace", str(wav), "--expect", str(wav.with_suffix(".json"))],
                         capture_output=True, text=True, check=True).stdout
    f = lambda v: float(v) if v else None
    return summary([(r["verdict"], f(r["low_drop_db"]), f(r["high_drop_db"]))
                    for r in csv.DictReader(out.splitlines())])


def main():
    runs, daemons = Path(sys.argv[1]), sys.argv[2].split(",")
    work = []
    for run in sorted(p for p in runs.iterdir() if (p / "scenario.toml").exists()):
        sc = tomllib.loads((run / "scenario.toml").read_text())
        for who_dir in sorted(p for p in run.iterdir() if (p / "recordings").is_dir()):
            radio = sc.get("stations", {}).get(who_dir.name, {}).get("radio", {})
            if radio.get("rx_filter_hz"):
                continue
            for wav in sorted((who_dir / "recordings").glob("*.wav")):
                work.append((run.name, who_dir.name, wav))
    def one(item):
        run, who, wav = item
        side = json.loads(wav.with_suffix(".json").read_text())
        return item, live(side), [traced(d, wav) for d in daemons]
    with ThreadPoolExecutor(4) as pool:
        got = list(pool.map(one, work))
    names = [Path(d).name for d in daemons]
    totals = {n: [0, 0] for n in ["live"] + names}
    fmt = lambda s: f"{s[0]}/{s[1]}" + (f" ≤{s[2]:.1f}" if s[2] is not None else "")
    for (run, who, wav), lv, tr in got:
        print(f"{run:40s} {who} live {fmt(lv):14s} " + " ".join(f"{n} {fmt(t):14s}" for n, t in zip(names, tr)))
        for n, s in zip(["live"] + names, [lv] + tr):
            totals[n][0] += bool(s[0]); totals[n][1] += 1
    print("stations warned: " + ", ".join(f"{n} {a} of {b}" for n, (a, b) in totals.items()))


main()
