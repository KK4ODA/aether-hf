"""Granted slots: the link engine of protocols 8 and 9 (ADR-0069; 9 adds measurement frames,
ADR-0077).

One :class:`GrantedEngine` per station, beside :class:`~aether_model.link.engine.LinkEngine`
(protocol 7). Like it, it is a pure state machine — :meth:`~GrantedEngine.on_frame`,
:meth:`~GrantedEngine.on_tx_done` and :meth:`~GrantedEngine.tick` in, actions out — and it
keeps its data path: selective repeat over a 16-frame window, HARQ with slot inference,
re-encoding, the rate controller and the connect handshake's measurements. What it changes is
*when* a station may key, and nothing else.

The protocol in one paragraph
-----------------------------
The caller is the **master** for the whole session and the only station that keys on its own
initiative; the called station is the **slave**. Every master transmission is a header control
frame and up to six data frames, each data frame stating exactly how many of the transmission
follow it. The header acknowledges the slave's data, recommends the slave's rung, states its own
count and *grants* the slave one slot, which starts a fixed gap after the transmission's end and
lasts at most the grant: an answer, an answer and ``n`` data frames, or, when neither side has
anything, an open window. The slave keys only in a slot granted by a frame it decoded in time to
start the slot on time — the daemon's keying gate drops a transmission that would start late,
whatever held it (``SlotTransmit.not_after``) — and its answer acknowledges the master's data,
says what follows it and what it has queued. The master keys again only once the slot can no
longer be in use: a gap after the end a decoded slave frame of the slot stated, or after the
slot's latest end when it decoded nothing. Only decoded content changes state: a detection, an
announcement or a silence moves no deadline. There is no turn, no TURN, OFFER, TAKEN, POLL or
turn request, and nothing to hand over. ``docs/research/link-architecture/07-protocol-8.md`` is
the design, ``04-formal-spec.md`` the rules and their proof, ADR-0069 the decision.

A station measures a rung (:meth:`GrantedEngine.measure`, the Test's ladder) with frames of their
own kind: filler, alone in a transmission whose control frame says so, sent once and never
delivered; they take no sequence number, the other station's acknowledgement reports which
arrived, and the station's data waits for that report (protocol 9, ADR-0077).

The slot, as both stations reckon it
------------------------------------
After a transmission ending on the air at ``E`` whose last frame is of family ``f`` (ordinary or
the tone floor), at gap level ``L``::

    start   = E + max(D_f + KEY, T_granter) + GUARD + LEVEL[L]
    end_max = start + SLACK + grant

``D_f`` is the decode budget (a frame's end to its report), ``KEY`` the keying latency a station
is allowed, ``T_granter`` the turnaround the granting station declared at the connection (its
radio's VOX hold and receiver recovery), ``GUARD`` propagation and clock error. These are
protocol constants (:data:`DECODE_BUDGET_S` …), the same at both ends: a station slower than they
allow asks for a gap level in its answers.
"""

from __future__ import annotations

import contextlib
import math
import random
from collections.abc import Sequence
from dataclasses import dataclass, field
from enum import Enum
from typing import TypedDict

from aether_model.link.engine import (
    LEFT_UNHEARD,
    OUTSIDE_SESSIONS,
    RV_SEQUENCE,
    SELF_DECODABLE_RVS,
    Action,
    Deliver,
    Event,
    LadderRung,
    ProbeResult,
    State,
    Transmit,
)
from aether_model.link.frames import (
    CAP_LEVEL_MASK,
    CONNECT_BODY_8_BYTES,
    MAX_BURST,
    MAX_FOLLOWS,
    PROTOCOL_VERSION,
    REPORT,
    WINDOW,
    ConnectBody,
    DataHeader,
    DataKind,
    GrantFrame,
    GrantKind,
    ProbeBody,
    bandwidth_code,
    bandwidth_hz_of,
    data_capacity,
    decode_data,
    encode_data,
    in_window,
    level_of,
    pack_callsign,
    seq_after,
    seq_distance,
    with_level,
)
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import CONTROL_THRESHOLD_DB, RateController, usable_modes

# ── the protocol's constants ──────────────────────────────────────────

DECODE_BUDGET_S = (0.30, 0.30)
"""From a frame's end on the air to its decode reaching the engine, ordinary and tone floor.
Measured on the harness (08-steps-0-and-1.md §1.3): 11 895 frames of both families on both airs
were reported at most 0.157 s after their end on the receiver's audio clock, the 99th percentile
0.156 s; the receiver runs at 0.07× real time, its slowest pass 35 ms. The floor's provisional
0.65 s was the OFDM floor family's settling (ADR-0009 §8), and the tone floor replaced that
family. Hardware in the loop checks it again before it freezes (Step 4)."""
KEY_S = 0.35
"""From the engine's decision to radiation: the daemon's lead (0.1 s) and the sound card
(0.25 s, the harness's figure and the upper end of a real card's). It leaves nothing for
rendering the transmission, which the harness cannot see (it runs in lockstep with the audio)
and which takes up to 0.18 s for four tone frames: the daemon renders a slot's data frames
before the header that grants it is decoded (07-protocol-8.md §10). A station slower than this
asks for a level."""
GUARD_S = 0.05
"""Propagation (under 20 ms one way on HF), clock error over a transmission, rounding."""
SLACK_S = 0.35
"""How late a slot may start on the air and still be used; the keying gate drops past it."""
LEVELS_S = (0.0, 0.5, 1.0, 2.0)
"""Extra gap by level: what a slave whose computer decodes or keys late asks for, and what a
master that hears nothing twice in a row grants."""
GRID_TOLERANCE_S = 0.15
"""How far from its slot a frame that did not decode may begin and still count as one of the
transmission's: the receiver places a frame to a few milliseconds, propagation adds 20."""
MAX_DATA = 6
"""Data frames in one transmission (three bits count them in the control frame)."""
HEADER_UNDECODABLE_SHARE = 0.4
"""How far into a transmission's first frame, a control frame, a cut still leaves nothing of
the transmission readable (10-step-3.md §3.6). A tone control frame decodes only from two of its
three sync blocks (ADR-0049), and the middle one begins past 45 % of the frame; an ordinary
control frame is coded at rate ½ at the most robust, and 60 % of its symbols lost leaves nothing
to decode. The daemon reports such a cut with :meth:`GrantedEngine.on_tx_cut_unread`."""
READABLE_ALONE_RVS = frozenset({0})
"""The redundancy versions a copy decodes alone in on every rung: what R7 needs before it may
leave a header out, since a count is believed only from a copy that decoded alone (R2). RV 1
and 2 never decode alone, and RV 3 only on the rungs whose RV 3 carries every systematic bit
(ADR-0043): on ND1J's Tests of 2026-10-15 every headerless transmission of RV 2 and 3 copies
went silent, eight of the 21 silent slots (ADR-0073)."""
OPEN_WINDOW_S = (4.0, 10.0)
"""An idle link's open window, after an ordinary and after a floor header."""


_MASK64 = (1 << 64) - 1


