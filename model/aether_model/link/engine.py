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
from collections.abc import Sequence
from dataclasses import dataclass, field
from enum import Enum

from aether_model.link.frames import (
    CONNECT_BODY_BYTES,
    MAX_BURST,
    PROTOCOL_VERSION,
    WINDOW,
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    ProbeBody,
    bandwidth_code,
    data_capacity,
    decode_data,
    encode_data,
    in_window,
    pack_callsign,
    seq_after,
    seq_distance,
)
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import RateController, usable_modes

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
    """No valid frame from the peer for this long ends the session — or longer where the
    link runs long frames: see :attr:`link_timeout_exchanges`."""
    link_timeout_exchanges: float = 4.0
    """Silence ends a session only after at least this many whole exchanges' air time at
    the family the link runs in — a full burst, its acknowledgement and both turnarounds
    (P9-7). On the ordinary layouts that is well inside :attr:`link_timeout_s`; on the
    500 Hz floor (ADR-0009) one exchange is nearly half a minute, and a fixed 45 s dropped
    two sessions in three at −4 dB on the fading bench (Good, Moderate) that 85 s carried
    to the end: a slow fade outlasts one missed exchange far more often than a real link
    fails."""
    ack_margin_s: float = 0.4
    """Slack added to every wait for a peer response."""
    burst_gap_s: float = 0.2
    """Silence after a DATA frame that marks the end of a burst (the IRS then ACKs)."""
    rate: dict[str, float | int] = field(default_factory=dict)
    """Overrides for the rate controller's tunables (``RateController`` fields by name),
    for benches that compare one controller against another; empty means the defaults."""
    initial_mode: int = 0
    """The slowest mode a session's first burst goes out at. The acceptance's SNR report
    picks the first mode (P9-2); this is the floor under it, which a bench that pins a
    mode sets along with :attr:`max_mode`."""
    max_mode: int = 15
    """The fastest rung the station sends at: the top of the widest ladder (2 300 Hz,
    ADR-0013) by default — a recommendation never leaves the air's own table."""
    bursts_before_turn: int = 3
    """With a WANT_TX peer, the ISS hands over after this many bursts of its own."""
    silence_step: int = 2
    """Usable modes the ISS steps its own recommendation down by for every burst that goes
    unanswered (P9-7). The recommendation otherwise moves only when an acknowledgement
    brings one, and on a fading path the burst and its acknowledgement fade together: a
    session whose first bursts went out on a connect frame measured at a peak repeated
    them at a mode the path could not carry until the link timed out, one session in
    twenty at 0–3 dB on the 500 Hz fading bench. Two steps a silence reaches the bottom of
    either table within the retries, and the frames stranded up there are re-encoded on
    the way (after :attr:`max_combines`); the next acknowledgement puts the peer's own
    recommendation back."""
    max_combines: int = 4
    """HARQ buffers are reset after this many failed combines (guards a wrong inference)."""
    capabilities: int = 0
    """Capability bits offered in the connect handshake. What they mean is the caller's
    business; the link layer carries them and reports what the peer offered. Bit 0 is stream
    compression (deflate, RFC 1951); bits 1–2 state the bandwidth this station transmits in
    (:func:`~aether_model.link.frames.with_bandwidth`), and a request or answer stating
    another is ignored — see ``docs/spec/air-interface.md`` §7.3."""


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
    reencoded: int = 0
    """Transmissions made under an earlier codeword, before a re-encoding."""


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
    # payload bytes the peer acknowledged: what has actually crossed, seen from the sender
    bytes_acked: int = 0
    probes_sent: int = 0
    probes_answered: int = 0
    """Probes from other stations this one answered."""
    probe_replies: int = 0
    """Answers to this station's own probes that arrived."""
    frames_reencoded: int = 0
    """Frames given another codeword at a slower mode after going unacknowledged at their
    own for :attr:`LinkConfig.max_combines` transmissions."""


@dataclass(frozen=True)
class LadderRung:
    """One burst sent at a pinned mode and what came back for it: a rung of the Test
    session's mode ladder (P6-7), the frame error rate the peer saw at the SNR it measured."""

    mode: int
    frames: int
    """New frames the burst carried at the pinned mode; retransmissions are not counted."""
    decoded: int
    """Of those, the ones the acknowledgement covered."""
    snr_db: float | None
    """The SNR the peer measured on the burst, from its acknowledgement."""


@dataclass(frozen=True)
class ProbeResult:
    """What a probe of ours came back with (ADR-0006): who answered, the SNR they measured
    on our probe, and the SNR we measured on their answer."""

    remote: str
    heard_there_db: float | None
    heard_here_db: float


