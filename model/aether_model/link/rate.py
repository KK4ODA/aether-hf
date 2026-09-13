"""Receiver-side rate control (roadmap P2-1 baseline, hysteresis and benchmarks in P2-2).

The receiving station knows two things the sender does not: the SNR it measures on every
frame and whether those frames decoded. It turns that into a mode recommendation with three
nested loops:

* **inner loop** — the fastest mode whose measured AWGN threshold, plus a margin, fits under
  the smoothed SNR. Thresholds come from the PHY sweep (``bench/baselines/phy_fer.csv``);
* **outer loop** — the margin itself adapts: every burst with a failure widens it, every
  clean burst narrows it slowly. A fading channel, whose frame errors the AWGN table cannot
  predict, settles at a wider margin on its own without anyone naming the propagation model;
* **hysteresis** — the decision is a state machine, not a lookup. Stepping *up* needs a
  clean burst and an extra ``up_hysteresis_db`` of headroom above the next mode's threshold;
  stepping *down* happens immediately on any failed frame. Fast down, slow up: the classic
  asymmetry, and what stops a link parked on a mode boundary from oscillating between two
  modes and losing a burst to each change.

Modes that another mode beats on *both* throughput and threshold are never recommended.
"""

from __future__ import annotations

from dataclasses import dataclass, field

AWGN_THRESHOLD_DB: dict[int, float] = {
    0: -5.2,
    1: -3.6,
    2: -2.0,
    3: -0.5,
    4: 1.0,
    5: 2.8,
    6: 4.4,
    7: 7.5,
    8: 6.0,
    9: 8.0,
    10: 9.9,
    11: 12.5,
    12: 14.5,
    13: 16.9,
}
"""Minimum usable SNR (3 kHz, FER ≤ 10 %) per mode on AWGN. Modes 0, 2, 4, 6, 8, 10 and 13
are measured (``bench/baselines/phy_fer.csv``, P2-3 air interface); the rest are interpolated
until a sweep covers them."""

PAYLOAD_BYTES: dict[int, float] = {
    0: 26,
    1: 46,
    2: 70,
    3: 95,
    4: 144,
    5: 193,
    6: 217,
    7: 291,
    8: 291,
    9: 389,
    10: 438,
    11: 585,
    12: 658,
    13: 732,
}


def usable_modes(
    thresholds: dict[int, float] = AWGN_THRESHOLD_DB, payload: dict[int, float] = PAYLOAD_BYTES
) -> list[int]:
    """Modes on the throughput/threshold Pareto front, ascending."""
    out = []
    for m, thr in thresholds.items():
        dominated = any(
            payload[o] >= payload[m] and thresholds[o] <= thr and o != m for o in thresholds
        )
        if not dominated:
            out.append(m)
    return sorted(out)


@dataclass
class RateController:
    margin_db: float = 3.0
    min_margin_db: float = 1.5
    max_margin_db: float = 12.0
    up_step_db: float = 1.5
    """Margin increase per burst that had a decoding failure."""
    down_step_db: float = 0.25
    """Margin decrease per clean burst."""
    up_hysteresis_db: float = 1.5
    """Extra headroom demanded before stepping up, on top of the margin. This is the whole
    anti-oscillation mechanism: a mode entered at SNR ``x`` is only left upward at
    ``thr(next) + margin + up_hysteresis``, and downward only when frames actually fail."""
    up_dwell: int = 1
    """Consecutive clean bursts required before any upshift."""
    max_up_step: int = 2
    """Most modes to climb at once, so a link never leaps onto an untried mode."""
    thresholds: dict[int, float] = field(default_factory=lambda: dict(AWGN_THRESHOLD_DB))
    modes: list[int] = field(default_factory=usable_modes)
    snr_db: float | None = None
    _smoothed: float | None = None
    _index: int = 0
    """Position in :attr:`modes` — the recommendation is ``modes[_index]``."""
    _clean_run: int = 0

    def __post_init__(self) -> None:
        self._index = min(self._index, len(self.modes) - 1)

    # ── inputs ────────────────────────────────────────────────────────

    def observe(self, snr_db: float | None, ok: int, failed: int) -> None:
        """Feed one burst: mean SNR of its frames and how many decoded / failed."""
        if snr_db is not None:
            self._smoothed = (
                snr_db if self._smoothed is None else 0.7 * self._smoothed + 0.3 * snr_db
            )
            self.snr_db = self._smoothed
        if failed:
            self.margin_db = min(self.max_margin_db, self.margin_db + self.up_step_db)
            self._clean_run = 0
            self._step_down()
        elif ok:
            self.margin_db = max(self.min_margin_db, self.margin_db - self.down_step_db)
            self._clean_run += 1
            self._step_up()

    def recommend(self) -> int:
        return self.modes[self._index]

    # ── the state machine ─────────────────────────────────────────────

    def _fits(self, index: int, extra_db: float = 0.0) -> bool:
        if self.snr_db is None:
            return index == 0
        return self.thresholds[self.modes[index]] + self.margin_db + extra_db <= self.snr_db

    def _step_down(self) -> None:
        """A failure: fall to the fastest mode the current SNR and margin still support, but
        always at least one step — a failure at the bottom of the table is still evidence."""
        target = self._index - 1
        for i in range(self._index - 1, -1, -1):
            if self._fits(i):
                target = i
                break
        self._index = max(0, target)

    def _step_up(self) -> None:
        if self._clean_run < self.up_dwell:
            return
        target = self._index
        for i in range(self._index + 1, len(self.modes)):
            if not self._fits(i, self.up_hysteresis_db):
                break
            target = i
        if target > self._index:
            self._index = min(target, self._index + self.max_up_step)
            self._clean_run = 0
