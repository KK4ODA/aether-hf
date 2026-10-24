"""What each rung delivered in one session, read from both stations' recordings (the rate
controller's review, `docs/research/rate-control/00-brief.md` §4, Gate 0).

    python tools/rung_reading.py MASTER.json SLAVE.json [--replay] [--json OUT] [--markdown]
    python tools/rung_reading.py --inbox DIR [--replay] [--csv OUT] [--json OUT] [--markdown]

A sidecar records what its station *sent* (each frame's rung, redundancy version and size) and
what it *heard* (each frame found, decoded or not); neither says what became of a frame at the
other end. With both put on one clock (``tools/two_sides.py``, the loops' offset at each keying),
every frame the sender put on the air is matched to the record the receiver kept of it, or to
none. The sender's acknowledgements, as it decoded them, say which frames arrived, and from them
each frame's sequence number is reconstructed (the engine sends the unacknowledged frames first,
then new ones, with the redundancy versions 0, 0, 2, 3 — ``granted._data_frames``), so a frame
delivered only by combining copies is told from one that decoded on its own.

Per rung, in each direction: frames sent (every copy), first copies, never detected, detected but
failed, decoded alone, blocks delivered by a first copy, by a later copy and by combining, lost;
the frame air and the whole exchange air spent at it; the data delivered per second of each. The
receiver's recommendations over time (from the answers the sender decoded), the Test's ladder
(what the master recorded of each measurement, and what the slave's receiver made of the same
frames), what each rung either side of the one used delivered where it was measured, and the
regret: what the best measured neighbour would have delivered in the air spent.

``--replay`` runs the current ``RateController`` on what the receiver recorded — the connect
frame it seeded from, every decoded frame's echo, and each transmission's outcome as the engine
counts it (``granted._observe``: a failure is a trusted frame at a self-decodable RV or one
combined; a burst faster than any rung this station asked for feeds its SNR only) — and compares
each recommendation with the one the station's control frame carried, as the other station
decoded it. An exact match is what lets a candidate controller be replayed open-loop on the field
record; a difference is explained, not averaged.

Only the standard library, numpy and the reference model; run from the repository with
``uv run --project model``. A recording's sidecar is untrusted data: nothing in it is executed.
"""

from __future__ import annotations

