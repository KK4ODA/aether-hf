"""tools/rung_reading.py: what each rung delivered, from both stations' sidecars.

A synthetic session on the 500 Hz air: a master sends three transmissions of data and two
measurements to a slave, each frame placed on both recordings' clocks with known latencies, the
slave's answers decoded at the master with their acknowledgements. The tool must put the two on
one clock, match every frame the master sent to what the slave recorded of it (or to nothing),
give each frame its sequence number from the sends and the acknowledgements, tell a block
delivered by combining from one decoded alone, read the ladder's measurements both ways, and
replay the controller to the recommendations the answers carried."""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

import rung_reading as rr  # noqa: E402
import two_sides  # noqa: E402
from aether_model.frame.modes import NARROW  # noqa: E402
from aether_model.link.frames import REPORT, data_capacity  # noqa: E402

OFFSET = 2.0
"""The slave's recording started this long after the master's: a's clock = b's + OFFSET."""
LATENCY = 0.022
HANDOVER = 0.02
TAIL = 0.102
RADIATE = two_sides.KEY_LEAD_S + LATENCY + HANDOVER + two_sides.RADIO_PATHS_S
"""From a key to the first frame on the other station's recording."""
SESSION = 5
CTL_S = NARROW.short.duration_s
RUNG_S = {r.index: r.duration_s for r in NARROW.ladder}
PAYLOAD = {r.index: r.payload_bytes for r in NARROW.ladder}


class Builder:
    """Two sidecars of one session, laid out on the master's clock."""

    def __init__(self) -> None:
        self.events: dict[str, list[dict[str, Any]]] = {"M": [], "S": []}
        self.sent: dict[str, list[dict[str, Any]]] = {"M": [], "S": []}
        self.frames: dict[str, list[dict[str, Any]]] = {"M": [], "S": []}
        self.ladder: list[dict[str, Any]] = []

    def own(self, who: str, t_master: float) -> float:
        return t_master if who == "M" else t_master - OFFSET

    def keying(
        self,
        who: str,
        key: float,
        frames: list[dict[str, Any]],
        heard: list[dict[str, Any] | None],
    ) -> float:
        """``who`` keys at ``key`` (the master's clock) and sends ``frames`` (sent records
        without times); the other station records ``heard[i]`` of frame i (its record without
        times, or None for a frame it never found). Returns where the transmission's frames
        ended on the air."""
        other = "S" if who == "M" else "M"
        own = self.own(who, key)
        air = sum(two_sides.frame_s(NARROW, f) for f in frames)
        release = own + two_sides.KEY_LEAD_S + HANDOVER + air + TAIL
        self.events[who] += [
            {"event": "ptt", "detail": "keyed", "t_s": own},
            {"event": "ptt", "detail": "released", "t_s": release},
            {
                "event": "tx_end",
                "t_s": release,
                "detail": (
                    f"played 1 drains_at 1 tail {TAIL:.3f} latency {LATENCY:.3f} deaf_for 0.750"
                ),
            },
        ]
        at = key + RADIATE
        for f, h in zip(frames, heard, strict=True):
            self.sent[who].append({**f, "t_s": own})
            seconds = two_sides.frame_s(NARROW, f)
            if h is not None:
                start = self.own(other, at)
                self.frames[other].append(
                    {
                        "kind": f["kind"],
                        "mode": f["mode"],
                        "rv": f["rv"],
                        "snr_3k_db": 8.0,
                        "confidence": 2.0,
                        "detect_confidence": 2.0,
                        "bytes": f["bytes"] if h.get("decoded") else 0,
                        "start_s": start,
                        "end_s": start + seconds,
                        "t_s": start + seconds + 0.12,
                        **h,
                    }
                )
            at += seconds
        return at

    def write(self, tmp_path: Path) -> tuple[Path, Path]:
        paths = []
        for who, call, _role, remote in (
            ("M", "W4AAA", "master", "N4BBB"),
            ("S", "N4BBB", "slave", "W4AAA"),
        ):
            test = (
                {"remote": remote, "outcome": "complete", "ladder": self.ladder}
                if who == "M"
                else None
            )
            session: dict[str, Any] = {"callsign": call, "bandwidth_hz": 500, "preroll_s": 0.0}
            if test is not None:
                session["test"] = test
            else:
                session["remote"] = remote
            doc = {
                "format": "aether-hf-session/5",
                "started": "2026-10-23T16:00:00.000Z" if who == "M" else "2026-10-23T16:00:02.000Z",
                "session": session,
                "events": sorted(self.events[who], key=lambda e: float(e["t_s"])),
                "sent": sorted(self.sent[who], key=lambda s: float(s["t_s"])),
                "frames": sorted(self.frames[who], key=lambda f: float(f["start_s"])),
                "audio": {"seconds": 100.0},
                "counters": {},
            }
            path = tmp_path / f"{who}.json"
            path.write_text(json.dumps(doc), encoding="utf-8")
            paths.append(path)
        return paths[0], paths[1]


