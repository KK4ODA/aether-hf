"""tools/bench_estimate.py: ADR-0131's criteria for de-rotation, judged as registered.

The judgement sums decodes over each air and channel and compares de-rotation with the receiver
before it and with the Wiener estimator. Rows built so every criterion passes must pass; let a
point fall on AWGN, or the gain on Poor shrink, or Wiener win a channel, and the criterion that
guards it must say so."""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import bench_estimate as be  # noqa: E402

CHANNELS = ("awgn", "good", "moderate", "poor", "flutter", "nvis")
DECODES = {"before": 20, "derotate": 20, "wiener": 10}
"""Decodes of 30 a point, per arm, before a test changes them; de-rotation gains on Poor below."""


def _rows() -> list[dict[str, object]]:
    rows: list[dict[str, object]] = []
    for air in ("2300", "500"):
        for channel in CHANNELS:
            for arm, n in DECODES.items():
                gain = 6 if (arm == "derotate" and channel == "poor") else 0
                rows.append(
                    {
                        "air": air,
                        "channel": channel,
                        "mode": "4",
                        "snr_3k_db": "5.0",
                        "arm": arm,
                        "frames": 30,
                        "decoded": n + gain,
                    }
                )
    return rows


def _set(rows: list[dict[str, object]], air: str, channel: str, arm: str, n: int) -> None:
    for r in rows:
        if (r["air"], r["channel"], r["arm"]) == (air, channel, arm):
            r["decoded"] = n


def test_rows_that_meet_every_criterion_pass() -> None:
    assert be.judge(_rows())


def test_a_point_that_falls_on_awgn_misses_e1() -> None:
    rows = _rows()
    _set(rows, "500", "awgn", "derotate", 16)
    assert not be.judge(rows)


def test_a_gain_on_poor_under_ten_percent_misses_e2() -> None:
    rows = _rows()
    _set(rows, "2300", "poor", "derotate", 21)
    assert not be.judge(rows)


def test_a_channel_wiener_wins_misses_e7() -> None:
    rows = _rows()
    _set(rows, "2300", "nvis", "wiener", 25)
    assert not be.judge(rows)


def test_the_records_arms_of_the_part_not_built_are_not_judged() -> None:
    rows = _rows()
    rows += [dict(r, arm="error", decoded=0) for r in rows if r["arm"] == "before"]
    assert be.judge(rows)
