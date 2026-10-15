"""What each collision on the scenario harness was: the two transmissions in it, and why each
was keyed (ADR-0069 §5).

    python tools/air_account.py runs/<scenario>/channel.json [more channel.json ...]

The channel server (``tools/channel_server.py``) reports every stretch two transmitters
radiated at once (``rf_overlaps``) and places on the air each transmission's account, as its
daemon gave it (``transmissions``: the engine's cause — the timer or call that keyed it —, its
role and state, the frames it carried, how long it was held). A radiated overlap is put down
to the transmission each station had on the air at its start, and named by the one keyed
*second* — the one that went on the air over the other:

* ``request`` — either is a turn request, keyed by the receiving station on quiet (ADR-0027);
* ``leaving`` — either is a DISC or its answer: an abort's, a leave repeat, a left session's
  answer (ADR-0022, ADR-0052), or the DISC_ACK to one;
* ``answer held`` — an answer queued before the other transmission began, and keyed late:
  the daemon held it (busy, the answer gap, the identifier) or its keying was slow;
* ``answer over a repeat`` — an answer decided while the other station's repeat — its timer
  having fired — was already on the air: a late answer meeting the sender's retry;
* ``answer over a burst`` — an answer decided while the burst it answers, or another, was
  still on the air: an early answer, or a late decode;
* ``repeat over an answer`` — a station's retry or poll keyed while the other's answer was on
  the air;
* ``both initiating`` — neither an answer: both stations started transmissions of their own;
* ``other`` — anything else (identifier, beacon, leave), named by the two causes.

The mechanism is a reading of the two accounts, not a proof: the pair of causes and frames is
printed with it.

    python tools/air_account.py --sidecars MASTER.json SLAVE.json

does the same for a session on the air (12-two-paths-80m.md), from the two stations' recordings
put on one clock by ``tools/two_sides.py`` (the loops' offset at each keying's time): every
change of hands with the clear air between the two radiations (an overlap is a negative gap),
the slave's forfeits and late drops (its ``dropped`` events and counters), and every master
transmission that went unanswered, named by what the slave had of it:

* ``answer unheard`` — the slave keyed in the slot and the master decoded nothing of it;
* ``no grant: header lost`` — the transmission had a header, the slave did not decode it, and
  no data frame it decoded placed the end (an exact count of 0–2, ADR-0068);
* ``no grant: header elided`` — no header (R7), and no data frame decoded with a count that
  places the end: the frames lost, or only ones with a count of three or more;
* ``no grant: nothing heard`` — the slave recorded nothing of it;
* ``forfeit`` — the slave decoded what granted the slot and keyed nothing: decoded too late,
  or dropped by its keying gate;
* ``outside the slave's recording`` — the slot falls before the slave's recording began (it
  starts at the call; a probe comes before) or after it stopped (the DISC_ACK).
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

ANSWER_KINDS = {"Ack", "ConnectAck", "ProbeAck", "DiscAck"}
LEAVING_KINDS = {"Disc", "DiscAck"}
REPEATS = {"timer:wait", "timer:resend", "timer:keepalive", "timer:connect", "timer:leave"}


def first_kind(tx: dict[str, Any]) -> str:
    frames = tx.get("frames") or []
    return str(frames[0].get("kind", "?")) if frames else str(tx.get("cause", "?"))


def is_answer(tx: dict[str, Any]) -> bool:
    return tx.get("cause") == "timer:ack" or first_kind(tx) in ANSWER_KINDS


def queued_at(tx: dict[str, Any], latency_s: float) -> float:
    """When the station decided the transmission, on the air's clock: its air start, less the
    card's latency and what it waited in the station's queue."""
    waited = tx.get("waited_s")
    return float(tx["air_s"]) - latency_s - (float(waited) if waited is not None else 0.0)


def on_air_at(transmissions: list[dict[str, Any]], t: float) -> dict[str, Any] | None:
    """The station's transmission that began last at or before ``t``."""
    best = None
    for tx in transmissions:
        if float(tx["air_s"]) <= t + 1e-6:
            best = tx
        else:
            break
    return best


