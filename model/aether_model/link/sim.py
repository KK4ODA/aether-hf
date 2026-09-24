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
* Loss: each frame draws a uniform once; it is lost if the draw exceeds the frame's success
  probability at the channel SNR. A retransmission of the same block accumulates ≈ 3 dB of
  "energy" per combine; the combined frame succeeds once the summed energy clears the
  threshold — the same qualitative behaviour as LDPC HARQ-IR.
* Each frame is judged at its own threshold: a DATA frame at its mode's, a CONTROL frame at
  its family's control frame's (the ordinary SHORT frame or the floor one, ADR-0009). A
  delivered frame carries its family, as the modem reports it: a DATA frame's is its
  mode's, a CONTROL frame's the layout it went out on — the engine drops a floor-mode frame
  whose flag disagrees with its mode, as it must for a real one.
"""

from __future__ import annotations

import heapq
import math
import random
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import cast

from aether_model.link.engine import Deliver, Event, LinkEngine, Transmit
from aether_model.link.fading import FadingPipe, success_probability
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    CONTROL_THRESHOLD_DB,
)

FrameFactory = Callable[[TxFrame, float, float, float], "SoftFrame | None"]
"""(frame, snr_db, t_start, t_end) -> a delivered SoftFrame, or None if it was not
detected at all. The lossy pipe uses a synthetic model; the PHY harness renders real DSP."""

_STEEP = 1.2
"""Logistic steepness of FER vs SNR (dB); larger = sharper waterfall."""
_HARQ_GAIN_DB = 3.0
"""Soft-combining energy gained per retransmission of the same block."""


def _success_prob(threshold: float, snr_db: float, energy_db: float) -> float:
    return 1.0 / (1.0 + math.exp(-_STEEP * (snr_db + energy_db - threshold)))


def control_thresholds_for(timing: PhyTiming) -> dict[bool, float]:
    """The AWGN thresholds of an air's two control frames, keyed by family, as its timing
    carries them — the wide air's when it does not."""
    return dict(timing.control_threshold_db or CONTROL_THRESHOLD_DB)


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
    floor: bool = False
    _decode_snr_db: float | None = None
    """The SNR this frame decodes at, when a fading channel has put it through its
    effective-SNR mapping (:mod:`aether_model.link.fading`): judged on the modem's steep
    AWGN waterfall. Unset, the frame is judged at its reported SNR on the pipe's soft
    logistic, the per-class average."""
    _threshold: float | None = None
    """The SNR at which this frame decodes nine times in ten on the channel being modelled:
    a DATA frame's mode's, a CONTROL frame's family's. The AWGN table's for its mode when
    unset."""
    _judge_snr_db: float | None = None
    """The SNR the pipe's logistic judges this frame at when the receiver reports another — a
    tone-floor frame whose reading is capped (:attr:`TwoStationSim.floor_reading_cap_db`).
    Unset, its reported SNR."""

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        prior = float(buffer) if isinstance(buffer, (int, float)) else 0.0
        gained = prior + (_HARQ_GAIN_DB if buffer is not None else 0.0)
        threshold = self._threshold
        if threshold is None:
            threshold = AWGN_THRESHOLD_DB.get(self.mode, 0.0)
        if self._decode_snr_db is not None:
            p = success_probability(self._decode_snr_db + gained, threshold)
        else:
            judged = self.snr_db if self._judge_snr_db is None else self._judge_snr_db
            p = _success_prob(threshold, judged, gained)
        if self._draw <= p:
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
        control_thresholds: dict[bool, float] | None = None,
        frame_snr_offset: Callable[[TxFrame], float] | None = None,
        fading: FadingPipe | None = None,
        floor_reading_cap_db: float | None = None,
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
        """Per-mode thresholds of the channel being modelled; the air's AWGN table when
        unset (the one its timing hands the rate controller, the wide table without one)."""
        self.control_thresholds = control_thresholds
        """The two control frames' thresholds on the channel being modelled, keyed by
        family; the air's AWGN values (:func:`control_thresholds_for`) when unset."""
        self.snr_schedule = snr_schedule
        """SNR as a function of time, for ramps. Overrides :attr:`snr_db` when set."""
        self.frame_snr_offset = frame_snr_offset
        """Decibels added to the channel SNR for one frame. A transmitter is driven to a
        fixed *peak*, so a frame's average power — the SNR the far end measures — is that
        peak less its own peak-to-average ratio: with the offset each frame's negative
        ratio, :attr:`snr_db` is the SNR at equal peak power (P9-6)."""
        self.floor_reading_cap_db = floor_reading_cap_db
        """The most a tone-floor frame's SNR reads: the floor's estimate saturates on a strong
        path, near +17 dB on AWGN and a few decibels on a dispersive one (ADR-0016). The
        frame is still judged at the channel's SNR. Unset, it reads what the channel gives."""
        self.fading = fading
        """A fading channel both stations share (P9-6): each frame's reported SNR and the
        SNR it decodes at come from its own stretch of the fade, and the per-mode
        thresholds are then the AWGN table's. Unset, every frame sees the channel SNR and
        the per-class averages of :attr:`thresholds`."""

    def _synthetic_frame(
        self, frame: TxFrame, snr_db: float, t_start: float, t_end: float
    ) -> SoftFrame | None:
        timing = self.st[0].engine.timing
        control = frame.container is Container.CONTROL
        # a data frame's family is its mode's; a control frame says which it went out on
        floor = frame.floor if control else timing.is_floor(frame.mode)
        if control:
            threshold = (self.control_thresholds or control_thresholds_for(timing))[floor]
        else:
            table = self.thresholds or timing.mode_threshold_db or AWGN_THRESHOLD_DB
            threshold = table.get(frame.mode, 0.0)
        decode_snr_db = None
        if self.fading is not None:
            snr_db, decode_snr_db = self.fading.judge(frame, snr_db, t_start, t_end)
        judge_snr_db = None
        cap = self.floor_reading_cap_db
        if floor and cap is not None and snr_db > cap:
            judge_snr_db, snr_db = snr_db, cap
        return SimFrame(
            container=frame.container,
            mode=frame.mode,
            rv=frame.rv,
            snr_db=snr_db,
            t_start=t_start,
            t_end=t_end,
            payload=frame.payload,
            _draw=self.rng.random(),
            floor=floor,
            _decode_snr_db=decode_snr_db,
            _threshold=threshold,
            _judge_snr_db=judge_snr_db,
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
        timing = st.engine.timing
        for frame in tx.frames:
            dur = timing.frame_s(frame)
            control = frame.container is Container.CONTROL
            floor = frame.floor if control else timing.is_floor(frame.mode)
            sof = timing.preamble_detect_s_for(floor)
            if sof is not None:
                # acquisition succeeds far below every mode's decode threshold (P2-3: 100 %
                # at −5 dB), so a listening receiver is assumed to see every preamble — a
                # control frame's too, as the daemon hands the engine every trusted one
                # (ADR-0016), each once its family announces it — and the layout it names,
                # so the frame's own length
                self._push(t + sof, "preamble", 1 - who, (t, t + sof, dur))
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
        snr = self.snr_at(0.5 * (t0 + t1))
        if self.frame_snr_offset is not None:
            snr += self.frame_snr_offset(frame)
        sf = self._factory(frame, snr, t0 + self.prop_s, arrival)
        if sf is None:
            return
        eng = self.st[rx].engine
        eng.tick(arrival)
        eng.on_frame(sf, arrival)
        self._pump(rx, arrival)

    def _announce(self, rx: int, t_start: float, at: float, frame_s: float) -> None:
        """Tell a listening receiver that a frame's preamble was detected (P2-2a), and how
        long the frame it names is."""
        if self._busy(rx, t_start, at):
            return
        eng = self.st[rx].engine
        eng.tick(at)
        eng.on_preamble(t_start + self.prop_s, at, frame_s)
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
                    t0, t1, frame_s = cast("tuple[float, float, float]", ev.data)
                    self._announce(ev.who, t0, t1, frame_s)
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
