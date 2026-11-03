"""The rate oracle's tables, from what `tools/rung_oracle.py` writes (one job per scenario of
`.github/workflows/oracle.yml`, or one local run).

    python tools/oracle_report.py regret oracle/*/summary.csv [--skip SCENARIO …]
                                  [--markdown FILE] [--out FILE]
    python tools/oracle_report.py compare --base base/*/rows.csv --candidate cand/*/rows.csv
                                  [--paired] [--margin 0.10] [--markdown FILE] [--out FILE]

``regret``: per scenario, the controller's delivery in each draw, the best fixed rung in
hindsight and what it delivered, the regret (best / controller − 1) per draw and its median over
the draws; over the scenarios, the median and the mean of the per-scenario medians and a
bootstrap of the median over each scenario's draws (5–95 %), as the first review reported it
(`docs/research/rate-control/03-candidates.md` §5.1). A draw the controller delivered nothing
in has an infinite regret: it counts, and sorts last.

``compare``: two builds' own runs (the ``auto`` rows of ``rows.csv``, or a summary's
``auto_metric``, as in `bench/baselines/rate_oracle_beta127.csv`) of the same scenarios, scenario
by scenario — how long the candidate's transfer takes against the base's, as a Hodges–Lehmann
ratio with its exact 90 % range from the Mann–Whitney distribution, and a one-sided Mann–Whitney
p-value each way (`docs/research/rate-control-2/03-candidates.md` §3). The runs are treated as
unpaired: the simulated channel follows the air's clock, so two controllers on the same draw
drift onto different stretches of it once their choices differ, and a paired test would claim
more than the runs show. A lost transfer is an infinitely long one: it ranks last.

``compare --paired`` pairs the two builds' runs by seed instead: the Hodges–Lehmann ratio is the
median of the Walsh averages of the per-seed differences, its exact 90 % range from the Wilcoxon
signed-rank distribution. The harness repeats a draw exactly when the controller makes the same
choices, so a scenario a candidate never acted on reads 1.00 (1.00–1.00), where the unpaired range
is as wide as the spread between draws (`04-results.md` §1.3). Which form judges K2 is the
author's call.
"""

from __future__ import annotations

import argparse
import csv
import math
import random
import statistics
from functools import cache
from pathlib import Path

BOOTSTRAP = 4000
LEVEL = 0.90
"""The two-sided coverage of the range a comparison reports."""


def read(paths: list[Path]) -> dict[str, list[dict[str, str]]]:
    """Every row, by scenario, in seed order."""
    rows: dict[str, list[dict[str, str]]] = {}
    for path in paths:
        with path.open(newline="", encoding="utf-8") as f:
            for row in csv.DictReader(f):
                rows.setdefault(row["scenario"], []).append(row)
    for draws in rows.values():
        draws.sort(key=lambda r: int(r["seed"]))
    return rows


def own_runs(rows: list[dict[str, str]]) -> list[dict[str, str]]:
    """The controller's own runs: ``rows.csv``'s ``auto`` rows, or every row of a summary (one a
    draw, its ``auto_metric`` the run's measure)."""
    if rows and "rung" not in rows[0] and "auto_metric" in rows[0]:
        return [{**r, "rung": "auto", "metric": r["auto_metric"]} for r in rows]
    return [r for r in rows if r.get("rung") == "auto"]


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


def regret_report(rows: dict[str, list[dict[str, str]]], seed: int = 1) -> str:
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


@cache
def u_counts(m: int, n: int) -> tuple[int, ...]:
    """How many of the C(m + n, m) orderings of two samples of sizes ``m`` and ``n`` give each
    Mann–Whitney U = 0 … m·n, under the null (no ties)."""
    if m == 0 or n == 0:
        return (1,)
    with_m = u_counts(m - 1, n)  # the largest value is the first sample's: U gains n
    with_n = u_counts(m, n - 1)
    out = [0] * (m * n + 1)
    for u, count in enumerate(with_m):
        out[u + n] += count
    for u, count in enumerate(with_n):
        out[u] += count
    return tuple(out)


def u_cdf(m: int, n: int, u: float) -> float:
    """P(U ≤ u) under the null."""
    counts = u_counts(m, n)
    total = sum(counts)
    return sum(c for k, c in enumerate(counts) if k <= u + 1e-9) / total