def mechanism(first: dict[str, Any], second: dict[str, Any], latency_s: float) -> str:
    """Name a radiated overlap by the transmission keyed second, over the first."""
    if "timer:request" in (first.get("cause"), second.get("cause")):
        return "request"
    if LEAVING_KINDS & {first_kind(first), first_kind(second)}:
        return "leaving"
    if is_answer(second):
        if queued_at(second, latency_s) < float(first["air_s"]):
            return "answer held"
        if first.get("cause") in REPEATS:
            return "answer over a repeat"
        return "answer over a burst"
    if is_answer(first) and second.get("cause") in REPEATS:
        return "repeat over an answer"
    if not is_answer(first):
        return "both initiating"
    return "other"


def describe(tx: dict[str, Any] | None) -> str:
    if tx is None:
        return "?"
    kinds = "+".join(str(f.get("kind", "?")) for f in tx.get("frames") or [])
    return f"{tx.get('cause') or '?'}[{tx.get('role') or '-'}:{kinds or '-'}]"


def account(report: dict[str, Any]) -> list[dict[str, Any]]:
    """One line per radiated overlap: when, how long, the two transmissions and the name."""
    latency_s = float(report.get("latency_s", 0.25))
    stations = report.get("stations", {})
    names = list(stations)
    out = []
    for start, end in report.get("rf_overlaps", []):
        on = {
            name: on_air_at(stations[name].get("transmissions", []), float(start)) for name in names
        }
        present = [(n, tx) for n, tx in on.items() if tx is not None]
        present.sort(key=lambda item: float(item[1]["air_s"]))
        if len(present) < 2:
            out.append({"start": start, "end": end, "mechanism": "untagged"})
            continue
        (who1, first), (who2, second) = present[-2], present[-1]
        out.append(
            {
                "start": start,
                "end": end,
                "seconds": round(float(end) - float(start), 3),
                "first": f"{who1} {describe(first)}",
                "second": f"{who2} {describe(second)}",
                "mechanism": mechanism(first, second, latency_s),
            }
        )
    return out


SLOT_SEARCH_S = 4.0
"""How long after a master transmission's frames an answer is looked for: the slot's gap, the
largest gap level, and an answer's first frame."""


def field_account(master: Any, slave: Any, offset: Any) -> list[dict[str, Any]]:
    """One line per master transmission — answered or not, and why not — from two
    :class:`two_sides.Side` placed on the master's clock with ``offset`` (``two_sides.place``;
    the slave's recording clock to the master's, one number or one per moment)."""
    import two_sides

    slave_frames = [
        (float(f["start_s"]) + two_sides.offset_of(offset, float(f["start_s"])), f)
        for f in slave.doc.get("frames") or []
    ]

    out: list[dict[str, Any]] = []
    # a transmission the slave recorded nothing of is placed where its keying puts it
    placed = []
    for k in master.keyings:
        if k.rf_start is None:
            k.rf_start = (
                k.key + two_sides.KEY_LEAD_S + k.latency_s + k.handover_s + two_sides.RADIO_PATHS_S
            )
        placed.append(k)
    replies = [k for k in slave.keyings if k.rf_start is not None]
    heard_by_master = [f for f in master.doc.get("frames") or [] if f.get("decoded")]
    # what the slave's recording covers, on the master's clock: a recording starts at the call
    # (its preroll is audio only, no events) and stops with the session, before its DISC_ACK
    began = next(
        (
            float(e["t_s"])
            for e in slave.doc.get("events") or []
            if e.get("event") == "recording" and str(e.get("detail", "")).startswith("started")
        ),
        0.0,
    )
    ended = float((slave.doc.get("audio") or {}).get("seconds") or float("inf"))
    covered = (
        began + two_sides.offset_of(offset, began),
        ended + two_sides.offset_of(offset, ended),
    )
    for i, k in enumerate(placed):
        assert k.rf_start is not None and k.rf_end is not None
        nxt = placed[i + 1].rf_start if i + 1 < len(placed) else None
        limit = min(k.rf_end + SLOT_SEARCH_S, nxt if nxt is not None else float("inf"))
        reply = next((r for r in replies if k.rf_end < float(r.rf_start or 0.0) < limit), None)
        # what the slave recorded of this transmission, frame by frame
        got = []
        pos = k.rf_start
        for kind, _rung, seconds in k.sent:
            near = [f for at, f in slave_frames if abs(at - pos) < 0.08]
            got.append((kind, near))
            pos += seconds
        has_header = any(kind == "control" for kind, *_ in k.sent)
        header_read = any(
            kind == "control" and any(f.get("decoded") for f in near) for kind, near in got
        )
        placing = any(
            kind == "data"
            and any(
                f.get("decoded")
                and isinstance(f.get("follows"), int)
                and f["follows"] < two_sides.MOST_FOLLOWS
                for f in near
            )
            for kind, near in got
        )
        anything = any(near for _, near in got)
        if reply is None and not covered[0] <= k.rf_end + 1.0 <= covered[1] - SLOT_SEARCH_S:
            what = "outside the slave's recording"
        elif reply is not None:
            heard = any(
                abs(float(f["start_s"]) - float(reply.rf_start or 0.0)) < 0.1
                for f in heard_by_master
            )
            what = "answered" if heard else "answer unheard"
        elif header_read or placing:
            what = "forfeit"
        elif not anything:
            what = "no grant: nothing heard"
        elif has_header:
            what = "no grant: header lost"
        else:
            what = "no grant: header elided"
        out.append(
            {
                "t": round(k.rf_start, 2),
                "frames": "+".join(
                    f"{'H' if kind == 'control' else 'r' + str(rung)}" for kind, rung, _ in k.sent
                ),
                "read": "".join(
                    "·" if not near else ("✓" if any(f.get("decoded") for f in near) else "✗")
                    for _, near in got
                ),
                "what": what,
            }
        )
    return out


