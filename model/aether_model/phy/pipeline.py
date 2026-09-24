"""Convenience end-to-end helpers: payload → burst and buffer → decoded frames (P1-7).

These tie the frame codec, transmitter, detector and receiver together for tests, the
benchmark runner and the link harness. They are deliberately thin. Both families of the air
go through here: the OFDM frames, and the tone floor's (ADR-0013) — a ladder rung says which.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.codec import FrameCodec
from aether_model.frame.modes import FrameLayout, Mode, air_interface
from aether_model.phy.blanker import NoiseBlanker
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.phy.rx import FrameReceiver, ReceivedFrame
from aether_model.phy.sync import FrameDetector, FrameSync
from aether_model.phy.tone import ToneDetector, ToneFrame, ToneSync, burst, demodulate
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

MODE_RETRY_CONFIDENCE = 1.3
"""Below this chip-metric ratio a CRC failure triggers a retry with the runner-up mode."""


@dataclass
class DecodedFrame:
    payload: bytes | None
    frame: ReceivedFrame | None
    """An OFDM frame's soft information; ``None`` for a tone-floor frame."""
    mode: Mode | None
    """An OFDM frame's mode; ``None`` for a tone-floor frame."""
    tone: ToneFrame | None = None
    """A tone-floor frame's soft information (ADR-0013)."""
    tone_sync: ToneSync | None = None

    @property
    def ok(self) -> bool:
        return self.payload is not None

    @property
    def start(self) -> int:
        if self.tone_sync is not None:
            return self.tone_sync.start
        assert self.frame is not None
        return self.frame.sync.start


