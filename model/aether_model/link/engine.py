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

import contextlib
import math
import random
from collections.abc import Sequence
from dataclasses import dataclass, field, replace
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
    bandwidth_hz_of,
    countdown_of,
    data_capacity,
    decode_data,
    encode_data,
    in_window,
    most_following,
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
    max_burst_s: float | None = None
    """The longest a burst may be on the air, seconds — the transmitter's key-time limit
    less the keying's lead and tail and anything appended to a burst (a Morse identifier).
    A burst carries at most as many frames as fit, one at least. Unset, the frame count
    alone decides. Six tone frames (ADR-0013) are 32 s, and the daemon's 30 s key watchdog
    cut the last one of every full tone burst on the air (ND1J, 2026-09-25): a frame lost
    to the station's own watchdog reads to the rate controller as a frame lost to the
    channel, and each burst that failed so stepped the link further down the tone floor,
    where every burst was full again (ADR-0017)."""
    max_retries: int = 8
    """Consecutive unanswered bursts / polls before the link is declared dead — and the most
    TURNs a station offering the turn sends while the other station may have taken it (see
    :attr:`turn_retries`)."""
    connect_retries: int = 8
    turn_retries: int = 3
    """TURNs a station sends, unanswered, before it takes the turn back — unless the last thing
    it heard from the other station meanwhile was a frame it could not read: that may be the
    other station's answer from the turn it now holds, a poll too weak to read, and taking the
    turn back then made two senders; it offers again, up to :attr:`max_retries` TURNs
    (ADR-0029). An acknowledgement it can read says the other station is still receiving, and
    silence says nothing either way: after those the turn is taken back as it always was."""
    disc_retries: int = 3
    disc_patience_exchanges: float = 2.0
    """A sender asked to disconnect finishes what it has queued first — and gives up on it, and
    sends its DISC, once this many whole exchanges at the family the link runs in (or
    :attr:`disc_patience_s`, whichever is longer) pass with nothing new acknowledged (ADR-0039).
    On a path where the other station's acknowledgements do not arrive the queue never
    empties, and Disconnect did nothing until the link timed out: "the disconnect button does
    not work" (ND1J, 2026-10-05). Half the link timeout (:attr:`link_timeout_exchanges`), so it
    always comes first; any progress starts the count again."""
    disc_patience_s: float = 20.0
    """See :attr:`disc_patience_exchanges`."""
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
    rate: dict[str, float | int | None] = field(default_factory=dict)
    """Overrides for the rate controller's tunables (``RateController`` fields by name),
    for benches that compare one controller against another; empty means the defaults."""
    initial_mode: int = 0
    """The slowest mode a session's first burst goes out at. The acceptance's SNR report
    picks the first mode (P9-2); this is the floor under it, which a bench that pins a
    mode sets along with :attr:`max_mode`."""
    max_mode: int = 19
    """The fastest rung the station sends at: the top of the widest ladder (2 300 Hz,
    ADR-0014) by default — a recommendation never leaves the air's own table."""
    ceiling: int | None = None
    """The fastest rung the rules allow this station to send at, where it is now — the
    regulatory policy's answer (ADR-0018), which link adaptation never overrides: an
    automatically controlled station answering outside the §97.221(b) segments may not
    climb past the rungs that occupy 500 Hz or less, however good the path. Unlike
    :attr:`max_mode`, which is the operator's taste, it reaches every frame the station
    sends: when it admits only tone-floor rungs, control frames, connect requests and
    answers, and probe answers go on the floor too. Unset, only :attr:`max_mode` applies."""
    bursts_before_turn: int = 3
    """With a WANT_TX peer, the ISS hands over after this many bursts of its own."""
    silence_step: int = 2
    """Usable modes the ISS steps its own recommendation down by for every burst that goes
    unanswered (P9-7), and for every poll from the :attr:`poll_silences`-th in a row
    (ADR-0032). The recommendation otherwise moves only when an acknowledgement brings one,
    and on a fading path the burst and its acknowledgement fade together: a session whose
    first bursts went out on a connect frame measured at a peak repeated them at a mode the
    path could not carry until the link timed out, one session in twenty at 0–3 dB on the
    500 Hz fading bench. Two steps a silence reaches the bottom of either table within the
    retries, and the frames stranded up there are re-encoded on the way (after
    :attr:`max_combines`); the next acknowledgement puts the peer's own recommendation back."""
    poll_silences: int = 2
    """Unanswered polls in a row after which each further silence steps the recommendation
    down, as every unanswered burst's does (:attr:`silence_step`) — and a poll goes out in
    the family of that recommendation, so the polls reach the tone floor within the retries
    (ADR-0032). Repeated in the ordinary family, a poll and its answer lost together in a slow
    fade were lost again every time until the retries ran out: "no response", with the floor,
    14 dB lower, never tried. One silence is no fade: near the ordinary control frame's
    threshold a poll or its answer is lost now and then on a path that carries the next, and
    a repeat on the floor costs 3.2 s and an answer as long."""
    max_combines: int = 4
    """HARQ buffers are reset after this many failed combines (guards a wrong inference)."""
    reencode_early_after: int = 2
    """A frame the peer measured further below its rung's threshold than combining all
    :attr:`max_combines` of its transmissions could make up — 10·log10 of their number, plus
    :attr:`reencode_hopeless_db` — is re-encoded after this many instead (ADR-0038). Its
    copies were being combined toward a sum that could never decode: ND1J's rung-12 frames,
    12 dB under their threshold, went out eight times each (2026-10-05)."""
    reencode_hopeless_db: float = 1.0
    """See :attr:`reencode_early_after`."""
    capabilities: int = 0
    """Capability bits offered in the connect handshake. What they mean is the caller's
    business; the link layer carries them and reports what the peer offered. Bit 0 is stream
    compression (deflate, RFC 1951); bits 1–2 state the bandwidth this station transmits in
    (:func:`~aether_model.link.frames.with_bandwidth`), and a request or answer stating
    another is ignored — see ``docs/spec/air-interface.md`` §7.3."""
    chat: bool = True
    """The station without the turn asks for it (ADR-0027, in every session since ADR-0044). A
    message queued at the station that does not hold the turn would otherwise wait for the
    sender's next poll — up to :attr:`keepalive_s` on an idle link — and then for the poll's
    answer and a TURN before its own burst could go: 13 s a change of direction at +6 to +12 dB,
    and a Winlink session changes direction at every proposal, answer and message. Instead that
    station asks as soon as the channel is quiet: an acknowledgement nobody asked for, with
    WANT_TX, which an idle sender answers with a TURN; and an idle sender does not poll over a
    frame it hears arriving, since that frame may be such a request. On the link bench with a
    host that answers at once (Winlink's B2F shape), a reply's median wait falls from 13 s to
    3.4–6 s at +6 to +12 dB, with no line lost and slightly less keyed time. Named for where it
    began — a host's ``CHAT ON`` — and kept switchable (:meth:`LinkEngine.set_chat`) for the
    benches that compare it with the engine before."""
    offer_turn: bool = False
    """A burst that empties this station's queue ends with a TURN that offers the turn, and a
    receiving station with data of its own takes it in its acknowledgement — the acknowledgement
    and its first burst in one transmission (ADR-0047) — instead of an acknowledgement asking for
    the turn, a TURN, and then the burst. Proposed, and off: on the link bench the offer costs the
    air the TURN it replaces did, and what it saves — a keying of the transmitter — is what the
    bench does not charge; the scenario harness is where it is to be decided."""


OUTSIDE_SESSIONS = frozenset(
    {DataKind.BEACON, DataKind.PROBE, DataKind.PROBE_ACK, DataKind.DATAGRAM}
)
"""DATA kinds sent outside any session: never taken into a session's stream."""


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

    @property
    def sent(self) -> bool:
        """It has gone on the air, under this codeword or an earlier one. A re-encoding starts
        :attr:`tx_count` again, and a frame re-encoded into a burst with no room left for it —
        six stranded OFDM frames given tone codewords, five of which fit the key time — has
        not gone under its new one; it is unacknowledged all the same (ADR-0031)."""
        return self.tx_count > 0 or self.reencoded > 0


