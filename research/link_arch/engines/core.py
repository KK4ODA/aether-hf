"""What the two prototype engines share: frames, selective repeat, HARQ, rate observation.

Everything here is scheduling-free. The prototypes (``gs.py``: granted slots; ``fc.py``: a
fixed cycle) differ only in *when* each station may key; what a transmission carries, how
frames are numbered, acknowledged, combined and rate-controlled is this module, kept close
to the model engine's (``aether_model.link.engine``) so that the comparison is of channel
access and nothing else.

The control frame keeps the PHY's seven bytes (``frames.CONTROL_BYTES``)::

    0: kind (2) | count (3): data frames after this frame in its transmission |
       data family (1) | exchange id (2)
    1: session (6) | family bit (1) | flag (1)
    2: base     3-4: bitmap      (selective-repeat acknowledgement, as ``frames.py``)
    5: SNR of the other station's last transmission (signed dB, 0x7f unknown)
    6: rung (5): recommended for the other station's data | n (3)

``family bit``: in the master's header, the family the slave's answer must use; in the
slave's answer, the family it asks the master's headers to use (each station asks for the
family it can read). ``flag``: the header's grant is an open window / the answer asks to
leave. ``n``: the header grants that many data frames to the slave / the answer says how
many it has queued (0-7).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from enum import IntEnum
from typing import Any

from aether_model.link.engine import RV_SEQUENCE, SELF_DECODABLE_RVS, Deliver, Event, Transmit
from aether_model.link.frames import (
    CONNECT_BODY_BYTES,
    MAX_BURST,
    SNR_UNKNOWN,
    WINDOW,
    DataHeader,
    DataKind,
    data_capacity,
    decode_data,
    encode_data,
    in_window,
    seq_after,
    seq_distance,
)
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import CONTROL_THRESHOLD_DB, RateController, usable_modes

MAX_FRAMES = 7
"""A transmission's data frames are counted in three bits."""


class Kind(IntEnum):
    HDR = 0
    ANS = 1
    DISC = 2
    DISC_ACK = 3


@dataclass(frozen=True)
class Ctl:
    kind: Kind
    session: int
    xid: int = 0
    count: int = 0
    data_floor: bool = False
    fam: bool = False
    flag: bool = False
    base: int = 0
    bitmap: int = 0
    snr_db: float | None = None
    rung: int = 0
    n: int = 0

    def encode(self) -> bytes:
        snr = SNR_UNKNOWN if self.snr_db is None else max(-40, min(40, round(self.snr_db)))
        return bytes(
            [
                (int(self.kind) << 6)
                | (min(self.count, 7) << 3)
                | (int(self.data_floor) << 2)
                | (self.xid & 3),
                ((self.session & 0x3F) << 2) | (int(self.fam) << 1) | int(self.flag),
                self.base & 0xFF,
                (self.bitmap >> 8) & 0xFF,
                self.bitmap & 0xFF,
                snr & 0xFF,
                ((self.rung & 0x1F) << 3) | (min(self.n, 7) & 7),
            ]
        )

    @classmethod
    def decode(cls, b: bytes) -> Ctl:
        raw = b[5]
        snr = None if raw == SNR_UNKNOWN else float(raw - 256 if raw >= 128 else raw)
        return cls(
            kind=Kind(b[0] >> 6),
            count=(b[0] >> 3) & 7,
            data_floor=bool(b[0] & 4),
            xid=b[0] & 3,
            session=b[1] >> 2,
            fam=bool(b[1] & 2),
            flag=bool(b[1] & 1),
            base=b[2],
            bitmap=(b[3] << 8) | b[4],
            snr_db=snr,
            rung=b[6] >> 3,
            n=b[6] & 7,
        )

    def received(self, seq: int) -> bool:
        d = (seq - self.base) % 256
        if d >= WINDOW:
            return d >= 128
        return bool((self.bitmap >> d) & 1)


@dataclass
class SlotTransmit(Transmit):
    """A transmission that must start on the air no later than ``not_after`` or not at all:
    the keying gate drops it rather than send it late (the forfeit rule)."""

    not_after: float | None = None


class State(IntEnum):
    """Mirrors ``aether_model.link.engine.State`` by value for the simulators."""

    IDLE = 0
    CONNECTING = 1
    CONNECTED = 2
    DISCONNECTING = 3

    @property
    def value_name(self) -> str:  # pragma: no cover - cosmetic
        return self.name.lower()


class _StateView:
    """``engine.state.value`` reads as the model's strings ("idle", "connected", ...)."""

    def __init__(self, s: State) -> None:
        self._s = s

    @property
    def value(self) -> str:
        return self._s.name.lower()


