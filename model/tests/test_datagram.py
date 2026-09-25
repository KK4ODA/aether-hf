"""Datagrams (ADR-0019): another program's frame, split into DATA frames and joined again."""

from __future__ import annotations

import pytest

from aether_model.link.datagram import (
    HEADER_BYTES,
    MAX_FRAGMENTS,
    Fragment,
    Reassembler,
    body,
    fragments,
    max_payload,
    parse_body,
    read_fragment,
    split,
)
from aether_model.link.frames import DATA_HEADER, DataKind, decode_data


def _payload(n: int) -> bytes:
    return bytes((i * 131 + 7) % 256 for i in range(n))


@pytest.mark.parametrize("capacity", [24, 26, 51, 144, 732])
def test_any_size_the_rung_can_carry_goes_and_comes_back_whole(capacity: int) -> None:
    full = capacity - DATA_HEADER
    sizes = {
        1,
        2,
        full - 2,
        full - 1,
        full,
        full + 1,
        2 * full - 1,
        2 * full,
        max_payload(capacity),
    }
    for size in sorted(s for s in sizes if 0 < s <= max_payload(capacity)):
        payload = _payload(size)
        frames = fragments(payload, 42, capacity)
        assert 1 <= len(frames) <= MAX_FRAGMENTS
        assert all(len(f) == capacity for f in frames), "every fragment is a whole frame"
        joiner = Reassembler()
        results = [joiner.add(read_fragment(f), 0.0) for f in frames]  # type: ignore[arg-type]
        assert results[:-1] == [None] * (len(frames) - 1)
        assert results[-1] == payload, size


def test_a_remainder_one_byte_short_of_a_full_frame_goes_as_two() -> None:
    # a partial frame needs two bytes for its length, so full − 1 bytes fit neither way
    capacity = 24
    full = capacity - DATA_HEADER
    pieces = split(_payload(full - 1), capacity)
    assert [len(p) for p in pieces] == [full - 2, 1]
    assert [len(p) for p in split(_payload(full), capacity)] == [full]
    assert [len(p) for p in split(_payload(full - 2), capacity)] == [full - 2]


def test_the_header_numbers_the_pieces() -> None:
    frames = fragments(_payload(100), 7, 24)
    for index, frame in enumerate(frames):
        header, _ = decode_data(frame)
        assert header.kind is DataKind.DATAGRAM
        assert header.session == 7
        assert header.seq >> 4 == index
        assert header.seq & 0x0F == len(frames) - 1


def test_what_does_not_fit_is_refused_not_cut() -> None:
    with pytest.raises(ValueError, match="at most 16"):
        fragments(_payload(max_payload(24) + 1), 1, 24)
    with pytest.raises(ValueError):
        fragments(b"", 1, 24)
    with pytest.raises(ValueError):
        fragments(b"x", 0, 24)
    # sixteen fragments at the tone floor hold an AX.25 frame with eight digipeaters
    assert max_payload(24) >= 330


def test_pieces_are_joined_in_any_order_once_each() -> None:
    payload = _payload(90)
    frames = fragments(payload, 9, 24)
    pieces = [read_fragment(f) for f in frames]
    joiner = Reassembler()
    order = [pieces[i] for i in (3, 0, 3, 4, 1, 0)] + [pieces[2]]
    results = [joiner.add(p, 1.0) for p in order]  # type: ignore[arg-type]
    assert results[-1] == payload
    assert all(r is None for r in results[:-1])
    assert joiner.waiting == 0


def test_a_datagram_missing_a_piece_is_dropped_in_time() -> None:
    frames = fragments(_payload(60), 3, 24)
    joiner = Reassembler(timeout_s=10.0)
    for frame in frames[:-1]:
        assert joiner.add(read_fragment(frame), 0.0) is None  # type: ignore[arg-type]
    joiner.expire(11.0)
    assert joiner.waiting == 0
    assert joiner.dropped == 1
    # and the same number, sent again later, is a new datagram
    assert joiner.add(read_fragment(frames[-1]), 12.0) is None  # type: ignore[arg-type]


def test_two_datagrams_at_once_do_not_mix() -> None:
    a, b = _payload(70), _payload(75)[::-1]
    fa, fb = fragments(a, 1, 24), fragments(b, 2, 24)
    joiner = Reassembler()
    out = []
    for x, y in zip(fa, fb, strict=False):
        out += [joiner.add(read_fragment(x), 0.0), joiner.add(read_fragment(y), 0.0)]  # type: ignore[arg-type]
    for rest in fa[len(fb) :] + fb[len(fa) :]:
        out.append(joiner.add(read_fragment(rest), 0.0))  # type: ignore[arg-type]
    assert sorted(o for o in out if o) == sorted([a, b])


def test_only_datagram_frames_are_read_as_fragments() -> None:
    from aether_model.link.frames import DataHeader, encode_data

    beacon = encode_data(DataHeader(kind=DataKind.BEACON, seq=0, session=0), b"x", 24)
    assert read_fragment(beacon) is None
    assert read_fragment(b"\x00") is None
    # an index past the last index is not a fragment
    bad = encode_data(DataHeader(kind=DataKind.DATAGRAM, seq=0x21, session=5), b"x", 24)
    assert read_fragment(bad) is None
    assert read_fragment(fragments(b"y", 5, 24)[0]) == Fragment(5, 0, 0, b"y")


def test_a_crowd_of_incomplete_datagrams_is_bounded() -> None:
    joiner = Reassembler(keep=4)
    for number in range(1, 20):
        first = fragments(_payload(60), number, 24)[0]
        joiner.add(read_fragment(first), float(number))  # type: ignore[arg-type]
    assert joiner.waiting == 4
    assert joiner.dropped == 15


def test_a_datagram_names_its_sender_and_keeps_the_frame_type() -> None:
    frame = bytes(range(40))
    for frame_type in (0, 1, 2):
        carried = body("KK4ODA-1", frame_type, frame)
        assert len(carried) == HEADER_BYTES + len(frame)
        assert parse_body(carried) == ("KK4ODA-1", frame_type, frame)
    with pytest.raises(ValueError):
        body("KK4ODA", 3, frame)
    # nothing after the header, a type this version does not read, no callsign: not a datagram
    assert parse_body(body("KK4ODA", 0, b"")) is None
    assert parse_body(bytes(8) + b"x") is None
    bad_type = bytearray(body("KK4ODA", 0, b"x"))
    bad_type[7] = 9
    assert parse_body(bytes(bad_type)) is None
