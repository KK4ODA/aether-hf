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
        w = csv.DictWriter(f, fieldnames=["scenario", "pass", "why", "invariants"])
        w.writeheader()
        w.writerow({"scenario": "x-s1009", "pass": "False", "why": "b exited (1)",
                    "invariants": "b exited (1)"})  # fmt: skip
        w.writerow({"scenario": "z-s2031", "pass": "False", "why": "message 1000: not within"})
        w.writerow({"scenario": "y-s1004", "pass": "True", "why": ""})
    (tmp_path / "soak").mkdir()
    (tmp_path / "soak" / "soak.json").write_text(
        json.dumps([{"scenario": "soak-7", "pass": False, "why": "b panicked"}]), encoding="utf-8"
    )
    (tmp_path / "mixed").mkdir()
    (tmp_path / "mixed" / "note.md").write_text("skipped: protocol 8 against 7", encoding="utf-8")
    results = nr.parse_results(
        "build=success full=success scenarios=failure trend=skipped soak=failure"
    )
    text = nr.body(tmp_path, results, "https://run", "0123456789", "2026-10-12")
    assert "scenarios, soak failed" in text
    serious, _, minor = text.partition("not serious on its own (1)")
    assert "broke an invariant:**\n- `x-s1009`: b exited (1)" in serious
    assert "z-s2031" not in serious
    assert "`z-s2031`: message 1000: not within" in minor
    assert "y-s1004" not in text
    assert "`soak-7`: b panicked" in text
    assert "protocol 8 against 7" in text
    # a night with nothing serious closes the issue and counts what stayed a warning
    quiet = nr.quiet(tmp_path, "https://run", "0123456789", "2026-10-13")
    assert quiet.startswith("Nothing serious on 2026-10-13 at `0123456`")
    assert "1 single-draw miss stayed a warning" in quiet


def test_a_scenario_that_misses_on_one_draw_is_not_serious() -> None:
    # the nightly of 2026-11-14: one draw of a hard NVIS path missed its message's budget
    def runs(name: str, passed: int, of: int = 3, air: float = 200.0) -> list[dict[str, str]]:
        return [row(f"{name}-s{k}", name, k < passed, air) for k in range(of)]

    others = [f"ok{k}" for k in range(6)]

    def night(**odd: list[dict[str, str]]) -> dict[str, st.Night]:
        rows = odd.get("agc", runs("agc", 3)) + odd.get("ok0", runs("ok0", 3)) + runs("hard", 0)
        return st.summarise(rows + [r for n in others[1:] for r in runs(n, 3)])

    base = night()
    for n in base.values():
        n.air_s_lo, n.air_s_hi = 180.0, 220.0
    now = night(agc=runs("agc", 2))
    _, flagged = st.report(now, base, 0.15, 30)
    assert [f[:2] for f in flagged] == [("agc", "failing")]
    assert st.serious(now, base, flagged) == []
    # every draw failing is the build, unless the baseline never passed it either ("hard")
    now = night(agc=runs("agc", 0))
    _, flagged = st.report(now, base, 0.15, 30)
    assert st.serious(now, base, flagged) == ["`agc` passed none of its 3 runs"]
    # and so is a quarter of the scenarios slower or failing at once
    now = night(agc=runs("agc", 2), ok0=runs("ok0", 3, air=400.0))
    _, flagged = st.report(now, base, 0.15, 30)
    assert st.serious(now, base, flagged) == [
        "2 of 8 scenarios slower or failing more than the baseline"
    ]


