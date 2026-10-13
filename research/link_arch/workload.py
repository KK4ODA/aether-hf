"""Workloads and one-run metrics for the architecture comparison.

Three shapes of traffic, each scripted from the trial number alone so every engine runs the
same script:

* ``bulk``: the caller sends ``nbytes`` and disconnects (``bench_link.py``'s session);
* ``exchange``: a Winlink-shaped conversation — proposals, answers and messages alternating
  direction, each sent a host's reaction time after the one before it arrived;
* ``chat``: ``bench_chat.py``'s keyboard-to-keyboard lines with operators' thinking time.

A run's row says whether everything arrived and the session ended at both ends, how long it
took, the air each transmitter used, and every overlap of the two transmitters, classified.
"""

from __future__ import annotations

import random
import statistics
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from research.link_arch.advsim import AdvSim
from research.link_arch.classify import classify


@dataclass(frozen=True)
class Message:
    sender: int
    body: bytes
    think_s: float


def script(kind: str, trial: int, nbytes: int = 4000) -> list[Message]:
    rng = random.Random(9173 + trial)
    if kind == "bulk":
        return [Message(0, bytes(rng.randrange(256) for _ in range(nbytes)), 0.0)]
    if kind == "exchange":
        # a B2F-like session: the client proposes, the gateway answers and sends its
        # proposal, then messages both ways, each a host's reaction after the last arrived
        sizes = [
            (0, 60),
            (1, 80),
            (0, 40),
            (1, rng.randint(800, 3000)),
            (0, 60),
            (0, rng.randint(500, 2000)),
            (1, 40),
            (1, 30),
        ]
        return [
            Message(s, bytes(rng.randrange(256) for _ in range(n)), rng.uniform(0.3, 1.5))
            for s, n in sizes
        ]
    if kind == "chat":
        out = []
        sender = rng.randrange(2)
        for i in range(rng.randint(8, 14)):
            if i > 0 and rng.random() >= 0.2:
                sender = 1 - sender
            n = rng.randint(20, 120)
            out.append(
                Message(
                    sender, bytes(rng.randrange(32, 127) for _ in range(n)), rng.uniform(5.0, 30.0)
                )
            )
        return out
    raise ValueError(kind)


class Driver:
    """Feeds a script to two engines: the caller calls, each message goes once the one before
    it has arrived and its sender's thinking time has passed, and the caller disconnects
    after the last."""

    def __init__(self, sim: AdvSim, msgs: list[Message], remote: str) -> None:
        self.sim = sim
        self.msgs = msgs
        self.remote = remote
        self.i = 0
        self.sent_at: list[float] = []
        self.arrived_at: list[float | None] = [None] * len(msgs)
        self.expect = [0, 0]  # bytes each station should have received so far
        self.started = False
        self.disc_at: float | None = None
        sim.app_hooks.append(self._grown)

    def start(self) -> None:
        eng = self.sim.st[0].engine
        eng.connect(self.remote)
        if self.msgs and self.msgs[0].sender == 0:
            # the caller's first message is handed over with the call, as bench_link does
            self._send(0.0)
        self.sim._pump(0, 0.0)

    def _send(self, now: float) -> None:
        m = self.msgs[self.i]
        self.sent_at.append(now)
        self.expect[1 - m.sender] += len(m.body)
        self.sim.st[m.sender].engine.send(m.body)

    def _grown(self, sim: AdvSim, who: int, now: float) -> None:
        if self.i >= len(self.msgs):
            return
        m = self.msgs[self.i]
        if who != 1 - m.sender or len(sim.st[who].delivered) < self.expect[who]:
            return
        self.arrived_at[self.i] = now
        self.i += 1
        if self.i >= len(self.msgs):
            eng = sim.st[0].engine
            sim.schedule_app(now + 0.5, 0, eng.disconnect)
            self.disc_at = now + 0.5
            return
        nxt = self.msgs[self.i]
        sim.schedule_app(now + nxt.think_s, nxt.sender, lambda t=now + nxt.think_s: self._send(t))

    def connected_hook(self) -> None:
        """For scripts whose first message is the called station's: send it once both are up."""


def run(
    make: Callable[[int], tuple[Any, Any, AdvSim]],
    kind: str,
    trial: int,
    nbytes: int = 4000,
    until: float = 1500.0,
) -> dict[str, Any]:
    _a, b, sim = make(trial)
    msgs = script(kind, trial, nbytes)
    drv = Driver(sim, msgs, "B2BBB")
    # a message the called station sends first waits for the session to be up
    if msgs and msgs[0].sender == 1:

        def poll_up() -> None:
            if b.state.value == "connected" and not drv.started:
                drv.started = True
                drv._send(sim.t)
            elif not drv.started:
                sim.schedule_app(sim.t + 0.25, 1, poll_up)

        sim.schedule_app(0.25, 1, poll_up)
    else:
        drv.started = True
    drv.start()
    end = sim.run(until=until)
    want = [b"".join(m.body for m in msgs if m.sender == 1 - w) for w in (0, 1)]
    ok = all(sim.delivered(w) == want[w] for w in (0, 1))
    reasons = [
        next(
            (
                e.split(":", 1)[1]
                for _, e in reversed(sim.events(w))
                if e.startswith("disconnected:")
            ),
            "up",
        )
        for w in (0, 1)
    ]
    lat = [
        (arr - sent)
        for sent, arr in zip(drv.sent_at, drv.arrived_at, strict=False)
        if arr is not None
    ]
    done = max((x for x in drv.arrived_at if x is not None), default=end)
    payload = sum(len(m.body) for m in msgs)
    over = sim.overlaps()
    classes = [classify(x, y, k, r) for x, y, k, r in over]
    dual, _ = sim.dual_time()
    row: dict[str, Any] = {
        "kind": kind,
        "trial": trial,
        "ok": int(ok),
        "clean_end": int(all(r in ("closed", "peer disconnected") for r in reasons)),
        "end_a": reasons[0],
        "end_b": reasons[1],
        "done_s": round(done, 2),
        "end_s": round(end, 2),
        "goodput_bps": round(8 * payload / done, 1) if ok and done > 0 else 0.0,
        "lat_med_s": round(statistics.median(lat), 2) if lat else "",
        "lat_max_s": round(max(lat), 2) if lat else "",
        "air_a_s": round(sim.airtime(0), 2),
        "air_b_s": round(sim.airtime(1), 2),
        "tx_count": len(sim.txs),
        "overlaps": len(over),
        "rf_overlaps": sum(1 for *_, r in over if r > 0),
        "overlap_s": round(sum(k for *_, k, _ in over), 2),
        "dual_s": round(dual, 2),
        "phantoms": sum(sim.phantoms),
        "dropped": sum(sim.dropped),
    }
    for c in classes:
        row[f"c:{c}"] = row.get(f"c:{c}", 0) + 1
    row["_overlaps"] = over
    row["_classes"] = classes
    return row
