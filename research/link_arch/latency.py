"""Decode latency per frame family, from the harness's recordings (07-protocol-8.md §11).

    uv run python research/link_arch/latency.py <runs> [<runs> …]

``<runs>`` is a ``session_matrix.py --out`` directory. Every frame a daemon recorded has its end
on the air (``end_s``) and the moment it was reported to the engine (``t_s``), both on the
receiving station's audio clock: the difference is what ``D_ORD`` and ``D_FLOOR`` budget. The
harness runs in lockstep with the audio, so processing costs nothing here (08-steps-0-and-1.md
§1.3 says what is not counted).
"""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path

FLOOR_FRAME_S = 2.0
"""Longer than any ordinary frame (0.4–1.1 s), shorter than any tone frame (3.2 s and up)."""


def percentile(xs: list[float], q: float) -> float:
    xs = sorted(xs)
    k = (len(xs) - 1) * q
    lo, hi = int(k), min(int(k) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def bandwidth(doc: dict, scenario: str) -> int:
    bw = doc.get("bandwidth_hz") or (doc.get("session") or {}).get("bandwidth_hz")
    if bw is None:
        bw = 500 if scenario.endswith("-500") else 2300
    return int(bw)


def main() -> int:
    groups: dict[tuple[int, str, str], list[float]] = defaultdict(list)
    for root in sys.argv[1:]:
        for side in Path(root).glob("*/[ab]/recordings/*.json"):
            try:
                doc = json.loads(side.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                continue
            bw = bandwidth(doc, side.parts[-4])
            for f in doc.get("frames", []):
                if f.get("end_s") is None or f.get("start_s") is None:
                    continue
                floor = f["end_s"] - f["start_s"] > FLOOR_FRAME_S
                key = (bw, "floor" if floor else "ordinary", str(f["kind"]))
                groups[key].append(float(f["t_s"]) - float(f["end_s"]))
    print("| air | family | kind | frames | median | 99th percentile | most |")
    print("|---|---|---|---:|---:|---:|---:|")
    for bw, family, kind in sorted(groups, key=lambda k: (-k[0], k[1] == "floor", k[2])):
        xs = groups[(bw, family, kind)]
        print(
            f"| {bw} Hz | {family} | {kind} | {len(xs)} | {percentile(xs, 0.5):.3f} s "
            f"| {percentile(xs, 0.99):.3f} s | {max(xs):.3f} s |"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
