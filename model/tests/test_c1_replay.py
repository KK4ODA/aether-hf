"""``tools/c1_replay.py``: C1 replayed on a session's own frames, the third rate review's Gate 0
(``docs/research/rate-control-3/00-brief.md``)."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

from test_rung_reading import wrapped_session  # noqa: E402

import c1_replay  # noqa: E402
import rung_reading as rr  # noqa: E402


def test_a_session_past_256_frames_is_replayed_direction_by_direction(tmp_path: Path) -> None:
    """The shape of the 2026-11-12 14:13Z session (316 data frames, tries of rung 12, the
    sequence numbers wrapped): every data transmission the replay could choose for is judged,
    every try that ended has its verdict and margin, and the blocked time is part of the
    exchanges'."""
    master, slave = wrapped_session(tmp_path)
    reading, directions = c1_replay.replay_session(master, slave)
    down = next(d for d in directions if d.role == "master")
    data = [tx for tx in reading.directions[0].txs if tx.data and not tx.measure]
    assert 0 < down.data_transmissions + down.copies_only <= len(data)
    assert 0 <= down.agree <= down.data_transmissions
    assert 0 <= down.below <= down.data_transmissions - down.agree
    assert len(down.states) == len(reading.directions[0].txs)
    assert 0.0 <= down.blocked_s <= down.exchange_s
    for t in down.tries:
        assert t.frames >= t.decoded >= 0
        if t.end is not None:
            assert t.won is not None and t.margin is not None and t.end >= t.start
    assert all(s["choice"] in range(15) for s in down.states)


def test_a_transmission_of_copies_alone_is_no_choice_of_c1s() -> None:
    """A transmission of blocks already sent goes at their own rungs (or a stuck-frame rule's),
    not at C1's choice: agreement is judged on the new blocks of a transmission, and one of
    copies alone is counted apart. On 2026-11-18's QRM-bound session four of nine transmissions
    were copies of three stuck blocks, and judged as choices they read as the replay parting
    from the session (report 19)."""

    def frame(rung: int, copy: int) -> rr.Frame:
        return rr.Frame("data", rung, 0, 150, False, 0.0, 5.36, copy=copy)

    copies = rr.Transmission(0, 0.0, 0.0, [frame(5, 2), frame(5, 3)])
    assert c1_replay.new_blocks_rung(copies) is None
    mixed = rr.Transmission(1, 0.0, 0.0, [frame(5, 4), frame(3, 1), frame(3, 1), frame(2, 1)])
    assert c1_replay.new_blocks_rung(mixed) == 3
    tie = rr.Transmission(2, 0.0, 0.0, [frame(4, 1), frame(3, 1)])
    assert c1_replay.new_blocks_rung(tie) == 3


def test_a_near_miss_is_a_lost_try_within_five_per_cent() -> None:
    d = c1_replay.Direction("A", "B", "master")
    d.tries = [
        c1_replay.Try(12, 80.0, 3, 3, 3, 105.0, won=False, margin=-0.018),  # 2026-11-12, 2300 Hz
        c1_replay.Try(12, 220.0, 2, 3, 0, 236.0, won=False, margin=-0.72),
        c1_replay.Try(11, 194.0, 3, 3, 3, 211.0, won=True, margin=0.089),
        c1_replay.Try(13, 300.0, 1, 1, 1),  # still running when the session ended
    ]
    assert d.near_misses == 1


def test_the_inbox_is_read_and_written_as_json(tmp_path: Path, capsys: object) -> None:
    master, slave = wrapped_session(tmp_path)
    out = tmp_path / "replay.json"
    argv = sys.argv
    sys.argv = ["c1_replay.py", "--inbox", str(tmp_path), "--json", str(out)]
    try:
        assert c1_replay.main() == 0
    finally:
        sys.argv = argv
    dump = json.loads(out.read_text(encoding="utf-8"))
    assert len(dump) == 1 and dump[0]["bandwidth_hz"] == 500
    roles = {d["role"] for d in dump[0]["directions"]}
    assert "master" in roles
    assert all("near_misses" in d and "tries" in d for d in dump[0]["directions"])
    del master, slave
