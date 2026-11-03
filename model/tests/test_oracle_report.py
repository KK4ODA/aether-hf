"""The rate review's statistics (``tools/oracle_report.py``) say what the textbooks say.

The second rate review judges a candidate controller against today's on ten draws a scenario,
with a Mann–Whitney comparison and a Hodges–Lehmann time ratio whose range decides "faster",
"not slower" or "confirm on fresh draws" (``docs/research/rate-control-2/03-candidates.md``
§3). A rank off in the range would move every verdict, so the exact distribution is held to the
published critical values and the estimator to a shift it must recover.
"""

from __future__ import annotations

import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import oracle_report as r  # noqa: E402


def test_the_mann_whitney_distribution_is_exact() -> None:
    for m, n in ((3, 3), (4, 7), (10, 10)):
        counts = r.u_counts(m, n)
        assert sum(counts) == math.comb(m + n, m)
        assert counts == counts[::-1], "the null distribution is symmetric"
    # the published one-sided 5 % critical value for ten against ten is 27
    assert r.u_cdf(10, 10, 27) <= 0.05 < r.u_cdf(10, 10, 28)
    # three against three can reach 5 % only when every pair goes one way
    assert r.u_cdf(3, 3, 0) == 1 / 20


def test_hodges_lehmann_recovers_a_shift_and_its_range_covers_it() -> None:
    ys = [0.1 * k for k in range(10)]
    xs = [y + 0.3 for y in ys]
    shift, low, high = r.hodges_lehmann(xs, ys)
    assert math.isclose(shift, 0.3)
    assert low < 0.3 < high
    # the 90 % range for ten against ten lies at the 28th smallest and 28th largest of the
    # hundred differences
    diffs = sorted(x - y for x in xs for y in ys)
    assert (low, high) == (diffs[27], diffs[72])
    # three against three: the range is every difference, the widest it can be
    shift, low, high = r.hodges_lehmann([1.0, 2.0, 3.0], [0.0, 0.5, 1.0])
    assert (low, high) == (0.0, 3.0)


def test_a_lost_transfer_ranks_as_the_longest() -> None:
    assert math.isinf(r.log_time({"metric": "0"}))
    assert r.log_time({"metric": "100"}) < r.log_time({"metric": "50"})
    shift, _, _ = r.hodges_lehmann([math.inf, 1.0], [1.0, 1.0])
    assert shift >= 0.0


def rows(scenario: str, rates: list[float]) -> dict[str, list[dict[str, str]]]:
    return {
        scenario: [
            {"scenario": scenario, "seed": str(k), "rung": "auto", "metric": str(rate)}
            for k, rate in enumerate(rates)
        ]
    }


def test_the_verdicts_follow_the_range() -> None:
    base = rows("s", [100.0 + k for k in range(10)])
    faster = r.compare_report(base, rows("s", [150.0 + k for k in range(10)]))
    assert "| faster |" in faster
    same = r.compare_report(base, rows("s", [100.5 + k for k in range(10)]))
    assert "not slower (within 10%)" in same
    slower = r.compare_report(base, rows("s", [60.0 + k for k in range(10)]))
    assert "may be slower: confirm on fresh draws" in slower


def test_a_summary_serves_as_a_build_s_own_runs() -> None:
    """The committed baseline is a summary (one row a draw); `compare` reads its controller runs."""
    summary = {
        "s": [
            {
                "scenario": "s",
                "seed": str(k),
                "auto_metric": str(100.0 + k),
                "best_rung": "3",
                "best_metric": "120",
            }
            for k in range(10)
        ]
    }
    report = r.compare_report(summary, rows("s", [100.5 + k for k in range(10)]))
    assert "| `s` | 10 v 10 |" in report
    assert "not slower (within 10%)" in report