def u_statistic(xs: list[float], ys: list[float]) -> float:
    """Mann–Whitney U of ``xs`` against ``ys``: pairs in which x is the larger, ties half."""
    return sum(1.0 if x > y else 0.5 if x == y else 0.0 for x in xs for y in ys)


def difference(x: float, y: float) -> float:
    """x − y, two infinities of the same sign being equal (two lost transfers)."""
    if math.isinf(x) and math.isinf(y) and (x > 0) == (y > 0):
        return 0.0
    return x - y


def hodges_lehmann(
    xs: list[float], ys: list[float], level: float = LEVEL
) -> tuple[float, float, float]:
    """The shift of ``xs`` over ``ys`` (Hodges–Lehmann: the median of all pairwise differences)
    and its exact two-sided range at ``level`` (Moses: the differences at the Mann–Whitney
    distribution's critical ranks)."""
    m, n = len(xs), len(ys)
    diffs = sorted(difference(x, y) for x in xs for y in ys)
    estimate = statistics.median(diffs)
    tail = (1.0 - level) / 2.0
    counts = u_counts(m, n)
    total = sum(counts)
    cumulative, c = 0, -1
    for u, count in enumerate(counts):
        # a tolerance: (1 − 0.9) / 2 is a hair under 0.05 in floating point
        if (cumulative + count) / total > tail + 1e-12:
            break
        cumulative += count
        c = u
    if c < 0:  # too few runs for any range at this level
        return estimate, -math.inf, math.inf
    # the (c + 1)-th smallest and the (c + 1)-th largest difference
    return estimate, diffs[c], diffs[m * n - c - 1]


@cache
def signed_rank_counts(n: int) -> tuple[int, ...]:
    """How many of the 2^n sign patterns of ``n`` ranked differences give each Wilcoxon signed-rank
    sum T = 0 … n(n + 1)/2, under the null (no ties, no zeros)."""
    counts = [1]
    for k in range(1, n + 1):
        out = [0] * (len(counts) + k)
        for t, c in enumerate(counts):
            out[t] += c
            out[t + k] += c
        counts = out
    return tuple(counts)


def paired_hodges_lehmann(diffs: list[float], level: float = LEVEL) -> tuple[float, float, float]:
    """The paired shift (the median of the Walsh averages of ``diffs``) and its exact two-sided
    range at ``level`` from the signed-rank distribution."""
    n = len(diffs)
    walsh = sorted(difference(diffs[i], -diffs[j]) / 2 for i in range(n) for j in range(i, n))
    estimate = statistics.median(walsh)
    tail = (1.0 - level) / 2.0
    counts = signed_rank_counts(n)
    total = sum(counts)
    cumulative, c = 0, -1
    for t, count in enumerate(counts):
        if (cumulative + count) / total > tail + 1e-12:
            break
        cumulative += count
        c = t
    if c < 0:
        return estimate, -math.inf, math.inf
    return estimate, walsh[c], walsh[len(walsh) - c - 1]


def log_time(row: dict[str, str]) -> float:
    """A run's transfer as the log of its time per bit (the inverse of its rate); a lost
    transfer is infinitely long."""
    rate = float(row.get("metric") or 0.0)
    return -math.log(rate) if rate > 0 else math.inf


def median_rate(rows: list[dict[str, str]]) -> float:
    """The median delivery of some runs, bit/s; a lost transfer is 0."""
    return statistics.median(float(r.get("metric") or 0) for r in rows)


def ratio(log_shift: float) -> str:
    if math.isinf(log_shift):
        return "∞" if log_shift > 0 else "0"
    return f"{math.exp(log_shift):.2f}"


def verdict(high: float, margin: float) -> str:
    if math.exp(high) < 1.0:
        return "faster"
    if math.exp(high) <= 1.0 + margin:
        return f"not slower (within {margin:.0%})"
    return "may be slower: confirm on fresh draws"


