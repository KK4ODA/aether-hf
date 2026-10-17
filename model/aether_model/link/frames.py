"""Link-layer frame formats — the bytes inside a PHY DATA or CONTROL payload (P2-1).

Two containers exist at the PHY: a DATA frame (a rung of the air's ladder — a tone-floor
kind, or an OFDM mode on the LONG layout with the mode signalled in the pilot chips — 15 … 732
payload bytes) and a CONTROL frame (7 bytes: the tone floor's control frame, or the SHORT
layout at the control mode). The link layer puts its own header at the front of each:

DATA container, 3-byte header (5 with an explicit length)::

    0: kind (3 bits) | flags (5 bits)
    1: seq                                         8-bit sequence number
    2: session id
    3–4: data length (only when flags & PARTIAL)   otherwise the frame is full

Nothing in the header may change between transmissions of the same codeword: a
retransmission is the *same codeword* under another redundancy version, and the receiver
soft-combines them. That is why the header carries no burst position — the receiver takes
the slot of a frame from its air time (frames of a burst are contiguous) and the end of a
burst from the silence after it. Kinds carried in the DATA container: DATA (user bytes),
MEASURE (a rung measured, never delivered: protocol 9), CONNECT_REQ and CONNECT_ACK (callsigns
do not fit a control frame), and — outside sessions — BEACON, PROBE, PROBE_ACK and DATAGRAM.

CONTROL container, 7 bytes::

    0: kind (4 bits) | flags (4 bits)
    1: session id
    2: base seq      ACK: next sequence number the receiver needs
    3–4: bitmap      ACK: bit i set ⇔ seq base + i has been received (16-frame window)
    5: SNR           signed dB, 3 kHz reference (−40 … +40), 0x7f = unknown: in an ACK the
                     burst's (ADR-0020), in any other kind the last frame decoded from the
                     other station (ADR-0021)
    6: recommended mode (5 bits) | burst family (1 bit) | follows (2 bits): how many frames
       of a burst follow this control frame in the same transmission, in slots of that
       burst's data frame, and the burst's family (1 = the tone floor) — an acknowledgement
       that takes the turn, its own burst after it (ADR-0047, ADR-0068); 0 otherwise. Until
       protocol 8 the three bits were a counter of acknowledgements, a label for logs.

Kinds: ACK, POLL (ISS keep-alive, answered by an ACK), TURN (ISS hands the sending role
to the peer), DISC / DISC_ACK. Everything is big-endian; nothing here depends on the PHY.

Callsigns are packed 6 bits per character (``A–Z 0–9 - /``), 9 characters in 7 bytes.
"""

from __future__ import annotations

import math
import struct
from dataclasses import dataclass
from enum import Enum, IntFlag

WINDOW = 16
"""Selective-repeat window: ACK bitmap width and the most frames in flight per burst."""
SEQ_MOD = 256
MAX_BURST = 16
CALL_ALPHABET = "\0ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-/"
CALL_MAX_CHARS = 9
CALL_BYTES = 7
DATA_HEADER = 3
CONTROL_BYTES = 7
SNR_UNKNOWN = 0x7F


