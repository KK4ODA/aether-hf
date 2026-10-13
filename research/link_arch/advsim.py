"""An adversarial two-station simulator that says why every transmission was keyed.

``aether_model.link.sim.TwoStationSim`` is the model's fast pipe: every preamble is announced,
every frame is trusted, decoding is instant, the radio keys and unkeys with the audio, and
nothing outside the engine ever delays or adds a transmission. Those are exactly the places
where the field and the scenario harness find collisions, so this simulator puts them back,
each behind its own knob, and records for every keying what the engine was doing and what
both stations believed at that moment:

* **Channel.** A level per direction (asymmetric paths), the fading pipe of P9-6 shared by
  both directions (reciprocal fades), the floor's capped SNR reading (ADR-0016).
* **Detection is not decoding.** A frame is detected with a probability that falls off below
  its family's acquisition point; one detected and not decoded is *trusted* only above a
  margin over that point. A receiver deaf at a frame's start never detects it; deaf later, it
  detects a frame that fails.
* **Phantoms.** False detections arrive as a Poisson process while a station listens: an
  announcement, then a frame that never decodes, mostly untrusted, with random chips.
* **Latency.** A frame reaches the engine its decode latency after it ends — a base, an
  exponential tail, rare stalls — and announcements their own latency after the preamble.
* **The radio and the daemon.** A keying lead beyond what the engine was told
  (``PhyTiming.tx_latency_s``), holds before keying (the busy detector, the answer gap) that
  the *keying* station's engine is told about through ``on_tx_delayed`` and the other is not,
  a VOX hold after the audio (keyed, radiating nothing, deaf) and the receiver's recovery.
* **Attribution.** Every transmission is logged with the engine's trigger (a timer, a frame,
  an announcement, the application) and the chain of engine methods that keyed it, plus
  both engines' beliefs about who may transmit. Overlaps are found after the run and handed
  to :mod:`research.link_arch.classify`.

The engines are driven through the ``LinkEngine`` interface (``connect``, ``send``,
``disconnect``, ``tick``, ``on_frame``, ``on_preamble``, ``on_tx_done``, ``on_tx_delayed``,
``drain``, ``next_deadline``); prototype engines implement the same and report their own
reasons (``tx_reasons``) and beliefs (``belief()``).
"""

from __future__ import annotations

import hashlib
import heapq
import math
import random
import sys
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from aether_model.link.engine import Deliver, Event, LinkEngine, Transmit
from aether_model.link.fading import FadingPipe
from aether_model.link.phy import Container, PhyTiming, TxFrame
from aether_model.link.sim import SimFrame

# ── configuration ─────────────────────────────────────────────────────


@dataclass
class StationCfg:
    """What one station's radio, computer and receiver do to the protocol."""

    lead_jitter_s: float = 0.0
    """Keying lead beyond the engine's ``tx_latency_s``, drawn uniformly in [0, this] per
    keying: CAT keying latency, a slow card. The engine is not told."""
    gate_prob: float = 0.0
    """Probability that the daemon holds a transmission before keying (busy hold, identifier
    wait); the hold is drawn from :attr:`gate_s` and the engine *is* told (``on_tx_delayed``)."""
    gate_s: tuple[float, float] = (0.0, 0.0)
    answer_gap_s: float = 0.0
    """Nothing is keyed until this long after the end of the last frame heard (ADR-0036):
    the keying station's engine is told; the other station's is not."""
    vox_hold_s: float = 0.0
    """The transmitter stays keyed this long after the audio, radiating nothing."""
    rx_recovery_s: float = 0.0
    """The receiver hears nothing for this long after unkeying."""
    key_lead_s: float = 0.1
    """PTT is asserted this long before the first radiated sample (for the keyed interval)."""
    decode_latency_s: float = 0.15
    """A frame is reported this long after its last sample, at the least."""
    latency_tail_s: float = 0.0
    """Mean of an exponential extra decode latency."""
    floor_latency_s: float = 0.0
    """Extra latency for a tone-floor frame (the streaming receiver settles a floor candidate
    18 symbols on, ADR-0009 §8)."""
    stall_prob: float = 0.0
    """Probability per frame that the receiver stalls before reporting it."""
    stall_s: tuple[float, float] = (1.0, 4.0)
    announce_latency_s: float = 0.0
    """Extra latency of an announcement beyond the family's own detection delay."""
    phantom_per_min: float = 0.0
    """False detections a minute of listening."""
    phantom_trusted: float = 0.05
    """Fraction of phantoms the PHY trusts (ADR-0020's flag, read at its false-trust rate)."""
    phantom_floor_frac: float = 0.3
    """Fraction of phantoms that look like tone-floor frames."""
    carrier_sense: bool = False
    """The daemon defers keying while it hears the other station radiating (told to the
    engine as a hold). Off: the protocol alone decides."""


