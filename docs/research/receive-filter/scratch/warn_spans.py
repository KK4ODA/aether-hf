"""When each clip warned: the spans of warning readings, for each daemon given."""
import csv
import subprocess
import sys
from pathlib import Path

WARN = {"top", "bottom", "both"}


def spans(daemon, wav, bw):
    out = subprocess.run([daemon, "--passband-trace", str(wav), "--bandwidth", str(bw)],
                         capture_output=True, text=True, check=True).stdout
    rows = list(csv.DictReader(out.splitlines()))
    s, cur = [], None
    for r in rows:
        t = float(r["t_s"])
        if r["verdict"] in WARN:
            if cur is None:
                cur = [t, t, r["verdict"], 0.0]
            cur[1] = t
            cur[3] = max(cur[3], max(float(r["low_drop_db"] or 0), float(r["high_drop_db"] or 0)))
        elif cur is not None:
            s.append(cur); cur = None
    if cur is not None:
        s.append(cur)
    last = rows[-1]
    return " ".join(f"{a:.0f}-{b:.0f}s {v} {m:.1f}" for a, b, v, m in s) or "-", f"end {last['verdict']} {last['low_drop_db']}/{last['high_drop_db']}"


def main():
    corpus = Path(sys.argv[1])
    daemons = sys.argv[2].split(",")
    for item in sys.argv[3:]:
        clip, bw = item.rsplit("@", 1)
        for d in daemons:
            sp, end = spans(d, corpus / f"{clip}.wav", bw)
            print(f"{clip}@{bw} {Path(d).name}: {sp} | {end}")


main()
