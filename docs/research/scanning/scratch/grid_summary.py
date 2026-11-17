"""The S3/S4 grid's runs, cell by cell, judged again from each run's own account of the air.

Each run directory holds ``result.json`` (the matrix's judgement) and ``channel.json`` (the air).
S1 is judged again here with ``tools/channel_server.py``'s ``dial_faults``, so a run made with an
earlier judge is judged by today's. A cell is a scenario name less its phase (``-p<k>``).

    python -m uv run python grid_summary.py RUNS_DIR [RUNS_DIR …] [--csv OUT]
"""

import json
import re
import statistics
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "tools"))
from channel_server import dial_faults  # noqa: E402


def call_s(transfers: str) -> float | None:
    found = re.search(r"call a ([\d.]+) s", transfers or "")
    return float(found.group(1)) if found else None


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    out = sys.argv[sys.argv.index("--csv") + 1] if "--csv" in sys.argv else None
    cells: dict[str, list[dict]] = defaultdict(list)
    for runs in args:
        if out and runs == out:
            continue
        for result in sorted(Path(runs).glob("*/result.json")):
            r = json.loads(result.read_text())
            air = json.loads((result.parent / "channel.json").read_text())
            faults = dial_faults(air)
            name = r["scenario"]
            cell = re.sub(r"-p\d+$", "", name)
            failures = [f for f in (r.get("failures") or "").split("; ") if f]
            failures = [f for f in failures if not f.startswith("dial: ")]
            failures += [f"dial: {f}" for f in faults]
            cells[cell].append(
                {
                    "name": name,
                    "pass": not failures,
                    "connected": "did not connect" not in failures,
                    "dial_faults": len(faults),
                    "collisions": int(r.get("collisions") or 0),
                    "rf_overlaps": int(r.get("rf_overlaps") or 0),
                    "call_s": call_s(r.get("transfers", "")),
                    "why": "; ".join(failures),
                }
            )
    rows = []
    print("cell | runs | connected | pass | dial faults | collisions | call median s | call max s")
    for cell, runs in sorted(cells.items()):
        calls = [r["call_s"] for r in runs if r["call_s"] is not None]
        row = {
            "cell": cell,
            "runs": len(runs),
            "connected": sum(r["connected"] for r in runs),
            "pass": sum(r["pass"] for r in runs),
            "dial_faults": sum(r["dial_faults"] for r in runs),
            "collisions": sum(r["collisions"] for r in runs),
            "call_median_s": round(statistics.median(calls), 1) if calls else None,
            "call_max_s": max(calls) if calls else None,
        }
        rows.append(row)
        print(" | ".join(str(v) for v in row.values()))
        for r in runs:
            if not r["pass"]:
                print(f"    {r['name']}: {r['why']}")
    if out:
        with open(out, "w", encoding="utf-8") as f:
            f.write(",".join(rows[0]) + "\n")
            for row in rows:
                f.write(",".join("" if v is None else str(v) for v in row.values()) + "\n")


if __name__ == "__main__":
    main()