@dataclass
class ChannelCfg:
    snr_db: float = 10.0
    """Level of the path from station 0 to station 1, 3 kHz reference."""
    asym_db: float = 0.0
    """Station 0 hears station 1 this much worse than station 1 hears station 0."""
    fading: FadingPipe | None = None
    det_offset_db: tuple[float, float] = (-4.0, -2.0)
    """Detection's 50 % point relative to the family's control-frame threshold (ordinary,
    floor): acquisition reaches below decoding."""
    det_steep: float = 1.5
    trust_offset_db: float = 3.0
    """A detected frame that fails is trusted when its reading is this far above the 50 %
    detection point."""
    floor_cap_db: float | None = None
    prop_s: float = 0.01
    outage: tuple[float, float] | None = None
    """Neither station hears the other in this interval (a dead band)."""


# ── records ───────────────────────────────────────────────────────────


@dataclass
class TxRec:
    who: int
    t_req: float
    t_key: float
    rad: tuple[float, float]
    keyed: tuple[float, float]
    kinds: list[str]
    reason: str
    trigger: str
    belief_self: dict[str, Any]
    belief_other: dict[str, Any]
    hold_s: float
    lead_extra_s: float
    anchor: float | None = None
    """For an answer: the end of what it answers (in the receiver's clock)."""


@dataclass(frozen=True)
class FrameCtx:
    """One frame on the air, as a scenario's adversary sees it."""

    rx: int
    tx: int
    tx_n: int
    """Which of the sender's transmissions this is (0 = its first)."""
    kinds: list[str]
    """Every frame of the transmission, by kind."""
    i: int
    kind: str
    t0: float


@dataclass
class _Ev:
    t: float
    seq: int
    kind: str
    who: int
    data: Any = None

    def __lt__(self, other: _Ev) -> bool:
        return (self.t, self.seq) < (other.t, other.seq)


@dataclass
class _St:
    engine: Any
    cfg: StationCfg
    rng: random.Random
    delivered: bytearray = field(default_factory=bytearray)
    events: list[tuple[float, str]] = field(default_factory=list)
    tx_free_at: float = 0.0
    """When the station's last queued transmission stops radiating."""
    deaf: list[tuple[float, float]] = field(default_factory=list)
    radiating: list[tuple[float, float]] = field(default_factory=list)
    heard_end: float = float("-inf")
    last_delivery: float = 0.0
    reasons: list[tuple[str, str]] = field(default_factory=list)
    growth: list[tuple[float, int]] = field(default_factory=list)


def frame_kind(frame: TxFrame) -> str:
    if frame.container is Container.DATA:
        if not frame.payload:
            return "data"
        names = ["data", "connect_req", "connect_ack", "beacon", "probe", "probe_ack", "dgram"]
        k = frame.payload[0] >> 5
        return names[k] if k < len(names) else f"data{k}"
    if not frame.payload:
        return "control"
    names = ["ack", "poll", "turn", "disc", "disc_ack", "k5", "k6", "k7", "k8"]
    k = frame.payload[0] >> 4
    return names[k] if k < len(names) else f"ctl{k}"


# ── tracing the model's engine ────────────────────────────────────────

_ENTRY = (
    "tick",
    "on_frame",
    "on_preamble",
    "on_tx_done",
    "on_tx_delayed",
    "connect",
    "send",
    "disconnect",
    "abort",
    "_expire",
)


