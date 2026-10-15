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

The station (ADR-0069 §5)
-------------------------
By default a station keys the instant its engine asks, its receiver reports a frame the instant
it ends, and it hears everything that is there. Protocol 8's safety rests on bounds of exactly
those things, so each can be put back, behind its own argument, as the review's adversarial
simulator (``research/link_arch/advsim.py``) has them: a keying lead per transmission
(``key_lead``), a decode latency per frame (``decode_latency``), a receiver deaf for a while after
its own transmission (``rx_recovery_s``), false detections (``phantoms_per_min``), and the keying
gate — a transmission that carries ``not_after`` (``granted.SlotTransmit``) and would start
radiating later is not sent, and the engine is told (``on_tx_dropped``). What each station
radiated is kept (:meth:`TwoStationSim.radiated`), so a test can assert that the two never
radiated at once (:meth:`TwoStationSim.overlaps`).
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
from aether_model.link.granted import HEADER_UNDECODABLE_SHARE, GrantedEngine
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
    trusted: bool = True
    """A simulated frame is a real frame, and its SNR the channel's."""
    follows: int | None = None
    """The burst countdown as sent, on an ordinary DATA frame (ADR-0041): a simulated
    receiver reads it right whenever it detects the frame."""

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        prior = float(buffer) if isinstance(buffer, (int, float)) else 0.0
        gained = prior + (_HARQ_GAIN_DB if buffer is not None else 0.0)
        if self._draw <= self._success(gained):
            return self.payload, gained
        return None, gained

    def decodes_alone(self) -> bool:
        """Whether this copy decodes without an earlier one of its block."""
        return self._draw <= self._success(0.0)

    def _success(self, gained: float) -> float:
        threshold = self._threshold
        if threshold is None:
            threshold = AWGN_THRESHOLD_DB.get(self.mode, 0.0)
        if self._decode_snr_db is not None:
            p = success_probability(self._decode_snr_db + gained, threshold)
        else:
            judged = self.snr_db if self._judge_snr_db is None else self._judge_snr_db
            p = _success_prob(threshold, judged, gained)
        return p


@dataclass(order=True)
class _Ev:
    t: float
    seq: int
    kind: str = field(compare=False)
    who: int = field(compare=False)
    data: object = field(compare=False)


Engine = LinkEngine | GrantedEngine


@dataclass
class _Station:
    engine: Engine
    delivered: bytearray = field(default_factory=bytearray)
    events: list[str] = field(default_factory=list)
    tx_end: float = 0.0
    busy: list[tuple[float, float]] = field(default_factory=list)
    """What the station radiated: one interval a transmission, its identifier included."""
    dropped: list[float] = field(default_factory=list)
    """When the keying gate refused a transmission that would have started late."""
    first_frame: tuple[float, float, bool] | None = None
    """The transmission on the air's first frame: its start, its end, and whether it is a
    control frame — what :meth:`TwoStationSim.cut` judges a cut by."""


