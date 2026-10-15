"""The nightly's speed baseline and the scenario variants it averages over."""

from __future__ import annotations

import csv
import sys
import tomllib
from pathlib import Path

import pytest

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


def night(air: float, msg: float | None, lo: float, hi: float, passed: int = 3) -> st.Night:
    n = st.Night("x", 3, passed, air, msg, None, None)
    n.air_s_lo, n.air_s_hi = lo, hi
    n.message_bps_lo, n.message_bps_hi = (None, None) if msg is None else (msg * 0.7, msg * 1.3)
    return n


def test_a_step_is_flagged_and_a_scenarios_own_spread_is_not() -> None:
    # the first night's case: rtty-crashes ran 226, 248 and 364 s on one daemon, and a later
    # median of 364 is inside that, not a regression
    base = night(248.0, 300.0, 226.0, 364.0)
    assert st.verdict(night(364.0, 300.0, 300, 400), base, 0.15, 30)[0] == "same"
    assert st.verdict(night(470.0, 300.0, 400, 500), base, 0.15, 30)[0] == "slower"
    assert st.verdict(night(150.0, 300.0, 140, 160), base, 0.15, 30)[0] == "faster"
    # a short scenario's growth past its range by under 30 s is not a step
    short = night(30.0, None, 28.0, 32.0)
    assert st.verdict(night(45.0, None, 40, 50), short, 0.15, 30)[0] == "same"
    assert st.verdict(night(248.0, 300.0, 226, 364, passed=1), base, 0.15, 30)[0] == "failing"
    # a rate under the slowest baseline run (210 bit/s) by more than the tolerance
    kind, why = st.verdict(night(248.0, 150.0, 226, 364), base, 0.15, 30)
    assert kind == "slower"
    assert why == ["message 150 bit/s, baseline runs 210–390"]


def test_the_baseline_round_trips(tmp_path: Path) -> None:
    nights = st.summarise([row("x", "x", True, 100.0, 300.0), row("z", "z", False, 9.0)])
    path = tmp_path / "scenarios.csv"
    st.write_baseline(path, nights, "abc1234")
    with path.open(encoding="utf-8") as f:
        assert next(csv.reader(f)) == st.COLUMNS
    back = st.read_baseline(path)
    assert back["x"] == nights["x"]
    assert (back["x"].air_s_lo, back["x"].air_s_hi) == (100.0, 100.0)
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


def test_a_runs_folder_reads_like_its_csv(tmp_path: Path) -> None:
    import json

    for name, ok, air in (("x", True, 100.0), ("x-s1009", False, None)):
        (tmp_path / name).mkdir()
        result = {"scenario": name, "pass": ok, "air_s": air, "base": "x", "message_bps": None}
        (tmp_path / name / "result.json").write_text(json.dumps(result), encoding="utf-8")
    nights = st.summarise(st.read_rows([tmp_path]))
    assert (nights["x"].runs, nights["x"].passed, nights["x"].air_s) == (2, 1, 100.0)


def test_the_fetch_tool_reads_the_script_the_daemon_uploads_to() -> None:
    import re

    import fetch_uploads

    source = (ROOT / "core" / "aetherd" / "src" / "upload.rs").read_text(encoding="utf-8")
    endpoint = re.search(r'pub const PROJECT_ENDPOINT: &str = "([^"]+)";', source)
    assert endpoint is not None
    assert endpoint.group(1) == fetch_uploads.ENDPOINT
    files = [
        {"id": "abc", "name": "20261012-015641Z KQ4SAX aether.zip"},
        {"id": "def", "name": "x"},
    ]
    assert fetch_uploads.matching("kq4sax", files) == files[:1]
    assert fetch_uploads.matching("def", files) == files[1:]


def test_the_fetch_tool_asks_again_and_carries_on_from_a_stopped_download(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import base64

    import fetch_uploads

    content = bytes(range(256)) * 40
    monkeypatch.setattr(fetch_uploads, "PIECE", 4096)
    asked: list[int] = []
    faults = iter(["empty", "reset"])

    def ask(payload: dict[str, object]) -> dict:
        offset = int(payload["offset"])  # type: ignore[call-overload]
        asked.append(offset)
        if offset == 8192:
            fault = next(faults, None)
            if fault == "empty":
                return {"ok": True, "data": ""}
            if fault == "reset":
                raise ConnectionResetError(104, "Connection reset by peer")
        length = int(payload["length"])  # type: ignore[call-overload]
        return {"ok": True, "data": base64.b64encode(content[offset : offset + length]).decode()}

    monkeypatch.setattr(fetch_uploads, "ask", ask)
    file = {"id": "abc", "name": "x.zip", "size": str(len(content))}
    (tmp_path / "x.zip.part").write_bytes(content[:4096])
    path = fetch_uploads.download(file, tmp_path, wait_s=0.0)
    assert path.read_bytes() == content
    assert asked == [4096, 8192, 8192, 8192]
    assert not (tmp_path / "x.zip.part").exists()
