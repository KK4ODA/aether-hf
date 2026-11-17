"""Q1's judged clips (05-busy-band.md §7), traced: whether each warned only while the station
warmed up (every warning before WARM_S) or after it too, for each daemon given.

    python3 q1_timing.py CORPUS JUDGE_CSV DAEMON[,DAEMON...] [WARM_S]
"""
import csv
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

WARN = {"top", "bottom", "both"}
SILENT_DB = 10.0


def parts(clip):
    p = clip.split("-")
    return next(x for x in p if x in ("moderate", "strong")), next(x for x in p if x in ("own", "half"))


def warnings(daemon, wav, bw):
    out = subprocess.run([daemon, "--passband-trace", str(wav), "--bandwidth", str(bw)],
                         capture_output=True, text=True, check=True).stdout
    return [float(r["t_s"]) for r in csv.DictReader(out.splitlines()) if r["verdict"] in WARN]


def main():
    corpus, table, daemons = Path(sys.argv[1]), sys.argv[2], sys.argv[3].split(",")
    warm = float(sys.argv[4]) if len(sys.argv) > 4 else 35.0
    rows = [r for r in csv.DictReader(open(table, encoding="utf-8")) if r["kind"] == "qrm"]
    silent = [r for r in rows if not r["need"] and float(r["loss_low"]) <= SILENT_DB
              and float(r["loss_high"]) <= SILENT_DB]
    for daemon in daemons:
        groups = {"judged": [], "strong own (reported)": []}
        jobs = []
        for r in silent:
            g = "strong own (reported)" if parts(r["clip"]) == ("strong", "own") else "judged"
            jobs.append((g, r))
        with ThreadPoolExecutor(4) as pool:
            got = list(pool.map(lambda j: warnings(daemon, corpus / f"{j[1]['clip']}.wav", j[1]["bandwidth"]), jobs))
        for (g, r), times in zip(jobs, got):
            groups[g].append((r, times))
        print(f"{Path(daemon).name}:")
        for g, items in groups.items():
            any_ = [x for x in items if x[1]]
            early = [x for x in any_ if max(x[1]) < warm]
            late = [x for x in any_ if max(x[1]) >= warm]
            print(f"  {g}: {len(any_)} of {len(items)} warn; {len(early)} only before {warm:.0f} s,"
                  f" {len(late)} after it too")
            for r, t in sorted(late, key=lambda x: (x[0]['clip'], x[0]['bandwidth'])):
                after = sum(1 for v in t if v >= warm)
                print(f"     after {warm:.0f} s: {r['clip']}@{r['bandwidth']}: {after} readings, last {max(t):.1f} s")


main()
