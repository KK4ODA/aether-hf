"""The rate oracle's regret table, from the summaries `tools/rung_oracle.py` writes (one per
scenario job of `.github/workflows/oracle.yml`, or one for a whole local run).

    python tools/oracle_report.py oracle/*/summary.csv [--markdown FILE] [--out FILE]

Per scenario: the controller's delivery in each draw, the best fixed rung in hindsight and what
it delivered, the regret (best / controller − 1) per draw and its median over the draws. Over
the scenarios: the median and the mean of the per-scenario medians, and a bootstrap of the
median over each scenario's draws (5–95 %), as the first review reported it
(`docs/research/rate-control/03-candidates.md` §5.1). A draw the controller delivered nothing
in has an infinite regret: it counts, and sorts last.
"""

from __future__ import annotations

import argparse
import csv
import math
import random
import statistics
from pathlib import Path

BOOTSTRAP = 4000


def read(paths: list[Path]) -> dict[str, list[dict[str, str]]]:
    """Every summary row, by scenario, its draws in seed order."""
    rows: dict[str, list[dict[str, str]]] = {}
    for path in paths:
        with path.open(newline="", encoding="utf-8") as f:
            for row in csv.DictReader(f):
                rows.setdefault(row["scenario"], []).append(row)
    for draws in rows.values():
        draws.sort(key=lambda r: int(r["seed"]))
    return rows


def regret(row: dict[str, str]) -> float | None:
    """Best fixed rung over the controller, less one; infinite when the controller delivered
    nothing and a fixed rung did; None when nothing delivered at all."""
    auto = float(row["auto_metric"] or 0.0)
    best = float(row["best_metric"] or 0.0)
    if auto > 0:
        return best / auto - 1.0
    return math.inf if best > 0 else None


def percent(value: float | None) -> str:
    if value is None:
        return "—"
    if math.isinf(value):
        return "lost"
    return f"{value * 100:+.0f} %"


def report(rows: dict[str, list[dict[str, str]]], seed: int = 1) -> str:
    lines = [
        "| scenario | the controller (bit/s) | best fixed rung | regret | median |",
        "|---|---|---|---|---|",
    ]
    medians: list[float] = []
    draws_by_scenario: list[list[float]] = []
    for scenario, draws in sorted(rows.items()):
        regrets = [r for r in (regret(d) for d in draws) if r is not None]
        median = statistics.median(regrets) if regrets else None
        if median is not None:
            medians.append(median)
            draws_by_scenario.append(regrets)
        auto = " / ".join(f"{float(d['auto_metric'] or 0):.0f}" for d in draws)
        best = " / ".join(f"{d['best_rung']}: {float(d['best_metric'] or 0):.0f}" for d in draws)
        each = " / ".join(percent(regret(d)) for d in draws)
        lines.append(f"| `{scenario}` | {auto} | {best} | {each} | **{percent(median)}** |")
    if medians:
        finite = [m for m in medians if math.isfinite(m)]
        rng = random.Random(seed)
        boot = sorted(
            statistics.median(rng.choice(draws) for draws in draws_by_scenario)
            for _ in range(BOOTSTRAP)
        )
        low, high = boot[int(0.05 * BOOTSTRAP)], boot[int(0.95 * BOOTSTRAP) - 1]
        lines += [
            "",
            f"**The median over the {len(medians)} scenarios of the per-scenario median regret "
            f"is {percent(statistics.median(medians))}** (mean "
            f"{percent(statistics.fmean(finite)) if finite else '—'} over the finite ones); "
            f"a bootstrap over each scenario's draws puts it between {percent(low)} and "
            f"{percent(high)} (5–95 %, {BOOTSTRAP} resamples).",
        ]
    return "\n".join(lines) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("summaries", nargs="+", type=Path)
    ap.add_argument("--markdown", type=Path, help="append the table here (a step summary)")
    ap.add_argument("--out", type=Path, help="write the table here")
    args = ap.parse_args()
    text = report(read(args.summaries))
    print(text, end="")
    if args.out:
        args.out.write_text(text, encoding="utf-8")
    if args.markdown:
        with args.markdown.open("a", encoding="utf-8") as f:
            f.write("## Rate oracle\n\n" + text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