class TwoStationSim:
    """Drive two engines to quiescence over a lossy half-duplex pipe."""

    def __init__(
        self,
        a: Engine,
        b: Engine,
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
        key_limit_s: float | None = None,
        unheard: Callable[[int, Container, float], bool] | None = None,
        countdown: bool = True,
        key_lead: Callable[[int], float] | None = None,
        decode_latency: Callable[[int, TxFrame, float], float] | None = None,
        rx_recovery_s: tuple[float, float] = (0.0, 0.0),
        phantoms_per_min: float = 0.0,
        phantom_trusted: float = 0.05,
        count_misread: float = 0.0,
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
        self.key_limit_s = key_limit_s
        """The transmitter's key-time limit, seconds: the daemon's watchdog unkeys the radio
        when a transmission reaches it. The frame on the air at that moment arrives cut —
        its receiver acquires it and cannot decode it, which counts as a frame the channel
        lost — and nothing after it is sent. Unset, a transmission is as long as it is."""
        self.fading = fading
        """A fading channel both stations share (P9-6): each frame's reported SNR and the
        SNR it decodes at come from its own stretch of the fade, and the per-mode
        thresholds are then the AWGN table's. Unset, every frame sees the channel SNR and
        the per-class averages of :attr:`thresholds`."""
        self.unheard = unheard
        self.countdown = countdown
        """False: frames arrive without the burst countdown (ADR-0041), as from a sender before
        it — how the receiver's other ways of finding a burst's end are measured."""
        self.floor_counts = (
            a.counts_on_floor if isinstance(a, GrantedEngine) else a.cfg.stated_answers
        )
        """Tone-floor frames carry their count too (ADR-0068: in the middle sync block): with
        stated answers, as the PHY of link protocol 8 sends them."""
        self.key_lead = key_lead
        """Seconds from a station's engine asking for a transmission to its first radiated
        sample, drawn per transmission: the daemon's lead, the card, CAT keying. Unset, none —
        the pipe plays what it is handed at once."""
        self.decode_latency = decode_latency
        """Seconds from a frame's end to its report reaching the receiving engine, drawn per
        frame: called with the receiver, the frame and when it ends. Unset, none."""
        self.rx_recovery_s = rx_recovery_s
        """How long each station's receiver hears nothing after it stops transmitting (a VOX
        hold, a receiver's recovery)."""
        self.phantoms_per_min = phantoms_per_min
        """False detections a minute of listening, per station: a frame that never decodes,
        of a random kind and mode, trusted (ADR-0020) a :attr:`phantom_trusted` part of the
        time."""
        self.phantom_trusted = phantom_trusted
        """Frames a receiver never detects, preamble or frame: called with the receiver, the
        frame's container and when it started. A fade that lets one kind of frame through and
        not another — the called station's data lost while its control frames arrive — is
        what turned KE4QCM's session of 2026-09-25 into two stations holding the turn."""
        self.count_misread = count_misread
        """The share of data frames too weak to decode alone whose count a receiver reads as
        none to follow: a weak copy's chips read a quarter turn off. On the scenario harness a
        frame HARQ rescued at -1.5 dB carried three to follow read as none (10-step-3.md §6).
        Unset, every count is read as sent; drawn only when set, so a run without it is the
        run it always was."""

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
        sf = SimFrame(
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
            follows=(
                None
                if control or (floor and not self.floor_counts) or not self.countdown
                else frame.follows
            ),
        )
        if (
            self.count_misread > 0.0
            and sf.follows
            and not sf.decodes_alone()
            and self.rng.random() < self.count_misread
        ):
            sf.follows = 0
        return sf

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
        lead = self.key_lead(who) if self.key_lead is not None else 0.0
        # an answer decided ahead of its slot is keyed to radiate at the slot's start: the
        # station keys its own latency before it, and radiates a lead after it keys
        not_before = getattr(tx, "not_before", None)
        if not_before is not None:
            at = max(at, not_before - st.engine.timing.tx_latency_s)
        t = max(at + lead, st.tx_end)
        not_after = getattr(tx, "not_after", None)
        if not_after is not None and t > not_after + 1e-9:
            # the keying gate: a transmission that would start past its slot is not sent
            st.dropped.append(t)
            self._push(t, "dropped", who)
            return
        st.busy.append((t, t + tx.duration_s))
        timing = st.engine.timing
        key_up = t + self.key_limit_s if self.key_limit_s is not None else float("inf")
        identifier_s = getattr(tx, "identifier_s", 0.0)
        st.first_frame = None
        for frame in tx.frames:
            dur = timing.frame_s(frame)
            if st.first_frame is None:
                st.first_frame = (t, t + dur, frame.container is Container.CONTROL)
            if t >= key_up - 1e-9:
                break  # the key is up: nothing more goes out
            cut = t + dur > key_up + 1e-9
            control = frame.container is Container.CONTROL
            floor = frame.floor if control else timing.is_floor(frame.mode)
            sof = timing.preamble_detect_s_for(floor)
            if sof is not None:
                # acquisition succeeds far below every mode's decode threshold (P2-3: 100 %
                # at −5 dB), so a listening receiver is assumed to see every preamble — a
                # control frame's too, as the daemon hands the engine every trusted one
                # (ADR-0016), each once its family announces it — and the layout it names,
                # so the frame's own length
                self._push(t + sof, "preamble", 1 - who, (t, t + sof, dur, frame.container))
            self._push(t + dur, "arrive", 1 - who, (frame, t, t + dur, cut))
            t += dur
        t += identifier_s  # the closing identifier, behind the frames or alone (ADR-0075)
        st.tx_end = t
        self._push(t, "tx_done", who)

    def cut(self, who: int) -> None:
        """Cut station ``who``'s transmission on the air now, as the daemon's abort or key
        watchdog does. What has not gone out never does, the frame on the air arrives cut, and
        the engine is told where the transmission ended: at the cut, as never read, when its
        first frame was a control frame not :data:`HEADER_UNDECODABLE_SHARE` of the way out
        (10-step-3.md §3.6); otherwise where its frames would have ended, which is where a
        header the other station read anchors the slot (09-step-2.md §2.2)."""
        st = self.st[who]
        now = self.t
        if now >= st.tx_end - 1e-9:
            return
        kept: list[_Ev] = []
        for ev in self._q:
            if ev.kind == "tx_done" and ev.who == who:
                continue
            if ev.kind in ("arrive", "preamble") and ev.who == 1 - who:
                if ev.kind == "arrive":
                    frame, t0, t1, _ = cast("tuple[TxFrame, float, float, bool]", ev.data)
                else:
                    t0 = cast("tuple[float, float, float, Container]", ev.data)[0]
                if t0 >= now - 1e-9 and t0 >= st.busy[-1][0] - 1e-9:
                    continue  # not begun: never sent
                if ev.kind == "arrive" and t1 > now + 1e-9:
                    # on the air at the cut: it arrives cut
                    kept.append(_Ev(ev.t, ev.seq, ev.kind, ev.who, (frame, t0, t1, True)))
                    continue
            kept.append(ev)
        heapq.heapify(kept)
        self._q = kept
        whole_end = st.tx_end
        st.tx_end = now
        st.busy[-1] = (st.busy[-1][0], now)
        engine = st.engine
        first = st.first_frame
        unread = (
            first is not None
            and first[2]
            and now < first[0] + HEADER_UNDECODABLE_SHARE * (first[1] - first[0])
        )
        if isinstance(engine, GrantedEngine) and unread:
            engine.on_tx_cut_unread(now)
        elif isinstance(engine, GrantedEngine):
            engine.on_tx_done(now, whole_end)
        else:
            engine.on_tx_done(now)
        self._pump(who, now)

    def _busy(self, who: int, t0: float, t1: float) -> bool:
        deaf = self.rx_recovery_s[who]
        return any(a < t1 - 1e-9 and t0 + 1e-9 < b + deaf for a, b in self.st[who].busy)

    def _deliver(self, rx: int, frame: TxFrame, t0: float, t1: float, *, cut: bool = False) -> None:
        if self.unheard is not None and self.unheard(rx, frame.container, t0):
            return  # never detected
        if self._busy(rx, t0, t1):
            return  # half-duplex or collision: the receiver was transmitting
        arrival = t1 + self.prop_s
        snr = self.snr_at(0.5 * (t0 + t1))
        if self.frame_snr_offset is not None:
            snr += self.frame_snr_offset(frame)
        sf = self._factory(frame, snr, t0 + self.prop_s, arrival)
        if sf is None:
            return
        if cut:
            if not isinstance(sf, SimFrame):
                return  # a real modem's frame cannot be cut here: it is not delivered
            sf._draw = 2.0  # acquired, and past every probability of decoding
        latency = 0.0
        if self.decode_latency is not None:
            latency = self.decode_latency(rx, frame, arrival)
        if latency > 0.0:
            self._push(arrival + latency, "report", rx, sf)
            return
        self._report(rx, sf, arrival)

    def _report(self, rx: int, sf: SoftFrame, at: float) -> None:
        eng = self.st[rx].engine
        eng.tick(at)
        eng.on_frame(sf, at)
        self._pump(rx, at)

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

    def _phantom(self, rx: int, t0: float) -> None:
        """A false detection starting at ``t0``: announced, then reported as a frame that does
        not decode — of a random container and mode, trusted now and then (ADR-0020)."""
        rate = self.phantoms_per_min / 60.0
        self._push(t0 + self.rng.expovariate(rate), "phantom", rx)
        timing = self.st[rx].engine.timing
        modes = sorted(timing.data_capacity or {0: 0})
        control = self.rng.random() < 0.5
        mode = self.rng.choice(modes)
        floor = self.rng.random() < 0.3 if control else timing.is_floor(mode)
        length = timing.control_frame_s_for(floor) if control else timing.data_frame_s_for(mode)
        if self._busy(rx, t0, t0 + length):
            return
        frame = TxFrame(
            Container.CONTROL if control else Container.DATA,
            bytes(self.rng.randrange(256) for _ in range(7)),
            mode=0 if control else mode,
            rv=self.rng.randrange(4),
            floor=floor,
            follows=self.rng.randrange(4),
        )
        sf = SimFrame(
            container=frame.container,
            mode=frame.mode,
            rv=frame.rv,
            snr_db=self.rng.uniform(-15.0, 5.0),
            t_start=t0,
            t_end=t0 + length,
            payload=frame.payload,
            _draw=2.0,
            floor=floor,
            trusted=self.rng.random() < self.phantom_trusted,
            follows=None if control else frame.follows,
        )
        sof = timing.preamble_detect_s_for(floor)
        if sof is not None:
            self._push(t0 + sof, "preamble", rx, (t0, t0 + sof, length, frame.container))
        latency = 0.0
        if self.decode_latency is not None:
            latency = self.decode_latency(rx, frame, t0 + length)
        self._push(t0 + length + latency, "report", rx, sf)

    # ── what went on the air ──────────────────────────────────────────

    def radiated(self, who: int) -> list[tuple[float, float]]:
        """The intervals station ``who`` radiated, in order."""
        return list(self.st[who].busy)

    def overlaps(self) -> list[tuple[float, float]]:
        """Every stretch both stations radiated at once."""
        out = []
        for a0, a1 in self.st[0].busy:
            for b0, b1 in self.st[1].busy:
                lo, hi = max(a0, b0), min(a1, b1)
                if hi > lo + 1e-9:
                    out.append((lo, hi))
        return sorted(out)

    def run(self, until: float = 600.0, idle_gap: float = 3.0) -> float:
        """Advance until both engines have been idle for ``idle_gap`` seconds or ``until``
        is reached. Returns the final simulation time."""
        if self.phantoms_per_min > 0.0:
            for who in (0, 1):
                self._push(
                    self.t + self.rng.expovariate(self.phantoms_per_min / 60.0), "phantom", who
                )
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
                    fr, t0, t1, cut = cast("tuple[TxFrame, float, float, bool]", ev.data)
                    self._deliver(ev.who, fr, t0, t1, cut=cut)
                elif ev.kind == "preamble":
                    t0, t1, frame_s, container = cast(
                        "tuple[float, float, float, Container]", ev.data
                    )
                    if self.unheard is None or not self.unheard(ev.who, container, t0):
                        self._announce(ev.who, t0, t1, frame_s)
                elif ev.kind == "tx_done":
                    self.st[ev.who].engine.on_tx_done(nt)
                    self._pump(ev.who, nt)
                elif ev.kind == "report":
                    self._report(ev.who, cast("SoftFrame", ev.data), nt)
                elif ev.kind == "dropped":
                    eng = self.st[ev.who].engine
                    if isinstance(eng, GrantedEngine):
                        eng.on_tx_dropped(nt)
                        self._pump(ev.who, nt)
                elif ev.kind == "phantom":
                    self._phantom(ev.who, nt)
                    continue  # a phantom alone is no progress: an idle pair still stops
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
