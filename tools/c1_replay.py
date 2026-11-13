"""C1, the rate controller, replayed on a session's own frames: the third rate review's Gate 0
(``docs/research/rate-control-3/00-brief.md``).

    python -I tools/c1_replay.py MASTER.json SLAVE.json
    python -I tools/c1_replay.py --inbox field/inbox [--json out.json]

Each station chooses the rung its own data goes at (ADR-0099), so each direction of a two-sided
session is replayed on its own: the sender's C1, built as the engine builds it, fed what the
sender's engine was fed — the other station's readings, the acknowledgements, the families both
stations' control frames went in, the echo of every frame it decoded, a silent slot's step — by
``rung_reading.replay_sender``. What this adds is what the review asks of it:

* **Agreement.** Each data transmission's rung (the rung most of its data went at) against the
  replay's choice before it. Where they agree the replay explains the session; where they part
  the session did something the replay does not model (a resend at its own rung, a re-encode, a
  split), and what follows is read with care.
* **The tries.** Each try of a rung above (one frame a transmission at it), how many
  transmissions it ran, whether its verdict took the rung, and by what margin: the tried rung's
  value over the rung it was tried from, less one, when the try ended. A try lost by a few per
  cent with all its frames decoded is the guess outweighing the path.
* **Blocked.** Exchange time C1 rated the rung above better than its choice and neither chose
  it nor tried it: a rung above becomes choosable only once its own frames outweigh the prior,
  and a lost try's wait is doubled.
* **Regret.** What the best measured neighbouring rung would have delivered in the same air
  (``rung_reading.regrets``), weighted by the air each rung took.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))

import rung_reading as rr
from aether_model.link.rate import RateController

NEAR_MISS = 0.05
"""A lost try within this share of the rung it was tried from is a near miss."""


@dataclass
class Try:
    rung: int
    """The rung tried."""
    start: float
    """The sender's clock when the controller decided to try it."""
    transmissions: int = 0
    frames: int = 0
    """Frames sent at the tried rung while the try ran."""
    decoded: int = 0
    end: float | None = None
    won: bool | None = None
    margin: float | None = None
    """The tried rung's value over the rung it was tried from, less one, at the verdict."""


@dataclass
class Direction:
    sender: str
    receiver: str
    role: str
    data_transmissions: int = 0
    agree: int = 0
    tries: list[Try] = field(default_factory=list)
    blocked_s: float = 0.0
    doubled_s: float = 0.0
    """Of :attr:`blocked_s`, the time the rung above's wait had been doubled by a lost try."""
    exchange_s: float = 0.0
    regret: float | None = None
    """Air-weighted: the best measured neighbour's delivery over the rung used, less one."""
    states: list[dict[str, Any]] = field(default_factory=list)
    """C1 after each transmission: its choice, its try, the rung above and how it rates it."""

    @property
    def near_misses(self) -> int:
        return sum(
            1
            for t in self.tries
            if t.won is False and t.margin is not None and t.margin > -NEAR_MISS
        )


def _up(ctl: RateController) -> int | None:
    """The usable rung above the choice."""
    i = ctl._index + 1
    while i < len(ctl.modes):
        if ctl._usable(i):
            return ctl.modes[i]
        i += 1
    return None


def replay_direction(sender: Any, receiver: Any, d: rr.Direction) -> Direction:
    """One direction of a session: its tries, its blocked time, its agreement with the session."""
    out = Direction(d.sender, d.receiver, d.role)
    running: list[Try] = []

    def watch(tx: rr.Transmission, ctl: RateController) -> None:
        here = ctl.modes[ctl._index]
        up = _up(ctl)
        live = running[-1] if running and running[-1].end is None else None
        if live is not None:
            sent = [f for f in tx.data if f.rung == live.rung]
            if sent:
                live.transmissions += 1
                live.frames += len(sent)
                live.decoded += sum(1 for f in sent if f.decoded)
            if ctl._probing != live.rung:
                live.end = tx.key
                live.won = here >= live.rung
                below = max((m for m in ctl.modes if m < live.rung), default=here)
                base = ctl.value(below if live.won else here)
                live.margin = ctl.value(live.rung) / base - 1.0 if base > 0 else None
        if ctl._probing is not None and (not running or running[-1].end is not None):
            running.append(Try(ctl._probing, tx.key))
        blocked = (
            ctl._probing is None
            and up is not None
            and ctl.snr_db is not None
            and ctl.value(up) > ctl.value(here)
        )
        span = tx.exchange_s if tx.exchange_s is not None else tx.frame_air_s
        doubled = (
            blocked
            and up is not None
            and ctl._probe_wait.get(up, ctl.probe_every) > ctl.probe_every
        )
        if tx.data and not tx.measure:
            out.exchange_s += span
            if blocked:
                out.blocked_s += span
            if doubled:
                out.doubled_s += span
        out.states.append(
            {
                "t": round(tx.key, 2),
                "choice": here,
                "try": ctl._probing,
                "up": up,
                "snr": None if ctl.snr_db is None else round(ctl.snr_db, 2),
                "offset": round(ctl._family.get(False, 0.0), 2),
                "value_here": round(ctl.value(here), 1),
                "value_up": None if up is None else round(ctl.value(up), 1),
                "prior_up": (
                    None
                    if up is None or ctl.snr_db is None
                    else round(ctl._prior_p(up, ctl.snr_db), 3)
                ),
                "frames_up": (
                    0.0
                    if up is None
                    else round(ctl._accounts[up].frames, 2)
                    if up in ctl._accounts
                    else 0.0
                ),
                "blocked": blocked,
                "doubled": doubled,
            }
        )

    steps = rr.replay_sender(sender, receiver, d.txs, d.blocks, watch=watch)
    by_key = {round(s.t, 3): s for s in steps}
    for tx in d.txs:
        step = by_key.get(round(tx.key, 3))
        if step is None or not step.seeded or tx.rung is None:
            continue
        out.data_transmissions += 1
        out.agree += step.rung_chosen == tx.rung
    out.tries = running
    weighted = [(g.air_s, g.regret) for g in d.regrets if g.regret is not None]
    total = sum(a for a, _ in weighted)
    if total > 0:
        out.regret = sum(a * r for a, r in weighted) / total
    return out


