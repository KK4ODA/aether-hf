"""Compare a night of scenario runs with the committed baseline, and say what got slower.

    python tools/scenario_trend.py results.csv [more.csv…]
                                   [--baseline bench/baselines/scenarios.csv]
                                   [--summary FILE] [--out FILE]
                                   [--tolerance 0.15] [--min-air-s 30]
    python tools/scenario_trend.py results.csv [more.csv…] --update [--build SHA]

``session_matrix.py`` judges each run on its own: did it connect, deliver, end cleanly. A
change that makes every session a third slower passes all of that. This reads the runs'
CSVs (``--csv``; several seeds of each scenario, ``--seeds`` / ``--variant``), takes each
scenario's median air time and Test rates over its passing runs and its pass count, and
compares them with ``bench/baselines/scenarios.csv`` (a runs folder given in place of a CSV is
read from its runs' ``result.json``):

* **failing** — fewer of its runs passed than the baseline's share, by more than a third;
* **slower** — the median air time is beyond the *longest* the baseline's runs took, by more
  than ``--tolerance`` (15 %) *and* more than ``--min-air-s`` (30 s); or a Test's median
  message or file rate is under the *slowest* baseline run's by more than the tolerance;
* **faster** — the same the other way, past the baseline's quickest run: not a fault, a reason
  to refresh the baseline.

Runs of one build differ (the daemons' threads), and one draw of a fading path is one draw: on
the same daemon, ``40m-good-2300-rtty-crashes`` took 226, 248 and 364 s on its three seeds,
and a median of three swings across that range from night to night. Each scenario's own spread
is therefore the yardstick — the baseline keeps the range of its runs (``*_lo``, ``*_hi``), and
a night is judged against the range, not the median (the first night, 2026-10-12, flagged one
scenario slower and four faster on a build with no daemon change). What this catches is a
step, not a drift. The summary is Markdown (``--summary``, the job summary in CI); the exit status is 1
when anything is failing or slower.

``--update`` writes the baseline from the runs given (the commit's short hash with
``--build``). Refresh it when a change is meant to alter the timing (an engine default, a new
rung) and say so in the commit: the baseline is the claim of how fast the sessions are.
"""

from __future__ import annotations

import argparse
import csv
import json
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
    "air_s_lo",
    "air_s_hi",
    "message_bps_lo",
    "message_bps_hi",
    "file_bps_lo",
    "file_bps_hi",
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
    # the range over the passing runs: what one build's spread looks like
    air_s_lo: float | None = None
    air_s_hi: float | None = None
    message_bps_lo: float | None = None
    message_bps_hi: float | None = None
    file_bps_lo: float | None = None
    file_bps_hi: float | None = None

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


def span(values: list[float | None]) -> tuple[float | None, float | None]:
    present = [v for v in values if v is not None]
    return (min(present), max(present)) if present else (None, None)


def summarise(rows: list[dict[str, str]]) -> dict[str, Night]:
    groups: dict[str, list[dict[str, str]]] = {}
    for row in rows:
        base = row.get("base") or VARIANT.sub("", row["scenario"])
        groups.setdefault(base, []).append(row)
    out = {}
    for name, runs in sorted(groups.items()):
        good = [r for r in runs if r.get("pass") == "True"]
        air = [number(r.get("air_s")) for r in good]
        message = [number(r.get("message_bps")) for r in good]
        file = [number(r.get("file_bps")) for r in good]
        out[name] = Night(
            scenario=name,
            runs=len(runs),
            passed=len(good),
            air_s=median(air),
            message_bps=median(message),
            file_bps=median(file),
            highest_rung=median([number(r.get("highest_rung")) for r in good]),
        )
        night = out[name]
        night.air_s_lo, night.air_s_hi = span(air)
        night.message_bps_lo, night.message_bps_hi = span(message)
        night.file_bps_lo, night.file_bps_hi = span(file)
    return out


def read_rows(paths: list[Path]) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    for path in paths:
        if path.is_dir():
            # a runs folder: each run's result.json, which outlives a runner that died
            # before it wrote the CSV
            for result in sorted(path.glob("*/result.json")):
                data = json.loads(result.read_text(encoding="utf-8"))
                rows.append({k: "" if v is None else str(v) for k, v in data.items()})
            continue
        with path.open(newline="", encoding="utf-8") as f:
            rows += list(csv.DictReader(f))
    return rows


def read_baseline(path: Path) -> dict[str, Night]:
    if not path.exists():
        return {}
    with path.open(newline="", encoding="utf-8") as f:
        out = {}
        for r in csv.DictReader(f):
            n = Night(
                scenario=r["scenario"],
                runs=int(r["runs"]),
                passed=int(r["passed"]),
                air_s=number(r["air_s"]),
                message_bps=number(r["message_bps"]),
                file_bps=number(r["file_bps"]),
                highest_rung=number(r["highest_rung"]),
            )
            # a baseline written before the ranges were kept reads its median as its range
            for key in ("air_s", "message_bps", "file_bps"):
                for end in ("lo", "hi"):
                    value = number(r.get(f"{key}_{end}"))
                    setattr(n, f"{key}_{end}", getattr(n, key) if value is None else value)
            out[n.scenario] = n
        return out


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
                    *(
                        "" if v is None else v
                        for v in (
                            n.air_s_lo,
                            n.air_s_hi,
                            n.message_bps_lo,
                            n.message_bps_hi,
                            n.file_bps_lo,
                            n.file_bps_hi,
                        )
                    ),
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
    # tonight's median against the range of the baseline's own runs
    now_air, lo, hi = now.air_s, base.air_s_lo, base.air_s_hi
    if now_air is not None and lo is not None and hi is not None:
        if now_air - hi > min_air_s and now_air > hi * (1 + tolerance):
            why_bad.append(f"air {now_air:.0f} s, baseline runs {lo:.0f}–{hi:.0f} s")
        elif lo - now_air > min_air_s and now_air < lo * (1 - tolerance):
            why_good.append(f"air {now_air:.0f} s, baseline runs {lo:.0f}–{hi:.0f} s")
    for label, b, lo, hi in (
        ("message", now.message_bps, base.message_bps_lo, base.message_bps_hi),
        ("file", now.file_bps, base.file_bps_lo, base.file_bps_hi),
    ):
        if b is None or lo is None or hi is None or lo <= 0:
            continue
        if b < lo * (1 - tolerance):
            why_bad.append(f"{label} {b:.0f} bit/s, baseline runs {lo:.0f}–{hi:.0f}")
        elif b > hi * (1 + tolerance):
            why_good.append(f"{label} {b:.0f} bit/s, baseline runs {lo:.0f}–{hi:.0f}")
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
    ap.add_argument("--tolerance", type=float, default=0.15)
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
