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
    floor: bool = False
    """A CONTROL frame to go out on the floor — the tone floor's control frame (ADR-0013).
    A DATA frame's family follows its mode; this flag is only read for control frames."""


class SoftFrame(Protocol):
    container: Container
    mode: int
    rv: int
    snr_db: float
    """Estimated SNR (3 kHz reference) of this frame, for rate control."""
    t_start: float
    t_end: float
    floor: bool
    """The frame is the floor's (the tone floor, ADR-0013). A DATA frame's family is also
    its mode's; for a control frame this is the only way the engine learns it."""
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
    tx_latency_s: float = 0.0
    """Delay from the engine asking for a transmission to its first sample leaving the
    antenna: the keying lead, and the audio the daemon keeps queued ahead of the sound card.
    Zero for a simulator that plays what it is handed at once; a real station's is a few
    hundred milliseconds, and an engine that does not know it under-waits for every reply
    by that much. Found by two real daemons over a socket, not by the simulator."""
    preamble_detect_s: float | None = None
    """Delay from a frame's *first* sample to the PHY reporting its preamble via
    :meth:`~aether_model.link.engine.LinkEngine.on_preamble`. When a PHY provides that
    signal the receiver learns a burst is continuing this quickly; when it is ``None`` the
    receiver has to wait a whole data frame of silence instead to be sure a burst has
    ended, which costs roughly a quarter of the air time (see ``bench/README.md``)."""
    data_capacity: dict[int, int] | None = None
    """PHY payload bytes per DATA-container mode index (mode → bytes)."""
    mode_threshold_db: dict[int, float] | None = None
    """Minimum usable SNR (3 kHz, AWGN) per mode index, as the PHY's own benchmark measured
    it — what the rate controller steps along. ``None`` means the wide waveform's table
    (:data:`~aether_model.link.rate.AWGN_THRESHOLD_DB`); a PHY with another mode table —
    the 500 Hz waveform, P7-0 — hands its own here, and the engine never knows which air
    it is on."""

    floor_data_frame_s: float | None = None
    """Air time of a DATA frame at a floor mode — the tone floor's frame (ADR-0013), five
    times an ordinary one. ``None`` on an air without a floor family."""
    floor_control_frame_s: float | None = None
    """Air time of the floor's control frame."""
    floor_modes: int = 0
    """How many of the leading modes are the floor's (the slowest ones)."""
    control_threshold_db: dict[bool, float] | None = None
    """The AWGN 10 % points of the air's two control frames, keyed by family (ordinary,
    floor) — what a simulated channel judges a control frame by. ``None``: the wide air's
    (:data:`~aether_model.link.rate.CONTROL_THRESHOLD_DB`)."""
    floor_margin_db: float | None = None
    """The most margin the rate controller holds the first OFDM rung to against the floor
    (:attr:`~aether_model.link.rate.RateController.floor_margin_db`): an air whose first rung
    stays productive on a fading path below the learned margin says how far; ``None`` leaves
    the learned margin in charge."""
    floor_preamble_detect_s: float | None = None
    """:attr:`preamble_detect_s` for the floor's frames: the tone floor announces a frame
    once its first sync block is in and has beaten its neighbours, 0.54 s after it starts,
    where an ordinary preamble takes 0.12 s. ``None``: as :attr:`preamble_detect_s`."""

    def capacity(self, mode: int) -> int:
        if self.data_capacity is None:
            raise ValueError("PhyTiming.data_capacity not set")
        return self.data_capacity[mode]

    def is_floor(self, mode: int) -> bool:
        return mode < self.floor_modes

    def data_frame_s_for(self, mode: int) -> float:
        """Air time of a DATA frame at ``mode``."""
        if self.is_floor(mode) and self.floor_data_frame_s is not None:
            return self.floor_data_frame_s
        return self.data_frame_s

    def preamble_detect_s_for(self, floor: bool) -> float | None:
        """How soon a frame of the given family is announced (``None``: it is not)."""
        if floor and self.floor_preamble_detect_s is not None:
            return self.floor_preamble_detect_s
        return self.preamble_detect_s

    def control_frame_s_for(self, floor: bool) -> float:
        """Air time of a control frame of the given family."""
        if floor and self.floor_control_frame_s is not None:
            return self.floor_control_frame_s
        return self.control_frame_s

    def frame_s(self, frame: TxFrame) -> float:
        """Air time of a frame the engine is about to send."""
        if frame.container is Container.DATA:
            return self.data_frame_s_for(frame.mode)
        return self.control_frame_s_for(frame.floor)
