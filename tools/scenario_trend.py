"""Compare a night of scenario runs with the committed baseline, and say what got slower.

    python tools/scenario_trend.py results.csv [more.csv…]
                                   [--baseline bench/baselines/scenarios.csv]
                                   [--summary FILE] [--out FILE]
                                   [--tolerance 0.3] [--min-air-s 30]
    python tools/scenario_trend.py results.csv [more.csv…] --update [--build SHA]

``session_matrix.py`` judges each run on its own: did it connect, deliver, end cleanly. A
change that makes every session a third slower passes all of that. This reads the runs'
CSVs (``--csv``; several seeds of each scenario, ``--seeds`` / ``--variant``), takes each
scenario's median air time and Test rates over its passing runs and its pass count, and
compares them with ``bench/baselines/scenarios.csv``:

* **failing** — fewer of its runs passed than the baseline's share, by more than a third;
* **slower** — the median air time grew by more than ``--tolerance`` (30 %) *and* by more
  than ``--min-air-s`` (30 s), or a Test's message or file rate fell by more than the
  tolerance;
* **faster** — the same the other way: not a fault, a reason to refresh the baseline.

Runs of one build differ (the daemons' threads), and one draw of a fading path is one draw,
so the thresholds are wide and the medians are over seeds; what they catch is a step, not a
drift. The summary is Markdown (``--summary``, the job summary in CI); the exit status is 1
when anything is failing or slower.

``--update`` writes the baseline from the runs given (the commit's short hash with
``--build``). Refresh it when a change is meant to alter the timing (an engine default, a new
rung) and say so in the commit: the baseline is the claim of how fast the sessions are.
"""

from __future__ import annotations

import argparse
import csv
import os
import re
import statistics
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASELINE = ROOT / "bench" / "baselines" / "scenarios.csv"
COLUMNS = [
    "scenario",
    "runs",
    "passed",
    "air_s",
    "message_bps",
    "file_bps",
    "highest_rung",
    "build",
]
VARIANT = re.compile(r"-s\d+$")


@dataclass
class Night:
    """One scenario's runs, summarised."""

    scenario: str
    runs: int
    passed: int
    air_s: float | None
    message_bps: float | None
    file_bps: float | None
    highest_rung: float | None

    @property
    def share(self) -> float:
        return self.passed / self.runs if self.runs else 0.0


def number(text: str | None) -> float | None:
    if text in (None, "", "None"):
        return None
    try:
        return float(text)  # type: ignore[arg-type]
    except ValueError:
        return None


def median(values: list[float | None]) -> float | None:
    present = [v for v in values if v is not None]
    return round(statistics.median(present), 1) if present else None


def summarise(rows: list[dict[str, str]]) -> dict[str, Night]:
    groups: dict[str, list[dict[str, str]]] = {}
    for row in rows:
        base = row.get("base") or VARIANT.sub("", row["scenario"])
        groups.setdefault(base, []).append(row)
    out = {}
    for name, runs in sorted(groups.items()):
        good = [r for r in runs if r.get("pass") == "True"]
        out[name] = Night(
            scenario=name,
            runs=len(runs),
            passed=len(good),
            air_s=median([number(r.get("air_s")) for r in good]),
            message_bps=median([number(r.get("message_bps")) for r in good]),
            file_bps=median([number(r.get("file_bps")) for r in good]),
            highest_rung=median([number(r.get("highest_rung")) for r in good]),
        )
    return out


def read_rows(paths: list[Path]) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for path in paths:
        with path.open(newline="", encoding="utf-8") as f:
            rows += list(csv.DictReader(f))
    return rows


def read_baseline(path: Path) -> dict[str, Night]:
    if not path.exists():
        return {}
    with path.open(newline="", encoding="utf-8") as f:
        return {
            r["scenario"]: Night(
                scenario=r["scenario"],
                runs=int(r["runs"]),
                passed=int(r["passed"]),
                air_s=number(r["air_s"]),
                message_bps=number(r["message_bps"]),
                file_bps=number(r["file_bps"]),
                highest_rung=number(r["highest_rung"]),
            )
            for r in csv.DictReader(f)
        }


def write_baseline(path: Path, nights: dict[str, Night], build: str) -> None:
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(COLUMNS)
        for n in nights.values():
            w.writerow(
                [
                    n.scenario,
                    n.runs,
                    n.passed,
                    *("" if v is None else v for v in (n.air_s, n.message_bps, n.file_bps)),
                    "" if n.highest_rung is None else n.highest_rung,
                    build,
                ]
            )


