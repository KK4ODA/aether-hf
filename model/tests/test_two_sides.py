"""tools/two_sides.py: two stations' recordings of one session on one clock (ADR-0069 Step 4).

Two synthetic recordings of one air: static crashes both stations hear, each station's own
noise, each station muted while it transmits, and sidecars whose keyings, sent frames and
received frames are placed with known latencies. The tool must recover the offset from the
audio alone, the handover delays and loop latencies, the gaps on the air and each answer's
lateness against its slot."""

from __future__ import annotations

import json
import sys
import wave
from pathlib import Path

import numpy as np
import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import two_sides  # noqa: E402

FS = 8000
SECONDS = 60.0
OFFSET = 3.217
"""A's recording clock less B's, for a sample both heard at once."""
HANDOVER = {"A": 0.090, "B": 0.020}
LOOP = {"A": 0.070, "B": 0.065}
LATENCY = 0.022
TAIL = 0.102
FLOOR_CTL = 3.2


def _wav(path: Path, x: np.ndarray) -> None:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(FS)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype(np.int16).tobytes())


def _session(tmp_path: Path) -> tuple[Path, Path, dict[str, list[float]]]:
    rng = np.random.default_rng(7)
    n = int(SECONDS * FS)
    # the air, on A's clock: crashes every half second or so, a few ms long, of varied size
    crashes = np.zeros(int((SECONDS + OFFSET + 1) * FS))
    t = 0.2
    while t < SECONDS + OFFSET:
        i = int(t * FS)
        crashes[i : i + 40] += rng.normal(size=40) * rng.uniform(0.2, 0.8)
        t += rng.exponential(0.5)
    rec = {
        "A": crashes[:n] + 0.01 * rng.normal(size=n),
        # B's recording clock runs OFFSET behind A's: B's sample j is A's j + OFFSET
        "B": crashes[int(OFFSET * FS) : int(OFFSET * FS) + n] + 0.01 * rng.normal(size=n),
    }
    # A (the master) keys a floor header, B answers in the slot, three times; A's clock
    keys: dict[str, list[float]] = {"A": [], "B": []}
    events: dict[str, list[dict]] = {"A": [], "B": []}
    sent: dict[str, list[dict]] = {"A": [], "B": []}
    frames: dict[str, list[dict]] = {"A": [], "B": []}  # what each recorded of the other
    late = [0.08, 0.15, 0.12]
    t_a = 5.0
    a_end = 0.0
    for x in range(3):
        for who in ("A", "B"):
            if who == "B":
                # the slot: A's frames end on the air, the gap, and this answer's lateness
                slot = a_end + 0.30 + 0.35 + 0.05
                rf = slot + late[x]
                key_on_a = rf - (0.1 + LATENCY + HANDOVER["B"] + LOOP["B"])
            else:
                key_on_a = t_a
                rf = key_on_a + 0.1 + LATENCY + HANDOVER["A"] + LOOP["A"]
            own = key_on_a if who == "A" else key_on_a - OFFSET
            release = own + 0.1 + HANDOVER[who] + FLOOR_CTL + TAIL
            keys[who].append(own)
            events[who] += [
                {"event": "ptt", "detail": "keyed", "t_s": own},
                {"event": "ptt", "detail": "released", "t_s": release},
                {
                    "event": "tx_end",
                    "t_s": release,
                    "detail": f"played 1 drains_at 1 tail {TAIL:.3f} latency {LATENCY:.3f} "
                    "deaf_for 0.750",
                },
            ]
            sent[who].append(
                {"kind": "control", "floor": True, "mode": 0, "rv": 0, "bytes": 7, "t_s": own}
            )
            # the other station records the frame where it was on the air, on its own clock
            other = "B" if who == "A" else "A"
            start = rf - OFFSET if other == "B" else rf
            text = (
                "Header session 1 count 0 level 0 base 0 bitmap 0x0000 rung 3 n 0 snr -3 fam floor"
            )
            frames[other].append(
                {
                    "kind": "control",
                    "mode": 0,
                    "rv": 0,
                    "decoded": True,
                    "start_s": start,
                    "end_s": start + FLOOR_CTL,
                    "t_s": start + FLOOR_CTL + 0.12,
                    "snr_3k_db": -3.0,
                    "control": text if who == "A" else "Answer session 1 count 0",
                }
            )
            # this station's own recording is muted from its key, a capture path later
            i0 = int((own + HANDOVER[who] + 0.01) * FS)
            i1 = int((release + 0.05) * FS)
            rec[who][i0:i1] = 0.0
            if who == "A":
                a_end = rf + FLOOR_CTL
        t_a = a_end + 12.0
    paths = []
    for who, call in (("A", "MASTER"), ("B", "SLAVE")):
        doc = {
            "format": "aether-hf-session/4",
            # B's recording started a fifth of a second off what the offset says: the stamps are
            # a few tenths from the truth and the audio has to settle it
            "started": "2026-10-14T23:59:00.000Z" if who == "A" else "2026-10-14T23:59:03.417Z",
            "session": {"callsign": call, "bandwidth_hz": 500, "preroll_s": 0.0},
            "events": [
                {"event": "role", "detail": "master" if who == "A" else "slave", "t_s": 0.0},
                *events[who],
            ],
            "sent": sent[who],
            "frames": frames[who],
        }
        path = tmp_path / f"{who}.json"
        path.write_text(json.dumps(doc), encoding="utf-8")
        _wav(path.with_suffix(".wav"), rec[who])
        paths.append(path)
    return paths[0], paths[1], keys


