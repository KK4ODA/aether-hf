"""Link layer over the *real* PHY and channel simulator (roadmap P2-1).

These drive two :class:`LinkEngine` stations through the actual modem — real acquisition,
demodulation, LDPC decoding and LLR-domain HARQ-IR combining — so they are slower than the
lossy-pipe tests in ``test_link.py``. They prove the protocol works on the waveform, not
just on the abstract channel model."""

from __future__ import annotations

import pytest

from aether_model.link.engine import LinkConfig, LinkEngine, State
from aether_model.link.harness import phy_timing, two_modem_sim


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
    cfg = LinkConfig(initial_mode=4, max_mode=4, max_retries=40)
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
