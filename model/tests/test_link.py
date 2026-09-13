"""Link layer: frame formats, ARQ engine and session FSM over the lossy-pipe simulator
(roadmap P2-1). The bit-exact-over-the-real-PHY check lives in ``test_link_harness.py``."""

from __future__ import annotations

import pytest

from aether_model.frame.modes import LONG, MODES, SHORT
from aether_model.link.engine import LinkConfig, LinkEngine, Role, State
from aether_model.link.frames import (
    ConnectBody,
    ControlFlags,
    ControlFrame,
    ControlKind,
    DataHeader,
    DataKind,
    decode_data,
    encode_data,
    pack_callsign,
    unpack_callsign,
)
from aether_model.link.phy import PhyTiming
from aether_model.link.rate import AWGN_THRESHOLD_DB, RateController, usable_modes
from aether_model.link.sim import TwoStationSim


@pytest.fixture(scope="module")
def timing() -> PhyTiming:
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    return PhyTiming(
        data_frame_s=LONG.duration_s, control_frame_s=SHORT.duration_s, data_capacity=caps
    )


def _pair(timing: PhyTiming, config: LinkConfig | None = None) -> tuple[LinkEngine, LinkEngine]:
    return (
        LinkEngine("W4ODA", timing, config, seed=1),
        LinkEngine("KK4XYZ", timing, config, seed=2),
    )


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


def test_connect_body_round_trip() -> None:
    body = ConnectBody("W4ODA", "KK4XYZ", caps=0b101, version=1)
    assert ConnectBody.decode(body.encode()) == body


def test_control_frame_round_trip() -> None:
    ctl = ControlFrame(
        ControlKind.ACK,
        session=9,
        flags=ControlFlags.WANT_TX,
        base=100,
        bitmap=0b1011,
        snr_db=-7.0,
        recommended_mode=6,
        counter=5,
    )
    got = ControlFrame.decode(ctl.encode())
    assert (got.kind, got.session, got.flags, got.base, got.bitmap) == (
        ctl.kind,
        ctl.session,
        ctl.flags,
        ctl.base,
        ctl.bitmap,
    )
    assert got.snr_db == -7.0 and got.recommended_mode == 6 and got.counter == 5


def test_ack_received_semantics() -> None:
    ack = ControlFrame(ControlKind.ACK, session=0, base=10, bitmap=0b0101)  # 10 and 12 present
    assert ack.received(10) and ack.received(12)
    assert not ack.received(11) and not ack.received(13)
    assert ack.received(9) and ack.received(5)  # below base = already acknowledged


# ── rate controller ───────────────────────────────────────────────────


def test_usable_modes_are_pareto_and_sorted() -> None:
    modes = usable_modes()
    assert modes == sorted(modes)
    # mode 7 (8-PSK 2/3) is dominated by mode 8 (16-QAM 1/2): more payload, lower threshold
    assert 7 not in modes
    for m in modes:
        assert not any(
            AWGN_THRESHOLD_DB[o] <= AWGN_THRESHOLD_DB[m] and o != m and o in modes
            for o in modes
            if o > m
        )


def test_rate_recommendation_climbs_with_snr() -> None:
    prev = -1
    for snr in range(-4, 22, 2):
        rc = RateController()
        rc.observe(float(snr), ok=6, failed=0)
        m = rc.recommend()
        assert m >= prev
        prev = m


def test_rate_margin_backs_off_on_failures() -> None:
    rc = RateController()
    rc.observe(10.0, ok=6, failed=0)
    good_mode = rc.recommend()
    base = rc.margin_db
    for _ in range(3):
        rc.observe(10.0, ok=0, failed=6)
    assert rc.margin_db > base  # failures widen the margin
    assert rc.recommend() <= good_mode  # a wider margin never picks a faster mode


# ── session FSM over the simulator ────────────────────────────────────


def _run_transfer(timing: PhyTiming, msg: bytes, snr_db: float, seed: int) -> TwoStationSim:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=seed)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=2000)
    return sim


