"""Candidate A, refined: ADR-0068 with six-frame bursts, the count saturating at three.

The amendment under discussion (2026-10-13): keep the stated answer, let a burst be six
frames, and let a count of 3 mean "three or more follow" — a frame that says 3 states no end,
the frames after it do. Nothing else changes.
"""

from __future__ import annotations

from dataclasses import replace

from aether_model.frame.modes import AirInterface  # noqa: F401  (re-exported for symmetry)
from aether_model.link.engine import LinkEngine
from aether_model.link.frames import MAX_BURST, MAX_FOLLOWS
from aether_model.link.phy import Container, SoftFrame, TxFrame


class Stated6Engine(LinkEngine):
    def _burst_capacity(self, frame_s: float, *, stated: bool = True) -> int:
        count = min(self.cfg.burst_frames, MAX_BURST)
        if self.cfg.max_burst_s is not None and frame_s > 0:
            count = min(count, max(1, int(self.cfg.max_burst_s / frame_s + 1e-9)))
        return count

    def _transmit(self, frames: list[TxFrame]) -> None:
        # two bits on the air: a count of three or more is sent as 3
        clamped = [
            replace(f, follows=min(f.follows, MAX_FOLLOWS)) if f.container is Container.DATA else f
            for f in frames
        ]
        super()._transmit(clamped)

    def _state_answer(
        self, frame: SoftFrame, follows: int | None = None, slot_s: float | None = None
    ) -> None:
        count = frame.follows if follows is None else follows
        if count is not None and count >= MAX_FOLLOWS and self.cfg.burst_frames > 4:
            return  # "three or more": no end stated; a later frame states it
        super()._state_answer(frame, follows, slot_s)