def data(rung: int, rv: int = 0) -> dict[str, Any]:
    return {
        "kind": "data",
        "mode": rung,
        "rv": rv,
        "floor": NARROW.is_floor(rung),
        "bytes": PAYLOAD[rung],
    }


def header() -> dict[str, Any]:
    return {"kind": "control", "mode": 0, "rv": 0, "floor": False, "bytes": 7}


def decoded(text: str | None = None) -> dict[str, Any]:
    out: dict[str, Any] = {"decoded": True}
    if text is not None:
        out["control"] = text
    return out


FAILED: dict[str, Any] = {"decoded": False}


def answer(base: int, bitmap: int, rung: int, snr: int = 8, n: int = 0) -> str:
    return (
        f"Answer session {SESSION} count 0 level 0 base {base} bitmap {bitmap:#06x} "
        f"rung {rung} n {n} snr {snr}"
    )


def head(count: int, rung: int, measure: bool = False, base: int = 0, bitmap: int = 0) -> str:
    return (
        f"Header session {SESSION} count {count} level 0 base {base} bitmap {bitmap:#06x} "
        f"rung {rung} n 0 snr 1" + (" measure" if measure else "")
    )


def session(tmp_path: Path) -> tuple[Path, Path]:
    b = Builder()
    # before the session: the call the slave seeds from (the master seeds from the acceptance
    # the slave keys below, read at 8 dB)
    b.frames["S"].append(
        {"kind": "data", "mode": 0, "rv": 0, "decoded": True, "snr_3k_db": 6.2, "bytes": 24,
         "confidence": 1.0, "detect_confidence": 60.0, "start_s": 0.3, "end_s": 5.66, "t_s": 5.8}
    )  # fmt: skip
    b.events["M"].append({"event": "role", "detail": "master", "t_s": 9.5})
    b.events["S"].append({"event": "role", "detail": "slave", "t_s": 5.9 - OFFSET + OFFSET})
    # the slave's acceptance goes out as its role begins: a lone data frame the tool skips
    b.keying("S", 8.0, [data(0)], [decoded()])

    def exchange(
        key: float, frames: list[dict[str, Any]], heard: list[dict[str, Any] | None], reply: str
    ) -> None:
        end = b.keying("M", key, frames, heard)
        # the answer in its slot, read by the master
        b.keying("S", end + 0.75 - RADIATE, [header()], [decoded(reply)])

    r = 11
    # 1: three new frames; the slave decodes the first, fails the second, never finds the third
    exchange(
        12.0,
        [header(), data(r), data(r), data(r)],
        [decoded(head(3, r)), decoded(), FAILED, None],
        answer(1, 0x0000, r),
    )
    # 2: seq 1 and 2 again (second copies, RV 0), seq 3 new: seq 1 combined, 2 and 3 alone
    exchange(
        20.0,
        [header(), data(r), data(r), data(r)],
        [decoded(head(3, r, base=1)), FAILED, decoded(), decoded()],
        answer(4, 0x0000, r),
    )
    # 3: a measurement at rung 12 the slave read the header of: one frame decoded, one failed
    exchange(
        28.0,
        [header(), data(12), data(12)],
        [decoded(head(2, 12, measure=True, base=4)), decoded(), FAILED],
        answer(4, REPORT | 0x0002, r),
    )
    b.ladder.append({"mode": 12, "frames": 2, "decoded": 1, "snr_db": 9.0})
    # 4: a measurement at rung 13 whose header the slave missed: the master's ladder names it
    exchange(
        36.0,
        [header(), data(13), data(13)],
        [FAILED, decoded(), decoded()],
        answer(4, REPORT | 0x0006, r),
    )
    b.ladder.append({"mode": 13, "frames": 2, "decoded": 2, "snr_db": 9.0})
    # the end
    end = b.keying(
        "M",
        44.0,
        [header()],
        [decoded(f"Disc session {SESSION} count 0 level 0 base 0 bitmap 0x0000 rung 0 n 0 snr 1")],
    )
    b.events["M"].append({"event": "disconnected", "detail": "closed", "t_s": end + 1.5})
    b.events["S"].append(
        {"event": "disconnected", "detail": "peer disconnected", "t_s": end + 0.2 - OFFSET}
    )
    b.keying(
        "S",
        end + 0.75 - RADIATE,
        [header()],
        [
            decoded(
                f"DiscAck session {SESSION} count 0 level 0 base 0 bitmap 0x0000 rung 0 n 0 snr 8"
            )
        ],
    )
    return b.write(tmp_path)