@dataclass
class _RxRecord:
    frame: SoftFrame
    slot: int
    payload: bytes | None = None
    seq: int | None = None
    """Sequence number if decoded, else the inferred guess (may be None)."""
    buffer: object = None
    """This transmission's soft-decode output, kept for HARQ combining."""
    combined: bool = False
    """It was combined with an earlier transmission of its block: a failure after that is a
    failure of both."""
    outside: bool = False
    """It decoded as a frame of nobody's session — a beacon, a probe, a datagram, another
    session's — and is no part of the burst (ADR-0038)."""


SELF_DECODABLE_RVS = frozenset({0, 3})
"""The redundancy versions that carry the systematic bits (TS 38.212 §5.4.2.1: RV 0 starts at
them, RV 3 wraps round to them): a frame at RV 1 or 2 is mostly parity and, at the rates this
modem runs, does not decode on its own at any SNR — 6 and 10 % on ND1J's path, 2026-09-25,
against 75 % at RV 0 — which is why a retransmission is combined with what came before."""

RV_SEQUENCE = (0, 0, 2, 3)
"""The redundancy version of a frame's first, second, third and fourth transmission under one
codeword, then round again (ADR-0043). On HF a retransmission is as often of a frame the
receiver never detected — a fade, a collision, a burst's faded end — as of one it detected and
could not decode, so the second copy must decode on its own: RV 0 again, which also combines
with a failed first copy as well as any other (27 of 29 against RV 1's 24, 2300 Hz Poor). RV 2
then adds the parity neither RV 0 carries, and RV 3 the systematic bits with the code's
outermost parity. The order was 0, 1, 2, 3, under which a frame whose first copy was missed
could not decode until its fourth: RV 1 and RV 2 decode alone at no SNR, and RV 3 not at all on
the 36 bit/s tone rung (85 s of tone frames at +5 dB on a 500 Hz path, the scenario harness)."""


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
    turn_requests: int = 0
    """Acknowledgements this station sent unasked, to ask for the turn in a chat
    (:attr:`LinkConfig.chat`)."""
    turn_offers: int = 0
    """Bursts that ended with the turn on offer (ADR-0047)."""
    turns_taken: int = 0
    """Acknowledgements that took the turn offered, this station's burst after them (ADR-0047)."""


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
    bandwidth_hz: int | None = None
    """The bandwidth the answering station runs, from its answer's capability byte — a
    probe is answered across bandwidths (ADR-0035), and this is how the prober learns that
    a call would not be; ``None`` for a reserved code."""


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
        self._offered = False
        """This station, receiving, was offered the turn at the end of the burst (ADR-0047)."""
        self._offer_out = False
        """The last burst this station sent offered the turn, and nothing has answered it."""
        self._retries = 0
        self._bursts_since_turn = 0
        self._peer_wants_tx = False
        self._peer_break = False
        self._recommended = self.cfg.initial_mode
        # the SNR the peer measured on our last burst, carried in its ACK — or on our last
        # frame, carried in its other control frames (ADR-0021): the one number an operator
        # cannot get from their own receiver
        self.peer_snr_db: float | None = None
        self.ended_peer_snr_db: float | None = None
        """:attr:`peer_snr_db` as the session that ended last left it: a disconnect is the
        frame that carries it to a station that only received, and a session's account is
        written once the session has ended (ADR-0021)."""
        self._heard_peer_db: float | None = None
        """The SNR of the last frame of this session decoded from the other station: what this
        station's control frames other than acknowledgements say of how it hears it."""
        self._turn_tries = 0
        self._turn_unread = False
        """While this station offers the turn: the last frame it heard from the other station
        since the first TURN was one it could not read — which may be the other station's
        answer from the turn it now holds (ADR-0029). An acknowledgement it can read clears it:
        the other station is still receiving."""
        self._unread_there = False
        """The other station answered this station's last POLL or TURN without reading it: it
        heard the frame's preamble, could not decode the frame, and answered all the same, as a
        receiving station answers a burst that may be ending. The frame goes again at once, and
        this station's POLLs and TURNs go out on the tone floor until one is read (ADR-0034). A
        station that reads a TURN answers with a burst or a poll, never an acknowledgement; one
        that reads a POLL answers in the POLL's family, since it answers in the family it last
        read the sender in — so a floor answer to an ordinary POLL did not read it."""
        self._poll_floor = False
        """The family this station's last POLL went out in (ADR-0034)."""
        self._disc_requested = False
        self._disc_patience_until: float | None = None
        """When a sender asked to disconnect stops waiting for its queue (ADR-0039)."""
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
        self._asked: int | None = None
        """The fastest rung this station has recommended to the sender in this session: a
        burst faster than any of them is the sender's choice (ADR-0020)."""
        self._break_requested = False
        self._stated_want = False
        """This station's last acknowledgement asked for the turn (WANT_TX): the other station
        knows it has something to send, and a request in a chat (:attr:`LinkConfig.chat`)
        would tell it nothing new."""
        self._confirmed = False
        self._caller = False
        """This station placed the call: when both stations believe they hold the turn, the
        caller keeps it and the called station yields (ADR-0023)."""
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

        One PROBE frame, on the tone floor — the most robust frame there is, which a probe
        exists to measure a weak path with (ADR-0016); the answer, if it comes, arrives as a
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
        self._send_probe(DataKind.PROBE, self._probing, None, floor=True)
        wait = self._response_wait(self.timing.data_frame_s_for(self._robust_mode(True)))
        self._arm("probe", self._tx_busy_until - self.now + wait)

    def disconnect(self) -> None:
        """Orderly close: finish sending every queued byte, get it acknowledged, then
        DISC / DISC_ACK. Requesting this while still connecting just means "disconnect once
        the transfer is done"; only :meth:`abort` tears down a half-open session.

        A receiving station has nothing of its own to finish, and leaves between the other
        station's bursts: its DISC goes at once unless a burst is arriving, whose
        acknowledgement it then replaces. Waiting to put it in place of the next
        acknowledgement waited for ever on a path where nothing decodable came — five
        sessions with KE4QCM on 2026-09-25 ended with Abort (ADR-0023)."""
        if self.state is State.IDLE:
            return
        self._disc_requested = True
        if self.state is State.CONNECTING:
            return
        if self.role is Role.ISS and self._waiting_for is None and not self._tx_busy():
            self._maybe_start_burst()
        elif (
            self.role is Role.IRS
            and self.state is State.CONNECTED
            and not self._burst
            and "ack" not in self._deadlines
            and not self._tx_busy()
        ):
            self._send_disc()

    @property
    def disconnect_requested(self) -> bool:
        """A disconnect was asked for and the DISC has not gone yet: the sender is finishing
        what it has queued, or the receiver is waiting for a burst to end."""
        return self._disc_requested and self.state is State.CONNECTED

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
        else:
            self._maybe_request_turn()

    def set_chat(self, on: bool) -> None:
        """The turn request on or off, from now on (:attr:`LinkConfig.chat`) — on in every
        session (ADR-0044); off only for a bench comparing the engine before. Was: the host
        program's ``CHAT ON`` / ``CHAT OFF``, which may come in the middle of a session."""
        self.cfg.chat = on
        if not on:
            self._disarm("request")

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
    def tx_undelivered_bytes(self) -> int:
        """Bytes handed to :meth:`send` that have not yet reached the other station's
        application: the queue not yet framed, and every frame from the lowest unacknowledged
        one up. A frame acknowledged past a hole still counts — the receiving station hands
        the stream on in order, so its bytes wait with the hole — which is where this differs
        from :attr:`tx_pending_bytes`. What a session's :meth:`send` calls were given, less
        this, has arrived: a panel marks a message delivered from it."""
        return len(self._tx_queue) + sum(len(r.body) for r in self._records.values())

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
        else:
            self._maybe_request_turn()

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
        elif not (self.state is State.DISCONNECTING and self.role is Role.ISS):
            # A sender waiting for the answer to its DISC has no burst to acknowledge. What it
            # hears is the answer's company — the other station's Morse identifier, which a
            # detector can take for a data frame — or a burst from a peer that missed the
            # DISC and will hear the next one. Taken as a burst, it armed an acknowledgement,
            # and a leaving station's acknowledgement is another DISC: ND1J, 2026-09-25, two
            # in one keying, the second over his identifier.
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
        if self.state is State.CONNECTING or self._probing is not None:
            # A caller does not call over a frame it hears arriving, and a prober does not
            # give up on one: it may be the answer, and if it is another station's the
            # channel is busy. The next try (or the probe's deadline) waits for its end. A
            # called station that has accepted a call whose acceptance was lost is
            # connected, and acknowledges the undecodable preamble of the caller's next try
            # on the floor — which the try after that ran into, again and again, until the
            # caller gave up (ADR-0016). The longest frame there is when the PHY cannot say.
            length = frame_s if frame_s is not None else self.timing.data_frame_s_for(0)
            clear = t_start + length + self.timing.turnaround_s
            for timer in ("connect", "probe"):
                if timer in self._deadlines:
                    self._deadlines[timer] = max(self._deadlines[timer], clear)
            return
        if (
            self.state is State.DISCONNECTING or self._waiting_for in ("turn", "ack", "poll")
        ) and "wait" in self._deadlines:
            # Nor does a leaving station repeat its DISC, a station that handed over the turn
            # its TURN, or a sender its burst or its poll, over a frame it hears arriving: it
            # may be the answer, late, and a repeat keyed over it is heard by nobody. A
            # receiving station that cannot decode a poll answers its preamble once its quiet
            # after a frame has passed — 0.99 s on the floor — and a 3.2 s acknowledgement
            # ended 0.2 s into the sender's re-poll, every time, until the sender gave up with
            # the link up (ADR-0022, ADR-0023, ADR-0030).
            length = frame_s if frame_s is not None else self.timing.data_frame_s_for(0)
            clear = t_start + length + self._response_wait(0.0)
            self._deadlines["wait"] = max(self._deadlines["wait"], clear)
        if self.cfg.chat and self.role is Role.ISS and "keepalive" in self._deadlines:
            # Nor does an idle sender in a chat poll over a frame it hears arriving: there the
            # receiving station speaks unasked, and a poll keyed over its request loses both —
            # at −6 dB on the tone floor that cost a typical line 2–3 s (ADR-0027).
            length = frame_s if frame_s is not None else self.timing.data_frame_s_for(0)
            clear = t_start + length + self._response_wait(0.0)
            self._deadlines["keepalive"] = max(self._deadlines["keepalive"], clear)
        if self._waiting_for == "turn":
            # until it is read, a frame heard after a TURN may be the answer to it (ADR-0029)
            self._turn_unread = True
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
        elif name == "request":
            self._maybe_request_turn()

    def _tx_busy(self) -> bool:
        return self.now < self._tx_busy_until

    # ── transmit helpers ──────────────────────────────────────────────

    def _transmit(self, frames: list[TxFrame]) -> None:
        dur = sum(self.timing.frame_s(f) for f in frames)
        self._tx_busy_until = self.now + self.timing.tx_latency_s + dur
        self.actions.append(Transmit(frames, dur))
        if "link" in self._deadlines:
            # The link timeout spans four exchanges at the family the link runs in, and was
            # reckoned only when a frame arrived: a sender whose OFDM bursts went unanswered
            # stepped down to the floor still holding the 45 s its last acknowledgement had
            # armed, which ran out during its first tone burst with the answer to it on its
            # way. Whenever this station sends, the deadline is the last frame heard plus the
            # timeout for the family the link runs in now, never less than it was (ADR-0033).
            self._deadlines["link"] = max(
                self._deadlines["link"], self._last_peer_frame + self._link_timeout()
            )

    def _control(self, kind: ControlKind, **kw: object) -> TxFrame:
        if kind is not ControlKind.ACK:
            # every control frame says how its sender hears the other station (ADR-0021): an
            # acknowledgement says it of the burst it answers; a poll, a turn, a disconnect
            # and its answer of the last frame heard — so a station that only received, and
            # was never acknowledged, still learns how it was heard
            kw.setdefault("snr_db", self._heard_peer_db)
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
        for it. The ISS's go on the floor while the other station answers them unread
        (ADR-0034)."""
        if self._floor_only():
            return True
        if self.role is Role.ISS and self.state is State.CONNECTED:
            return self._unread_there or self.timing.is_floor(self._burst_mode())
        return self._peer_floor

    def _hopeless(self, mode: int) -> bool:
        """Whether the peer's last measurement of this station's frames is further below
        ``mode``'s threshold than :attr:`LinkConfig.max_combines` transmissions combined could
        make up (ADR-0038). Only a measurement will do: the rung the peer recommends sits a
        margin and the ladder's spacing below the path, and on the tone floor 15 dB below a
        frame that combining still rescues."""
        thresholds = self.rate.thresholds
        if self.peer_snr_db is None or mode not in thresholds:
            return False
        reach = thresholds[mode] - 10.0 * math.log10(max(1, self.cfg.max_combines))
        return self.peer_snr_db < reach - self.cfg.reencode_hopeless_db

    def _burst_mode(self) -> int:
        """The mode the next burst's new frames go out at: the pin while one is set, the
        peer's recommendation otherwise, never past the operator's ceiling."""
        chosen = self._recommended if self._pinned is None else self._pinned
        return min(chosen, self._cap())

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
        """An unanswered burst or poll is evidence too: step the recommendation down
        :attr:`LinkConfig.silence_step` usable modes (never below the table's first)."""
        modes = self.rate.modes
        current = min(self._recommended, self._cap())
        below = [m for m in modes if m <= current]
        index = modes.index(below[-1]) if below else 0
        self._recommended = modes[max(0, index - self.cfg.silence_step)]

    def _exchange_s(self) -> float:
        """One whole exchange at the family the link runs in: a full burst of the longest
        data frame either side sends or may send next, the control frame of that family that
        answers it, both turnarounds and the gap that ends the burst."""
        frame = max(self._peer_data_frame_s(), self.timing.data_frame_s_for(self._burst_mode()))
        floor = self._peer_floor or self.timing.is_floor(self._burst_mode())
        return (
            self._burst_capacity(frame) * frame
            + self.timing.control_frame_s_for(floor)
            + 2 * self.timing.turnaround_s
            + self.cfg.burst_gap_s
        )

    def _link_timeout(self) -> float:
        """How long the peer may stay silent before the session ends: the configured time,
        or :attr:`LinkConfig.link_timeout_exchanges` whole exchanges (:meth:`_exchange_s`),
        whichever is longer."""
        return max(self.cfg.link_timeout_s, self.cfg.link_timeout_exchanges * self._exchange_s())

    def _disc_patience(self) -> float:
        """How long a sender asked to disconnect waits for its queue to be acknowledged before
        it leaves without the rest (ADR-0039)."""
        return max(self.cfg.disc_patience_s, self.cfg.disc_patience_exchanges * self._exchange_s())

    def _robust_mode(self, floor: bool) -> int:
        """The slowest mode of the given family whose frame carries a connect body (with a
        DATA header and length): what connect requests and their answers, probes and theirs,
        and beacons go out at — on the tone floor first (ADR-0016). Falls back to the
        ordinary family when the floor has no such mode."""
        need = CONNECT_BODY_BYTES + 5
        caps = self.timing.data_capacity or {}
        for m in sorted(caps):
            if self.timing.is_floor(m) == floor and caps[m] >= need:
                return m
        if floor:
            return self._robust_mode(False)
        raise ValueError("no mode carries a connect frame")

    def _connect_floor(self) -> bool:
        """Whether the next connect request goes out on the tone floor: the first try does,
        and every other one after it (ADR-0016). A call is made before anything is known of
        the path, so it goes where the path most likely carries it — the floor reaches 14 dB
        lower than the ordinary family's control rung — and the ordinary tries between keep a
        path the floor does not carry (a narrowband interferer on the floor's tones) from
        failing every one. ADR-0009 had the first two tries ordinary, from when the floor was
        an OFDM frame of its own that reached a few decibels lower; with the tone floor a
        weak path's first two tries were wasted, and a probe or a beacon never reached it."""
        if self.timing.floor_modes > 0 and self._floor_only():
            return True
        return self.timing.floor_modes > 0 and self._connect_tries % 2 == 0

    def _data_frame(self, rec: _TxRecord) -> TxFrame:
        rec.tx_count += 1
        cap = self.timing.capacity(rec.mode)
        payload = encode_data(DataHeader(rec.kind, rec.seq, self.session), rec.body, cap)
        rv = RV_SEQUENCE[(rec.tx_count - 1) % len(RV_SEQUENCE)]
        return TxFrame(Container.DATA, payload, mode=rec.mode, rv=rv)

    def _send_connect(self, kind: DataKind, snr_db: float | None = None) -> None:
        src, dst = self.my_call, self.remote_call
        body = ConnectBody(src, dst, caps=self.cfg.capabilities, snr_db=snr_db).encode()
        # a request starts on the tone floor and alternates families (ADR-0016); an answer
        # goes back on the layout the request arrived on
        floor = (
            self._connect_floor()
            if kind is DataKind.CONNECT_REQ
            else self._peer_floor or self._floor_only()
        )
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

    def _send_probe(
        self, kind: DataKind, remote: str, snr_db: float | None, *, floor: bool
    ) -> None:
        """A PROBE (``snr_db`` absent) or a PROBE_ACK (the SNR the probe arrived at),
        outside any session: session 0, sequence 0, at the robust mode of ``floor``'s family
        — a probe on the tone floor, an answer in the family the probe arrived in, as a
        connect answer goes (ADR-0016)."""
        body = ProbeBody(self.my_call, remote, snr_db, caps=self.cfg.capabilities).encode()
        mode = self._robust_mode(floor)
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
        poll = self._control(ControlKind.POLL)
        self._poll_floor = poll.floor
        self._transmit([poll])
        self._wait_for("poll", self._reply_control_s(), self._unread_poll_delay())

    def _unread_poll_delay(self) -> float:
        """How late after the poll's end its answer may begin. A receiving station that
        decodes the poll answers a turnaround after it; one that hears the poll's preamble
        and cannot decode the poll answers all the same, once its quiet after the frame has
        run out (:meth:`on_preamble`'s acknowledgement deadline) — the quiet of the family
        it last heard us in, which is ours or the one its last answer came in. Waiting as if
        every answer began a turnaround after the poll, an ordinary poll answered on the
        floor — 0.99 s of quiet, then 3.2 s — was repeated into the answer's end, every
        repeat into the next answer, until the sender gave up (ADR-0028). A poll no preamble
        report announces is never answered undecoded, and waits as it always did."""
        if self.timing.preamble_detect_s_for(self._control_floor()) is None:
            return 0.0
        # the longest frame there is, for a family whose frames the PHY does not announce
        longest = self.timing.data_frame_s_for(0)
        families = {self._control_floor(), self._peer_floor}
        return max(self._irs_reply_delay(f, longest) for f in families)

    def _send_turn(self) -> None:
        self._turn_tries += 1
        if self._turn_tries == 1:
            self._turn_unread = False
        self.stats.turns += 1
        self._transmit([self._control(ControlKind.TURN)])
        self.role = Role.IRS
        self._bursts_since_turn = 0
        self._peer_wants_tx = self._peer_break = False
        self._stated_want = False
        self._disarm("keepalive")
        # The peer answers with its first burst (or a POLL), at a rung of its own choosing —
        # after a fade the tone floor's, a frame five seconds long. Waiting one frame of the
        # rung this station recommends, a second of OFDM, repeated the TURN over the answer
        # until the tries ran out and both stations held the turn (KE4QCM, 2026-09-25,
        # ADR-0023): the wait covers the longest first frame there is.
        longest = max(
            self.timing.data_frame_s_for(0),
            self.timing.data_frame_s_for(self.rate.recommend()),
        )
        self._wait_for("turn", longest)
        self.actions.append(Event("role", "irs"))

    def _turn_offers(self) -> int:
        """How many TURNs go out before this station takes the turn back (ADR-0029).

        A TURN is answered by the other station's first burst or its poll, and a TURN heard
        again by a station that already holds the turn is answered again. When no answer is
        read, either the other station never read a TURN, or its answers are what is being
        lost — and then it holds the turn, and taking it back makes two senders: on the chat
        bench (ADR-0027) such a sender sent a 27 s burst over the other station's polls until
        one of the two gave up. A frame this station heard and could not read may be that
        answer — a poll too weak to read — and while the last one heard was such a frame the
        station offers again, up to :attr:`LinkConfig.max_retries` TURNs. An acknowledgement it
        can read says the other station is still receiving (it answered a TURN's preamble as
        the end of a burst), and silence says nothing either way: after either the turn is
        taken back at :attr:`LinkConfig.turn_retries`, as before. Silence has to stay that way:
        on a path that lets control frames through and no data, the station holding the turn
        answers every TURN with a burst nobody hears, and the poll a station that took the turn
        back sends — which the other answers by yielding (ADR-0023) — is what keeps the link
        alive; offered for as long, a link that had climbed timed out."""
        if self._turn_unread:
            return max(self.cfg.turn_retries, self.cfg.max_retries)
        return self.cfg.turn_retries

    def _send_disc(self) -> None:
        self._disc_tries += 1
        self.state = State.DISCONNECTING
        self._transmit([self._control(ControlKind.DISC)])
        self._wait_for("disc", self._reply_control_s())

    def _reply_control_s(self) -> float:
        """How long the control frame that answers ours may take: ours goes out in our
        family, and the answer comes back in it — or on the floor, from a station whose
        regulatory ceiling admits only the floor (ADR-0018), which is the family it last
        sent in. Waiting for ours alone, a poll was repeated every second and a half into
        a floor acknowledgement three seconds long, and the link timed out with both ends
        up. The longer of the two, as a burst's acknowledgement is waited for (ADR-0016)."""
        families = {self._control_floor(), self._peer_floor}
        return max(self.timing.control_frame_s_for(f) for f in families)

    def _on_response_timeout(self) -> None:
        what, self._waiting_for = self._waiting_for, None
        if self.state is State.DISCONNECTING or what == "disc":
            if self._disc_tries >= self.cfg.disc_retries:
                self._end_session("closed (no DISC_ACK)")
            else:
                self._send_disc()
            return
        if what == "turn":
            if self._turn_tries >= self._turn_offers():
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
            # A poll and its answer fade together as a burst and its acknowledgement do, and a
            # poll goes out in the family of the rung this station would send at: repeated as
            # it was, an ordinary poll into a fade below the ordinary control frame was lost
            # every time, until the retries ran out with the tone floor, 14 dB lower, never
            # tried — "no response" on the chat bench (ADR-0030 §3). From the second silence in
            # a row each steps the recommendation down as a burst's does, and the polls reach
            # the floor within the retries; the answer puts the other station's recommendation
            # back. A single silence is no fade, and its repeat stays in its family (ADR-0032).
            if self._retries >= self.cfg.poll_silences:
                self._back_off()
            self._send_poll()

    # ── ISS: bursts ───────────────────────────────────────────────────

    def _unacked(self) -> list[int]:
        seqs = [s for s, r in self._records.items() if not r.acked and r.sent]
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
        if self._disc_requested:
            # the queue is waited for, but not for ever: a path whose acknowledgements never
            # arrive never empties it (ADR-0039)
            if self._disc_patience_until is None:
                self._disc_patience_until = self.now + self._disc_patience()
            elif self.now >= self._disc_patience_until:
                left = len(self._tx_queue) + sum(
                    len(self._records[s].body) for s in self._unacked()
                )
                self.actions.append(
                    Event("disconnect", f"{left} bytes not acknowledged; leaving without them")
                )
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

    # ── chat: asking for the turn (ADR-0027) ──────────────────────────

    def _reaction_s(self) -> float:
        """How long after this station's last transmission the other station's answer to it
        may take to announce itself: a sender answers an acknowledgement at once — with its
        next burst, a TURN or a DISC — and until that much quiet has passed the channel is not
        known to be idle. The floor's frames announce themselves last; a PHY that reports no
        preambles is heard only at a frame's end, so the longest frame decides."""
        # the family the other station answers in — the one it was last heard in, as the
        # acknowledgement it answers went in the one this station last read (ADR-0045); a
        # station that has heard nothing of it yet allows for the longer
        families = (self._peer_floor,) if self._last_peer_frame > 0.0 else (False, True)
        announce = [self.timing.preamble_detect_s_for(floor) for floor in families]
        if any(a is None for a in announce):
            first = self.timing.data_frame_s_for(0)
        else:
            first = max(a for a in announce if a is not None)
        return self._response_wait(first)

    def _maybe_request_turn(self) -> None:
        """In a chat: a receiving station with something to send asks for the turn once the
        channel is quiet — now, or when the other station's answer to its last transmission
        has had time to begin (the ``request`` timer). Not while a burst is arriving or its
        acknowledgement is due (that acknowledgement asks), and not twice: once asked, the
        next acknowledgement asks again if the first was missed."""
        if (
            not self.cfg.chat
            or self.state is not State.CONNECTED
            or self.role is not Role.IRS
            or self._waiting_for is not None
            or self._stated_want
            or not self._has_work()
            or self._burst
            or "ack" in self._deadlines
            or self._tx_busy()
        ):
            return
        quiet_at = max(self._tx_busy_until, self._last_peer_frame) + self._reaction_s()
        if self.now < quiet_at - 1e-9:
            self._arm("request", quiet_at - self.now)
            return
        self._send_request()

    def _send_request(self) -> None:
        """An acknowledgement nobody asked for, with WANT_TX: what this station has received
        (unchanged since its last one, so it is a true acknowledgement too) and that it has
        something to send. A sender with nothing of its own hands over at once; a sender that
        misses it polls in its own time, and the answer to the poll asks again."""
        bitmap, _, _ = self._receive_window()
        self._stated_want = True
        self.stats.turn_requests += 1
        self._ack_counter = (self._ack_counter + 1) % 8
        recommended = self.rate.recommend()
        self._asked = recommended if self._asked is None else max(self._asked, recommended)
        self._disarm("request")
        self._transmit(
            [
                self._control(
                    ControlKind.ACK,
                    flags=ControlFlags.WANT_TX,
                    base=self._rx_base,
                    bitmap=bitmap,
                    snr_db=self._heard_peer_db,
                    recommended_mode=recommended,
                    counter=self._ack_counter,
                )
            ]
        )

    def _burst_capacity(self, frame_s: float) -> int:
        """Frames of ``frame_s`` seconds one burst may carry: the configured count, and no
        more than fit in :attr:`LinkConfig.max_burst_s` — one at least."""
        count = min(self.cfg.burst_frames, MAX_BURST)
        if self.cfg.max_burst_s is not None and frame_s > 0:
            count = min(count, max(1, int(self.cfg.max_burst_s / frame_s + 1e-9)))
        return count

    def set_max_burst_s(self, seconds: float | None) -> None:
        """A new limit on a burst's air time, from the next burst on: the key-time limit
        is a live setting of the station's."""
        self.cfg.max_burst_s = seconds

    def set_ceiling(self, rung: int | None) -> None:
        """A new regulatory ceiling (:attr:`LinkConfig.ceiling`), from the next frame on:
        the dial, the station's control or the session's direction changed what the rules
        allow."""
        self.cfg.ceiling = rung

    def set_max_mode(self, rung: int) -> None:
        """A new fastest rung (:attr:`LinkConfig.max_mode`), from the next burst on: the
        operator's ``max_mode`` is a live setting of the station's, and one set while the
        daemon ran used to reach the gate and the Test's ladder but not the link, which
        kept recommending up to the value it started with."""
        self.cfg.max_mode = rung

    def set_air(self, timing: PhyTiming, capabilities: int, max_mode: int | None = None) -> None:
        """Run on another air interface from the next session on (ADR-0026).

        The engine knows one air at a time — its frame lengths, its ladder, the thresholds its
        rate controller steps by — and the daemon moves it between sessions: to the bandwidth
        a host program's ``BW500``/``BW2300`` asked for, as VARA does, or to the narrower one
        a call to this station came in. ``capabilities`` is what the connect handshake offers
        from then on, its bandwidth bits (:func:`~aether_model.link.frames.with_bandwidth`)
        included, and ``max_mode`` the fastest rung on the new ladder. The callsigns, the
        counters, the session numbering and the last probe stay: the station is the same one.

        Refused while anything is under way — a session, a call, a probe: each runs to its end
        on the air it started on, and the other station is on that one."""
        if self.state is not State.IDLE or self._probing is not None:
            raise RuntimeError("a session, a call or a probe is running")
        self.timing = timing
        self.cfg.capabilities = capabilities
        if max_mode is not None:
            self.cfg.max_mode = max_mode
        # the controller steps by the new ladder's thresholds; a fresh one is what a new
        # session starts from anyway (every session seeds it from its connect frame)
        self.rate = self._rate_controller()
        self._recommended = self.cfg.initial_mode
        self._peer_floor = False
        self._peer_mode = None

    def _cap(self) -> int:
        """The fastest rung this station may send at: the operator's ceiling, and the
        rules' when they set one."""
        if self.cfg.ceiling is None:
            return self.cfg.max_mode
        return min(self.cfg.max_mode, self.cfg.ceiling)

    def _floor_only(self) -> bool:
        """Whether the rules admit only the tone floor: every frame this station sends —
        control frames and connect and probe answers included — goes out in that family."""
        return self.cfg.ceiling is not None and self.timing.is_floor(self.cfg.ceiling)

    def _send_burst(self, prefix: tuple[TxFrame, ...] = ()) -> None:
        """A burst, after ``prefix`` in the same transmission: the acknowledgement that took
        the turn (ADR-0047)."""
        mode = self._burst_mode()
        recommendation = min(self._recommended, self._cap())
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
            # so far under its threshold that combining cannot close the gap: sooner
            # (ADR-0038)
            due = (
                self.cfg.reencode_early_after if self._hopeless(rec.mode) else self.cfg.max_combines
            )
            if rec.tx_count >= due and recommendation < rec.mode:
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
        # every frame of a burst is one family, and so one length: as many as fit
        frame_s = self.timing.data_frame_s_for(self._records[seqs[0]].mode if seqs else mode)
        room = self._burst_capacity(frame_s)
        seqs = seqs[:room]
        new_frames = family == self.timing.is_floor(mode)
        cap = data_capacity(self.timing.capacity(mode))
        while new_frames and len(seqs) < room and self._outstanding() < WINDOW and self._tx_queue:
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
            fresh = [s for s in seqs if not self._records[s].sent]
            if fresh:
                self._ladder_pending = (mode, fresh)
        # A burst that empties the queue, with nothing else outstanding, ends with the turn on
        # offer (ADR-0047): the other station takes it in its acknowledgement if it has
        # something to send. Ordinary frames only: a floor control frame is 3.2 s.
        offer = (
            self.cfg.offer_turn
            and self.state is State.CONNECTED
            and not family
            and not self._tx_queue
            and self._pinned is None
            and not self._disc_requested
            and set(self._unacked()) <= set(seqs)
        )
        frames = list(prefix)
        for i, s in enumerate(seqs):
            rec = self._records[s]
            if rec.sent:
                self.stats.frames_resent += 1
            # each frame says how many of the burst come after it, so a receiver that loses
            # one in a fade still knows the burst is not over and does not answer over it
            # (ADR-0041) — the offer counted among them
            follows = countdown_of(len(seqs) - 1 - i + int(offer))
            frames.append(replace(self._data_frame(rec), follows=follows))
        if offer:
            frames.append(self._control(ControlKind.TURN, flags=ControlFlags.OFFER))
            self.stats.turn_offers += 1
        self._offer_out = offer
        self.stats.frames_sent += len(frames) - len(prefix) - int(offer)
        self.stats.bursts += 1
        self._burst_seqs = seqs
        self._bursts_since_turn += 1
        self._disarm("keepalive")
        self._transmit(frames)
        # the IRS's quiet after this burst: its family, and the longest frame it will expect
        # — this burst's, or the mode it recommended, as its own _peer_data_frame_s has it
        expected = max(
            self.timing.data_frame_s_for(self._records[seqs[0]].mode) if seqs else 0.0,
            self.timing.data_frame_s_for(min(self._recommended, self._cap())),
        )
        # The IRS answers in the family it last heard from us: this burst's, if it decodes
        # any of it, and the one its last answer came in (our _peer_floor) if it decodes
        # none — the acceptance of a call on the floor before a first OFDM burst, or the
        # floor bursts before a climb. Wait for whichever of the two is longer: waiting for
        # this burst's alone gave up on a floor acknowledgement a second into it, and the
        # recommendation it carried was lost with it (ADR-0016).
        families = {family, self._peer_floor}
        # A receiver that lost the burst's last frame takes the one before at its word and
        # answers that frame late (ADR-0046); one that lost the offer takes the last data frame's
        # count — the offer among it — at its word, up to two frames (ADR-0047). The wait covers
        # them.
        frame_s = self.timing.data_frame_s_for(self._records[seqs[-1]].mode)
        late = 0.0 if family else frame_s * (2 if offer else 1 if len(seqs) >= 2 else 0)
        self._wait_for(
            "ack",
            max(self.timing.control_frame_s_for(f) for f in families),
            max(self._irs_reply_delay(f, expected) for f in families) + late,
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
            if not rec.acked and rec.sent and ack.received(s):
                rec.acked = True
                self.stats.bytes_acked += len(rec.body)
                if self._disc_patience_until is not None:
                    # progress: a sender leaving waits its patience again from here
                    self._disc_patience_until = self.now + self._disc_patience()
        while self._tx_base != self._tx_next and self._records[self._tx_base].acked:
            del self._records[self._tx_base]
            self._tx_base = seq_after(self._tx_base)
        self._recommended = max(0, min(self._cap(), ack.recommended_mode))
        if ack.snr_db is not None:
            self.peer_snr_db = ack.snr_db
        self._peer_wants_tx = bool(ack.flags & ControlFlags.WANT_TX)
        self._peer_break = bool(ack.flags & ControlFlags.BREAK)
        self._waiting_for = None
        self._disarm("wait")
        self._offer_out = False
        if ack.flags & ControlFlags.TAKEN:
            # the other station took the turn this one offered: its burst follows (ADR-0047)
            self._take_irs()
            self._peer_wants_tx = self._peer_break = False
            self._bursts_since_turn = 0
            return
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
            # Or, after a burst that offered the turn, one that does not decode: the
            # acknowledgement that took the turn was lost (ADR-0047).
            payload, _ = frame.decode(None)
            if payload is None:
                if not self._turn_taken_unread(frame):
                    return
            else:
                try:
                    header, _ = decode_data(payload)
                except ValueError:
                    return
                if header.session != self.session:
                    return
                if header.kind is DataKind.CONNECT_ACK:
                    return  # repeated accept: our confirmation is on its way
            self._offer_out = False
            self.role = Role.IRS
            self._waiting_for = None
            self._unread_there = False
            self._disarm("wait")
            self._disarm("keepalive")
            self.actions.append(Event("role", "irs"))
        if self.role is Role.IRS and self._waiting_for == "turn":
            # the other station took the turn: it read a TURN
            self._waiting_for = None
            self._turn_tries = 0
            self._unread_there = False
            self._disarm("wait")
        if frame.floor != self.timing.is_floor(frame.mode):
            # a floor frame whose chips name an ordinary mode, or the reverse: the chips
            # are noise — a false detection, most likely — and so are its SNR and its soft
            # bits; it is not part of any burst
            self.stats.frames_failed += 1
            return
        # part of the current burst: record it, decode what decodes, ACK after the gap —
        # unless it was the caller's request again, which is answered by the acceptance
        # again and by nothing else (_accept)
        rec = _RxRecord(frame, self._slot_of(frame))
        self._burst.append(rec)
        if self._decode_record(rec):
            # a burst whose last frame said none follow it is over when that frame ends: the
            # answer waits only the turnaround, not the silence that would otherwise show
            # it (ADR-0045)
            reply = self.timing.turnaround_s if self._burst_closed() else self._irs_reply_delay()
            self._arm("ack", max(0.0, self._burst_end() - self.now) + reply)
        elif rec.outside:
            self._burst.remove(rec)
            if not self._burst:
                self._disarm("ack")

    @staticmethod
    def _announced_end(rec: _RxRecord) -> float | None:
        """The latest a frame of a burst says the burst ends: its own end, and the most frames
        its countdown says may follow it, each as long as itself (ADR-0041, ADR-0046). None from
        a frame neither decoded nor trusted: a phantom's chips are noise."""
        frame = rec.frame
        if frame.follows is None or not (rec.payload is not None or frame.trusted):
            return None
        return frame.t_end + most_following(frame.follows) * max(0.0, frame.t_end - frame.t_start)

    def _burst_closed(self) -> bool:
        """Whether the burst is known to be over at :meth:`_burst_end`: the frame that ends
        latest said, believably, that none follow it (a countdown of 0 is exact). Without it
        the end is inferred from silence."""
        end = self._burst_end()
        for rec in self._burst:
            frame = rec.frame
            believed = rec.payload is not None or frame.trusted
            if believed and frame.follows == 0 and frame.t_end >= end - 1e-9:
                return True
        return False

    def _burst_end(self) -> float:
        """Where the burst being received ends, as far as anything heard of it says: the
        latest end any of its frames announced. A later frame never brings it earlier — one
        whose count could not be believed said nothing of the frames after it, and taken alone
        it set the answer for its own end, over the two frames an earlier one had announced
        (the scenario harness, ADR-0042: eight collisions on an 80 m Poor path). In ND1J's
        session of 2026-10-06 the acknowledgement went out over the burst's last frame 21
        times, and both were lost (ADR-0040).

        Every countdown is a bound — the most frames that may follow (ADR-0046) — so the burst
        ends by the tightest any believed frame gives, and no earlier than the end of any frame
        heard."""
        heard = max(rec.frame.t_end for rec in self._burst)
        bounds = [b for b in (self._announced_end(rec) for rec in self._burst) if b is not None]
        return max(heard, min(bounds)) if bounds else heard

    def _decode_record(self, rec: _RxRecord) -> bool:
        """Decode a frame of the burst, alone or combined with an earlier transmission of its
        block. ``False`` when it was no frame of a burst: the caller's request again
        (:meth:`_accept`)."""
        payload, buffer = rec.frame.decode(None)
        if payload is not None:
            return self._accept(rec, payload)
        # inference: which sequence number is this slot? Then combine with any earlier
        # transmission of that block (HARQ-IR) and, either way, keep this transmission's
        # soft information for the next retransmission. A wrong guess only wastes a combine
        # — the CRC never lets mismatched LLRs through — and max_combines caps that waste.
        guess = self._infer_seq(rec.slot)
        rec.seq = guess
        rec.buffer = buffer
        if guess is None or not in_window(guess, self._rx_base):
            self.stats.frames_failed += 1
            return True
        if guess in self._harq and self._harq[guess][2] != rec.frame.mode:
            del self._harq[guess]  # re-encoded at another mode: start over
        if guess in self._harq:
            prev, combines, _ = self._harq[guess]
            rec.combined = True
            combined, merged = rec.frame.decode(prev)
            if combined is not None:
                self.stats.harq_rescues += 1
                return self._accept(rec, combined)
            self._harq[guess] = (
                (buffer, 0, rec.frame.mode)
                if combines + 1 >= self.cfg.max_combines
                else (merged, combines + 1, rec.frame.mode)
            )
        else:
            self._harq[guess] = (buffer, 0, rec.frame.mode)
        self.stats.frames_failed += 1
        return True

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

    def _accept(self, rec: _RxRecord, payload: bytes) -> bool:
        """Take a decoded frame. ``False`` when it was the caller's request again, which the
        acceptance answers again: no frame of a burst, and nothing to acknowledge."""
        rec.payload = payload
        try:
            header, body = decode_data(payload)
        except ValueError:
            rec.payload = None
            return True
        # a frame of nobody's session — a beacon, a probe or its answer, a datagram — is never
        # session data, whatever its session byte says: a datagram's holds its own number
        # (ADR-0019), which can equal this session's
        if header.kind in OUTSIDE_SESSIONS or header.session != self.session:
            # no frame of a burst (ADR-0038): counted, it was a failure that widened the
            # margin, and the acknowledgement its preamble armed answered nobody — ND1J's
            # beacon in the middle of a session drew one (2026-10-05)
            rec.payload = None
            rec.outside = True
            return False
        self._note_peer_frame(rec.frame)
        self._last_peer_frame = self.now
        self._heard_peer_db = rec.frame.snr_db
        self._arm("link", self._link_timeout())
        rec.seq = header.seq
        self.stats.frames_received += 1
        if header.kind is DataKind.CONNECT_REQ:
            # our CONNECT_ACK was lost: answer again, with the SNR this request arrived at, as
            # the first answer carried the first's — a caller that hears only the repeat
            # starts its first burst from it (P9-2). Without it the caller started on the
            # ladder's first rung whatever the path.
            self._send_connect(DataKind.CONNECT_ACK, rec.frame.snr_db)
            # and by nothing else: an acknowledgement a burst's quiet after the request went
            # out over the caller's first burst, which follows the acceptance at once
            self._burst.clear()
            self._burst_t0 = None
            self._disarm("ack")
            return False
        if header.kind is DataKind.CONNECT_ACK:
            return True
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
        return True

    def _finish_burst(self) -> None:
        """End of a burst: HARQ buffers were already stored per frame in
        :meth:`_decode_record`, so just clear the burst accumulator."""
        self._burst.clear()
        self._burst_t0 = None

    def _send_ack(self) -> None:
        if self.state is not State.CONNECTED and self.state is not State.DISCONNECTING:
            return
        decoded = [r for r in self._burst if r.payload is not None]
        ok = len(decoded)
        # A failure is news of the path when the frame could have decoded (ADR-0020): a real
        # frame (the PHY trusts it), at a redundancy version that decodes on its own or
        # combined with an earlier transmission of its block. A retransmission at RV 1 or 2
        # with nothing to combine with does not decode at any SNR — and that is how the
        # retransmission of a frame this station already has arrives, after an
        # acknowledgement the sender missed: each lost one had taught the margin 3 dB.
        failed = sum(
            1
            for r in self._burst
            if r.payload is None
            and r.frame.trusted
            and (r.frame.rv in SELF_DECODABLE_RVS or r.combined)
        )
        # The SNR of the frames that were really there (ADR-0020): what decoded, and what the
        # PHY acquired confidently enough to trust — a real frame that failed says how the path
        # was. A detection just over its threshold that did not decode is as likely noise, and
        # its SNR an estimate of that noise: on ND1J's 40 m path one read -11 dB between frames
        # decoding at +5 to +8, and that number took the recommendation from rung 4 to rung 1
        # (2026-09-25). A burst with none reports no SNR; its failure is what is learned from.
        snrs = [r.frame.snr_db for r in self._burst if r.payload is not None or r.frame.trusted]
        snr = sum(snrs) / len(snrs) if snrs else None
        modes = [r.frame.mode for r in self._burst]
        burst_mode = max(set(modes), key=modes.count) if modes else None
        self._finish_burst()
        if failed and modes and self._asked is not None and min(modes) > self._asked:
            # every frame faster than any rung this station has asked for: the sender's
            # choice, and its failure no news — the Test's ladder, pinned past what the path
            # carries, held the margin at its ceiling for the file that followed it (ADR-0020).
            # Anything slower is judged as usual: a retransmission keeps the rung it was first
            # sent at, so after a step down the sender still sends rungs this station asked
            # for once, and their failures are the path's.
            self.rate.observe_snr(snr)
        else:
            self.rate.observe(snr, ok, failed, burst_mode)
        if self._disc_requested:
            self._send_disc()
            return
        bitmap, missing, next_new = self._receive_window()
        self._ack_history = [_AckSnapshot(self._rx_base, missing, next_new), *self._ack_history][:2]
        flags = ControlFlags.NONE
        # frames sent and not yet acknowledged are work as much as the queue is: a station
        # that gave up the turn with some in flight — a BREAK, or yielding to a poll — asked
        # for nothing back, and they waited for the other station to run out of its own
        if self._has_work():
            flags |= ControlFlags.WANT_TX
        if self._break_requested:
            flags |= ControlFlags.BREAK | ControlFlags.WANT_TX
        # the turn offered is taken when there is something to send and nothing of the other
        # station's is missing: it has nothing left to send (ADR-0047)
        take = (
            self._offered
            and self.role is Role.IRS
            and bool(self._tx_queue)
            and not missing
            and self.state is State.CONNECTED
        )
        self._offered = False
        if take:
            flags |= ControlFlags.TAKEN | ControlFlags.WANT_TX
        self._stated_want = bool(flags & ControlFlags.WANT_TX)
        self._ack_counter = (self._ack_counter + 1) % 8
        self.stats.acks_sent += 1
        recommended = self.rate.recommend()
        self._asked = recommended if self._asked is None else max(self._asked, recommended)
        ack = self._control(
            ControlKind.ACK,
            flags=flags,
            base=self._rx_base,
            bitmap=bitmap,
            snr_db=snr,
            recommended_mode=recommended,
            counter=self._ack_counter,
        )
        if take:
            self.stats.turns_taken += 1
            self._become_iss()
            self._send_burst(prefix=(ack,))
            return
        self._transmit([ack])

    def _receive_window(self) -> tuple[int, list[int], int]:
        """What an acknowledgement says has arrived: the bitmap of the window from the base,
        the sequence numbers missing below the highest seen, and the first one never seen."""
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

    # ── control frames ────────────────────────────────────────────────

    def _on_control(self, frame: SoftFrame) -> None:
        payload, _ = frame.decode(None)
        ctl = None
        if payload is not None:
            with contextlib.suppress(ValueError):
                ctl = ControlFrame.decode(payload)
        if ctl is None:
            if self._waiting_for == "turn":
                self._turn_unread = True  # it may be the answer to the TURN (ADR-0029)
            return
        self._note_peer_frame(frame)
        if (
            self.state is State.IDLE
            or self.state is State.CONNECTING
            or ctl.session != self.session
        ):
            return
        self._last_peer_frame = self.now
        self._heard_peer_db = frame.snr_db
        if ctl.kind is not ControlKind.ACK and ctl.snr_db is not None:
            # how the other station hears this one, from a frame other than an
            # acknowledgement (ADR-0021): a disconnect carries it to a station that only
            # received
            self.peer_snr_db = ctl.snr_db
        self._arm("link", self._link_timeout())
        if ctl.kind is ControlKind.DISC:
            self._transmit([self._control(ControlKind.DISC_ACK)])
            self._end_session("peer disconnected")
        elif ctl.kind is ControlKind.DISC_ACK:
            if self.state is State.DISCONNECTING:
                self._end_session("closed")
        elif ctl.kind is ControlKind.ACK:
            unread_poll = False
            if self._waiting_for == "poll":
                # The other station answers in the family it last read this one in, and answers
                # a poll it heard and could not read all the same (ADR-0028): a floor answer to
                # an ordinary poll answered the poll's preamble alone. Taken for the answer, it
                # told this station the link was up while the other read nothing of it until its
                # link timed out (ADR-0034).
                unread_poll = frame.floor and not self._poll_floor
                self._unread_there = unread_poll
            if self.role is Role.ISS and (
                self._waiting_for in ("ack", "poll")
                or (
                    # an idle sender in a chat takes the other station's request for the turn
                    # (ADR-0027) — and, with nothing of its own to finish, hands over at once
                    self._waiting_for is None
                    and self.cfg.chat
                    and bool(ctl.flags & ControlFlags.WANT_TX)
                )
            ):
                self._on_ack(ctl)
                if unread_poll and "keepalive" in self._deadlines:
                    # the answer asked for nothing — no turn, no burst went — and the poll was
                    # not read: it goes again now, on the floor, rather than a keepalive later
                    self._arm("keepalive", self.timing.turnaround_s)
            elif self.role is Role.IRS and self.state is State.CONNECTED:
                # The other station is receiving too: it has not taken a turn this one offered,
                # and it answered a preamble of this one's as the end of a burst. That needs no
                # answer, and the acknowledgement the frame's own preamble armed is withdrawn:
                # sent, it was answered in turn, and keyed after a TURN it went out over the
                # answer to the TURN (ADR-0029).
                if self._waiting_for == "turn":
                    self._turn_unread = False
                    # A station that reads a TURN takes the turn and answers with its burst or a
                    # poll: this acknowledgement answered the TURN's preamble. Repeated in its
                    # family when the wait ran out, an ordinary TURN answered on the floor went
                    # unread three times and the other station timed out (ADR-0034). It goes
                    # again now, on the floor.
                    self._unread_there = True
                    self._arm("wait", self.timing.turnaround_s)
                if not self._burst:
                    self._disarm("ack")
        elif ctl.kind is ControlKind.POLL:
            if self.role is Role.IRS or self._waiting_for == "turn":
                self._take_irs()
                self._arm("ack", self.timing.turnaround_s + max(0.0, frame.t_end - self.now))
            elif self.role is Role.ISS and not self._caller:
                # Both stations hold the turn: the other missed this one's answer to its TURN,
                # took the turn back when its tries ran out, and polls — and a sender that
                # ignores a poll leaves both sending into a path the other cannot hear until
                # the session dies (KE4QCM, 2026-09-25: "no response"). The called station
                # yields: it answers the poll and asks for the turn back (WANT_TX), and the
                # caller keeps the turn, so the two can never both yield (ADR-0023).
                self._take_irs()
                self._arm("ack", self.timing.turnaround_s + max(0.0, frame.t_end - self.now))
        elif ctl.kind is ControlKind.TURN and ctl.flags & ControlFlags.OFFER:
            # the end of a burst that emptied the sender's queue (ADR-0047): the burst is over
            # with this frame, and the acknowledgement may take the turn
            if self.role is Role.IRS:
                self._offered = True
                self._arm("ack", self.timing.turnaround_s + max(0.0, frame.t_end - self.now))
        elif ctl.kind is ControlKind.TURN:
            if self.role is Role.IRS or self._waiting_for == "turn":
                self._take_iss()
            elif self.state is State.CONNECTED and not self._tx_busy():
                # The turn is this station's already, and the other station offers it again: it
                # has heard nothing of this one taking it — the burst or the poll that answered
                # its TURN was lost — and it would take the turn back when its tries ran out,
                # making two senders. The answer goes again (ADR-0029). A station whose own
                # transmission is still going out has its answer on the way.
                self._retries = 0
                self._waiting_for = None
                self._disarm("wait")
                self._disarm("keepalive")
                self._answer_turn()

    def _turn_taken_unread(self, frame: SoftFrame) -> bool:
        """Whether a data frame that did not decode says the turn this station offered was
        taken (ADR-0047): the acknowledgement that took it was lost, and the other station's
        burst is arriving. Only data can follow an offer that way — the other station answers
        an offer it does not take with an acknowledgement alone — so a frame the physical
        layer trusts, in the family of the session's ordinary frames, is that burst. Waiting
        for an acknowledgement that will not come, the sender sent its burst again over the
        other station's (the scenario harness, 80 m at 500 Hz)."""
        return (
            self._offer_out
            and self._waiting_for == "ack"
            and frame.trusted
            and not frame.floor
            and frame.floor == self.timing.is_floor(frame.mode)
        )

    def _take_irs(self) -> None:
        self._offer_out = False
        if self.role is not Role.IRS:
            self.actions.append(Event("role", "irs"))
        self.role = Role.IRS
        self._waiting_for = None
        self._turn_tries = 0
        self._unread_there = False
        self._disarm("wait")
        self._disarm("keepalive")

    def _take_iss(self) -> None:
        self._become_iss()
        self._answer_turn()

    def _become_iss(self) -> None:
        self.role = Role.ISS
        self._unread_there = False
        # the first burst of a turn goes out where this station's own measurements of the
        # peer put it — HF is reciprocal — as a caller's goes out where the acceptance puts
        # it (P9-2), not at the slowest rung of the ladder: that is the tone floor
        # (ADR-0013), five times slower than the first OFDM mode
        if self.rate.snr_db is not None:
            first = self.rate.first_mode(self.rate.snr_db)
            self._recommended = max(self.cfg.initial_mode, min(self._cap(), first))
        self._waiting_for = None
        self._disarm("wait")
        self._disarm("ack")
        self._disarm("request")
        self._burst.clear()
        self._burst_t0 = None
        self._break_requested = False
        self._stated_want = False
        self._bursts_since_turn = 0
        self._retries = 0
        self._offered = False
        self.actions.append(Event("role", "iss"))

    def _answer_turn(self) -> None:
        """What tells the station that sent a TURN that this one has taken the turn: the first
        burst of the turn, or a poll when there is nothing to send."""
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
        if self.state is not State.IDLE:
            return  # a session's frames matter more than a question from outside it
        # a probe in another bandwidth is answered too (ADR-0035): it and its answer are
        # tone-floor frames, the same 400 Hz frames on both airs, and "you hear me, but we
        # run different bandwidths" is exactly what a prober needs to hear — ignoring it
        # left two stations each hearing the other and neither knowing why nothing came back
        across = bandwidth_code(req.caps) != bandwidth_code(self.cfg.capabilities)
        # answer as the callsign that was probed, with the SNR the probe arrived at —
        # the one number the prober cannot measure for itself
        self.my_call = req.dst
        self.stats.probes_answered += 1
        self.actions.append(
            Event("probed", f"{req.src} at {frame.snr_db:.1f} dB{self._mismatch(req.caps)}")
        )
        # back in the family the probe came in (noted from it), as an acceptance goes back on
        # the layout its request arrived on — and across bandwidths always on the floor, the
        # one family both airs share
        self._send_probe(
            DataKind.PROBE_ACK,
            req.src,
            frame.snr_db,
            floor=across or self._peer_floor or self._floor_only(),
        )

    def _mismatch(self, caps: int) -> str:
        """`` — runs 2300 Hz, this station 500 Hz`` when a frame's capability byte states
        another bandwidth than this station's, empty otherwise: the words a probe's event
        adds so an operator sees why a call between the two would be ignored (ADR-0035)."""
        if bandwidth_code(caps) == bandwidth_code(self.cfg.capabilities):
            return ""
        theirs = bandwidth_hz_of(caps)
        ours = bandwidth_hz_of(self.cfg.capabilities)
        there = "an unknown bandwidth" if theirs is None else f"{theirs} Hz"
        here = "an unknown bandwidth" if ours is None else f"{ours} Hz"
        return f" — runs {there}, this station {here}"

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
        self.last_probe = ProbeResult(ack.src, ack.snr_db, frame.snr_db, bandwidth_hz_of(ack.caps))
        theirs = "?" if ack.snr_db is None else f"{ack.snr_db:.0f}"
        self.actions.append(
            Event(
                "probe",
                f"{ack.src} hears us at {theirs} dB, heard at {frame.snr_db:.1f} dB"
                f"{self._mismatch(ack.caps)}",
            )
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
            self.actions.append(
                Event(
                    "ignored",
                    f"{req.src} calls in another bandwidth{self._mismatch(req.caps)}",
                )
            )
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
        self._heard_peer_db = snr_db
        self._arm("link", self._link_timeout())
        # the request is the first measurement of how the caller is heard: the
        # controller starts from it, and the acceptance carries it back so the caller's
        # first burst can too (P9-2) — a lower bound if it came on the tone floor (ADR-0016)
        self.rate.seed(snr_db, lower_bound=self._peer_floor)
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
        self._caller = True
        self._confirmed = True
        self._last_peer_frame = self.now
        self._heard_peer_db = snr_db
        self._arm("link", self._link_timeout())
        self.actions.append(Event("connected", f"{self.remote_call} (iss)"))
        # the acceptance says how the request was heard: the first burst starts at what
        # that supports, less a step, instead of at the slowest mode; the acceptance's
        # own SNR is how the other station is heard here, which this station's
        # controller starts from for the day it receives (P9-2)
        self.rate.seed(snr_db, lower_bound=self._peer_floor)
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
        self._heard_peer_db = None
        self._turn_tries = self._disc_tries = 0
        self._turn_unread = False
        self._unread_there = self._poll_floor = False
        self._disc_requested = False
        self._disc_patience_until = None
        self._waiting_for = None
        self._rx_base = 0
        self._rx_buf.clear()
        self._max_seen = None
        self._harq.clear()
        self._burst.clear()
        self._burst_t0 = None
        self._ack_history = []
        self._ack_counter = 0
        self._asked = None
        self._break_requested = False
        self._stated_want = False
        self._confirmed = False
        self._caller = False
        self.peer_capabilities = 0
        self.rate = self._rate_controller()
        for name in ("ack", "wait", "keepalive", "link", "request"):
            self._disarm(name)

    def _end_session(self, reason: str) -> None:
        self.ended_peer_snr_db = self.peer_snr_db
        self.state = State.IDLE
        self.role = Role.NONE
        self._reset_transfer_state()
        self._disarm("connect")
        self.actions.append(Event("disconnected", reason))
