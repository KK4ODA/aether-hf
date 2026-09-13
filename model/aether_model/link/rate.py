"""Receiver-side rate recommendation (P2-1 baseline; hysteresis and benchmarks are P2-2).

The receiving station knows two things the sender does not: the SNR it measures on every
frame and whether the frames decoded. The recommendation is *inner loop + outer loop*:

* inner loop: the fastest mode whose AWGN threshold plus a margin is below the measured
  SNR (thresholds from the Phase 1 sweep, FER ≈ 10 %);
* outer loop: the margin adapts to what actually happens — every burst with failures
  raises it, every clean burst lowers it a little. Fading channels, whose frame-error
  behaviour the AWGN table cannot predict, settle at a larger margin by themselves.

Modes that another mode beats on both throughput and threshold are never recommended.
"""

from __future__ import annotations

from dataclasses import dataclass, field

AWGN_THRESHOLD_DB: dict[int, float] = {
    0: -3.3,
    1: -2.4,
    2: -1.3,
    3: -0.2,
    4: 1.0,
    5: 3.0,
    6: 4.4,
    7: 7.5,
    8: 6.2,
    9: 8.2,
    10: 9.9,
    11: 12.5,
    12: 14.5,
    13: 16.9,
}
"""Minimum usable SNR (3 kHz, FER ≤ 10 %) per mode on AWGN. Measured entries come from
``bench/baselines/phy_fer_phase1_uw.csv`` (modes 0, 2, 4, 6, 8, 10, 13); the others are
interpolated until the Phase 2 sweep replaces them."""

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
    thresholds: dict[int, float] = field(default_factory=lambda: dict(AWGN_THRESHOLD_DB))
    modes: list[int] = field(default_factory=usable_modes)
    snr_db: float | None = None
    _smoothed: float | None = None

    def observe(self, snr_db: float | None, ok: int, failed: int) -> None:
        """Feed one burst: mean SNR of its frames and how many decoded / failed."""
        if snr_db is not None:
            self._smoothed = (
                snr_db if self._smoothed is None else 0.7 * self._smoothed + 0.3 * snr_db
            )
            self.snr_db = self._smoothed
        if failed:
            self.margin_db = min(self.max_margin_db, self.margin_db + self.up_step_db)
        elif ok:
            self.margin_db = max(self.min_margin_db, self.margin_db - self.down_step_db)

    def recommend(self) -> int:
        if self.snr_db is None:
            return self.modes[0]
        best = self.modes[0]
        for m in self.modes:
            if self.thresholds[m] + self.margin_db <= self.snr_db:
                best = m
        return best