def test_every_frame_sent_is_matched_and_numbered(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    reading = rr.read_session(master, slave)
    down = reading.directions[0]
    assert (reading.master, reading.slave, reading.bandwidth_hz) == ("W4AAA", "N4BBB", 500)
    assert down.data_transmissions == 2 and sum(tx.measure for tx in down.txs) == 2
    acc = down.accounts[11]
    assert (acc.sends, acc.first_copies, acc.never_detected, acc.failed, acc.decoded_alone) == (
        6,
        4,
        1,
        2,
        3,
    )
    assert (
        acc.delivered_alone_first,
        acc.delivered_alone_later,
        acc.delivered_combined,
        acc.lost,
    ) == (2, 1, 1, 0)
    assert acc.bytes_delivered == 4 * data_capacity(PAYLOAD[11])
    assert acc.first_copy_share == pytest.approx(2 / 4)
    blocks = down.blocks
    assert [blocks[s].how for s in range(4)] == ["alone", "combined", "alone", "alone"]
    assert [len(blocks[s].copies) for s in range(4)] == [1, 2, 2, 1]
    assert blocks[2].delivered_by is not None and blocks[2].delivered_by.copy == 2
    assert not down.flagged


def test_the_ladder_is_read_both_ways(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    reading = rr.read_session(master, slave)
    acc = reading.directions[0].accounts
    assert (acc[12].measured_sent, acc[12].measured_detected, acc[12].measured_decoded) == (2, 2, 1)
    assert (acc[12].measured_reported, acc[12].measured_snr_db) == (1, 9.0)
    # the header the slave missed: the master's ladder names the transmission
    assert (acc[13].measured_sent, acc[13].measured_decoded, acc[13].measured_reported) == (2, 2, 2)
    assert acc[12].sends == 0 and acc[13].sends == 0


def test_the_recommendations_and_the_regret_are_read(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    reading = rr.read_session(master, slave)
    down = reading.directions[0]
    assert [r for _, r, _ in down.recommendations] == [11, 11, 11, 11]
    assert down.answers_read == 4  # the DISC_ACK acknowledges nothing and is not one
    regret = next(g for g in down.regrets if g.rung == 11)
    # 2 exchanges of 8 s each carried 4 blocks of 78 bytes
    assert regret.air_s == pytest.approx(16.0)
    assert regret.actual_bps == pytest.approx(8 * 4 * 78 / 16.0)
    # the neighbours on the ladder (rung 8 is not usable): 10 unmeasured, 12 measured 1 of 2
    assert [n.rung for n in regret.neighbours] == [12]
    overhead = 8.0 - 3 * RUNG_S[11]
    expected = 8 * 6 * data_capacity(PAYLOAD[12]) * 0.5 / (overhead + 6 * RUNG_S[12])
    assert regret.neighbours[0].bps == pytest.approx(expected)
    assert regret.best_all is not None and regret.best_all.rung == 13


def test_the_controller_replays_to_the_recommendations_sent(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    reading = rr.read_session(master, slave, replay_controller=True)
    down = reading.directions[0]
    compared = [s for s in down.steps if s.rung_sent is not None]
    assert len(compared) == 4
    # the same controller, fed by hand as the engine feeds it: the call's reading seeds it (a
    # floor frame: a lower bound), then each transmission's outcome
    ctl = rr.controller_for(NARROW)
    ctl.seed(6.2, lower_bound=True)
    ctl.observe(8.0, 1, 1, 11)  # frame 0 decoded, frame 1 failed, frame 2 never there
    want = [ctl.recommend()]
    ctl.observe(8.0, 3, 0, 11)  # seq 1 combined, 2 and 3 alone
    want.append(ctl.recommend())
    # the measurement at rung 12 lost a frame above any rung the slave asked for: the sender's
    # choice, not news of the path, so only its SNR counts (ADR-0020)
    ctl.observe_snr(8.0)
    want.append(ctl.recommend())
    ctl.observe(8.0, 2, 0, 13)
    want.append(ctl.recommend())
    assert [s.rung_replayed for s in compared] == want
    assert [(s.ok, s.failed) for s in compared] == [(1, 1), (3, 0), (1, 1), (2, 0)]
    # the master's controller, seeded from the acceptance it read, recommended in each header
    # the slave decoded; the slave sent no data, so the seed's rung stands
    up = reading.directions[1]
    heads = [s for s in up.steps if s.rung_sent is not None]
    assert [s.rung_sent for s in heads] == [11, 11, 12]  # the fourth header went unread
    master_ctl = rr.controller_for(NARROW)
    master_ctl.seed(8.0, lower_bound=True)  # the acceptance, a floor frame read at 8 dB
    assert [s.rung_replayed for s in heads] == [master_ctl.recommend()] * 3


def test_pairs_are_found_among_sidecars(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    decoy = tmp_path / "other.json"
    decoy.write_text(
        json.dumps(
            {
                "format": "aether-hf-session/5",
                "started": "2026-10-23T16:00:01Z",
                "session": {"callsign": "K4ZZZ", "remote": "W4AAA"},
                "events": [],
            }
        ),
        encoding="utf-8",
    )
    (tmp_path / "not.json").write_text("[]", encoding="utf-8")
    assert rr.pairs(sorted(tmp_path.glob("*.json"))) == [(master, slave)]


def test_a_control_frame_is_read_as_the_daemon_spells_it() -> None:
    ack = rr.parse_control(
        "Answer session 84 count 0 level 0 base 13 bitmap 0x0005 rung 11 n 0 snr -3 fam floor flag",
        1.0,
    )
    assert ack is not None
    assert (ack.kind, ack.base, ack.bitmap, ack.rung, ack.snr, ack.measure) == (
        "Answer",
        13,
        5,
        11,
        -3.0,
        False,
    )
    assert ack.reports and ack.measured(1) and not ack.measured(0)
    assert not ack.received(13)  # a report acknowledges nothing at or above base
    assert ack.received(12)
    plain = rr.parse_control(
        "Header session 84 count 6 on the floor level 1 base 2 bitmap 0x0004 rung 5 n 2 "
        "snr - measure"
    )
    assert plain is not None and plain.measure and plain.count == 6 and plain.snr is None
    assert plain.received(4) and not plain.received(3)
    assert rr.parse_control("nothing like it") is None


def _numbered(master: Path, slave: Path) -> None:
    """Give the session's sidecars what beta.122 records: the sender's sequence number on every
    data frame it sends, and on every one the receiver hears the number its engine filed it under
    and whether copies combined into it."""
    doc = json.loads(master.read_text(encoding="utf-8"))
    sent = [s for s in doc["sent"] if s["kind"] == "data"]
    seqs = [0, 1, 2, 1, 2, 3, 0, 1, 0, 1]  # two exchanges of data, then two measurements
    assert len(sent) == len(seqs)
    for s, seq in zip(sent, seqs, strict=True):
        s["seq"] = seq
    for s in sent[6:]:
        s["measure"] = True
    master.write_text(json.dumps(doc), encoding="utf-8")
    doc = json.loads(slave.read_text(encoding="utf-8"))
    heard = [f for f in doc["frames"] if f["kind"] == "data" and f["mode"] == 11]
    notes: list[dict[str, Any]] = [
        {"seq": 0},
        {"seq": 1},
        {"seq": 1, "combined": True, "rescued": True},
        {"seq": 2},
        {"seq": 3},
    ]
    assert len(heard) == len(notes)
    for f, note in zip(heard, notes, strict=True):
        f.update(note)
    slave.write_text(json.dumps(doc), encoding="utf-8")


def test_the_numbers_a_sidecar_records_are_taken_and_checked(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    reconstructed = rr.read_session(master, slave).directions[0]
    _numbered(master, slave)
    reading = rr.read_session(master, slave)
    down = reading.directions[0]
    assert down.numbering == "recorded"
    assert down.agrees == (10, 10)  # the reconstruction numbers every frame as recorded
    assert [down.blocks[s].how for s in range(4)] == ["alone", "combined", "alone", "alone"]
    assert vars(down.accounts[11]) == vars(reconstructed.accounts[11])
    assert reconstructed.numbering == "reconstructed" and reconstructed.agrees is None
    # the slave sent no data: nothing to number
    assert reading.directions[1].numbering == "reconstructed"


def test_a_recorded_number_wins_over_the_reconstruction(tmp_path: Path) -> None:
    master, slave = session(tmp_path)
    _numbered(master, slave)
    doc = json.loads(master.read_text(encoding="utf-8"))
    sent = [s for s in doc["sent"] if s["kind"] == "data"]
    # say the second exchange's first frame was a new block, not seq 1 again
    sent[3]["seq"] = 4
    master.write_text(json.dumps(doc), encoding="utf-8")
    down = rr.read_session(master, slave).directions[0]
    assert down.numbering == "recorded"
    assert down.agrees == (9, 10)
    assert 4 in down.blocks and len(down.blocks[1].copies) == 1


def test_c1_at_the_sender_replays_as_the_engine_feeds_it(tmp_path: Path) -> None:
    """``replay_sender`` (Gate 3's K6 for ``delivery-s``): seeded by the slave's reading of the
    call, each acknowledgement judging the copies since the last — the newly acknowledged
    block's latest copy delivered it, every other copy air for nothing — a step per data
    transmission, the measurements none."""
    from aether_model.link.rate_review import DeliveryController, FrameOutcome

    master, slave = session(tmp_path)
    reading = rr.read_session(master, slave)
    down = reading.directions[0]
    steps = rr.replay_sender(rr.load_side(master), rr.load_side(slave), down.txs, down.blocks)
    ctl = rr.controller_for(NARROW, controller="delivery")
    assert isinstance(ctl, DeliveryController)
    ctl.seed(6.2)
    first = ctl.modes[ctl._index]
    air = RUNG_S[11]
    cap = data_capacity(rr.timing_of(NARROW).capacity(11))

    def copy(ok: bool) -> FrameOutcome:
        return FrameOutcome(11, air, cap, True, ok, False, None)

    ctl.observe_frames([copy(True), copy(False), copy(False)], 8.0)  # seq 0 acknowledged
    second = ctl.modes[ctl._index]
    assert [(s.rung_advised, s.rung_chosen, s.seeded) for s in steps] == [
        (11, first, True),
        (11, second, True),
    ]