def test_a_filter_warning_on_an_uncut_radio_is_listed_and_one_on_a_cut_radio_is_not(
    tmp_path: Path,
) -> None:
    import json

    import nightly_report as nr

    # H1 (docs/research/receive-filter/05-busy-band.md): a and b's radios cut nothing, c's does
    scenario = {"stations": {"c": {"radio": {"rx_filter_hz": [300, 1900]}}}}
    run = sm.Run(scenario, tmp_path, tmp_path / "channel.progress")

    def recording(who: str, name: str, readings: list[list[object]]) -> None:
        folder = tmp_path / who / "recordings"
        folder.mkdir(parents=True, exist_ok=True)
        side = {"passband": {"readings": readings}}
        (folder / f"{name}.json").write_text(json.dumps(side), encoding="utf-8")

    clear = [1.0, 0.5, "clear", 3.0, 4.0, 40.0, 12, 0.0]
    recording("a", "20261117-200314_W4ODA_KK4XYZ", [clear, [2.0, 1.5, "bottom", 13.4, 5.0,
              40.0, 13, 0.0], [3.0, 2.5, "both", 14.9, 12.2, 40.0, 14, 0.0]])  # fmt: skip
    recording("b", "20261117-200315_KK4XYZ_W4ODA", [clear, clear])
    recording("c", "20261117-200316_N4CCC_W4ODA", [[2.0, 1.5, "top", 2.0, 25.0, 40, 13, 0.0]])
    (tmp_path / "a" / "recordings" / "sessions.json").write_text("[1, 2]", encoding="utf-8")
    # b's warning came while it idled, before any recording: only its log says so (L1)
    (tmp_path / "b" / "aetherd.out").write_text(
        "2026-11-17T23:00:51.339Z info  filter: waiting -> both | bottom 12.1 dB, top 14.5 dB,"
        " 12 looks (Idle)\n"
        "2026-11-17T23:00:58.000Z info  ptt: keyed (Connecting)\n"
        "2026-11-17T23:01:20.010Z info  filter: both -> clear | bottom 8.2 dB, top 9.0 dB,"
        " 40 looks (Idle)\n",
        encoding="utf-8",
    )
    (tmp_path / "c" / "aetherd.out").write_text(
        "x info  filter: waiting -> top | bottom 2.0 dB, top 25.0 dB, 12 looks (Idle)\n",
        encoding="utf-8",
    )
    assert sm.filter_warnings(run) == [
        "a: both/bottom up to 14.9 dB in 2 of 3 readings (20261117-200314_W4ODA_KK4XYZ)",
        "b: both up to 14.5 dB at 1 of 2 changes (log)",
    ]
    # the nightly lists them, a warning on a green run as on a red one
    runs = tmp_path / "artifacts"
    (runs / "scenarios-1").mkdir(parents=True)
    with (runs / "scenarios-1" / "results.csv").open("w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=["scenario", "pass", "why", "invariants",
                                          "filter_warnings"])  # fmt: skip
        w.writeheader()
        w.writerow({"scenario": "rtty-s1", "pass": "True", "filter_warnings": "a: bottom"})
        w.writerow({"scenario": "plain-s1", "pass": "True", "filter_warnings": ""})
    assert nr.filter_rows(runs) == ["- `rtty-s1`: a: bottom"]
    text = nr.body(runs, nr.parse_results("full=failure"), "https://run", "0123456", "2026-11-17")
    assert "cuts nothing (1)</summary>\n\n- `rtty-s1`: a: bottom" in text
    quiet = nr.quiet(runs, "https://run", "0123456789", "2026-11-17")
    assert "cuts nothing in 1 run, a warning on the run." in quiet


def test_the_runner_fails_on_invariants_alone_when_asked() -> None:
    missed = {"scenario": "a", "pass": False, "invariants": ""}
    broke = {"scenario": "b", "pass": False, "invariants": "b panicked"}
    passed = {"scenario": "c", "pass": True, "invariants": ""}
    assert sm.exit_status([passed, missed]) == 1
    assert sm.exit_status([passed, missed], "invariants") == 0
    assert sm.exit_status([passed, missed, broke], "invariants") == 1
    assert sm.exit_status([passed], "invariants") == 0


def test_the_last_release_fails_a_scenario_only_both_ways_round(tmp_path: Path) -> None:
    import mixed_pair as mp

    def results(name: str, failing: list[str]) -> Path:
        path = tmp_path / name / "results.csv"
        path.parent.mkdir()
        with path.open("w", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=["scenario", "pass"])
            w.writeheader()
            for s in ("p", "q", "r"):
                w.writerow({"scenario": s, "pass": str(s not in failing)})
        return path

    calling, called = results("calling", ["p", "q"]), results("called", ["q", "r"])
    assert mp.failed_both_ways(calling, called) == ["q"]
    assert mp.failed_both_ways(calling, tmp_path / "missing.csv") == []


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
