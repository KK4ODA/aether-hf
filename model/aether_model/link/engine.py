"""ARQ engine and session state machine (roadmap P2-1).

One :class:`LinkEngine` per station. It is a pure state machine driven by three inputs —
:meth:`on_frame` (a detected frame, decoded or not), :meth:`on_tx_done` and :meth:`tick`
(time) — and it emits :class:`Action` objects: frames to transmit, bytes to deliver,
events to report. No threads, no clocks of its own, no PHY: the same engine runs under
the lossy-pipe simulator, the PHY-level harness and, later, the real modem.

Protocol in one paragraph
-------------------------
A session has one *information sending station* (ISS) and one *information receiving
station* (IRS). The ISS transmits bursts of up to 16 DATA frames back to back; the IRS
answers every burst with one short ACK carrying a selective-repeat bitmap, the SNR it
measured and the mode it recommends. The ISS retransmits what the bitmap says is missing
(same codeword, next redundancy version — the IRS soft-combines) and fills the rest of the
burst with new frames at the recommended mode. A TURN control frame hands the ISS role to
the peer (sent when the peer flagged WANT_TX or BREAK in an ACK); POLL keeps an idle link
alive; DISC / DISC_ACK end it. Connection is a two-way handshake in DATA-container frames
that carry both callsigns; the caller's first burst or POLL confirms it.

HARQ without a decoded header
-----------------------------
A failed frame has no readable sequence number, so the IRS infers it: the ISS orders every
burst as *unacknowledged frames ascending, then new frames*, and the IRS knows what it has
acknowledged — so it can predict the burst from either of its last two ACKs (the ISS may
have missed the latest one) and map each frame's *slot* (from its air time) to a sequence
number; frames that did decode anchor the mapping. A wrong guess only costs a wasted
combine — the CRC decides, and a standalone decode is always tried as well.
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from enum import Enum

from aether_model.link.frames import (
    CONNECT_BODY_BYTES,
    MAX_BURST,
    WINDOW,
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
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
from aether_model.link.rate import RateController

# ── configuration, actions, states ────────────────────────────────────


@dataclass
class LinkConfig:
    burst_frames: int = 6
    """Frames per burst (≤ 16). Longer bursts amortise the ACK turnaround."""
    max_retries: int = 8
    """Consecutive unanswered bursts / polls before the link is declared dead."""
    connect_retries: int = 8
    turn_retries: int = 3
    disc_retries: int = 3
    keepalive_s: float = 10.0
    """Idle ISS polls the IRS this often."""
    link_timeout_s: float = 45.0
    """No valid frame from the peer for this long ends the session."""
    ack_margin_s: float = 0.4
    """Slack added to every wait for a peer response."""
    burst_gap_s: float = 0.2
    """Silence after a DATA frame that marks the end of a burst (the IRS then ACKs)."""
    initial_mode: int = 0
    max_mode: int = 13
    bursts_before_turn: int = 3
    """With a WANT_TX peer, the ISS hands over after this many bursts of its own."""
    max_combines: int = 4
    """HARQ buffers are reset after this many failed combines (guards a wrong inference)."""
    capabilities: int = 0
    """Capability bits offered in the connect handshake. What they mean is the caller's
    business; the link layer carries them and reports what the peer offered. Bit 0 is stream
    compression (deflate, RFC 1951) — see ``docs/spec/air-interface.md``."""


class State(Enum):
    IDLE = "idle"
    CONNECTING = "connecting"
    CONNECTED = "connected"
    DISCONNECTING = "disconnecting"


class Role(Enum):
    NONE = "none"
    ISS = "iss"
    IRS = "irs"


@dataclass
class Transmit:
    frames: list[TxFrame]
    duration_s: float


@dataclass
class Deliver:
    data: bytes


@dataclass
class Event:
    name: str
    detail: str = ""


Action = Transmit | Deliver | Event


@dataclass
class _TxRecord:
    seq: int
    kind: DataKind
    body: bytes
    mode: int
    tx_count: int = 0
    acked: bool = False


@dataclass
class _RxRecord:
    frame: SoftFrame
    slot: int
    payload: bytes | None = None
    seq: int | None = None
    """Sequence number if decoded, else the inferred guess (may be None)."""
    buffer: object = None
    """This transmission's soft-decode output, kept for HARQ combining."""


