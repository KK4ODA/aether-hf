"""Step 3's judge applies the registration's rules (10-step-3.md §1).

uv run pytest research/link_arch/tests/test_step3.py -q
"""

from __future__ import annotations

import csv
import tomllib
from collections import Counter
from pathlib import Path

from research.link_arch import step3

SCENARIOS = Path(__file__).resolve().parents[3] / "bench" / "scenarios"


def test_every_scenario_falls_in_one_registered_class() -> None:
    classes = Counter(
        step3.scenario_class(tomllib.loads(p.read_text(encoding="utf-8")))
        for p in sorted(SCENARIOS.glob("*.toml"))
    )
    assert classes == {"gateway": 5, "robustness": 8, "band": 7, "2300 Hz": 12, "500 Hz": 12}


def test_a_run_completes_when_only_the_old_collision_count_failed() -> None:
    assert step3.completed("")
    assert step3.completed("29 collisions")
    assert not step3.completed("29 collisions; did not connect")
    assert not step3.completed("test failed")


def test_s4_holds_for_a_declared_vox_and_breaks_for_an_undeclared_one() -> None:
    vox = {"vox_hold_ms": 900, "rx_recovery_ms": 100}
    declared = {"stations": {"a": {"turnaround_ms": 1000, "radio": vox}}}
    undeclared = {"stations": {"a": {"radio": vox}}}
    signalink = {"stations": {"b": {"radio": {"vox_hold_ms": 400}}}}
    slow_line = {"stations": {"a": {"radio": {"rx_recovery_ms": 280}}}}
    assert step3.s4_holds(declared, "a")
    assert not step3.s4_holds(undeclared, "a")
    assert step3.s4_holds(signalink, "b")
    # the keyed tail (0.25 s) and the recovery: within the 0.65 s left in any case
    assert step3.s4_holds(slow_line, "a")
    assert step3.s4_holds({}, "a")


def _tx(at: float, cause: str) -> dict[str, object]:
    return {"air_s": at, "cause": cause}


def test_an_overlap_of_two_initiations_is_left_out_and_any_other_counts() -> None:
    report = {
        "rf_overlaps": [[10.0, 10.5], [20.0, 20.2]],
        "stations": {
            "a": {"transmissions": [_tx(9.0, "connect"), _tx(19.5, "timer:proceed")]},
            "b": {"transmissions": [_tx(9.8, "beacon"), _tx(20.0, "connect")]},
        },
    }
    counted, initiations, seconds = step3.overlaps(report)
    assert (counted, initiations) == (1, 1)
    assert abs(seconds - 0.2) < 1e-9


def test_deaf_hits_count_unless_the_radio_breaks_s4_or_an_initiation_made_them() -> None:
    report = {
        "stations": {
            "a": {
                "keyed": [[0.0, 2.0]],
                "radiated": [[0.1, 1.9]],
                "deaf_hits_s": {"transmitting": 0.0, "recovering": 0.1},
                "transmissions": [_tx(0.1, "timer:proceed")],
            },
            "b": {
                "keyed": [[2.05, 3.0]],
                "radiated": [[2.05, 3.0]],
                "deaf_hits_s": {},
                "transmissions": [_tx(2.05, "frame")],
            },
        }
    }
    line = {"stations": {"a": {"radio": {"rx_recovery_ms": 200}}}}
    counted, reported, _ = step3.deaf_hits(report, line)
    assert (counted, reported) == (0.1, 0.0)
    undeclared = {"stations": {"a": {"radio": {"vox_hold_ms": 900, "rx_recovery_ms": 100}}}}
    counted, reported, detail = step3.deaf_hits(report, undeclared)
    assert (counted, reported) == (0.0, 0.1)
    assert "S4 broken" in detail
    report["stations"]["b"]["transmissions"] = [_tx(2.05, "beacon")]
    counted, reported, detail = step3.deaf_hits(report, line)
    assert (counted, reported) == (0.0, 0.1)
    assert "initiation" in detail


def _write(path: Path, rows: list[dict[str, object]]) -> None:
    with path.open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]))
        w.writeheader()
        w.writerows(rows)


def _row(seed: int, air: float, done: bool = True, rf: int = 0, **kw: object) -> dict[str, object]:
    row: dict[str, object] = {
        "base": "s",
        "seed": seed,
        "class": "500 Hz",
        "completed": done,
        "air_s": air,
        "rf_counted": rf,
        "deaf_counted_s": 0.0,
        "message_bps": 100,
        "file_bps": 200,
    }
    row.update(kw)
    return row


def test_the_verdict_follows_the_criteria(tmp_path: Path) -> None:
    p7, p8 = tmp_path / "p7.csv", tmp_path / "p8.csv"
    _write(p7, [_row(s, 100.0) for s in range(10)])
    _write(p8, [_row(s, 108.0) for s in range(10)])
    assert step3.judge(p7, p8) == 0
    # slower than the pooled bound
    _write(p8, [_row(s, 112.0) for s in range(10)])
    assert step3.judge(p7, p8) == 1
    # one radiated overlap
    _write(p8, [_row(s, 100.0, rf=int(s == 3)) for s in range(10)])
    assert step3.judge(p7, p8) == 1
    # one run fewer completed
    _write(p8, [_row(s, 100.0, done=s != 3) for s in range(10)])
    assert step3.judge(p7, p8) == 1
    # the file rate under ×0.85
    _write(p8, [_row(s, 100.0, file_bps=160) for s in range(10)])
    assert step3.judge(p7, p8) == 1