# ── the engine ────────────────────────────────────────────────────────


class LinkEngine:
    def __init__(
        self,
        my_call: str,
        timing: PhyTiming,
        config: LinkConfig | None = None,
        seed: int | None = None,
    ) -> None:
        self.callsigns = [my_call.upper()]
        """The callsigns this station answers to. The first is the one it calls as unless a
        call says otherwise; see :meth:`set_callsigns`."""
        self.my_call = self.callsigns[0]
        """The callsign the current (or next) session runs under: the one that was called
        when this station answered, the one it chose when it called."""
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
        self.rate = self._rate_controller()
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
        # the SNR the peer measured on our last burst, carried in its ACK: the
        # one number an operator cannot get from their own receiver
        self.peer_snr_db: float | None = None
        self._turn_tries = 0
        self._disc_requested = False
        self._disc_tries = 0
        self._connect_tries = 0
        self._peer_floor = False
        """Whether the last frame heard from the peer came on a floor layout (ADR-0009):
        the family our control frames answer in, and the layout a connect answer goes
        back on."""
        self._peer_mode: int | None = None
        """The mode of the last DATA frame heard from the peer — what its next frames are
        most likely to take on the air."""
        self._probing: str | None = None
        """The station a probe of ours is out to, until it answers or the timer fires."""
        self.last_probe: ProbeResult | None = None
        """What the last probe came back with; ``None`` while one is out, or after one
        went unanswered."""
        self._pinned: int | None = None
        """A mode every new burst goes out at while set, whatever the peer recommends:
        the Test session's mode ladder (P6-7)."""
        self._pin_body: int | None = None
        """While pinned, the most payload a new frame takes — small, so a frame the
        pinned mode cannot carry can be re-encoded at one that can."""
        self._ladder_pending: tuple[int, list[int]] | None = None
        self.ladder: list[LadderRung] = []
        """What each pinned burst reported back, in order; see :meth:`pin_mode`."""
        self._waiting_for: str | None = None  # "ack" | "poll" | "turn" | "disc" | "connect"
        # receiving side
        self._rx_base = 0
        self._rx_buf: dict[int, bytes] = {}
        self._max_seen: int | None = None
        self._harq: dict[int, tuple[object, int, int]] = {}
        """Per sequence number: the soft information kept for combining, how many combines
        it has been through, and the mode it was sent at — another mode is another
        codeword, which cannot be combined with it."""
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

    def set_callsigns(self, calls: Sequence[str]) -> None:
        """Change the callsigns this station answers to; the first is the one it calls as.

        The operator's callsign belongs to the host program in practice — VARA's published
        interface has no callsign of its own, ``MYCALL`` is the only place one is ever set,
        and clients send several when a station also answers to a club or tactical call —
        so the engine takes a list at run time rather than one name at construction. Refused
        while a session is up: the callsign is in every frame's addressing, and changing it
        would orphan the peer."""
        if self.state is not State.IDLE:
            raise RuntimeError("a session is running")
        cleaned = [c.strip().upper() for c in calls if c.strip()]
        if not cleaned:
            raise ValueError("at least one callsign is needed")
        for call in cleaned:
            pack_callsign(call)  # what the air interface cannot carry is refused here
        self.callsigns = cleaned
        self.my_call = cleaned[0]

    def connect(self, remote_call: str, as_call: str | None = None) -> None:
        """Call a station, as this station's first callsign unless ``as_call`` picks
        another of them."""
        if self.state is not State.IDLE:
            raise RuntimeError("already in a session")
        if as_call is None:
            self.my_call = self.callsigns[0]
        elif as_call.upper() in self.callsigns:
            self.my_call = as_call.upper()
        else:
            raise ValueError(f"{as_call.upper()} is not one of this station's callsigns")
        self.remote_call = remote_call.upper()
        # never 0: with kind DATA and sequence 0 an all-zero body would make an all-zero
        # frame, which the PHY refuses (the all-zero codeword passes any CRC)
        self.session = self.rng.randrange(1, 256)
        self.state = State.CONNECTING
        self.role = Role.NONE
        self._connect_tries = 0
        self._reset_transfer_state()
        self._send_connect(DataKind.CONNECT_REQ)

    def probe(self, remote_call: str, as_call: str | None = None) -> None:
        """Ask a station whether it hears this one, and how well, without a session.

        One PROBE frame, at the most robust mode; the answer, if it comes, arrives as a
        ``probe`` event naming both directions of the path — the SNR the other station
        measured on our probe, and the SNR we measured on its answer. A probe that goes
        unanswered within one frame's turnaround is reported as such; the operator asks
        again if they want, so there are no retries to fill a channel with."""
        if self.state is not State.IDLE:
            raise RuntimeError("already in a session")
        if self._probing is not None:
            raise RuntimeError("a probe is already out")
        if as_call is None:
            self.my_call = self.callsigns[0]
        elif as_call.upper() in self.callsigns:
            self.my_call = as_call.upper()
        else:
            raise ValueError(f"{as_call.upper()} is not one of this station's callsigns")
        self._probing = remote_call.upper()
        self.last_probe = None
        self.stats.probes_sent += 1
        self._send_probe(DataKind.PROBE, self._probing, None)
        wait = self._response_wait(self.timing.data_frame_s_for(self._robust_mode(False)))
        self._arm("probe", self._tx_busy_until - self.now + wait)

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

    def pin_mode(self, mode: int | None, body_bytes: int | None = None) -> None:
        """Send every new burst at ``mode`` until unpinned (``None``), whatever the peer
        recommends — the Test session's mode ladder (P6-7): a burst at each mode from the
        floor up, and what its acknowledgement said appended to :attr:`ladder` as the
        frame error rate the peer saw at the SNR it measured. ``body_bytes`` caps the
        payload of each new frame while pinned, so that a frame a mode cannot carry can
        be re-encoded at one that can (see :meth:`_send_burst`); the operator's
        :attr:`LinkConfig.max_mode` still applies. Raises ``ValueError`` for a mode the
        table has not."""
        if mode is not None and mode not in (self.timing.data_capacity or {}):
            raise ValueError(f"mode {mode} is not in the table")
        if body_bytes is not None and body_bytes < 1:
            raise ValueError("body_bytes must be at least 1")
        self._pinned = mode
        self._pin_body = body_bytes if mode is not None else None

    def take_ladder(self) -> list[LadderRung]:
        """The rungs recorded since the last call, oldest first."""
        rungs, self.ladder = self.ladder, []
        return rungs

    def request_break(self) -> None:
        """IRS: demand the sending role in the next ACK."""
        self._break_requested = True

    @property
    def tx_pending_bytes(self) -> int:
        queued = len(self._tx_queue)
        return queued + sum(len(r.body) for r in self._records.values() if not r.acked)

    @property
    def probing(self) -> bool:
        """Whether a probe of ours is out, unanswered and not yet given up on."""
        return self._probing is not None

    def all_acknowledged(self) -> bool:
        """Whether everything handed to :meth:`send` has left and been acknowledged."""
        return not self._tx_queue and all(r.acked for r in self._records.values())

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
        """The physical layer finished a transmission at ``now`` (the last sample on the air).

        Work that arrived while the transmitter was busy — an acknowledgement that came in
        during a re-poll, data queued mid-burst — could not start a burst then, and nothing
        else would start it later: this is the moment to try.
        """
        self.now = max(self.now, now)
        self._tx_busy_until = min(self._tx_busy_until, now)
        if self.state is State.CONNECTED and self.role is Role.ISS and self._waiting_for is None:
            self._maybe_start_burst()

    def on_tx_delayed(self, seconds: float) -> None:
        """The physical layer is holding the last transmission back — the channel is busy —
        and has now held it for another ``seconds``.

        Every deadline moves with it. A timer set when the burst was handed over expects a
        reply to a burst that has not left yet; left alone it fires against nothing, a
        retry of the same frame is queued behind the one still waiting, and the two go out
        back to back when the channel clears. Seen on the air on the first attempt: pairs of
        connect requests in one keying, at a cadence set by the busy detector rather than
        by the backoff."""
        if seconds <= 0.0:
            return
        self._tx_busy_until += seconds
        for name in self._deadlines:
            self._deadlines[name] += seconds

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

    def on_preamble(self, t_start: float, now: float, frame_s: float | None = None) -> None:
        """The PHY has detected a frame starting at ``t_start`` but has not decoded it yet.

        This is what lets the IRS answer a burst promptly: it now knows the burst is still
        running and can hold its ACK until that frame has finished, instead of assuming a
        whole frame of silence means the burst ended. Optional — a PHY that cannot report
        preambles simply leaves :attr:`PhyTiming.preamble_detect_s` unset.

        ``frame_s`` is the announced frame's own air time, which its preamble names (the
        layout, and with it the family: a floor frame is four times an ordinary one). Without
        it the IRS assumes the longest frame the peer may send next — a guess that went wrong
        when a session's first burst dropped into the floor after the connect frames were
        ordinary: the ACK fired in the middle of every floor frame and trampled it (P9-7)."""
        self.now = max(self.now, now)
        if self.role is not Role.IRS or self.state not in (State.CONNECTED, State.DISCONNECTING):
            return
        length = frame_s if frame_s is not None else self._peer_data_frame_s()
        deadline = t_start + length + self._irs_reply_delay()
        self._deadlines["ack"] = max(self._deadlines.get("ack", 0.0), deadline)

    # ── timers ────────────────────────────────────────────────────────

    def next_deadline(self) -> float | None:
        """Earliest armed timer, for an external scheduler (the simulators)."""
        return min(self._deadlines.values()) if self._deadlines else None

    def _arm(self, name: str, delay: float) -> None:
        self._deadlines[name] = self.now + delay

    def _disarm(self, name: str) -> None:
        self._deadlines.pop(name, None)

    def _irs_reply_delay(self, floor: bool | None = None, frame_s: float | None = None) -> float:
        """How long the IRS waits after a burst's last frame before sending its ACK.

        The ISS carries no burst length — the header must be identical across the
        retransmissions the receiver soft-combines — so the end of a burst is inferred from
        silence. How *much* silence depends on what the PHY reports: given a start-of-frame
        signal (:attr:`PhyTiming.preamble_detect_s`) a contiguous next frame announces itself
        that quickly, so the IRS only waits that long; without one it has to wait a whole
        data frame, which is roughly a quarter of the air time. The floor's frames announce
        themselves later than the ordinary ones (the tone floor's first sync block is eight
        symbols of 40 ms), so a burst on the floor gets the floor's wait (ADR-0013).

        The IRS asks with its own view — the family and the frames it has been hearing. The
        ISS asks too, to size its wait for the ACK, and has to ask about the burst it has
        just *sent*: ``floor`` its family and ``frame_s`` the longest data frame the IRS
        will expect. Asked with its own view instead, it answered with the family it last
        *heard* — an ordinary acceptance before a session's first floor burst — and waited
        too little by the difference, the whole ACK margin (ADR-0013)."""
        quiet = self.timing.preamble_detect_s_for(self._peer_floor if floor is None else floor)
        if quiet is None:
            quiet = self._peer_data_frame_s() if frame_s is None else frame_s
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
        elif name == "probe":
            if self._probing is not None:
                self.actions.append(Event("probe", f"{self._probing}: no answer"))
            self._probing = None
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
        dur = sum(self.timing.frame_s(f) for f in frames)
        self._tx_busy_until = self.now + self.timing.tx_latency_s + dur
        self.actions.append(Transmit(frames, dur))

    def _control(self, kind: ControlKind, **kw: object) -> TxFrame:
        payload = ControlFrame(kind, self.session, **kw).encode()  # type: ignore[arg-type]
        return TxFrame(Container.CONTROL, payload, floor=self._control_floor())

    def _note_peer_frame(self, frame: SoftFrame) -> None:
        """Learn the family (and, for data, the mode) the peer sends in — from frames that
        decoded, so a false detection cannot switch our control frames to the wrong
        layout."""
        if frame.container is Container.DATA:
            self._peer_floor = self.timing.is_floor(frame.mode)
            self._peer_mode = frame.mode
        else:
            self._peer_floor = frame.floor

    def _control_floor(self) -> bool:
        """The family our control frames go out in (ADR-0009): the ISS answers in the family
        of the bursts it sends, the IRS in the family of what it last heard — so an ACK
        comes back the way the burst went out, and either side can tell how long to wait
        for it."""
        if self.role is Role.ISS and self.state is State.CONNECTED:
            return self.timing.is_floor(self._burst_mode())
        return self._peer_floor

    def _burst_mode(self) -> int:
        """The mode the next burst's new frames go out at: the pin while one is set, the
        peer's recommendation otherwise, never past the operator's ceiling."""
        chosen = self._recommended if self._pinned is None else self._pinned
        return min(chosen, self.cfg.max_mode)

    def _fits(self, body_len: int, mode: int) -> bool:
        """Whether a body can be encoded at ``mode``: within its capacity, and not the one
        length short of full that the container cannot carry."""
        cap = data_capacity(self.timing.capacity(mode))
        return body_len == cap or body_len <= cap - 2

    def _reencode_target(self, body_len: int, from_mode: int, to_mode: int) -> int | None:
        """The slowest mode from ``to_mode`` up to (not including) ``from_mode`` that
        carries a body of ``body_len`` bytes, or ``None`` when none does — a full frame
        has nowhere slower to go."""
        for mode in sorted(self.timing.data_capacity or {}):
            if to_mode <= mode < from_mode and self._fits(body_len, mode):
                return mode
        return None

    def _peer_data_frame_s(self) -> float:
        """The longest DATA frame the peer may send next: the family of what we recommended
        or of what it last sent, whichever is longer."""
        modes = [self.rate.recommend()]
        if self._peer_mode is not None:
            modes.append(self._peer_mode)
        return max(self.timing.data_frame_s_for(m) for m in modes)

    def _back_off(self) -> None:
        """An unanswered burst is evidence too: step the recommendation down
        :attr:`LinkConfig.silence_step` usable modes (never below the table's first)."""
        modes = self.rate.modes
        current = min(self._recommended, self.cfg.max_mode)
        below = [m for m in modes if m <= current]
        index = modes.index(below[-1]) if below else 0
        self._recommended = modes[max(0, index - self.cfg.silence_step)]

    def _link_timeout(self) -> float:
        """How long the peer may stay silent before the session ends: the configured time,
        or :attr:`LinkConfig.link_timeout_exchanges` whole exchanges at the family the link
        runs in — the longest data frame either side sends or may send next, and the
        control frame of that family — whichever is longer."""
        frame = max(self._peer_data_frame_s(), self.timing.data_frame_s_for(self._burst_mode()))
        floor = self._peer_floor or self.timing.is_floor(self._burst_mode())
        exchange = (
            self.cfg.burst_frames * frame
            + self.timing.control_frame_s_for(floor)
            + 2 * self.timing.turnaround_s
            + self.cfg.burst_gap_s
        )
        return max(self.cfg.link_timeout_s, self.cfg.link_timeout_exchanges * exchange)

    def _robust_mode(self, floor: bool) -> int:
        """The slowest mode of the given family whose frame carries a connect body (with a
        DATA header and length): what connect requests, answers, probes and beacons go out
        at. Falls back to the ordinary family when the floor has no such mode."""
        need = CONNECT_BODY_BYTES + 5
        caps = self.timing.data_capacity or {}
        for m in sorted(caps):
            if self.timing.is_floor(m) == floor and caps[m] >= need:
                return m
        if floor:
            return self._robust_mode(False)
        raise ValueError("no mode carries a connect frame")

    def _connect_floor(self) -> bool:
        """Whether the next connect request goes out on the floor layout: the first two tries
        are ordinary frames, then the two families alternate, so a station that can only
        be heard at the floor is still reached (ADR-0009)."""
        return (
            self.timing.floor_modes > 0
            and self._connect_tries >= 2
            and (self._connect_tries - 2) % 2 == 0
        )

    def _data_frame(self, rec: _TxRecord) -> TxFrame:
        rec.tx_count += 1
        cap = self.timing.capacity(rec.mode)
        payload = encode_data(DataHeader(rec.kind, rec.seq, self.session), rec.body, cap)
        return TxFrame(Container.DATA, payload, mode=rec.mode, rv=(rec.tx_count - 1) % 4)

    def _send_connect(self, kind: DataKind, snr_db: float | None = None) -> None:
        src, dst = self.my_call, self.remote_call
        body = ConnectBody(src, dst, caps=self.cfg.capabilities, snr_db=snr_db).encode()
        # a request alternates families once the ordinary frame has gone unanswered; an
        # answer goes back on the layout the request arrived on
        floor = self._connect_floor() if kind is DataKind.CONNECT_REQ else self._peer_floor
        mode = self._robust_mode(floor)
        cap = self.timing.capacity(mode)
        payload = encode_data(DataHeader(kind, 0, self.session), body, cap)
        self._transmit([TxFrame(Container.DATA, payload, mode=mode, rv=0)])
        if kind is DataKind.CONNECT_REQ:
            self._connect_tries += 1
            # backoff that widens with each retry, so two stations that called each other
            # at the same instant desynchronise instead of colliding on every attempt
            frame_s = self.timing.data_frame_s_for(mode)
            span = (1 + self._connect_tries) * frame_s
            wait = self._response_wait(frame_s) + self.rng.uniform(0.0, span)
            self._arm("connect", self._tx_busy_until - self.now + wait)

    def _send_probe(self, kind: DataKind, remote: str, snr_db: float | None) -> None:
        """A PROBE (``snr_db`` absent) or a PROBE_ACK (the SNR the probe arrived at),
        outside any session: session 0, sequence 0, the most robust mode."""
        body = ProbeBody(self.my_call, remote, snr_db, caps=self.cfg.capabilities).encode()
        mode = self._robust_mode(False)
        cap = self.timing.capacity(mode)
        payload = encode_data(DataHeader(kind, 0, 0), body, cap)
        self._transmit([TxFrame(Container.DATA, payload, mode=mode, rv=0)])

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
        self._wait_for("poll", self.timing.control_frame_s_for(self._control_floor()))

    def _send_turn(self) -> None:
        self._turn_tries += 1
        self.stats.turns += 1
        self._transmit([self._control(ControlKind.TURN)])
        self.role = Role.IRS
        self._bursts_since_turn = 0
        self._peer_wants_tx = self._peer_break = False
        self._disarm("keepalive")
        # the peer answers with its first burst (or a POLL); we wait a full data frame
        self._wait_for("turn", self.timing.data_frame_s_for(self.rate.recommend()))
        self.actions.append(Event("role", "irs"))

    def _send_disc(self) -> None:
        self._disc_tries += 1
        self.state = State.DISCONNECTING
        self._transmit([self._control(ControlKind.DISC)])
        self._wait_for("disc", self.timing.control_frame_s_for(self._control_floor()))

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
            self._back_off()
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
        mode = self._burst_mode()
        recommendation = min(self._recommended, self.cfg.max_mode)
        unacked = self._unacked()
        # A frame sent max_combines times at its mode without an acknowledgement is
        # stranded there: the peer has reset its buffer for it, and another round at the
        # same mode has the odds the last one had. When the recommendation has moved
        # below that mode, the frame is re-encoded at the slowest mode down to it that
        # carries the body — another codeword, which the peer starts fresh on. A full
        # frame has nowhere slower to go and keeps trying; the ladder (P6-7) sends small
        # bodies for that reason, and so should anything that expects to fall far.
        for s in unacked:
            rec = self._records[s]
            if rec.tx_count >= self.cfg.max_combines and recommendation < rec.mode:
                target = self._reencode_target(len(rec.body), rec.mode, recommendation)
                if target is not None:
                    rec.mode = target
                    rec.reencoded += rec.tx_count
                    rec.tx_count = 0
                    self.stats.frames_reencoded += 1
        family = self.timing.is_floor(mode)
        # One family per burst (ADR-0009): the receiver infers a frame's slot from its air
        # time, which needs every frame of the burst to be the same length. A frame keeps
        # its codeword — and so its mode — across retransmissions, so when the oldest
        # unacknowledged frame is of the other family the burst carries that family's
        # retransmissions alone and new frames wait for the next one.
        if unacked:
            family = self.timing.is_floor(self._records[unacked[0]].mode)
        seqs = [s for s in unacked if self.timing.is_floor(self._records[s].mode) == family]
        seqs = seqs[: self.cfg.burst_frames]
        new_frames = family == self.timing.is_floor(mode)
        cap = data_capacity(self.timing.capacity(mode))
        while (
            new_frames
            and len(seqs) < min(self.cfg.burst_frames, MAX_BURST)
            and self._outstanding() < WINDOW
            and self._tx_queue
        ):
            take = min(cap, len(self._tx_queue))
            if self._pin_body is not None:
                take = min(take, self._pin_body)
            # A body one byte short of full is the one length the container cannot carry:
            # it is partial, so it needs its two length bytes, and then it no longer fits.
            # Leave one more byte for the next frame instead of failing on the air.
            if take == cap - 1:
                take -= 1
            body = bytes(self._tx_queue[:take])
            del self._tx_queue[:take]
            rec = _TxRecord(self._tx_next, DataKind.DATA, body, mode)
            self._records[rec.seq] = rec
            self._tx_next = seq_after(self._tx_next)
            seqs.append(rec.seq)
        if not seqs:
            return
        if self._pinned is not None:
            fresh = [
                s for s in seqs if not (self._records[s].tx_count or self._records[s].reencoded)
            ]
            if fresh:
                self._ladder_pending = (mode, fresh)
        frames = []
        for s in seqs:
            rec = self._records[s]
            if rec.tx_count or rec.reencoded:
                self.stats.frames_resent += 1
            frames.append(self._data_frame(rec))
        self.stats.frames_sent += len(frames)
        self.stats.bursts += 1
        self._burst_seqs = seqs
        self._bursts_since_turn += 1
        self._disarm("keepalive")
        self._transmit(frames)
        # the IRS's quiet after this burst: its family, and the longest frame it will expect
        # — this burst's, or the mode it recommended, as its own _peer_data_frame_s has it
        expected = max(
            self.timing.data_frame_s_for(self._records[seqs[0]].mode) if seqs else 0.0,
            self.timing.data_frame_s_for(min(self._recommended, self.cfg.max_mode)),
        )
        self._wait_for(
            "ack",
            self.timing.control_frame_s_for(family),
            self._irs_reply_delay(family, expected),
        )

    def _on_ack(self, ack: ControlFrame) -> None:
        self.stats.acks_received += 1
        self._retries = 0
        if self._ladder_pending is not None:
            pinned_mode, fresh = self._ladder_pending
            self._ladder_pending = None
            decoded = sum(1 for s in fresh if ack.received(s))
            self.ladder.append(LadderRung(pinned_mode, len(fresh), decoded, ack.snr_db))
        for s, rec in list(self._records.items()):
            if not rec.acked and rec.tx_count and ack.received(s):
                rec.acked = True
                self.stats.bytes_acked += len(rec.body)
        while self._tx_base != self._tx_next and self._records[self._tx_base].acked:
            del self._records[self._tx_base]
            self._tx_base = seq_after(self._tx_base)
        self._recommended = max(0, min(self.cfg.max_mode, ack.recommended_mode))
        if ack.snr_db is not None:
            self.peer_snr_db = ack.snr_db
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
        frame_s = self.timing.data_frame_s_for(frame.mode)
        return max(0, round((frame.t_start - self._burst_t0) / frame_s))

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
        if frame.floor != self.timing.is_floor(frame.mode):
            # a floor frame whose chips name an ordinary mode, or the reverse: the chips
            # are noise — a false detection, most likely — and so are its SNR and its soft
            # bits; it is not part of any burst
            self.stats.frames_failed += 1
            return
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
        if guess in self._harq and self._harq[guess][2] != rec.frame.mode:
            del self._harq[guess]  # re-encoded at another mode: start over
        if guess in self._harq:
            prev, combines, _ = self._harq[guess]
            combined, merged = rec.frame.decode(prev)
            if combined is not None:
                self.stats.harq_rescues += 1
                self._accept(rec, combined)
                return
            self._harq[guess] = (
                (buffer, 0, rec.frame.mode)
                if combines + 1 >= self.cfg.max_combines
                else (merged, combines + 1, rec.frame.mode)
            )
        else:
            self._harq[guess] = (buffer, 0, rec.frame.mode)
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
        self._note_peer_frame(rec.frame)
        self._last_peer_frame = self.now
        self._arm("link", self._link_timeout())
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
        self._note_peer_frame(frame)
        if (
            self.state is State.IDLE
            or self.state is State.CONNECTING
            or ctl.session != self.session
        ):
            return
        self._last_peer_frame = self.now
        self._arm("link", self._link_timeout())
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
        # the first burst of a turn goes out where this station's own measurements of the
        # peer put it — HF is reciprocal — as a caller's goes out where the acceptance puts
        # it (P9-2), not at the slowest rung of the ladder: that is the tone floor
        # (ADR-0013), five times slower than the first OFDM mode
        if self.rate.snr_db is not None:
            first = self.rate.first_mode(self.rate.snr_db)
            self._recommended = max(self.cfg.initial_mode, min(self.cfg.max_mode, first))
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
        self._note_peer_frame(frame)
        if header.kind is DataKind.CONNECT_REQ:
            self._handle_connect_req(header, body, frame.snr_db)
        elif header.kind is DataKind.CONNECT_ACK:
            self._handle_connect_ack(header, body, frame.snr_db)
        elif header.kind is DataKind.PROBE:
            self._handle_probe(body, frame)
        elif header.kind is DataKind.PROBE_ACK:
            self._handle_probe_ack(body, frame)

    def _handle_probe(self, body: bytes, frame: SoftFrame) -> None:
        try:
            req = ProbeBody.decode(body)
        except ValueError:
            return
        if req.dst not in self.callsigns:
            return
        if bandwidth_code(req.caps) != bandwidth_code(self.cfg.capabilities):
            self.actions.append(Event("ignored", f"{req.src} probes in another bandwidth"))
            return
        if self.state is not State.IDLE:
            return  # a session's frames matter more than a question from outside it
        # answer as the callsign that was probed, with the SNR the probe arrived at —
        # the one number the prober cannot measure for itself
        self.my_call = req.dst
        self.stats.probes_answered += 1
        self.actions.append(Event("probed", f"{req.src} at {frame.snr_db:.1f} dB"))
        self._send_probe(DataKind.PROBE_ACK, req.src, frame.snr_db)

    def _handle_probe_ack(self, body: bytes, frame: SoftFrame) -> None:
        try:
            ack = ProbeBody.decode(body)
        except ValueError:
            return
        if self._probing is None or ack.dst != self.my_call or ack.src != self._probing:
            return
        self._disarm("probe")
        self._probing = None
        self.stats.probe_replies += 1
        self.last_probe = ProbeResult(ack.src, ack.snr_db, frame.snr_db)
        theirs = "?" if ack.snr_db is None else f"{ack.snr_db:.0f}"
        self.actions.append(
            Event("probe", f"{ack.src} hears us at {theirs} dB, heard at {frame.snr_db:.1f} dB")
        )

    def _handle_connect_req(self, header: DataHeader, body: bytes, snr_db: float) -> None:
        try:
            req = ConnectBody.decode(body)
        except ValueError:
            return
        if req.dst not in self.callsigns:
            return
        if bandwidth_code(req.caps) != bandwidth_code(self.cfg.capabilities):
            # a call that says it was made in another bandwidth than this station's: the
            # frame decoded, so the claim is wrong, or the station is not set up for the
            # bandwidth it was called in — either way not a session to start
            self.actions.append(Event("ignored", f"{req.src} calls in another bandwidth"))
            return
        if req.version != PROTOCOL_VERSION:
            self.actions.append(
                Event("ignored", f"{req.src} calls with link protocol {req.version}")
            )
            return
        if self.state is State.CONNECTING and self.my_call > self.remote_call:
            return  # simultaneous call: the higher callsign keeps calling
        self.my_call = req.dst  # answer as the callsign that was called
        self.remote_call = req.src
        self.session = header.session
        self._disarm("connect")
        self._reset_transfer_state()
        self.peer_capabilities = req.caps
        self.state = State.CONNECTED
        self.role = Role.IRS
        self._confirmed = False
        self._last_peer_frame = self.now
        self._arm("link", self._link_timeout())
        # the request is the first measurement of how the caller is heard: the
        # controller starts from it, and the acceptance carries it back so the caller's
        # first burst can too (P9-2)
        self.rate.seed(snr_db)
        self._send_connect(DataKind.CONNECT_ACK, snr_db)
        self.actions.append(Event("connected", f"{self.remote_call} (irs)"))

    def _handle_connect_ack(self, header: DataHeader, body: bytes, snr_db: float) -> None:
        if self.state is not State.CONNECTING or header.session != self.session:
            return
        try:
            ack = ConnectBody.decode(body)
        except ValueError:
            return
        if ack.dst != self.my_call:
            return
        if bandwidth_code(ack.caps) != bandwidth_code(self.cfg.capabilities):
            self.actions.append(Event("ignored", f"{ack.src} answers in another bandwidth"))
            return
        if ack.version != PROTOCOL_VERSION:
            self.actions.append(
                Event("ignored", f"{ack.src} answers with link protocol {ack.version}")
            )
            return
        self._disarm("connect")
        self.peer_capabilities = ack.caps
        self.state = State.CONNECTED
        self.role = Role.ISS
        self._confirmed = True
        self._last_peer_frame = self.now
        self._arm("link", self._link_timeout())
        self.actions.append(Event("connected", f"{self.remote_call} (iss)"))
        # the acceptance says how the request was heard: the first burst starts at what
        # that supports, less a step, instead of at the slowest mode; the acceptance's
        # own SNR is how the other station is heard here, which this station's
        # controller starts from for the day it receives (P9-2)
        self.rate.seed(snr_db)
        self._recommended = self.cfg.initial_mode
        if ack.snr_db is not None:
            self._recommended = max(self.cfg.initial_mode, self.rate.first_mode(ack.snr_db))
        if self._has_work():
            self._send_burst()
        else:
            self._send_poll()  # confirms the handshake and fetches the first ACK

    def _rate_controller(self) -> RateController:
        """A fresh controller for the PHY's mode table: its thresholds when the timing
        carries them, the wide waveform's otherwise; its capacities decide which modes
        another mode beats on both counts and are never recommended."""
        thresholds = self.timing.mode_threshold_db
        floor = {
            "floor_modes": self.timing.floor_modes,
            "floor_margin_db": self.timing.floor_margin_db,
        }
        if thresholds is None:
            return RateController(**{**floor, **self.cfg.rate})  # type: ignore[arg-type]
        payload = self.timing.data_capacity or {m: 0 for m in thresholds}
        frame_s = {m: self.timing.data_frame_s_for(m) for m in thresholds}
        return RateController(
            thresholds=dict(thresholds),
            modes=usable_modes(thresholds, payload, frame_s),
            **{**floor, **self.cfg.rate},  # type: ignore[arg-type]
        )

    def _reset_transfer_state(self) -> None:
        self._records.clear()
        self._tx_base = self._tx_next = 0
        self._burst_seqs = []
        self._retries = 0
        self._bursts_since_turn = 0
        self._peer_wants_tx = self._peer_break = False
        self._recommended = self.cfg.initial_mode
        self.peer_snr_db = None
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
        self.rate = self._rate_controller()
        for name in ("ack", "wait", "keepalive", "link"):
            self._disarm(name)

    def _end_session(self, reason: str) -> None:
        self.state = State.IDLE
        self.role = Role.NONE
        self._reset_transfer_state()
        self._disarm("connect")
        self.actions.append(Event("disconnected", reason))