def verdict(now: Night, base: Night, tolerance: float, min_air_s: float) -> tuple[str, list[str]]:
    """``failing``, ``slower``, ``faster`` or ``same``, and the reasons."""
    why_bad: list[str] = []
    why_good: list[str] = []
    if base.runs and now.share < base.share - 1 / 3:
        why_bad.append(f"passed {now.passed}/{now.runs} (baseline {base.passed}/{base.runs})")
        return "failing", why_bad
    if now.air_s is not None and base.air_s is not None:
        grew = now.air_s - base.air_s
        if grew > min_air_s and now.air_s > base.air_s * (1 + tolerance):
            why_bad.append(f"air {base.air_s:.0f} → {now.air_s:.0f} s")
        elif -grew > min_air_s and now.air_s < base.air_s * (1 - tolerance):
            why_good.append(f"air {base.air_s:.0f} → {now.air_s:.0f} s")
    for label, a, b in (
        ("message", base.message_bps, now.message_bps),
        ("file", base.file_bps, now.file_bps),
    ):
        if a is None or b is None or a <= 0:
            continue
        if b < a * (1 - tolerance):
            why_bad.append(f"{label} {a:.0f} → {b:.0f} bit/s")
        elif b > a * (1 + tolerance):
            why_good.append(f"{label} {a:.0f} → {b:.0f} bit/s")
    if why_bad:
        return "slower", why_bad
    if why_good:
        return "faster", why_good
    return "same", []


def cell(v: float | None, unit: str = "") -> str:
    return "—" if v is None else f"{v:.0f}{unit}"


def report(
    nights: dict[str, Night], baseline: dict[str, Night], tolerance: float, min_air_s: float
) -> tuple[str, list[tuple[str, str, list[str]]]]:
    flagged: list[tuple[str, str, list[str]]] = []
    lines = [
        "| scenario | passed | air s (base) | message bit/s (base) | file bit/s (base) | |",
        "|---|---|---|---|---|---|",
    ]
    for name, now in nights.items():
        base = baseline.get(name)
        if base is None:
            kind, why = "new", []
        else:
            kind, why = verdict(now, base, tolerance, min_air_s)
        if kind in ("failing", "slower", "faster"):
            flagged.append((name, kind, why))
        b = base or Night(name, 0, 0, None, None, None, None)
        lines.append(
            f"| {name} | {now.passed}/{now.runs} | {cell(now.air_s)} ({cell(b.air_s)}) "
            f"| {cell(now.message_bps)} ({cell(b.message_bps)}) "
            f"| {cell(now.file_bps)} ({cell(b.file_bps)}) | {kind if kind != 'same' else ''} |"
        )
    missing = sorted(set(baseline) - set(nights))
    head = ["## Scenarios against the baseline", ""]
    bad = [f for f in flagged if f[1] in ("failing", "slower")]
    good = [f for f in flagged if f[1] == "faster"]
    if not bad:
        head.append(f"Nothing slower or failing more than the baseline ({len(nights)} scenarios).")
    for name, kind, why in bad:
        head.append(f"- **{kind}**: `{name}` — {'; '.join(why)}")
    for name, _, why in good:
        head.append(f"- faster: `{name}` — {'; '.join(why)} (refresh the baseline if meant)")
    if missing:
        head.append(f"- not run tonight: {', '.join(missing)}")
    head += ["", "<details><summary>Every scenario</summary>", "", *lines, "", "</details>", ""]
    return "\n".join(head), flagged


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("results", nargs="+", type=Path)
    ap.add_argument("--baseline", type=Path, default=BASELINE)
    ap.add_argument("--summary", type=Path, help="append the Markdown report here")
    ap.add_argument("--out", type=Path, help="write the Markdown report here")
    ap.add_argument("--tolerance", type=float, default=0.3)
    ap.add_argument("--min-air-s", type=float, default=30.0)
    ap.add_argument("--update", action="store_true", help="write the baseline from these runs")
    ap.add_argument("--build", help="the build the runs were of (default: git HEAD)")
    args = ap.parse_args()
    nights = summarise(read_rows([p for p in args.results if p.exists()]))
    if not nights:
        print("no runs", file=sys.stderr)
        return 2
    if args.update:
        build = (
            args.build
            or subprocess.run(
                ["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, check=False
            ).stdout.strip()
        )
        write_baseline(args.baseline, nights, build)
        print(f"wrote {args.baseline}: {len(nights)} scenarios")
        return 0
    text, flagged = report(nights, read_baseline(args.baseline), args.tolerance, args.min_air_s)
    print(text)
    if args.out:
        args.out.write_text(text + "\n", encoding="utf-8")
    if args.summary:
        with args.summary.open("a", encoding="utf-8") as f:
            f.write(text + "\n")
    bad = [f for f in flagged if f[1] in ("failing", "slower")]
    if os.environ.get("GITHUB_ACTIONS"):
        for name, kind, why in bad:
            print(f"::warning title={kind}: {name}::{'; '.join(why)}")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
