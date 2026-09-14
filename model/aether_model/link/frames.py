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


@dataclass(frozen=True)
class ConnectBody:
    """Body of CONNECT_REQ / CONNECT_ACK: who is calling whom, and what they can do."""

    src: str
    dst: str
    caps: int = 0
    """Capability bits (reserved: compression, bandwidth options)."""
    version: int = 1

    def encode(self) -> bytes:
        return pack_callsign(self.src) + pack_callsign(self.dst) + bytes([self.caps, self.version])

    @classmethod
    def decode(cls, body: bytes) -> ConnectBody:
        if len(body) < 2 * CALL_BYTES + 2:
            raise ValueError("short connect body")
        return cls(
            src=unpack_callsign(body[:CALL_BYTES]),
            dst=unpack_callsign(body[CALL_BYTES : 2 * CALL_BYTES]),
            caps=body[2 * CALL_BYTES],
            version=body[2 * CALL_BYTES + 1],
        )


CONNECT_BODY_BYTES = 2 * CALL_BYTES + 2


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
