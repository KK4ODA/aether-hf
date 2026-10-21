"""Link layer: the frame formats and the rate controller. The engine's tests are
``test_granted.py``; the bit-exact-over-the-real-PHY check lives in ``test_link_harness.py``."""

from __future__ import annotations

import math
from itertools import pairwise

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
from aether_model.link.rate import AWGN_THRESHOLD_DB, RateController, usable_modes

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


def test_the_first_mode_keeps_a_step_in_hand() -> None:
    rc = RateController()
    # far below every mode but the slowest: the slowest
    assert rc.first_mode(-20.0) == usable_modes()[0]
    # steps below the fastest that fits with margin and hysteresis — within the family it is
    # in: a fit on the OFDM rungs does not start on the floor
    ordinary = next(i for i, m in enumerate(rc.modes) if m >= rc.floor_modes)
    for snr in (4.0, 9.0, 15.0, 20.0):
        modes = rc.modes
        fits = [
            m for m in modes if AWGN_THRESHOLD_DB[m] + rc.margin_db + rc.up_hysteresis_db <= snr
        ]
        top = modes.index(fits[-1])
        lowest = ordinary if top >= ordinary else 0
        assert rc.first_mode(snr) == modes[max(lowest, top - rc.first_mode_back)], snr
    # seeding places the controller there and takes the measurement, once
    rc.seed(15.0)
    assert rc.recommend() == rc.first_mode(15.0)
    assert rc.snr_db == 15.0
    rc.seed(2.0)
    assert rc.snr_db == 15.0  # a second seed changes nothing


def test_on_the_floor_the_steps_in_hand_are_decibels() -> None:
    """WC4Y's Test of 2026-10-11 (80 m, 500 Hz; ADR-0066): the call was heard at about −2 dB,
    which the floor's top rung (then tone4x100-75, −13 dB) cleared by ten decibels and the
    first OFDM rung did not fit; two rungs in hand sent the message at tone-36, half the rate,
    on a path that carried tone4x100-75 in every burst after. On the floor the steps keep the
    decibels they would at the fit's own edge; among the OFDM rungs they stay rungs. The
    floor's top is tone4x100-93 since ADR-0081, and the same call still starts on it."""
    from aether_model.frame.modes import NARROW
    from aether_model.link.rate import (
        NARROW_AWGN_THRESHOLD_DB,
        NARROW_FRAME_S,
        NARROW_PAYLOAD_BYTES,
    )

    thresholds = dict(NARROW_AWGN_THRESHOLD_DB)
    narrow = RateController(
        thresholds=thresholds,
        modes=usable_modes(thresholds, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S),
        floor_modes=NARROW.floor_modes,
    )
    top = NARROW.floor_modes - 1

    def required(rung: int) -> float:
        return thresholds[rung] + narrow.margin_db + narrow.up_hysteresis_db

    assert required(top) <= -2.0 < required(top + 1)  # the floor's top fits, the OFDM rung not
    assert narrow.first_mode(-2.0) == top
    keep = required(top) - required(top - 2)  # what two rungs in hand keep at the top's edge
    assert narrow.first_mode(required(top) + keep) == top
    assert narrow.first_mode(required(top - 1) + keep) == top - 1
    assert narrow.first_mode(required(top - 1) + keep - 0.01) == top - 2
    assert narrow.first_mode(required(top)) == top - 2  # at the fit's edge: two rungs, as before
    assert narrow.first_mode(-20.0) == 0
    # the OFDM rungs: the steps are rungs (ADR-0008)
    ordinary = narrow._first_ordinary()
    for snr in (3.0, 6.0, 9.0, 14.0):
        modes = narrow.modes
        top = max(i for i in range(len(modes)) if required(modes[i]) <= snr)
        assert narrow.first_mode(snr) == modes[max(ordinary, top - narrow.first_mode_back)]

    wide = RateController()  # the wide ladder's floor top is rung 5, the fast kinds' fastest
    assert wide.first_mode(-3.5) == 5
    assert wide.first_mode(-6.7) == 3  # at its edge, two rungs in hand


