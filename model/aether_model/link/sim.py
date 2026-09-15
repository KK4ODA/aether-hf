"""Two-engine discrete-event simulator over a lossy pipe (roadmap P2-1).

This is the *fast* harness: no DSP. A single half-duplex channel connects two
:class:`~aether_model.link.engine.LinkEngine` stations; frames are delivered frame by
frame with a per-mode error model and an energy-accumulation stand-in for HARQ-IR, so the
protocol's timers, selective repeat, HARQ inference, turnaround and handshakes can be
exercised in milliseconds over thousands of seeded trials.
:mod:`aether_model.link.harness` is the slow harness that swaps this pipe for the real PHY
and channel simulator, and confirms the qualitative behaviour end to end.

Channel model
-------------
* Half-duplex: a station hears a frame only if it was not transmitting for any part of it.
* Simultaneous transmissions collide — both frames are lost (the ``connect`` race).
* Loss: each frame draws a uniform once; it is lost if the draw exceeds the mode's success
  probability at the channel SNR. A retransmission of the same block accumulates ≈ 3 dB of
  "energy" per combine; the combined frame succeeds once the summed energy clears the mode
  threshold — the same qualitative behaviour as LDPC HARQ-IR.
"""

from __future__ import annotations

import heapq
import math
import random
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import cast

from aether_model.link.engine import Deliver, Event, LinkEngine, Transmit
from aether_model.link.phy import Container, SoftFrame, TxFrame
from aether_model.link.rate import AWGN_THRESHOLD_DB

FrameFactory = Callable[[TxFrame, float, float, float], "SoftFrame | None"]
"""(frame, snr_db, t_start, t_end) -> a delivered SoftFrame, or None if it was not
detected at all. The lossy pipe uses a synthetic model; the PHY harness renders real DSP."""

_STEEP = 1.2
"""Logistic steepness of FER vs SNR (dB); larger = sharper waterfall."""
_HARQ_GAIN_DB = 3.0
"""Soft-combining energy gained per retransmission of the same block."""


def _success_prob(
    mode: int, snr_db: float, energy_db: float, thresholds: dict[int, float] | None = None
) -> float:
    threshold = (thresholds or AWGN_THRESHOLD_DB).get(mode, 0.0)
    return 1.0 / (1.0 + math.exp(-_STEEP * (snr_db + energy_db - threshold)))


@dataclass
class SimFrame:
    """A frame as delivered to the receiving engine (implements the SoftFrame protocol).

    The HARQ ``buffer`` is the accumulated combining energy in dB; ``decode`` returns the
    new energy so the engine can carry it to the next retransmission."""

    container: Container
    mode: int
    rv: int
    snr_db: float
    t_start: float
    t_end: float
    payload: bytes
    _draw: float
    _thresholds: dict[int, float] | None = None
    """Per-mode FER thresholds of the channel being modelled; AWGN when unset."""

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        prior = float(buffer) if isinstance(buffer, (int, float)) else 0.0
        gained = prior + (_HARQ_GAIN_DB if buffer is not None else 0.0)
        if self._draw <= _success_prob(self.mode, self.snr_db, gained, self._thresholds):
            return self.payload, gained
        return None, gained


@dataclass(order=True)
class _Ev:
    t: float
    seq: int
    kind: str = field(compare=False)
    who: int = field(compare=False)
    data: object = field(compare=False)


@dataclass
class _Station:
    engine: LinkEngine
    delivered: bytearray = field(default_factory=bytearray)
    events: list[str] = field(default_factory=list)
    tx_end: float = 0.0
    busy: list[tuple[float, float]] = field(default_factory=list)


