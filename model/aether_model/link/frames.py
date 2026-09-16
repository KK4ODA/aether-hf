"""Link-layer frame formats — the bytes inside a PHY DATA or CONTROL payload (P2-1).

Two containers exist at the PHY: a DATA frame (LONG layout, mode signalled in the pilot
chips, 26 … 732 payload bytes) and a CONTROL frame (SHORT layout, control mode, 7 bytes).
The link layer puts its own header at the front of each:

DATA container, 3-byte header (5 with an explicit length)::

    0: kind (3 bits) | flags (5 bits)
    1: seq                                         8-bit sequence number
    2: session id
    3–4: data length (only when flags & PARTIAL)   otherwise the frame is full

Nothing in the header may change between transmissions of the same sequence number: a
retransmission is the *same codeword* under another redundancy version, and the receiver
soft-combines them. That is why the header carries no burst position — the receiver takes
the slot of a frame from its air time (frames of a burst are contiguous) and the end of a
burst from the silence after it. Kinds carried in the DATA container: DATA (user bytes),
CONNECT_REQ and CONNECT_ACK (callsigns do not fit a control frame).

CONTROL container, 7 bytes::

    0: kind (4 bits) | flags (4 bits)
    1: session id
    2: base seq      ACK: next sequence number the receiver needs
    3–4: bitmap      ACK: bit i set ⇔ seq base + i has been received (16-frame window)
    5: SNR           ACK/POLL: signed dB, 3 kHz reference (−40 … +40), 0x7f = unknown
    6: recommended mode (4 bits) | counter (4 bits, wraps; distinguishes repeated ACKs)

Kinds: ACK, POLL (ISS keep-alive, answered by an ACK), TURN (ISS hands the sending role
to the peer), DISC / DISC_ACK. Everything is big-endian; nothing here depends on the PHY.

Callsigns are packed 6 bits per character (``A–Z 0–9 - /``), 9 characters in 7 bytes.
"""

from __future__ import annotations

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


def with_bandwidth(caps: int, bandwidth_hz: int) -> int:
    """A capability byte with the bandwidth bits set for ``bandwidth_hz``."""
    code = BANDWIDTH_CODES[bandwidth_hz]
    return (caps & ~CAP_BANDWIDTH_MASK & 0xFF) | (code << CAP_BANDWIDTH_SHIFT)


@dataclass(frozen=True)
class ConnectBody:
    """Body of CONNECT_REQ / CONNECT_ACK: who is calling whom, and what they can do."""

    src: str
    dst: str
    caps: int = 0
    """Capability bits: compression (bit 0) and the bandwidth this frame was sent in
    (bits 1–2, :func:`bandwidth_code`)."""
    version: int = 1
    snr_db: float | None = None
    """In an acceptance: the SNR (3 kHz) the request arrived at, whole decibels, in the
    CONTROL frame's byte — what the caller starts its first burst from (P9-2). Absent
    in a request, and from a station of an earlier version, whose body stops at the
    version byte; a receiver takes a short body as "not measured"."""

    def encode(self) -> bytes:
        snr = SNR_UNKNOWN if self.snr_db is None else max(-40, min(40, round(self.snr_db)))
        return (
            pack_callsign(self.src)
            + pack_callsign(self.dst)
            + bytes([self.caps, self.version, snr & 0xFF])
        )

    @classmethod
    def decode(cls, body: bytes) -> ConnectBody:
        if len(body) < 2 * CALL_BYTES + 2:
            raise ValueError("short connect body")
        raw = body[2 * CALL_BYTES + 2] if len(body) > 2 * CALL_BYTES + 2 else SNR_UNKNOWN
        snr = None if raw == SNR_UNKNOWN else float(raw - 256 if raw >= 128 else raw)
        return cls(
            src=unpack_callsign(body[:CALL_BYTES]),
            dst=unpack_callsign(body[CALL_BYTES : 2 * CALL_BYTES]),
            caps=body[2 * CALL_BYTES],
            version=body[2 * CALL_BYTES + 1],
            snr_db=snr,
        )


CONNECT_BODY_BYTES = 2 * CALL_BYTES + 3


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
    counter: int = 0

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
                ((self.recommended_mode & 0x0F) << 4) | (self.counter & 0x0F),
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
            recommended_mode=payload[6] >> 4,
            counter=payload[6] & 0x0F,
        )

    def received(self, seq: int) -> bool:
        """ACK semantics: has ``seq`` been received (below base, or set in the bitmap)?"""
        d = (seq - self.base) % SEQ_MOD
        if d >= WINDOW:
            return d >= SEQ_MOD // 2  # far behind base → already acknowledged
        return bool((self.bitmap >> d) & 1)


def seq_after(seq: int, n: int = 1) -> int:
    return (seq + n) % SEQ_MOD


def seq_distance(seq: int, base: int) -> int:
    """Forward distance from ``base`` to ``seq`` (0 … 255)."""
    return (seq - base) % SEQ_MOD


def in_window(seq: int, base: int, width: int = WINDOW) -> bool:
    return seq_distance(seq, base) < width