def test_two_recordings_are_put_on_one_clock_and_the_slots_read(tmp_path: Path) -> None:
    a_path, b_path, keys = _session(tmp_path)
    a, b = two_sides.load(a_path), two_sides.load(b_path)
    for side in (a, b):
        side.keyings = two_sides.keyings(side)
        two_sides.mute_onsets(side)
    guess = two_sides.started_offset(a, b)
    assert abs(guess - (OFFSET + 0.2)) < 1e-6
    ea, la = two_sides.envelope(a, (2700.0, 3000.0))
    eb, lb = two_sides.envelope(b, (2700.0, 3000.0))
    lag, r, _ = two_sides.best_lag(ea, la, eb, lb, (0.0, SECONDS), guess, 0.6)
    assert abs(lag - OFFSET) <= 0.002 and r > 0.5, (lag, r)
    two_sides.place(a, b, lag)
    for side, who in ((a, "A"), (b, "B")):
        assert [round(k.key, 6) for k in side.keyings] == [round(k, 6) for k in keys[who]]
        for k in side.keyings:
            assert k.handover_s == pytest.approx(HANDOVER[who], abs=1e-6)
            assert k.loop_s == pytest.approx(LOOP[who], abs=0.003)
            assert k.mute_s == pytest.approx(HANDOVER[who] + 0.01, abs=0.003)
    changes = two_sides.gaps(a, b)
    assert len(changes) == 5 and min(g for *_, g in changes) > 0.6
    rows = two_sides.answers(a, b, lag)
    assert [round(row["late"], 3) for row in rows] == pytest.approx([0.08, 0.15, 0.12], abs=0.003)
    assert min(row["margin"] for row in rows) == pytest.approx(0.35 - 0.15, abs=0.003)


def test_an_overlap_is_a_negative_gap() -> None:
    k = two_sides.Keying("A", 0.0, 4.0, 3.0, TAIL, LATENCY, 0.0, rf_start=1.0)
    j = two_sides.Keying("B", 3.5, 6.0, 2.0, TAIL, LATENCY, 0.0, rf_start=3.8)
    side_a = two_sides.Side("A", {}, np.zeros(1), FS, two_sides.NARROW, [k])
    side_b = two_sides.Side("B", {}, np.zeros(1), FS, two_sides.NARROW, [j])
    [(first, second, gap)] = two_sides.gaps(side_a, side_b)
    assert (first.who, second.who) == ("A", "B") and gap == pytest.approx(-0.2)


