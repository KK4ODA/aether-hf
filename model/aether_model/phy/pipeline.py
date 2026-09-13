"""Convenience end-to-end helpers: payload → burst and buffer → decoded frames (P1-7).

These tie the frame codec, transmitter, detector and receiver together for tests, the
benchmark runner and the (future) link layer. They are deliberately thin.
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from aether_model.frame.codec import FrameCodec
from aether_model.frame.modes import CONTROL_MODE, LONG, MODES, SHORT, FrameLayout, Mode
from aether_model.phy.preamble import FrameHeader, FrameType
from aether_model.phy.rx import FrameReceiver, ReceivedFrame, layout_for
from aether_model.phy.sync import FrameDetector, FrameSync
from aether_model.phy.tx import FrameTransmitter
from aether_model.waveform import WIDE_2300, WaveformParams

ComplexArray = NDArray[np.complex128]
FloatArray = NDArray[np.float64]

MODE_RETRY_CONFIDENCE = 1.3
"""Below this chip-metric ratio a CRC failure triggers a retry with the runner-up mode."""


@dataclass
class DecodedFrame:
    payload: bytes | None
    frame: ReceivedFrame
    mode: Mode

    @property
    def ok(self) -> bool:
        return self.payload is not None


class Modem:
    """Stateless-ish TX/RX front for one waveform (codecs are cached per mode/layout)."""

    def __init__(self, params: WaveformParams = WIDE_2300) -> None:
        self.p = params
        self.tx = FrameTransmitter(params)
        self.detector = FrameDetector(params)
        self.rx = FrameReceiver(params)
        self._codecs: dict[tuple[int, str], FrameCodec] = {}

    def codec(self, mode: Mode, layout: FrameLayout) -> FrameCodec:
        key = (mode.index, layout.name)
        if key not in self._codecs:
            self._codecs[key] = FrameCodec(mode, layout)
        return self._codecs[key]

    # ── transmit ──────────────────────────────────────────────────────

    def data_burst(self, payload: bytes, mode: Mode, rv: int = 0) -> ComplexArray:
        codec = self.codec(mode, LONG)
        return self.tx.baseband(
            FrameHeader(FrameType.DATA, mode.index), LONG, codec.encode(payload, rv)
        )

    def control_burst(self, payload: bytes, rv: int = 0) -> ComplexArray:
        codec = self.codec(CONTROL_MODE, SHORT)
        return self.tx.baseband(FrameHeader(FrameType.CONTROL), SHORT, codec.encode(payload, rv))

    def audio(self, baseband: ComplexArray, level: float = 0.25) -> NDArray[np.float32]:
        """48 kHz float32 audio of a baseband burst at RMS ``level`` (default −12 dBFS)."""
        return (self.tx.audio(baseband) * level).astype(np.float32)

    def payload_bytes(self, mode: Mode | None = None) -> int:
        return (
            self.codec(mode, LONG).payload_bytes
            if mode
            else self.codec(CONTROL_MODE, SHORT).payload_bytes
        )

    # ── receive ───────────────────────────────────────────────────────

    def decode_sync(
        self, x: ComplexArray, sync: FrameSync, rv: int = 0, buffer: FloatArray | None = None
    ) -> DecodedFrame:
        frame = self.rx.receive(x, sync)
        layout = layout_for(sync.header.frame_type)
        if sync.header.frame_type is FrameType.CONTROL:
            codec = self.codec(CONTROL_MODE, layout)
            payload, _ = codec.decode(frame.symbols, frame.noise_var, rv=rv, buffer=buffer)
            return DecodedFrame(payload, frame, CONTROL_MODE)
        mode = MODES[frame.mode]
        payload, _ = self.codec(mode, layout).decode(
            frame.symbols, frame.noise_var, rv=rv, buffer=buffer
        )
        if payload is None and frame.mode_confidence < MODE_RETRY_CONFIDENCE:
            # the chip metric was close: try the runner-up mode before giving up
            alt = self.rx.receive(x, sync, mode=frame.mode_runner_up)
            alt_mode = MODES[alt.mode]
            payload, _ = self.codec(alt_mode, layout).decode(
                alt.symbols, alt.noise_var, rv=rv, buffer=buffer
            )
            if payload is not None:
                return DecodedFrame(payload, alt, alt_mode)
        return DecodedFrame(payload, frame, mode)

    def decode_buffer(self, x: ComplexArray, max_frames: int = 4) -> list[DecodedFrame]:
        """Band-limit, detect and decode every frame in a baseband buffer."""
        y = self.detector.condition(x)
        out = []
        for sync in self.detector.detect(y, max_frames=max_frames):
            try:
                out.append(self.decode_sync(y, sync))
            except (ValueError, IndexError):
                continue
        return out
