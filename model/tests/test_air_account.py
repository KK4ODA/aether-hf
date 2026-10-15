"""The harness's account of a collision: the two transmissions in it and why each was keyed
(``tools/air_account.py``, ADR-0069 §5)."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import air_account as aa  # noqa: E402


def tx(air_s: float, cause: str, kinds: list[str], waited_s: float = 0.0) -> dict:
    return {
        "air_s": air_s,
        "cause": cause,
        "role": "Iss",
        "waited_s": waited_s,
        "frames": [
            {"c": "control" if k[0].isupper() and k != "Data" else "data", "kind": k} for k in kinds
        ],
    }


def report(a: list[dict], b: list[dict], overlaps: list[list[float]]) -> dict:
    return {
        "latency_s": 0.25,
        "stations": {"A": {"transmissions": a}, "B": {"transmissions": b}},
        "rf_overlaps": overlaps,
    }


def test_a_turn_request_is_named_whoever_keyed_second() -> None:
    r = report(
        [tx(10.0, "tx_done", ["Data", "Data"])],
        [tx(11.0, "timer:request", ["Ack"])],
        [[11.1, 11.5]],
    )
    (line,) = aa.account(r)
    assert line["mechanism"] == "request"
    assert line["first"].startswith("A ") and line["second"].startswith("B ")


def test_an_answer_queued_before_the_other_began_was_held() -> None:
    # B decided its ACK at 9.0 s (waited 1.5 s in its queue, 0.25 s in its card): A's repeat
    # began at 10.0 s, B's ACK went on the air over it at 10.75 s
    r = report(
        [tx(10.0, "timer:wait", ["Data"])], [tx(10.75, "timer:ack", ["Ack"], 1.5)], [[10.8, 11.2]]
    )
    assert aa.account(r)[0]["mechanism"] == "answer held"


def test_an_answer_decided_over_a_repeat_meets_it() -> None:
    r = report(
        [tx(10.0, "timer:resend", ["Data"])], [tx(10.5, "timer:ack", ["Ack"])], [[10.6, 11.0]]
    )
    assert aa.account(r)[0]["mechanism"] == "answer over a repeat"


def test_an_answer_over_the_burst_it_answers_is_early() -> None:
    r = report(
        [tx(10.0, "tx_done", ["Data", "Data", "Data"])],
        [tx(11.0, "timer:ack", ["Ack"])],
        [[11.1, 11.5]],
    )
    assert aa.account(r)[0]["mechanism"] == "answer over a burst"


def test_a_retry_over_an_answer_and_two_initiators_are_told_apart() -> None:
    retry = report(
        [tx(10.0, "timer:ack", ["Ack"])], [tx(10.2, "timer:wait", ["Poll"])], [[10.3, 10.6]]
    )
    assert aa.account(retry)[0]["mechanism"] == "repeat over an answer"
    both = report(
        [tx(10.0, "timer:keepalive", ["Poll"])], [tx(10.2, "tx_done", ["Data"])], [[10.3, 10.6]]
    )
    assert aa.account(both)[0]["mechanism"] == "both initiating"


def test_a_disc_or_its_answer_is_leaving_and_the_summary_counts() -> None:
    r = report(
        [tx(10.0, "abort", ["Disc"]), tx(20.0, "tx_done", ["Data"])],
        [tx(10.5, "timer:ack", ["Ack"]), tx(21.0, "timer:request", ["Ack"])],
        [[10.6, 11.0], [21.1, 21.4]],
    )
    lines = aa.account(r)
    assert [line["mechanism"] for line in lines] == ["leaving", "request"]
    assert aa.summary(lines) in ("leaving 1; request 1", "request 1; leaving 1")


def test_a_session_on_the_air_names_each_unanswered_transmission() -> None:
    import numpy as np

    import two_sides

    def side(name: str, frames: list[dict[str, Any]], role: str) -> two_sides.Side:
        doc = {
            "session": {"callsign": name, "bandwidth_hz": 500},
            "events": [
                {"event": "recording", "detail": "started x.wav", "t_s": 0.0},
                {"event": "role", "detail": role, "t_s": 0.0},
            ],
            "frames": frames,
            "audio": {"seconds": 200.0},
        }
        return two_sides.Side(name, doc, np.zeros(1), 8000, two_sides.NARROW)

    def keying(who: str, rf: float, sent: list[tuple[str, int, float]]) -> two_sides.Keying:
        frames = sum(s for *_, s in sent)
        return two_sides.Keying(
            who, rf - 0.3, rf + frames, frames, 0.1, 0.022, 0.0, rf_start=rf, sent=sent
        )

    header, r3, ctl = ("control", 0, 0.43), ("data", 3, 5.36), ("control", 0, 3.2)
    slave_frames = [
        # 1: the header read: answered
        {"start_s": 10.0, "kind": "control", "decoded": True},
        # 2: no header, the last frame decoded with its count of 0: placed, answered
        {"start_s": 30.0, "kind": "data", "mode": 3, "decoded": False},
        {"start_s": 35.36, "kind": "data", "mode": 3, "decoded": True, "follows": 0},
        # 3: no header, both frames failed: a silent slot the elision cost
        {"start_s": 60.0, "kind": "data", "mode": 3, "decoded": False},
        {"start_s": 65.36, "kind": "data", "mode": 3, "decoded": False},
        # 4: the header detected, not read
        {"start_s": 90.0, "kind": "control", "decoded": False},
        # 5: nothing recorded at all
        # 6: the header read, and nothing keyed: a forfeit
        {"start_s": 150.0, "kind": "control", "decoded": True},
    ]
    master = side("M", [{"start_s": 11.2, "kind": "control", "decoded": True}], "master")
    slave = side("S", slave_frames, "slave")
    master.keyings = [
        keying("M", 10.0, [header]),
        keying("M", 30.0, [r3, r3]),
        keying("M", 60.0, [r3, r3]),
        keying("M", 90.0, [header]),
        keying("M", 120.0, [header]),
        keying("M", 150.0, [header]),
    ]
    master.keyings[4].rf_start = None  # the slave recorded nothing of it
    slave.keyings = [keying("S", 11.2, [ctl]), keying("S", 41.5, [ctl])]
    lines = aa.field_account(master, slave, 0.0)
    assert [line["what"] for line in lines] == [
        "answered",
        "answer unheard",
        "no grant: header elided",
        "no grant: header lost",
        "no grant: nothing heard",
        "forfeit",
    ]
    assert lines[2]["read"] == "✗✗" and lines[4]["read"] == "·"