class Modem:
    """Stateless-ish TX/RX front for one waveform (codecs are cached per mode/layout)."""

    def __init__(
        self,
        params: WaveformParams = WIDE_2300,
        blank_impulses: bool = True,
        blanker: NoiseBlanker | None = None,
    ) -> None:
        self.p = params
        self.air = air_interface(params)
        """The layouts and modes of this waveform (the wide or the narrow table)."""
        self.tx = FrameTransmitter(params)
        self.detector = FrameDetector(params)
        self.rx = FrameReceiver(params)
        self.tone_detector = ToneDetector((self.air.tone_control, *self.air.tone_data))
        self.blanker = (blanker or NoiseBlanker()) if blank_impulses else None
        """Impulse blanker run ahead of band-limiting (P2-5). On by default: measured to cost
        a clean channel nothing at any mode while removing impulsive noise that otherwise
        takes the link to 100 % frame errors."""
        self._codecs: dict[tuple[int, str, int], FrameCodec] = {}

    def codec(self, mode: Mode, layout: FrameLayout) -> FrameCodec:
        key = (mode.index, layout.name, layout.waveform.bandwidth.value)
        if key not in self._codecs:
            self._codecs[key] = FrameCodec(mode, layout)
        return self._codecs[key]

    # ── transmit ──────────────────────────────────────────────────────

    @property
    def modes(self) -> tuple[Mode, ...]:
        return self.air.modes

    def data_burst(self, payload: bytes, mode: Mode, rv: int = 0) -> ComplexArray:
        """An OFDM DATA frame at ``mode`` on the LONG layout."""
        layout = self.air.long
        codec = self.codec(mode, layout)
        return self.tx.baseband(
            FrameHeader(FrameType.DATA, mode.index, rv), layout, codec.encode(payload, rv)
        )

    def rung_burst(self, payload: bytes, rung: int, rv: int = 0) -> ComplexArray:
        """A DATA frame at a rung of the ladder: a tone-floor kind or an OFDM mode."""
        r = self.air.ladder[rung]
        if r.tone is not None:
            return burst(r.tone, payload, rv)
        assert r.mode is not None
        return self.data_burst(payload, r.mode, rv)

    def rung_payload_bytes(self, rung: int) -> int:
        return self.air.ladder[rung].payload_bytes

    def control_burst(self, payload: bytes, rv: int = 0, floor: bool = False) -> ComplexArray:
        """A control frame: SHORT at the control mode, or — while the link runs the floor —
        the tone floor's control frame (ADR-0013)."""
        if floor:
            kind = self.air.tone_control
            return burst(kind, payload.ljust(kind.payload_bytes, b"\0"), 0)
        layout = self.air.short
        codec = self.codec(self.air.control_mode, layout)
        payload = payload.ljust(codec.payload_bytes, b"\0")
        return self.tx.baseband(FrameHeader(FrameType.CONTROL), layout, codec.encode(payload, rv))

    def audio(self, baseband: ComplexArray, level: float = 0.25) -> NDArray[np.float32]:
        """48 kHz float32 audio of a baseband burst at RMS ``level`` (default −12 dBFS)."""
        return (self.tx.audio(baseband) * level).astype(np.float32)

    def payload_bytes(self, mode: Mode | None = None) -> int:
        """Payload bytes of an OFDM DATA frame at ``mode``, or of an ordinary control frame."""
        return (
            self.codec(mode, self.air.long).payload_bytes
            if mode
            else self.codec(self.air.control_mode, self.air.short).payload_bytes
        )

    # ── receive ───────────────────────────────────────────────────────

    def demodulate(self, x: ComplexArray, sync: FrameSync) -> ReceivedFrame:
        """Equalized symbols plus the (mode, rv) read from the chips — the soft frame a link
        layer combines across retransmissions. Retries the runner-up chip hypothesis only
        through :meth:`decode_sync`."""
        return self.rx.receive(x, sync)

    def decode_frame(
        self, frame: ReceivedFrame, buffer: FloatArray | None = None
    ) -> tuple[bytes | None, FloatArray]:
        """Decode a demodulated frame with the RV it announced, optionally soft-combining
        with ``buffer`` (HARQ-IR). Returns ``(payload or None, llr buffer)``."""
        control = frame.sync.header.frame_type is FrameType.CONTROL
        mode = self.air.control_mode if control else self.air.modes[frame.mode]
        return self.codec(mode, frame.layout).decode(
            frame.symbols, frame.noise_var, rv=frame.rv, buffer=buffer
        )

    def decode_sync(
        self, x: ComplexArray, sync: FrameSync, buffer: FloatArray | None = None
    ) -> DecodedFrame:
        frame = self.rx.receive(x, sync)
        if sync.header.frame_type is FrameType.CONTROL:
            payload, _ = self.decode_frame(frame, buffer)
            return DecodedFrame(payload, frame, self.air.control_mode)
        mode = self.air.modes[frame.mode]
        payload, _ = self.decode_frame(frame, buffer)
        if payload is None and frame.mode_confidence < MODE_RETRY_CONFIDENCE:
            # the chip metric was close: try the runner-up (mode, rv) before giving up
            alt = self.rx.receive(x, sync, hypothesis=frame.chip_runner_up)
            payload, _ = self.decode_frame(alt, buffer)
            if payload is not None:
                return DecodedFrame(payload, alt, self.air.modes[alt.mode])
        return DecodedFrame(payload, frame, mode)

    def decode_tone(self, x: ComplexArray, sync: ToneSync) -> DecodedFrame:
        """Demodulate and decode a tone-floor frame found at ``sync``."""
        frame = demodulate(x, sync.kind, sync.rv, sync.start, sync.cfo_hz)
        payload, _ = frame.decode()
        return DecodedFrame(payload, None, None, frame, sync)

    def decode_buffer(self, x: ComplexArray, max_frames: int = 4) -> list[DecodedFrame]:
        """Blank impulses, band-limit, detect and decode every frame of either family in a
        baseband buffer, in the order they start."""
        if self.blanker is not None:
            x = self.blanker.process(x).samples
        y = self.detector.condition(x)
        out: list[DecodedFrame] = []
        for sync in self.detector.detect(y, max_frames=max_frames):
            try:
                out.append(self.decode_sync(y, sync))
            except (ValueError, IndexError):
                continue
        for ts in self.tone_detector.detect(y, max_frames=max_frames):
            out.append(self.decode_tone(y, ts))
        return sorted(out, key=lambda d: d.start)[:max_frames]