@dataclass
class TxRec:
    seq: int
    body: bytes
    mode: int
    tx_count: int = 0
    acked: bool = False
    reencoded: int = 0


@dataclass
class RxRec:
    frame: SoftFrame
    slot: int
    payload: bytes | None = None
    seq: int | None = None
    combined: bool = False


@dataclass
class Snapshot:
    base: int
    missing: list[int]
    next_new: int


@dataclass
class Stats:
    frames_sent: int = 0
    frames_resent: int = 0
    frames_received: int = 0
    frames_failed: int = 0
    harq_rescues: int = 0
    transmissions: int = 0
    silent_slots: int = 0
    forfeits: int = 0
    dropped: int = 0
    bytes_delivered: int = 0
    bytes_acked: int = 0
    frames_reencoded: int = 0


@dataclass
class CoreConfig:
    max_frames: int = 6
    """Data frames in a transmission at most (three bits carry the count)."""
    max_burst_s: float = 26.0
    max_mode: int = 19
    ceiling: int | None = None
    initial_mode: int = 0
    capabilities: int = 0
    connect_retries: int = 8
    max_retries: int = 8
    max_combines: int = 4
    silence_step: int = 2
    link_timeout_s: float = 45.0
    link_timeout_exchanges: float = 4.0
    answer_floor_margin_db: float = 3.0
    guard_s: float = 0.05
    """Added to every gap: propagation, clock and rounding."""
    turnaround_s: float = 0.0
    """What the other station needs after a transmission before it can hear one: its VOX
    hold and receiver recovery. Added to every gap; stated at the connection (in the
    prototype, configured)."""
    decode_budget_s: tuple[float, float] = (0.25, 0.6)
    """How long after a frame ends its decode may reach the engine and still be in time,
    ordinary and floor: the streaming receiver settles a tone-floor candidate later. A
    decode later than this (plus :attr:`slack_s`) forfeits the slot it would answer."""
    slack_s: float = 0.3
    """How late a station may start inside its slot; past that it forfeits the slot."""
    rate: dict[str, Any] = field(default_factory=dict)