def replay_session(master_path: Path, slave_path: Path) -> tuple[rr.Reading, list[Direction]]:
    reading = rr.read_session(master_path, slave_path)
    master, slave = rr.load_side(master_path), rr.load_side(slave_path)
    if rr.role_of(master.doc) == "slave" or rr.role_of(slave.doc) == "master":
        master, slave = slave, master
    out = []
    for d in reading.directions:
        if not any(tx.data for tx in d.txs):
            continue
        sender, receiver = (master, slave) if d.role == "master" else (slave, master)
        out.append(replay_direction(sender, receiver, d))
    return reading, out


def report(reading: rr.Reading, directions: list[Direction]) -> str:
    lines = [
        f"{reading.recording}: {reading.master} (master) and {reading.slave}, "
        f"{reading.bandwidth_hz} Hz, {reading.started} ({reading.versions[0]} / {reading.versions[1]})"
    ]
    for d in directions:
        share = f"{d.agree} of {d.data_transmissions}" if d.data_transmissions else "none"
        lines.append(
            f"  {d.sender}'s data ({d.role}): the replay chose as the session sent in {share} "
            "data transmissions"
        )
        for t in d.tries:
            verdict = "running at the end" if t.won is None else ("taken" if t.won else "lost")
            margin = "" if t.margin is None else f", {100 * t.margin:+.1f} %"
            lines.append(
                f"    try of rung {t.rung} at {t.start:.0f} s: {t.transmissions} transmissions, "
                f"{t.decoded} of {t.frames} frames decoded, {verdict}{margin}"
            )
        if d.exchange_s > 0:
            lines.append(
                f"    rated the rung above better and held off it: {d.blocked_s:.0f} s of "
                f"{d.exchange_s:.0f} s of exchanges ({100 * d.blocked_s / d.exchange_s:.0f} %), "
                f"{d.doubled_s:.0f} s of it under a wait a lost try had doubled"
            )
        if d.regret is not None:
            lines.append(
                f"    the best measured neighbour, air-weighted: {100 * d.regret:+.0f} % on the "
                "rungs used"
            )
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("sidecars", nargs="*", type=Path, help="the master's and the slave's")
    ap.add_argument("--inbox", type=Path, help="every pair of sidecars found under this directory")
    ap.add_argument("--json", type=Path, help="write every direction's replay here")
    args = ap.parse_args()
    if args.inbox:
        found = rr.pairs(sorted(args.inbox.rglob("*.json")))
        if not found:
            print(f"no two-sided session under {args.inbox}", file=sys.stderr)
            return 1
    elif len(args.sidecars) == 2:
        found = [(args.sidecars[0], args.sidecars[1])]
    else:
        ap.error("two sidecars, or --inbox DIR")
    dump = []
    seen: set[tuple[str, str]] = set()
    for master, slave in found:
        if (master.name, slave.name) in seen:  # the same session in another upload's zip
            continue
        seen.add((master.name, slave.name))
        try:
            reading, directions = replay_session(master, slave)
        except Exception as error:  # a session the reading cannot place is said, not fatal
            print(f"{master.stem}: not read ({error})\n")
            continue
        print(report(reading, directions))
        print()
        dump.append(
            {
                "recording": reading.recording,
                "bandwidth_hz": reading.bandwidth_hz,
                "started": reading.started,
                "versions": list(reading.versions),
                "directions": [{**asdict(d), "near_misses": d.near_misses} for d in directions],
            }
        )
    if args.json:
        args.json.write_text(json.dumps(dump, indent=1), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
