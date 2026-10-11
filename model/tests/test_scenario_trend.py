"""The nightly's speed baseline and the scenario variants it averages over."""

from __future__ import annotations

import csv
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import scenario_trend as st  # noqa: E402
import session_matrix as sm  # noqa: E402


def row(name: str, base: str, ok: bool, air: float, msg: float | None = None) -> dict[str, str]:
    return {
        "scenario": name,
        "base": base,
        "pass": str(ok),
        "air_s": str(air),
        "message_bps": "" if msg is None else str(msg),
        "file_bps": "",
        "highest_rung": "",
    }


def test_variants_are_grouped_and_failures_left_out_of_the_timing() -> None:
    nights = st.summarise(
        [
            row("x", "x", True, 100.0, 300.0),
            row("x-s1009", "x", True, 120.0, 200.0),
            row("x-s2009", "x", False, 600.0, 10.0),
            row("y-s5", "", True, 50.0),  # an older CSV with no base column
        ]
    )
    assert nights["x"].runs == 3
    assert nights["x"].passed == 2
    assert nights["x"].air_s == 110.0
    assert nights["x"].message_bps == 250.0
    assert nights["y"].runs == 1


def test_a_step_is_flagged_and_noise_is_not() -> None:
    base = st.Night("x", 3, 3, 200.0, 300.0, None, None)
    same = st.Night("x", 3, 3, 240.0, 250.0, None, None)  # +20 %, −17 %
    slower = st.Night("x", 3, 3, 300.0, 300.0, None, None)  # +50 %, +100 s
    short = st.Night("x", 3, 3, 30.0, None, None, None)
    tiny = st.Night("x", 3, 3, 45.0, None, None, None)  # +50 % of 30 s is 15 s
    failing = st.Night("x", 3, 1, 200.0, 300.0, None, None)
    assert st.verdict(same, base, 0.3, 30)[0] == "same"
    assert st.verdict(slower, base, 0.3, 30)[0] == "slower"
    assert st.verdict(tiny, short, 0.3, 30)[0] == "same"
    assert st.verdict(failing, base, 0.3, 30)[0] == "failing"
    assert st.verdict(base, slower, 0.3, 30)[0] == "faster"
    rate = st.Night("x", 3, 3, 200.0, 150.0, None, None)
    assert st.verdict(rate, base, 0.3, 30) == ("slower", ["message 300 → 150 bit/s"])


def test_the_baseline_round_trips(tmp_path: Path) -> None:
    nights = st.summarise([row("x", "x", True, 100.0, 300.0), row("z", "z", False, 9.0)])
    path = tmp_path / "scenarios.csv"
    st.write_baseline(path, nights, "abc1234")
    with path.open(encoding="utf-8") as f:
        assert next(csv.reader(f)) == st.COLUMNS
    back = st.read_baseline(path)
    assert back["x"] == nights["x"]
    assert back["z"].passed == 0
    assert back["z"].air_s is None
    text, flagged = st.report(nights, back, 0.3, 30)
    assert flagged == []
    assert "Nothing slower" in text


def test_a_variant_changes_only_the_seed_and_the_name() -> None:
    path = ROOT / "bench" / "scenarios" / "quick-awgn-2300.toml"
    original = tomllib.loads(path.read_text(encoding="utf-8"))
    (_, none), (_, seed) = sm.variants(path, [0, 2])
    assert none is None
    assert seed == original["seed"] + 2 * sm.SEED_STEP
    variant = tomllib.loads(sm.with_seed(path.read_text(encoding="utf-8"), seed))
    assert variant["name"] == f"{original['name']}-s{seed}"
    assert variant["seed"] == seed
    assert {k: v for k, v in variant.items() if k not in ("name", "seed")} == {
        k: v for k, v in original.items() if k not in ("name", "seed")
    }


def test_the_nightly_issue_names_what_failed(tmp_path: Path) -> None:
    import json

    import nightly_report as nr

    (tmp_path / "scenarios-1").mkdir()
    with (tmp_path / "scenarios-1" / "results.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["scenario", "pass", "why"])
        w.writeheader()
        w.writerow({"scenario": "x-s1009", "pass": "False", "why": "did not connect"})
        w.writerow({"scenario": "y-s1004", "pass": "True", "why": ""})
    (tmp_path / "soak").mkdir()
    (tmp_path / "soak" / "soak.json").write_text(
        json.dumps([{"scenario": "soak-7", "pass": False, "why": "b panicked"}]), encoding="utf-8"
    )
    (tmp_path / "mixed").mkdir()
    (tmp_path / "mixed" / "note.md").write_text("skipped: protocol 8 against 7", encoding="utf-8")
    results = nr.parse_results("full=success scenarios=failure trend=skipped soak=failure")
    text = nr.body(tmp_path, results, "https://run", "0123456789", "2026-10-12")
    assert "scenarios, soak failed" in text
    assert "`x-s1009`: did not connect" in text
    assert "y-s1004" not in text
    assert "`soak-7`: b panicked" in text
    assert "protocol 8 against 7" in text
