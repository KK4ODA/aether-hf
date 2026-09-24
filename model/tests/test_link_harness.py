"""Link layer over the *real* PHY and channel simulator (roadmap P2-1).

These drive two :class:`LinkEngine` stations through the actual modem — real acquisition,
demodulation, LDPC decoding and LLR-domain HARQ-IR combining — so they are slower than the
lossy-pipe tests in ``test_link.py``. They prove the protocol works on the waveform, not
just on the abstract channel model."""

from __future__ import annotations

import pytest

from aether_model.frame.modes import WIDE
from aether_model.link.engine import LinkConfig, LinkEngine, State
from aether_model.link.harness import PhyBridge, phy_timing, two_modem_sim
from aether_model.waveform import NARROW_500, WIDE_2300


@pytest.fixture(scope="module")
def timing() -> object:
    return phy_timing()


def test_real_phy_connect_transfer_disconnect(timing: object) -> None:
    a = LinkEngine("W4ODA", timing, seed=1)
    b = LinkEngine("KK4XYZ", timing, seed=2)
    sim = two_modem_sim(a, b, channel="awgn", snr_db=8.0, seed=7)
    msg = b"Aether HF link layer over the real waveform. The quick brown fox. 73" * 2
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=600)
    assert sim.delivered(1) == msg
    assert a.state is State.IDLE and b.state is State.IDLE
    assert sim.bridge.detected == sim.bridge.rendered  # clean channel: nothing dropped


def test_real_phy_harq_ir_rescue_below_threshold(timing: object) -> None:
    """QPSK ½ pinned and driven at 0 dB (≈ 1 dB below its single-shot threshold): the
    transfer completes only because the receiver soft-combines real LLRs across redundancy
    versions."""
    rung = WIDE.rung_of(4)
    cfg = LinkConfig(initial_mode=rung, max_mode=rung, max_retries=40)
    a = LinkEngine("W4ODA", timing, cfg, seed=1)
    b = LinkEngine("KK4XYZ", timing, cfg, seed=2)
    sim = two_modem_sim(a, b, channel="awgn", snr_db=0.0, seed=3)
    msg = bytes((i * 91) % 256 for i in range(300))
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=1500)
    assert sim.delivered(1) == msg
    assert b.stats.harq_rescues > 0


def test_the_bridge_leaves_room_for_a_control_burst_read_as_data() -> None:
    """On a fading channel the detector now and then reads a control burst's header as
    DATA and the receiver wants the long layout's samples; a real receiver has them,
    because audio keeps arriving, so the bridge's buffer must too. Found seven minutes
    into a Poor-channel run of the link bench, as "frame runs past the end of the buffer"."""
    for params in (WIDE_2300, NARROW_500):
        bridge = PhyBridge(channel="poor", snr_db=10.0, params=params)
        control = bridge.modem.control_burst(bytes(7), 0)
        buf = bridge.padded(control)
        # the longest OFDM layout of the air
        span = max(x.samples for x in bridge.modem.air.layouts) + bridge.modem.rx.dem.fft_offset
        # a start anywhere up to a symbol late still fits a DATA span
        assert len(buf) >= bridge.lead + params.symbol_samples + span, params.bandwidth
        for rung in (0, bridge.modem.air.floor_modes):  # the tone floor's, and an OFDM one
            capacity = int(phy_timing(params).data_capacity[rung])
            data = bridge.modem.rung_burst(bytes(capacity), rung, 0)
            assert len(bridge.padded(data)) >= bridge.lead + len(data) + bridge.tail


@pytest.mark.parametrize("params", [WIDE_2300, NARROW_500], ids=["2300", "500"])
def test_real_phy_session_on_the_tone_floor(params: object) -> None:
    """At −14 dB (3 kHz, the OFDM frames' reference) nothing but the tone floor (ADR-0013)
    carries a frame on either air: the call goes out on it from the first try (ADR-0016), and
    the session runs connect, data and acknowledgements on tone frames through the real
    modem."""
    timing = phy_timing(params)  # type: ignore[arg-type]
    a = LinkEngine("W4ODA", timing, seed=1)
    b = LinkEngine("KK4XYZ", timing, seed=2)
    sim = two_modem_sim(a, b, channel="awgn", snr_db=-14.0, seed=4, params=params)  # type: ignore[arg-type]
    msg = b"Aether HF on the tone floor. 73"
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=900)
    assert sim.delivered(1) == msg
    assert a.state is State.IDLE and b.state is State.IDLE
    # the call on the floor's most robust kind, answered at once; everything on the floor
    air = sim.bridge.modem.air
    sent = sim.bridge.modes_sent
    assert sent[0] == 0 and a.stats.frames_sent > 0, sent
    assert set(sent) <= set(range(air.floor_modes)), sent


def test_real_phy_probe_reports_both_directions(timing: object) -> None:
    """A probe over the real modem: the answer carries the SNR the probe arrived at, and
    the prober measures the answer — two readings of one path, no session."""
    a = LinkEngine("W4ODA", timing, seed=1)
    b = LinkEngine("KK4XYZ", timing, seed=2)
    sim = two_modem_sim(a, b, channel="awgn", snr_db=8.0, seed=9)
    a.probe("KK4XYZ")
    sim.run(until=60)
    reports = [e for e in sim.events(0) if e.startswith("probe:")]
    assert len(reports) == 1, sim.events(0)
    words = reports[0].split()
    # "probe:KK4XYZ hears us at <x> dB, heard at <y> dB"
    theirs, ours = float(words[4]), float(words[8])
    assert abs(theirs - 8.0) < 3.0 and abs(ours - 8.0) < 3.0, reports
    assert any(e.startswith("probed:W4ODA at") for e in sim.events(1))
    assert a.state is State.IDLE and b.state is State.IDLE


@pytest.mark.slow
def test_real_phy_transfer_on_poor_channel(timing: object) -> None:
    """ITU Poor at +12 dB with adaptive rate: the transfer must still complete bit-exact."""
    a = LinkEngine("W4ODA", timing, seed=1)
    b = LinkEngine("KK4XYZ", timing, seed=2)
    sim = two_modem_sim(a, b, channel="poor", snr_db=12.0, seed=5)
    msg = bytes((i * 53) % 256 for i in range(500))
    a.connect("KK4XYZ")
    a.send(msg)
    a.disconnect()
    sim.run(until=2000)
    assert sim.delivered(1) == msg