def paired_report(
    base: dict[str, list[dict[str, str]]],
    candidate: dict[str, list[dict[str, str]]],
    margin: float = 0.10,
) -> str:
    """Each scenario both builds ran, paired by seed: the candidate's transfer time over the
    base's on the same draw."""
    lines = [
        f"| scenario | pairs | identical | time ratio | {LEVEL:.0%} range | verdict |",
        "|---|---|---|---|---|---|",
    ]
    for scenario in sorted(set(base) & set(candidate)):
        b = {r["seed"]: log_time(r) for r in own_runs(base[scenario])}
        c = {r["seed"]: log_time(r) for r in own_runs(candidate[scenario])}
        seeds = sorted(set(b) & set(c), key=int)
        if not seeds:
            continue
        diffs = [difference(c[k], b[k]) for k in seeds]
        shift, low, high = paired_hodges_lehmann(diffs)
        same = sum(1 for d in diffs if d == 0.0)
        lines.append(
            f"| `{scenario}` | {len(seeds)} | {same} | {ratio(shift)} | "
            f"{ratio(low)}–{ratio(high)} | {verdict(high, margin)} |"
        )
    lines += [
        "",
        "Paired by seed: the median of the Walsh averages of the per-seed log time ratios, with its "
        f"exact {LEVEL:.0%} range from the Wilcoxon signed-rank distribution. A lost transfer is "
        "infinitely long.",
    ]
    return "\n".join(lines) + "\n"


def compare_report(
    base: dict[str, list[dict[str, str]]],
    candidate: dict[str, list[dict[str, str]]],
    margin: float = 0.10,
) -> str:
    """Each scenario both builds ran: the candidate's transfer time over the base's."""
    lines = [
        "| scenario | runs | base (bit/s, median) | candidate | time ratio | "
        f"{LEVEL:.0%} range | p (faster) | p (slower) | verdict |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for scenario in sorted(set(base) & set(candidate)):
        b, c = own_runs(base[scenario]), own_runs(candidate[scenario])
        if not b or not c:
            continue
        xs, ys = [log_time(r) for r in c], [log_time(r) for r in b]
        shift, low, high = hodges_lehmann(xs, ys)
        u = u_statistic(xs, ys)  # pairs in which the candidate took longer
        p_slower = 1.0 - u_cdf(len(xs), len(ys), u - 0.5)
        p_faster = u_cdf(len(xs), len(ys), u)
        lines.append(
            f"| `{scenario}` | {len(c)} v {len(b)} | {median_rate(b):.0f} | {median_rate(c):.0f} | "
            f"{ratio(shift)} | {ratio(low)}–{ratio(high)} | {p_faster:.3f} | {p_slower:.3f} | "
            f"{verdict(high, margin)} |"
        )
    lines += [
        "",
        "Time ratio: the candidate's transfer time over the base's (Hodges–Lehmann; under 1 is "
        f"faster), with its exact {LEVEL:.0%} range (Moses, from the Mann–Whitney distribution). "
        "The p-values are one-sided Mann–Whitney tests, unpaired; a lost transfer ranks as the "
        "longest.",
    ]
    return "\n".join(lines) + "\n"


def write(text: str, out: Path | None, markdown: Path | None, title: str) -> None:
    print(text, end="")
    if out:
        out.write_text(text, encoding="utf-8")
    if markdown:
        with markdown.open("a", encoding="utf-8") as f:
            f.write(f"## {title}\n\n" + text)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="what", required=True)
    reg = sub.add_parser("regret", help="the controller against the best fixed rung")
    reg.add_argument("summaries", nargs="+", type=Path)
    reg.add_argument("--skip", nargs="*", default=[], help="leave these scenarios out")
    cmp = sub.add_parser("compare", help="a candidate build against a base build")
    cmp.add_argument("--base", nargs="+", type=Path, required=True)
    cmp.add_argument("--candidate", nargs="+", type=Path, required=True)
    cmp.add_argument("--margin", type=float, default=0.10, help="not slower by more than this")
    cmp.add_argument("--paired", action="store_true", help="pair the runs by seed")
    for p in (reg, cmp):
        p.add_argument("--markdown", type=Path, help="append the table here (a step summary)")
        p.add_argument("--out", type=Path, help="write the table here")
    args = ap.parse_args()
    if args.what == "regret":
        rows = {k: v for k, v in read(args.summaries).items() if k not in set(args.skip)}
        write(regret_report(rows), args.out, args.markdown, "Rate oracle")
    else:
        judge = paired_report if args.paired else compare_report
        text = judge(read(args.base), read(args.candidate), args.margin)
        write(text, args.out, args.markdown, "Candidate against base")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
