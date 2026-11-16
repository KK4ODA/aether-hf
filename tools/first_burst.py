"""The first burst after a call, read from scenario harness runs (ADR-0128's A/B).

For every session in every run under the given directories, each station's first transmission
carrying data: its blocks (sequence numbers), the rungs they went at, how many of them the station
sent again later (lost on the first copy, or their acknowledgement lost) and the air those later
copies took. Read from each station's own sidecars (``<run>/<who>/recordings/*.json``): a sender's
``sent`` frames carry the block's sequence number, so nothing has to be matched across the two
stations' clocks. A later copy counts within ``LATER_S`` of the first burst, before a sequence
number could come round again.

    python tools/first_burst.py runs/A runs/B --csv first_burst.csv
    python tools/first_burst.py --compare runs/A runs/B

``--compare A B`` pairs the two arms' runs by scenario and seed and prints the measures of
``docs/research/first-burst/00-plan.md`` §4 (F1-F6).
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

LATER_S = 600.0
"""How long after the first burst a copy of one of its blocks counts as sent again."""
FLOOR_FRAME_S = 5.36
ORDINARY_FRAME_S = 1.054
GAP_S = 0.5
"""Data frames further apart than one frame and this are in different transmissions."""


def frame_s(entry: dict) -> float:
    return FLOOR_FRAME_S if entry.get("floor") else ORDINARY_FRAME_S


def first_burst(sidecar: dict) -> dict | None:
    """The first data transmission of one station's session and what became of its blocks."""
    data = [
        e
        for e in sidecar.get("sent") or []
        if e.get("kind") == "data" and "seq" in e and not e.get("measure")
    ]
    if not data:
        return None
    data.sort(key=lambda e: e["t_s"])
    burst = [data[0]]
    for e in data[1:]:
        prev = burst[-1]
        if e["t_s"] - (prev["t_s"] + frame_s(prev)) > GAP_S:
            break
        burst.append(e)
    end = burst[-1]["t_s"] + frame_s(burst[-1])
    seqs = {e["seq"] for e in burst}
    later = [e for e in data[len(burst) :] if e["seq"] in seqs and e["t_s"] - end < LATER_S]
    again = {e["seq"] for e in later}
    return {
        "blocks": len(seqs),
        "again": len(again),
        "again_air_s": round(sum(frame_s(e) for e in later), 2),
        "rungs": sorted({e["mode"] for e in burst}),
        "start_s": round(burst[0]["t_s"], 2),
    }