def _exchange(
    steps_at: float | None = None, step: float = 0.0, drift_ppm: float = 0.0
) -> tuple[two_sides.Side, two_sides.Side]:
    """A master and a slave trading floor control frames every 8 s for six minutes, with no
    audio both heard: B's recording clock is A's less OFFSET, less ``step`` from ``steps_at``
    on (audio B lost), and running ``drift_ppm`` slow."""

    def b_clock(t_a: float) -> float:
        lost = step if steps_at is not None and t_a >= steps_at else 0.0
        return (t_a - OFFSET) * (1.0 - drift_ppm * 1e-6) - lost

    sides = {}
    docs: dict[str, dict] = {w: {"events": [], "sent": [], "frames": []} for w in ("A", "B")}
    t = 5.0
    while t < 360.0:
        for who in ("A", "B"):
            other = "B" if who == "A" else "A"
            key_a = t
            rf = key_a + 0.1 + LATENCY + HANDOVER[who] + LOOP[who]
            own = (lambda x: x) if who == "A" else b_clock
            release_a = key_a + 0.1 + HANDOVER[who] + FLOOR_CTL + TAIL
            docs[who]["events"] += [
                {"event": "ptt", "detail": "keyed", "t_s": own(key_a)},
                {"event": "ptt", "detail": "released", "t_s": own(release_a)},
                {
                    "event": "tx_end",
                    "t_s": own(release_a),
                    "detail": f"played 1 drains_at 1 tail {TAIL:.3f} latency {LATENCY:.3f}",
                },
            ]
            docs[who]["sent"].append(
                {"kind": "control", "floor": True, "mode": 0, "rv": 0, "t_s": own(key_a)}
            )
            heard_at = rf if other == "A" else b_clock(rf)
            docs[other]["frames"].append(
                {"kind": "control", "mode": 0, "decoded": True, "start_s": heard_at}
            )
            t = rf + FLOOR_CTL + 0.7
        t += 1.0
    for who, call in (("A", "MASTER"), ("B", "SLAVE")):
        docs[who]["events"].sort(key=lambda e: e["t_s"])
        docs[who]["frames"].sort(key=lambda f: f["start_s"])
        doc = {"session": {"callsign": call, "bandwidth_hz": 500}, **docs[who]}
        side = two_sides.Side(call, doc, np.zeros(1), FS, two_sides.NARROW)
        side.keyings = two_sides.keyings(side)
        sides[who] = side
    return sides["A"], sides["B"]


def test_the_loops_fix_the_offset_when_nothing_was_heard_by_both() -> None:
    a, b = _exchange()
    # the stamps are a few tenths off; the loops' sum does not care and their difference fixes it
    two_sides.place(a, b, OFFSET + 0.2)
    series = two_sides.loop_offsets(a, b, OFFSET + 0.2)
    assert len(series) > 40
    # the two radios differ by 5 ms (LOOP), so equal loops put the offset 2.5 ms off
    skew = (LOOP["B"] - LOOP["A"]) / 2
    assert all(p.offset == pytest.approx(OFFSET + skew, abs=1e-6) for p in series)
    assert all(p.loop_sum_s == pytest.approx(LOOP["A"] + LOOP["B"], abs=1e-6) for p in series)
    assert two_sides.offset_steps(series) == []


def test_audio_one_recording_lost_is_a_step_in_the_loops_offset() -> None:
    a, b = _exchange(steps_at=200.0, step=0.092, drift_ppm=40.0)
    two_sides.place(a, b, OFFSET)
    series = two_sides.loop_offsets(a, b, OFFSET)
    [(before, after, jump)] = two_sides.offset_steps(series)
    assert before < 200.0 <= after and after - before < 20.0
    assert jump == pytest.approx(0.092, abs=0.003)
    ppm = two_sides.drift_ppm(series, two_sides.offset_steps(series))
    assert ppm == pytest.approx(40.0, abs=5.0)
    # placed with the offset of its time, every keying reads its own loop again
    along = two_sides.offset_along(series, OFFSET)
    two_sides.place(a, b, along)
    skew = (LOOP["B"] - LOOP["A"]) / 2
    for side, who in ((a, "A"), (b, "B")):
        for k in side.keyings[2:-2]:
            assert k.loop_s == pytest.approx(LOOP[who] + (skew if who == "A" else -skew), abs=0.004)


def test_a_transmission_is_placed_by_the_frames_recorded_of_it() -> None:
    k = two_sides.Keying(
        "A",
        0.0,
        30.0,
        3.2 + 4 * 5.36,
        TAIL,
        LATENCY,
        0.05,
        sent=[("control", 0, 3.2)] + [("data", 3, 5.36)] * 4,
    )
    start = 0.3
    heard = two_sides.Heard
    # the header and the first data frame were never recorded: frames 2 and 4 place it
    two_four = [heard(start + 3.2 + 5.36, "data", 3), heard(start + 3.2 + 3 * 5.36, "data", 3)]
    assert two_sides.transmission_start(k, two_four, expected=0.25) == pytest.approx(start)
    # a lone frame of a burst of one rung could be any of the four: its count says which
    third = [heard(start + 3.2 + 2 * 5.36, "data", 3, follows=1)]
    assert two_sides.transmission_start(k, third, expected=5.0) == pytest.approx(start)
    # without a count, the position the key predicts
    lone = [heard(start + 3.2 + 2 * 5.36, "data", 3)]
    assert two_sides.transmission_start(k, lone, expected=0.25) == pytest.approx(start)
    # detections that never decoded cannot outvote the frame that did
    phantoms = [
        heard(start - 0.3, "data", 3, decoded=False),
        heard(start - 0.3 + 5.36, "data", 3, decoded=False),
        heard(start + 3.2 + 3 * 5.36, "data", 3, follows=0),
    ]
    assert two_sides.transmission_start(k, phantoms, expected=0.0) == pytest.approx(start)
    # a frame read as another kind fits nowhere
    assert two_sides.transmission_start(k, [heard(5.0, "data", 9)], expected=0.25) is None


