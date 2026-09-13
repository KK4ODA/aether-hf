"""What the link layer needs from a PHY — and nothing more (P2-1).

* :class:`TxFrame` — what the engine asks the PHY to send.
* :class:`SoftFrame` — what the PHY hands the engine for every frame it *detected*, whether
  or not the payload decoded. Decoding is the engine's call because it owns the HARQ
  buffers: it decides which earlier transmission a failed frame should be combined with.
* :class:`PhyTiming` — the durations the engine's timers are built from.

The HARQ buffer is opaque to the engine (``object``); the real PHY passes full-codeword
LLR arrays, the lossy-pipe simulator passes accumulated "energy".
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum
from typing import Protocol


class Container(Enum):
    DATA = 0
    CONTROL = 1


@dataclass(frozen=True)
class TxFrame:
    container: Container
    payload: bytes
    mode: int = 0
    rv: int = 0


class SoftFrame(Protocol):
    container: Container
    mode: int
    rv: int
    snr_db: float
    """Estimated SNR (3 kHz reference) of this frame, for rate control."""
    t_start: float
    t_end: float
    """Air time of the frame in the receiver's clock (seconds)."""

    def decode(self, buffer: object | None = None) -> tuple[bytes | None, object]:
        """Payload (or ``None`` on CRC failure) and the HARQ buffer to keep for this
        sequence number. Combining with ``buffer`` must never mutate it."""
        ...


@dataclass(frozen=True)
class PhyTiming:
    data_frame_s: float
    control_frame_s: float
    turnaround_s: float = 0.25
    """Guard from the end of a received burst to keying up (PTT, audio latency, RX flush)."""
    detect_latency_s: float = 0.15
    """Worst-case delay from a frame's last sample to the engine hearing about it."""
    data_capacity: dict[int, int] | None = None
    """PHY payload bytes per DATA-container mode index (mode → bytes)."""

    def capacity(self, mode: int) -> int:
        if self.data_capacity is None:
            raise ValueError("PhyTiming.data_capacity not set")
        return self.data_capacity[mode]