def test_a_seed_from_the_tone_floor_is_a_lower_bound() -> None:
    """ADR-0016: calls start on the tone floor, whose SNR estimate reads low on a strong path
    (it saturates near +17 dB on AWGN and reads a few decibels on a dispersive path). A
    controller seeded from it seeds again from the first clean burst measured on an ordinary
    frame — upward, past the estimates' own spread, and once; a failed burst or a floor
    burst does not."""
    rc = RateController()
    ordinary = rc.modes[rc._first_ordinary()]
    rc.seed(4.0, lower_bound=True)
    assert rc.recommend() == rc.first_mode(4.0)
    rc.observe(20.0, ok=6, failed=0, mode=0)  # a floor burst: a lower bound again
    assert rc.snr_db == pytest.approx(0.7 * 4.0 + 0.3 * 20.0)
    rc.observe(20.0, ok=3, failed=3, mode=ordinary)  # a failure says nothing of the path
    assert rc.recommend() < rc.first_mode(20.0)
    rc.observe(20.0, ok=6, failed=0, mode=ordinary)
    assert rc.snr_db == 20.0
    assert rc.recommend() == rc.first_mode(20.0)
    rc.observe(10.0, ok=6, failed=0, mode=ordinary)  # once: then smoothed as ever
    assert rc.snr_db == pytest.approx(0.7 * 20.0 + 0.3 * 10.0)

    high = RateController()  # upward only
    high.seed(15.0, lower_bound=True)
    high.observe(10.0, ok=6, failed=0, mode=ordinary)
    assert high.snr_db == pytest.approx(0.7 * 15.0 + 0.3 * 10.0)
    near = RateController()  # within the estimates' own spread: smoothed, not replaced
    near.seed(10.0, lower_bound=True)
    near.observe(10.0 + near.reseed_margin_db, ok=6, failed=0, mode=ordinary)
    assert near.snr_db == pytest.approx(0.7 * 10.0 + 0.3 * (10.0 + near.reseed_margin_db))
    plain = RateController()  # an ordinary connect frame's measurement is not a lower bound
    plain.seed(4.0)
    plain.observe(20.0, ok=6, failed=0, mode=ordinary)
    assert plain.snr_db == pytest.approx(0.7 * 4.0 + 0.3 * 20.0)


def test_the_floor_boundary_is_crossed_by_what_the_rungs_are_worth() -> None:
    """ADR-0013 §4, ADR-0014: the floor's frames are five times as long as an OFDM frame. A
    session the SNR puts on the first OFDM rungs does not start on the floor; on an air that
    caps the first OFDM rung's margin the rung is held against the floor to its threshold
    plus the cap, however wide the learned margin, and one failed burst there does not leave
    for the floor — a second in a row does; the climb back asks for the cap and the
    hysteresis. The first OFDM rung is the first *usable* one: an OFDM mode the floor beats
    on rate and threshold both is on the ladder and never recommended."""
    rc = RateController()  # the wide ladder: rungs 0–5 are the tone floor
    assert rc.floor_modes == 6 and rc.modes[:6] == list(range(6))
    first = rc.modes[rc._first_ordinary()]
    second = rc.modes[rc._first_ordinary() + 1]
    fits_second = AWGN_THRESHOLD_DB[second] + rc.margin_db + rc.up_hysteresis_db
    assert rc.first_mode(fits_second) == first  # not two steps down, on the floor
    assert rc.first_mode(-12.0) < rc.floor_modes  # nothing above the floor fits: the floor

    capped = RateController(floor_margin_db=1.0)
    capped.seed(AWGN_THRESHOLD_DB[first] + 1.5)
    capped._index = capped.modes.index(first)
    capped.margin_db = 8.0  # a fading channel's learned margin
    snr = capped.snr_db
    capped.observe(snr, ok=0, failed=6, mode=first)
    assert capped.recommend() == first  # one lost burst on the capped rung: held
    capped.observe(snr, ok=0, failed=6, mode=first)
    assert capped.recommend() < first  # two in a row: the floor
    for _ in range(3):
        capped.observe(snr, ok=6, failed=0)
    assert capped.recommend() < first  # the cap and the hysteresis are not met at that SNR
    for _ in range(6):
        capped.observe(
            AWGN_THRESHOLD_DB[first] + 1.0 + capped.up_hysteresis_db + 0.5, ok=6, failed=0
        )
    assert capped.recommend() >= first  # they are here, whatever the learned margin

    # uncapped the learned margin decides, as between any two rungs
    plain = RateController(floor_margin_db=None)
    plain.seed(AWGN_THRESHOLD_DB[first] + 1.5)
    plain._index = plain.modes.index(first)
    plain.margin_db = 8.0
    plain.observe(plain.snr_db, ok=0, failed=6, mode=first)
    assert plain.recommend() < first


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