class TwoStationSim:
    """Drive two engines to quiescence over a lossy half-duplex pipe."""

    def __init__(
        self,
        a: LinkEngine,
        b: LinkEngine,
        *,
        snr_db: float = 10.0,
        seed: int = 0,
        prop_s: float = 0.01,
        frame_factory: FrameFactory | None = None,
        thresholds: dict[int, float] | None = None,
        snr_schedule: Callable[[float], float] | None = None,
    ) -> None:
        self.st = [_Station(a), _Station(b)]
        self.snr_db = snr_db
        self.prop_s = prop_s
        self.rng = random.Random(seed)
        self._q: list[_Ev] = []
        self._seq = 0
        self.t = 0.0
        self._factory: FrameFactory = frame_factory or self._synthetic_frame
        self.thresholds = thresholds
        self.snr_schedule = snr_schedule
        """SNR as a function of time, for ramps. Overrides :attr:`snr_db` when set."""

    def _synthetic_frame(
        self, frame: TxFrame, snr_db: float, t_start: float, t_end: float
    ) -> SoftFrame | None:
        return SimFrame(
            container=frame.container,
            mode=frame.mode,
            rv=frame.rv,
            snr_db=snr_db,
            t_start=t_start,
            t_end=t_end,
            payload=frame.payload,
            _draw=self.rng.random(),
            _thresholds=self.thresholds,
        )

    # ── scheduling ────────────────────────────────────────────────────

    def set_snr(self, snr_db: float) -> None:
        self.snr_db = snr_db

    def snr_at(self, t: float) -> float:
        return self.snr_schedule(t) if self.snr_schedule is not None else self.snr_db

    def _push(self, t: float, kind: str, who: int, data: object = None) -> None:
        heapq.heappush(self._q, _Ev(t, self._seq, kind, who, data))
        self._seq += 1

    def _pump(self, who: int, at: float) -> None:
        st = self.st[who]
        for act in st.engine.drain():
            if isinstance(act, Transmit):
                self._launch(who, at, act)
            elif isinstance(act, Deliver):
                st.delivered += act.data
            elif isinstance(act, Event):
                st.events.append(f"{act.name}:{act.detail}")

    def _launch(self, who: int, at: float, tx: Transmit) -> None:
        st = self.st[who]
        t = max(at, st.tx_end)
        st.busy.append((t, t + tx.duration_s))
        sof = st.engine.timing.preamble_detect_s
        for frame in tx.frames:
            dur = (
                st.engine.timing.data_frame_s
                if frame.container is Container.DATA
                else st.engine.timing.control_frame_s
            )
            if sof is not None and frame.container is Container.DATA:
                # acquisition succeeds far below every mode's decode threshold (P2-3: 100 %
                # at −5 dB), so a listening receiver is assumed to see every preamble
                self._push(t + sof, "preamble", 1 - who, (t, t + sof))
            self._push(t + dur, "arrive", 1 - who, (frame, t, t + dur))
            t += dur
        st.tx_end = t
        self._push(t, "tx_done", who)

    def _busy(self, who: int, t0: float, t1: float) -> bool:
        return any(a < t1 - 1e-9 and t0 + 1e-9 < b for a, b in self.st[who].busy)

    def _deliver(self, rx: int, frame: TxFrame, t0: float, t1: float) -> None:
        if self._busy(rx, t0, t1):
            return  # half-duplex or collision: the receiver was transmitting
        arrival = t1 + self.prop_s
        sf = self._factory(frame, self.snr_at(0.5 * (t0 + t1)), t0 + self.prop_s, arrival)
        if sf is None:
            return
        eng = self.st[rx].engine
        eng.tick(arrival)
        eng.on_frame(sf, arrival)
        self._pump(rx, arrival)

    def _announce(self, rx: int, t_start: float, at: float) -> None:
        """Tell a listening receiver that a frame's preamble was detected (P2-2a)."""
        if self._busy(rx, t_start, at):
            return
        eng = self.st[rx].engine
        eng.tick(at)
        eng.on_preamble(t_start + self.prop_s, at)
        self._pump(rx, at)

    # ── run loop ──────────────────────────────────────────────────────

    def run(self, until: float = 600.0, idle_gap: float = 3.0) -> float:
        """Advance until both engines have been idle for ``idle_gap`` seconds or ``until``
        is reached. Returns the final simulation time."""
        for who in (0, 1):
            self._pump(who, self.t)
        last = 0.0
        while self.t < until:
            nq = self._q[0].t if self._q else math.inf
            nds = [self.st[w].engine.next_deadline() for w in (0, 1)]
            nt = min([nq] + [d for d in nds if d is not None] or [math.inf])
            if not math.isfinite(nt) or nt > until:
                break
            self.t = nt
            progressed = False
            for who in (0, 1):
                dl = self.st[who].engine.next_deadline()
                if dl is not None and dl <= nt + 1e-9:
                    self.st[who].engine.tick(nt)
                    self._pump(who, nt)
                    progressed = True
            while self._q and self._q[0].t <= nt + 1e-9:
                ev = heapq.heappop(self._q)
                if ev.kind == "arrive":
                    fr, t0, t1 = cast("tuple[TxFrame, float, float]", ev.data)
                    self._deliver(ev.who, fr, t0, t1)
                elif ev.kind == "preamble":
                    t0, t1 = cast("tuple[float, float]", ev.data)
                    self._announce(ev.who, t0, t1)
                elif ev.kind == "tx_done":
                    self.st[ev.who].engine.on_tx_done(nt)
                    self._pump(ev.who, nt)
                progressed = True
            if progressed:
                last = self.t
            elif self.t - last > idle_gap:
                break
        return self.t

    # ── convenience ───────────────────────────────────────────────────

    def delivered(self, who: int) -> bytes:
        return bytes(self.st[who].delivered)

    def events(self, who: int) -> list[str]:
        return self.st[who].events
