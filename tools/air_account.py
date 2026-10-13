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
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

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


def summary(lines: list[dict[str, Any]]) -> str:
    """``mechanism n`` pairs, most frequent first: what a results row carries."""
    counts = Counter(line["mechanism"] for line in lines)
    return "; ".join(f"{name} {n}" for name, n in counts.most_common())


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("reports", nargs="+", type=Path)
    args = ap.parse_args()
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
