"""Summaries of a campaign's runs and overlaps, with bootstrap intervals.

uv run python -m research.link_arch.analyze runs.csv [overlaps.csv] [--by arm,profile]
"""

from __future__ import annotations

import argparse
import csv
import gzip
import math
import random
import statistics
from collections import Counter, defaultdict
from collections.abc import Callable, Sequence


def load(path: str) -> list[dict[str, str]]:
    opener = gzip.open if path.endswith(".gz") else open
    with opener(path, "rt", encoding="utf-8") as f:
        return list(csv.DictReader(f))


def boot_ci(
    values: Sequence[float],
    stat: Callable[[Sequence[float]], float] = statistics.fmean,
    n: int = 2000,
    seed: int = 1,
) -> tuple[float, float, float]:
    """The statistic and a 95 % percentile-bootstrap interval."""
    if not values:
        return (math.nan, math.nan, math.nan)
    rng = random.Random(seed)
    k = len(values)
    reps = sorted(stat([values[rng.randrange(k)] for _ in range(k)]) for _ in range(n))
    return (stat(values), reps[int(0.025 * n)], reps[int(0.975 * n) - 1])


def paired_ratio_ci(
    pairs: Sequence[tuple[float, float]], n: int = 2000, seed: int = 1
) -> tuple[float, float, float]:
    """Geometric mean of b/a over paired runs, with a bootstrap interval over pairs."""
    logs = [math.log(b / a) for a, b in pairs if a > 0 and b > 0]
    if not logs:
        return (math.nan, math.nan, math.nan)
    g, lo, hi = boot_ci(logs, statistics.fmean, n, seed)
    return (math.exp(g), math.exp(lo), math.exp(hi))


def fmt(t: tuple[float, float, float], digits: int = 2) -> str:
    v, lo, hi = t
    return f"{v:.{digits}f} [{lo:.{digits}f}, {hi:.{digits}f}]"


def summarize(runs: list[dict[str, str]], by: list[str]) -> None:
    groups: dict[tuple[str, ...], list[dict[str, str]]] = defaultdict(list)
    for r in runs:
        groups[tuple(r[k] for k in by)].append(r)
    print(
        " | ".join(by),
        "| runs | ok | clean | rf-overlaps/run [CI] | runs with overlap | dual s/run",
    )
    for key in sorted(groups):
        rs = groups[key]
        ok = sum(int(r["ok"]) for r in rs) / len(rs)
        clean = sum(int(r.get("clean_end") or 0) for r in rs) / len(rs)
        ov = [float(r.get("rf_overlaps") or 0) for r in rs]
        anyov = sum(1 for x in ov if x > 0) / len(rs)
        dual = statistics.fmean(float(r.get("dual_s") or 0) for r in rs)
        print(
            " | ".join(key),
            f"| {len(rs)} | {ok:.3f} | {clean:.3f} | {fmt(boot_ci(ov))} | {anyov:.3f} | {dual:.1f}",
        )


def classes(overs: list[dict[str, str]], runs: list[dict[str, str]], by: list[str]) -> None:
    nruns: Counter[tuple[str, ...]] = Counter(tuple(r[k] for k in by) for r in runs)
    counts: dict[tuple[str, ...], Counter[str]] = defaultdict(Counter)
    for o in overs:
        if float(o["rf_s"]) <= 0:
            counts[tuple(o[k] for k in by)]["keying"] += 1
            continue
        c = o["class"]
        counts[tuple(o[k] for k in by)][c] += 1
    for key in sorted(nruns):
        c = counts.get(key, Counter())
        total = sum(v for k, v in c.items() if k != "keying")
        parts = ", ".join(f"{k} {v / nruns[key]:.2f}" for k, v in c.most_common() if v)
        print(" | ".join(key), f"| per run: rf {total / nruns[key]:.2f} :: {parts}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("runs")
    ap.add_argument("overlaps", nargs="?", default="")
    ap.add_argument("--by", default="arm,profile")
    ap.add_argument("--where", default="", help="k=v,k=v filters")
    args = ap.parse_args()
    runs = load(args.runs)
    flt = [kv.split("=", 1) for kv in args.where.split(",") if kv]
    runs = [r for r in runs if all(r.get(k) == v for k, v in flt)]
    by = args.by.split(",")
    summarize(runs, by)
    if args.overlaps:
        overs = [o for o in load(args.overlaps) if all(o.get(k) == v for k, v in flt)]
        print()
        classes(overs, runs, by)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
