"""The field tools of P6-7: the ingest of a contributed session and the replay of one."""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import bench_link  # noqa: E402
import field_ingest  # noqa: E402
from aether_model.link.rate import AWGN_THRESHOLD_DB  # noqa: E402


def _rungs(shift: float, frames: int = 6) -> list[dict[str, object]]:
    """A ladder as a channel `shift` dB worse than AWGN would return it, with the pipe's
    own logistic: the expected decodes at each mode, rounded."""
    rungs = []
    for mode, threshold in sorted(AWGN_THRESHOLD_DB.items()):
        snr = 12.0
        success = 1.0 / (1.0 + math.exp(-1.2 * (snr - threshold - shift)))
        rungs.append(
            {
                "mode": mode,
                "frames": frames,
                "decoded": round(frames * success),
                "snr_db": snr,
                "seconds": 5.0,
            }
        )
    return rungs


def _sidecar(tmp_path: Path, name: str = "20260916-200000_KK4ODA_W1AW_test") -> Path:
    frames = [
        {
            "t_s": 2.0 + 3.0 * i,
            "kind": "control",
            "mode": 0,
            "rv": 0,
            "snr_3k_db": 11.0 + (i % 3),
            "cfo_hz": 0.4,
            "decoded": True,
            "bytes": 7,
            "control": "Ack session 9 flags 0x00 base 0 bitmap 0x003f mode 6 snr 12 #1",
        }
        for i in range(30)
    ]
    document = {
        "format": "aether-hf-session/3",
        "version": "0.2.0-beta.53",
        "started": "2026-09-16T20:00:00.000Z",
        "ended": "2026-09-16T20:03:00.000Z",
        "audio": {
            "file": f"{name}.wav",
            "sample_rate": 48000,
            "samples": 8_640_000,
            "seconds": 180.0,
        },
        "session": {
            "callsign": "KK4ODA",
            "remote": "W1AW",
            "notes": "from the bench",
            "frequency_hz": 7_101_000,
            "bandwidth_hz": 2300,
            "operator": {"grid": "EM73", "rig": "FTDX10", "power_w": 30, "antenna": "EFHW"},
            "test": {
                "remote": "W1AW",
                "started": "2026-09-16T20:00:01.000Z",
                "elapsed_s": 170.0,
                "step": "done",
                "outcome": "complete",
                "bandwidth_hz": 2300,
                "probe": {"heard_there_db": 12.0, "heard_here_db": 11.5},
                "message": {"bytes": 2048, "seconds": 20.0, "bps": 819.2},
                "file": {"bytes": 16384, "seconds": 100.0, "bps": 1310.7},
                "ladder": _rungs(4.0),
                "path": {"my_grid": "EM73", "their_grid": "FN31", "km": 1231.0},
            },
        },
        "events": [],
        "frames": frames,
        "counters": {"bytes_delivered": 0, "bytes_acked": 18432},
    }
    path = tmp_path / f"{name}.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def test_a_contributed_session_is_folded_into_the_log_once(tmp_path: Path) -> None:
    sidecar = _sidecar(tmp_path)
    log, paths, ladders = tmp_path / "LOG.md", tmp_path / "paths.csv", tmp_path / "ladders.csv"
    log.write_text("| Date | Band |\n|---|---|\n", encoding="utf-8")
    added = field_ingest.ingest(
        [sidecar], log=log, paths=paths, ladders=ladders, klass="moderate", band=None, note="a"
    )
    assert added == 1
    row = log.read_text(encoding="utf-8").splitlines()[-1]
    assert row.startswith(
        "| 2026-09-16 | 40 m, 2300 Hz | 1231 km (EM73 → FN31) | moderate | KK4ODA → W1AW |"
    )
    assert "`20260916-200000_KK4ODA_W1AW_test`" in row
    assert "1311 / — b/s" in row and "probe 12 dB there, 11.5 dB here" in row
    assert "FTDX10, 30 W, EFHW" in row and "from the bench" in row
    paths_rows = paths.read_text(encoding="utf-8").splitlines()
    assert len(paths_rows) == 2 and paths_rows[0].startswith("date,recording,")
    rungs = len(AWGN_THRESHOLD_DB)
    assert f",moderate,complete,12.0,12.0,11.5,819.2,1310.7,1310.7,{rungs}," in paths_rows[1]
    ladder_rows = ladders.read_text(encoding="utf-8").splitlines()
    assert len(ladder_rows) == 1 + rungs
    assert ladder_rows[1].endswith(",0,6,6,0.000,12.0")
    # a second pass adds nothing
    again = field_ingest.ingest(
        [sidecar], log=log, paths=paths, ladders=ladders, klass="moderate", band=None, note=""
    )
    assert again == 0
    assert len(paths.read_text(encoding="utf-8").splitlines()) == 2