class Core:
    """Selective repeat with HARQ, rate observation and the control frame, both directions."""

    def __init__(self, my_call: str, timing: PhyTiming, cfg: CoreConfig, seed: int = 0) -> None:
        import random

        self.my_call = my_call.upper()
        self.timing = timing
        self.cfg = cfg
        self.rng = random.Random(seed)
        self._state = State.IDLE
        self.remote_call = ""
        self.session = 0
        self.master = False
        self.now = 0.0
        self.actions: list[Any] = []
        self.tx_reasons: list[tuple[str, str, float | None]] = []
        self.stats = Stats()
        self._deadlines: dict[str, float] = {}
        self._trigger = ""
        self.reset()

    # ── the simulators' view ──────────────────────────────────────────
    @property
    def state(self) -> _StateView:
        return _StateView(self._state)

    def drain(self) -> list[Any]:
        out, self.actions = self.actions, []
        return out

    def next_deadline(self) -> float | None:
        return min(self._deadlines.values()) if self._deadlines else None

    def _arm(self, name: str, at: float) -> None:
        self._deadlines[name] = at

    def _disarm(self, name: str) -> None:
        self._deadlines.pop(name, None)

    def on_tx_delayed(self, seconds: float) -> None:
        """Holds before keying move nothing: slots are absolute, and a transmission that
        cannot start in its slot is dropped by the gate (:meth:`on_tx_dropped`)."""

    # ── per-session state ─────────────────────────────────────────────
    def reset(self) -> None:
        self._queue = bytearray()
        self._records: dict[int, TxRec] = {}
        self._tx_base = self._tx_next = 0
        self._rx_base = 0
        self._rx_buf: dict[int, bytes] = {}
        self._max_seen: int | None = None
        self._harq: dict[int, tuple[object, int, int]] = {}
        self._snaps: list[Snapshot] = []
        self._rx: list[RxRec] = []
        self._rx_first: float | None = None
        self.rate = self._rate_controller()
        self.send_rung = self.cfg.initial_mode
        self.peer_report_db: float | None = None
        """How the other station says it hears this one (its control frames' SNR)."""
        self.heard_db: float | None = None
        """How this station heard the other's last transmission."""
        self._last_heard = 0.0
        self._retries = 0
        self._disc_requested = False
        self._disc_tries = 0
        self.top_acked_mode: int | None = None

    def _rate_controller(self) -> RateController:
        t = self.timing
        thresholds = t.mode_threshold_db or {}
        payload = t.data_capacity or {m: 0 for m in thresholds}
        frame_s = {m: t.data_frame_s_for(m) for m in thresholds}
        return RateController(
            thresholds=dict(thresholds),
            modes=usable_modes(thresholds, payload, frame_s),
            floor_modes=t.floor_modes,
            floor_margin_db=t.floor_margin_db,
            **self.cfg.rate,
        )

    # ── timing helpers ────────────────────────────────────────────────
    def ctl_s(self, floor: bool) -> float:
        return self.timing.control_frame_s_for(floor)

    def frame_s(self, floor: bool) -> float:
        t = self.timing
        return t.floor_data_frame_s if floor and t.floor_data_frame_s else t.data_frame_s

    def gap_s(self, floor: bool = False) -> float:
        """From the end of a transmission to the start of the slot it grants, by the family
        of its last frame: the other engine hears that frame end a decode budget later and
        keys a keying lead after that."""
        return (
            self.cfg.decode_budget_s[int(floor)]
            + self.timing.tx_latency_s
            + self.cfg.guard_s
            + self.cfg.turnaround_s
        )

    def heard_by(self, floor: bool) -> float:
        """How long after a frame of this family ends this engine has surely decoded it."""
        return self.cfg.decode_budget_s[int(floor)] + self.cfg.guard_s

    @staticmethod
    def frame_kind(frame: TxFrame) -> str:
        if frame.container is Container.DATA:
            k = frame.payload[0] >> 5 if frame.payload else 0
            return {0: "data", 1: "connect_req", 2: "connect_ack"}.get(k, f"data{k}")
        return Kind(frame.payload[0] >> 6).name.lower() if frame.payload else "ctl"

    def cap(self) -> int:
        return (
            self.cfg.max_mode
            if self.cfg.ceiling is None
            else min(self.cfg.max_mode, self.cfg.ceiling)
        )

    def is_floor(self, mode: int) -> bool:
        return self.timing.is_floor(mode)

    def ctl_threshold(self) -> float:
        return (self.timing.control_threshold_db or CONTROL_THRESHOLD_DB)[False]

    def wants_floor(self) -> bool:
        """Whether this station needs the other's control frames on the floor: it hears the
        other under the ordinary control frame's threshold plus a margin, or not yet."""
        if self.timing.floor_modes <= 0:
            return False
        return self.heard_db is None or (
            self.heard_db < self.ctl_threshold() + self.cfg.answer_floor_margin_db
        )

    # ── outgoing data ─────────────────────────────────────────────────
    def send_queue(self, data: bytes) -> None:
        self._queue += data

    def unacked(self) -> list[int]:
        seqs = [s for s, r in self._records.items() if not r.acked and (r.tx_count or r.reencoded)]
        return sorted(seqs, key=lambda s: seq_distance(s, self._tx_base))

    def has_work(self) -> bool:
        return bool(self._queue) or bool(self.unacked())

    def queued_frames(self) -> int:
        """Frames' worth of what is waiting, at the rung this station sends at."""
        cap = data_capacity(self.timing.capacity(min(self.send_rung, self.cap())))
        return len(self.unacked()) + math.ceil(len(self._queue) / max(1, cap))

    def _fits(self, n: int, mode: int) -> bool:
        cap = data_capacity(self.timing.capacity(mode))
        return n == cap or n <= cap - 2

    def _reencode(self, rec: TxRec, rung: int) -> None:
        if rec.tx_count < self.cfg.max_combines or rung >= rec.mode:
            return
        for m in sorted(self.timing.data_capacity or {}):
            if rung <= m < rec.mode and self._fits(len(rec.body), m):
                rec.mode = m
                rec.reencoded += rec.tx_count
                rec.tx_count = 0
                self.stats.frames_reencoded += 1
                return

    def build_data(self, room_s: float) -> list[TxFrame]:
        """Data frames for a transmission: the oldest unacknowledged first, then new ones, of
        one family, as many as ``room_s`` and :attr:`CoreConfig.max_frames` allow."""
        rung = min(self.send_rung, self.cap())
        unacked = self.unacked()
        for s in unacked:
            self._reencode(self._records[s], rung)
        family = self.is_floor(rung)
        if unacked:
            family = self.is_floor(self._records[unacked[0]].mode)
        seqs = [s for s in unacked if self.is_floor(self._records[s].mode) == family]
        fs = self.frame_s(family)
        room = min(self.cfg.max_frames, MAX_FRAMES, max(0, int(room_s / fs + 1e-9)))
        seqs = seqs[:room]
        cap = data_capacity(self.timing.capacity(rung))
        while (
            family == self.is_floor(rung)
            and len(seqs) < room
            and seq_distance(self._tx_next, self._tx_base) < WINDOW
            and self._queue
        ):
            take = min(cap, len(self._queue))
            if take == cap - 1:
                take -= 1
            body = bytes(self._queue[:take])
            del self._queue[:take]
            rec = TxRec(self._tx_next, body, rung)
            self._records[rec.seq] = rec
            self._tx_next = seq_after(self._tx_next)
            seqs.append(rec.seq)
        frames = []
        for i, s in enumerate(seqs):
            rec = self._records[s]
            if rec.tx_count or rec.reencoded:
                self.stats.frames_resent += 1
            rec.tx_count += 1
            payload = encode_data(
                DataHeader(DataKind.DATA, rec.seq, self.session),
                rec.body,
                self.timing.capacity(rec.mode),
            )
            rv = RV_SEQUENCE[(rec.tx_count - 1) % len(RV_SEQUENCE)]
            follows = min(3, len(seqs) - 1 - i)
            frames.append(TxFrame(Container.DATA, payload, mode=rec.mode, rv=rv, follows=follows))
        self.stats.frames_sent += len(frames)
        return frames

    def take_ack(self, ctl: Ctl) -> bool:
        """An acknowledgement of this station's data. True when something new was."""
        progress = False
        for s, rec in list(self._records.items()):
            if not rec.acked and (rec.tx_count or rec.reencoded) and ctl.received(s):
                rec.acked = True
                progress = True
                self.stats.bytes_acked += len(rec.body)
                self.top_acked_mode = max(self.top_acked_mode or 0, rec.mode)
        while self._tx_base != self._tx_next and self._records[self._tx_base].acked:
            del self._records[self._tx_base]
            self._tx_base = seq_after(self._tx_base)
        self.send_rung = max(0, min(self.cap(), ctl.rung))
        if ctl.snr_db is not None:
            self.peer_report_db = ctl.snr_db
        return progress

    def back_off(self) -> None:
        modes = self.rate.modes
        current = min(self.send_rung, self.cap())
        below = [m for m in modes if m <= current]
        i = modes.index(below[-1]) if below else 0
        self.send_rung = modes[max(0, i - self.cfg.silence_step)]

    # ── incoming data ─────────────────────────────────────────────────
    def begin_rx(self) -> None:
        self._rx = []
        self._rx_first = None

    def take_data(self, frame: SoftFrame, first_data_start: float | None = None) -> bool:
        """A data frame of the other station's current transmission. True if it decoded."""
        if self._rx_first is None:
            self._rx_first = first_data_start if first_data_start is not None else frame.t_start
        fs = max(1e-6, frame.t_end - frame.t_start)
        slot = max(0, round((frame.t_start - self._rx_first) / fs))
        rec = RxRec(frame, slot)
        self._rx.append(rec)
        payload, buffer = frame.decode(None)
        if payload is not None:
            return self._accept(rec, payload)
        guess = self._infer(slot)
        rec.seq = guess
        if guess is None or not in_window(guess, self._rx_base):
            self.stats.frames_failed += 1
            return False
        if guess in self._harq and self._harq[guess][2] != frame.mode:
            del self._harq[guess]
        if guess in self._harq:
            prev, combines, _ = self._harq[guess]
            rec.combined = True
            combined, merged = frame.decode(prev)
            if combined is not None:
                self.stats.harq_rescues += 1
                return self._accept(rec, combined)
            self._harq[guess] = (
                (buffer, 0, frame.mode)
                if combines + 1 >= self.cfg.max_combines
                else (merged, combines + 1, frame.mode)
            )
        else:
            self._harq[guess] = (buffer, 0, frame.mode)
        self.stats.frames_failed += 1
        return False

    def _infer(self, slot: int) -> int | None:
        anchors = [(r.slot, r.seq) for r in self._rx if r.payload is not None and r.seq is not None]
        snaps = self._snaps or [Snapshot(self._rx_base, [], self._rx_base)]
        for snap in snaps:
            expected = list(snap.missing)
            s = snap.next_new
            while len(expected) < MAX_BURST:
                expected.append(s)
                s = seq_after(s)
            offsets: set[int] = set()
            ok = True
            for a_slot, a_seq in anchors:
                if a_seq not in expected:
                    ok = False
                    break
                offsets.add(expected.index(a_seq) - a_slot)
            if not ok or len(offsets) > 1:
                continue
            idx = slot + (offsets.pop() if offsets else 0)
            if 0 <= idx < len(expected):
                return expected[idx]
        return None

    def _accept(self, rec: RxRec, payload: bytes) -> bool:
        try:
            header, body = decode_data(payload)
        except ValueError:
            return False
        if header.kind is not DataKind.DATA or header.session != self.session:
            return False
        rec.payload = payload
        rec.seq = header.seq
        self.stats.frames_received += 1
        if in_window(header.seq, self._rx_base):
            if self._max_seen is None or seq_distance(header.seq, self._rx_base) > seq_distance(
                self._max_seen, self._rx_base
            ):
                self._max_seen = header.seq
            self._rx_buf.setdefault(header.seq, body)
            self._harq.pop(header.seq, None)
            while self._rx_base in self._rx_buf:
                data = self._rx_buf.pop(self._rx_base)
                self._harq.pop(self._rx_base, None)
                self._rx_base = seq_after(self._rx_base)
                self.stats.bytes_delivered += len(data)
                if data:
                    self.actions.append(Deliver(data))
            if self._max_seen is not None and seq_distance(self._max_seen, self._rx_base) >= WINDOW:
                self._max_seen = None
        return True

    def observe_rx(self) -> float | None:
        """What the other station's last transmission's data said of the path, to the rate
        controller (as ``LinkEngine._observe_burst``). Returns the SNR measured on it."""
        burst = self._rx
        ok = sum(1 for r in burst if r.payload is not None)
        failed = sum(
            1
            for r in burst
            if r.payload is None
            and r.frame.trusted
            and (r.frame.rv in SELF_DECODABLE_RVS or r.combined)
        )
        snrs = [r.frame.snr_db for r in burst if r.payload is not None or r.frame.trusted]
        snr = sum(snrs) / len(snrs) if snrs else None
        if burst:
            modes = [r.frame.mode for r in burst]
            self.rate.observe(snr, ok, failed, max(set(modes), key=modes.count))
        return snr

    def window(self) -> tuple[int, list[int], int]:
        bitmap = 0
        missing: list[int] = []
        limit = 0 if self._max_seen is None else seq_distance(self._max_seen, self._rx_base) + 1
        for i in range(WINDOW):
            s = seq_after(self._rx_base, i)
            if s in self._rx_buf:
                bitmap |= 1 << i
            elif i < limit:
                missing.append(s)
        return bitmap, missing, seq_after(self._rx_base, limit)

    def ack_fields(self) -> dict[str, Any]:
        """What a control frame says of what this station has received, and remembers it
        said (the HARQ inference's snapshots)."""
        bitmap, missing, next_new = self.window()
        self._snaps = [Snapshot(self._rx_base, missing, next_new), *self._snaps][:2]
        return {"base": self._rx_base, "bitmap": bitmap}

    # ── connection frames ─────────────────────────────────────────────
    def robust_mode(self, floor: bool) -> int:
        need = CONNECT_BODY_BYTES + 5
        caps = self.timing.data_capacity or {}
        for m in sorted(caps):
            if self.timing.is_floor(m) == floor and caps[m] >= need:
                return m
        if floor:
            return self.robust_mode(False)
        raise ValueError("no mode carries a connect frame")

    # ── keying ────────────────────────────────────────────────────────
    def transmit(
        self,
        frames: list[TxFrame],
        reason: str,
        anchor: float | None = None,
        not_after: float | None = None,
    ) -> float:
        """Key now; returns when the transmission ends on the air (as far as the engine
        knows: its keying latency plus the frames)."""
        dur = sum(self.timing.frame_s(f) for f in frames)
        self.actions.append(SlotTransmit(frames, dur, not_after=not_after))
        self.tx_reasons.append((self._trigger, reason, anchor))
        self.stats.transmissions += 1
        return self.now + self.timing.tx_latency_s + dur

    def event(self, name: str, detail: str = "") -> None:
        self.actions.append(Event(name, detail))

    def link_timeout(self) -> float:
        """Silence that ends a session: :attr:`CoreConfig.link_timeout_exchanges` of the
        longest exchange the protocol can make — two transmissions as long as the key-time
        limit allows, two floor control frames and both gaps — so that no silence the protocol
        itself makes (a lost long grant waited out) is ever taken for a dead link."""
        exchange = 2 * (self.cfg.max_burst_s + self.ctl_s(True) + self.gap_s(True)) + 2 * (
            self.cfg.slack_s
        )
        return max(self.cfg.link_timeout_s, self.cfg.link_timeout_exchanges * exchange)