def instrument(engine: LinkEngine) -> None:
    """Make the model's engine say why it keys: the entry point and timer that set it going,
    and the chain of its own methods down to ``_transmit``."""
    ctx: list[str] = []
    reasons: list[tuple[str, str, float | None]] = []
    last_end: list[float | None] = [None]
    engine.tx_reasons = reasons  # type: ignore[attr-defined]
    for name in _ENTRY:
        orig = getattr(engine, name)

        def wrapper(*a: Any, _orig: Callable[..., Any] = orig, _n: str = name, **k: Any) -> Any:
            ctx.append(f"timer:{a[0]}" if _n == "_expire" else _n)
            if _n == "on_frame":
                last_end[0] = a[0].t_end
            try:
                return _orig(*a, **k)
            finally:
                ctx.pop()

        setattr(engine, name, wrapper)
    orig_tx = engine._transmit
    orig_finish = engine._finish_burst
    burst_end: list[float | None] = [None]

    def finish() -> Any:
        # the burst being answered is cleared just before its answer is keyed: keep its end
        burst = engine._burst
        burst_end[0] = max((r.frame.t_end for r in burst), default=None)
        return orig_finish()

    engine._finish_burst = finish  # type: ignore[method-assign]

    def transmit(frames: list[TxFrame], _orig: Callable[..., Any] = orig_tx) -> Any:
        chain: list[str] = []
        f = sys._getframe(1)
        while f is not None and len(chain) < 10:
            if f.f_code.co_filename.endswith("link/engine.py"):
                chain.append(f.f_code.co_name)
            f = f.f_back
        trig = "/".join(ctx)
        # what the transmission answers: the burst an acknowledgement closes, else the last
        # frame the engine was handed (a poll, a TURN, a DISC, a call)
        anchor = burst_end[0] if "timer:ack" in trig and burst_end[0] is not None else last_end[0]
        burst_end[0] = None
        reasons.append((trig, ">".join(reversed(chain)), anchor))
        return _orig(frames)

    engine._transmit = transmit  # type: ignore[method-assign]


def model_belief(engine: LinkEngine) -> dict[str, Any]:
    """What the model's engine believes about the channel: its role is its claim to send on
    its own initiative (ISS), the IRS only answers — in principle."""
    state = engine.state.value
    role = engine.role.value
    up = state in ("connected", "disconnecting")
    return {
        "state": state,
        "role": role,
        "waiting": engine._waiting_for,
        "holder": up and role == "iss",
    }


# ── the simulator ─────────────────────────────────────────────────────


def _u(*parts: object) -> float:
    """A uniform draw fixed by its arguments: the same frame at the same time gets the same
    draw in every arm of a comparison (common random numbers)."""
    h = hashlib.blake2b(repr(parts).encode(), digest_size=8).digest()
    return (int.from_bytes(h, "big") + 0.5) / 2.0**64