import argparse
import copy
import csv
import dataclasses
import itertools
import json
import math
import os
import re
import statistics
import sys
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
TREE = (
    Path(os.environ["AETHER_MODEL_TREE"]).resolve() if os.environ.get("AETHER_MODEL_TREE") else ROOT
)
"""The repository whose reference model reads the session: this one, or a worktree of the build
the session ran (``--tree``; ``git worktree add <dir> v0.2.0-beta.N``), whose rate controller,
thresholds and ladder are the session's own — what a replay has to be checked against."""
sys.path.insert(0, str(TREE / "model"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import aether_model  # noqa: E402, F401  — the tree's package, claimed before two_sides asks
import two_sides  # noqa: E402
from aether_model.frame.modes import NARROW, WIDE, AirInterface  # noqa: E402
from aether_model.link.frames import SEQ_MOD, WINDOW, data_capacity, seq_distance  # noqa: E402
from aether_model.link.granted import (  # noqa: E402
    RV_SEQUENCE,
    SELF_DECODABLE_RVS,
    GrantConfig,
    GrantedEngine,
)
from aether_model.link.harness import phy_timing  # noqa: E402
from aether_model.link.rate import RateController  # noqa: E402
from aether_model.phy.pipeline import MODE_RETRY_CONFIDENCE  # noqa: E402
from aether_model.waveform import NARROW_500, WIDE_2300  # noqa: E402

try:
    from aether_model.link.granted import PhyTiming
except ImportError:  # a tree before Step 5 (2026-10-20) kept it with protocol 7's engine
    from aether_model.link.engine import PhyTiming
try:
    from aether_model.link.frames import REPORT
except ImportError:  # before protocol 9 (ADR-0077) bit 0 of a plain acknowledgement is never set
    REPORT = 0x0001

MATCH_S = 0.15
"""How far a receiver's record may lie from where the sender's frame was on the air and still be
its: the engine's own grid tolerance (``granted.GRID_TOLERANCE_S``)."""
DETECT_CONFIDENCE_TRUSTED = 1.3
"""The daemon's gate on an acquisition's confidence (``station::DETECT_CONFIDENCE_TRUSTED``)."""
PAIR_WINDOW_S = 180.0
"""How far apart two recordings of one session may have started: the slave's begins at the call,
the master's a probe and a call earlier."""
EXCHANGE_MAX_S = 60.0
"""An exchange longer than this is a silence (a step of the Test, a stall), not air the rung
spent: a floor exchange of six frames is about 40 s."""
FRAMES_PER_EXCHANGE = 6
"""What a counterfactual rung's rate is reckoned over: a full transmission."""
CONTROL = re.compile(
    r"(?P<kind>\w+) session (?P<session>\d+) count (?P<count>\d+)(?P<data_floor> on the floor)? "
    r"level (?P<level>\d+) base (?P<base>\d+) bitmap 0x(?P<bitmap>[0-9a-fA-F]+) rung (?P<rung>\d+) "
    r"n (?P<n>\d+) snr (?P<snr>-?\d+|-)(?P<flags>.*)"
)
"""A decoded control frame as the daemon spells it in a sidecar (``record::describe_control``)."""


# ── the sidecars ──────────────────────────────────────────────────────


def narrow_before_93() -> AirInterface:
    """The 500 Hz ladder a ``/4`` sidecar numbers by: QPSK ⅓ on the ordinary frame at rung 4,
    where tone4x100-93 is now (ADR-0081) — the tree's own ladder when it still has it there."""
    if NARROW.ladder[4].tone is None:
        return NARROW
    return dataclasses.replace(
        NARROW, ofdm_ladder=tuple(range(2, len(NARROW.modes))), tone_data=NARROW.tone_data[:4]
    )


def air_for(doc: dict[str, Any]) -> AirInterface:
    """The ladder a sidecar's mode numbers index: its air's, as its format numbered it."""
    if (doc.get("session") or {}).get("bandwidth_hz") != 500:
        return WIDE
    return narrow_before_93() if doc.get("format") == "aether-hf-session/4" else NARROW


def load_side(path: Path) -> two_sides.Side:
    """A sidecar and, when it is beside it, its audio (``two_sides.load`` needs the audio; this
    reads the sidecar alone when there is none, since nothing here listens to the band)."""
    if path.with_suffix(".wav").exists():
        side = two_sides.load(path)
        side.air = air_for(side.doc)
        return side
    doc = json.loads(path.read_text(encoding="utf-8"))
    name = str((doc.get("session") or {}).get("callsign") or path.stem)
    return two_sides.Side(name, doc, np.zeros(0), 48_000, air_for(doc))


def started_at(doc: dict[str, Any]) -> float:
    stamp = str(doc.get("started") or "1970-01-01T00:00:00Z").replace("Z", "+00:00")
    return datetime.fromisoformat(stamp).timestamp()


def remote_of(doc: dict[str, Any]) -> str:
    session = doc.get("session") or {}
    test = session.get("test") or {}
    return str(test.get("remote") or session.get("remote") or "").upper()


def role_of(doc: dict[str, Any]) -> str:
    return next(
        (str(e.get("detail")) for e in doc.get("events") or [] if e.get("event") == "role"), ""
    )


def pairs(sidecars: list[Path]) -> list[tuple[Path, Path]]:
    """Both sides of one session among ``sidecars``: two recordings whose stations name each
    other, started within :data:`PAIR_WINDOW_S`, the master's first."""
    docs = []
    for path in sidecars:
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if not isinstance(doc, dict) or not str(doc.get("format", "")).startswith(
            "aether-hf-session/"
        ):
            continue
        call = str((doc.get("session") or {}).get("callsign") or "").upper()
        docs.append((path, doc, call, remote_of(doc), started_at(doc)))
    out: list[tuple[Path, Path]] = []
    used: set[Path] = set()
    for a, da, ca, ra, ta in docs:
        if a in used:
            continue
        for b, db, cb, rb, tb in docs:
            if b in used or b == a or cb != ra or ca != rb or abs(ta - tb) > PAIR_WINDOW_S:
                continue
            master, slave = (a, b) if role_of(da) == "master" or role_of(db) == "slave" else (b, a)
            out.append((master, slave))
            used.update((a, b))
            break
    return out


def _event_t(doc: dict[str, Any], name: str) -> float | None:
    return next((float(e["t_s"]) for e in doc.get("events") or [] if e.get("event") == name), None)


def session_span(side: two_sides.Side) -> tuple[float, float]:
    """On the station's own clock: from its role in the session to its end (the identifiers and
    anything after are outside it; a call, an acceptance or a probe before it too)."""
    began = _event_t(side.doc, "role")
    ended = _event_t(side.doc, "disconnected")
    return (began if began is not None else 0.0), (ended if ended is not None else float("inf"))


# ── one clock ─────────────────────────────────────────────────────────


GUESS_DISAGREES_S = 1.0
"""How far the start stamps' offset may lie from the one the frames heard imply before the
frames' is taken: the field's stamps agree with it within a few tenths."""


def heard_offset(master: two_sides.Side, slave: two_sides.Side) -> float | None:
    """The slave's recording clock to the master's from the master's control frames the slave
    decoded against the master's keyings: the offset most of them agree on, within a tenth of a
    second (a transmission's first frame is its control frame, on the air a keying lead after the
    key). None when the slave decoded none."""
    starts = [
        float(f["start_s"])
        for f in slave.doc.get("frames") or []
        if f.get("kind") == "control" and f.get("decoded") and f.get("start_s") is not None
    ]
    keys = [k.key for k in master.keyings]
    if not starts or not keys:
        return None
    diffs = [k - t for k in keys for t in starts if abs(k - t) < 120.0]
    if not diffs:
        return None
    votes: dict[int, int] = {}
    for d in diffs:
        votes[round(d * 10)] = votes.get(round(d * 10), 0) + 1
    best = max(votes, key=lambda b: (votes[b] + votes.get(b - 1, 0) + votes.get(b + 1, 0), -abs(b)))
    near = [d for d in diffs if abs(d - best / 10) <= 0.15]
    return statistics.median(near) + two_sides.KEY_LEAD_S


def place(master: two_sides.Side, slave: two_sides.Side) -> two_sides.Offset:
    """Both stations' keyings on the master's recording clock, with the offset the loop
    latencies imply at each keying's time (``two_sides.py --from-loops``); a transmission the
    other station recorded nothing of is placed where its key puts it. Returns the slave's
    recording clock to the master's."""
    for side in (master, slave):
        side.keyings = two_sides.keyings(side)
    guess = two_sides.started_offset(master, slave)
    heard = heard_offset(master, slave)
    if heard is not None and abs(heard - guess) > GUESS_DISAGREES_S:
        # the start stamps are wall-clock time, and the scenario harness runs faster than the
        # air: its recordings' stamps put the clocks seconds apart, and the loops then pair the
        # wrong keyings and agree on it
        guess = heard
    two_sides.place(master, slave, guess)
    series = two_sides.loop_offsets(master, slave, guess)
    offset: two_sides.Offset = guess
    if series:
        offset = two_sides.offset_along(series, statistics.median(p.offset for p in series))
        two_sides.place(master, slave, offset)
    for side in (master, slave):
        for k in side.keyings:
            if k.rf_start is None:
                to_master = two_sides.offset_of(offset, k.key) if side is slave else 0.0
                k.rf_start = (
                    k.key
                    + to_master
                    + two_sides.KEY_LEAD_S
                    + k.latency_s
                    + k.handover_s
                    + two_sides.RADIO_PATHS_S
                )
    return offset


# ── frames and transmissions ──────────────────────────────────────────


@dataclass
class Frame:
    """One frame the sender put on the air, and what the receiver kept of it."""

    kind: str
    rung: int
    rv: int
    payload: int
    floor: bool
    at: float
    """Where it began on the air, on the master's recording clock."""
    seconds: float
    heard: dict[str, Any] | None = None
    seq: int | None = None
    copy: int = 0
    """Which copy of its block this is, 1 first; 0 for a measurement frame."""
    measure: bool = False
    combined: bool = False
    """The receiver held an earlier copy to combine it with (``_RxRecord.combined``)."""
    sent_seq: int | None = None
    """The sequence number its sender recorded (beta.122 on), or a measurement frame's index."""
    sent_measure: bool = False
    """Its sender recorded it as a measurement frame (beta.122 on)."""

    @property
    def detected(self) -> bool:
        return self.heard is not None

    @property
    def decoded(self) -> bool:
        """Decoded on its own by the receiver's PHY (a copy combined later is not)."""
        return bool(self.heard and self.heard.get("decoded") and self.heard_rung == self.rung)

    @property
    def heard_rung(self) -> int | None:
        return None if self.heard is None else int(self.heard.get("mode") or 0)

    @property
    def heard_rv(self) -> int | None:
        return None if self.heard is None else int(self.heard.get("rv") or 0)

    @property
    def snr(self) -> float | None:
        return None if self.heard is None else float(self.heard.get("snr_3k_db", 0.0))

    @property
    def trusted(self) -> bool:
        """What the daemon's engine was told of it (``station::trusted_measurement``; a
        capture skip inside it makes it untrusted, ADR-0076)."""
        if self.heard is None or self.heard.get("capture_skip"):
            return False
        return (
            float(self.heard.get("confidence") or 0.0) >= MODE_RETRY_CONFIDENCE
            or float(self.heard.get("detect_confidence") or 0.0) >= DETECT_CONFIDENCE_TRUSTED
        )


@dataclass
class Ack:
    """A control frame of the receiver's that the sender decoded: its acknowledgement and its
    recommendation."""

    t: float
    """On the sender's own clock, when the sender reported it."""
    kind: str
    base: int
    bitmap: int
    rung: int
    snr: float | None
    measure: bool
    count: int
    n: int

    @property
    def reports(self) -> bool:
        return bool(self.bitmap & REPORT)

    def received(self, seq: int) -> bool:
        d = (seq - self.base) % SEQ_MOD
        if d >= WINDOW:
            return d >= SEQ_MOD // 2
        return not self.reports and bool((self.bitmap >> d) & 1)

    def measured(self, index: int) -> bool:
        return self.reports and 0 <= index < WINDOW - 1 and bool((self.bitmap >> (index + 1)) & 1)


def parse_control(text: str, t: float = 0.0) -> Ack | None:
    m = CONTROL.match(text)
    if m is None:
        return None
    snr = m["snr"]
    return Ack(
        t=t,
        kind=m["kind"],
        base=int(m["base"]),
        bitmap=int(m["bitmap"], 16),
        rung=int(m["rung"]),
        snr=None if snr == "-" else float(snr),
        measure=" measure" in m["flags"],
        count=int(m["count"]),
        n=int(m["n"]),
    )


@dataclass
class Transmission:
    index: int
    key: float
    """On the sender's own clock."""
    start: float
    """On the master's recording clock."""
    frames: list[Frame]
    exchange_s: float | None = None
    """From its start to the sender's next transmission: the whole exchange, when it was one."""
    ack: Ack | None = None
    """The receiver's control frame the sender decoded after it, before its next transmission."""
    measure: bool = False
    flagged: list[str] = field(default_factory=list)

    @property
    def header(self) -> Frame | None:
        return next((f for f in self.frames if f.kind == "control"), None)

    @property
    def header_read(self) -> bool | None:
        """Whether the receiver decoded this transmission's control frame (``None``: none)."""
        h = self.header
        return None if h is None else h.decoded

    @property
    def header_said(self) -> Ack | None:
        h = self.header
        if h is None or h.heard is None or not h.decoded:
            return None
        return parse_control(str(h.heard.get("control") or ""))

    @property
    def data(self) -> list[Frame]:
        return [f for f in self.frames if f.kind == "data"]

    @property
    def rung(self) -> int | None:
        """The rung most of its data went at; of two as common, the slower."""
        modes = [f.rung for f in self.data]
        return max(sorted(set(modes)), key=modes.count) if modes else None

    @property
    def end(self) -> float:
        return self.start + sum(f.seconds for f in self.frames)

    @property
    def frame_air_s(self) -> float:
        return sum(f.seconds for f in self.data)

    @property
    def heard_any(self) -> bool:
        return any(f.decoded for f in self.frames)


def transmissions(
    sender: two_sides.Side, receiver: two_sides.Side, rx_offset: two_sides.Offset
) -> list[Transmission]:
    """The sender's transmissions inside the session, each frame placed on the air and matched
    to what the receiver recorded of it. The sender's keyings are on the master's clock already
    (:func:`place`); ``rx_offset`` takes the receiver's frames there (0 for the master's)."""
    began, ended = session_span(sender)
    heard = [
        (float(f["start_s"]) + two_sides.offset_of(rx_offset, float(f["start_s"])), f)
        for f in receiver.doc.get("frames") or []
        if f.get("start_s") is not None
    ]
    used: set[int] = set()
    out: list[Transmission] = []
    for k in sender.keyings:
        if not (began - 0.01 <= k.key < ended) or not k.records:
            continue
        # the slave's acceptance goes out as its role begins: a lone data frame, no header
        if (
            k.key <= began + 0.5
            and len(k.records) == 1
            and all(str(s.get("kind")) == "data" for s in k.records)
        ):
            continue
        assert k.rf_start is not None
        at = k.rf_start
        frames: list[Frame] = []
        for s in k.records:
            seconds = two_sides.frame_s(sender.air, s)
            kind = str(s.get("kind"))
            rung = int(s.get("mode") or 0)
            frame = Frame(
                kind=kind,
                rung=rung,
                rv=int(s.get("rv") or 0),
                payload=int(s.get("bytes") or 0),
                floor=bool(s.get("floor")) if kind == "control" else sender.air.is_floor(rung),
                at=at,
                seconds=seconds,
                sent_seq=None if s.get("seq") is None else int(s["seq"]),
                sent_measure=bool(s.get("measure")),
            )
            best = None
            for i, (start, rec) in enumerate(heard):
                if i in used or str(rec.get("kind")) != kind or abs(start - at) > MATCH_S:
                    continue
                score = (bool(rec.get("decoded")), -abs(start - at))
                if best is None or score > best[0]:
                    best = (score, i, rec)
            if best is not None:
                used.add(best[1])
                frame.heard = best[2]
            frames.append(frame)
            at += seconds
        out.append(Transmission(len(out), k.key, k.rf_start, frames))
    for tx, nxt in itertools.pairwise(out):
        gap = nxt.start - tx.start
        tx.exchange_s = gap if gap <= EXCHANGE_MAX_S else None
    return out


def acks_of(sender: two_sides.Side, kinds: tuple[str, ...]) -> list[Ack]:
    """The receiver's control frames of ``kinds`` the sender decoded, on the sender's clock."""
    out = []
    for f in sender.doc.get("frames") or []:
        if f.get("kind") != "control" or not f.get("decoded") or not f.get("control"):
            continue
        ack = parse_control(str(f["control"]), float(f["t_s"]))
        if ack is not None and ack.kind in kinds:
            out.append(ack)
    return out


def attach_acks(txs: list[Transmission], acks: list[Ack]) -> None:
    """Each transmission's acknowledgement: the first of the receiver's control frames the
    sender decoded after it and before its next transmission."""
    for tx, nxt in zip(txs, [*txs[1:], None], strict=False):
        limit = nxt.key if nxt is not None else float("inf")
        tx.ack = next((a for a in acks if tx.key < a.t <= limit), None)


# ── sequence numbers ──────────────────────────────────────────────────


@dataclass
class Block:
    """One sequence number's history: its copies, in order, and the copy that delivered it."""

    seq: int
    copies: list[Frame] = field(default_factory=list)
    delivered_by: Frame | None = None
    acked_after: int | None = None
    """The transmission index whose acknowledgement said it had arrived."""
    unknown: bool = False
    held: int = 0
    """Failed copies the receiver holds to combine with, since its buffer was last dropped."""
    tx_count: int = 0
    """Copies sent since the block was last encoded: what its next redundancy version follows
    (``_TxRecord.tx_count``, which a re-encode starts over)."""

    @property
    def mode(self) -> int:
        return self.copies[-1].rung

    @property
    def next_rv(self) -> int:
        return RV_SEQUENCE[self.tx_count % len(RV_SEQUENCE)]

    @property
    def delivered(self) -> bool:
        return self.delivered_by is not None

    @property
    def how(self) -> str:
        """``alone`` (its delivering copy decoded on its own), ``combined`` (it did not: the
        copies together did), or ``lost``."""
        if self.delivered_by is None:
            return "lost"
        heard = self.delivered_by.heard or {}
        if heard.get("rescued"):
            return "combined"
        return "alone" if self.delivered_by.decoded else "combined"


def numbered(txs: list[Transmission]) -> bool:
    """Whether the sender recorded every data frame's sequence number (beta.122 on)."""
    data = [f for tx in txs for f in tx.data]
    return bool(data) and all(f.sent_seq is not None for f in data)


def reconstruct(
    txs: list[Transmission],
    ladder: list[dict[str, Any]] | None = None,
    *,
    recorded: bool = True,
    receiver_records: bool = False,
) -> dict[int, Block]:
    """Every data frame's block. ``recorded`` and :func:`numbered`: the numbers its sender
    recorded; and with ``receiver_records``, whether copies combined into each is what the
    receiver's engine said (``combined``, ``rescued``), not inferred. Otherwise reconstructed."""
    if recorded and numbered(txs):
        return _from_record(txs, receiver_records)
    return _reconstructed(txs, ladder)


def _from_record(txs: list[Transmission], receiver_records: bool) -> dict[int, Block]:
    """The blocks from the sequence numbers the sender recorded: a number heard again before its
    block was acknowledged is a copy of it, one heard after is a new block."""
    blocks: dict[int, Block] = {}
    base = 0
    for tx in txs:
        data = tx.data
        if data and all(f.sent_measure for f in data):
            tx.measure = True
            for f in data:
                f.measure = True
                f.seq = f.sent_seq
            for b in blocks.values():
                b.held = 0  # the receiver drops what it holds to combine (ADR-0077)
        else:
            for f in data:
                assert f.sent_seq is not None
                block = blocks.get(f.sent_seq)
                if block is None or block.delivered:
                    block = Block(f.sent_seq)
                    blocks[f.sent_seq] = block
                elif f.rung != block.mode or f.rv != block.next_rv:
                    block.held = 0  # re-encoded, or in pieces: the receiver starts over
                    block.tx_count = 0
                if receiver_records and f.heard is not None:
                    f.combined = bool(f.heard.get("combined"))
                else:
                    f.combined = block.held > 0
                block.copies.append(f)
                block.tx_count += 1
                f.seq = block.seq
                f.copy = len(block.copies)
                if f.detected and not f.decoded:
                    block.held += 1
        if tx.ack is not None:
            tx.flagged.extend(_take_ack(blocks, tx.ack, tx.index))
            base = _advance(blocks, base)
    return blocks


def agreement(
    txs: list[Transmission], ladder: list[dict[str, Any]] | None = None
) -> tuple[int, int]:
    """How many data frames the reconstruction numbers as their sender recorded, of how many:
    the check of what the sessions before beta.122 were read with."""
    trial = copy.deepcopy(txs)
    for tx in trial:
        tx.flagged = []
        tx.measure = False
    _reconstructed(trial, ladder)
    pairs = [
        (f.sent_seq, g.seq)
        for tx, ty in zip(txs, trial, strict=True)
        for f, g in zip(tx.data, ty.data, strict=True)
    ]
    return sum(1 for a, b in pairs if a == b), len(pairs)


def _reconstructed(
    txs: list[Transmission], ladder: list[dict[str, Any]] | None = None
) -> dict[int, Block]:
    """Give every data frame its sequence number from the order the engine sends in: a
    transmission's unacknowledged frames first (the oldest first; those whose answer went unheard
    held back through one silent slot), then new ones; a copy's redundancy version follows
    ``RV_SEQUENCE`` and a frame re-encoded at another rung starts over at RV 0. Measurement
    frames (the Test's ladder) take none: the receiver said so in the header it decoded, or the
    master's ladder names a transmission of that rung and size whose answer reported, or went
    unheard."""
    blocks: dict[int, Block] = {}
    tx_next = 0
    base = 0
    silent = 0
    ladder_left = list(ladder or [])
    last_silent_measure: int | None = None
    for tx in txs:
        data = tx.data
        if not data:
            if tx.ack is not None:
                tx.flagged.extend(_take_ack(blocks, tx.ack, tx.index))
                base = _advance(blocks, base)
                silent = 0
            elif tx.frames:
                silent += 1
            continue
        said = tx.header_said
        measure = said is not None and said.measure and said.count > 0
        if not measure and ladder_left and said is None:
            entry = ladder_left[0]
            fits = int(entry.get("frames", 0)) == len(data) and all(
                f.rung == int(entry.get("mode", -1)) for f in data
            )
            measure = fits and (tx.ack is None or tx.ack.reports or tx.ack.measure)
        if measure:
            tx.measure = True
            for i, f in enumerate(data):
                f.measure = True
                f.seq = i
            for b in blocks.values():
                b.held = 0  # the receiver drops what it holds to combine (ADR-0077)
            # the master's ladder has one entry per measurement reported, and one for a rung
            # whose slot went silent twice (ADR-0092)
            rung = tx.rung
            if ladder_left and (
                (tx.ack is not None and tx.ack.reports)
                or (tx.ack is None and last_silent_measure == rung)
            ):
                ladder_left.pop(0)
            last_silent_measure = rung if tx.ack is None else None
        else:
            unacked = sorted(
                (s for s, b in blocks.items() if not b.delivered),
                key=lambda s: seq_distance(s, base),
            )
            if silent < 2:
                unacked = [s for s in unacked if not blocks[s].unknown]
            # one family a transmission: a block of the other family is deferred — unless it
            # is re-encoded into this one (the family is the first unacked block's, after its
            # re-encode), which `_consistent` reads as a lower rung at RV 0
            resent = _consistent(data, [blocks[s] for s in unacked])
            if resent is None:
                tx.flagged.append("sequence numbers could not be placed by the RVs")
                resent = min(len(unacked), len(data))
            for i, f in enumerate(data):
                if i < resent:
                    block = blocks[unacked[i]]
                    if f.rung != block.mode or f.rv != block.next_rv:
                        # re-encoded, or in pieces: the receiver starts over, and so do the
                        # copies' redundancy versions
                        block.held = 0
                        block.tx_count = 0
                    f.combined = block.held > 0
                    block.copies.append(f)
                    block.tx_count += 1
                    f.seq = block.seq
                    f.copy = len(block.copies)
                else:
                    if seq_distance(tx_next, base) >= WINDOW:
                        tx.flagged.append("more new frames than the window holds")
                    block = Block(tx_next, [f], tx_count=1)
                    blocks[tx_next] = block
                    f.seq = tx_next
                    f.copy = 1
                    tx_next = (tx_next + 1) % SEQ_MOD
                if f.detected and not f.decoded:
                    block.held += 1
        if tx.ack is None:
            silent += 1
            if not measure:
                for f in data:
                    assert f.seq is not None
                    blocks[f.seq].unknown = True
            continue
        silent = 0
        tx.flagged.extend(_take_ack(blocks, tx.ack, tx.index))
        base = _advance(blocks, base)
    return blocks


def _consistent(data: list[Frame], unacked: list[Block]) -> int | None:
    """How many of the transmission's data frames are the unacknowledged ones, resent in order:
    the most that fits every frame's rung and redundancy version."""
    for resent in range(min(len(unacked), len(data)), -1, -1):
        ok = True
        for i, f in enumerate(data):
            if i < resent:
                block = unacked[i]
                same = f.rung == block.mode and f.rv == block.next_rv
                reencoded = f.rung < block.mode and f.rv == 0
                ok = ok and (same or reencoded)
            else:
                ok = ok and f.rv == 0 and f.rung == data[resent].rung
        if ok:
            return resent
    return None


def _take_ack(blocks: dict[int, Block], ack: Ack, index: int) -> list[str]:
    """Apply an acknowledgement; returns what it contradicts: a block whose copy the receiver
    decoded on its own in the transmission acknowledged, and which it does not acknowledge —
    the numbering has drifted, or the receiver dropped the frame (another session, a
    measurement it took for data)."""
    doubts = []
    for b in blocks.values():
        b.unknown = False
        if b.delivered or not b.copies:
            continue
        if ack.received(b.seq):
            b.delivered_by = b.copies[-1]
            b.acked_after = index
        elif b.copies[-1].decoded and not b.copies[-1].measure and not ack.reports:
            doubts.append(f"block {b.seq} decoded and not acknowledged")
    return doubts


def _advance(blocks: dict[int, Block], base: int) -> int:
    while base in blocks and blocks[base].delivered:
        base = (base + 1) % SEQ_MOD
    return base


# ── the account ───────────────────────────────────────────────────────


@dataclass
class RungAccount:
    rung: int
    name: str
    sends: int = 0
    first_copies: int = 0
    decoded_first: int = 0
    never_detected: int = 0
    failed: int = 0
    decoded_alone: int = 0
    delivered_alone_first: int = 0
    delivered_alone_later: int = 0
    delivered_combined: int = 0
    lost: int = 0
    """Blocks whose last copy went at this rung and were never acknowledged."""
    bytes_delivered: int = 0
    frame_air_s: float = 0.0
    exchange_air_s: float = 0.0
    exchanges: int = 0
    measured_sent: int = 0
    measured_detected: int = 0
    measured_decoded: int = 0
    measured_reported: int | None = None
    measured_snr_db: float | None = None
    snrs: list[float] = field(default_factory=list)

    @property
    def delivered(self) -> int:
        return self.delivered_alone_first + self.delivered_alone_later + self.delivered_combined

    @property
    def first_copy_share(self) -> float | None:
        """Of the first copies and measurement frames sent at this rung, the share that decoded
        on their own — a frame never detected counted as lost."""
        n = self.first_copies + self.measured_sent
        if n == 0:
            return None
        return (self.decoded_first + self.measured_decoded) / n

    @property
    def frame_bps(self) -> float | None:
        return 8.0 * self.bytes_delivered / self.frame_air_s if self.frame_air_s > 0 else None

    @property
    def exchange_bps(self) -> float | None:
        if self.exchange_air_s <= 0:
            return None
        return 8.0 * self.bytes_delivered / self.exchange_air_s

    @property
    def snr_db(self) -> float | None:
        return statistics.median(self.snrs) if self.snrs else None


def account(
    air: AirInterface, txs: list[Transmission], blocks: dict[int, Block]
) -> dict[int, RungAccount]:
    """Every rung's account from the placed, numbered frames."""
    out: dict[int, RungAccount] = {}

    def at(r: int) -> RungAccount:
        if r not in out:
            out[r] = RungAccount(r, air.ladder[r].name if r < air.n_rungs else f"rung {r}")
        return out[r]

    for tx in txs:
        for f in tx.data:
            acc = at(f.rung)
            if f.measure:
                acc.measured_sent += 1
                acc.measured_detected += int(f.detected)
                acc.measured_decoded += int(f.decoded)
            else:
                acc.sends += 1
                acc.first_copies += int(f.copy == 1)
                acc.decoded_first += int(f.copy == 1 and f.decoded)
                acc.frame_air_s += f.seconds
            acc.never_detected += int(not f.detected)
            acc.failed += int(f.detected and not f.decoded)
            acc.decoded_alone += int(f.decoded)
            if f.decoded and f.snr is not None:
                acc.snrs.append(f.snr)
        rung = tx.rung
        if rung is not None and not tx.measure and tx.exchange_s is not None:
            acc = at(rung)
            acc.exchange_air_s += tx.exchange_s
            acc.exchanges += 1
    for b in blocks.values():
        if not b.copies:
            continue
        acc = at(b.mode)
        if b.delivered_by is None:
            acc.lost += 1
            continue
        acc.bytes_delivered += data_capacity(b.delivered_by.payload)
        if not b.delivered_by.decoded:
            acc.delivered_combined += 1
        elif b.delivered_by.copy == 1:
            acc.delivered_alone_first += 1
        else:
            acc.delivered_alone_later += 1
    return out


def ladder_report(master: two_sides.Side, accounts: dict[int, RungAccount]) -> None:
    """The master's record of each measurement beside what the slave's receiver made of it."""
    test = (master.doc.get("session") or {}).get("test") or {}
    for entry in test.get("ladder") or []:
        rung = int(entry.get("mode", -1))
        if rung in accounts:
            acc = accounts[rung]
            acc.measured_reported = (acc.measured_reported or 0) + int(entry.get("decoded", 0))
            snr = entry.get("snr_db")
            acc.measured_snr_db = float(snr) if snr is not None else acc.measured_snr_db


# ── neighbours and regret ─────────────────────────────────────────────


def timing_of(air: AirInterface) -> PhyTiming:
    """The air's timing as the model's engine takes it."""
    return phy_timing(NARROW_500 if air.params.bandwidth is NARROW.params.bandwidth else WIDE_2300)


def controller_for(
    air: AirInterface, rate: dict[str, Any] | None = None, controller: str = "current"
) -> RateController:
    """The controller the engine builds for an air: the engine's own, so that whatever the
    tree's engine hands it (the thresholds, the usable rungs, the floor, its echo limits, a cap
    it once had) is what the replay runs — ``rate`` the daemon's ``[link] rate`` overrides,
    none by default; ``controller`` a prototype of the rate controller's review (Gate 3), which
    only this tree's engine has."""
    cfg = GrantConfig(rate=dict(rate or {}))
    if controller != "current":
        cfg.controller = controller  # type: ignore[attr-defined]
    engine = GrantedEngine("TOOL", timing_of(air), cfg)
    return engine.rate


def ladder_of(air: AirInterface) -> tuple[PhyTiming, list[int]]:
    """The air's timing, and its usable rungs as the engine's controller ranks them."""
    return timing_of(air), list(controller_for(air).modes)


def exchange_overhead_s(txs: list[Transmission]) -> float | None:
    """What an exchange cost beyond its data frames' air in this session: the median over the
    data transmissions another followed — the header, the gaps, the answer."""
    costs = [
        tx.exchange_s - tx.frame_air_s
        for tx in txs
        if tx.exchange_s is not None and tx.data and not tx.measure
    ]
    return statistics.median(costs) if costs else None


def rate_at(
    air: AirInterface, rung: int, share: float, overhead_s: float, frames: int = FRAMES_PER_EXCHANGE
) -> float:
    """Data bit/s of exchange air a rung would deliver at ``share`` of its frames decoding on
    their own, in transmissions of ``frames`` frames with this session's overhead."""
    r = air.ladder[rung]
    return (
        8.0 * frames * data_capacity(r.payload_bytes) * share / (overhead_s + frames * r.duration_s)
    )


@dataclass
class Neighbour:
    rung: int
    name: str
    share: float
    measured_on: int
    bps: float


@dataclass
class Regret:
    """What the data at one rung delivered against what the best measured neighbour would have."""

    rung: int
    air_s: float
    actual_bps: float
    neighbours: list[Neighbour]
    best_all: Neighbour | None

    @property
    def best(self) -> Neighbour | None:
        return max(self.neighbours, key=lambda n: n.bps, default=None)

    @property
    def regret(self) -> float | None:
        """The best measured neighbour's delivery over this rung's, less one: +0.5 means it would
        have delivered half as much again in the same air; negative, the rung used was better."""
        if self.best is None or self.actual_bps <= 0:
            return None
        return self.best.bps / self.actual_bps - 1.0


def regrets(
    air: AirInterface, accounts: dict[int, RungAccount], txs: list[Transmission]
) -> list[Regret]:
    overhead = exchange_overhead_s(txs)
    if overhead is None:
        return []
    _, ladder = ladder_of(air)
    measured = {
        r: acc.first_copy_share
        for r, acc in accounts.items()
        if acc.first_copy_share is not None and r < air.n_rungs
    }

    def neighbour(r: int) -> Neighbour | None:
        share = measured.get(r)
        if share is None:
            return None
        acc = accounts[r]
        return Neighbour(
            r,
            air.ladder[r].name,
            share,
            acc.first_copies + acc.measured_sent,
            rate_at(air, r, share, overhead),
        )

    out = []
    for r, acc in sorted(accounts.items()):
        if acc.exchange_air_s <= 0 or acc.exchange_bps is None:
            continue
        if r in ladder:
            i = ladder.index(r)
            around = [ladder[j] for j in (i - 1, i + 1) if 0 <= j < len(ladder)]
        else:
            around = [x for x in (r - 1, r + 1) if 0 <= x < air.n_rungs]
        neighbours = [n for n in (neighbour(x) for x in around) if n is not None]
        best_all = max(
            (n for n in (neighbour(x) for x in measured if x != r) if n is not None),
            key=lambda n: n.bps,
            default=None,
        )
        out.append(Regret(r, acc.exchange_air_s, acc.exchange_bps, neighbours, best_all))
    return out


# ── the controller, replayed ──────────────────────────────────────────


@dataclass
class Step:
    """One recommendation the replayed controller made, beside the one the station sent."""

    t: float
    rung_sent: int | None
    rung_replayed: int
    smoothed_db: float | None
    margin_db: float
    ok: int
    failed: int
    snr_db: float | None
    mode: int | None
    recorded: tuple[float | None, float] | None = None
    """The station's own controller's smoothed SNR and margin as it decided the control frame
    (its sidecar's ``rate`` event, beta.122 on): None when it recorded none."""

    @property
    def state_agrees(self) -> bool | None:
        """Whether the replayed controller's state is the recorded one, to a hundredth of a
        decibel; None when nothing was recorded."""
        if self.recorded is None:
            return None
        snr, margin = self.recorded
        same_snr = (snr is None and self.smoothed_db is None) or (
            snr is not None and self.smoothed_db is not None and abs(snr - self.smoothed_db) < 0.01
        )
        return same_snr and abs(margin - self.margin_db) < 0.01


RATE_EVENT = re.compile(r"snr (\S+) margin (\S+)")


def rate_events(side: two_sides.Side) -> list[tuple[float, float | None, float]]:
    """(t on the station's own clock, smoothed SNR, margin) at each control frame it decided."""
    out = []
    for e in side.doc.get("events") or []:
        m = RATE_EVENT.match(str(e.get("detail") or "")) if e.get("event") == "rate" else None
        if m:
            out.append((float(e["t_s"]), None if m[1] == "-" else float(m[1]), float(m[2])))
    return out


def recorded_in(
    events: list[tuple[float, float | None, float]], since: float, until: float
) -> tuple[float | None, float] | None:
    """The first recorded state inside [since, until), on the station's own clock."""
    return next(((snr, margin) for t, snr, margin in events if since <= t < until), None)


@dataclass
class Observation:
    ok: int
    failed: int
    snr: float | None
    mode: int | None
    """The rung most of the transmission's frames were read at, the slower of two as common."""
    min_mode: int | None = None
    """The slowest rung read in it: what the sender's-choice rule compares with what this
    station asked for (``granted._observe``)."""


def observation(frames: list[Frame], blocks: dict[int, Block]) -> Observation:
    """What a transmission's data said to the controller, as ``granted._observe`` counts it:
    a frame neither decoded nor trusted is nothing; one decoded (alone, or by combining: it was
    delivered by this copy) is ``ok``; one trusted, at a self-decodable RV or combined, a
    failure; the SNR is the mean over the rest; the mode the most common rung read, the slower
    of two as common."""
    ok = failed = 0
    snrs: list[float] = []
    modes: list[int] = []
    for f in frames:
        if not f.detected:
            continue
        heard = f.heard or {}
        delivered_here = bool(heard.get("rescued")) or (
            f.seq is not None
            and not f.measure
            and blocks[f.seq].delivered_by is f
            and not f.decoded
        )
        if not (f.decoded or delivered_here or f.trusted):
            continue  # untrusted and unread: the engine takes nothing of it, its rung neither
        if f.decoded or delivered_here:
            ok += 1
        elif f.heard_rv in SELF_DECODABLE_RVS or f.combined:
            # the redundancy version as the receiver read it off the chips, as the rung: a
            # misread copy is judged as the engine judged it
            failed += 1
        if f.snr is not None:
            snrs.append(f.snr)
        # the rung as the receiver read it off the chips: a misread copy counts at the rung read
        if f.heard_rung is not None:
            modes.append(f.heard_rung)
    mode = max(sorted(set(modes)), key=modes.count) if modes else None
    return Observation(
        ok, failed, sum(snrs) / len(snrs) if snrs else None, mode, min(modes) if modes else None
    )


def outcomes_of(
    frames: list[Frame], blocks: dict[int, Block], air: AirInterface, header_read: bool | None
) -> list[Any]:
    """What a transmission's data frames came to, as the engine hands a prototype of the
    review (``granted._outcomes``): every frame decoded or trusted, at the rung read off its
    chips, and — when the station read the header, or decoded a frame of the transmission and
    could believe its count — every frame sent and never detected, at its rung."""
    from aether_model.link.rate_review import FrameOutcome

    timing = timing_of(air)
    out = []
    counted = header_read is True or any(f.decoded for f in frames)
    for f in frames:
        capacity = data_capacity(timing.capacity(f.rung))
        if not f.detected:
            if counted:
                out.append(FrameOutcome(f.rung, f.seconds, capacity, False, False, False, None))
            continue
        heard = f.heard or {}
        delivered_here = bool(heard.get("rescued")) or (
            f.seq is not None
            and not f.measure
            and blocks[f.seq].delivered_by is f
            and not f.decoded
        )
        if not (f.decoded or delivered_here or f.trusted):
            continue
        rung = f.heard_rung if f.heard_rung is not None else f.rung
        out.append(
            FrameOutcome(
                rung,
                f.seconds,
                data_capacity(timing.capacity(rung)),
                True,
                f.decoded or delivered_here,
                f.combined,
                f.snr,
                f.measure,
            )
        )
    return out


def _seed(ctl: RateController, receiver: two_sides.Side) -> None:
    """The connect frame the station seeded from: the last data frame it decoded before its
    role began (a slave's is the call, the master's the acceptance), read for its echo first."""
    began, _ = session_span(receiver)
    opening = [
        f
        for f in receiver.doc.get("frames") or []
        if f.get("kind") == "data" and f.get("decoded") and float(f.get("start_s") or 0.0) <= began
    ]
    if opening:
        f = opening[-1]
        _echo(ctl, f.get("echo"), float(f["snr_3k_db"]))
        ctl.seed(float(f["snr_3k_db"]), lower_bound=receiver.air.is_floor(int(f.get("mode") or 0)))


def _echoes(ctl: RateController, tx: Transmission) -> None:
    for f in tx.frames:
        if f.decoded and f.heard is not None:
            _echo(ctl, f.heard.get("echo"), float(f.heard["snr_3k_db"]))


def _echo(ctl: RateController, echo: object, snr_db: float) -> None:
    """A decoded frame's echo reading to the controller (ADR-0087): a build before it has no
    such reading, and its controller no such input."""
    observe = getattr(ctl, "observe_echo", None)
    if observe is not None:
        observe(echo if isinstance(echo, int | float) else None, snr_db)


def _feed(ctl: RateController, obs: Observation, asked: int | None) -> None:
    if obs.mode is None or (obs.ok == 0 and obs.failed == 0 and obs.snr is None):
        return
    if obs.failed and asked is not None and obs.min_mode is not None and obs.min_mode > asked:
        ctl.observe_snr(obs.snr)  # the sender's choice, not news of the path (ADR-0020)
    else:
        ctl.observe(obs.snr, obs.ok, obs.failed, obs.mode)


def stray_records(
    receiver: two_sides.Side, txs: list[Transmission], rx_offset: two_sides.Offset
) -> list[Frame]:
    """The receiver's data records matched to no frame the sender put on the air — false
    detections, and frames placed too far from where they were sent — as frames of their own,
    on the master's clock."""
    matched = {id(f.heard) for tx in txs for f in tx.frames if f.heard is not None}
    # nothing heard before the session is of it: the call, a probe, another station's frames
    began, _ = session_span(receiver)
    out = []
    for rec in receiver.doc.get("frames") or []:
        if rec.get("kind") != "data" or id(rec) in matched or rec.get("start_s") is None:
            continue
        start = float(rec["start_s"])
        if start < began:
            continue
        at = start + two_sides.offset_of(rx_offset, start)
        rung = int(rec.get("mode") or 0)
        out.append(
            Frame(
                "data",
                rung,
                int(rec.get("rv") or 0),
                0,
                receiver.air.is_floor(rung),
                at,
                float(rec.get("end_s") or start) - start,
                heard=rec,
            )
        )
    return sorted(out, key=lambda f: f.at)


def strays_before(strays: list[Frame], txs: list[Transmission], i: int) -> list[Frame]:
    """Of the strays, those the engine would have taken as the ``i``-th transmission's: found
    after the transmission before ended and before the first frame it decoded of this one."""
    tx = txs[i]
    since = txs[i - 1].end if i > 0 else -math.inf
    first = next((f.at for f in tx.data if f.decoded), tx.end)
    return [s for s in strays if since < s.at < first]


def replay_slave(
    slave: two_sides.Side,
    master_txs: list[Transmission],
    blocks: dict[int, Block],
    rx_offset: two_sides.Offset = 0.0,
    controller: str = "current",
) -> list[Step]:
    """The current controller on what the slave recorded of the master's transmissions, against
    the recommendation each of its answers carried as the master decoded it. The slave observes
    a transmission when it answers it (or forfeits it): when it read the header or decoded a
    data frame of it; the frames of one it could not place are nothing to it."""
    ctl = controller_for(slave.air, controller=controller)
    _seed(ctl, slave)
    asked: int | None = None
    steps = []
    strays = stray_records(slave, master_txs, rx_offset)
    states = rate_events(slave)
    _, slave_end = session_span(slave)  # after it, a fresh controller decides the closing

    def own(t: float) -> float:
        return t - two_sides.offset_of(rx_offset, t)

    for i, tx in enumerate(master_txs):
        _echoes(ctl, tx)
        if not (tx.header_read is True or any(f.decoded for f in tx.data)):
            continue
        frames = list(tx.data)
        if tx.header_read is not True:
            # without the header, the engine takes any data frame found since its last answer
            # and before the first it decoded of this transmission as the transmission's
            # (``granted._on_grid`` with no reference yet): a phantom there is counted
            frames = [*strays_before(strays, master_txs, i), *frames]
        obs = observation(frames, blocks)
        if frames:
            if hasattr(ctl, "observe_frames"):
                ctl.observe_frames(outcomes_of(frames, blocks, slave.air, tx.header_read), obs.snr)
            else:
                _feed(ctl, obs, asked)
        rec = ctl.recommend()
        if tx.ack is not None:
            asked = rec if asked is None else max(asked, rec)
        nxt = master_txs[i + 1].start if i + 1 < len(master_txs) else math.inf
        steps.append(
            Step(
                tx.ack.t if tx.ack is not None else tx.key,
                None if tx.ack is None else tx.ack.rung,
                rec,
                ctl.snr_db,
                ctl.margin_db,
                obs.ok,
                obs.failed,
                obs.snr,
                obs.mode,
                recorded_in(states, own(tx.end) - 0.1, min(own(nxt), slave_end)),
            )
        )
    return steps


def replay_master(
    master: two_sides.Side,
    master_txs: list[Transmission],
    slave_txs: list[Transmission],
    blocks: dict[int, Block],
    controller: str = "current",
) -> list[Step]:
    """The current controller on what the master recorded of the slave's answers, against the
    recommendation each of its headers carried as the slave decoded it. The master observes the
    slave's data when it closes the slot; a slot heard silent twice in a row, with frames
    granted, counts as a failure (``granted._close_slot``). ``blocks`` are the slave's data's."""
    ctl = controller_for(master.air, controller=controller)
    _seed(ctl, master)
    asked: int | None = None
    silent = 0
    steps = []
    granted = False
    states = rate_events(master)
    _, master_end = session_span(master)
    previous_end = -math.inf
    for tx, nxt in zip(master_txs, [*master_txs[1:], None], strict=False):
        said = tx.header_said
        rec = ctl.recommend()
        if said is not None and said.kind == "Header":
            asked = rec if asked is None else max(asked, rec)
        # a DISC carries no recommendation: only a header is compared
        if tx.header is not None and (said is None or said.kind == "Header"):
            steps.append(
                Step(
                    tx.key,
                    None if said is None else said.rung,
                    rec,
                    ctl.snr_db,
                    ctl.margin_db,
                    0,
                    0,
                    None,
                    None,
                    recorded_in(states, previous_end, min(tx.key + 0.05, master_end)),
                )
            )
        previous_end = tx.end
        limit = nxt.start if nxt is not None else float("inf")
        in_slot = [s for s in slave_txs if tx.end <= s.start < limit]
        heard = tx.ack is not None or any(s.heard_any for s in in_slot)
        for s in in_slot:
            _echoes(ctl, s)
            if s.data and s.heard_any:
                obs = observation(s.data, blocks)
                if hasattr(ctl, "observe_frames"):
                    ctl.observe_frames(
                        outcomes_of(s.data, blocks, master.air, s.header_read), obs.snr
                    )
                else:
                    _feed(ctl, obs, asked)
        if heard:
            silent = 0
        else:
            silent += 1
            if granted and silent >= 2:
                ctl.observe(None, 0, 1)
        if tx.ack is not None:
            granted = tx.ack.n > 0
    return steps


# ── one session ───────────────────────────────────────────────────────


@dataclass
class Direction:
    sender: str
    receiver: str
    role: str
    """The sender's role."""
    txs: list[Transmission]
    blocks: dict[int, Block]
    accounts: dict[int, RungAccount]
    regrets: list[Regret]
    recommendations: list[tuple[float, int, float | None]]
    """(t on the sender's clock, rung, the receiver's reading) from the answers decoded."""
    steps: list[Step] = field(default_factory=list)
    numbering: str = "reconstructed"
    """``recorded`` when the sender's sidecar numbered its frames (beta.122 on)."""
    agrees: tuple[int, int] | None = None
    """When recorded: the data frames the reconstruction numbers alike, of all of them."""

    @property
    def answers_read(self) -> int:
        return sum(1 for tx in self.txs if tx.ack is not None)

    @property
    def data_transmissions(self) -> int:
        return sum(1 for tx in self.txs if tx.data and not tx.measure)

    @property
    def flagged(self) -> list[tuple[int, str]]:
        return [(tx.index, w) for tx in self.txs for w in tx.flagged]


@dataclass
class Reading:
    master: str
    slave: str
    recording: str
    bandwidth_hz: int
    started: str
    test: dict[str, Any]
    directions: list[Direction]
    versions: tuple[str, str] = ("", "")
    """The builds the master and the slave ran."""
    fmt: str = ""


def records_engine(side: two_sides.Side) -> bool:
    """Whether a station's sidecar records what its engine made of each data frame (beta.122
    on): any data frame there carries a sequence number."""
    return any(
        f.get("seq") is not None for f in side.doc.get("frames") or [] if f.get("kind") == "data"
    )


def read_session(
    master_path: Path,
    slave_path: Path,
    replay_controller: bool = False,
    controller: str = "current",
) -> Reading:
    master, slave = load_side(master_path), load_side(slave_path)
    if role_of(master.doc) == "slave" or role_of(slave.doc) == "master":
        master, slave = slave, master
    offset = place(master, slave)
    test = (master.doc.get("session") or {}).get("test") or {}
    ladder = test.get("ladder") or []
    down = transmissions(master, slave, offset)
    attach_acks(down, acks_of(master, ("Answer",)))
    down_agrees = agreement(down, ladder) if numbered(down) else None
    down_blocks = reconstruct(down, ladder, receiver_records=records_engine(slave))
    up = transmissions(slave, master, 0.0)
    attach_acks(up, acks_of(slave, ("Header",)))
    up_agrees = agreement(up) if numbered(up) else None
    up_blocks = reconstruct(up, receiver_records=records_engine(master))
    directions = []
    for sender, receiver, role, txs, blocks, agrees in (
        (master, slave, "master", down, down_blocks, down_agrees),
        (slave, master, "slave", up, up_blocks, up_agrees),
    ):
        accounts = account(sender.air, txs, blocks)
        if role == "master":
            ladder_report(master, accounts)
        recs = [(tx.ack.t, tx.ack.rung, tx.ack.snr) for tx in txs if tx.ack is not None]
        d = Direction(
            sender.name,
            receiver.name,
            role,
            txs,
            blocks,
            accounts,
            regrets(sender.air, accounts, txs),
            recs,
            numbering="recorded" if agrees is not None else "reconstructed",
            agrees=agrees,
        )
        if replay_controller:
            d.steps = (
                replay_slave(slave, down, down_blocks, offset, controller)
                if role == "master"
                else replay_master(master, down, up, up_blocks, controller)
            )
        directions.append(d)
    return Reading(
        master.name,
        slave.name,
        master_path.stem,
        500 if (master.doc.get("session") or {}).get("bandwidth_hz") == 500 else 2300,
        str(master.doc.get("started") or ""),
        test,
        directions,
        (str(master.doc.get("version") or "?"), str(slave.doc.get("version") or "?")),
        str(master.doc.get("format") or ""),
    )


# ── reports ───────────────────────────────────────────────────────────


def table(accounts: dict[int, RungAccount], markdown: bool) -> str:
    head = [
        "rung", "frame", "sends", "1st", "never", "failed", "alone", "d:1st", "d:later",
        "d:comb", "lost", "bytes", "frame s", "bit/s", "exch s", "bit/s", "SNR",
        "meas", "slave", "master",
    ]  # fmt: skip
    rows = []
    for r, a in sorted(accounts.items()):
        rows.append(
            [
                str(r),
                a.name,
                str(a.sends),
                str(a.first_copies),
                str(a.never_detected),
                str(a.failed),
                str(a.decoded_alone),
                str(a.delivered_alone_first),
                str(a.delivered_alone_later),
                str(a.delivered_combined),
                str(a.lost),
                str(a.bytes_delivered),
                f"{a.frame_air_s:.1f}",
                "" if a.frame_bps is None else f"{a.frame_bps:.0f}",
                f"{a.exchange_air_s:.1f}",
                "" if a.exchange_bps is None else f"{a.exchange_bps:.0f}",
                "" if a.snr_db is None else f"{a.snr_db:+.1f}",
                str(a.measured_sent) if a.measured_sent else "",
                f"{a.measured_decoded}/{a.measured_detected}" if a.measured_sent else "",
                ""
                if a.measured_reported is None
                else f"{a.measured_reported}"
                + ("" if a.measured_snr_db is None else f"@{a.measured_snr_db:.0f}"),
            ]
        )
    return _grid(head, rows, markdown)


def _grid(head: list[str], rows: list[list[str]], markdown: bool) -> str:
    if markdown:
        lines = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
        lines += ["| " + " | ".join(r) + " |" for r in rows]
        return "\n".join(lines)
    widths = [max(len(x) for x in col) for col in zip(head, *rows, strict=True)]
    return "\n".join(
        "  ".join(x.rjust(w) for x, w in zip(r, widths, strict=True)) for r in [head, *rows]
    )


def timeline(d: Direction) -> list[str]:
    """Every transmission of the sender's: when, what, what the receiver made of each frame
    (``✓`` decoded alone, ``✗`` detected, ``·`` never found), and the control frame that
    answered it."""
    out = []
    for tx in d.txs:
        frames = []
        for f in tx.frames:
            mark = "✓" if f.decoded else ("✗" if f.detected else "·")
            if f.kind == "control":
                frames.append(f"H{'f' if f.floor else ''}{mark}")
            else:
                who = "m" if f.measure else (f"#{f.seq}c{f.copy}" if f.seq is not None else "?")
                frames.append(f"r{f.rung}/rv{f.rv}{who}{mark}")
        ack = "-"
        if tx.ack is not None:
            ack = (
                f"{tx.ack.kind} base {tx.ack.base} bitmap {tx.ack.bitmap:#06x} rung {tx.ack.rung}"
                + (" report" if tx.ack.reports else "")
            )
        exch = "" if tx.exchange_s is None else f" exch {tx.exchange_s:.1f}s"
        out.append(
            f"    {tx.index:3d} key {tx.key:7.1f} air {tx.start:7.1f}{exch}"
            f"{' MEASURE' if tx.measure else ''}: {' '.join(frames)}  <- {ack}"
            + ("" if not tx.flagged else "  !! " + "; ".join(tx.flagged))
        )
    return out


def report(reading: Reading, markdown: bool = False, with_timeline: bool = False) -> str:
    out = []
    test = reading.test
    summary = ""
    if test:
        m, f = test.get("message") or {}, test.get("file") or {}
        summary = (
            f"; Test {test.get('outcome')}: message {m.get('bytes')} B at {m.get('bps')} bit/s, "
            f"file {f.get('bytes')} B at {f.get('bps')} bit/s"
        )
    out.append(
        f"{reading.recording}: {reading.master} (master) -> {reading.slave}, "
        f"{reading.bandwidth_hz} Hz, {reading.started}{summary}"
    )
    out.append(
        f"  builds {reading.versions[0]} / {reading.versions[1]}, sidecars {reading.fmt}; read with "
        f"the model at {TREE}"
    )
    for d in reading.directions:
        if not d.txs:
            continue
        out.append("")
        out.append(
            f"{d.sender}'s data, read at {d.receiver}: {d.data_transmissions} data transmissions, "
            f"{sum(1 for tx in d.txs if tx.measure)} measurements, {d.answers_read} of "
            f"{len(d.txs)} transmissions answered with a control frame {d.sender} read"
        )
        out.append(
            "  (sends: every copy; 1st: first copies; never: never detected; failed: detected,"
            " not decoded alone; alone: decoded on their own; d: blocks delivered by a first"
            " copy, a later copy, or combining; bytes and bit/s over the frames' air and over"
            " the whole exchanges; SNR: the median read; meas: the ladder's frames, what the"
            " slave decoded of what it detected, and what the master recorded, with its SNR)"
        )
        out.append(table(d.accounts, markdown))
        if d.agrees is not None:
            out.append(
                f"  numbered as {d.sender} recorded; the reconstruction agrees on "
                f"{d.agrees[0]} of {d.agrees[1]} data frames"
            )
        if d.flagged:
            out.append("  flagged: " + "; ".join(f"transmission {i}: {w}" for i, w in d.flagged))
        if with_timeline:
            out.append("  transmissions:")
            out.extend(timeline(d))
        if d.recommendations:
            out.append(
                f"  {d.receiver} recommended ({d.sender}'s clock: rung @ {d.receiver}'s reading):"
            )
            out.append(
                "    "
                + "  ".join(
                    f"{t:.0f}s:{r}" + ("" if s is None else f"@{s:+.0f}")
                    for t, r, s in d.recommendations
                )
            )
        for g in d.regrets:
            best = g.best
            line = (
                f"  rung {g.rung} ({d.accounts[g.rung].name}): {g.air_s:.0f} s of exchanges "
                f"delivering {g.actual_bps:.0f} bit/s"
            )
            if g.neighbours:
                line += "; neighbours measured: " + ", ".join(
                    f"rung {n.rung} {n.share:.0%} of {n.measured_on} -> {n.bps:.0f} bit/s"
                    for n in g.neighbours
                )
            if best is not None and g.regret is not None:
                line += f"; the best neighbour would have delivered {g.regret:+.0%}"
            if g.best_all is not None and (best is None or g.best_all.rung != best.rung):
                line += f"; the best measured rung, {g.best_all.rung}: {g.best_all.bps:.0f} bit/s"
            out.append(line)
        if d.steps:
            compared = [s for s in d.steps if s.rung_sent is not None]
            agree = sum(1 for s in compared if s.rung_sent == s.rung_replayed)
            out.append(
                f"  the controller replayed at {d.receiver}: {agree} of {len(compared)} "
                f"recommendations as sent ({len(d.steps) - len(compared)} not read by {d.sender})"
            )
            judged = [s for s in d.steps if s.state_agrees is not None]
            if judged:
                same = sum(1 for s in judged if s.state_agrees)
                first = next((s for s in judged if not s.state_agrees), None)
                out.append(
                    f"  its state as {d.receiver} recorded it at {same} of {len(judged)} steps"
                    + ("" if first is None else f"; first apart at {first.t:.1f}s")
                )
            for s in d.steps:
                mark = (
                    ""
                    if s.rung_sent is None
                    else ("  ok" if s.rung_sent == s.rung_replayed else "  DIFFERS")
                )
                if s.recorded is not None and not s.state_agrees:
                    rs, rm = s.recorded
                    mark += f"  (recorded {'-' if rs is None else f'{rs:+.1f}'} margin {rm:.1f})"
                sent = "-" if s.rung_sent is None else str(s.rung_sent)
                snr = "-" if s.snr_db is None else f"{s.snr_db:+.1f}"
                smoothed = "-" if s.smoothed_db is None else f"{s.smoothed_db:+.1f}"
                out.append(
                    f"    {s.t:7.1f}s sent {sent:>2} replayed {s.rung_replayed:>2}  ok {s.ok} "
                    f"failed {s.failed} at {snr} dB on rung {'-' if s.mode is None else s.mode};"
                    f" smoothed {smoothed} margin {s.margin_db:.1f}{mark}"
                )
    return "\n".join(out)


def as_json(reading: Reading) -> dict[str, Any]:
    def rung_json(a: RungAccount) -> dict[str, Any]:
        base = {k: v for k, v in vars(a).items() if k != "snrs"}
        base.update(
            snr_db=a.snr_db,
            first_copy_share=a.first_copy_share,
            frame_bps=a.frame_bps,
            exchange_bps=a.exchange_bps,
            delivered=a.delivered,
        )
        return base

    return {
        "recording": reading.recording,
        "master": reading.master,
        "slave": reading.slave,
        "bandwidth_hz": reading.bandwidth_hz,
        "started": reading.started,
        "test": reading.test,
        "directions": [
            {
                "sender": d.sender,
                "receiver": d.receiver,
                "transmissions": len(d.txs),
                "answers_read": d.answers_read,
                "rungs": {str(r): rung_json(a) for r, a in sorted(d.accounts.items())},
                "recommendations": d.recommendations,
                "regrets": [
                    {
                        "rung": g.rung,
                        "air_s": g.air_s,
                        "actual_bps": g.actual_bps,
                        "neighbours": [vars(n) for n in g.neighbours],
                        "best_all": None if g.best_all is None else vars(g.best_all),
                        "regret": g.regret,
                    }
                    for g in d.regrets
                ],
                "replay": [vars(s) for s in d.steps],
                "flagged": [w for _, w in d.flagged],
                "numbering": d.numbering,
                "agrees": d.agrees,
            }
            for d in reading.directions
        ],
    }


CSV_COLUMNS = [
    "recording", "master", "slave", "bandwidth_hz", "sender", "rung", "name", "sends",
    "first_copies", "decoded_first", "never_detected", "failed", "decoded_alone",
    "delivered_alone_first", "delivered_alone_later", "delivered_combined", "lost",
    "bytes_delivered", "frame_air_s", "frame_bps", "exchange_air_s", "exchange_bps", "snr_db",
    "measured_sent", "measured_detected", "measured_decoded", "measured_reported",
    "measured_snr_db", "first_copy_share",
]  # fmt: skip


def csv_rows(reading: Reading) -> list[dict[str, Any]]:
    rows = []
    for d in reading.directions:
        for r, a in sorted(d.accounts.items()):
            row: dict[str, Any] = {
                "recording": reading.recording,
                "master": reading.master,
                "slave": reading.slave,
                "bandwidth_hz": reading.bandwidth_hz,
                "sender": d.sender,
                "rung": r,
            }
            for c in CSV_COLUMNS[6:]:
                v = getattr(a, c)
                row[c] = "" if v is None else (f"{v:.3f}" if isinstance(v, float) else v)
            rows.append(row)
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("sidecars", nargs="*", type=Path, help="the master's and the slave's")
    ap.add_argument("--inbox", type=Path, help="every pair of sidecars found under this directory")
    ap.add_argument(
        "--replay", action="store_true", help="run the current controller on the receiver's record"
    )
    ap.add_argument("--json", type=Path, help="write the reading(s) here")
    ap.add_argument("--csv", type=Path, help="append one row per rung and direction here")
    ap.add_argument("--markdown", action="store_true")
    ap.add_argument("--timeline", action="store_true", help="list every transmission")
    ap.add_argument(
        "--tree",
        type=Path,
        help="read with this checkout's model (a worktree of the build the session ran)",
    )
    args = ap.parse_args()
    if args.tree is not None and args.tree.resolve() != TREE:
        # the tree's model has to be imported before anything else: start again with it named
        argv = [a for a in sys.argv[1:] if a != "--tree" and Path(a) != args.tree]
        env = {**os.environ, "AETHER_MODEL_TREE": str(args.tree.resolve())}
        os.execve(sys.executable, [sys.executable, "-I", __file__, *argv], env)
    if args.inbox:
        found = pairs(sorted(args.inbox.rglob("*.json")))
        if not found:
            print(f"no two-sided session under {args.inbox}", file=sys.stderr)
            return 1
    elif len(args.sidecars) == 2:
        found = [(args.sidecars[0], args.sidecars[1])]
    else:
        ap.error("two sidecars, or --inbox DIR")
    readings = []
    for master, slave in found:
        reading = read_session(master, slave, replay_controller=args.replay)
        readings.append(reading)
        print(report(reading, args.markdown, args.timeline))
        print()
    if args.json:
        args.json.write_text(json.dumps([as_json(r) for r in readings], indent=1), encoding="utf-8")
    if args.csv:
        new = not args.csv.exists() or args.csv.stat().st_size == 0
        with args.csv.open("a", newline="", encoding="utf-8") as f:
            w = csv.DictWriter(f, fieldnames=CSV_COLUMNS)
            if new:
                w.writeheader()
            for r in readings:
                w.writerows(csv_rows(r))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
