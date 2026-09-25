"""Datagrams: another program's frame, carried outside any session (ADR-0019).

A KISS client — APRS software, a packet node — hands the modem whole AX.25 frames and
expects them on the air as they are, one transmission each, no acknowledgement: the client
does any repeating its own protocol needs. Aether carries such a frame as a *datagram*: split
into DATA frames of kind ``DATAGRAM`` at whichever rung the station sends datagrams at, all
in one burst, and joined again by every station that decodes the burst, which hands the
frame to its own KISS clients. Nothing here reads the frame: it is bytes.

The DATA header's two free bytes number the pieces. ``seq`` holds the fragment's index in its
high nibble and the last index in its low one, so a datagram is at most sixteen fragments;
``session`` holds the datagram's number, 1–255, which a station advances for every datagram
it sends, so pieces of two datagrams are never joined. Every fragment but the last is a full
frame; the last is partial, with the explicit length the DATA container gives a short frame
— and when the remainder is one byte too long for that (a full frame minus one), it goes as
two fragments rather than as a frame the container cannot encode.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from aether_model.link.frames import (
    CALL_BYTES,
    DATA_HEADER,
    DataHeader,
    DataKind,
    decode_data,
    encode_data,
    pack_callsign,
    unpack_callsign,
)

MAX_FRAGMENTS = 16
"""The index and the last index share one byte, four bits each."""

FRAME_TYPES = (0, 1, 2)
"""The frame types a datagram carries, as a VARA-style KISS client names them in the byte after
``FEND``: 0 an AX.25 frame, 1 an AX.25 frame whose address fields are eight bytes (seven
characters and the SSID byte — VarAC's broadcasts), 2 unformatted data. The receiving station
hands the frame on with the type it came with (ADR-0019)."""

HEADER_BYTES = CALL_BYTES + 1
"""What a datagram carries before the client's frame: the sender's callsign and the type."""


def body(source: str, frame_type: int, frame: bytes) -> bytes:
    """What a datagram carries: the sending station's callsign — so every datagram identifies
    its station in the emission itself, whatever the client's frame holds — the frame's type,
    and the frame."""
    if frame_type not in FRAME_TYPES:
        raise ValueError(f"frame type {frame_type} is not one a datagram carries")
    return pack_callsign(source) + bytes([frame_type]) + frame


def parse_body(payload: bytes) -> tuple[str, int, bytes] | None:
    """The sender, the frame type and the frame a joined datagram carries; None when it is not
    one this version reads."""
    if len(payload) <= HEADER_BYTES or payload[CALL_BYTES] not in FRAME_TYPES:
        return None
    try:
        source = unpack_callsign(payload[:CALL_BYTES])
    except ValueError:
        return None
    if not source:
        return None
    return source, payload[CALL_BYTES], payload[HEADER_BYTES:]


PARTIAL_LENGTH = 2
"""The explicit length a partial DATA frame carries after its header."""


def max_payload(capacity: int) -> int:
    """The longest datagram a rung whose frames carry ``capacity`` bytes can send."""
    return MAX_FRAGMENTS * (capacity - DATA_HEADER)


def split(payload: bytes, capacity: int) -> list[bytes]:
    """The pieces a payload goes as, at a rung whose frames carry ``capacity`` bytes.

    Full frames first; the last piece short enough to carry its length, or split in two when
    it is exactly one byte too long for that.
    """
    full = capacity - DATA_HEADER
    if full <= PARTIAL_LENGTH:
        raise ValueError(f"a frame of {capacity} bytes carries no datagram")
    pieces = []
    rest = payload
    while len(rest) > full:
        pieces.append(rest[:full])
        rest = rest[full:]
    if len(rest) == full - 1:
        # too long to go partial (it needs two length bytes) and one short of full
        pieces += [rest[: full - PARTIAL_LENGTH], rest[full - PARTIAL_LENGTH :]]
    else:
        pieces.append(rest)
    return pieces


def fragments(payload: bytes, number: int, capacity: int) -> list[bytes]:
    """The DATA frames (``capacity`` bytes each) that carry ``payload`` as datagram ``number``.

    Raises ``ValueError`` when it needs more than sixteen, or the number is not 1–255.
    """
    if not 1 <= number <= 255:
        raise ValueError("a datagram's number is 1–255")
    if not payload:
        raise ValueError("an empty datagram carries nothing")
    pieces = split(payload, capacity)
    if len(pieces) > MAX_FRAGMENTS:
        raise ValueError(
            f"{len(payload)} bytes need {len(pieces)} fragments at {capacity} bytes a frame; "
            f"a datagram is at most {MAX_FRAGMENTS} ({max_payload(capacity)} bytes)"
        )
    last = len(pieces) - 1
    return [
        encode_data(
            DataHeader(kind=DataKind.DATAGRAM, seq=(index << 4) | last, session=number),
            piece,
            capacity,
        )
        for index, piece in enumerate(pieces)
    ]


@dataclass(frozen=True)
class Fragment:
    """One decoded piece: which datagram, where in it, and its bytes."""

    number: int
    index: int
    last: int
    body: bytes


def read_fragment(payload: bytes) -> Fragment | None:
    """The fragment a decoded DATA frame carries, or None when it is not a datagram's."""
    try:
        header, body = decode_data(payload)
    except ValueError:
        return None
    if header.kind is not DataKind.DATAGRAM or header.session == 0:
        return None
    index, last = header.seq >> 4, header.seq & 0x0F
    if index > last:
        return None
    return Fragment(number=header.session, index=index, last=last, body=body)


@dataclass
class _Partial:
    last: int
    pieces: dict[int, bytes] = field(default_factory=dict)
    first_s: float = 0.0


class Reassembler:
    """Joins the fragments of datagrams as they are decoded.

    A datagram is handed on once every piece has arrived; one that stays incomplete longer
    than ``timeout_s`` — a piece lost, since nothing is retransmitted — is dropped, and so
    are the oldest when more than ``keep`` are waiting. A piece seen twice counts once.
    """

    def __init__(self, timeout_s: float = 120.0, keep: int = 8) -> None:
        self.timeout_s = timeout_s
        self.keep = keep
        self._waiting: dict[tuple[int, int], _Partial] = {}
        self.dropped = 0

    def add(self, fragment: Fragment, now_s: float) -> bytes | None:
        """Take a piece; return the whole datagram when this piece completes it."""
        self.expire(now_s)
        key = (fragment.number, fragment.last)
        partial = self._waiting.get(key)
        if partial is None:
            if len(self._waiting) >= self.keep:
                oldest = min(self._waiting, key=lambda k: self._waiting[k].first_s)
                del self._waiting[oldest]
                self.dropped += 1
            partial = _Partial(last=fragment.last, first_s=now_s)
            self._waiting[key] = partial
        partial.pieces.setdefault(fragment.index, fragment.body)
        if len(partial.pieces) <= partial.last:
            return None
        del self._waiting[key]
        return b"".join(partial.pieces[i] for i in range(partial.last + 1))

    def expire(self, now_s: float) -> None:
        """Drop what has waited too long for a piece that is not coming."""
        stale = [k for k, p in self._waiting.items() if now_s - p.first_s > self.timeout_s]
        for key in stale:
            del self._waiting[key]
            self.dropped += 1

    @property
    def waiting(self) -> int:
        return len(self._waiting)