class Draws:
    """The engine's random draws (a session number, a call's jitter): SplitMix64, written out so
    that the port draws exactly the same numbers from the same seed, and an engine's every
    action can be checked against the model's (``tools/make_granted_vectors.py``)."""

    def __init__(self, seed: int) -> None:
        self.state = seed & _MASK64

    def next_u64(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & _MASK64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & _MASK64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & _MASK64
        return z ^ (z >> 31)

    def unit(self) -> float:
        """Uniform in [0, 1), from the top 53 bits."""
        return (self.next_u64() >> 11) * (1.0 / (1 << 53))

    def uniform(self, low: float, high: float) -> float:
        return low + (high - low) * self.unit()

    def below(self, n: int) -> int:
        """Uniform in [0, n) (the modulo's bias is under 2**-56 for the engine's n)."""
        return self.next_u64() % n


def measure_body(session: int, index: int, length: int) -> bytes:
    """A measurement frame's filler: SplitMix64 from the session and the frame's index, so a
    frame of every rung carries bytes that look like data — a body of zeros is not what data
    sends, and on the four-tone kinds it is a lopsided spectrum (12-two-paths-80m.md §9)."""
    draws = Draws(0x4D45_4153_5552_4500 | (session & 0xFF) << 8 | index & 0xFF)
    out = bytearray()
    while len(out) < length:
        out += draws.next_u64().to_bytes(8, "big")
    return bytes(out[:length])


def _carries_measurement(frames: Sequence[TxFrame]) -> bool:
    return any(
        f.container is Container.DATA and f.payload[0] >> 5 == DataKind.MEASURE.value
        for f in frames
    )


class Role(Enum):
    NONE = "none"
    MASTER = "master"
    SLAVE = "slave"


@dataclass
class SlotTransmit(Transmit):
    """A transmission, with what the daemon's keying gate needs to know about it."""

    not_after: float | None = None
    """The latest its radiation may start, on the air's clock, or ``None`` for one the station
    makes on its own initiative. Past it the transmission is not sent at all (the forfeit rule,
    ADR-0069 §2.3), whatever held it, and the engine is told
    (:meth:`GrantedEngine.on_tx_dropped`)."""
    not_before: float | None = None
    """The earliest its radiation may start, on the air's clock: the start of the slot it is
    the answer in. An answer is decided as soon as what it says is final — typically a decode's
    worth after the transmission it answers, where the slot starts the whole decode budget and
    keying latency after — so the daemon renders it in the meantime and keys it to start
    radiating here (07-protocol-8.md §10). ``None``: as soon as it can."""
    identifier_s: float = 0.0
    """The station's closing Morse identifier, this long, and the moment of silence that parts
    it from the frames: after the frames — behind a DISC_ACK, in the time every DISC grants it —
    or alone, with no frames; the last thing the station sends for the session (ADR-0075).
    Already counted in :attr:`duration_s`."""
    listen_first: bool = False
    """A transmission made outside every slot, which no protocol rule keeps clear: the closing
    identifier of a station that could not be given a slot for it — a slave whose session timed
    out, or whose left session never closed. The daemon waits for a clear channel, as it does
    for a call (ADR-0075)."""
    cause: str = ""
    """What set the transmission going (a timer, a frame, a command): for the harness's account
    of the air, never for the protocol."""

    @property
    def identifier(self) -> bool:
        return self.identifier_s > 0.0


# ── configuration and counters ────────────────────────────────────────


@dataclass
class GrantConfig:
    max_data_frames: int = MAX_DATA
    """Data frames in one transmission, at most :data:`MAX_DATA`."""
    max_burst_s: float | None = None
    """The longest a transmission may be on the air, seconds: the key-time limit less the
    keying's lead and tail (ADR-0017). Unset, the frame count alone decides."""
    max_retries: int = 8
    """Silent slots in a row before a master gives the session up ("no response")."""
    connect_retries: int = 8
    disc_retries: int = 3
    """DISCs a leaving master sends, unanswered, before it leaves without the DISC_ACK."""
    disc_patience_exchanges: float = 2.0
    """A leaving master gives up on what it has not got acknowledged after this many whole
    exchanges with nothing new acknowledged, or :attr:`disc_patience_s` (ADR-0039)."""
    disc_patience_s: float = 20.0
    disc_drain_s: float = 60.0
    """The longest a disconnect waits for what is still queued, counted from when it was asked:
    past it the master sends its DISC at its next opportunity, delivered or not, and says what
    it left (ADR-0072). A slow path that still delivers never runs out of the patience above —
    the soak of 2026-10-15 drew three that would have taken 30–40 minutes."""
    leave_repeats: int = 3
    """DISCs an aborting master says, one a slot, while nothing answers (ADR-0052)."""
    floor_answer_margin_db: float = 12.0
    """A call heard in the ordinary family less than this above the ordinary control frame's
    threshold is accepted on the tone floor (ADR-0056)."""
    answer_floor_margin_db: float = 3.0
    """A station asks for the other's control frames on the floor while it hears the other
    less than this above the ordinary control frame's threshold (07-protocol-8.md §7)."""
    family_readings: int = 3
    """How many of the other station's frames, read in a row, must all clear that before a
    station asks for ordinary control frames. One was the rule: on a fading path an ordinary
    frame is decoded only at a peak, so the frame that said "ordinary is fine" was the one most
    likely to — a slave 6 dB under its master asked for ordinary headers at every peak, missed
    the next two in the fade, and its data waited three exchanges each time (the review's
    campaign, field-asym, -6 dB, until the 1 500 s horizon). On that campaign one and two
    readings each leave a cell completing less than protocol 7 (field-asym exchanges, 329 and
    332 of 336 against 333); three and four complete as much or more everywhere, and their
    times are within each other's intervals. Three is the fewest that holds completion."""
    link_timeout_s: float = 45.0
    link_timeout_exchanges: float = 4.0
    """Silence ends a session after this many exchanges of the size in use, 45 s at least."""
    rate: dict[str, float | int | None] = field(default_factory=dict)
    initial_mode: int = 0
    max_mode: int = 19
    ceiling: int | None = None
    """The fastest rung the rules allow (ADR-0018); see ``LinkConfig.ceiling``."""
    silence_step: int = 2
    """Rungs a master steps its own data down by for every data slot heard silent (P9-7)."""
    max_combines: int = 4
    reencode_early_after: int = 2
    reencode_hopeless_db: float = 1.0
    capabilities: int = 0
    """Offered in the handshake: compression (bit 0) and the bandwidth (bits 1–2)."""
    first_grant: int = 2
    """Data frames granted to the slave before it has said what it has: a gateway greets."""
    window_doublings: int = 1
    """How far an idle link's open window grows, by doubling, while it stays idle."""
    elide: bool = True
    """Leave the header out of a transmission it would change nothing in (R7)."""
    header_above_recommendation: bool = True
    """Keep the header on a transmission whose data goes above the rung the slave recommends —
    copies still at the rung they first went at, data pinned (:meth:`GrantedEngine.pin_mode`) —
    when the header is ordinary (ADR-0076). Left out,
    such a transmission granted nothing when none of its frames decoded: the two silent slots
    of WC4Y's Test of 2026-10-17, each a wait and a floor poll, where a 0.43 s header would have
    been read (13-wc4y-beta111.md §6). A floor header costs 3.2 s and broke even on ND1J's weak
    paths (12-two-paths-80m.md §12.1), so it is kept only where it is cheap."""
    turnaround_s: float = 0.0
    """This station's radio: how long after it stops sending it hears again (VOX hold and
    receiver recovery). Declared at the connection, 0.1 s steps up to 1.5 s."""
    identifier_s: float = 0.0
    """This station's closing Morse identifier's length, 0 for none: declared at the connection,
    so the master grants the slave's in the DISC_ACK's slot and the slave knows how long the
    master's lasts (ADR-0075). The length for every callsign not in :attr:`identifiers`."""
    identifiers: dict[str, float] = field(default_factory=dict)
    """The identifier's length by the callsign a session runs under: the daemon's, since the
    identifier sends that callsign and whether one is sent at all can follow it (a US callsign
    always identifies). Set by :meth:`GrantedEngine.set_identifiers`."""
    answer_gap_s: float = 0.0
    """The least time between the end of the other station's frames and this station radiating:
    the operator's ``[radio] answer_gap_ms``, or what this station learned the other needs
    (ADR-0036, ADR-0054). For a peer whose radio is slower to hear again than it declared — a
    VOX interface's hold left at 0. As the master the station keys no sooner than the gap after
    the slave's frames; as the slave it asks for the gap level that starts its slots that late
    (07-protocol-8.md §10). Set by :meth:`GrantedEngine.set_answer_gap`."""
    decode_budget_s: tuple[float, float] = DECODE_BUDGET_S
    key_s: float = KEY_S
    guard_s: float = GUARD_S
    slack_s: float = SLACK_S
    levels_s: tuple[float, ...] = LEVELS_S
    """The protocol's constants (above). Both stations must hold the same values; a bench may
    change them for both."""
    open_window_s: tuple[float, float] = OPEN_WINDOW_S
    keepalive_s: float = 10.0
    """On an idle link the master grants an answer, not an open window, once it has heard
    nothing from the slave for this long: an open window nobody needs is silent, and a link
    that is only ever silent cannot tell idle from dead."""
    silence_levels_after: int = 2
    """Silent slots in a row after which a master raises the gap level one step."""
    hold_unknown: bool = True
    """Frames whose answer went unheard are held back until an answer says what became of
    them, not resent blind (10-step-3.md §3.4). The next transmission carries new data, or no
    data at all — a header granting an answer, a poll. A second silent slot in a row sends them
    again: answers not getting through must not stop the data too. On the PACTOR path the slave
    decoded more frames than protocol 7's and still finished later: copies of frames it had."""
    floor_answer_memory: int = 4
    """Answer slots looked back over: while any of the last this many went silent, a header asks
    for the slave's answers on the floor (10-step-3.md §3.4). The readings that otherwise decide
    the family come only from frames that decoded, and miss what kills a short ordinary answer
    and spares a floor one — a PACTOR station's bursts on a lopsided path. 0: only after two
    silent slots in a row. 4 by the harness (10-step-3.md §5.4)."""
    on_time_to_lower: int = 4
    """Slots answered in a row after which a master lowers the silence level one step."""
    need_ease_after: int = 6
    """Slots answered on time after which a slave asks for one level less."""


@dataclass
class GrantStats:
    frames_sent: int = 0
    frames_resent: int = 0
    frames_received: int = 0
    frames_failed: int = 0
    harq_rescues: int = 0
    bytes_delivered: int = 0
    bytes_acked: int = 0
    probes_sent: int = 0
    probes_answered: int = 0
    probe_replies: int = 0
    frames_reencoded: int = 0
    left_answered: int = 0
    transmissions: int = 0
    headers_elided: int = 0
    silent_slots: int = 0
    """Slots this master granted in which it decoded nothing."""
    forfeits: int = 0
    """Slots this station was granted and did not use: the grant decoded too late."""
    dropped: int = 0
    """Transmissions the keying gate refused to start late (:meth:`GrantedEngine.on_tx_dropped`)."""


# ── internal records ──────────────────────────────────────────────────


@dataclass
class _TxRecord:
    seq: int
    body: bytes
    mode: int
    tx_count: int = 0
    acked: bool = False
    reencoded: int = 0
    unknown: bool = False
    """Sent in a transmission whose answer went unheard: whether it arrived is not known."""

    @property
    def sent(self) -> bool:
        return self.tx_count > 0 or self.reencoded > 0


@dataclass
class _RxRecord:
    frame: SoftFrame
    slot: int
    payload: bytes | None = None
    seq: int | None = None
    combined: bool = False


class _AckFields(TypedDict):
    base: int
    bitmap: int


@dataclass
class _AckSnapshot:
    base: int
    missing: list[int]
    next_new: int


@dataclass
class _Slot:
    """A slot this station granted: the other station's time on the air, and this station's
    wait for it."""

    kind: str
    """``header``, ``disc``, ``connect`` (a call's acceptance) or ``probe``."""
    grant_s: float
    """The longest the answering transmission may be, its identifier included."""
    last_floor: bool
    """The family of the granting transmission's last frame: the gap's decode budget."""
    answer_floors: frozenset[bool]
    """Every family the answer, and what follows it, may be in: the wait covers the slowest."""
    level: int
    granter_turnaround_s: float
    open_window: bool = False
    data: bool = False
    """The granting transmission carried data: a silent slot steps its rung down. Measurement
    frames are none: a measurement always carries its header, and the slot's silence says the
    header or the answer was lost, not that the rung measured was too fast (ADR-0077)."""
    granted: int = 0
    tx_end: float = 0.0
    start: float = 0.0
    end_max: float = 0.0
    final: bool = False
    """:attr:`tx_end` is where the granting transmission really ended (``on_tx_done``), not
    this engine's estimate."""
    heard: bool = False
    """A frame of the other station's, begun in the slot, decoded."""
    stated_end: float | None = None
    stated_floor: bool = False


@dataclass
class _Answer:
    """A transmission this station owes in a slot it was granted (or, idle, a call's or a
    probe's or a left session's slot)."""

    what: str
    """``ans``, ``window``, ``connect_ack``, ``probe_ack``, ``disc`` or ``disc_ack``."""
    start: float
    not_after: float
    floor: bool
    """The family of its control frame."""
    session: int = 0
    anchor: float = 0.0
    """The end of the transmission it answers."""
    remote: str = ""
    snr_db: float | None = None
    room_s: float = 0.0
    """Data time granted after the answer."""


# ── the engine ────────────────────────────────────────────────────────


class GrantedEngine:
    """The link engine of granted slots (ADR-0069), protocol 9 (ADR-0077)."""

    counts_on_floor = True
    """Tone frames carry their count too (ADR-0068 §2.1, kept): a simulated PHY delivers it."""

    def __init__(
        self,
        my_call: str,
        timing: PhyTiming,
        config: GrantConfig | None = None,
        seed: int | None = None,
    ) -> None:
        self.callsigns = [my_call.upper()]
        """The callsigns this station answers to; the first is the one it calls as."""
        self.my_call = self.callsigns[0]
        self.timing = timing
        self.cfg = config or GrantConfig()
        self.rng = Draws(seed if seed is not None else random.getrandbits(64))
        self.state = State.IDLE
        self.role = Role.NONE
        self.remote_call = ""
        self.session = 0
        self.now = 0.0
        self.actions: list[Action] = []
        self.stats = GrantStats()
        self._deadlines: dict[str, float] = {}
        self._cause = ""
        self._slot: _Slot | None = None
        """The slot this station granted last — a call's, a probe's, a header's, a DISC's."""
        self._in_flight: list[tuple[_Slot | None, float, bool]] = []
        """This station's transmissions handed over and not yet done, in order, each with the
        slot it granted — ``on_tx_done`` reports where that slot's granting transmission ended —,
        the closing identifier it carries behind a DISC_ACK, owed again if it is dropped, and
        whether it carries a measurement, asked again if it is dropped."""
        self._hold_until = 0.0
        """Nothing this station starts on its own — a call, a probe, a leaving DISC — or answers
        while idle keys before this: the last slot it granted may still be in use."""
        self._left: tuple[int, float, bool] | None = None
        """A session this station has left: its number, until when its frames are answered, and
        whether the other station's DISC ended it (then only a DISC is answered, with its
        DISC_ACK; otherwise a header is answered with a DISC — ADR-0052)."""
        self._leave_repeats = 0
        self._call_jitter = 0.0
        self._my_ident_s = 0.0
        """This station's closing identifier's length for the session under way or just ended:
        what its call or acceptance declared, so what the other station reckons with."""
        self._id_owed = 0.0
        """The closing identifier of a session that has ended, not yet handed over: its length,
        or 0 (ADR-0075)."""
        self._id_alone = False
        """Whether the owed identifier goes on its own initiative — no slot could be had for it
        — rather than in time the closing exchange left this station."""
        self._id_listen = False
        """Whether the owed identifier listens first: one on its own, and a master's after a
        session in which it read the other station no more — that station may be sending its
        own (ADR-0075)."""
        self._probing: str | None = None
        self._probe_unsent = False
        self.last_probe: ProbeResult | None = None
        self.ended_peer_snr_db: float | None = None
        self.ladder: list[LadderRung] = []
        """What the other station reported of each measurement (:meth:`measure`): the Test's
        ladder."""
        self._pinned: int | None = None
        self._pin_body: int | None = None
        self._connect_tries = 0
        self._tx_queue = bytearray()
        """What :meth:`send` was given and no frame carries yet. Kept across the start of a
        session — what an idle station is handed goes in the next one — and cleared at its end:
        left queued, it opened the next session, to anybody (ADR-0052)."""
        self._reset_session()

    # ── per-session state ─────────────────────────────────────────────

    def _reset_session(self) -> None:
        self.top_acked_mode: int | None = None
        self._records: dict[int, _TxRecord] = {}
        self._tx_base = self._tx_next = 0
        self._recommended = self.cfg.initial_mode
        """The rung this station's data goes at: what the other station recommends."""
        self.peer_snr_db: float | None = None
        """How the other station says it hears this one (every control frame carries it)."""
        self._heard_db: float | None = None
        """How this station heard the other's last frame it decoded."""
        self._heard_recent: list[float] = []
        """The readings of the last :attr:`GrantConfig.family_readings` frames of the other's
        decoded: the family this station asks for its control frames in goes by the weakest."""
        self.peer_capabilities = 0
        self._peer_turnaround_s = 0.0
        self._peer_identifier_s = 0.0
        self._peer_left = False
        """The master read the slave's DISC: the slave has left, and the master closes the
        session with its own so that the slave's closing identifier gets its slot (ADR-0075)."""
        self._disc_requested = False
        self._disc_tries = 0
        self._patience_until: float | None = None
        self._leave_since: float | None = None
        """When a disconnect was asked: by this station's operator, or — for the master — when
        it first read the slave's request (ADR-0072)."""
        self._leave_drain_s = self.cfg.disc_drain_s
        """How long this session's disconnect waits for what is queued (:meth:`disconnect`)."""
        self._measure_ask: tuple[int, int] | None = None
        """A measurement asked for (:meth:`measure`) and not yet sent: its rung and frames."""
        self._measure_out: tuple[int, int] | None = None
        """A measurement sent and not yet reported: its rung and the frames it went with. Until
        the report, this station sends no data, and every control frame of its says so (the
        ``measure`` bit): the other station keeps the report for it, and whatever that station
        holds to combine is of measurement frames, which it drops (ADR-0077)."""
        self._measure_unheard = False
        """The measurement's answer was not read: the report follows a poll, and the SNR it
        carries is the poll's, not the rung's (ADR-0076)."""
        self.rate = self._rate_controller()
        self._asked: int | None = None
        # receiving, either role
        self._rx_base = 0
        self._rx_buf: dict[int, bytes] = {}
        self._max_seen: int | None = None
        self._harq: dict[int, tuple[object, int, int]] = {}
        self._rx: list[_RxRecord] = []
        """The frames heard of the other station's transmission being received."""
        self._rx_t0: float | None = None
        self._grid: tuple[float, int, bool] | None = None
        """Where that transmission's data frames are, as its opening control frame stated:
        the first one's start, how many, their family."""
        self._ack_history: list[_AckSnapshot] = []
        self._measured: int | None = None
        """The report of the other station's last measurement, as a bitmap (:data:`REPORT`):
        what this station's acknowledgements carry from the measurement until the other
        station's data comes again."""
        self._rx_measure = False
        """The transmission being received is a measurement: its frames are filler, and one that
        does not decode is never sent again — nothing to combine it with."""
        # the master
        self._recent_silent: list[bool] = []
        """Whether each of the last answer slots went silent, the latest last."""
        self._slot = None
        self._level_asked = 0
        self._silence_level = 0
        self._silent = 0
        self._answered_run = 0
        self._slave_want: int | None = None
        """Frames the slave said it has queued; ``None`` before its first answer."""
        self._slave_leaves = False
        self._slave_floor_only = False
        """The slave said its data can go only on the tone floor (an answer's ``data_floor``
        with no data after it)."""
        self._hdr_floor = True
        self._possible: set[bool] = {True}
        """Every family the slave may answer in when it has not read the header: the family of
        its last answer read here and every family asked for since (R6)."""
        self._idle_windows = 0
        self._ack_owed = False
        self._heard_at = self.now
        self._peer_late_s = 0.0
        """How far past the slot's slack the slave's answers have been seen to start: a
        station whose keying is slower than it declared (CAT keying, a slow card) radiates
        late, and a master that waited only the slot's own length keyed over the end of an
        answer it had not yet decoded. Measured on the answers decoded, eased as they come in
        on time, and added to the wait for a slot nothing of which decoded — the master's own
        margin, which only ever delays it."""
        # the slave
        self._answer: _Answer | None = None
        self._grant: GrantFrame | None = None
        """The header of the master transmission being received, if it was read."""
        self._grant_floor = False
        self._m_end: float | None = None
        """Where that transmission ends, as the frames read of it say: the slot's anchor."""
        self._m_last_floor = False
        self._answered_end: float | None = None
        """The end of the last master transmission this station answered or forfeited."""
        self._ans_sticky = True
        self._need_level = 0
        self._on_time = 0
        self._window: tuple[float, float, bool] | None = None
        for name in ("proceed", "answer", "link"):
            self._deadlines.pop(name, None)

    # ── public commands ───────────────────────────────────────────────

    def set_callsigns(self, calls: Sequence[str]) -> None:
        """As :meth:`LinkEngine.set_callsigns`: refused while a session is up."""
        if self.state is not State.IDLE:
            raise RuntimeError("a session is running")
        cleaned = [c.strip().upper() for c in calls if c.strip()]
        if not cleaned:
            raise ValueError("at least one callsign is needed")
        for call in cleaned:
            pack_callsign(call)
        self.callsigns = cleaned
        self.my_call = cleaned[0]

    def _calling_as(self, as_call: str | None) -> None:
        if as_call is None:
            self.my_call = self.callsigns[0]
        elif as_call.upper() in self.callsigns:
            self.my_call = as_call.upper()
        else:
            raise ValueError(f"{as_call.upper()} is not one of this station's callsigns")

    def connect(self, remote_call: str, as_call: str | None = None) -> None:
        """Call a station; the caller is the session's master once it reads the acceptance.
        The first try goes once the last slot this station granted can no longer be in use."""
        if self.state is not State.IDLE:
            raise RuntimeError("already in a session")
        self._calling_as(as_call)
        remote = remote_call.upper()
        if self._id_owed > 0.0 and remote == self.remote_call:
            # the station calls back the one it just lost: the communication goes on, and is
            # identified at the end of the session this call opens (ADR-0055 §4, ADR-0075)
            self._drop_identifier()
        elif self._id_owed > 0.0:
            # another station: the last communication's identifier goes now, ahead of the call
            self._arm_at("ident", max(self.now, self._hold_until))
        self.remote_call = remote
        self.session = self._new_session_number()
        self.state = State.CONNECTING
        self.role = Role.NONE
        self._connect_tries = 0
        self._left = None
        self._disarm("leave")
        self._reset_session()
        self._my_ident_s = self._ident_for(self.my_call)
        self._cause = "connect"
        # an identifier still owed to another station's session goes first: the call waits
        # for it, as for the last slot this station granted
        if self.now >= self._hold_until - 1e-9 and "ident" not in self._deadlines:
            self._send_connect()
        else:
            self._arm_at("connect", self._hold_until)

    def _new_session_number(self) -> int:
        # 1–255, and never 0 in the control frame's seven bits: a header with no counts, at
        # level 0, and an SNR of 0 dB would be all zeros, which the PHY refuses
        while True:
            session = 1 + self.rng.below(255)
            if session & 0x7F:
                return session

    def probe(self, remote_call: str, as_call: str | None = None) -> None:
        """Ask a station whether it hears this one, outside a session (ADR-0006). The probe
        grants its answer a slot, at the level this station's own turnaround needs (a probe
        carries no turnaround); the answer comes in it or not at all."""
        if self.state is not State.IDLE:
            raise RuntimeError("already in a session")
        if self._probing is not None:
            raise RuntimeError("a probe is already out")
        self._calling_as(as_call)
        self._probing = remote_call.upper()
        if self._id_owed > 0.0:
            self._arm_at("ident", max(self.now, self._hold_until))  # it goes first
        self.last_probe = None
        self.stats.probes_sent += 1
        self._cause = "probe"
        self._probe_unsent = True
        if self.now >= self._hold_until - 1e-9 and "ident" not in self._deadlines:
            self._send_probe()
        else:
            self._arm_at("probe", self._hold_until)

    def disconnect(self, drain_s: float | None = None) -> None:
        """Orderly close. The master finishes what it has queued, and what the slave has, for
        no longer than :attr:`GrantConfig.disc_drain_s` from now (ADR-0072) — or ``drain_s``,
        0 to leave at its next opportunity with what it has not delivered left undelivered (the
        Test, whose last queued bytes are its ladder's filler: ADR-0076) — or its patience with
        nothing moving (ADR-0039), then sends DISC; the slave asks to leave in its next answer
        and the master closes the session."""
        if self.state is State.IDLE:
            return
        self._disc_requested = True
        if drain_s is not None:
            self._leave_drain_s = max(0.0, drain_s)
        if self._leave_since is None:
            self._leave_since = self.now
        if self.role is Role.SLAVE and self._window is not None:
            self._cause = "disconnect"
            self._try_window()

    @property
    def disconnect_requested(self) -> bool:
        return self._disc_requested and self.state is State.CONNECTED

    @property
    def leave_by(self) -> float | None:
        """When, at the latest, a disconnect asked for stops waiting for what is queued (engine
        time); the DISC goes at the master's next opportunity after it. ``None`` when none was
        asked. A slave's counts from its own request; the master's count starts when it reads
        it, an answer later."""
        if self._leave_since is None or self.state is not State.CONNECTED:
            return None
        return self._leave_since + self._leave_drain_s

    def abort(self) -> None:
        """Drop the session now. A master says DISC from idle, on the floor, at its next
        opportunities, until one is answered or :attr:`GrantConfig.leave_repeats` have gone; a
        slave, which never keys outside a slot, answers the old session's next header with a
        DISC in its slot (ADR-0052)."""
        if self.state is State.IDLE:
            return
        was_master = self.role is Role.MASTER
        self._cause = "abort"
        self._end_session("aborted")
        if was_master and self._left is not None:
            self._leave_repeats = self.cfg.leave_repeats
            self._arm_at("leave", max(self.now, self._hold_until))
        elif was_master:
            self._identify_after_leaving()

    def send(self, data: bytes) -> None:
        self._tx_queue += data
        if self.role is Role.SLAVE and self._window is not None:
            self._cause = "send"
            self._try_window()

    def set_answer_gap(self, seconds: float) -> None:
        """The least time between the other station's frames and this station radiating
        (:attr:`GrantConfig.answer_gap_s`): the daemon's, for the station it is calling or in a
        session with. From the next key as the master, the next answer as the slave."""
        self.cfg.answer_gap_s = max(0.0, seconds)

    def set_identifier_s(self, seconds: float) -> None:
        """This station's closing identifier's length for every callsign not given its own
        (:attr:`GrantConfig.identifier_s`): from the next call or acceptance on."""
        self.cfg.identifier_s = max(0.0, seconds)

    def set_identifiers(self, lengths: dict[str, float]) -> None:
        """The closing identifier's length by callsign (:attr:`GrantConfig.identifiers`), 0
        for a callsign that sends none: from the next call or acceptance on. A session keeps the
        length its call or acceptance declared, which is what the other station reckons with."""
        self.cfg.identifiers = {c.strip().upper(): max(0.0, s) for c, s in lengths.items()}

    def _ident_for(self, call: str) -> float:
        return self.cfg.identifiers.get(call.upper(), self.cfg.identifier_s)

    @property
    def peer_identifier_s(self) -> float:
        """The other station's closing identifier's length, as its call or acceptance declared
        it: the session's, and kept once it ends, while the identifiers are still to go
        (ADR-0075). A recording runs on past the session's end until it has (ADR-0076)."""
        return self._peer_identifier_s

    @property
    def identifier_owed(self) -> bool:
        """Whether a session's closing identifier is owed and not yet handed over: the daemon
        tells a host program the session is over only once it has gone out (ADR-0075)."""
        return self._id_owed > 0.0

    def pin_mode(self, mode: int | None, body_bytes: int | None = None) -> None:
        """As :meth:`LinkEngine.pin_mode`: this station's own new data goes at ``mode`` until
        unpinned, in bodies of at most ``body_bytes`` — the benches' and the tests'. The Test's
        ladder measures a rung with :meth:`measure` instead."""
        if mode is not None and mode not in (self.timing.data_capacity or {}):
            raise ValueError(f"mode {mode} is not in the table")
        if body_bytes is not None and body_bytes < 1:
            raise ValueError("body_bytes must be at least 1")
        self._pinned = mode
        self._pin_body = body_bytes if mode is not None else None

    def measure(self, mode: int, frames: int) -> None:
        """Measure ``mode`` on the path to the other station (ADR-0077): ``frames`` frames of
        filler at it, in a transmission of their own as soon as all of this station's data is
        acknowledged — as many as the transmission holds — never delivered and never sent again.
        What the other station reports of them goes to :attr:`ladder`; this station's data waits
        for the report. Asked again before it is sent, the later asking stands."""
        if mode not in (self.timing.data_capacity or {}):
            raise ValueError(f"mode {mode} is not in the table")
        if frames < 1:
            raise ValueError("a measurement takes at least one frame")
        self._measure_ask = (mode, frames)
        if self.role is Role.SLAVE and self._window is not None:
            self._cause = "measure"
            self._try_window()

    @property
    def measuring(self) -> bool:
        """A measurement is asked for and not yet reported."""
        return self._measure_ask is not None or self._measure_out is not None

    def take_ladder(self) -> list[LadderRung]:
        rungs, self.ladder = self.ladder, []
        return rungs

    def request_break(self) -> None:
        """Kept for the daemon's interface. Every answer states the slave's queue and the next
        header grants it, so the slave's data never waits for the master's to run out: there is
        no turn to break into."""

    @property
    def tx_pending_bytes(self) -> int:
        return len(self._tx_queue) + sum(len(r.body) for r in self._records.values() if not r.acked)

    @property
    def tx_undelivered_bytes(self) -> int:
        return len(self._tx_queue) + sum(len(r.body) for r in self._records.values())

    @property
    def probing(self) -> bool:
        return self._probing is not None

    def all_acknowledged(self) -> bool:
        return not self._tx_queue and all(r.acked for r in self._records.values())

    @property
    def connected(self) -> bool:
        return self.state is State.CONNECTED

    @property
    def current_mode(self) -> int:
        return self._recommended

    @property
    def level(self) -> int:
        """The gap level in use: what a master grants, what a slave asks for."""
        if self.role is Role.MASTER:
            return max(self._level_asked, self._silence_level)
        return self._asked_level()

    def drain(self) -> list[Action]:
        out, self.actions = self.actions, []
        return out

    def reset_stats(self) -> None:
        self.stats = GrantStats()

    def robust_mode(self, floor: bool) -> int:
        """The slowest mode of a family that carries a connect body: what calls, acceptances,
        probes and their answers go at."""
        need = CONNECT_BODY_8_BYTES + 5
        caps = self.timing.data_capacity or {}
        for m in sorted(caps):
            if self.timing.is_floor(m) == floor and caps[m] >= need:
                return m
        if floor:
            return self.robust_mode(False)
        raise ValueError("no mode carries a connect frame")

    def burst_capacity(self, frame_s: float) -> int:
        """Data frames of ``frame_s`` seconds one transmission may carry, one at least."""
        count = min(self.cfg.max_data_frames, MAX_DATA)
        if self.cfg.max_burst_s is not None and frame_s > 0:
            count = min(count, max(1, int(self.cfg.max_burst_s / frame_s + 1e-9)))
        return count

    def set_max_burst_s(self, seconds: float | None) -> None:
        self.cfg.max_burst_s = seconds

    def set_ceiling(self, rung: int | None) -> None:
        self.cfg.ceiling = rung

    @property
    def ceiling(self) -> int | None:
        return self.cfg.ceiling

    def set_max_mode(self, rung: int) -> None:
        self.cfg.max_mode = rung

    def set_air(self, timing: PhyTiming, capabilities: int, max_mode: int | None = None) -> None:
        """As :meth:`LinkEngine.set_air` (ADR-0026): another air from the next session on."""
        if self.state is not State.IDLE or self._probing is not None:
            raise RuntimeError("a session, a call or a probe is running")
        self._move(timing, capabilities, max_mode)

    def move_call(self, timing: PhyTiming, capabilities: int, max_mode: int | None = None) -> None:
        """As :meth:`LinkEngine.move_call` (ADR-0058): an unanswered call's next tries go out on
        another air, its tries and timers standing."""
        if self.state is not State.CONNECTING:
            raise RuntimeError("no call is going out")
        self._move(timing, capabilities, max_mode)

    def _move(self, timing: PhyTiming, capabilities: int, max_mode: int | None) -> None:
        self.timing = timing
        self.cfg.capabilities = capabilities
        if max_mode is not None:
            self.cfg.max_mode = max_mode
        self.rate = self._rate_controller()
        self._recommended = self.cfg.initial_mode

    @property
    def connect_tries(self) -> int:
        return self._connect_tries

    @property
    def connect_due(self) -> float | None:
        return self._deadlines.get("connect")

    # ── inputs ────────────────────────────────────────────────────────

    def on_tx_done(self, now: float, frames_end: float | None = None) -> None:
        """A transmission finished. ``frames_end`` is where its last frame ended on the air —
        the card's clock less the keying tail; ``now`` when not given. The slot a transmission
        granted is measured from it, as the other station measures it from the frames it hears
        end (07-protocol-8.md §3.4)."""
        self.now = max(self.now, now)
        if not self._in_flight:
            return
        granted, _, _ = self._in_flight.pop(0)
        if granted is not None and granted is self._slot and not granted.final:
            self._slot.final = True
            self._open_slot(now if frames_end is None else frames_end)

    def on_tx_cut_unread(self, now: float) -> None:
        """A transmission was cut — an abort, the key watchdog — before any frame of it could
        be read: its first frame a control frame not :data:`HEADER_UNDECODABLE_SHARE` of the
        way out (10-step-3.md §3.6). The other station learned nothing from it, so it granted
        nothing. No slot waits on it, and what waited for this station's own transmission to
        end — the master's next key, an abort's DISC — waits only for the cut. Reported as
        ending where its frames would have (``on_tx_done``), it held a Test's leaving DISC 29 s
        for the slot of a header nobody had read."""
        self.now = max(self.now, now)
        if not self._in_flight:
            return
        granted, ident, measured = self._in_flight.pop(0)
        self._owe_again(ident)
        self._ask_again(measured)
        if granted is None or granted is not self._slot or granted.final:
            return
        self._slot = None
        self._hold_until = self.now
        if self.role is Role.MASTER and self.state in (State.CONNECTED, State.DISCONNECTING):
            self._arm_at("proceed", self.now)

    def on_tx_dropped(self, now: float) -> None:
        """The keying gate did not start a transmission: it would have begun past its
        ``not_after``, and nothing of it was sent."""
        self.now = max(self.now, now)
        if self._in_flight:
            _, ident, measured = self._in_flight.pop(0)
            self._owe_again(ident)
            self._ask_again(measured)
        self.stats.dropped += 1

    def _ask_again(self, measured: bool) -> None:
        """A measurement that never reached the air is asked for again, not reported."""
        if measured and self._measure_out is not None and self._measure_ask is None:
            self._measure_ask, self._measure_out = self._measure_out, None
            self._measure_unheard = False

    def on_tx_delayed(self, seconds: float) -> None:
        """The daemon is holding this station's last transmission. A slot is absolute and a
        late answer is dropped by the gate, so what moves is only what waits for this station's
        own transmission to end: the master's next key, a call's next try, a probe's wait, a
        leaving DISC's repeat."""
        if seconds <= 0.0:
            return
        for name in self._deadlines:
            if name not in ("link", "answer"):
                self._deadlines[name] += seconds
        self._hold_until += seconds

    def tick(self, now: float) -> None:
        self.now = max(self.now, now)
        for name in sorted(self._deadlines, key=self._deadlines.__getitem__):
            if self._deadlines.get(name, math.inf) <= self.now + 1e-9:
                del self._deadlines[name]
                self._cause = f"timer:{name}"
                self._expire(name)

    def next_deadline(self) -> float | None:
        return min(self._deadlines.values()) if self._deadlines else None

    def on_preamble(self, t_start: float, now: float, frame_s: float | None = None) -> None:
        """A frame announced and not yet decoded. Inside a session it changes nothing: only
        decoded content does (ADR-0069 §2.5). A caller or a prober takes it as carrier — its
        next try, or its giving up, waits past the frame, which may be the answer."""
        self.now = max(self.now, now)
        if self.state is not State.CONNECTING and self._probing is None:
            return
        length = frame_s if frame_s is not None else self.timing.data_frame_s_for(0)
        clear = t_start + length + self._budget(True) + self.cfg.guard_s
        if "connect" in self._deadlines and self._connect_tries > 0:
            self._deadlines["connect"] = max(self._deadlines["connect"], clear)
        if "probe" in self._deadlines and not self._probe_unsent:
            self._deadlines["probe"] = max(self._deadlines["probe"], clear)

    def on_frame(self, frame: SoftFrame, now: float) -> None:
        self.now = max(self.now, now)
        self._cause = "frame"
        payload, _ = frame.decode(None)
        if frame.container is Container.CONTROL:
            if payload is None:
                return  # a control frame not read changes nothing
            ctl = None
            with contextlib.suppress(ValueError):
                ctl = GrantFrame.decode(payload)
            if ctl is not None:
                self._on_control(ctl, frame)
            return
        if payload is not None and not self._outside(payload, frame):
            return
        if self.state not in (State.CONNECTED, State.DISCONNECTING):
            return
        if payload is None and not frame.trusted:
            return  # untrusted and unread: nothing at all
        if frame.floor != self.timing.is_floor(frame.mode):
            self.stats.frames_failed += 1  # the chips are noise
        elif self.role is Role.MASTER:
            self._master_data(frame, payload is not None)
        elif self.role is Role.SLAVE:
            self._slave_data(frame, payload is not None)

    def _outside(self, payload: bytes, frame: SoftFrame) -> bool:
        """Handle a decoded DATA frame that is no session data — a call, an acceptance, a probe
        or its answer — and ``True`` only for this session's data."""
        try:
            header, body = decode_data(payload)
        except ValueError:
            return False
        if header.kind in (DataKind.CONNECT_REQ, DataKind.CONNECT_ACK):
            self._on_connect(header, body, frame)
            return False
        if header.kind in (DataKind.PROBE, DataKind.PROBE_ACK):
            self._on_probe(header.kind, body, frame)
            return False
        # a beacon or a datagram is nobody's session's, whatever its session byte says
        return header.kind not in OUTSIDE_SESSIONS and header.session == self.session

    # ── timers ────────────────────────────────────────────────────────

    def _arm_at(self, name: str, at: float) -> None:
        self._deadlines[name] = at

    def _disarm(self, name: str) -> None:
        self._deadlines.pop(name, None)

    def _expire(self, name: str) -> None:
        if name == "link":
            # whole exchanges of the size in use now: the answer that last armed the timer may
            # have moved the link to the floor, whose exchanges are three times as long
            due = self._heard_at + self._link_timeout()
            if due > self.now:
                self._arm_at("link", due)
            else:
                self._end_session("link timeout")
        elif name == "connect":
            if self.now < self._hold_until - 1e-9:
                self._arm_at("connect", self._hold_until)  # a closing identifier went first
            else:
                self._retry_connect()
        elif name == "probe":
            if self._probe_unsent and self.now < self._hold_until - 1e-9:
                self._arm_at("probe", self._hold_until)
            elif self._probe_unsent:
                self._send_probe()
            elif self._probing is not None:
                self.actions.append(Event("probe", f"{self._probing}: no answer"))
                self._probing = None
        elif name == "proceed":
            self._proceed()
        elif name == "answer":
            self._key_answer()
        elif name == "leave":
            self._repeat_leave()
        elif name == "ident":
            self._send_identifier()

    # ── timing ────────────────────────────────────────────────────────

    def _budget(self, floor: bool) -> float:
        return self.cfg.decode_budget_s[int(floor)]

    def _own_latency(self) -> float:
        return self.timing.tx_latency_s

    def _slot_start(self, end: float, floor: bool, level: int, granter_turnaround: float) -> float:
        """Where the slot granted by a transmission ending at ``end`` starts, as both stations
        reckon it (07-protocol-8.md §3.1)."""
        cfg = self.cfg
        gap = max(self._budget(floor) + cfg.key_s, granter_turnaround) + cfg.guard_s
        return end + gap + cfg.levels_s[min(level, len(cfg.levels_s) - 1)]

    def _after(self, end: float, floor: bool, other_turnaround: float) -> float:
        """The earliest this station keys after the other's transmission ends at ``end``: with
        its last frame decoded, and radiating no sooner than the other hears again (§3.2). The
        keying latency counted is this station's own, never more than the protocol's: a
        station that radiates sooner than ``KEY`` waits the difference."""
        lead = min(self.cfg.key_s, self._own_latency())
        return end + max(self._budget(floor), other_turnaround - lead) + self.cfg.guard_s

    def _peer_wait_s(self) -> float:
        """How long after the other station's frames this station's radiation waits: the
        other's declared turnaround, or the answer gap when that is longer."""
        return max(self._peer_turnaround_s, self.cfg.answer_gap_s)

    def _asked_level(self) -> int:
        """The gap level this station asks for as the slave: what its own lateness needs, and at
        least what starts its slots the answer gap after the master's frames."""
        gap = self.cfg.answer_gap_s
        if gap <= 0.0:
            return self._need_level
        base = min(
            max(self._budget(floor) + self.cfg.key_s, self._peer_turnaround_s)
            for floor in (False, True)
        )
        return max(self._need_level, self._level_for(gap - base - self.cfg.guard_s))

    def _level_for(self, extra_s: float) -> int:
        """The smallest gap level at least ``extra_s`` long."""
        for i, step in enumerate(self.cfg.levels_s):
            if extra_s <= step + 1e-9:
                return i
        return len(self.cfg.levels_s) - 1

    def _ctl_s(self, floor: bool) -> float:
        return self.timing.control_frame_s_for(floor)

    def _frame_s(self, floor: bool) -> float:
        if floor and self.timing.floor_data_frame_s is not None:
            return self.timing.floor_data_frame_s
        return self.timing.data_frame_s

    def _robust_frame_s(self, floor: bool) -> float:
        return self.timing.data_frame_s_for(self.robust_mode(floor))

    def _window_s(self, floor: bool, index: int) -> float:
        return self.cfg.open_window_s[int(floor)] * (1 << index)

    def _exchange_s(self) -> float:
        """One whole exchange of the size in use: two full transmissions at the rungs the link
        runs at with their control frames, the gaps at the highest level and the slack — or an
        idle exchange's longest open window, whichever is longer."""
        floor = (
            self.timing.is_floor(self._burst_mode())
            or self.timing.is_floor(self.rate.recommend())
            or self._hdr_floor
        )
        frame = max(
            self._frame_s(self.timing.is_floor(self._burst_mode())),
            self._frame_s(self.timing.is_floor(self.rate.recommend())),
        )
        gap = (
            self._slot_start(0.0, True, len(self.cfg.levels_s) - 1, self._peer_turnaround_s)
            + self.cfg.slack_s
        )
        busy = 2 * (self._ctl_s(floor) + self.burst_capacity(frame) * frame + gap)
        idle = self._ctl_s(True) + self._window_s(True, self.cfg.window_doublings) + 2 * gap
        return max(busy, idle)

    def _link_timeout(self) -> float:
        return max(self.cfg.link_timeout_s, self.cfg.link_timeout_exchanges * self._exchange_s())

    def _disc_patience(self) -> float:
        return max(self.cfg.disc_patience_s, self.cfg.disc_patience_exchanges * self._exchange_s())

    def _heard(self, frame: SoftFrame) -> None:
        """A decoded frame of the session: the link is alive."""
        self._heard_db = frame.snr_db
        self._heard_recent = [*self._heard_recent, frame.snr_db][-self.cfg.family_readings :]
        self._heard_at = self.now
        self._arm_at("link", self.now + self._link_timeout())

    # ── rate and family ───────────────────────────────────────────────

    def _cap(self) -> int:
        if self.cfg.ceiling is None:
            return self.cfg.max_mode
        return min(self.cfg.max_mode, self.cfg.ceiling)

    def _floor_only(self) -> bool:
        return self.cfg.ceiling is not None and self.timing.is_floor(self.cfg.ceiling)

    def _burst_mode(self) -> int:
        chosen = self._recommended if self._pinned is None else self._pinned
        return min(chosen, self._cap())

    def _ctl_threshold(self) -> float:
        return (self.timing.control_threshold_db or CONTROL_THRESHOLD_DB)[False]

    def _wants_floor(self) -> bool:
        """Whether this station needs the other's control frames on the floor: it hears the
        other under the ordinary control frame's threshold plus a margin, or not yet at all."""
        if self.timing.floor_modes <= 0:
            return False
        if self._floor_only() or not self._heard_recent:
            return True
        return min(self._heard_recent) < self._ctl_threshold() + self.cfg.answer_floor_margin_db

    def _recommend(self, floor_only: bool = False) -> int:
        """The rung this station's control frame recommends for the other's data, remembered
        (ADR-0020). ``floor_only``: the other station has said its data can only go on the tone
        floor — its ceiling or its pinned rung is there — and a header's rung is also what its
        grant is sized by, so it stays on the floor: granted in ordinary frames, a floor-only
        slave's data never fitted, and never went."""
        rung = self.rate.recommend()
        if floor_only and self.timing.floor_modes > 0:
            rung = min(rung, self.timing.floor_modes - 1)
        self._asked = rung if self._asked is None else max(self._asked, rung)
        return rung

    def _back_off(self) -> None:
        modes = self.rate.modes
        current = min(self._recommended, self._cap())
        below = [m for m in modes if m <= current]
        index = modes.index(below[-1]) if below else 0
        self._recommended = modes[max(0, index - self.cfg.silence_step)]

    def _hopeless(self, mode: int) -> bool:
        thresholds = self.rate.thresholds
        if self.peer_snr_db is None or mode not in thresholds:
            return False
        reach = thresholds[mode] - 10.0 * math.log10(max(1, self.cfg.max_combines))
        return self.peer_snr_db < reach - self.cfg.reencode_hopeless_db

    def _fits(self, body_len: int, mode: int) -> bool:
        cap = data_capacity(self.timing.capacity(mode))
        return body_len == cap or body_len <= cap - 2

    def _reencode_target(self, body_len: int, from_mode: int, to_mode: int) -> int | None:
        for mode in sorted(self.timing.data_capacity or {}):
            if to_mode <= mode < from_mode and self._fits(body_len, mode):
                return mode
        return None

    def _rate_controller(self) -> RateController:
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

    # ── keying ────────────────────────────────────────────────────────

    def _transmit(
        self,
        frames: list[TxFrame],
        *,
        not_after: float | None = None,
        not_before: float | None = None,
        identifier_s: float = 0.0,
        listen_first: bool = False,
        granting: _Slot | None = None,
    ) -> float:
        """Hand a transmission over; returns where it ends on the air, as far as this engine
        knows. An identifier goes after the frames (ADR-0075)."""
        dur = identifier_s + sum(self.timing.frame_s(f) for f in frames)
        self.actions.append(
            SlotTransmit(
                frames,
                dur,
                not_after=not_after,
                not_before=not_before,
                identifier_s=identifier_s,
                listen_first=listen_first,
                cause=self._cause,
            )
        )
        self._in_flight.append(
            (granting, identifier_s if frames else 0.0, _carries_measurement(frames))
        )
        self.stats.transmissions += 1
        on_air = self.now + self._own_latency()
        if not_before is not None:
            on_air = max(on_air, not_before)
        return on_air + dur

    def _control(self, ctl: GrantFrame, floor: bool) -> TxFrame:
        return TxFrame(Container.CONTROL, ctl.encode(), floor=floor)

    # ── slots this station grants ─────────────────────────────────────

    def _key_granting(self, slot: _Slot, frames: list[TxFrame]) -> None:
        """Key a transmission that grants a slot, and wait for the slot — placed from this
        engine's estimate of the transmission's end until ``on_tx_done`` says where it was."""
        self._slot = slot
        end = self._transmit(frames, granting=slot)
        self._open_slot(end)

    def _open_slot(self, end: float) -> None:
        slot = self._slot
        if slot is None:
            return
        slot.tx_end = end
        slot.start = self._slot_start(end, slot.last_floor, slot.level, slot.granter_turnaround_s)
        slot.end_max = slot.start + slot.grant_s + (0.0 if slot.open_window else self.cfg.slack_s)
        self._arm_wait()

    def _slot_close(self, slot: _Slot) -> float:
        """When this station may key again: a gap after the end a decoded frame of the slot
        stated, or after the slot's latest end when nothing of it was decoded (§3.2)."""
        if slot.stated_end is not None:
            return self._after(slot.stated_end, slot.stated_floor, self._peer_wait_s())
        end = slot.end_max + self._peer_late_s
        return self._after(end, any(slot.answer_floors), self._peer_wait_s())

    def _arm_wait(self) -> None:
        """Arm what ends this station's wait for the slot it granted last."""
        slot = self._slot
        if slot is None:
            return
        due = self._slot_close(slot)
        self._hold_until = due
        if slot.kind == "connect" and self.state is State.CONNECTING:
            self._arm_at("connect", due + self._call_jitter)
        elif slot.kind == "probe":
            if self._probing is not None and not self._probe_unsent:
                self._arm_at("probe", due)
        elif self.role is Role.MASTER and self.state in (State.CONNECTED, State.DISCONNECTING):
            self._arm_at("proceed", max(self.now, due))

    def _in_slot(self, frame: SoftFrame) -> bool:
        """A frame of the other station's is of the slot this station granted last if it began
        after the granting transmission ended: the other station keys only in a slot."""
        slot = self._slot
        return slot is not None and slot.final and frame.t_start >= slot.tx_end - 1e-9

    def _state_end(self, end: float, floor: bool) -> None:
        """A decoded frame of the slot says where the other station's transmission ends."""
        slot = self._slot
        if slot is None:
            return
        slot.heard = True
        if slot.stated_end is None or end >= slot.stated_end - 1e-9:
            slot.stated_end = end
            slot.stated_floor = floor
        self._arm_wait()

    # ── the call and the probe ────────────────────────────────────────

    def _connect_floor(self) -> bool:
        if self.timing.floor_modes > 0 and self._floor_only():
            return True
        return self.timing.floor_modes > 0 and self._connect_tries % 2 == 0

    def _call_level(self) -> int:
        """The gap level a call grants its acceptance: up one every second try, so a called
        station whose computer decodes late is reached (capability bits 3–4)."""
        return min(len(self.cfg.levels_s) - 1, self._connect_tries // 2)

    def _send_connect(self) -> None:
        """A try of the call: on the tone floor first, the families alternating (ADR-0016); the
        acceptance's slot at the try's level; the next try once that slot can no longer be in
        use, and a random part of a floor frame per try later, so that two stations calling
        each other fall apart."""
        floor = self._connect_floor()
        level = self._call_level()
        body = ConnectBody(
            self.my_call,
            self.remote_call,
            caps=with_level(self.cfg.capabilities, level),
            version=PROTOCOL_VERSION,
            turnaround_s=self.cfg.turnaround_s,
            identifier_s=self._my_ident_s,
        ).encode()
        mode = self.robust_mode(floor)
        payload = encode_data(
            DataHeader(DataKind.CONNECT_REQ, 0, self.session), body, self.timing.capacity(mode)
        )
        self._connect_tries += 1
        self._call_jitter = self.rng.uniform(
            0.0, (1 + self._connect_tries) * self._robust_frame_s(True)
        )
        # the acceptance comes in the request's family, or on the floor when the call arrived
        # weak (ADR-0056) or the rules allow only the floor
        families = frozenset({True, False}) if self.timing.floor_modes > 0 else frozenset({False})
        self._key_granting(
            _Slot(
                kind="connect",
                grant_s=max(self._robust_frame_s(f) for f in families),
                last_floor=self.timing.is_floor(mode),
                answer_floors=families,
                level=level,
                granter_turnaround_s=self.cfg.turnaround_s,
            ),
            [TxFrame(Container.DATA, payload, mode=mode, rv=0)],
        )

    def _retry_connect(self) -> None:
        if self.state is not State.CONNECTING:
            return
        if self._connect_tries >= self.cfg.connect_retries:
            self._end_session("no answer")
            return
        self._send_connect()

    def _send_probe(self) -> None:
        """The probe goes on the tone floor (ADR-0016). It carries no turnaround: its level
        covers this station's own."""
        self._probe_unsent = False
        if self._probing is None:
            return
        level = self._level_for(self.cfg.turnaround_s - self._budget(True) - self.cfg.key_s)
        body = ProbeBody(
            self.my_call, self._probing, None, caps=with_level(self.cfg.capabilities, level)
        ).encode()
        mode = self.robust_mode(True)
        payload = encode_data(DataHeader(DataKind.PROBE, 0, 0), body, self.timing.capacity(mode))
        families = frozenset({True, False}) if self.timing.floor_modes > 0 else frozenset({False})
        self._key_granting(
            _Slot(
                kind="probe",
                grant_s=max(self._robust_frame_s(f) for f in families),
                last_floor=self.timing.is_floor(mode),
                answer_floors=families,
                level=level,
                granter_turnaround_s=0.0,
            ),
            [TxFrame(Container.DATA, payload, mode=mode, rv=0)],
        )

    def _on_connect(self, header: DataHeader, body: bytes, frame: SoftFrame) -> None:
        try:
            cb = ConnectBody.decode(body)
        except ValueError:
            return
        if header.kind is DataKind.CONNECT_REQ:
            self._on_request(header, cb, frame, len(body))
        else:
            self._on_acceptance(header, cb, frame, len(body))

    def _version_ok(self, cb: ConnectBody, size: int, verb: str) -> bool:
        if cb.version == PROTOCOL_VERSION and size >= CONNECT_BODY_8_BYTES:
            return True
        self.actions.append(Event("ignored", f"{cb.src} {verb} with link protocol {cb.version}"))
        return False

    def _on_request(
        self, header: DataHeader, req: ConnectBody, frame: SoftFrame, size: int
    ) -> None:
        if req.dst not in self.callsigns:
            return
        if bandwidth_code(req.caps) != bandwidth_code(self.cfg.capabilities):
            self.actions.append(
                Event("ignored", f"{req.src} calls in another bandwidth{self._mismatch(req.caps)}")
            )
            return
        if not self._version_ok(req, size, "calls"):
            return
        start = self._slot_start(frame.t_end, frame.floor, level_of(req.caps), req.turnaround_s)
        if self.state in (State.CONNECTED, State.DISCONNECTING) and not self._called_again(
            header, req, frame, start
        ):
            return
        if start < self._hold_until - 1e-9 or self._probing is not None:
            return  # its slot would fall in one this station granted, still open
        if self.state is State.CONNECTING:
            if req.src == self.remote_call and self.my_call > self.remote_call:
                return  # simultaneous calls: the higher callsign keeps calling
            self._disarm("connect")
        self._accept_call(header, req, frame, start)

    def _called_again(
        self, header: DataHeader, req: ConnectBody, frame: SoftFrame, start: float
    ) -> bool:
        """A call heard in a session. The peer's under a new session number has given the
        session up (ADR-0056): the session ends, ``True``, and the call is answered as from
        idle. The peer's under this session's number is a try whose acceptance was not read:
        answered again, in its own slot, with the SNR it arrived at (P9-2)."""
        if req.src != self.remote_call:
            return False  # another station's call: this one is in a session
        if header.session != self.session:
            self._end_session(f"{req.src} called again")
            return True
        if self.role is Role.SLAVE and self._m_end is None and self._grant is None:
            self._heard(frame)
            self._measure_lateness(frame)
            self._schedule_acceptance(req, frame, start)
        return False

    def _accept_call(
        self, header: DataHeader, req: ConnectBody, frame: SoftFrame, start: float
    ) -> None:
        self.my_call = req.dst
        # an identifier still owed for an earlier session is not sent now: an acceptance cannot
        # wait for it, and this session ends with one (ADR-0075)
        self._drop_identifier()
        self.remote_call = req.src
        self.session = header.session
        self._left = None
        self._disarm("leave")
        self._reset_session()
        self._my_ident_s = self._ident_for(self.my_call)
        self.peer_capabilities = req.caps & ~CAP_LEVEL_MASK & 0xFF
        self._peer_turnaround_s = req.turnaround_s
        self._peer_identifier_s = req.identifier_s
        self.state = State.CONNECTED
        self.role = Role.SLAVE
        self._heard(frame)
        self._measure_lateness(frame)
        # the request is the first measurement of how the caller is heard; a lower bound if it
        # came on the tone floor (ADR-0016)
        self.rate.seed(frame.snr_db, lower_bound=frame.floor)
        self._ans_sticky = self._acceptance_floor(frame)
        self.actions.append(Event("connected", f"{self.remote_call} (slave)"))
        self.actions.append(Event("role", "slave"))
        self._schedule_acceptance(req, frame, start)

    def _acceptance_floor(self, frame: SoftFrame) -> bool:
        """The acceptance goes back in the request's family — or on the floor when an ordinary
        call arrived too weak to bet the session on an ordinary acceptance making it back
        (ADR-0056), or when the rules allow only the floor."""
        if frame.floor or self._floor_only():
            return self.timing.floor_modes > 0
        if self.timing.floor_modes <= 0:
            return False
        threshold = self.rate.thresholds.get(self.robust_mode(False))
        return threshold is not None and frame.snr_db < threshold + self.cfg.floor_answer_margin_db

    def _schedule_acceptance(self, req: ConnectBody, frame: SoftFrame, start: float) -> None:
        self._schedule(
            _Answer(
                what="connect_ack",
                start=start,
                not_after=start + self.cfg.slack_s,
                floor=self._acceptance_floor(frame),
                session=self.session,
                anchor=frame.t_end,
                remote=req.src,
                snr_db=frame.snr_db,
            )
        )

    def _on_acceptance(
        self, header: DataHeader, ack: ConnectBody, frame: SoftFrame, size: int
    ) -> None:
        if self.state is not State.CONNECTING or header.session != self.session:
            return
        if ack.dst != self.my_call:
            return
        if bandwidth_code(ack.caps) != bandwidth_code(self.cfg.capabilities):
            self.actions.append(Event("ignored", f"{ack.src} answers in another bandwidth"))
            return
        if not self._version_ok(ack, size, "answers"):
            return
        self._disarm("connect")
        self.peer_capabilities = ack.caps & ~CAP_LEVEL_MASK & 0xFF
        self._peer_turnaround_s = ack.turnaround_s
        self._peer_identifier_s = ack.identifier_s
        # an acceptance's level bits say the gap level the slave needs, measured on the call
        self._level_asked = level_of(ack.caps)
        self.state = State.CONNECTED
        self.role = Role.MASTER
        self._heard(frame)
        self.rate.seed(frame.snr_db, lower_bound=frame.floor)
        self._recommended = self.cfg.initial_mode
        if ack.snr_db is not None:
            first = self.rate.first_mode(ack.snr_db)
            self._recommended = max(self.cfg.initial_mode, min(self._cap(), first))
            self.peer_snr_db = ack.snr_db
            # the first header goes in the family the slave's reading of the call asks for: a
            # floor reading is a lower bound, never too fast
            if self.timing.floor_modes > 0:
                self._hdr_floor = self._floor_only() or (
                    ack.snr_db < self._ctl_threshold() + self.cfg.answer_floor_margin_db
                )
        self._possible = {frame.floor}
        self.actions.append(Event("connected", f"{self.remote_call} (master)"))
        self.actions.append(Event("role", "master"))
        if self._slot is not None and self._in_slot(frame):
            self._state_end(frame.t_end, frame.floor)
        else:
            # an earlier try's acceptance, read late: the latest try's slot may be in use
            self._arm_wait()

    def _on_probe(self, kind: DataKind, body: bytes, frame: SoftFrame) -> None:
        try:
            pb = ProbeBody.decode(body)
        except ValueError:
            return
        if kind is DataKind.PROBE:
            if pb.dst not in self.callsigns or self.state is not State.IDLE:
                return  # a session's frames matter more than a question from outside it
            if self._probing is not None:
                return
            start = self._slot_start(frame.t_end, frame.floor, level_of(pb.caps), 0.0)
            if start < self._hold_until - 1e-9:
                return
            # answered across bandwidths too, on the floor both airs share (ADR-0035)
            across = bandwidth_code(pb.caps) != bandwidth_code(self.cfg.capabilities)
            self.my_call = pb.dst
            self.stats.probes_answered += 1
            self.actions.append(
                Event("probed", f"{pb.src} at {frame.snr_db:.1f} dB{self._mismatch(pb.caps)}")
            )
            self._schedule(
                _Answer(
                    what="probe_ack",
                    start=start,
                    not_after=start + self.cfg.slack_s,
                    floor=self.timing.floor_modes > 0
                    and (across or frame.floor or self._floor_only()),
                    remote=pb.src,
                    snr_db=frame.snr_db,
                    anchor=frame.t_end,
                )
            )
            return
        if self._probing is None or pb.dst != self.my_call or pb.src != self._probing:
            return
        self._disarm("probe")
        self._probing = None
        self.stats.probe_replies += 1
        self.last_probe = ProbeResult(pb.src, pb.snr_db, frame.snr_db, bandwidth_hz_of(pb.caps))
        theirs = "?" if pb.snr_db is None else f"{pb.snr_db:.0f}"
        self.actions.append(
            Event(
                "probe",
                f"{pb.src} hears us at {theirs} dB, heard at {frame.snr_db:.1f} dB"
                f"{self._mismatch(pb.caps)}",
            )
        )
        if self._slot is not None and self._slot.kind == "probe" and self._in_slot(frame):
            self._state_end(frame.t_end, frame.floor)

    def _mismatch(self, caps: int) -> str:
        if bandwidth_code(caps) == bandwidth_code(self.cfg.capabilities):
            return ""
        theirs = bandwidth_hz_of(caps)
        ours = bandwidth_hz_of(self.cfg.capabilities)
        there = "an unknown bandwidth" if theirs is None else f"{theirs} Hz"
        here = "an unknown bandwidth" if ours is None else f"{ours} Hz"
        return f" — runs {there}, this station {here}"

    # ── control frames ────────────────────────────────────────────────

    def _on_control(self, ctl: GrantFrame, frame: SoftFrame) -> None:
        if self.state in (State.IDLE, State.CONNECTING):
            self._answer_left(ctl, frame)
            return
        if ctl.session != self.session & 0x7F:
            return
        if self.role is Role.MASTER:
            self._master_control(ctl, frame)
        else:
            self._slave_control(ctl, frame)

    # ── the master ────────────────────────────────────────────────────

    def _master_control(self, ctl: GrantFrame, frame: SoftFrame) -> None:
        if not self._in_slot(frame):
            return  # a frame of a slot already closed: its timing is nothing's now
        self._heard(frame)
        if ctl.snr_db is not None:
            self.peer_snr_db = ctl.snr_db
        if ctl.kind is GrantKind.DISC_ACK:
            if self.state is State.DISCONNECTING:
                # the slave's closing identifier follows its DISC_ACK, as long as it declared
                # (ADR-0075): its transmission ends there, and the master's identifier goes
                # after it, whatever the slot's latest end
                self._state_end(frame.t_end + self._peer_identifier_s, frame.floor)
                self._end_session("peer disconnected" if self._peer_left else "closed")
            return
        if ctl.kind is GrantKind.DISC:
            # the slave has left (an abort, a left session's answer). The master closes with a
            # DISC of its own, whose slot carries the slave's closing identifier (ADR-0075);
            # ended here, neither station's identifier had a place both could count on
            self._peer_left = True
            self.state = State.DISCONNECTING
            self._state_end(frame.t_end, frame.floor)
            return
        if ctl.kind is not GrantKind.ANSWER:
            return
        slot = self._slot
        if slot is not None and not slot.open_window:
            late = frame.t_start - slot.start - self.cfg.slack_s
            if late > self._peer_late_s:
                self._peer_late_s = late
            else:
                self._peer_late_s = max(0.0, late, self._peer_late_s - 0.1)
        self._take_ack(ctl)
        self._slave_want = ctl.n
        self._level_asked = ctl.level
        self._hdr_floor = ctl.fam or self._floor_only()
        self._possible = {frame.floor}
        self._slave_leaves = ctl.flag
        if ctl.flag and self._leave_since is None:
            self._leave_since = self.now
        if not ctl.count:
            self._slave_floor_only = ctl.data_floor
        self._open_rx(frame, ctl.count, ctl.data_floor)
        self._note_opening(ctl)
        if ctl.count or ctl.measure:
            self._ack_owed = True
        last_floor = ctl.data_floor if ctl.count else frame.floor
        self._state_end(frame.t_end + ctl.count * self._frame_s(ctl.data_floor), last_floor)

    def _master_data(self, frame: SoftFrame, decoded: bool) -> None:
        if not self._in_slot(frame):
            if decoded:
                # of an earlier slot, decoded late: what it carries is kept (sequence numbers
                # make that safe), its timing is not
                payload, _ = frame.decode(None)
                if payload is not None:
                    self._accept(_RxRecord(frame, 0), payload, late=True)
            return
        rec = self._take_data(frame)
        if rec.payload is None:
            return
        self._heard(frame)
        self._ack_owed = True
        if frame.follows is not None and frame.follows < MAX_FOLLOWS and not rec.combined:
            # a count rescued with its block is not believed (the note at _take_data): the
            # master waits for the slot's latest end
            floor = self.timing.is_floor(frame.mode)
            self._state_end(frame.t_end + frame.follows * (frame.t_end - frame.t_start), floor)
        elif self._slot is not None:
            self._slot.heard = True

    def _proceed(self) -> None:
        """The slot is over: the master keys again."""
        if self.role is not Role.MASTER or self.state not in (
            State.CONNECTED,
            State.DISCONNECTING,
        ):
            return
        self._close_slot()
        if self.state is State.IDLE:
            return
        if self.state is State.DISCONNECTING:
            if self._disc_tries >= self.cfg.disc_retries:
                self._end_session(
                    "peer disconnected" if self._peer_left else "closed (no DISC_ACK)",
                    unread=True,
                )
                return
            self._send_disc()
            return
        if (self._disc_requested or self._slave_leaves) and self._can_leave():
            self.state = State.DISCONNECTING
            self._send_disc()
            return
        self._send_header()

    def _can_leave(self) -> bool:
        """Whether the master may send its DISC now. What the slave still has goes first, and
        the master's own queue when the disconnect is its own; when the slave asked, its
        operator has stopped receiving, and what the master has not delivered stays undelivered
        (ADR-0023's receiving station; `80m-leave-together-500`). Either waits no longer than
        :meth:`_disc_patience` (ADR-0039)."""
        own_done = self.all_acknowledged() or self._slave_leaves
        if own_done and not self._slave_want and self._max_seen is None:
            return True
        if self._patience_until is None:
            self._patience_until = self.now + self._disc_patience()
        if self._leave_since is None:
            self._leave_since = self.now
        if self.now >= self._leave_since + self._leave_drain_s:
            self._say_left(f"{self.now - self._leave_since:.0f} s after the disconnect was asked")
            return True
        if self.now >= self._patience_until:
            self._say_left(f"nothing acknowledged for {self._disc_patience():.0f} s")
            return True
        return False

    def _say_left(self, why: str) -> None:
        """What a closing master leaves behind, said once as it decides to close: its own
        queue when the disconnect is its own, and what the slave still said it had."""
        left = []
        own = 0 if self._slave_leaves else self.tx_undelivered_bytes
        if own:
            left.append(f"{own} bytes not delivered")
        if self._slave_want:
            left.append(f"the other station's last {self._slave_want} frames not received")
        if left:
            self.actions.append(Event("disconnect", f"{'; '.join(left)}; closing {why}"))

    def _close_slot(self) -> None:
        """What the slave sent in the slot goes to the rate controller; silence is counted and
        stepped down from (ADR-0069 §2.5)."""
        slot, self._slot = self._slot, None
        if slot is None:
            return
        if self._rx:
            self._observe(self._rx)
        self._restart_rx()
        if not slot.open_window:
            memory = self.cfg.floor_answer_memory
            self._recent_silent = [*self._recent_silent, not slot.heard][-memory:] if memory else []
        if slot.heard:
            self._silent = 0
            self._answered_run += 1
            if self._answered_run >= self.cfg.on_time_to_lower and self._silence_level > 0:
                self._silence_level -= 1
                self._answered_run = 0
            self._idle_windows = self._idle_windows + 1 if slot.open_window else 0
            return
        if slot.open_window:
            self._idle_windows += 1  # an open window nobody needed is no silence
            return
        self.stats.silent_slots += 1
        self._silent += 1
        self._answered_run = 0
        if self._measure_out is not None:
            self._measure_unheard = True
        if slot.data and self.cfg.hold_unknown:
            for s in self._unacked():
                self._records[s].unknown = True
        if (
            self._silent >= self.cfg.silence_levels_after
            and self._silence_level < len(self.cfg.levels_s) - 1
        ):
            self._silence_level += 1
        if slot.data:
            self._back_off()
        if slot.granted and self._silent >= 2:
            self.rate.observe(None, 0, 1)
        if self._silent > self.cfg.max_retries:
            self._end_session("no response")

    def _has_work(self) -> bool:
        return bool(self._tx_queue) or bool(self._unacked()) or self.measuring

    def _grant_size(self, rung: int) -> tuple[int, bool, int]:
        """(data frames granted, open window, window index): the master's policy (§5.2)."""
        frame = self._frame_s(self.timing.is_floor(rung))
        fit = min(self.cfg.max_data_frames, MAX_DATA)
        if self.cfg.max_burst_s is not None:
            room = self.cfg.max_burst_s - self._ctl_s(True) - self._peer_identifier_s
            fit = min(fit, max(0, int(room / frame + 1e-9)))
        if self._slave_want is None:
            return min(self.cfg.first_grant, fit), False, 0
        if self._slave_want > 0:
            return min(self._slave_want, fit), False, 0
        idle = not self._has_work() and not self._disc_requested
        if idle and self.now - self._heard_at < self.cfg.keepalive_s:
            return 0, True, min(self.cfg.window_doublings, self._idle_windows)
        return 0, False, 0

    def _elide(self, n: int, open_window: bool, wanted: bool) -> bool:
        """Whether this transmission may go without its header (R7): data only, nothing granted
        beyond an answer, the gap at level 0, nothing to acknowledge or recommend anew, the last
        slot heard, no measurement under way, and the answer's family one the slave surely
        holds."""
        return (
            self.cfg.elide
            and not open_window
            and n == 0
            and self._slave_want == 0
            and self._silent == 0
            and self.level == 0
            and self._possible == {wanted}
            and not self._ack_owed
            and self._has_work()
        )

    def _send_header(self) -> None:
        rung = self._recommend(self._slave_floor_only)
        n, open_window, index = self._grant_size(rung)
        wanted = self._wants_floor() or self._silent >= 2 or any(self._recent_silent)
        elide = self._elide(n, open_window, wanted)
        hdr_floor = self._hdr_floor or self._silent >= 2 or self._floor_only()
        room = math.inf if self.cfg.max_burst_s is None else self.cfg.max_burst_s
        data: list[TxFrame] = []
        if not open_window:
            data = self._data_frames(room - self._ctl_s(hdr_floor))
        # a measurement's frames, and every transmission until its report, say so in the
        # header, which therefore always goes (ADR-0077)
        measuring = self._measure_out is not None
        elide = elide and not measuring
        # the header is left out only of a transmission that carries no fewer data frames for
        # it: capped at the four whose counts can anchor it, the 15-byte rung of the 500 Hz
        # air sent 136 transmissions where protocol 7 sent 82 (the link bench, AWGN -3 dB).
        # And only where a count can anchor it at all: the slave believes a count only from a
        # copy that decoded alone (R2), and only an RV 0 copy can on every rung (ADR-0073)
        elide = (
            elide
            and 0 < len(data) <= MAX_FOLLOWS + 1
            and any(f.rv in READABLE_ALONE_RVS for f in data)
        )
        # data above the rung the slave asks for is a bet that it decodes — the Test's ladder
        # pins its rungs there — and when none of it does, a transmission without its header
        # granted nothing: a wait and a floor poll. An ordinary header is cheap, so it goes
        # (ADR-0076)
        if self.cfg.header_above_recommendation and not hdr_floor:
            recommendation = min(self._recommended, self._cap())
            elide = elide and not any(f.mode > recommendation for f in data)
        level = 0 if elide else self.level
        if not elide:
            self._possible.add(wanted)
        answer_floors = set(self._possible | {wanted})
        ans_s = max(self._ctl_s(f) for f in answer_floors)
        slave_floor = self.timing.is_floor(rung)
        if open_window:
            grant_s = self._window_s(hdr_floor, index)
        else:
            grant_s = ans_s + n * self._frame_s(slave_floor)
            if n:
                answer_floors.add(slave_floor)
        data_floor = bool(data) and self.timing.is_floor(data[0].mode)
        frames: list[TxFrame] = list(data)
        if elide:
            self.stats.headers_elided += 1
        else:
            ctl = GrantFrame(
                GrantKind.HEADER,
                self.session & 0x7F,
                count=len(data),
                data_floor=data_floor,
                level=level,
                fam=wanted,
                flag=open_window,
                measure=measuring,
                snr_db=self._heard_db,
                rung=rung,
                n=index if open_window else n,
                **self._ack_fields(),
            )
            frames.insert(0, self._control(ctl, hdr_floor))
            self._ack_owed = False
        self._key_granting(
            _Slot(
                kind="header",
                grant_s=grant_s,
                last_floor=data_floor if data else hdr_floor,
                answer_floors=frozenset(answer_floors),
                level=level,
                granter_turnaround_s=self.cfg.turnaround_s,
                open_window=open_window,
                data=bool(data) and not measuring,
                granted=n,
            ),
            frames,
        )

    def _send_disc(self) -> None:
        """DISC grants the DISC_ACK's slot with the slave's identifier time: its closing
        identifier goes right behind the DISC_ACK. The master's goes on its own once that slot
        is over (ADR-0075), the last thing either station sends for the session."""
        self._disc_tries += 1
        wanted = self._wants_floor() or self._silent >= 1
        floor = self._hdr_floor or self._silent >= 1 or self._floor_only()
        level = self.level
        ctl = GrantFrame(
            GrantKind.DISC,
            self.session & 0x7F,
            level=level,
            fam=wanted,
            ident=self._peer_identifier_s > 0,
            snr_db=self._heard_db,
        )
        self._possible.add(wanted)
        answer_floors = frozenset(self._possible | {wanted})
        self._key_granting(
            _Slot(
                kind="disc",
                grant_s=max(self._ctl_s(f) for f in answer_floors) + self._peer_identifier_s,
                last_floor=floor,
                answer_floors=answer_floors,
                level=level,
                granter_turnaround_s=self.cfg.turnaround_s,
            ),
            [self._control(ctl, floor)],
        )

    # ── the slave ─────────────────────────────────────────────────────

    def _placed(self, frame: SoftFrame, decoded: bool) -> bool:
        """Place a frame of the master's among its transmissions: ``False`` for one of a
        transmission this station has answered already, decoded late, and for one that cannot
        be the master's. A frame that begins at or after the end of the transmission being
        received starts the next one — once this station has answered that transmission, or
        when the frame decoded: the master keys only after the slot, so until then a frame
        nothing vouches for is no transmission of its, and changes nothing (ADR-0069 §2.5)."""
        if self._answered_end is not None and frame.t_end <= self._answered_end + 1e-6:
            return False
        if self._m_end is not None and frame.t_start >= self._m_end - 1e-3:
            owed = self._answered_end is None or abs(self._m_end - self._answered_end) > 1e-6
            if owed and not decoded:
                return False
            self._next_transmission()
        return True

    def _next_transmission(self) -> None:
        """The master transmission being received is over and another begins. What is left of
        the last was heard after its answer, or with no anchor to answer it by: nothing the
        rate controller can take as a transmission — false detections between transmissions,
        each counted a failed transmission of its own, held the master's rung on the floor
        through the review's phantom storm."""
        self._restart_rx()
        self._grant = None
        self._m_end = None
        self._window = None
        if self._answer is not None and self._answer.what in ("ans", "window"):
            self._answer = None
            self._disarm("answer")

    def _slave_control(self, ctl: GrantFrame, frame: SoftFrame) -> None:
        if not self._placed(frame, True):
            if ctl.kind is GrantKind.HEADER:
                self._take_ack(ctl, recommendation=False)
            return
        self._heard(frame)
        self._measure_lateness(frame)
        if ctl.snr_db is not None:
            self.peer_snr_db = ctl.snr_db
        if ctl.kind is GrantKind.DISC:
            self._end_session("peer disconnected", disc_heard=True)
            self._answer_disc(ctl, frame, GrantKind.DISC_ACK)
            return
        if ctl.kind is not GrantKind.HEADER:
            return
        self._open_rx(frame, ctl.count, ctl.data_floor)
        self._note_opening(ctl)
        self._take_ack(ctl)
        self._ans_sticky = ctl.fam or self._floor_only()
        self._grant = ctl
        self._grant_floor = frame.floor
        self._m_end = frame.t_end + ctl.count * self._frame_s(ctl.data_floor)
        self._m_last_floor = ctl.data_floor if ctl.count else frame.floor
        self._schedule_slave()

    def _slave_data(self, frame: SoftFrame, decoded: bool) -> None:
        if not self._placed(frame, decoded):
            if decoded:
                payload, _ = frame.decode(None)
                if payload is not None:
                    self._accept(_RxRecord(frame, 0), payload, late=True)
            return
        rec = self._take_data(frame)
        if rec.payload is not None:
            self._heard(frame)
            self._measure_lateness(frame)
            if (
                self._m_end is None
                and not rec.combined
                and frame.follows is not None
                and frame.follows < MAX_FOLLOWS
            ):
                # the header was not read: the transmission's end is the one this frame states —
                # if its own copy decoded (the note at _take_data)
                self._m_end = frame.t_end + frame.follows * (frame.t_end - frame.t_start)
                self._m_last_floor = self.timing.is_floor(frame.mode)
                self._schedule_slave()
        self._decide_if_final()

    def _measure_lateness(self, frame: SoftFrame) -> None:
        """How late this station's receiver reported a frame of the other's, and how late its
        keying is, against what the slot allows: the level its answers ask for. Up at once,
        down one level after :attr:`GrantConfig.need_ease_after` frames on time."""
        late = self.now - frame.t_end - self._budget(frame.floor)
        late += max(0.0, self._own_latency() - self.cfg.key_s)
        need = self._level_for(late)
        if need > self._need_level:
            self._need_level = need
            self._on_time = 0
        elif need < self._need_level:
            self._on_time += 1
            if self._on_time >= self.cfg.need_ease_after:
                self._need_level -= 1
                self._on_time = 0

    def _schedule_slave(self) -> None:
        """The slot the master transmission being received grants. A value the master can
        change is taken only from that transmission's own header; without it, the one that
        fits every slot the master may have granted — level 0, an answer alone, the family last
        asked for (ADR-0069 §2.3)."""
        if self._m_end is None or self.state is not State.CONNECTED:
            return
        if self._answered_end is not None and abs(self._m_end - self._answered_end) < 1e-6:
            return
        grant = self._grant
        level = grant.level if grant is not None else 0
        start = self._slot_start(self._m_end, self._m_last_floor, level, self._peer_turnaround_s)
        floor = self._ans_sticky
        if grant is not None and grant.flag:
            window = self._window_s(self._grant_floor, grant.n)
            self._window = (start, start + window, floor)
            self._answer = None
            self._disarm("answer")
            if self._has_work() or self._disc_requested:
                self._try_window()
            return
        self._window = None
        room = 0.0
        if grant is not None:
            room = grant.n * self._frame_s(self.timing.is_floor(grant.rung))
        self._schedule(
            _Answer(
                what="ans",
                start=start,
                not_after=start + self.cfg.slack_s,
                floor=floor,
                session=self.session,
                anchor=self._m_end,
                room_s=room,
            )
        )

    def _schedule(self, answer: _Answer) -> None:
        """Owe an answer in a slot — or forfeit it, decoded too late to start on time. Nothing
        is ever sent late."""
        if self.now > answer.start - self._own_latency() + self.cfg.slack_s + 1e-9:
            self.stats.forfeits += 1
            if answer.what == "ans":
                self._forfeit(answer.anchor)
            return
        self._answer = answer
        self._arm_at("answer", max(self.now, answer.start - self._own_latency()))
        self._decide_if_final()

    def _decide_if_final(self) -> None:
        """Decide an owed answer as soon as what it says is final, not at the last moment: it
        radiates at its slot's start all the same (``not_before``), and the daemon renders it
        meanwhile — four tone frames take up to 0.18 s to render, more under load, which the
        keying latency the slot allows has no room for (07-protocol-8.md §10). An answer to a
        call, a probe or a DISC is final at once; a slot's answer once every data frame its
        header stated has been reported, decoded or not. Without the header, or with a frame
        never reported, it is decided at the last moment, as it always was."""
        answer = self._answer
        if answer is None or answer.what == "window":
            return
        if answer.what == "ans" and not self._heard_all():
            return
        self._disarm("answer")
        self._key_answer()

    def _heard_all(self) -> bool:
        """Every data frame the header being answered stated has been reported."""
        if self._grid is None or self._grant is None:
            return False
        _, count, _ = self._grid
        slots = {r.slot for r in self._rx}
        return all(k in slots for k in range(count))

    def _forfeit(self, anchor: float) -> None:
        """A transmission of the master's this station will not answer: what it heard of it is
        still news of the path."""
        self._answered_end = anchor
        if self._rx:
            self._observe(self._rx)
        self._restart_rx()

    def _key_answer(self) -> None:
        answer, self._answer = self._answer, None
        if answer is None:
            return
        if answer.what == "window":
            self._try_window()
            return
        if self.now > answer.start - self._own_latency() + self.cfg.slack_s + 1e-9:
            self.stats.forfeits += 1  # the tick came too late
            if answer.what == "ans":
                self._forfeit(answer.anchor)
            return
        if answer.what == "connect_ack":
            self._send_acceptance(answer)
        elif answer.what == "probe_ack":
            self._send_probe_answer(answer)
        elif answer.what in ("disc", "disc_ack"):
            self._send_disc_answer(answer)
        elif answer.what == "ans" and self.state is State.CONNECTED and self.role is Role.SLAVE:
            self._answered_end = answer.anchor
            self._send_answer(
                answer.floor, answer.room_s, answer.not_after, not_before=answer.start
            )

    def _try_window(self) -> None:
        """Inside an open window: speak now if there is something to say, ending inside it."""
        if self._window is None or self.state is not State.CONNECTED:
            return
        start, end, floor = self._window
        on_air = self.now + self._own_latency()
        if on_air < start - 1e-9:
            self._answer = _Answer("window", start, end, floor)
            self._arm_at("answer", start - self._own_latency())
            return
        if not (self._has_work() or self._disc_requested):
            return
        room = end - on_air - self._ctl_s(floor)
        if room < 0:
            return
        self._window = None
        self._answered_end = self._m_end
        self._send_answer(floor, room, None, window_end=end)

    def _send_answer(
        self,
        floor: bool,
        room_s: float,
        not_after: float | None,
        *,
        window_end: float | None = None,
        not_before: float | None = None,
    ) -> None:
        snr = self._observe(self._rx) if self._rx else None
        self._restart_rx()
        if snr is not None:
            self._heard_db = snr
        if self.cfg.max_burst_s is not None:
            room_s = min(room_s, self.cfg.max_burst_s - self._ctl_s(floor))
        data = self._data_frames(room_s)
        ctl = GrantFrame(
            GrantKind.ANSWER,
            self.session & 0x7F,
            count=len(data),
            # with no data after it, the bit says where this station's data can go: only on
            # the floor, when its ceiling or its pinned rung is there
            data_floor=self.timing.is_floor(data[0].mode) if data else self._data_floor_only(),
            level=self._asked_level(),
            fam=self._wants_floor(),
            flag=self._disc_requested,
            measure=self._measure_out is not None,
            snr_db=self._heard_db,
            rung=self._recommend(),
            n=min(7, self._queued_frames()),
            **self._ack_fields(),
        )
        frames = [self._control(ctl, floor), *data]
        if window_end is not None:
            dur = sum(self.timing.frame_s(f) for f in frames)
            not_after = window_end - dur
        self._transmit(frames, not_after=not_after, not_before=not_before)

    def _data_floor_only(self) -> bool:
        """Whether this station's data can go only on the tone floor: its ceiling, or the rung
        it is pinned at, is a floor rung."""
        limit = self._cap() if self._pinned is None else min(self._pinned, self._cap())
        return self.timing.floor_modes > 0 and self.timing.is_floor(limit)

    def _queued_frames(self) -> int:
        cap = data_capacity(self.timing.capacity(self._burst_mode()))
        queued = len(self._unacked()) + math.ceil(len(self._tx_queue) / max(1, cap))
        if self._measure_ask is not None:
            queued += self._measure_ask[1]
        return queued

    def _send_acceptance(self, answer: _Answer) -> None:
        if self.session != answer.session or self.role is not Role.SLAVE:
            return
        body = ConnectBody(
            self.my_call,
            answer.remote,
            caps=with_level(self.cfg.capabilities, self._asked_level()),
            version=PROTOCOL_VERSION,
            snr_db=answer.snr_db,
            turnaround_s=self.cfg.turnaround_s,
            identifier_s=self._my_ident_s,
        ).encode()
        mode = self.robust_mode(answer.floor)
        payload = encode_data(
            DataHeader(DataKind.CONNECT_ACK, 0, self.session), body, self.timing.capacity(mode)
        )
        self._transmit(
            [TxFrame(Container.DATA, payload, mode=mode, rv=0)],
            not_after=answer.not_after,
            not_before=answer.start,
        )

    def _send_probe_answer(self, answer: _Answer) -> None:
        if self.state is not State.IDLE:
            return
        body = ProbeBody(
            self.my_call, answer.remote, answer.snr_db, caps=self.cfg.capabilities
        ).encode()
        mode = self.robust_mode(answer.floor)
        payload = encode_data(
            DataHeader(DataKind.PROBE_ACK, 0, 0), body, self.timing.capacity(mode)
        )
        self._transmit(
            [TxFrame(Container.DATA, payload, mode=mode, rv=0)],
            not_after=answer.not_after,
            not_before=answer.start,
        )

    def _send_disc_answer(self, answer: _Answer) -> None:
        """A DISC_ACK, and right behind it this station's closing identifier — the last thing it
        sends for the session, in the time every DISC grants it (07-protocol-8.md §5.4: the
        DISC_ACK's slot always includes the identifier the slave declared). A DISC heard again
        is answered again, identifier and all, so the identifier stays last (ADR-0075). Or a
        DISC, to a left session's header, after which the other station closes."""
        kind = GrantKind.DISC_ACK if answer.what == "disc_ack" else GrantKind.DISC
        ctl = GrantFrame(kind, answer.session & 0x7F, snr_db=answer.snr_db)
        ident_s = 0.0
        if kind is GrantKind.DISC_ACK and self._my_ident_s > 0.0:
            ident_s = self._my_ident_s
            self._id_owed = 0.0
            self._disarm("ident")
        if kind is GrantKind.DISC:
            self.stats.left_answered += 1
        self._transmit(
            [self._control(ctl, answer.floor)],
            not_after=answer.not_after,
            not_before=answer.start,
            identifier_s=ident_s,
        )
        if ident_s:
            self._hold_until = max(self._hold_until, self._closing_hold(answer.start))

    def _closing_hold(self, start: float) -> float:
        """The latest the other station's closing identifier can be on the air, after a DISC_ACK
        of this station's that answered in a slot starting at ``start``: the slot's latest end
        as that station may reckon it — its slack, a floor DISC_ACK, this station's identifier —
        the wait that follows, at the top gap level with room for a slow keying, and the other's
        declared identifier (ADR-0075). Nothing this station starts on its own keys before it:
        a call answered or made into the other's identifier would go over it."""
        cfg = self.cfg
        latest = start + cfg.slack_s + self._ctl_s(True) + self._my_ident_s
        wait = max(self._budget(True), cfg.turnaround_s) + cfg.guard_s + cfg.levels_s[-1]
        return latest + wait + cfg.key_s + cfg.slack_s + self._peer_identifier_s

    def _answer_disc(self, ctl: GrantFrame, frame: SoftFrame, kind: GrantKind) -> None:
        """A DISC_ACK to a DISC, or a DISC to a left session's header: in the slot the frame
        grants, or not at all."""
        end = frame.t_end + ctl.count * self._frame_s(ctl.data_floor)
        last_floor = ctl.data_floor if ctl.count else frame.floor
        start = self._slot_start(end, last_floor, ctl.level, self._peer_turnaround_s)
        self._schedule(
            _Answer(
                what="disc_ack" if kind is GrantKind.DISC_ACK else "disc",
                start=start,
                not_after=start + self.cfg.slack_s,
                floor=ctl.fam or self._floor_only(),
                session=ctl.session,
                anchor=end,
                snr_db=frame.snr_db,
            )
        )

    # ── sending data ──────────────────────────────────────────────────

    def _unacked(self) -> list[int]:
        seqs = [s for s, r in self._records.items() if not r.acked and r.sent]
        return sorted(seqs, key=lambda s: seq_distance(s, self._tx_base))

    def _data_frames(self, room_s: float) -> list[TxFrame]:
        """This station's data for a transmission, laid out as ``LinkEngine`` lays out a burst:
        the unacknowledged frames first — re-encoded when stranded (ADR-0038) — then new ones,
        one family a transmission, as many as ``room_s`` and :data:`MAX_DATA`
        allow. Each frame states how many follow it, three meaning three or more. A measurement
        asked for goes alone once nothing of this station's data is unacknowledged, and nothing
        goes while one waits for its report."""
        if room_s <= 0 or self._measure_out is not None:
            return []
        if self._measure_ask is not None and not self._unacked():
            return self._measurement(room_s)
        mode = self._burst_mode()
        recommendation = min(self._recommended, self._cap())
        # what an unheard answer would have said is not resent blind: it waits for an answer —
        # through one silent slot. Two in a row and the answers are not getting through, and
        # the frames go again rather than not at all
        unacked = self._unacked()
        if self._silent < 2:
            unacked = [s for s in unacked if not self._records[s].unknown]
        for s in unacked:
            rec = self._records[s]
            if self._hopeless(rec.mode):
                due = self.cfg.reencode_early_after
            else:
                due = self.cfg.max_combines
            if rec.tx_count >= due and recommendation < rec.mode:
                target = self._reencode_target(len(rec.body), rec.mode, recommendation)
                if target is not None:
                    rec.mode = target
                    rec.reencoded += rec.tx_count
                    rec.tx_count = 0
                    self.stats.frames_reencoded += 1
        family = self.timing.is_floor(mode)
        if unacked:
            family = self.timing.is_floor(self._records[unacked[0]].mode)
        seqs = [s for s in unacked if self.timing.is_floor(self._records[s].mode) == family]
        frame_s = self._frame_s(family)
        room = self.burst_capacity(frame_s)
        if math.isfinite(room_s):
            room = min(room, int(room_s / frame_s + 1e-9))
        if room <= 0:
            return []
        seqs = seqs[:room]
        # a measurement asked for waits only for what is in flight: nothing new goes before it
        new_frames = family == self.timing.is_floor(mode) and self._measure_ask is None
        cap = data_capacity(self.timing.capacity(mode))
        while (
            new_frames
            and len(seqs) < room
            and seq_distance(self._tx_next, self._tx_base) < WINDOW
            and self._tx_queue
        ):
            take = min(cap, len(self._tx_queue))
            if self._pin_body is not None:
                take = min(take, self._pin_body)
            if take == cap - 1:
                take -= 1  # the one length the container cannot carry
            body = bytes(self._tx_queue[:take])
            del self._tx_queue[:take]
            rec = _TxRecord(self._tx_next, body, mode)
            self._records[rec.seq] = rec
            self._tx_next = seq_after(self._tx_next)
            seqs.append(rec.seq)
        frames = []
        for i, s in enumerate(seqs):
            rec = self._records[s]
            if rec.sent:
                self.stats.frames_resent += 1
            rec.tx_count += 1
            payload = encode_data(
                DataHeader(DataKind.DATA, rec.seq, self.session),
                rec.body,
                self.timing.capacity(rec.mode),
            )
            rv = RV_SEQUENCE[(rec.tx_count - 1) % len(RV_SEQUENCE)]
            follows = min(MAX_FOLLOWS, len(seqs) - 1 - i)
            frames.append(TxFrame(Container.DATA, payload, mode=rec.mode, rv=rv, follows=follows))
        self.stats.frames_sent += len(frames)
        return frames

    def _measurement(self, room_s: float) -> list[TxFrame]:
        """The measurement asked for, as many of its frames as the transmission holds: filler
        at its rung, each stating how many follow, indexed in place of a sequence number."""
        assert self._measure_ask is not None
        mode, asked = self._measure_ask
        mode = min(mode, self._cap())
        frame_s = self._frame_s(self.timing.is_floor(mode))
        count = min(asked, MAX_DATA, self.burst_capacity(frame_s))
        if math.isfinite(room_s):
            count = min(count, int(room_s / frame_s + 1e-9))
        if count <= 0:
            return []
        self._measure_ask = None
        self._measure_out = (mode, count)
        self._measure_unheard = False
        capacity = self.timing.capacity(mode)
        body = data_capacity(capacity)
        frames = [
            TxFrame(
                Container.DATA,
                encode_data(
                    DataHeader(DataKind.MEASURE, i, self.session),
                    measure_body(self.session, i, body),
                    capacity,
                ),
                mode=mode,
                rv=0,
                follows=min(MAX_FOLLOWS, count - 1 - i),
            )
            for i in range(count)
        ]
        self.stats.frames_sent += count
        return frames

    def _take_ack(self, ctl: GrantFrame, *, recommendation: bool = True) -> None:
        """The other station's acknowledgement of this one's data, and its recommendation — or
        its report of this station's measurement (a header decoded late reports nothing: the
        next one says it again)."""
        if self._measure_out is not None and recommendation:
            if ctl.reports:
                mode, sent = self._measure_out
                self._measure_out = None
                decoded = sum(1 for i in range(sent) if ctl.measured(i))
                # after a silent slot the report is the poll's answer: its frames counted, its
                # SNR not the rung's — WC4Y's rung 8 went into the field log "at -4 dB", the
                # poll's tone-floor reading, where its frames had arrived at 0 dB
                # (13-wc4y-beta111.md §10)
                snr = None if self._measure_unheard else ctl.snr_db
                self._measure_unheard = False
                self.ladder.append(LadderRung(mode, sent, decoded, snr))
            else:
                self._measure_unheard = True
        progress = False
        for rec in self._records.values():
            rec.unknown = False  # the answer says what arrived
        for s, rec in list(self._records.items()):
            if not rec.acked and rec.sent and ctl.received(s):
                rec.acked = True
                progress = True
                self.top_acked_mode = max(self.top_acked_mode or 0, rec.mode)
                self.stats.bytes_acked += len(rec.body)
        while self._tx_base != self._tx_next and self._records[self._tx_base].acked:
            del self._records[self._tx_base]
            self._tx_base = seq_after(self._tx_base)
        if progress and self._patience_until is not None:
            self._patience_until = self.now + self._disc_patience()
        if recommendation:
            self._recommended = max(0, min(self._cap(), ctl.rung))

    # ── receiving data ────────────────────────────────────────────────

    # A frame's count rides in its chips, outside the codeword, so a CRC that passes vouches
    # for the payload and not for the count. A copy that decodes alone was read well enough for
    # its count too; a copy HARQ rescued was not — it is the earlier copies that decoded — and
    # on the scenario harness one read three frames to follow as none: the slave answered over
    # them (10-step-3.md §6). Such a count places no end: the slave keeps waiting for one it can
    # believe (a silent slot at worst), the master for the slot's latest end.

    def _take_data(self, frame: SoftFrame) -> _RxRecord:
        """A data frame of the transmission being received: decoded alone, or combined with an
        earlier copy of its block, its sequence number inferred from its slot (as
        ``LinkEngine``)."""
        payload, buffer = frame.decode(None)
        rec = _RxRecord(frame, 0)
        if payload is None and not self._on_grid(frame):
            self.stats.frames_failed += 1  # where no frame of the transmission can be
            return rec
        if self._rx_t0 is None:
            self._rx_t0 = frame.t_start
        frame_s = max(1e-6, frame.t_end - frame.t_start)
        rec.slot = max(0, round((frame.t_start - self._rx_t0) / frame_s))
        self._rx.append(rec)
        if payload is not None:
            if not self._accept(rec, payload):
                self._rx.remove(rec)
            return rec
        if self._rx_measure:
            self.stats.frames_failed += 1  # a measurement frame: never sent again
            return rec
        guess = self._infer_seq(rec.slot)
        rec.seq = guess
        if guess is None or not in_window(guess, self._rx_base):
            self.stats.frames_failed += 1
            return rec
        if guess in self._harq and self._harq[guess][2] != frame.mode:
            del self._harq[guess]  # re-encoded at another mode: start over
        if guess in self._harq:
            prev, combines, _ = self._harq[guess]
            rec.combined = True
            combined, merged = frame.decode(prev)
            if combined is not None:
                self.stats.harq_rescues += 1
                if not self._accept(rec, combined):
                    self._rx.remove(rec)
                return rec
            self._harq[guess] = (
                (buffer, 0, frame.mode)
                if combines + 1 >= self.cfg.max_combines
                else (merged, combines + 1, frame.mode)
            )
        else:
            self._harq[guess] = (buffer, 0, frame.mode)
        self.stats.frames_failed += 1
        return rec

    def _restart_rx(self) -> None:
        self._rx = []
        self._rx_t0 = None
        self._grid = None
        self._rx_measure = False

    def _note_opening(self, ctl: GrantFrame) -> None:
        """What the other station's opening control frame says of its own frames: measurement
        frames follow, or its last were and their report is owed; or data follows, and the
        report of its last measurement is over."""
        if ctl.measure:
            self._measurement_heard(new=ctl.count > 0)
        elif ctl.count:
            self._measured = None

    def _measurement_heard(self, *, new: bool) -> None:
        """The other station is measuring: ``new``, the transmission being received is its
        measurement; otherwise its last one was. Nothing of its data is in flight while it
        measures, so whatever this station holds to combine is of measurement frames — kept, the
        next data frame under the sequence number inferred for one would combine with another
        codeword — and goes."""
        if self._rx_measure:
            return
        if new:
            self._rx_measure = True
            self._measured = REPORT
        elif self._measured is None:
            self._measured = REPORT
        self._harq.clear()
        for rec in self._rx:
            if rec.payload is None:
                rec.seq = None

    def _open_rx(self, opener: SoftFrame, count: int, floor: bool) -> None:
        """A control frame read opens the other station's transmission and states it: ``count``
        data frames of ``floor``'s family, back to back from its end. Nothing heard before is of
        it, and from now on a frame that does not decode counts only where one of those frames
        can be."""
        self._rx_t0 = opener.t_end
        self._grid = (opener.t_end, count, floor)
        self._rx = [r for r in self._rx if self._on_grid(r.frame)]

    def _on_grid(self, frame: SoftFrame) -> bool:
        """Whether a frame sits where a data frame of the transmission being received can: in
        its family, on its slots, within its count — as its opening control frame stated, or,
        without it, in line with a frame of it that decoded. A false detection between the
        slots — 30 a minute in the review's phantom storm, a fifth of them trusted — is a
        failure of nothing: counted in, it stepped the rate down the floor (ADR-0020's rule is
        for frames that are there)."""
        floor = self.timing.is_floor(frame.mode)
        if self._grid is not None:
            t0, count, grid_floor = self._grid
            slot_s = self._frame_s(grid_floor)
            k = round((frame.t_start - t0) / slot_s)
            return (
                floor == grid_floor
                and 0 <= k < count
                and abs(frame.t_start - (t0 + k * slot_s)) <= GRID_TOLERANCE_S
            )
        ref = next((r.frame for r in self._rx if r.payload is not None), None)
        if ref is None:
            return True
        slot_s = max(1e-6, ref.t_end - ref.t_start)
        k = round((frame.t_start - ref.t_start) / slot_s)
        offset = abs(frame.t_start - (ref.t_start + k * slot_s))
        return floor == self.timing.is_floor(ref.mode) and offset <= GRID_TOLERANCE_S

    def _infer_seq(self, slot: int) -> int | None:
        anchors = [(r.slot, r.seq) for r in self._rx if r.payload is not None and r.seq is not None]
        snapshots = self._ack_history or [_AckSnapshot(self._rx_base, [], self._rx_base)]
        for snap in snapshots:
            expected = list(snap.missing)
            s = snap.next_new
            while len(expected) < MAX_BURST:
                expected.append(s)
                s = seq_after(s)
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

    def _accept(self, rec: _RxRecord, payload: bytes, *, late: bool = False) -> bool:
        """Take a decoded data frame of this session; ``False`` for anything else. ``late``: of
        a transmission already over — its data is kept, a measurement frame is of a report
        already made."""
        try:
            header, body = decode_data(payload)
        except ValueError:
            return False
        if header.session != self.session:
            return False
        if header.kind is DataKind.MEASURE:
            if late:
                return False
            self._measurement_heard(new=True)
            assert self._measured is not None
            if header.seq < WINDOW - 1:
                self._measured |= 2 << header.seq
            rec.payload = payload
            return True
        if header.kind is not DataKind.DATA:
            return False
        if not late:
            self._measured = None
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

    def _observe(self, records: list[_RxRecord]) -> float | None:
        """What a transmission's data said of the path, to the rate controller (ADR-0020's
        rules, as ``LinkEngine._observe_burst``). Returns the SNR measured on it."""
        ok = sum(1 for r in records if r.payload is not None)
        failed = sum(
            1
            for r in records
            if r.payload is None
            and r.frame.trusted
            and (r.frame.rv in SELF_DECODABLE_RVS or r.combined)
        )
        snrs = [r.frame.snr_db for r in records if r.payload is not None or r.frame.trusted]
        snr = sum(snrs) / len(snrs) if snrs else None
        modes = [r.frame.mode for r in records]
        if not modes:
            return snr
        if failed and self._asked is not None and min(modes) > self._asked:
            self.rate.observe_snr(snr)  # the sender's choice, not news of the path
        else:
            # the rung most of it went at; of two as common, the slower (stated, so the port
            # takes the same one)
            self.rate.observe(snr, ok, failed, max(sorted(set(modes)), key=modes.count))
        return snr

    def _receive_window(self) -> tuple[int, list[int], int]:
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

    def _ack_fields(self) -> _AckFields:
        """What a control frame says this station has received — remembered, for the slot
        inference of the copies sent after it."""
        bitmap, missing, next_new = self._receive_window()
        snap = _AckSnapshot(self._rx_base, missing, next_new)
        self._ack_history = [snap, *self._ack_history][:2]
        if self._measured is not None:
            bitmap = self._measured  # nothing of the other's data is beyond base
        return {"base": self._rx_base, "bitmap": bitmap}

    # ── the end ───────────────────────────────────────────────────────

    def _answer_left(self, ctl: GrantFrame, frame: SoftFrame) -> None:
        """An idle station hears a control frame of a session it left: a DISC is answered with
        its DISC_ACK, a header (the other station still in the session) with a DISC — in the
        frame's slot, or not at all (ADR-0052). A DISC_ACK ends the leaving."""
        if self._left is None:
            return
        session, until, disc_heard = self._left
        if self.now > until:
            self._left = None
            return
        if ctl.session != session & 0x7F:
            return
        if ctl.kind is GrantKind.DISC_ACK:
            self._left = None
            self._disarm("leave")
            slot = self._slot
            if slot is not None and slot.kind == "leave" and self._in_slot(frame):
                # an aborting master's DISC answered: the slave's identifier follows the
                # DISC_ACK, and this station's own goes after it (ADR-0075)
                self._state_end(frame.t_end + self._peer_identifier_s, frame.floor)
            self._identify_after_leaving()
            return
        if self.state is State.CONNECTING:
            return  # a call of this station's is out: its slot is the one that matters
        if self._id_owed > 0.0 and self._id_alone:
            # the other station is still there: its next words may close the session properly,
            # and this station's identifier is not sent over it on its own (ADR-0075)
            self._arm_at("ident", self.now + self._exchange_s())
        if ctl.kind is GrantKind.DISC:
            self._answer_disc(ctl, frame, GrantKind.DISC_ACK)
        elif ctl.kind is GrantKind.HEADER and not disc_heard:
            self._answer_disc(ctl, frame, GrantKind.DISC)

    def _repeat_leave(self) -> None:
        """An aborting master's DISC, from idle: on the floor, granting the DISC_ACK's slot, up
        to :attr:`GrantConfig.leave_repeats` times, each once the last one's slot is over."""
        if self._left is None or self.state is not State.IDLE or self._leave_repeats <= 0:
            self._identify_after_leaving()
            return
        if self.now < self._hold_until - 1e-9:
            self._arm_at("leave", self._hold_until)
            return
        session, until, _ = self._left
        if self.now > until:
            self._left = None
            self._identify_after_leaving()
            return
        self._leave_repeats -= 1
        ctl = GrantFrame(
            GrantKind.DISC,
            session & 0x7F,
            fam=True,
            level=self._silence_level,
            ident=self._peer_identifier_s > 0,
        )
        self._key_granting(
            _Slot(
                kind="leave",
                grant_s=self._ctl_s(True) + self._peer_identifier_s,
                last_floor=True,
                answer_floors=frozenset({True}),
                level=self._silence_level,
                granter_turnaround_s=self.cfg.turnaround_s,
            ),
            [self._control(ctl, True)],
        )
        if self._leave_repeats > 0:
            self._arm_at("leave", self._hold_until)
        else:
            self._identify_after_leaving()

    # ── the closing identifier (ADR-0075) ─────────────────────────────

    def _identify_after_leaving(self) -> None:
        """An aborting master's leaving is over — its DISC answered, or the last one's slot
        past: its closing identifier goes once that slot can no longer be in use."""
        if self._id_owed > 0.0 and not self._id_alone and self.state is State.IDLE:
            self._arm_at("ident", max(self.now, self._hold_until))

    def _send_identifier(self) -> None:
        """The closing identifier on its own: a master's, once the last slot it granted is
        over, or a station's that no slot could be had for, listening first. Nothing of the
        session follows it."""
        length = self._id_owed
        if length <= 0.0:
            return
        if self.state is not State.IDLE and not (
            self.state is State.CONNECTING and self._connect_tries == 0
        ):
            return  # a session is under way: it ends with an identifier of its own
        if self.now < self._hold_until - 1e-9:
            self._arm_at("ident", self._hold_until)
            return
        self._id_owed = 0.0
        end = self._transmit([], identifier_s=length, listen_first=self._id_listen)
        # a call or a probe of this station's, and an answer to another's, waits for it
        self._hold_until = max(self._hold_until, end)

    def _drop_identifier(self) -> None:
        """No identifier for the session just ended: the communication goes on in a new one."""
        self._id_owed = 0.0
        self._disarm("ident")

    def _owe_again(self, ident: float) -> None:
        """A DISC_ACK that carried the closing identifier did not go out: the identifier is owed
        again — behind the next DISC_ACK, or on its own if no DISC comes (ADR-0075)."""
        if ident <= 0.0 or self.state is not State.IDLE or self._id_owed > 0.0:
            return
        self._id_owed = ident
        self._id_alone = True
        self._id_listen = True
        self._arm_at("ident", self.now + self._exchange_s())

    def _end_session(self, reason: str, *, disc_heard: bool = False, unread: bool = False) -> None:
        left = None
        if self.state is not State.CONNECTING and (reason in LEFT_UNHEARD or disc_heard):
            left = (self.session, self.now + self._link_timeout(), disc_heard)
        # a session that came up ends with this station's closing identifier — unless the other
        # station ended it by calling again, which goes on in the session its call opens
        # (ADR-0055 §4); a call never answered is the daemon's to identify (ADR-0055)
        owes = (
            self.state is not State.CONNECTING
            and self._my_ident_s > 0.0
            and not reason.endswith(" called again")
        )
        was_master = self.role is Role.MASTER
        self.ended_peer_snr_db = self.peer_snr_db
        kept = (self._peer_turnaround_s, self._peer_identifier_s, self._silence_level)
        self.state = State.IDLE
        self.role = Role.NONE
        self._disarm("connect")
        self._disarm("ident")
        self._reset_session()
        self._tx_queue = bytearray()
        # what answering or leaving the old session in its slots still needs
        self._peer_turnaround_s, self._peer_identifier_s, self._silence_level = kept
        self._left = left
        self._id_owed = self._my_ident_s if owes else 0.0
        self.actions.append(Event("disconnected", reason))
        if not owes:
            return
        # where the closing identifier goes (ADR-0075): the master's once the last slot it
        # granted can no longer be in use — an aborting master's once its leaving is over; a
        # slave's behind its DISC_ACK, or on its own, listening first, when nothing of the
        # session is heard for an exchange — a live master keys at least that often
        self._id_alone = not was_master
        # a master that read nothing of the other station at the end listens before its own: the
        # other may have given the session up first and be sending its identifier alone
        self._id_listen = self._id_alone or unread or reason in ("no response", "link timeout")
        if was_master and reason != "aborted":
            self._arm_at("ident", max(self.now, self._hold_until))
        elif not was_master and reason == "link timeout":
            # a slave that timed out cannot hear the master, so listening first cannot keep
            # their identifiers apart: it waits until the master has given up and identified.
            # The master gives up no later than its own link timeout after the last answer it
            # can have read — this station's last slot, inside the exchange after the frame that
            # last armed this station's timer — and identifies then (I1; the band-closes
            # scenario's two identifiers met 0.6 s apart)
            self._arm_at("ident", self.now + self._exchange_s() + self._peer_identifier_s)
        elif not was_master:
            self._arm_at("ident", self.now if left is None else self.now + self._exchange_s())