@dataclass
class _AckSnapshot:
    base: int
    missing: list[int]
    """Unreceived sequence numbers between base and the highest seen, ascending."""
    next_new: int


@dataclass
class LinkStats:
    frames_sent: int = 0
    frames_resent: int = 0
    frames_received: int = 0
    frames_failed: int = 0
    harq_rescues: int = 0
    acks_sent: int = 0
    acks_received: int = 0
    ack_timeouts: int = 0
    bursts: int = 0
    turns: int = 0
    bytes_delivered: int = 0


# ── the engine ────────────────────────────────────────────────────────


class LinkEngine:
    def __init__(
        self,
        my_call: str,
        timing: PhyTiming,
        config: LinkConfig | None = None,
        seed: int | None = None,
    ) -> None:
        self.my_call = my_call.upper()
        self.timing = timing
        self.cfg = config or LinkConfig()
        self.rng = random.Random(seed)
        self.state = State.IDLE
        self.role = Role.NONE
        self.remote_call = ""
        self.session = 0
        self.now = 0.0
        self.actions: list[Action] = []
        self.stats = LinkStats()
        self.rate = RateController()
        self._deadlines: dict[str, float] = {}
        self._tx_busy_until = 0.0
        self._last_peer_frame = 0.0
        # sending side
        self._tx_queue = bytearray()
        self._records: dict[int, _TxRecord] = {}
        self._tx_base = 0
        self._tx_next = 0
        self._burst_seqs: list[int] = []
        self._retries = 0
        self._bursts_since_turn = 0
        self._peer_wants_tx = False
        self._peer_break = False
        self._recommended = self.cfg.initial_mode
        self._turn_tries = 0
        self._disc_requested = False
        self._disc_tries = 0
        self._connect_tries = 0
        self._waiting_for: str | None = None  # "ack" | "poll" | "turn" | "disc" | "connect"
        # receiving side
        self._rx_base = 0
        self._rx_buf: dict[int, bytes] = {}
        self._max_seen: int | None = None
        self._harq: dict[int, tuple[object, int]] = {}
        self._burst: list[_RxRecord] = []
        self._burst_t0: float | None = None
        self._ack_history: list[_AckSnapshot] = []
        self._ack_counter = 0
        self._break_requested = False
        self._confirmed = False
        self.peer_capabilities = 0
        """Capability bits the peer offered. Zero until a session is up, which is the safe
        reading: a station that has not said it can do something cannot be assumed to."""

    # ── public commands ───────────────────────────────────────────────

    def connect(self, remote_call: str) -> None:
        if self.state is not State.IDLE:
            raise RuntimeError("already in a session")
        self.remote_call = remote_call.upper()
        self.session = self.rng.randrange(256)
        self.state = State.CONNECTING
        self.role = Role.NONE
        self._connect_tries = 0
        self._reset_transfer_state()
        self._send_connect(DataKind.CONNECT_REQ)

    def disconnect(self) -> None:
        """Orderly close: finish sending every queued byte, get it acknowledged, then
        DISC / DISC_ACK. Requesting this while still connecting just means "disconnect once
        the transfer is done"; only :meth:`abort` tears down a half-open session."""
        if self.state is State.IDLE:
            return
        self._disc_requested = True
        if self.state is State.CONNECTING:
            return
        if self.role is Role.ISS and self._waiting_for is None and not self._tx_busy():
            self._maybe_start_burst()

    def abort(self) -> None:
        """Drop the session now (one DISC on the way out)."""
        if self.state is State.IDLE:
            return
        if self.state is not State.CONNECTING:
            self._transmit([self._control(ControlKind.DISC)])
        self._end_session("aborted")

    def send(self, data: bytes) -> None:
        self._tx_queue += data
        if self.state is State.CONNECTED and self.role is Role.ISS:
            self._maybe_start_burst()

    def request_break(self) -> None:
        """IRS: demand the sending role in the next ACK."""
        self._break_requested = True

    @property
    def tx_pending_bytes(self) -> int:
        queued = len(self._tx_queue)
        return queued + sum(len(r.body) for r in self._records.values() if not r.acked)

    @property
    def connected(self) -> bool:
        return self.state is State.CONNECTED

    @property
    def current_mode(self) -> int:
        return self._recommended

    def drain(self) -> list[Action]:
        out, self.actions = self.actions, []
        return out

    # ── inputs ────────────────────────────────────────────────────────

    def on_tx_done(self, now: float) -> None:
        self.now = max(self.now, now)
        self._tx_busy_until = min(self._tx_busy_until, now)

    def tick(self, now: float) -> None:
        self.now = max(self.now, now)
        for name in sorted(self._deadlines, key=self._deadlines.__getitem__):
            if self._deadlines.get(name, float("inf")) <= self.now:
                del self._deadlines[name]
                self._expire(name)

    def on_frame(self, frame: SoftFrame, now: float) -> None:
        self.now = max(self.now, now)
        if frame.container is Container.CONTROL:
            self._on_control(frame)
        else:
            self._on_data(frame)

    def on_preamble(self, t_start: float, now: float) -> None:
        """The PHY has detected a frame starting at ``t_start`` but has not decoded it yet.

        This is what lets the IRS answer a burst promptly: it now knows the burst is still
        running and can hold its ACK until that frame has finished, instead of assuming a
        whole frame of silence means the burst ended. Optional — a PHY that cannot report
        preambles simply leaves :attr:`PhyTiming.preamble_detect_s` unset."""
        self.now = max(self.now, now)
        if self.role is not Role.IRS or self.state not in (State.CONNECTED, State.DISCONNECTING):
            return
        deadline = t_start + self.timing.data_frame_s + self._irs_reply_delay()
        self._deadlines["ack"] = max(self._deadlines.get("ack", 0.0), deadline)

    # ── timers ────────────────────────────────────────────────────────

    def next_deadline(self) -> float | None:
        """Earliest armed timer, for an external scheduler (the simulators)."""
        return min(self._deadlines.values()) if self._deadlines else None

    def _arm(self, name: str, delay: float) -> None:
        self._deadlines[name] = self.now + delay

    def _disarm(self, name: str) -> None:
        self._deadlines.pop(name, None)

    def _irs_reply_delay(self) -> float:
        """How long the IRS waits after a burst's last frame before sending its ACK.

        The ISS carries no burst length — the header must be identical across the
        retransmissions the receiver soft-combines — so the end of a burst is inferred from
        silence. How *much* silence depends on what the PHY reports: given a start-of-frame
        signal (:attr:`PhyTiming.preamble_detect_s`) a contiguous next frame announces itself
        that quickly, so the IRS only waits that long; without one it has to wait a whole
        data frame, which is roughly a quarter of the air time."""
        quiet = self.timing.preamble_detect_s
        if quiet is None:
            quiet = self.timing.data_frame_s
        return quiet + self.cfg.burst_gap_s + self.timing.turnaround_s

    def _response_wait(self, response_s: float, responder_delay: float = 0.0) -> float:
        """How long to wait for a peer response of the given air time, from the end of our
        own transmission. ``responder_delay`` is any processing the peer does first (the
        IRS's end-of-burst wait before an ACK)."""
        t = self.timing
        return (
            responder_delay
            + t.turnaround_s
            + response_s
            + t.detect_latency_s
            + self.cfg.ack_margin_s
        )

    def _expire(self, name: str) -> None:
        if name == "link":
            self._end_session("link timeout")
        elif name == "connect":
            self._retry_connect()
        elif name == "ack":
            self._send_ack()
        elif name == "wait":
            self._on_response_timeout()
        elif name == "keepalive" and self.role is Role.ISS and self._waiting_for is None:
            self._send_poll()

    def _tx_busy(self) -> bool:
        return self.now < self._tx_busy_until

    # ── transmit helpers ──────────────────────────────────────────────

    def _transmit(self, frames: list[TxFrame]) -> None:
        dur = sum(
            self.timing.data_frame_s
            if f.container is Container.DATA
            else self.timing.control_frame_s
            for f in frames
        )
        self._tx_busy_until = self.now + dur
        self.actions.append(Transmit(frames, dur))

    def _control(self, kind: ControlKind, **kw: object) -> TxFrame:
        return TxFrame(Container.CONTROL, ControlFrame(kind, self.session, **kw).encode())  # type: ignore[arg-type]

    def _data_frame(self, rec: _TxRecord) -> TxFrame:
        rec.tx_count += 1
        cap = self.timing.capacity(rec.mode)
        payload = encode_data(DataHeader(rec.kind, rec.seq, self.session), rec.body, cap)
        return TxFrame(Container.DATA, payload, mode=rec.mode, rv=(rec.tx_count - 1) % 4)

    def _send_connect(self, kind: DataKind) -> None:
        src, dst = self.my_call, self.remote_call
        body = ConnectBody(src, dst, caps=self.cfg.capabilities).encode()
        cap = self.timing.capacity(0)
        if cap < CONNECT_BODY_BYTES + 5:
            raise ValueError("mode 0 too small for a connect frame")
        payload = encode_data(DataHeader(kind, 0, self.session), body, cap)
        self._transmit([TxFrame(Container.DATA, payload, mode=0, rv=0)])
        if kind is DataKind.CONNECT_REQ:
            self._connect_tries += 1
            # backoff that widens with each retry, so two stations that called each other
            # at the same instant desynchronise instead of colliding on every attempt
            span = (1 + self._connect_tries) * self.timing.data_frame_s
            wait = self._response_wait(self.timing.data_frame_s) + self.rng.uniform(0.0, span)
            self._arm("connect", self._tx_busy_until - self.now + wait)

    def _retry_connect(self) -> None:
        if self.state is not State.CONNECTING:
            return
        if self._connect_tries >= self.cfg.connect_retries:
            self._end_session("no answer")
            return
        self._send_connect(DataKind.CONNECT_REQ)

    def _wait_for(self, what: str, response_s: float, responder_delay: float = 0.0) -> None:
        self._waiting_for = what
        self._arm(
            "wait",
            self._tx_busy_until - self.now + self._response_wait(response_s, responder_delay),
        )

    def _send_poll(self) -> None:
        self._transmit([self._control(ControlKind.POLL)])
        self._wait_for("poll", self.timing.control_frame_s)

    def _send_turn(self) -> None:
        self._turn_tries += 1
        self.stats.turns += 1
        self._transmit([self._control(ControlKind.TURN)])
        self.role = Role.IRS
        self._bursts_since_turn = 0
        self._peer_wants_tx = self._peer_break = False
        self._disarm("keepalive")
        # the peer answers with its first burst (or a POLL); we wait a full data frame
        self._wait_for("turn", self.timing.data_frame_s)
        self.actions.append(Event("role", "irs"))

    def _send_disc(self) -> None:
        self._disc_tries += 1
        self.state = State.DISCONNECTING
        self._transmit([self._control(ControlKind.DISC)])
        self._wait_for("disc", self.timing.control_frame_s)

    def _on_response_timeout(self) -> None:
        what, self._waiting_for = self._waiting_for, None
        if self.state is State.DISCONNECTING or what == "disc":
            if self._disc_tries >= self.cfg.disc_retries:
                self._end_session("closed (no DISC_ACK)")
            else:
                self._send_disc()
            return
        if what == "turn":
            if self._turn_tries >= self.cfg.turn_retries:
                # the peer never took the turn: carry on as ISS
                self.role = Role.ISS
                self._turn_tries = 0
                self.actions.append(Event("role", "iss"))
                self._maybe_start_burst()
            else:
                self.role = Role.ISS
                self._send_turn()
            return
        self._retries += 1
        self.stats.ack_timeouts += 1
        if self._retries > self.cfg.max_retries:
            self._end_session("no response")
            return
        if what == "ack":
            self._send_burst()  # same composition: nothing was acknowledged
        elif what == "poll":
            self._send_poll()

    # ── ISS: bursts ───────────────────────────────────────────────────

    def _unacked(self) -> list[int]:
        seqs = [s for s, r in self._records.items() if not r.acked and r.tx_count > 0]
        return sorted(seqs, key=lambda s: seq_distance(s, self._tx_base))

    def _outstanding(self) -> int:
        return seq_distance(self._tx_next, self._tx_base)

    def _maybe_start_burst(self) -> None:
        if (
            self.state is not State.CONNECTED
            or self.role is not Role.ISS
            or self._waiting_for is not None
            or self._tx_busy()
        ):
            return
        if self._disc_requested and not self._unacked() and not self._tx_queue:
            self._send_disc()
            return
        if self._peer_break or (
            self._peer_wants_tx
            and (self._bursts_since_turn >= self.cfg.bursts_before_turn or not self._has_work())
        ):
            self._turn_tries = 0
            self._send_turn()
            return
        if self._has_work():
            self._send_burst()
        elif "keepalive" not in self._deadlines:
            self._arm("keepalive", self.cfg.keepalive_s)

    def _has_work(self) -> bool:
        return bool(self._unacked()) or bool(self._tx_queue)

    def _send_burst(self) -> None:
        seqs = self._unacked()[: self.cfg.burst_frames]
        mode = min(self._recommended, self.cfg.max_mode)
        cap = data_capacity(self.timing.capacity(mode))
        while (
            len(seqs) < min(self.cfg.burst_frames, MAX_BURST)
            and self._outstanding() < WINDOW
            and self._tx_queue
        ):
            body = bytes(self._tx_queue[:cap])
            del self._tx_queue[:cap]
            rec = _TxRecord(self._tx_next, DataKind.DATA, body, mode)
            self._records[rec.seq] = rec
            self._tx_next = seq_after(self._tx_next)
            seqs.append(rec.seq)
        if not seqs:
            return
        frames = []
        for s in seqs:
            rec = self._records[s]
            if rec.tx_count:
                self.stats.frames_resent += 1
            frames.append(self._data_frame(rec))
        self.stats.frames_sent += len(frames)
        self.stats.bursts += 1
        self._burst_seqs = seqs
        self._bursts_since_turn += 1
        self._disarm("keepalive")
        self._transmit(frames)
        self._wait_for("ack", self.timing.control_frame_s, self._irs_reply_delay())

    def _on_ack(self, ack: ControlFrame) -> None:
        self.stats.acks_received += 1
        self._retries = 0
        for s, rec in list(self._records.items()):
            if not rec.acked and rec.tx_count and ack.received(s):
                rec.acked = True
        while self._tx_base != self._tx_next and self._records[self._tx_base].acked:
            del self._records[self._tx_base]
            self._tx_base = seq_after(self._tx_base)
        self._recommended = max(0, min(self.cfg.max_mode, ack.recommended_mode))
        self._peer_wants_tx = bool(ack.flags & ControlFlags.WANT_TX)
        self._peer_break = bool(ack.flags & ControlFlags.BREAK)
        self._waiting_for = None
        self._disarm("wait")
        self._maybe_start_burst()

    # ── IRS: bursts, HARQ and ACKs ────────────────────────────────────

    def _slot_of(self, frame: SoftFrame) -> int:
        if self._burst_t0 is None:
            self._burst_t0 = frame.t_start
            return 0
        return max(0, round((frame.t_start - self._burst_t0) / self.timing.data_frame_s))

    def _on_data(self, frame: SoftFrame) -> None:
        if self.state is State.IDLE or self.state is State.CONNECTING:
            payload, _ = frame.decode(None)
            if payload is not None:
                self._on_data_payload(payload, frame)
            return
        if self.role is Role.ISS and self._waiting_for in ("ack", "poll", None):
            # a DATA frame from the peer while we hold the turn: it believes it is ISS
            # (a TURN of ours it answered late, or a lost TURN retry). Data wins.
            payload, _ = frame.decode(None)
            if payload is None:
                return
            try:
                header, _ = decode_data(payload)
            except ValueError:
                return
            if header.session != self.session:
                return
            if header.kind is DataKind.CONNECT_ACK:
                return  # repeated accept: our confirmation is on its way
            self.role = Role.IRS
            self._waiting_for = None
            self._disarm("wait")
            self._disarm("keepalive")
            self.actions.append(Event("role", "irs"))
        if self.role is Role.IRS and self._waiting_for == "turn":
            self._waiting_for = None
            self._turn_tries = 0
            self._disarm("wait")
        # part of the current burst: record it, decode what decodes, ACK after the gap
        rec = _RxRecord(frame, self._slot_of(frame))
        self._burst.append(rec)
        self._decode_record(rec)
        self._arm("ack", max(0.0, frame.t_end - self.now) + self._irs_reply_delay())

    def _decode_record(self, rec: _RxRecord) -> None:
        payload, buffer = rec.frame.decode(None)
        if payload is not None:
            self._accept(rec, payload)
            return
        # inference: which sequence number is this slot? Then combine with any earlier
        # transmission of that block (HARQ-IR) and, either way, keep this transmission's
        # soft information for the next retransmission. A wrong guess only wastes a combine
        # — the CRC never lets mismatched LLRs through — and max_combines caps that waste.
        guess = self._infer_seq(rec.slot)
        rec.seq = guess
        rec.buffer = buffer
        if guess is None or not in_window(guess, self._rx_base):
            self.stats.frames_failed += 1
            return
        if guess in self._harq:
            prev, combines = self._harq[guess]
            combined, merged = rec.frame.decode(prev)
            if combined is not None:
                self.stats.harq_rescues += 1
                self._accept(rec, combined)
                return
            self._harq[guess] = (
                (buffer, 0) if combines + 1 >= self.cfg.max_combines else (merged, combines + 1)
            )
        else:
            self._harq[guess] = (buffer, 0)
        self.stats.frames_failed += 1

    def _infer_seq(self, slot: int) -> int | None:
        """Map a burst slot to a sequence number. Decoded frames in this burst anchor the
        mapping when present; otherwise fall back to the most recent ACK snapshot (the ISS
        sends unacknowledged frames first, then new ones, so slot 0 is the oldest gap)."""
        anchors = [
            (r.slot, r.seq) for r in self._burst if r.payload is not None and r.seq is not None
        ]
        snapshots = self._ack_history or [_AckSnapshot(self._rx_base, [], self._rx_base)]
        for snap in snapshots:
            expected = self._expected_burst(snap)
            offsets: set[int] = set()
            consistent = True
            for a_slot, a_seq in anchors:
                if a_seq not in expected:
                    consistent = False
                    break
                offsets.add(expected.index(a_seq) - a_slot)
            if not consistent or len(offsets) > 1:
                continue
            idx = slot + (offsets.pop() if offsets else 0)
            if 0 <= idx < len(expected):
                return expected[idx]
        return None

    def _expected_burst(self, snap: _AckSnapshot) -> list[int]:
        out = list(snap.missing)
        s = snap.next_new
        while len(out) < MAX_BURST:
            out.append(s)
            s = seq_after(s)
        return out

    def _accept(self, rec: _RxRecord, payload: bytes) -> None:
        rec.payload = payload
        try:
            header, body = decode_data(payload)
        except ValueError:
            rec.payload = None
            return
        if header.session != self.session:
            rec.payload = None
            return
        self._last_peer_frame = self.now
        self._arm("link", self.cfg.link_timeout_s)
        rec.seq = header.seq
        self.stats.frames_received += 1
        if header.kind is DataKind.CONNECT_REQ:
            # our CONNECT_ACK was lost: answer again
            self._send_connect(DataKind.CONNECT_ACK)
            self._burst.clear()
            self._burst_t0 = None
            self._disarm("ack")
            return
        if header.kind is DataKind.CONNECT_ACK:
            return
        if self.state is State.CONNECTED and self.role is Role.IRS and not self._confirmed:
            self._confirmed = True
        if in_window(header.seq, self._rx_base):
            if self._max_seen is None or seq_distance(header.seq, self._rx_base) > seq_distance(
                self._max_seen, self._rx_base
            ):
                self._max_seen = header.seq
            if header.seq not in self._rx_buf:
                self._rx_buf[header.seq] = body
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
        # else: an old duplicate (our ACK was lost); the next ACK covers it

    def _finish_burst(self) -> None:
        """End of a burst: HARQ buffers were already stored per frame in
        :meth:`_decode_record`, so just clear the burst accumulator."""
        self._burst.clear()
        self._burst_t0 = None

    def _send_ack(self) -> None:
        if self.state is not State.CONNECTED and self.state is not State.DISCONNECTING:
            return
        ok = sum(1 for r in self._burst if r.payload is not None)
        failed = len(self._burst) - ok
        snrs = [r.frame.snr_db for r in self._burst]
        snr = sum(snrs) / len(snrs) if snrs else None
        modes = [r.frame.mode for r in self._burst]
        burst_mode = max(set(modes), key=modes.count) if modes else None
        self._finish_burst()
        self.rate.observe(snr, ok, failed, burst_mode)
        if self._disc_requested:
            self._send_disc()
            return
        bitmap = 0
        missing: list[int] = []
        limit = 0 if self._max_seen is None else seq_distance(self._max_seen, self._rx_base) + 1
        for i in range(WINDOW):
            s = seq_after(self._rx_base, i)
            if s in self._rx_buf:
                bitmap |= 1 << i
            elif i < limit:
                missing.append(s)
        next_new = seq_after(self._rx_base, limit)
        self._ack_history = [_AckSnapshot(self._rx_base, missing, next_new), *self._ack_history][:2]
        flags = ControlFlags.NONE
        if self._tx_queue:
            flags |= ControlFlags.WANT_TX
        if self._break_requested:
            flags |= ControlFlags.BREAK | ControlFlags.WANT_TX
        self._ack_counter = (self._ack_counter + 1) % 16
        self.stats.acks_sent += 1
        self._transmit(
            [
                self._control(
                    ControlKind.ACK,
                    flags=flags,
                    base=self._rx_base,
                    bitmap=bitmap,
                    snr_db=snr,
                    recommended_mode=self.rate.recommend(),
                    counter=self._ack_counter,
                )
            ]
        )

    # ── control frames ────────────────────────────────────────────────

    def _on_control(self, frame: SoftFrame) -> None:
        payload, _ = frame.decode(None)
        if payload is None:
            return
        try:
            ctl = ControlFrame.decode(payload)
        except ValueError:
            return
        if (
            self.state is State.IDLE
            or self.state is State.CONNECTING
            or ctl.session != self.session
        ):
            return
        self._last_peer_frame = self.now
        self._arm("link", self.cfg.link_timeout_s)
        if ctl.kind is ControlKind.DISC:
            self._transmit([self._control(ControlKind.DISC_ACK)])
            self._end_session("peer disconnected")
        elif ctl.kind is ControlKind.DISC_ACK:
            if self.state is State.DISCONNECTING:
                self._end_session("closed")
        elif ctl.kind is ControlKind.ACK:
            if self.role is Role.ISS and self._waiting_for in ("ack", "poll"):
                self._on_ack(ctl)
        elif ctl.kind is ControlKind.POLL:
            if self.role is Role.IRS or self._waiting_for == "turn":
                self._take_irs()
                self._arm("ack", self.timing.turnaround_s + max(0.0, frame.t_end - self.now))
        elif ctl.kind is ControlKind.TURN:
            if self.role is Role.IRS or self._waiting_for == "turn":
                self._take_iss()

    def _take_irs(self) -> None:
        if self.role is not Role.IRS:
            self.actions.append(Event("role", "irs"))
        self.role = Role.IRS
        self._waiting_for = None
        self._turn_tries = 0
        self._disarm("wait")
        self._disarm("keepalive")

    def _take_iss(self) -> None:
        self.role = Role.ISS
        self._waiting_for = None
        self._disarm("wait")
        self._disarm("ack")
        self._burst.clear()
        self._burst_t0 = None
        self._break_requested = False
        self._bursts_since_turn = 0
        self._retries = 0
        self.actions.append(Event("role", "iss"))
        if self._has_work():
            self._send_burst()
        else:
            self._send_poll()

    # ── connection handling ───────────────────────────────────────────

    def _on_data_payload(self, payload: bytes, frame: SoftFrame) -> None:
        try:
            header, body = decode_data(payload)
        except ValueError:
            return
        if header.kind is DataKind.CONNECT_REQ:
            self._handle_connect_req(header, body)
        elif header.kind is DataKind.CONNECT_ACK:
            self._handle_connect_ack(header, body)

    def _handle_connect_req(self, header: DataHeader, body: bytes) -> None:
        try:
            req = ConnectBody.decode(body)
        except ValueError:
            return
        if req.dst != self.my_call:
            return
        if self.state is State.CONNECTING and self.my_call > self.remote_call:
            return  # simultaneous call: the higher callsign keeps calling
        self.remote_call = req.src
        self.session = header.session
        self._disarm("connect")
        self._reset_transfer_state()
        self.peer_capabilities = req.caps
        self.state = State.CONNECTED
        self.role = Role.IRS
        self._confirmed = False
        self._last_peer_frame = self.now
        self._arm("link", self.cfg.link_timeout_s)
        self._send_connect(DataKind.CONNECT_ACK)
        self.actions.append(Event("connected", f"{self.remote_call} (irs)"))

    def _handle_connect_ack(self, header: DataHeader, body: bytes) -> None:
        if self.state is not State.CONNECTING or header.session != self.session:
            return
        try:
            ack = ConnectBody.decode(body)
        except ValueError:
            return
        if ack.dst != self.my_call:
            return
        self._disarm("connect")
        self.peer_capabilities = ack.caps
        self.state = State.CONNECTED
        self.role = Role.ISS
        self._confirmed = True
        self._last_peer_frame = self.now
        self._arm("link", self.cfg.link_timeout_s)
        self.actions.append(Event("connected", f"{self.remote_call} (iss)"))
        self._recommended = self.cfg.initial_mode
        if self._has_work():
            self._send_burst()
        else:
            self._send_poll()  # confirms the handshake and fetches the first ACK

    def _reset_transfer_state(self) -> None:
        self._records.clear()
        self._tx_base = self._tx_next = 0
        self._burst_seqs = []
        self._retries = 0
        self._bursts_since_turn = 0
        self._peer_wants_tx = self._peer_break = False
        self._recommended = self.cfg.initial_mode
        self._turn_tries = self._disc_tries = 0
        self._disc_requested = False
        self._waiting_for = None
        self._rx_base = 0
        self._rx_buf.clear()
        self._max_seen = None
        self._harq.clear()
        self._burst.clear()
        self._burst_t0 = None
        self._ack_history = []
        self._ack_counter = 0
        self._break_requested = False
        self._confirmed = False
        self.peer_capabilities = 0
        self.rate = RateController()
        for name in ("ack", "wait", "keepalive", "link"):
            self._disarm(name)

    def _end_session(self, reason: str) -> None:
        self.state = State.IDLE
        self.role = Role.NONE
        self._reset_transfer_state()
        self._disarm("connect")
        self.actions.append(Event("disconnected", reason))