def _settle(rc: RateController, snr_db: float, bursts: int = 20) -> int:
    """Feed clean bursts at a fixed SNR until the recommendation stops moving."""
    for _ in range(bursts):
        rc.observe(snr_db, ok=6, failed=0)
    return rc.recommend()


def test_rate_recommendation_climbs_with_snr() -> None:
    floor = usable_modes()[0]
    prev = -1
    for snr in range(-6, 22, 2):
        m = _settle(RateController(), float(snr))
        assert m >= prev, (snr, m, prev)
        # never recommends a mode the SNR cannot carry — unless already on the slowest one
        assert m == floor or AWGN_THRESHOLD_DB[m] <= snr, (snr, m)
        prev = m


def test_rate_controller_converges_quickly() -> None:
    """A clean link must reach its final mode in a handful of bursts, not crawl up the table
    one mode at a time for half a minute. From the bottom of the ladder two things pace it
    and nothing else: the climb, :attr:`RateController.max_up_step` usable modes a burst,
    and the margin, which gives up :attr:`RateController.down_step_db` a clean burst before
    the first failure — a mode that fits only at the least margin waits for it. One burst
    more for the first measurement. (With the ladder of ADR-0013, sixteen rungs, the bound
    at 20 dB was the eight bursts this test once stated.)"""
    for snr in (0.0, 8.0, 14.0, 20.0):
        rc = RateController()
        climb = -(-rc.modes.index(_settle(RateController(), snr)) // rc.max_up_step)
        decay = math.ceil((rc.margin_db - rc.min_margin_db) / rc.down_step_db)
        final = _settle(RateController(), snr)
        track = [rc.observe(snr, ok=6, failed=0) or rc.recommend() for _ in range(30)]
        assert track.index(final) + 1 <= max(climb, decay) + 1, (snr, track)


def _boundary_track(rc: RateController, snr_db: float, bursts: int) -> list[int]:
    """Drive the controller against a channel that fails any mode needing more than
    ``snr_db − 1`` dB, and return the sequence of recommendations."""
    track = []
    for _ in range(bursts):
        m = rc.recommend()
        track.append(m)
        failed = 3 if AWGN_THRESHOLD_DB[m] > snr_db - 1.0 else 0
        rc.observe(snr_db, ok=6 - failed, failed=failed)
    return track


def test_rate_controller_does_not_oscillate_at_a_mode_boundary() -> None:
    """The point of the hysteresis: parked where two modes are nearly equally plausible, the
    controller must settle instead of flapping and losing a burst to every change."""
    track = _boundary_track(RateController(), 9.0, bursts=40)
    assert len(set(track[-20:])) == 1, track  # perfectly steady once settled
    assert AWGN_THRESHOLD_DB[track[-1]] <= 9.0


def test_rate_controller_backs_off_fast_when_the_channel_collapses() -> None:
    """Fast down, slow up: a 14 dB collapse must be absorbed in a couple of bursts."""
    rc = RateController()
    high = _settle(rc, 18.0)
    track = _boundary_track(rc, 4.0, bursts=12)
    assert track[0] == high
    sustainable = next(i for i, m in enumerate(track) if AWGN_THRESHOLD_DB[m] <= 3.0)
    assert sustainable <= 4, track
    assert len(set(track[-5:])) == 1, track  # and it stays there


def test_rate_margin_backs_off_on_failures() -> None:
    rc = RateController()
    rc.observe(10.0, ok=6, failed=0)
    good_mode = rc.recommend()
    base = rc.margin_db
    for _ in range(3):
        rc.observe(10.0, ok=0, failed=6)
    assert rc.margin_db > base  # failures widen the margin
    assert rc.recommend() <= good_mode  # a wider margin never picks a faster mode


def test_margin_learns_the_channel_penalty_in_one_step() -> None:
    """P2-2b: a failure is a measurement. Mode 4 dying at +12 dB says this channel wants
    ~11 dB more than the AWGN table predicts, so the margin jumps toward that instead of
    creeping up in fixed steps and overshooting the mode for several bursts first."""
    rc = RateController()
    start = rc.margin_db
    rc.observe(12.0, ok=0, failed=4, mode=4)
    assert rc.margin_db - start > rc.up_step_db  # targeted, not a fixed nudge
    assert rc.margin_db - start <= rc.max_jump_db  # but a single fade cannot strand the link


def test_learned_margin_is_sticky_then_decays() -> None:
    rc = RateController()
    rc.observe(12.0, ok=0, failed=4, mode=4)
    learned = rc.margin_db
    for _ in range(rc.decay_every - 1):
        rc.observe(12.0, ok=6, failed=0, mode=4)
    assert rc.margin_db == learned  # a couple of clean bursts do not give it up
    rc.observe(12.0, ok=6, failed=0, mode=4)
    assert rc.margin_db < learned  # but it is not permanent either


def test_a_learned_margin_is_given_back_faster_the_longer_bursts_stay_clean() -> None:
    """P9-2 (the faster climb): after the sticky bursts, every further clean burst gives back
    more of a learned margin than the one before, so a fade that has passed — or a collision
    that never was one — costs a handful of bursts, not twenty. Found on the VarAC bench,
    where one lost burst held a transfer a mode and a half below what the SNR carried."""
    rc = RateController()
    rc.observe(12.0, ok=0, failed=4, mode=4)
    learned = rc.margin_db
    steps: list[float] = []
    before = learned
    for _ in range(8):
        rc.observe(12.0, ok=6, failed=0, mode=4)
        steps.append(before - rc.margin_db)
        before = rc.margin_db
    assert steps[: rc.decay_every - 1] == [0.0] * (rc.decay_every - 1)  # sticky first
    taken = [s for s in steps if s > 0]
    assert taken[0] == rc.down_step_db
    # never slower, until the floor cuts the last step short
    assert all(b >= a for a, b in pairwise(taken[:-1]))
    assert taken[1] > taken[0]  # and faster from the second step on
    assert learned - rc.margin_db >= 2.0, steps  # most of it back within eight bursts
    assert max(taken) <= rc.max_down_step_db


def test_a_burst_that_failed_in_part_moves_the_margin_by_its_share() -> None:
    """ADR-0084: on a fading path a lone lost frame is the operating point HARQ pays for, not a
    cliff. One frame of six lost widens the margin by a sixth of what the whole burst lost
    would, the rung stays and the climb waits; half or more lost is a failure as before."""
    rcs = {lost: RateController() for lost in (1, 3, 6)}
    modes = {lost: _settle(rc, 12.0) for lost, rc in rcs.items()}
    mode = modes[1]
    assert set(modes.values()) == {mode}
    start = rcs[1].margin_db
    for lost, rc in rcs.items():
        rc.observe(12.0, ok=6 - lost, failed=lost, mode=mode)
    whole = rcs[6].margin_db - start
    assert rcs[1].recommend() == mode
    assert 0 < rcs[1].margin_db - start <= whole / 6 + 1e-9
    assert rcs[3].recommend() < mode and rcs[6].recommend() < mode
    # a clean burst after it: no climb past the rung on the burst that failed in part
    rcs[1].observe(14.0, ok=6, failed=0, mode=mode)
    assert rcs[1].recommend() >= mode


def test_a_burst_that_failed_in_part_on_the_first_500_hz_ofdm_rung_goes_back_to_the_floor() -> None:
    """ADR-0084: under the 500 Hz air's first OFDM rung is the tone floor, which carries an
    exchange better than the rung does once the rung loses frames; holding the rung there on
    a partial failure cost 3 % at 0 dB on Moderate and Poor, so it steps down as before."""
    from aether_model.frame.modes import NARROW
    from aether_model.link.rate import (
        NARROW_AWGN_THRESHOLD_DB,
        NARROW_FRAME_S,
        NARROW_PAYLOAD_BYTES,
    )

    thresholds = dict(NARROW_AWGN_THRESHOLD_DB)

    def narrow() -> RateController:
        return RateController(
            thresholds=thresholds,
            modes=usable_modes(thresholds, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S),
            floor_modes=NARROW.floor_modes,
        )

    rc = narrow()
    edge = _settle(rc, -2.0)
    assert edge == NARROW.floor_modes  # the first OFDM rung
    rc.observe(-2.0, ok=5, failed=1, mode=edge)
    assert rc.recommend() < edge
    # one rung up, a partial failure holds the rung as anywhere else
    rc = narrow()
    above = _settle(rc, 1.0)
    assert above > edge
    rc.observe(1.0, ok=5, failed=1, mode=above)
    assert rc.recommend() == above


def test_margin_decays_freely_before_anything_is_learned() -> None:
    """Stickiness protects a *learned* penalty; on a link that has never failed there is
    nothing to protect, so a clean channel must still reach its mode quickly."""
    rc = RateController()
    for _ in range(3):
        rc.observe(10.0, ok=6, failed=0, mode=0)
    assert rc.margin_db < RateController().margin_db - 2 * rc.down_step_db


def _narrow_echo() -> RateController:
    from aether_model.frame.modes import NARROW
    from aether_model.link.rate import (
        NARROW_AWGN_THRESHOLD_DB,
        NARROW_ECHO_LIMIT,
        NARROW_FRAME_S,
        NARROW_PAYLOAD_BYTES,
    )

    thresholds = dict(NARROW_AWGN_THRESHOLD_DB)
    return RateController(
        thresholds=thresholds,
        modes=usable_modes(thresholds, NARROW_PAYLOAD_BYTES, NARROW_FRAME_S),
        floor_modes=NARROW.floor_modes,
        echo_limit=dict(NARROW_ECHO_LIMIT),
    )


def test_an_echo_keeps_the_rungs_it_breaks_off_the_ladder() -> None:
    """ADR-0087: a floor reading of -3 dB is a quiet path at -3 dB or ITU's disturbed NVIS at
    any SNR, and on the second the 500 Hz air's rungs 3 and 4 and its OFDM rungs do not carry.
    The echo the floor's frames read tells the two apart: the session starts and stays below
    the rungs the echo breaks, and climbs past them once the echo is gone."""
    from aether_model.link.rate import NARROW_ECHO_LIMIT

    quiet = _narrow_echo()
    assert quiet.first_mode(-3.0) == 4
    rc = _narrow_echo()
    rc.observe_echo(0.01, -3.0)
    assert rc.first_mode(-3.0) <= 2
    rc.seed(-3.0, lower_bound=True)
    for _ in range(12):
        rc.observe(-3.0, ok=6, failed=0, mode=rc.recommend())
        rc.observe_echo(0.01, -3.0)
        assert all(rc.recommend() < m for m in NARROW_ECHO_LIMIT)
    for _ in range(12):
        rc.observe_echo(0.0, 5.0)
        rc.observe(5.0, ok=6, failed=0, mode=rc.recommend())
    assert rc.recommend() > min(NARROW_ECHO_LIMIT)


def test_an_echo_read_at_once_takes_the_recommendation_down() -> None:
    """A recommendation the echo rules out does not wait for its frames to fail: the next
    burst goes at the fastest rung the echo allows and the SNR carries. A weak frame's reading
    counts for nothing (``echo_min_snr_db``): its noise is as large as a 4 ms echo's spill."""
    rc = _narrow_echo()
    rc.seed(-3.0, lower_bound=True)
    assert rc.recommend() == 4
    rc.observe_echo(0.05, rc.echo_min_snr_db - 1.0)
    assert rc.echo is None and rc.recommend() == 4
    rc.observe_echo(0.01, -3.0)
    assert rc.recommend() == 2
