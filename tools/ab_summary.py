"""The A/B bench's results, side by side (roadmap P9-1).

    python tools/ab_summary.py [bench/ab/results.csv]

One line per (profile, SNR): each modem's mean goodput over its runs, how many runs, and the
ratio of the second modem to the first. A point one modem has not run yet shows a dash.
"""

from __future__ import annotations

import csv
import statistics
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PROFILE_ORDER = ("awgn", "good", "moderate", "poor")


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "bench" / "ab" / "results.csv"
    rows = list(csv.DictReader(path.open(encoding="utf-8")))
    if not rows:
        print(f"{path}: no runs yet")
        return 0
    modems = sorted({row["modem"] for row in rows})
    points: dict[tuple[str, float], dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    failed: dict[tuple[str, float], dict[str, int]] = defaultdict(lambda: defaultdict(int))
    for row in rows:
        key = (row["profile"], float(row["snr_db"]))
        if row.get("goodput_bps"):
            points[key][row["modem"]].append(float(row["goodput_bps"]))
        else:
            failed[key][row["modem"]] += 1

    def order(key: tuple[str, float]) -> tuple[int, float]:
        profile, snr = key
        return (PROFILE_ORDER.index(profile) if profile in PROFILE_ORDER else 9, snr)

    head = f"{'profile':9s} {'SNR':>6s}"
    for modem in modems:
        head += f"  {modem:>16s}"
    if len(modems) == 2:
        head += f"  {modems[1] + '/' + modems[0]:>14s}"
    print(head)
    for key in sorted(set(points) | set(failed), key=order):
        profile, snr = key
        line = f"{profile:9s} {snr:+6.1f}"
        means: list[float | None] = []
        for modem in modems:
            runs = points[key].get(modem, [])
            lost = failed[key].get(modem, 0)
            if runs:
                mean = statistics.mean(runs)
                means.append(mean)
                line += f"  {mean:8.0f} b/s ×{len(runs)}"
                if lost:
                    line += f"-{lost}"
            else:
                means.append(None)
                line += f"  {'—':>12s}" + (f" ×0-{lost}" if lost else "    ")
        if len(modems) == 2 and means[0] and means[1] is not None:
            line += f"  {means[1] / means[0]:14.2f}"
        print(line)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
