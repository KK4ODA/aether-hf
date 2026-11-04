"""Link layer: the frame formats and the usable rungs. The rate controller's tests are
``test_rate.py``, the engine's ``test_granted.py``; the bit-exact-over-the-real-PHY check lives
in ``test_link_harness.py``."""

from __future__ import annotations

import math

import pytest

from aether_model.frame.modes import LONG, MODES, WIDE
from aether_model.link.frames import (
    CONNECT_BODY_8_BYTES,
    CONNECT_BODY_BYTES,
    DATA_HEADER,
    FRAGMENT_HEADER,
    FRAGMENT_MAX_OFFSET,
    ConnectBody,
    DataFlags,
    DataHeader,
    DataKind,
    ProbeBody,
    decode_data,
    encode_data,
    pack_callsign,
    pieces,
    unpack_callsign,
)
from aether_model.link.rate import AWGN_THRESHOLD_DB, usable_modes

# ── frame formats ─────────────────────────────────────────────────────


@pytest.mark.parametrize("call", ["W4ODA", "KK4XYZ", "M0ABC", "VK2DEF/P", "A1", "9Z9ZZZ9ZZ"])
def test_callsign_round_trip(call: str) -> None:
    assert unpack_callsign(pack_callsign(call)) == call
    assert len(pack_callsign(call)) == 7


def test_callsign_rejects_bad_input() -> None:
    with pytest.raises(ValueError):
        pack_callsign("")
    with pytest.raises(ValueError):
        pack_callsign("TOOLONGCALL")
    with pytest.raises(ValueError):
        pack_callsign("W4ODA!")


@pytest.mark.parametrize("mode_idx", [0, 4, 13])
def test_data_frame_round_trip_full_and_partial(mode_idx: int) -> None:
    cap = MODES[mode_idx].payload_bytes(LONG)
    for body_len in (0, 5, cap - 3, cap - 5):
        if body_len < 0:
            continue
        body = bytes(range(256))[:body_len] if body_len <= 256 else bytes(body_len)
        header = DataHeader(DataKind.DATA, seq=200, session=42)
        frame = encode_data(header, body, cap)
        assert len(frame) == cap
        got_header, got_body = decode_data(frame)
        assert got_header == header
        assert got_body == body


def test_data_frame_is_identical_across_retransmissions() -> None:
    """HARQ combines identical codewords: the encoded bytes must not depend on anything
    outside (header, body, capacity)."""
    cap = MODES[4].payload_bytes(LONG)
    body = bytes(range(100))
    h = DataHeader(DataKind.DATA, seq=7, session=3)
    assert encode_data(h, body, cap) == encode_data(h, body, cap)


@pytest.mark.parametrize("cap", [24, 25, 51, 137, 732])
def test_a_body_in_pieces_fits_its_frames_and_makes_the_body_again(cap: int) -> None:
    """A stuck frame's body goes in pieces at a rung it does not fit whole (ADR-0086). Every
    piece fits its frame, the one length the container cannot carry included, the offsets run
    on from piece to piece, and the last one says so."""
    room = cap - DATA_HEADER - FRAGMENT_HEADER
    for body_len in range(1, 3 * room + 3):
        body = bytes((7 * i + 1) % 256 for i in range(body_len))
        parts = pieces(body, cap)
        got = bytearray()
        for n, (offset, piece, last) in enumerate(parts):
            assert offset == len(got)
            assert piece
            assert last == (n == len(parts) - 1)
            header = DataHeader(DataKind.DATA, 9, 42, DataFlags.FRAGMENT, offset=offset, last=last)
            frame = encode_data(header, piece, cap)
            assert len(frame) == cap
            assert decode_data(frame) == (header, piece)
            got += piece
        assert bytes(got) == body
        assert len(parts) <= math.ceil(body_len / room) + 1


def test_a_piece_header_is_twelve_bits_of_offset_and_a_last_flag() -> None:
    header = DataHeader(
        DataKind.DATA, 3, 4, DataFlags.FRAGMENT, offset=FRAGMENT_MAX_OFFSET, last=True
    )
    frame = encode_data(header, b"xyz", 24)
    assert frame[:5] == bytes([0x03, 3, 4, 0xFF, 0xF8])  # DATA, FRAGMENT | PARTIAL
    assert decode_data(frame) == (header, b"xyz")
    with pytest.raises(ValueError):
        DataHeader(DataKind.DATA, 3, 4, DataFlags.FRAGMENT, offset=FRAGMENT_MAX_OFFSET + 1)
    # a frame sent whole carries neither
    whole = DataHeader(DataKind.DATA, 3, 4)
    assert decode_data(encode_data(whole, b"xyz", 24)) == (whole, b"xyz")
    assert not whole.fragment


def test_connect_body_round_trip() -> None:
    body = ConnectBody("W4ODA", "KK4XYZ", caps=0b101, version=1)
    assert ConnectBody.decode(body.encode()) == body


def test_a_connect_body_carries_the_snr_the_request_arrived_at() -> None:
    # the acceptance says how the request was heard (P9-2); a request says nothing, and a
    # body from an earlier version, which stops at the version byte, reads as nothing
    ack = ConnectBody("KK4XYZ", "W4ODA", caps=0b010, snr_db=12.5)
    assert len(ack.encode()) == CONNECT_BODY_8_BYTES == 19  # protocol 8 on: two bytes more
    old_ack = ConnectBody("KK4XYZ", "W4ODA", caps=0b010, version=7, snr_db=-7.0)
    assert len(old_ack.encode()) == CONNECT_BODY_BYTES == 17
    assert ConnectBody.decode(old_ack.encode()) == old_ack  # read, to be refused and said so
    assert ConnectBody.decode(ack.encode()).snr_db == 12.0  # whole dB, ties to even
    req = ConnectBody("W4ODA", "KK4XYZ")
    assert ConnectBody.decode(req.encode()).snr_db is None
    old = req.encode()[:16]
    assert ConnectBody.decode(old) == req


def test_probe_body_round_trips_and_clamps_its_snr() -> None:
    probe = ProbeBody("W4ODA", "KK4XYZ", None, caps=0b10)
    assert ProbeBody.decode(probe.encode()) == probe
    assert len(probe.encode()) == 16
    for snr, expect in [(-7.4, -7.0), (12.5, 12.0), (13.5, 14.0), (200.0, 40.0), (-200.0, -40.0)]:
        ack = ProbeBody("KK4XYZ", "W4ODA", snr, caps=0)
        assert ProbeBody.decode(ack.encode()).snr_db == expect, snr
    # a whole-decibel byte, 0x7F meaning "not measured", as the control frame's SNR byte
    assert ProbeBody.decode(b"\0" * 14 + bytes([0x7F, 0])).snr_db is None


# ── rate controller ───────────────────────────────────────────────────


def test_usable_modes_are_pareto_and_sorted() -> None:
    modes = usable_modes()
    assert modes == sorted(modes)
    # 8-PSK 2/3 is dominated by 16-QAM 1/2: more payload, lower threshold
    assert WIDE.rung_of(7) not in modes and WIDE.rung_of(8) in modes
    for m in modes:
        assert not any(
            AWGN_THRESHOLD_DB[o] <= AWGN_THRESHOLD_DB[m] and o != m and o in modes
            for o in modes
            if o > m
        )
