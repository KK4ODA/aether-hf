"""Summarise ``bench/baselines/phy_fer.csv`` as a Markdown table of minimum usable SNR.

    python tools/bench_summary.py [--csv bench/baselines/phy_fer.csv] [--fer 0.10]

For each (channel, mode) the lowest swept SNR at which FER ≤ ``fer`` is reported, with
linear interpolation between the two bracketing points; "> x" means the sweep ended
above the target. Also prints the best measured throughput per mode/channel.
"""

from __future__ import annotations

import argparse
import csv
from collections import defaultdict
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--csv", default="bench/baselines/phy_fer.csv")
    ap.add_argument("--fer", type=float, default=0.10)
    args = ap.parse_args()

    with Path(args.csv).open(encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    by: dict[tuple[str, str], list[tuple[float, float, int]]] = defaultdict(list)
    for r in rows:
        by[(r["channel"], r["mode_name"])].append(
            (float(r["snr_3k_db"]), float(r["fer"]), int(r["throughput_bps"]))
        )
    channels = sorted(
        {c for c, _ in by},
        key=lambda c: (
            ["awgn", "good", "moderate", "poor"].index(c)
            if c in ("awgn", "good", "moderate", "poor")
            else 9
        ),
    )
    modes = sorted(
        {m for _, m in by}, key=lambda m: int(next(r["mode"] for r in rows if r["mode_name"] == m))
    )

    def threshold(points: list[tuple[float, float, int]]) -> str:
        pts = sorted(points)
        prev = None
        for snr, fer, _ in pts:
            if fer <= args.fer:
                if prev is None or prev[1] == fer:
                    return f"{snr:+.1f}"
                s0, f0 = prev
                x = s0 + (f0 - args.fer) / (f0 - fer) * (snr - s0)
                return f"{x:+.1f}"
            prev = (snr, fer)
        return f"> {pts[-1][0]:+.1f}"

    print("| Mode | " + " | ".join(channels) + " |")
    print("|---|" + "---|" * len(channels))
    for m in modes:
        cells = [threshold(by[(c, m)]) if (c, m) in by else "—" for c in channels]
        print(f"| {m} | " + " | ".join(cells) + " |")
    print()
    print("Best throughput (bps) at the highest swept SNR:")
    print("| Mode | " + " | ".join(channels) + " |")
    print("|---|" + "---|" * len(channels))
    for m in modes:
        cells = [str(max(t for _, _, t in by[(c, m)])) if (c, m) in by else "—" for c in channels]
        print(f"| {m} | " + " | ".join(cells) + " |")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