class DataKind(Enum):
    DATA = 0
    CONNECT_REQ = 1
    CONNECT_ACK = 2
    BEACON = 3
    """Unproto: sent outside any session, addressed to nobody, carrying this station's
    callsign. It is how an operator answers "can anybody hear me?" without arranging a
    contact first, which on HF is most of what a new station needs to know."""
    PROBE = 4
    """A beacon with a destination (P7-1): "can *you* hear me, and how well?" — sent outside
    any session, answered with a PROBE_ACK carrying the SNR the answering station measured
    on it, so both operators see both directions of the path without arranging a contact.
    The one thing a receiver cannot measure is how it is heard; this is how it asks."""
    PROBE_ACK = 5
    """The answer to a PROBE: the probed station's callsign, the prober's, and the SNR the
    probe arrived at. Answering is a *response* in the sense of §97.221(c), so a station
    that may only answer may answer this too; sending a probe is a call, and may not."""
    DATAGRAM = 6
    """A fragment of another program's frame — an AX.25 frame a KISS client handed the modem —
    sent outside any session, with no acknowledgement, to be handed to the KISS clients of
    every station that decodes it (ADR-0019). ``seq`` is the fragment's index in its high
    nibble and the last index in its low one; ``session`` is the datagram's number (1–255).
    See ``datagram.py``."""
    MEASURE = 7
    """A frame that measures a rung of the path (protocol 9, ADR-0077): filler at the rung
    measured, sent once in a transmission of its own and never delivered or sent again. Its
    ``seq`` is its index among the transmission's measurement frames, not a sequence number:
    it takes none, so a frame of it that does not arrive leaves nothing to deliver. The other
    station reports which arrived (:data:`REPORT`). The Test's ladder is built of them; it had
    pinned the session's own data at each rung, and on WC4Y's Test of 2026-10-17 delivering the
    rungs' failed frames took 93 s of 435 (13-wc4y-beta111.md §10)."""


class DataFlags(IntFlag):
    NONE = 0
    PARTIAL = 0x01
    """An explicit data length follows the header (the frame is not full)."""


class ControlKind(Enum):
    ACK = 0
    POLL = 1
    TURN = 2
    DISC = 3
    DISC_ACK = 4


class ControlFlags(IntFlag):
    NONE = 0
    WANT_TX = 0x1
    """(ACK) the receiving station has data to send."""
    BREAK = 0x2
    """(ACK) the receiving station demands the sending role now."""
    OFFER = 0x4
    """(TURN) sent at the end of a burst that emptied the sender's queue: the turn is the
    receiving station's for the taking, with its acknowledgement (ADR-0047)."""
    TAKEN = 0x8
    """(ACK) the turn offered is taken: this station's own data follows the acknowledgement in
    the same transmission (ADR-0047)."""


# ── callsigns ─────────────────────────────────────────────────────────


def pack_callsign(call: str) -> bytes:
    call = call.upper().strip()
    if not 1 <= len(call) <= CALL_MAX_CHARS:
        raise ValueError(f"callsign must be 1 … {CALL_MAX_CHARS} characters")
    value = 0
    for ch in call.ljust(CALL_MAX_CHARS, "\0"):
        if ch not in CALL_ALPHABET:
            raise ValueError(f"character {ch!r} not allowed in a callsign")
        value = (value << 6) | CALL_ALPHABET.index(ch)
    return (value << 2).to_bytes(CALL_BYTES, "big")


def unpack_callsign(raw: bytes) -> str:
    value = int.from_bytes(raw[:CALL_BYTES], "big") >> 2
    chars = []
    for i in range(CALL_MAX_CHARS):
        code = (value >> (6 * (CALL_MAX_CHARS - 1 - i))) & 0x3F
        if code >= len(CALL_ALPHABET):
            raise ValueError("corrupt callsign")
        chars.append(CALL_ALPHABET[code])
    return "".join(chars).rstrip("\0")


# ── DATA container ────────────────────────────────────────────────────


@dataclass(frozen=True)
class DataHeader:
    kind: DataKind
    seq: int
    session: int
    flags: DataFlags = DataFlags.NONE

    def __post_init__(self) -> None:
        if not 0 <= self.seq < SEQ_MOD or not 0 <= self.session < 256:
            raise ValueError("seq and session are 8-bit")


def encode_data(header: DataHeader, body: bytes, capacity: int) -> bytes:
    """Header + body padded to ``capacity`` (the PHY payload size for the mode)."""
    partial = len(body) < capacity - DATA_HEADER
    flags = header.flags | (DataFlags.PARTIAL if partial else DataFlags.NONE)
    out = bytes([(header.kind.value << 5) | int(flags), header.seq, header.session])
    if partial:
        if len(body) > capacity - DATA_HEADER - 2:
            raise ValueError("body does not fit with an explicit length")
        out += struct.pack(">H", len(body))
    out += body
    if len(out) > capacity:
        raise ValueError(f"frame body too long for capacity {capacity}")
    return out + bytes(capacity - len(out))