def test_connect_transfer_disconnect_clean_channel(timing: PhyTiming) -> None:
    msg = b"The quick brown fox jumps over the lazy dog. " * 20
    sim = _run_transfer(timing, msg, snr_db=15.0, seed=7)
    a, b = sim.st[0].engine, sim.st[1].engine
    assert sim.delivered(1) == msg
    assert a.state is State.IDLE and b.state is State.IDLE
    assert "connected:KK4XYZ (iss)" in sim.events(0)
    assert any(e.startswith("disconnected") for e in sim.events(0))
    assert any(e.startswith("disconnected") for e in sim.events(1))


@pytest.mark.parametrize("snr_db", [20.0, 12.0, 6.0, 3.0])
def test_transfer_completes_across_snr(timing: PhyTiming, snr_db: float) -> None:
    msg = bytes((i * 37) % 256 for i in range(1200))
    sim = _run_transfer(timing, msg, snr_db=snr_db, seed=int(snr_db) + 3)
    assert sim.delivered(1) == msg


def test_throughput_increases_with_snr(timing: PhyTiming) -> None:
    msg = bytes(4000)
    times = {}
    for snr in (4.0, 18.0):
        a, b = _pair(timing)
        sim = TwoStationSim(a, b, snr_db=snr, seed=11)
        a.connect("KK4XYZ")
        a.send(msg)
        a.disconnect()
        times[snr] = sim.run(until=3000)
        assert sim.delivered(1) == msg
    assert times[18.0] < 0.6 * times[4.0]


def test_harq_ir_rescues_below_threshold(timing: PhyTiming) -> None:
    """Mode pinned to QPSK ½ (threshold ≈ +1 dB) and driven at −3 dB: the transfer only
    completes because retransmissions are soft-combined."""
    cfg = LinkConfig(initial_mode=4, max_mode=4, max_retries=60)
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    sim = TwoStationSim(a, b, snr_db=-3.0, seed=99)
    msg = bytes(1500)
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=6000)
    assert sim.delivered(1) == msg
    assert b.stats.harq_rescues > 0


def test_break_hands_over_the_channel(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=12.0, seed=5)
    reply = b"REPLY FROM KK4XYZ. " * 8
    a.connect("KK4XYZ")
    a.send(bytes(3000))
    b.send(reply)
    b.request_break()
    sim.run(until=600)
    assert sim.delivered(1) == bytes(3000)  # A → B still completes
    assert sim.delivered(0) == reply  # B → A after the handover
    assert a.stats.turns == 1


def test_connect_fails_when_peer_is_absent(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    b.state = State.IDLE  # B never listens: give it a session that ignores everything
    sim = TwoStationSim(a, b, snr_db=15.0, seed=1)
    # deafen B by dropping every frame to it
    sim._busy = lambda who, t0, t1: who == 1 or TwoStationSim._busy(sim, who, t0, t1)  # type: ignore[method-assign]
    a.connect("KK4XYZ")
    sim.run(until=200)
    assert a.state is State.IDLE
    assert any(e == "disconnected:no answer" for e in sim.events(0))


def test_simultaneous_connect_resolves(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=2)
    a.connect("KK4XYZ")
    b.connect("W4ODA")
    a.send(b"data from A" * 3)
    sim.run(until=300)
    # exactly one becomes ISS, one IRS, and they agree on a session
    assert {a.role, b.role} == {Role.ISS, Role.IRS}
    assert a.session == b.session
    assert sim.delivered(1) == b"data from A" * 3


def test_peer_disconnect_is_observed(timing: PhyTiming) -> None:
    a, b = _pair(timing)
    sim = TwoStationSim(a, b, snr_db=15.0, seed=4)
    a.connect("KK4XYZ")
    a.send(b"short message")
    a.disconnect()
    sim.run(until=200)
    assert any("disconnected" in e for e in sim.events(1))
    assert b.state is State.IDLE