def test_a_sidecar_from_before_the_ladder_is_read_onto_it(tmp_path: Path) -> None:
    """A sidecar recorded before the tone floor (``aether-hf-session/1``) numbered the OFDM
    modes: on the wide air six rungs below where they sit now, two for the tone floor's own
    kinds (ADR-0013) and four for its fast ones (ADR-0014); on the narrow air modes 0 and 1
    were the retired OFDM floor, which is on no rung, and modes 2–12 sit two rungs up, above
    the narrow middle kinds (ADR-0015). One recorded with the tone floor and before the fast
    kinds (``/2``) numbered the wide ladder's rungs from 2 up four lower; one from before the
    narrow middle kinds (``/2``, ``/3``) the narrow ladder's from 2 up two lower."""
    legacy = {"format": "aether-hf-session/1", "session": {"bandwidth_hz": 2300}}
    assert [field_ingest.sidecar_rung(legacy, m) for m in (0, 5, 13)] == [6, 11, 19]
    narrow = {"format": "aether-hf-session/1", "session": {"bandwidth_hz": 500}}
    assert [field_ingest.sidecar_rung(narrow, m) for m in (0, 1, 2, 12)] == [None, None, 4, 14]
    floor = {"format": "aether-hf-session/2", "session": {"bandwidth_hz": 2300}}
    assert [field_ingest.sidecar_rung(floor, m) for m in (0, 1, 2, 15)] == [0, 1, 6, 19]
    for fmt in ("aether-hf-session/2", "aether-hf-session/3"):
        before = {"format": fmt, "session": {"bandwidth_hz": 500}}
        assert [field_ingest.sidecar_rung(before, m) for m in (0, 1, 2, 12)] == [0, 1, 4, 14]
    fast = {"format": "aether-hf-session/3", "session": {"bandwidth_hz": 2300}}
    assert [field_ingest.sidecar_rung(fast, m) for m in (0, 2, 19)] == [0, 2, 19]
    for bandwidth, top in ((2300, 19), (500, 14)):
        current = {"format": "aether-hf-session/4", "session": {"bandwidth_hz": bandwidth}}
        assert [field_ingest.sidecar_rung(current, m) for m in (0, 2, top)] == [0, 2, top]
    document = json.loads(_sidecar(tmp_path).read_text(encoding="utf-8"))
    document["format"] = "aether-hf-session/1"
    document["session"]["test"]["ladder"] = [
        {"mode": m, "frames": 6, "decoded": 6, "snr_db": 12.0} for m in range(14)
    ]
    session = field_ingest.summarise(document, "old")
    assert [r.mode for r in session.rungs] == list(range(6, 20))
    assert [o[0] for o in bench_link.sidecar_observations(document)] == list(range(6, 20))
    document["format"] = "aether-hf-session/2"
    document["session"]["test"]["ladder"] = [
        {"mode": m, "frames": 6, "decoded": 6, "snr_db": 12.0} for m in range(16)
    ]
    session = field_ingest.summarise(document, "floor")
    assert [r.mode for r in session.rungs] == [0, 1, *range(6, 20)]


def test_a_plain_session_is_summarised_from_its_frames() -> None:
    document = {
        "format": "aether-hf-session/1",
        "started": "2026-09-14T01:00:00.000Z",
        "audio": {"seconds": 60.0},
        "session": {"callsign": "KK4ODA", "remote": "N0CALL", "bandwidth_hz": 500},
        "frames": [
            {"t_s": 1.0, "kind": "data", "mode": 3, "snr_3k_db": 5.0, "decoded": True},
            {"t_s": 3.0, "kind": "data", "mode": 3, "snr_3k_db": 7.0, "decoded": False},
        ],
        "counters": {"bytes_delivered": 600},
    }
    session = field_ingest.summarise(document, "x")
    assert (session.frames, session.decoded, session.snr_db) == (2, 1, 6.0)
    assert session.goodput_bps == 80.0 and session.outcome == "session"
    assert field_ingest.band_of(None) is None
    assert field_ingest.band_of(14_074_000) == "20 m"
    assert field_ingest.band_of(5_000_000) == "5.000 MHz"
    with pytest.raises(ValueError):
        field_ingest.summarise({"format": "other"}, "x")


def test_the_replay_fits_the_penalty_the_ladder_shows(tmp_path: Path) -> None:
    sidecar = _sidecar(tmp_path)
    document = json.loads(sidecar.read_text(encoding="utf-8"))
    observations = bench_link.sidecar_observations(document)
    assert len(observations) == len(AWGN_THRESHOLD_DB)
    penalty = bench_link.fit_penalty(observations, dict(AWGN_THRESHOLD_DB))
    assert abs(penalty - 4.0) <= 0.5, penalty
    assert bench_link.fit_penalty([], dict(AWGN_THRESHOLD_DB)) == 0.0
    schedule = bench_link.sidecar_schedule(document)
    assert schedule is not None
    assert (
        schedule(-1.0) == 11.0 and schedule(3.0) == 12.0 and schedule(1000.0) == pytest.approx(13.0)
    )
    row = bench_link.replay_sidecar(sidecar, seed=3)
    assert row["ok"] == 1 and row["replay_of"] == sidecar.stem
    assert row["penalty_db"] == penalty and row["measured_bps"] == 1310.7
    assert row["bytes"] == 2048 + 16384
    assert float(row["goodput_bps"]) > 0.0