def decode_data(payload: bytes) -> tuple[DataHeader, bytes]:
    if len(payload) < DATA_HEADER:
        raise ValueError("short DATA payload")
    kind = DataKind(payload[0] >> 5)
    flags = DataFlags(payload[0] & 0x1F)
    header = DataHeader(
        kind=kind, seq=payload[1], session=payload[2], flags=flags & ~DataFlags.PARTIAL
    )
    if flags & DataFlags.PARTIAL:
        (length,) = struct.unpack(">H", payload[3:5])
        if length > len(payload) - 5:
            raise ValueError("corrupt DATA length")
        return header, payload[5 : 5 + length]
    return header, payload[DATA_HEADER:]


def data_capacity(phy_payload_bytes: int) -> int:
    """User bytes per full DATA frame for a PHY payload of the given size."""
    return phy_payload_bytes - DATA_HEADER


CAP_COMPRESSION = 0x01
"""Capability bit 0: stream compression (deflate) — offered, and used only if both offer."""
CAP_BANDWIDTH_SHIFT = 1
CAP_BANDWIDTH_MASK = 0x03 << CAP_BANDWIDTH_SHIFT
"""Capability bits 1–2: the bandwidth of the waveform the frame was sent in. Not a
negotiation — a receiver knows the waveform from having decoded the frame — but a
statement, so a station can refuse a request that claims a bandwidth other than the one
it arrived in, and a station listening in more than one answers in the one it was called
in. A session lives its whole life in one bandwidth."""
BANDWIDTH_CODES: dict[int, int] = {2300: 0, 500: 1, 2750: 2}
"""Bandwidth in hertz → the code in the capability byte (3 is reserved)."""


def bandwidth_code(caps: int) -> int:
    """The bandwidth code stated in a capability byte."""
    return (caps & CAP_BANDWIDTH_MASK) >> CAP_BANDWIDTH_SHIFT


def bandwidth_hz_of(caps: int) -> int | None:
    """The bandwidth in hertz a capability byte states; ``None`` for the reserved code."""
    code = bandwidth_code(caps)
    return next((hz for hz, c in BANDWIDTH_CODES.items() if c == code), None)


def with_bandwidth(caps: int, bandwidth_hz: int) -> int:
    """A capability byte with the bandwidth bits set for ``bandwidth_hz``."""
    code = BANDWIDTH_CODES[bandwidth_hz]
    return (caps & ~CAP_BANDWIDTH_MASK & 0xFF) | (code << CAP_BANDWIDTH_SHIFT)


MAX_FOLLOWS = 3
"""The largest countdown a DATA frame carries (ADR-0041): the PHY turns the frame's mode chips
a quarter turn a step, so four values."""


