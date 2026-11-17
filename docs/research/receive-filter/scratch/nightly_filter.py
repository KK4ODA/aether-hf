"""Every harness station's live receive-filter readings in a nightly's scenario artifacts: which
scenarios warned on an uncut radio, at which side and how deep, and what QRM the scenario had.

    python -I nightly_filter.py DIR
"""

import glob
import json
import os
import sys
import tomllib
from collections import Counter, defaultdict


def main():
    root = sys.argv[1]
    per = defaultdict(lambda: {"stations": 0, "warned": 0, "readings": 0, "warn_readings": 0, "max": 0.0, "sides": Counter()})
    qrm = {}
    for sidecar in glob.glob(os.path.join(root, "**", "*.json"), recursive=True):
        base = os.path.basename(sidecar)
        if base in ("result.json", "channel.json") or not base[:8].isdigit():
            continue
        try:
            with open(sidecar, encoding="utf-8") as f:
                side = json.load(f)
        except (OSError, ValueError):
            continue
        p = side.get("passband") if isinstance(side, dict) else None
        if not p:
            continue
        parts = os.path.normpath(sidecar).split(os.sep)
        run = next((parts[i] for i in range(len(parts) - 1, -1, -1) if os.path.exists(os.path.join(*parts[: i + 1], "scenario.toml"))), "?")
        rundir = None
        for i in range(len(parts) - 1, 0, -1):
            cand = os.path.join(*parts[:i])
            if os.path.exists(os.path.join(cand, "scenario.toml")):
                rundir, run = cand, parts[i - 1]
                break
        if rundir and run not in qrm:
            with open(os.path.join(rundir, "scenario.toml"), "rb") as f:
                sc = tomllib.load(f)
            kinds = [f"{q.get('kind')}@{q.get('audio_hz')}" for q in sc.get("qrm", [])]
            filt = [w for w, s in sc.get("stations", {}).items() if s.get("radio", {}).get("rx_filter_hz")]
            qrm[run] = (", ".join(kinds) or "-", ",".join(filt) or "-")
        rd = p.get("readings", [])
        e = per[run]
        e["stations"] += 1
        warn = [r for r in rd if r[2] in ("top", "bottom", "both")]
        e["readings"] += len(rd)
        e["warn_readings"] += len(warn)
        if warn:
            e["warned"] += 1
        for r in warn:
            e["sides"][r[2]] += 1
            e["max"] = max(e["max"], max(v for v in (r[3], r[4]) if v is not None))
    print(f"{len(per)} runs")
    for run, e in sorted(per.items(), key=lambda kv: (-kv[1]["warn_readings"], kv[0])):
        if e["warn_readings"] == 0 and qrm.get(run, ("-",))[0] == "-":
            continue
        q, f = qrm.get(run, ("?", "?"))
        print(f"{run:48s} stations {e['stations']} warned {e['warned']} readings {e['warn_readings']}/{e['readings']} max {e['max']:.1f} {dict(e['sides'])} | qrm {q} | cut {f}")


main()