def read_run(run: Path) -> dict | None:
    result_path = run / "result.json"
    if not result_path.exists():
        return None
    result = json.loads(result_path.read_text(encoding="utf-8"))
    bursts = []
    for sidecar_path in sorted(run.glob("*/recordings/**/*.json")):
        try:
            sidecar = json.loads(sidecar_path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(sidecar, dict) or "sent" not in sidecar:
            continue
        b = first_burst(sidecar)
        if b is not None:
            b["who"] = sidecar_path.relative_to(run).parts[0]
            bursts.append(b)
    return {
        "run": run.name,
        "scenario": result.get("base") or re.sub(r"-s\d+$", "", run.name),
        "seed": result.get("seed"),
        "pass": bool(result.get("pass")),
        "air_s": result.get("air_s"),
        "message_bps": result.get("message_bps"),
        "file_bps": result.get("file_bps"),
        "rf_overlaps": result.get("rf_overlaps") or 0,
        "invariants": result.get("invariants") or "",
        "blocks": sum(b["blocks"] for b in bursts),
        "again": sum(b["again"] for b in bursts),
        "again_air_s": round(sum(b["again_air_s"] for b in bursts), 2),
        "bursts": " ".join(
            f"{b['who']}:{'/'.join(map(str, b['rungs']))}:{b['again']}of{b['blocks']}"
            for b in bursts
        ),
    }


def read_arm(root: Path) -> dict[tuple[str, int], dict]:
    out = {}
    for result in sorted(root.glob("**/result.json")):
        r = read_run(result.parent)
        if r is not None:
            out[(r["scenario"], r["seed"])] = r
    return out


def gmean(xs: list[float]) -> float:
    return math.exp(statistics.fmean(math.log(x) for x in xs)) if xs else float("nan")


def compare(a: dict, b: dict) -> int:
    keys = sorted(set(a) & set(b))
    print(f"paired runs: {len(keys)} (A {len(a)}, B {len(b)})")
    by = defaultdict(list)
    for k in keys:
        by[k[0]].append(k)
    # F1
    blocks_a = sum(a[k]["blocks"] for k in keys)
    blocks_b = sum(b[k]["blocks"] for k in keys)
    again_a = sum(a[k]["again"] for k in keys)
    again_b = sum(b[k]["again"] for k in keys)
    share_a, share_b = again_a / max(1, blocks_a), again_b / max(1, blocks_b)
    print(
        f"F1 first-burst blocks sent again: A {again_a}/{blocks_a} = {share_a:.1%}, "
        f"B {again_b}/{blocks_b} = {share_b:.1%} (ratio {share_b / share_a if share_a else 0:.2f},"
        f" ≤ 0.67 asked); air of later copies A {sum(a[k]['again_air_s'] for k in keys):.0f} s,"
        f" B {sum(b[k]['again_air_s'] for k in keys):.0f} s"
    )
    lossy = lower = 0
    rows = []
    for sc, ks in sorted(by.items()):
        ba = sum(a[k]["blocks"] for k in ks)
        bb = sum(b[k]["blocks"] for k in ks)
        sa = sum(a[k]["again"] for k in ks) / max(1, ba)
        sb = sum(b[k]["again"] for k in ks) / max(1, bb)
        if sa >= 0.25:
            lossy += 1
            lower += sb < sa
        both = [k for k in ks if a[k]["pass"] and b[k]["pass"] and a[k]["air_s"] and b[k]["air_s"]]
        air = [b[k]["air_s"] / a[k]["air_s"] for k in both]
        msg = [
            b[k]["message_bps"] / a[k]["message_bps"]
            for k in both
            if a[k]["message_bps"] and b[k]["message_bps"]
        ]
        fil = [
            b[k]["file_bps"] / a[k]["file_bps"]
            for k in both
            if a[k]["file_bps"] and b[k]["file_bps"]
        ]
        rows.append(
            (
                sc,
                len(ks),
                sum(a[k]["pass"] for k in ks),
                sum(b[k]["pass"] for k in ks),
                sa,
                sb,
                statistics.median(air) if air else None,
                statistics.median(msg) if msg else None,
                statistics.median(fil) if fil else None,
                sum(a[k]["rf_overlaps"] for k in ks),
                sum(b[k]["rf_overlaps"] for k in ks),
            )
        )
    print(f"   scenarios with A's share ≥ 25 %: {lossy}, B lower in {lower}")
    both = [k for k in keys if a[k]["pass"] and b[k]["pass"] and a[k]["air_s"] and b[k]["air_s"]]
    print(
        f"F2 air, geometric mean of paired B/A over {len(both)} runs passed in both: "
        f"{gmean([b[k]['air_s'] / a[k]['air_s'] for k in both]):.3f} (≤ 1.02 asked)"
    )
    print("F3 scenarios whose median air ratio is over 1.15:", end=" ")
    print(", ".join(f"{r[0]} {r[6]:.2f}" for r in rows if r[6] and r[6] > 1.15) or "none")
    msg = [
        b[k]["message_bps"] / a[k]["message_bps"]
        for k in both
        if a[k]["message_bps"] and b[k]["message_bps"]
    ]
    fil = [
        b[k]["file_bps"] / a[k]["file_bps"] for k in both if a[k]["file_bps"] and b[k]["file_bps"]
    ]
    print(
        f"F4 Tests: message {gmean(msg):.3f} over {len(msg)}, file {gmean(fil):.3f} over "
        f"{len(fil)} (≥ 0.97 asked); under 0.85: "
        + (
            ", ".join(
                f"{r[0]} {'m' if r[7] and r[7] < 0.85 else 'f'} "
                f"{(r[7] if r[7] and r[7] < 0.85 else r[8]):.2f}"
                for r in rows
                if (r[7] and r[7] < 0.85) or (r[8] and r[8] < 0.85)
            )
            or "none"
        )
    )
    pa = sum(a[k]["pass"] for k in keys)
    pb = sum(b[k]["pass"] for k in keys)
    lost2 = [r[0] for r in rows if r[2] - r[3] >= 2]
    print(f"F5 passed: A {pa}, B {pb}; scenarios losing two or more: {', '.join(lost2) or 'none'}")
    new_overlap = [k for k in keys if b[k]["rf_overlaps"] > a[k]["rf_overlaps"]]
    broke = [k for k in keys if b[k]["invariants"]]
    print(
        f"F6 radiated overlaps A {sum(a[k]['rf_overlaps'] for k in keys)}, "
        f"B {sum(b[k]['rf_overlaps'] for k in keys)}; B over A on: "
        f"{', '.join(f'{s}-s{d}' for s, d in new_overlap) or 'none'}; B invariants broken: "
        f"{', '.join(f'{s}-s{d}' for s, d in broke) or 'none'}"
    )
    print()
    print(
        "| scenario | runs | pass A | pass B | again A | again B | air B/A | msg B/A | file B/A |"
        " overlaps A/B |"
    )
    print("|---|---|---|---|---|---|---|---|---|---|")

    def f(x: float | None, fmt: str = "{:.2f}") -> str:
        return "" if x is None else fmt.format(x)

    for r in rows:
        print(
            f"| {r[0]} | {r[1]} | {r[2]} | {r[3]} | {r[4]:.0%} | {r[5]:.0%} | {f(r[6])} | "
            f"{f(r[7])} | {f(r[8])} | {r[9]}/{r[10]} |"
        )
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("roots", nargs="+", type=Path)
    ap.add_argument("--csv", type=Path)
    ap.add_argument("--compare", action="store_true", help="pair two arms: A then B")
    args = ap.parse_args()
    if args.compare:
        if len(args.roots) != 2:
            ap.error("--compare takes two directories, A then B")
        return compare(read_arm(args.roots[0]), read_arm(args.roots[1]))
    rows = []
    for root in args.roots:
        for r in read_arm(root).values():
            rows.append({"arm": str(root), **r})
    if args.csv:
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            w = csv.DictWriter(fh, fieldnames=list(rows[0]))
            w.writeheader()
            w.writerows(rows)
    for r in rows:
        print(r["arm"], r["run"], r["pass"], r["air_s"], r["bursts"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