def countdown_of(following: int) -> int:
    """The countdown a DATA frame carries for the frames of its burst that follow it: half of
    them, rounded up (ADR-0046) — 0 for the last frame, 1 for one or two following, 2 for three
    or four, 3 for five or six. It never says fewer than follow, so a receiver that hears only
    the burst's first frames does not answer over the rest, as "three or more" let it; it says
    at most one more, and only the last frame's loss makes that matter. Exact for bursts up to
    seven frames; :attr:`LinkConfig.burst_frames` is six."""
    return min(MAX_FOLLOWS, (following + 1) // 2)


def most_following(countdown: int) -> int:
    """The most frames a countdown says may follow (:func:`countdown_of`)."""
    return 2 * countdown


TURNS_VERSION = 7
"""Link protocol 7, the turn engine's (:class:`~aether_model.link.engine.LinkEngine`), in the
connect body's version byte — kept until granted slots replace that engine (ADR-0069 Step 5).
2 since the tone floor (ADR-0013): a mode number is a rung of the air's ladder — on the
2 300 Hz air two above the OFDM mode of version 1 — so a session between the two would run on
numbers that mean different frames at either end; a station ignores a call or an acceptance
of another version, and says so. 3 since the fast kinds (ADR-0014): four more rungs on the
2 300 Hz air, between the floor's two and the OFDM modes, and a control frame whose recommended
mode has five bits and its counter three. 4 since the narrow middle kinds (ADR-0015): two more
rungs on the 500 Hz air, between the floor's two and the OFDM modes (the 2 300 Hz ladder is
as it was, but one number says what both ladders are). 5 since the burst countdown
(ADR-0041): an ordinary DATA frame's mode chips are turned by the number of frames of its
burst that follow it, which a receiver of version 4 cannot read — it takes a turned frame for
noise. 6 since the countdown counts in pairs (ADR-0046): a 5 reading 1 where 6 says 1 or 2
would answer a frame early. 7 since the turn offered at the end of a burst (ADR-0047): a TURN's
OFFER flag and an acknowledgement's TAKEN, which a station of version 6 would take for a TURN to
answer and an acknowledgement alone."""


GRANTED_VERSION = 8
"""The first link protocol of granted slots (ADR-0069): the caller is the session's master and
the only station that keys on its own initiative; every master transmission grants the other
station one slot, defined by what it states; a station keys only in a slot granted by a frame it
decoded in time. Its control frame (:class:`GrantFrame`) and its connect body's two more bytes
are not version 7's, and neither is any of its timing."""

PROTOCOL_VERSION = 9
"""The link protocol a station speaks, in the connect body's version byte; what a release
states (``link_protocol``); a station ignores a call of another, and says so. 9 since the
measurement frames (ADR-0077): a MEASURE frame, the control frame's ``measure`` bit in the place
of version 8's identifier request inside a session, and an acknowledgement that reports a
measurement (:data:`REPORT`), none of which a station of version 8 reads."""

CAP_LEVEL_SHIFT = 3
CAP_LEVEL_MASK = 0x03 << CAP_LEVEL_SHIFT
"""Capability bits 3–4, in a request (a call, a probe) of protocol 8: the gap level of the slot
its answer goes in (ADR-0069). A caller raises it every second try, so a called station whose
computer decodes late is reached; an answer carries 0."""


def with_level(caps: int, level: int) -> int:
    """A capability byte with the answer slot's gap level set (bits 3–4)."""
    return (caps & ~CAP_LEVEL_MASK & 0xFF) | ((level & 0x03) << CAP_LEVEL_SHIFT)


def level_of(caps: int) -> int:
    """The answer slot's gap level a request's capability byte states."""
    return (caps & CAP_LEVEL_MASK) >> CAP_LEVEL_SHIFT


@dataclass(frozen=True)
class ConnectBody:
    """Body of CONNECT_REQ / CONNECT_ACK: who is calling whom, and what they can do."""

    src: str
    dst: str
    caps: int = 0
    """Capability bits: compression (bit 0) and the bandwidth this frame was sent in
    (bits 1–2, :func:`bandwidth_code`); in a request of protocol 8, the gap level of the
    acceptance's slot (bits 3–4, :func:`level_of`)."""
    version: int = TURNS_VERSION
    snr_db: float | None = None
    """In an acceptance: the SNR (3 kHz) the request arrived at, whole decibels, in the
    CONTROL frame's byte — what the caller starts its first burst from (P9-2). Absent
    in a request, and from a station of an earlier version, whose body stops at the
    version byte; a receiver takes a short body as "not measured"."""
    turnaround_s: float = 0.0
    """Protocol 8: how long the sender's radio takes to hear again after it stops sending —
    its VOX hold and receiver recovery — in 0.1 s steps up to 1.5 s. The other station's slot
    starts no sooner after this station's transmission, and its transmissions no sooner after
    this station's slot (ADR-0069). Absent before protocol 8."""
    identifier_s: float = 0.0
    """Protocol 8: how long the sender's Morse identifier lasts, in 0.5 s steps (0 for none):
    a master grants a slot that time longer when the slave's identifier is due. Absent before
    protocol 8."""

    def encode(self) -> bytes:
        snr = SNR_UNKNOWN if self.snr_db is None else max(-40, min(40, round(self.snr_db)))
        out = (
            pack_callsign(self.src)
            + pack_callsign(self.dst)
            + bytes([self.caps, self.version, snr & 0xFF])
        )
        if self.version >= GRANTED_VERSION:
            turnaround = max(0, min(15, math.ceil(self.turnaround_s * 10 - 1e-9)))
            identifier = max(0, min(255, math.ceil(self.identifier_s * 2 - 1e-9)))
            out += bytes([turnaround, identifier])
        return out

    @classmethod
    def decode(cls, body: bytes) -> ConnectBody:
        if len(body) < 2 * CALL_BYTES + 2:
            raise ValueError("short connect body")
        raw = body[2 * CALL_BYTES + 2] if len(body) > 2 * CALL_BYTES + 2 else SNR_UNKNOWN
        snr = None if raw == SNR_UNKNOWN else float(raw - 256 if raw >= 128 else raw)
        version = body[2 * CALL_BYTES + 1]
        turnaround = identifier = 0.0
        if version >= GRANTED_VERSION and len(body) >= CONNECT_BODY_8_BYTES:
            turnaround = (body[2 * CALL_BYTES + 3] & 0x0F) / 10.0
            identifier = body[2 * CALL_BYTES + 4] / 2.0
        return cls(
            src=unpack_callsign(body[:CALL_BYTES]),
            dst=unpack_callsign(body[CALL_BYTES : 2 * CALL_BYTES]),
            caps=body[2 * CALL_BYTES],
            version=version,
            snr_db=snr,
            turnaround_s=turnaround,
            identifier_s=identifier,
        )


CONNECT_BODY_BYTES = 2 * CALL_BYTES + 3
CONNECT_BODY_8_BYTES = CONNECT_BODY_BYTES + 2
"""Protocol 8's connect body: the turnaround and the identifier's length after the SNR. With
a DATA header and an explicit length it fills the 24 bytes of the tone floor's slowest rung."""


@dataclass(frozen=True)
class ProbeBody:
    """Body of PROBE / PROBE_ACK: who is asking whom, the SNR the answer reports, and the
    capability bits (the bandwidth the frame was sent in, so a probe in another bandwidth
    than it arrived in is ignored like a connect request would be)."""

    src: str
    dst: str
    snr_db: float | None = None
    """PROBE_ACK: the SNR (3 kHz) the probe arrived at, as the control frame carries it —
    whole decibels, ties to even, clamped to −40 … +40; PROBE: absent."""
    caps: int = 0

    def encode(self) -> bytes:
        snr = SNR_UNKNOWN if self.snr_db is None else max(-40, min(40, round(self.snr_db)))
        return (
            pack_callsign(self.src)
            + pack_callsign(self.dst)
            + (snr & 0xFF).to_bytes(1, "big")
            + bytes([self.caps])
        )

    @classmethod
    def decode(cls, body: bytes) -> ProbeBody:
        if len(body) < PROBE_BODY_BYTES:
            raise ValueError("short probe body")
        raw = body[2 * CALL_BYTES]
        snr = None if raw == SNR_UNKNOWN else float(raw - 256 if raw >= 128 else raw)
        return cls(
            src=unpack_callsign(body[:CALL_BYTES]),
            dst=unpack_callsign(body[CALL_BYTES : 2 * CALL_BYTES]),
            snr_db=snr,
            caps=body[2 * CALL_BYTES + 1],
        )


PROBE_BODY_BYTES = 2 * CALL_BYTES + 2


# ── CONTROL container ─────────────────────────────────────────────────


@dataclass(frozen=True)
class ControlFrame:
    kind: ControlKind
    session: int
    flags: ControlFlags = ControlFlags.NONE
    base: int = 0
    bitmap: int = 0
    snr_db: float | None = None
    recommended_mode: int = 0
    """A rung of the air's ladder: five bits, room for 32 — the 2 300 Hz ladder has 20 since
    the fast kinds (ADR-0014), past the four bits of protocol version 2."""
    follows: int = 0
    """How many frames of a burst follow this control frame in the same transmission, in
    slots of that burst's data frame (0–3, ADR-0068): an acknowledgement that took the turn
    states its own burst after it (ADR-0047), so a receiver that reads it and nothing of the
    burst still knows when the burst ends."""
    burst_floor: bool = False
    """The family of the burst :attr:`follows` counts — the length of its slots."""

    def encode(self) -> bytes:
        snr = SNR_UNKNOWN if self.snr_db is None else max(-40, min(40, round(self.snr_db)))
        return bytes(
            [
                (self.kind.value << 4) | int(self.flags),
                self.session,
                self.base,
                (self.bitmap >> 8) & 0xFF,
                self.bitmap & 0xFF,
                snr & 0xFF,
                ((self.recommended_mode & 0x1F) << 3)
                | (int(self.burst_floor) << 2)
                | (self.follows & 0x03),
            ]
        )

    @classmethod
    def decode(cls, payload: bytes) -> ControlFrame:
        if len(payload) < CONTROL_BYTES:
            raise ValueError("short CONTROL payload")
        raw_snr = payload[5]
        if raw_snr == SNR_UNKNOWN:
            snr: float | None = None
        else:
            snr = float(raw_snr - 256 if raw_snr >= 128 else raw_snr)
        return cls(
            kind=ControlKind(payload[0] >> 4),
            session=payload[1],
            flags=ControlFlags(payload[0] & 0x0F),
            base=payload[2],
            bitmap=(payload[3] << 8) | payload[4],
            snr_db=snr,
            recommended_mode=payload[6] >> 3,
            burst_floor=bool(payload[6] & 0x04),
            follows=payload[6] & 0x03,
        )

    def received(self, seq: int) -> bool:
        """ACK semantics: has ``seq`` been received (below base, or set in the bitmap)?"""
        d = (seq - self.base) % SEQ_MOD
        if d >= WINDOW:
            return d >= SEQ_MOD // 2  # far behind base → already acknowledged
        return bool((self.bitmap >> d) & 1)


# ── protocol 8: the CONTROL container ──────────────────────────────────


class GrantKind(Enum):
    HEADER = 0
    """The master's: what this transmission carries, what it acknowledges, and the slot it
    grants the slave."""
    ANSWER = 1
    """The slave's, in its slot: what it acknowledges, what follows, what it has queued."""
    DISC = 2
    """Either's: the session ends. A master's grants the DISC_ACK's slot; a slave's goes in a
    slot it was granted, the last it uses."""
    DISC_ACK = 3


GRANT_SNR_UNKNOWN = 0x20
"""The six-bit SNR field's "not measured": −32 in two's complement, outside −31 … +31."""

REPORT = 0x0001
"""An acknowledgement's bitmap bit 0, set: the bitmap reports the other station's last
measurement — bit ``i + 1`` set ⇔ its measurement frame ``i`` arrived — and acknowledges nothing
at or above ``base`` (protocol 9, ADR-0077). In a plain acknowledgement the bit is always clear:
``base`` is the first sequence number not received."""


@dataclass(frozen=True)
class GrantFrame:
    """Protocol 8's control frame (ADR-0069): seven bytes, 56 bits::

        0: kind (2) | count (3) | data family (1) | level (2)
        1: session (7) | fam (1)
        2: flag (1) | id / measure (1) | SNR (6)
        3: base       4–5: bitmap         (selective acknowledgement, as version 7's)
        6: rung (5) | n (3)

    ``count``: data frames after this one in its transmission, exactly (0–6), and their family
    (``data_floor``) — with it a receiver that reads this frame and none of them still knows
    where the transmission ends. ``level``: in a header, the gap level of the slot it grants;
    in an answer, the level its sender needs. ``fam``: in a header, the family the answer goes
    in; in an answer, the family its sender needs headers in. ``flag``: the header's slot is an
    open window; the answer asks to leave. ``ident`` (a DISC, a DISC_ACK): the slot includes
    the slave's closing Morse identifier (ADR-0075). ``measure`` (a header, an answer; the same
    bit, protocol 9): the data frames after this one are measurement frames (:class:`DataKind`
    ``MEASURE``), and with none after it, the sender's last were and it has not read their
    report — it sends no data until it has (ADR-0077). ``base``/``bitmap``: what the sender has
    received of the other station's data, or with :data:`REPORT` set, which of its last
    measurement's frames arrived. ``snr_db``: how the sender hears the other
    station, −31 … +31 dB (ADR-0021). ``rung``: recommended for the other station's data.
    ``n``: frames granted (or an open window's size step) / frames queued, at most 7.
    ``session`` is the session number's low seven bits: data frames carry all eight."""

    kind: GrantKind
    session: int
    count: int = 0
    data_floor: bool = False
    level: int = 0
    fam: bool = False
    flag: bool = False
    ident: bool = False
    measure: bool = False
    base: int = 0
    bitmap: int = 0
    snr_db: float | None = None
    rung: int = 0
    n: int = 0

    def encode(self) -> bytes:
        snr = (
            GRANT_SNR_UNKNOWN
            if self.snr_db is None
            else max(-31, min(31, round(self.snr_db))) & 0x3F
        )
        return bytes(
            [
                (self.kind.value << 6)
                | (min(self.count, 7) << 3)
                | (int(self.data_floor) << 2)
                | (self.level & 0x03),
                ((self.session & 0x7F) << 1) | int(self.fam),
                (int(self.flag) << 7) | (int(self.ident or self.measure) << 6) | snr,
                self.base & 0xFF,
                (self.bitmap >> 8) & 0xFF,
                self.bitmap & 0xFF,
                ((self.rung & 0x1F) << 3) | (min(self.n, 7) & 0x07),
            ]
        )

    @classmethod
    def decode(cls, payload: bytes) -> GrantFrame:
        if len(payload) < CONTROL_BYTES:
            raise ValueError("short CONTROL payload")
        raw = payload[2] & 0x3F
        snr = None if raw == GRANT_SNR_UNKNOWN else float(raw - 64 if raw >= 32 else raw)
        kind = GrantKind(payload[0] >> 6)
        bit = bool(payload[2] & 0x40)
        closing = kind in (GrantKind.DISC, GrantKind.DISC_ACK)
        return cls(
            kind=kind,
            count=(payload[0] >> 3) & 0x07,
            data_floor=bool(payload[0] & 0x04),
            level=payload[0] & 0x03,
            session=payload[1] >> 1,
            fam=bool(payload[1] & 0x01),
            flag=bool(payload[2] & 0x80),
            ident=bit and closing,
            measure=bit and not closing,
            snr_db=snr,
            base=payload[3],
            bitmap=(payload[4] << 8) | payload[5],
            rung=payload[6] >> 3,
            n=payload[6] & 0x07,
        )

    def received(self, seq: int) -> bool:
        """Acknowledgement semantics, as :meth:`ControlFrame.received`; a report acknowledges
        only what is below ``base``."""
        d = (seq - self.base) % SEQ_MOD
        if d >= WINDOW:
            return d >= SEQ_MOD // 2
        return not self.reports and bool((self.bitmap >> d) & 1)

    @property
    def reports(self) -> bool:
        """Whether the bitmap reports the other station's last measurement (:data:`REPORT`)."""
        return bool(self.bitmap & REPORT)

    def measured(self, index: int) -> bool:
        """In a report: whether the measurement's frame ``index`` arrived."""
        return self.reports and 0 <= index < WINDOW - 1 and bool((self.bitmap >> (index + 1)) & 1)


def seq_after(seq: int, n: int = 1) -> int:
    return (seq + n) % SEQ_MOD


def seq_distance(seq: int, base: int) -> int:
    """Forward distance from ``base`` to ``seq`` (0 … 255)."""
    return (seq - base) % SEQ_MOD


def in_window(seq: int, base: int, width: int = WINDOW) -> bool:
    return seq_distance(seq, base) < width