def field_main(master_path: Path, slave_path: Path) -> int:
    import statistics

    import two_sides

    a, b = two_sides.load(master_path), two_sides.load(slave_path)
    if two_sides.role(a) == "slave" or two_sides.role(b) == "master":
        a, b = b, a
    for side in (a, b):
        side.keyings = two_sides.keyings(side)
    guess = two_sides.started_offset(a, b)
    two_sides.place(a, b, guess)
    series = two_sides.loop_offsets(a, b, guess)
    offset: two_sides.Offset = guess
    if series:
        offset = two_sides.offset_along(series, statistics.median(p.offset for p in series))
    two_sides.place(a, b, offset)
    changes = two_sides.gaps(a, b)
    overlaps = [(x, y, g) for x, y, g in changes if g < 0]
    print(
        f"{a.name} (master) and {b.name}: {len(changes)} changes of hands, the least clear air "
        f"{min((g for *_, g in changes), default=float('nan')):.3f} s; radiated overlaps "
        f"{len(overlaps)}"
    )
    for x, y, g in overlaps:
        print(f"  OVERLAP {x.who} -> {y.who} at {y.rf_start:.2f}: {g:.3f} s")
    for side in (a, b):
        counters = side.doc.get("counters") or {}
        dropped = [e for e in side.doc.get("events") or [] if e.get("event") == "dropped"]
        print(
            f"{side.name}: forfeits {counters.get('forfeits', 0)}, dropped late "
            f"{counters.get('dropped_late', 0)} (counters since the daemon started); "
            f"{len(dropped)} late drops in this recording"
        )
    lines = field_account(a, b, offset)
    tally = Counter(line["what"] for line in lines)
    print("master transmissions: " + "; ".join(f"{k} {v}" for k, v in tally.most_common()))
    for line in lines:
        if line["what"] != "answered":
            print(f"  {line['t']:8.2f}  {line['frames']:<28} {line['read']:<8} {line['what']}")
    return 0


def summary(lines: list[dict[str, Any]]) -> str:
    """``mechanism n`` pairs, most frequent first: what a results row carries."""
    counts = Counter(line["mechanism"] for line in lines)
    return "; ".join(f"{name} {n}" for name, n in counts.most_common())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("reports", nargs="*", type=Path)
    ap.add_argument(
        "--sidecars",
        nargs=2,
        type=Path,
        metavar=("MASTER", "SLAVE"),
        help="a session on the air: the two stations' recordings' sidecars",
    )
    args = ap.parse_args()
    if args.sidecars:
        return field_main(*args.sidecars)
    total: Counter[str] = Counter()
    for path in args.reports:
        report = json.loads(path.read_text(encoding="utf-8"))
        lines = account(report)
        split = report.get("overlap_s", {})
        print(f"{path}: {len(lines)} radiated overlaps; keyed overlap split {split}")
        for line in lines:
            total[line["mechanism"]] += 1
            print(
                f"  {line['start']:8.2f}–{line['end']:8.2f} s  {line['mechanism']:<22} "
                f"{line.get('first', '')}  ⟶  {line.get('second', '')}"
            )
    if len(args.reports) > 1:
        print("all:", "; ".join(f"{k} {v}" for k, v in total.most_common()))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
