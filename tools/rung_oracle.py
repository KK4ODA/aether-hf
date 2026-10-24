"""The rate controller's oracle on the scenario harness: each scenario run at each fixed rung,
and the best rung in hindsight against what the controller chose (the rate controller's review,
`docs/research/rate-control/00-brief.md` §4, Gate 0).

    python tools/rung_oracle.py [scenarios…] --rungs 1,3,5,7,9,11 [--seeds N | --draws 1,2]
                                [--jobs J] [--out runs/oracle] [--daemon PATH]
                                [--csv oracle.csv] [--summary oracle_summary.csv] [--tag T]

Every run is ``tools/session_matrix.py``'s: one scenario, one draw of its fading and its
neighbours, both daemons real. ``auto`` runs the scenario as it is, the rate controller choosing;
``r<N>`` pins every station's own data at rung N (``rate.pin``). A run that has its
``result.json`` under ``--out`` is not run again, so a set can be continued after a stop. Draw
*k* is the scenario at its seed + k·1000 (``session_matrix.py --variant``); draw 0 is the
scenario's own seed, kept under the scenario's own name.

The measure of a run is what it delivered: a ``message`` step's rate (0 when the message was
lost), or a Test's file rate, or its message rate. The oracle for a scenario and seed is the
pinned rung that delivered the most; the controller's regret is that over its own, less one
(+0.5: the best fixed rung would have delivered half as much again). A fixed rung knows the path
in hindsight and never pays to find out, so the regret is a ceiling on what any controller could
gain, not a target.
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
import tomllib
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import session_matrix

ROOT = Path(__file__).resolve().parents[1]
TRANSFER = re.compile(r"(message|reply) (\d+) B (\d+) s (\d+) bps")

COLUMNS = [
    "scenario", "seed", "rung", "pass", "why", "air_s", "message_bps", "file_bps",
    "transfer_bps", "metric", "rf_overlaps", "wall_s",
]  # fmt: skip
SUMMARY_COLUMNS = [
    "scenario", "seed", "auto_metric", "best_rung", "best_metric", "regret", "rungs_run", "by_rung",
]  # fmt: skip


def transfer_bps(result: dict[str, Any]) -> float | None:
    """The largest ``message`` step's rate — a short first message is mostly the exchange's
    overhead, whatever the rung — 0 when it was lost; None when the script sent none."""
    best: tuple[int, float] | None = None
    for part in str(result.get("transfers") or "").split(";"):
        part = part.strip()
        if not part.startswith("message"):
            continue
        m = TRANSFER.match(part)
        if m:
            size, rate = int(m[2]), float(m[4])
        else:
            lost = re.match(r"message (\d+) B lost", part)
            size, rate = (int(lost[1]) if lost else 0), 0.0
        if best is None or size > best[0]:
            best = (size, rate)
    return None if best is None else best[1]


def metric(result: dict[str, Any]) -> float:
    """What the run delivered, in bit/s of air: the message step's rate, else the Test's file
    rate, else its message rate; nothing delivered is 0."""
    t = transfer_bps(result)
    if t is not None:
        return t
    for key in ("file_bps", "message_bps"):
        value = result.get(key)
        if isinstance(value, int | float):
            return float(value)
    return 0.0


def row(scenario: str, seed: int, rung: int | None, result: dict[str, Any]) -> dict[str, Any]:
    return {
        "scenario": scenario,
        "seed": seed,
        "rung": "auto" if rung is None else rung,
        "pass": result.get("pass"),
        "why": result.get("why", ""),
        "air_s": result.get("air_s"),
        "message_bps": result.get("message_bps"),
        "file_bps": result.get("file_bps"),
        "transfer_bps": transfer_bps(result),
        "metric": round(metric(result), 1),
        "rf_overlaps": result.get("rf_overlaps"),
        "wall_s": result.get("wall_s"),
    }


def summarise(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Per scenario and seed: the controller's run against the best fixed rung."""
    groups: dict[tuple[str, int], list[dict[str, Any]]] = {}
    for r in rows:
        groups.setdefault((str(r["scenario"]), int(r["seed"])), []).append(r)
    out = []
    for (scenario, seed), group in sorted(groups.items()):
        auto = next((r for r in group if r["rung"] == "auto"), None)
        pinned = [r for r in group if r["rung"] != "auto"]
        best = max(pinned, key=lambda r: float(r["metric"]), default=None)
        auto_metric = None if auto is None else float(auto["metric"])
        best_metric = None if best is None else float(best["metric"])
        regret = None
        if auto_metric is not None and best_metric is not None and auto_metric > 0:
            regret = round(best_metric / auto_metric - 1.0, 3)
        out.append(
            {
                "scenario": scenario,
                "seed": seed,
                "auto_metric": auto_metric,
                "best_rung": None if best is None else best["rung"],
                "best_metric": best_metric,
                "regret": regret,
                "rungs_run": len(pinned),
                "by_rung": " ".join(
                    f"{r['rung']}:{float(r['metric']):.0f}{'' if r['pass'] else '!'}"
                    for r in sorted(pinned, key=lambda r: int(r["rung"]))
                ),
            }
        )
    return out


