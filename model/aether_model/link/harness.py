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
from aether_model.frame.modes import air_interface
from aether_model.link.engine import LinkEngine
from aether_model.link.frames import with_bandwidth
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB
from aether_model.link.sim import TwoStationSim
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType
from aether_model.phy.rx import ReceivedFrame, layout_for
from aether_model.waveform import WIDE_2300, Bandwidth, WaveformParams

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


def bandwidth_capabilities(params: WaveformParams = WIDE_2300, caps: int = 0) -> int:
    """``caps`` with the bandwidth bits of ``params`` set — what a station puts in its
    :attr:`~aether_model.link.engine.LinkConfig.capabilities` for the waveform it runs."""
    return with_bandwidth(caps, params.bandwidth.hz)


def phy_timing(params: WaveformParams = WIDE_2300, start_of_frame: bool = True) -> PhyTiming:
    """Timing and per-mode capacities for the real waveform.

    ``preamble_detect_s`` is four symbol periods: the two Schmidl–Cox symbols the detector
    correlates against, plus the one-symbol sidelobe guard it needs before accepting a peak
    (P2-3), plus a symbol of slack for block-boundary latency in the streaming receiver.
    Pass ``start_of_frame=False`` to model a PHY that cannot report preambles.
    """
    air = air_interface(params)
    caps = {m.index: m.payload_bytes(air.long) for m in air.modes}
    thresholds = (
        AWGN_THRESHOLD_DB if params.bandwidth is Bandwidth.WIDE_2300 else NARROW_AWGN_THRESHOLD_DB
    )
    return PhyTiming(
        data_frame_s=air.long.duration_s,
        control_frame_s=air.short.duration_s,
        turnaround_s=0.25,
        detect_latency_s=0.15,
        preamble_detect_s=4 * params.symbol_period_s if start_of_frame else None,
        data_capacity=caps,
        mode_threshold_db=dict(thresholds),
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
        self.overrun = 0
        """Frames the detector placed so late that their span left the buffer even with
        the padding below — lost, as a receiver that ran out of audio would lose them."""
        self.modes_sent: list[int] = []
        """The mode of every DATA frame rendered, in order — what the rate controller did."""

    def _burst(self, frame: TxFrame) -> ComplexArray:
        if frame.container is Container.DATA:
            self.modes_sent.append(frame.mode)
            return self.modem.data_burst(frame.payload, self.modem.modes[frame.mode], frame.rv)
        return self.modem.control_burst(frame.payload, frame.rv)

    def padded(self, burst: ComplexArray) -> ComplexArray:
        """The burst between its lead and tail of silence.

        The tail is long enough for a DATA frame's span from any start inside the burst,
        whatever was sent: on a fading channel the detector now and then reads a control
        burst's header as DATA, and the receiver then wants the long layout's samples —
        which a real receiver has, since audio keeps arriving. Here it would run off the
        end of the buffer instead, and did, seven minutes into a Poor-channel run."""
        p = self.modem.p
        span = layout_for(FrameType.DATA, p).samples + self.modem.rx.dem.fft_offset
        tail = max(self.tail, span + p.symbol_samples - len(burst))
        return np.concatenate((np.zeros(self.lead, complex), burst, np.zeros(tail, complex)))

    def factory(
        self, frame: TxFrame, snr_db: float, t_start: float, t_end: float
    ) -> SoftFrame | None:
        self.rendered += 1
        buf = self.padded(self._burst(frame))
        seed = int(self._rng.integers(0, 2**31))
        cfo = float(self._rng.uniform(-100, 100))
        ch = make_channel(
            self.channel, snr_db=snr_db, fs=self.fs, seed=seed, signal_power=1.0, cfo_hz=cfo
        )
        y = self.modem.detector.condition(ch.process(buf))
        syncs = self.modem.detector.detect(y, max_frames=1)
        if not syncs:
            return None
        try:
            received = self.modem.demodulate(y, syncs[0])
        except ValueError:
            # placed later than a symbol past the burst: gone, and counted
            self.overrun += 1
            return None
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
