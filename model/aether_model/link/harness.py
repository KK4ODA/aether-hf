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

from collections.abc import Callable
from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.channel import WattersonChannel, make_channel
from aether_model.frame.modes import PREAMBLE_SYMBOLS, air_interface
from aether_model.link.engine import LinkEngine
from aether_model.link.frames import with_bandwidth
from aether_model.link.phy import Container, PhyTiming, SoftFrame, TxFrame
from aether_model.link.rate import (
    AWGN_THRESHOLD_DB,
    CONTROL_THRESHOLD_DB,
    NARROW_AWGN_THRESHOLD_DB,
    NARROW_CONTROL_THRESHOLD_DB,
)
from aether_model.link.sim import TwoStationSim
from aether_model.phy import tone
from aether_model.phy.pipeline import Modem
from aether_model.phy.preamble import FrameType
from aether_model.waveform import WIDE_2300, Bandwidth, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

WIDE_FLOOR_MARGIN_DB = 1.0
"""``PhyTiming.floor_margin_db`` of the 2 300 Hz air (ADR-0013 §4, the link bench)."""


@dataclass
class RealSoftFrame:
    """A frame recovered by the real receiver — OFDM or the tone floor's; ``decode`` runs
    the LDPC decoder and combines LLRs with the opaque HARQ buffer of an earlier
    transmission of the same block."""

    container: Container
    mode: int
    """The rung of the ladder the frame was sent at (0 for a control frame)."""
    rv: int
    snr_db: float
    t_start: float
    t_end: float
    floor: bool
    _decode: Callable[[FloatArray | None], tuple[bytes | None, FloatArray]]
    trusted: bool = True
    """The harness detects in a buffer that holds the frame it sent: what it finds is real."""

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        buf = buffer if isinstance(buffer, np.ndarray) else None
        return self._decode(buf)


def bandwidth_capabilities(params: WaveformParams = WIDE_2300, caps: int = 0) -> int:
    """``caps`` with the bandwidth bits of ``params`` set — what a station puts in its
    :attr:`~aether_model.link.engine.LinkConfig.capabilities` for the waveform it runs."""
    return with_bandwidth(caps, params.bandwidth.hz)