def plan(
    scenarios: list[Path], rungs: list[int], draws: list[int]
) -> list[tuple[Path, int | None, int | None]]:
    """(scenario, rung or None for the controller, draw or None for the scenario's own seed) in
    the order run: the controller's run of each first, then the rungs."""
    ks: list[int | None] = [k or None for k in draws]
    out: list[tuple[Path, int | None, int | None]] = []
    for k in ks:
        for path in scenarios:
            out.append((path, None, k))
        for rung in rungs:
            for path in scenarios:
                if rung < n_rungs(path):
                    out.append((path, rung, k))
    return out


def n_rungs(path: Path) -> int:
    """How many rungs the scenario's air has: fifteen at 500 Hz, twenty at 2 300 Hz."""
    return 15 if tomllib.loads(path.read_text(encoding="utf-8")).get("bandwidth") == 500 else 20


def run_dir(out: Path, path: Path, rung: int | None, k: int | None) -> Path:
    name = tomllib.loads(path.read_text(encoding="utf-8")).get("name", path.stem)
    if k is not None:
        name = f"{name}-k{k}"
    return out / name / ("auto" if rung is None else f"r{rung}")


def seed_of(path: Path, k: int | None) -> int | None:
    if k is None:
        return None
    base = int(tomllib.loads(path.read_text(encoding="utf-8")).get("seed", 1))
    return base + k * session_matrix.SEED_STEP


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("scenarios", nargs="*", type=Path)
    ap.add_argument("--tag", help="only scenarios carrying this tag")
    ap.add_argument("--rungs", required=True, help="rungs to pin, comma separated")
    ap.add_argument("--seeds", type=int, default=1, help="draws 0 … N-1 of each scenario")
    ap.add_argument("--draws", help="which draws, comma separated (0 is the scenario's own seed)")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--daemon", type=Path, default=session_matrix.DEFAULT_DAEMON)
    ap.add_argument("--out", type=Path, default=ROOT / "runs" / "oracle")
    ap.add_argument("--csv", type=Path, help="every run, one row each")
    ap.add_argument("--summary", type=Path, help="one row per scenario and seed")
    ap.add_argument("--drop-audio", action="store_true", help="delete each run's recordings")
    args = ap.parse_args()
    paths = args.scenarios or sorted(session_matrix.SCENARIOS.glob("*.toml"))
    if args.tag:
        paths = [
            p
            for p in paths
            if args.tag in tomllib.loads(p.read_text(encoding="utf-8")).get("tags", [])
        ]
    if not paths:
        print("no scenarios", file=sys.stderr)
        return 2
    if not args.daemon.exists():
        print(f"no daemon at {args.daemon}: cargo build --release in core/", file=sys.stderr)
        return 2
    rungs = [int(r) for r in args.rungs.split(",") if r.strip()]
    args.out.mkdir(parents=True, exist_ok=True)

    def one(item: tuple[Path, int | None, int | None]) -> dict[str, Any]:
        path, rung, k = item
        where = run_dir(args.out, path, rung, k)
        done = where / "result.json"
        scenario = tomllib.loads(path.read_text(encoding="utf-8")).get("name", path.stem)
        seed = seed_of(path, k) or int(
            tomllib.loads(path.read_text(encoding="utf-8")).get("seed", 1)
        )
        if done.exists():
            result = json.loads(done.read_text(encoding="utf-8"))
        else:
            where.parent.mkdir(parents=True, exist_ok=True)
            # session_matrix names the run's directory after the scenario: run it under a
            # directory of its own and let it make <out>/<name>
            result = session_matrix.run_one(
                path,
                where.parent / f".{where.name}",
                args.daemon,
                args.daemon,
                seed_of(path, k),
                pin=rung,
            )
            made = (
                where.parent / f".{where.name}" / (scenario if k is None else f"{scenario}-s{seed}")
            )
            if where.exists():
                import shutil

                shutil.rmtree(where, ignore_errors=True)
            made.rename(where)
            (where.parent / f".{where.name}").rmdir()
            if args.drop_audio:
                for wav in where.rglob("*.wav"):
                    wav.unlink(missing_ok=True)
        r = row(scenario, seed, rung, result)
        print(
            f"{'PASS' if r['pass'] else 'FAIL'}  {scenario:40} {r['rung']!s:>5}  "
            f"{r['metric']:7.1f} bit/s  air {r['air_s']}  {r['why']}",
            flush=True,
        )
        return r

    draws = (
        [int(k) for k in args.draws.split(",") if k.strip()]
        if args.draws
        else list(range(max(1, args.seeds)))
    )
    items = plan(paths, rungs, draws)
    if args.jobs > 1:
        with ThreadPoolExecutor(args.jobs) as pool:
            rows = list(pool.map(one, items))
    else:
        rows = [one(item) for item in items]
    summary = summarise(rows)
    print()
    print(" | ".join(SUMMARY_COLUMNS))
    for s in summary:
        print(" | ".join(str(s.get(c, "")) for c in SUMMARY_COLUMNS))
    for path, columns, data in (
        (args.csv, COLUMNS, rows),
        (args.summary, SUMMARY_COLUMNS, summary),
    ):
        if path is None:
            continue
        with path.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=columns)
            w.writeheader()
            w.writerows(data)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