class AdvSim:
    """Two engines over an adversarial half-duplex channel."""

    def __init__(
        self,
        a: Any,
        b: Any,
        *,
        channel: ChannelCfg,
        cfg_a: StationCfg | None = None,
        cfg_b: StationCfg | None = None,
        seed: int = 0,
        belief: Callable[[Any], dict[str, Any]] | None = None,
        key_limit_s: float | None = None,
    ) -> None:
        self.seed = seed
        self.ch = channel
        self.st = [
            _St(a, cfg_a or StationCfg(), random.Random(seed * 7 + 1)),
            _St(b, cfg_b or StationCfg(), random.Random(seed * 7 + 2)),
        ]
        self._q: list[_Ev] = []
        self._seq = 0
        self.t = 0.0
        self.txs: list[TxRec] = []
        self.belief = belief or model_belief
        self.key_limit_s = key_limit_s
        self.timeline: list[tuple[float, bool, bool]] = []
        """(time, station 0 claims the turn, station 1 claims it) after every engine call."""
        self.app_hooks: list[Callable[[AdvSim, int, float], None]] = []
        """Called after a station's delivered stream grows: (sim, who, now)."""
        self.phantoms = [0, 0]
        self.dropped = [0, 0]
        self._tx_count = [0, 0]
        self.scenario: Callable[[FrameCtx], object] | None = None
        """A deterministic adversary: told about every frame put on the air, it may make the
        other station miss it (``"unheard"``), detect it and fail to decode it
        (``"undecodable"``), or decode it late (``("late", seconds)``)."""
        self.snr_schedule: Callable[[float], float] | None = None
        """The path's level against time, overriding ``ChannelCfg.snr_db``."""
        """Transmissions the keying gate refused to start late (the forfeit rule)."""
        self.log: list[tuple[float, int, str]] = []
        """(time, station, what) for every detection and delivery: for traces."""
        for who in (0, 1):
            self._next_phantom(who, 0.0)

    # ── bookkeeping ──────────────────────────────────────────────────

    def _push(self, t: float, kind: str, who: int, data: Any = None) -> None:
        heapq.heappush(self._q, _Ev(t, self._seq, kind, who, data))
        self._seq += 1

    def timing(self, who: int) -> PhyTiming:
        return self.st[who].engine.timing  # type: ignore[no-any-return]

    def is_deaf(self, who: int, t0: float, t1: float) -> bool:
        return any(a < t1 - 1e-9 and t0 + 1e-9 < b for a, b in self.st[who].deaf)

    def radiating_at(self, who: int, t: float) -> tuple[float, float] | None:
        for a, b in self.st[who].radiating:
            if a <= t < b:
                return (a, b)
        return None

    def _note_beliefs(self) -> None:
        b0 = self.belief(self.st[0].engine)
        b1 = self.belief(self.st[1].engine)
        self.timeline.append((self.t, bool(b0.get("holder")), bool(b1.get("holder"))))

    def schedule_app(self, t: float, who: int, fn: Callable[[], None]) -> None:
        self._push(t, "app", who, fn)

    # ── engine calls ─────────────────────────────────────────────────

    def _pump(self, who: int, at: float) -> None:
        st = self.st[who]
        reasons: list[tuple[str, str, float | None]] = getattr(st.engine, "tx_reasons", [])
        for act in st.engine.drain():
            if isinstance(act, Transmit):
                trig, why, anchor = reasons.pop(0) if reasons else ("?", "?", None)
                self._launch(who, at, act, trig, why, anchor)
            elif isinstance(act, Deliver):
                st.delivered += act.data
                st.growth.append((at, len(st.delivered)))
                for hook in self.app_hooks:
                    hook(self, who, at)
            elif isinstance(act, Event):
                st.events.append((at, f"{act.name}:{act.detail}"))
        self._note_beliefs()

    def _hold(self, who: int, at: float, frames: list[TxFrame]) -> float:
        """How long the daemon holds a transmission before keying it."""
        st = self.st[who]
        c = st.cfg
        hold = 0.0
        if c.answer_gap_s > 0.0:
            hold = max(hold, st.heard_end + c.answer_gap_s - at)
        if c.gate_prob > 0.0 and st.rng.random() < c.gate_prob:
            hold += st.rng.uniform(*c.gate_s)
        if c.carrier_sense:
            t = at + hold
            for _ in range(8):
                other = self.radiating_at(1 - who, t)
                if other is None or self.is_deaf(who, other[0], other[0] + 0.05):
                    break
                t = other[1] + 0.1
            hold = t - at
        return max(0.0, hold)

    def _launch(
        self,
        who: int,
        at: float,
        tx: Transmit,
        trigger: str,
        reason: str,
        anchor: float | None = None,
    ) -> None:
        st = self.st[who]
        timing = self.timing(who)
        hold = self._hold(who, at, tx.frames)
        if hold > 0.0:
            st.engine.on_tx_delayed(hold)
        t_key = max(at + hold, st.tx_free_at)
        not_before = getattr(tx, "not_before", None)
        if not_before is not None:
            # an answer decided ahead of its slot is keyed its latency before the slot starts
            t_key = max(t_key, not_before - timing.tx_latency_s)
        not_after = getattr(tx, "not_after", None)
        if not_after is not None and t_key + timing.tx_latency_s > not_after + 1e-9:
            # the keying gate: a transmission that cannot start in its slot is not sent
            self.dropped[who] += 1
            dropped = getattr(st.engine, "on_tx_dropped", None)
            if dropped is not None:
                dropped(at)
            return
        extra = st.rng.uniform(0.0, st.cfg.lead_jitter_s) if st.cfg.lead_jitter_s else 0.0
        t = t_key + timing.tx_latency_s + extra
        start = t
        key_up = t + self.key_limit_s if self.key_limit_s is not None else math.inf
        label = getattr(st.engine, "frame_kind", frame_kind)
        kinds = [label(f) for f in tx.frames]
        n = self._tx_count[who]
        self._tx_count[who] += 1
        for i, frame in enumerate(tx.frames):
            dur = timing.frame_s(frame)
            if t >= key_up - 1e-9:
                kinds = kinds[:i]
                break
            cut = t + dur > key_up + 1e-9
            ctx = FrameCtx(rx=1 - who, tx=who, tx_n=n, kinds=kinds, i=i, kind=kinds[i], t0=t)
            self._radiate(who, frame, t, t + dur, cut, ctx)
            t += dur
        end = t
        st.tx_free_at = end
        st.radiating.append((start, end))
        keyed = (start - st.cfg.key_lead_s, end + st.cfg.vox_hold_s)
        st.deaf.append((keyed[0], keyed[1] + st.cfg.rx_recovery_s))
        self._push(end, "tx_done", who)
        self.txs.append(
            TxRec(
                who=who,
                t_req=at,
                t_key=t_key,
                rad=(start, end),
                keyed=keyed,
                kinds=kinds,
                reason=reason,
                trigger=trigger,
                belief_self=self.belief(st.engine),
                belief_other=self.belief(self.st[1 - who].engine),
                hold_s=hold,
                lead_extra_s=extra,
                anchor=anchor,
            )
        )

    # ── the channel ──────────────────────────────────────────────────

    def _family(self, who: int, frame: TxFrame) -> bool:
        timing = self.timing(who)
        if frame.container is Container.CONTROL:
            return frame.floor
        return timing.is_floor(frame.mode)

    def _threshold(self, who: int, frame: TxFrame) -> float:
        timing = self.timing(who)
        if frame.container is Container.CONTROL:
            table = timing.control_threshold_db or {False: -5.1, True: -19.5}
            return table[frame.floor]
        return (timing.mode_threshold_db or {}).get(frame.mode, 0.0)

    def _det50(self, who: int, floor: bool) -> float:
        timing = self.timing(who)
        table = timing.control_threshold_db or {False: -5.1, True: -19.5}
        return table[floor] + self.ch.det_offset_db[int(floor)]

    def _radiate(
        self,
        tx: int,
        frame: TxFrame,
        t0: float,
        t1: float,
        cut: bool,
        ctx: FrameCtx | None = None,
    ) -> None:
        """Schedule what the other station makes of one frame: whether its preamble is
        detected (decided when the preamble is in), then what the frame decodes to (at its
        end, when the receiver's own keying may have cut it)."""
        rx = 1 - tx
        timing = self.timing(tx)
        floor = self._family(tx, frame)
        sof = timing.preamble_detect_s_for(floor) or 0.0
        at = t0 + self.ch.prop_s + sof + self.st[rx].cfg.announce_latency_s
        fate = self.scenario(ctx) if self.scenario is not None and ctx is not None else None
        if fate == "unheard":
            return
        self._push(max(at, self.t), "detect", rx, (tx, frame, t0, t1, cut, floor, sof, fate))

    def _detect(
        self,
        rx: int,
        tx: int,
        frame: TxFrame,
        t0: float,
        t1: float,
        cut: bool,
        floor: bool,
        sof: float,
        fate: object = None,
    ) -> bool:
        ch = self.ch
        p = ch.prop_s
        if self.is_deaf(rx, t0 + p, t0 + p + max(sof, 0.05)):
            return False  # deaf at the start: never detected
        if ch.outage is not None and ch.outage[0] < t1 and t0 < ch.outage[1]:
            return False
        base = self.snr_schedule(t0) if self.snr_schedule is not None else ch.snr_db
        level = base - (ch.asym_db if rx == 0 else 0.0)
        if ch.fading is not None:
            reported, decode_at = ch.fading.judge(frame, level, t0, t1)
        else:
            reported = decode_at = level
        det50 = self._det50(tx, floor)
        p_det = 1.0 / (1.0 + math.exp(-ch.det_steep * (reported - det50)))
        if _u(self.seed, "det", tx, round(t0, 3)) > p_det:
            return False
        eng = self.st[rx].engine
        eng.tick(self.t)
        eng.on_preamble(t0 + p, self.t, t1 - t0)
        self._pump(rx, self.t)
        self._push(t1 + p, "finish", rx, (tx, frame, t0, t1, cut, floor, reported, decode_at, fate))
        return True

    def _finish(
        self,
        rx: int,
        tx: int,
        frame: TxFrame,
        t0: float,
        t1: float,
        cut: bool,
        floor: bool,
        reported: float,
        decode_at: float,
        fate: object = None,
    ) -> None:
        st = self.st[rx]
        ch = self.ch
        p = ch.prop_s
        cut = cut or self.is_deaf(rx, t0 + p, t1 + p)
        det50 = self._det50(tx, floor)
        trusted = reported >= det50 + ch.trust_offset_db
        shown = reported
        if floor and ch.floor_cap_db is not None:
            shown = min(reported, ch.floor_cap_db)
        draw = 2.0 if cut or fate == "undecodable" else _u(self.seed, "dec", tx, round(t0, 3))
        counts = getattr(getattr(self.st[tx].engine, "cfg", None), "stated_answers", True)
        follows = None
        if frame.container is Container.DATA and (not floor or counts):
            follows = frame.follows
        sf = SimFrame(
            container=frame.container,
            mode=frame.mode,
            rv=frame.rv,
            snr_db=shown,
            t_start=t0 + p,
            t_end=t1 + p,
            payload=frame.payload,
            _draw=draw,
            floor=floor,
            _decode_snr_db=decode_at,
            _threshold=self._threshold(tx, frame),
            trusted=trusted or cut,
            follows=follows,
        )
        c = st.cfg
        lat = c.decode_latency_s + (c.floor_latency_s if floor else 0.0)
        if c.latency_tail_s > 0.0:
            lat += st.rng.expovariate(1.0 / c.latency_tail_s)
        if c.stall_prob > 0.0 and st.rng.random() < c.stall_prob:
            lat += st.rng.uniform(*c.stall_s)
        if isinstance(fate, tuple) and fate[0] == "late":
            lat += float(fate[1])
        at = max(t1 + p + lat, st.last_delivery)
        st.last_delivery = at
        self._push(at, "deliver", rx, sf)

    # ── phantoms ─────────────────────────────────────────────────────

    def _next_phantom(self, who: int, after: float) -> None:
        rate = self.st[who].cfg.phantom_per_min / 60.0
        if rate <= 0.0:
            return
        r = random.Random(f"{self.seed}:ph:{who}:{after:.3f}")
        self._push(after + r.expovariate(rate), "phantom", who)

    def _phantom(self, who: int) -> None:
        st = self.st[who]
        c = st.cfg
        timing = self.timing(1 - who)
        r = random.Random(f"{self.seed}:phx:{who}:{self.t:.3f}")
        floor = r.random() < c.phantom_floor_frac and timing.floor_modes > 0
        modes = [m for m in (timing.data_capacity or {}) if timing.is_floor(m) == floor]
        mode = r.choice(modes) if modes else 0
        dur = timing.data_frame_s_for(mode)
        t0 = self.t
        t1 = t0 + dur
        self._next_phantom(who, t1)
        if self.is_deaf(who, t0, t1):
            return
        self.phantoms[who] += 1
        det50 = self._det50(1 - who, floor)
        sf = SimFrame(
            container=Container.DATA,
            mode=mode,
            rv=r.randrange(4),
            snr_db=det50 + r.uniform(0.0, 3.0),
            t_start=t0,
            t_end=t1,
            payload=b"",
            _draw=2.0,
            floor=floor,
            trusted=r.random() < c.phantom_trusted,
            follows=r.randrange(4),
        )
        sof = timing.preamble_detect_s_for(floor) or 0.0
        self._push(t0 + sof, "announce", who, (t0, dur))
        self._push(t1 + c.decode_latency_s, "deliver", who, sf)

    # ── the run loop ─────────────────────────────────────────────────

    def run(self, until: float = 900.0, idle_gap: float = 30.0) -> float:
        for who in (0, 1):
            self._pump(who, self.t)
        last = 0.0
        while self.t < until:
            nq = self._q[0].t if self._q else math.inf
            nds = [self.st[w].engine.next_deadline() for w in (0, 1)]
            nt = min([nq] + [d for d in nds if d is not None])
            if not math.isfinite(nt) or nt > until:
                break
            self.t = max(self.t, nt)
            progressed = False
            for who in (0, 1):
                dl = self.st[who].engine.next_deadline()
                if dl is not None and dl <= self.t + 1e-9:
                    self.st[who].engine.tick(self.t)
                    self._pump(who, self.t)
                    progressed = True
            while self._q and self._q[0].t <= self.t + 1e-9:
                ev = heapq.heappop(self._q)
                progressed = self._event(ev) or progressed
            if progressed:
                last = self.t
            elif self.t - last > idle_gap and all(d is None for d in nds):
                # only phantoms for a while, and no engine waiting on a timer: nothing more
                # will happen (a pending timer, a call's next try, is no idleness)
                break
            if self._quiet():
                break
        return self.t

    def _quiet(self) -> bool:
        """Both engines idle and nothing left to happen but phantoms."""
        for st in self.st:
            if getattr(st.engine, "state", None) is None:
                return False
            if st.engine.state.value != "idle" or st.engine.next_deadline() is not None:
                return False
        return all(ev.kind == "phantom" for ev in self._q)

    def _event(self, ev: _Ev) -> bool:
        st = self.st[ev.who]
        eng = st.engine
        if ev.kind == "detect":
            return self._detect(ev.who, *ev.data)
        if ev.kind == "finish":
            self._finish(ev.who, *ev.data)
            return False
        if ev.kind == "announce":
            t_start, frame_s = ev.data
            if self.is_deaf(ev.who, t_start, self.t):
                return False
            eng.tick(self.t)
            eng.on_preamble(t_start, self.t, frame_s)
            self._pump(ev.who, self.t)
            return True
        if ev.kind == "deliver":
            sf = ev.data
            if not sf.payload and self.is_deaf(ev.who, sf.t_start, sf.t_end):
                return False  # a phantom over the station's own keying: nothing to hear
            if sf.payload:
                st.heard_end = max(st.heard_end, sf.t_end)
            ok = sf.decode(None)[0] is not None
            self.log.append(
                (
                    self.t,
                    ev.who,
                    f"deliver {sf.container.name} m{sf.mode} "
                    f"{sf.t_start:.2f}-{sf.t_end:.2f} snr={sf.snr_db:.1f} "
                    f"{'OK' if ok else 'fail'} trusted={sf.trusted} "
                    f"f={sf.follows} {'PHANTOM' if not sf.payload else ''}",
                )
            )
            eng.tick(self.t)
            eng.on_frame(sf, self.t)
            self._pump(ev.who, self.t)
            return True
        if ev.kind == "tx_done":
            eng.on_tx_done(self.t)
            self._pump(ev.who, self.t)
            return True
        if ev.kind == "phantom":
            self._phantom(ev.who)
            return False
        if ev.kind == "app":
            eng.tick(self.t)
            ev.data()
            self._pump(ev.who, self.t)
            return True
        return False

    # ── results ──────────────────────────────────────────────────────

    def delivered(self, who: int) -> bytes:
        return bytes(self.st[who].delivered)

    def events(self, who: int) -> list[tuple[float, str]]:
        return self.st[who].events

    def dual_time(self) -> tuple[float, float]:
        """Seconds both engines claimed the turn, and seconds neither did while both were up."""
        dual = 0.0
        for (t0, a0, b0), (t1, _, _) in zip(self.timeline, self.timeline[1:], strict=False):
            if a0 and b0:
                dual += t1 - t0
        return dual, 0.0

    def overlaps(self) -> list[tuple[TxRec, TxRec, float, float]]:
        """Every pair of keyed intervals of the two stations that overlap: (the one keyed
        first, the one keyed second, keyed overlap, radiated overlap)."""
        out = []
        recs = sorted(self.txs, key=lambda r: r.keyed[0])
        for i, x in enumerate(recs):
            for y in recs[i + 1 :]:
                if y.keyed[0] >= x.keyed[1]:
                    break
                if y.who == x.who:
                    continue
                k = min(x.keyed[1], y.keyed[1]) - max(x.keyed[0], y.keyed[0])
                r = min(x.rad[1], y.rad[1]) - max(x.rad[0], y.rad[0])
                if k > 0:
                    out.append((x, y, k, max(0.0, r)))
        return out

    def airtime(self, who: int) -> float:
        return sum(b - a for a, b in self.st[who].radiating)