def phy_timing(params: WaveformParams = WIDE_2300, start_of_frame: bool = True) -> PhyTiming:
    """Timing and per-mode capacities for the real waveform.

    ``preamble_detect_s`` is the preamble plus two symbol periods: the two Schmidl–Cox
    symbols the detector correlates against, plus the one-symbol sidelobe guard it needs
    before accepting a peak (P2-3), plus a symbol of slack for block-boundary latency in
    the streaming receiver; the tone floor's frames are announced later
    (:func:`~aether_model.phy.tone.announce_delay_s`). Pass ``start_of_frame=False`` to
    model a PHY that cannot report preambles.
    """
    air = air_interface(params)
    caps = {r.index: r.payload_bytes for r in air.ladder}
    wide = params.bandwidth is Bandwidth.WIDE_2300
    thresholds = AWGN_THRESHOLD_DB if wide else NARROW_AWGN_THRESHOLD_DB
    controls = CONTROL_THRESHOLD_DB if wide else NARROW_CONTROL_THRESHOLD_DB
    floor_s = {k.duration_s for k in air.tone_data}
    if len(floor_s) != 1:
        raise ValueError("the link layer takes one floor data-frame length")
    return PhyTiming(
        data_frame_s=air.long.duration_s,
        control_frame_s=air.short.duration_s,
        turnaround_s=0.25,
        detect_latency_s=0.15,
        preamble_detect_s=(
            (PREAMBLE_SYMBOLS + 2) * params.symbol_period_s if start_of_frame else None
        ),
        data_capacity=caps,
        mode_threshold_db=dict(thresholds),
        control_threshold_db=dict(controls),
        floor_data_frame_s=floor_s.pop(),
        floor_control_frame_s=air.tone_control.duration_s,
        floor_modes=air.floor_modes,
        floor_preamble_detect_s=tone.announce_delay_s() if start_of_frame else None,
        # the wide air's first OFDM rung stays productive on a fading path a decibel above its
        # 10 % point; the narrow air's does not (ADR-0013 §4)
        floor_margin_db=WIDE_FLOOR_MARGIN_DB if wide else None,
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
        continuous: bool = False,
    ) -> None:
        self.modem = Modem(params)
        self.continuous = continuous
        """One fade for the whole session, shared by both directions and running on through
        every gap (P9-6), instead of a fresh channel per frame: consecutive frames, and a
        burst and its acknowledgement, then see the same fade, as they do on the air."""
        self._fading = WattersonChannel(channel, params.fs_baseband, seed=seed + 7)
        self._clock = 0.0
        """Channel time already consumed, seconds: where the next burst's fade starts."""
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
            return self.modem.rung_burst(frame.payload, frame.mode, frame.rv)
        return self.modem.control_burst(frame.payload, frame.rv, floor=frame.floor)

    def padded(self, burst: ComplexArray) -> ComplexArray:
        """The burst between its lead and tail of silence.

        The tail is long enough for a DATA frame's span from any start inside the burst,
        whatever was sent: on a fading channel the detector now and then reads a control
        burst's header as DATA, and the receiver then wants the long layout's samples —
        which a real receiver has, since audio keeps arriving. Here it would run off the
        end of the buffer instead, and did, seven minutes into a Poor-channel run."""
        p = self.modem.p
        span = max(x.samples for x in self.modem.air.layouts) + self.modem.rx.dem.fft_offset
        tail = max(self.tail, span + p.symbol_samples - len(burst), 2 * p.symbol_samples)
        return np.concatenate((np.zeros(self.lead, complex), burst, np.zeros(tail, complex)))

    def factory(
        self, frame: TxFrame, snr_db: float, t_start: float, t_end: float
    ) -> SoftFrame | None:
        self.rendered += 1
        burst = self._burst(frame)
        seed = int(self._rng.integers(0, 2**31))
        cfo = float(self._rng.uniform(-100, 100))
        if self.continuous:
            # the burst through the session's fade at its own time; the silence either side
            # carries only noise, so the fade need not run through it
            self._fading.skip(max(0, round((t_start - self._clock) * self.fs)))
            burst = self._fading.process(burst)
            self._clock = max(self._clock, t_start) + len(burst) / self.fs
            ch = make_channel(
                "awgn", snr_db=snr_db, fs=self.fs, seed=seed, signal_power=1.0, cfo_hz=cfo
            )
        else:
            ch = make_channel(
                self.channel, snr_db=snr_db, fs=self.fs, seed=seed, signal_power=1.0, cfo_hz=cfo
            )
        buf = self.padded(burst)
        y = self.modem.detector.condition(ch.process(buf))
        # the tone floor first: its detector confirms what it finds, and an OFDM frame is
        # never taken for one (``test_tone.py``)
        tones = self.modem.tone_detector.detect(y, max_frames=1)
        if tones:
            ts = tones[0]
            if ts.start + ts.kind.samples > len(y):
                self.overrun += 1
                return None
            self.detected += 1
            soft = tone.demodulate(y, ts.kind, ts.rv, ts.start, ts.cfo_hz)
            air = self.modem.air
            return RealSoftFrame(
                container=Container.CONTROL if ts.kind.control else Container.DATA,
                mode=0 if ts.kind.control else air.tone_data.index(ts.kind),
                rv=ts.rv,
                snr_db=soft.snr_db,
                t_start=t_start,
                t_end=t_end,
                floor=True,
                _decode=soft.decode,
            )
        syncs = self.modem.detector.detect(y, max_frames=1)
        if not syncs:
            return None
        try:
            received = self.modem.demodulate(y, syncs[0])
        except ValueError:
            # placed later than a symbol past the burst: gone, and counted
            self.overrun += 1
            return None
        data = received.sync.header.frame_type is FrameType.DATA
        try:
            rung = self.modem.air.rung_of(received.mode) if data else 0
        except ValueError:
            return None  # chips naming an OFDM mode on no rung: noise
        self.detected += 1

        def decode(buffer: FloatArray | None) -> tuple[bytes | None, FloatArray]:
            return self.modem.decode_frame(received, buffer)

        return RealSoftFrame(
            container=Container.DATA if data else Container.CONTROL,
            mode=rung,
            rv=received.rv,
            snr_db=received.snr_3k_db,
            t_start=t_start,
            t_end=t_end,
            floor=False,
            _decode=decode,
        )


def two_modem_sim(
    a: LinkEngine,
    b: LinkEngine,
    *,
    channel: str = "awgn",
    snr_db: float = 10.0,
    seed: int = 0,
    params: WaveformParams = WIDE_2300,
    continuous: bool = False,
) -> TwoStationSim:
    """A :class:`TwoStationSim` whose channel is the real PHY. Both directions share one
    bridge (one modem, one RNG stream) — fine because the pipe is half-duplex."""
    bridge = PhyBridge(
        channel=channel, snr_db=snr_db, seed=seed, params=params, continuous=continuous
    )
    sim = TwoStationSim(a, b, snr_db=snr_db, seed=seed, frame_factory=bridge.factory)
    sim.bridge = bridge  # type: ignore[attr-defined]
    return sim
