"""How alike are harness runs of one scenario, seed and daemon (08-steps-0-and-1.md §1.4).

    uv run python research/link_arch/repeatability.py <runs> <runs> [<runs> …]

Each ``<runs>`` is a ``session_matrix.py --out`` directory of the same scenarios. For each
scenario: the spread of air time and transmissions, and where each run's keying first parts from
the first run's (a keying more than 1 ms apart, or by the other station).
"""

from __future__ import annotations

import json
import statistics
import sys
from pathlib import Path

Keying = tuple[str, float, str]


def timeline(run: Path) -> tuple[float, list[Keying]]:
    doc = json.loads((run / "channel.json").read_text(encoding="utf-8"))
    keyings = [
        (who, float(t["air_s"]), str(t.get("cause", "")))
        for who, station in doc["stations"].items()
        for t in station.get("transmissions", [])
    ]
    keyings.sort(key=lambda k: k[1])
    return float(doc.get("seconds", 0.0)), keyings


def parting(a: list[Keying], b: list[Keying]) -> str:
    for x, y in zip(a, b, strict=False):
        if x[0] != y[0] or abs(x[1] - y[1]) > 0.001:
            first, second = f"{x[0]} {x[1]:.3f} ({x[2]})", f"{y[0]} {y[1]:.3f} ({y[2]})"
            return f"at {min(x[1], y[1]):.3f} s: {first} vs {second}"
    if len(a) != len(b):
        return "one run keyed more after the same start"
    return "identical"


def main() -> int:
    roots = [Path(p) for p in sys.argv[1:]]
    names = sorted({d.name for r in roots for d in r.iterdir() if (d / "channel.json").exists()})
    for name in names:
        runs = [r / name for r in roots if (r / name / "channel.json").exists()]
        lines = [timeline(r) for r in runs]
        air = [s for s, _ in lines]
        count = [len(k) for _, k in lines]
        print(
            f"{name}: {len(runs)} runs, air {min(air):.1f}-{max(air):.1f} s "
            f"(median {statistics.median(air):.1f}), transmissions {min(count)}-{max(count)}"
        )
        for _, keyings in lines[1:]:
            print(f"    {parting(lines[0][1], keyings)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