def test_a_transmission_only_a_phantom_was_recorded_of_is_placed_by_its_key() -> None:
    heard = two_sides.Heard
    # the answer's span was taken by a phantom read as a data frame 0.44 s before it
    phantom = [heard(212.384, "data", 8, decoded=False)]
    assert two_sides.rf_start_of(None, phantom, expected=212.85) == (212.85, False)
    # a decoded frame that fits no position still places it, as before
    decoded = [heard(212.384, "data", 8, decoded=False), heard(212.9, "control", 0)]
    assert two_sides.rf_start_of(None, decoded, expected=212.85) == (212.9, True)
    assert two_sides.rf_start_of(212.8, phantom, expected=212.85) == (212.8, True)


def test_an_answer_belongs_to_the_last_transmission_before_it() -> None:
    def keying(who: str, key: float, rf: float, frames: float) -> two_sides.Keying:
        return two_sides.Keying(
            who, key, key + frames + 0.3, frames, TAIL, LATENCY, 0.0, rf_start=rf
        )

    # the master's data went unanswered (a silent slot), it polled, and the poll was answered
    master = two_sides.Side(
        "M",
        {"frames": []},
        np.zeros(1),
        FS,
        two_sides.NARROW,
        [keying("M", 0.0, 0.25, 4.22), keying("M", 6.3, 6.55, 0.43)],
    )
    answer_rf = 6.55 + 0.43 + 0.30 + 0.35 + 0.05 + 0.09
    slave = two_sides.Side(
        "S", {"frames": []}, np.zeros(1), FS, two_sides.NARROW, [keying("S", 7.6, answer_rf, 3.2)]
    )
    [row] = two_sides.answers(master, slave, 0.0)
    assert row["master_key"] == 6.3 and row["late"] == pytest.approx(0.09)


def test_a_call_s_acceptance_is_judged_at_the_level_its_try_granted() -> None:
    def keying(key: float, rf: float | None, frames: float, state: str) -> two_sides.Keying:
        return two_sides.Keying(
            "M", key, key + frames + 0.3, frames, TAIL, LATENCY, 0.0, rf_start=rf, state=state
        )

    # WC4Y's Test of 2026-10-24: a probe, then four tries of the call, the families alternating;
    # the slave recorded only the fourth, which granted level 1 (up one every second try)
    tries = [
        keying(0.0, None, 5.36, "Idle"),
        keying(14.0, None, 5.36, "Connecting"),
        keying(35.0, None, 1.05, "Connecting"),
        keying(46.0, None, 5.36, "Connecting"),
        keying(79.0, 79.25, 1.05, "Connecting"),
    ]
    assert [two_sides.call_levels(tries).get(id(k)) for k in tries] == [None, 0, 0, 1, 1]
    master = two_sides.Side("M", {"frames": []}, np.zeros(1), FS, two_sides.NARROW, tries)
    gap = two_sides.DECODE_BUDGET_S[0] + two_sides.KEY_S + two_sides.GUARD_S
    accepted = 79.25 + 1.05 + gap + two_sides.LEVELS_S[1] + 0.09
    slave = two_sides.Side(
        "S",
        {"frames": []},
        np.zeros(1),
        FS,
        two_sides.NARROW,
        [two_sides.Keying("S", accepted - 0.3, accepted + 5.66, 5.36, TAIL, LATENCY, 0.0,
                          rf_start=accepted)],
    )  # fmt: skip
    [row] = two_sides.answers(master, slave, 0.0)
    assert row["late"] == pytest.approx(0.09)
    # a call made again after a session starts its tries over
    again = [*tries, keying(200.0, 200.25, 3.2, "Connected"), keying(240.0, None, 5.36,
                                                                   "Connecting")]  # fmt: skip
    assert two_sides.call_levels(again)[id(again[-1])] == 0
