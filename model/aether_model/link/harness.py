"""Two-modem harness over the *real* PHY and channel simulator (roadmap P2-1).

Where :mod:`aether_model.link.sim` fakes the channel with a probability model, this renders
every link frame to complex baseband with the real :class:`~aether_model.phy.pipeline.Modem`,
pushes it through :func:`~aether_model.channel.make_channel`, and recovers it with real
acquisition, demodulation and LDPC decoding. It is slow (seconds of audio per frame) but it
proves the ARQ engine drives the actual waveform — including genuine HARQ-IR soft-combining
of LLRs across redundancy versions — not just an abstract model.

It reuses :class:`TwoStationSim`'s event loop through the ``frame_factory`` hook: the factory
renders one :class:`~aether_model.link.phy.TxFrame` and returns a :class:`RealSoftFrame`
(or ``None`` if acquisition failed on that frame).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.channel import make_channel
from aether_model.frame.modes import LONG, MODES, SHORT
from aether_model.link.engine import LinkEngine
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.sim import TwoStationSim
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType
from aether_model.phy.rx import ReceivedFrame
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]


@dataclass
class RealSoftFrame:
    """A frame recovered by the real receiver; ``decode`` runs the LDPC decoder and combines
    LLRs with the opaque HARQ buffer of an earlier transmission of the same block."""

    container: Container
    mode: int
    rv: int
    snr_db: float
    t_start: float
    t_end: float
    _modem: Modem
    _received: ReceivedFrame

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        buf = buffer if isinstance(buffer, np.ndarray) else None
        payload, llr = self._modem.decode_frame(self._received, buf)
        return payload, llr


def phy_timing(params: WaveformParams = WIDE_2300) -> PhyTiming:
    """Timing and per-mode capacities for the real waveform."""
    caps = {m.index: m.payload_bytes(LONG) for m in MODES}
    return PhyTiming(
        data_frame_s=LONG.duration_s,
        control_frame_s=SHORT.duration_s,
        turnaround_s=0.25,
        detect_latency_s=0.15,
        data_capacity=caps,
    )


class PhyBridge:
    """Renders link frames through the real modem and a channel, one frame at a time."""

    def __init__(
        self,
        channel: str = "awgn",
        snr_db: float = 10.0,
        *,
        seed: int = 0,
        params: WaveformParams = WIDE_2300,
        lead: int = 900,
        tail: int = 900,
    ) -> None:
        self.modem = Modem(params)
        self.channel = channel
        self.snr_db = snr_db
        self.lead = lead
        self.tail = tail
        self.fs = params.fs_baseband
        self._rng = np.random.default_rng(seed)
        self.rendered = 0
        self.detected = 0

    def _burst(self, frame: TxFrame) -> ComplexArray:
        if frame.container is Container.DATA:
            return self.modem.data_burst(frame.payload, MODES[frame.mode], frame.rv)
        return self.modem.control_burst(frame.payload, frame.rv)

    def factory(
        self, frame: TxFrame, snr_db: float, t_start: float, t_end: float
    ) -> SoftFrame | None:
        self.rendered += 1
        burst = self._burst(frame)
        buf = np.concatenate((np.zeros(self.lead, complex), burst, np.zeros(self.tail, complex)))
        seed = int(self._rng.integers(0, 2**31))
        cfo = float(self._rng.uniform(-100, 100))
        ch = make_channel(
            self.channel, snr_db=snr_db, fs=self.fs, seed=seed, signal_power=1.0, cfo_hz=cfo
        )
        y = self.modem.detector.condition(ch.process(buf))
        syncs = self.modem.detector.detect(y, max_frames=1)
        if not syncs:
            return None
        received = self.modem.demodulate(y, syncs[0])
        self.detected += 1
        container = (
            Container.DATA
            if received.sync.header.frame_type is FrameType.DATA
            else Container.CONTROL
        )
        return RealSoftFrame(
            container=container,
            mode=received.mode,
            rv=received.rv,
            snr_db=received.snr_3k_db,
            t_start=t_start,
            t_end=t_end,
            _modem=self.modem,
            _received=received,
        )


def two_modem_sim(
    a: LinkEngine,
    b: LinkEngine,
    *,
    channel: str = "awgn",
    snr_db: float = 10.0,
    seed: int = 0,
    params: WaveformParams = WIDE_2300,
) -> TwoStationSim:
    """A :class:`TwoStationSim` whose channel is the real PHY. Both directions share one
    bridge (one modem, one RNG stream) — fine because the pipe is half-duplex."""
    bridge = PhyBridge(channel=channel, snr_db=snr_db, seed=seed, params=params)
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=seed, frame_factory=bridge.factory)
    sim.bridge = bridge  # type: ignore[attr-defined]
    return sim
