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

from collections.abc import Mapping
from dataclasses import dataclass, field

AWGN_THRESHOLD_DB: dict[int, float] = {
    0: -5.1,
    1: -3.2,
    2: -1.8,
    3: -0.4,
    4: 1.4,
    5: 2.9,
    6: 4.7,
    7: 6.9,
    8: 6.0,
    9: 8.9,
    10: 9.9,
    11: 13.9,
    12: 15.6,
    13: 16.9,
}
"""Minimum usable SNR (3 kHz, FER ≤ 10 %) per mode on AWGN. **Every mode is measured** —
``bench/baselines/phy_fer_awgn14.csv``, current air interface including ADR-0004 peak
reduction — regenerate with ``tools/update_rate_table.py --apply``. The interpolated guesses
this replaced were optimistic by up to 1.4 dB on the 64-QAM modes, which the rate controller
had no way to discover except by losing frames."""

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


NARROW_AWGN_THRESHOLD_DB: dict[int, float] = {
    0: -12.4,
    1: -10.2,
    2: -6.0,
    3: -5.2,
    4: -3.6,
    5: -2.1,
    6: 0.5,
    7: -0.1,
    8: 2.0,
    9: 3.5,
    10: 6.9,
    11: 8.8,
    12: 10.4,
}
"""The 500 Hz waveform's table (P7-0), 3 kHz-referenced like the wide one, so the two read
as an operator would compare them: the same transmitter power into the same noise. Every
entry is measured (``bench/baselines/phy_fer_500.csv``, ``tools/bench_phy.py --bandwidth
500``; written by ``tools/update_rate_table.py --bandwidth 500 --apply``). Its control
mode, QPSK ½ (mode 3), sits at −5.2 dB against the wide table's BPSK ⅕ at −5.1 because
twelve carriers carry ≈ 6.8 dB more per carrier than fifty-seven; below it the floor
family (ADR-0009) — modes 0 and 1 on a frame four times as long, and mode 2 on the
ordinary one — reaches −12 dB."""

NARROW_PAYLOAD_BYTES: dict[int, float] = {
    0: 19,
    1: 41,
    2: 15,
    3: 25,
    4: 34,
    5: 39,
    6: 53,
    7: 53,
    8: 71,
    9: 81,
    10: 109,
    11: 123,
    12: 137,
}
"""Payload bytes per frame of each narrow mode *on the layout it goes out on*: the floor
modes' frames are four times as long, which is why :func:`usable_modes` needs
:data:`NARROW_FRAME_S` to compare them."""

NARROW_FRAME_S: dict[int, float] = {m: (4.216 if m < 2 else 1.054) for m in range(13)}
"""Air time of each narrow mode's DATA frame: 136 symbols on the floor layout, 34 on the
ordinary one, at 31 ms a symbol (the link layer's copy of the layouts; tested against
them)."""


def usable_modes(
    thresholds: Mapping[int, float] = AWGN_THRESHOLD_DB,
    payload: Mapping[int, float] = PAYLOAD_BYTES,
    frame_s: Mapping[int, float] | None = None,
) -> list[int]:
    """Modes on the throughput/threshold Pareto front, ascending. ``payload`` is bytes per
    frame; given ``frame_s`` (air time per mode) the comparison is bytes per second, which
    is what tells a floor mode's long frame from an ordinary one (ADR-0009)."""

    def worth(m: int) -> float:
        return payload[m] / (frame_s[m] if frame_s is not None else 1.0)

    out = []
    for m, thr in thresholds.items():
        dominated = any(
            worth(o) >= worth(m) and thresholds[o] <= thr and o != m for o in thresholds
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
    """Margin increase per failed burst when the mode that failed is not reported."""
    max_jump_db: float = 3.0
    """Cap on a single targeted margin increase, so one deep fade cannot slam the link to
    its slowest mode and strand it there."""
    down_step_db: float = 0.25
    """Margin decrease at the first decay step after a failure, applied once
    :attr:`decay_every` clean bursts have gone by."""
    decay_growth: float = 2.0
    """How much each further clean burst's decay step grows on the last (1.0 keeps the
    step fixed and the old every-``decay_every`` cadence): once the stickiness has run
    its course the margin comes back at an accelerating rate, capped at
    :attr:`max_down_step_db`. A learned penalty is given up slowly at first and quickly
    once clean burst follows clean burst, which is what a fade that has passed — or a
    collision that was never a fade — looks like. Found on the VarAC bench: one lost
    burst cost eighteen bursts at the mode below the one the SNR carried."""
    max_down_step_db: float = 1.0
    """Cap on a single decay step."""
    decay_every: int = 3
    """Clean bursts per step of margin decay, *once a failure has taught the margin
    something*. The margin encodes what this channel costs over AWGN — a property of the
    propagation, not of the last burst — so it is learned quickly from failures and given up
    slowly. Before the first failure there is nothing to protect and the margin decays on
    every clean burst, so a good link still reaches its mode within a few bursts."""
    up_hysteresis_db: float = 1.5
    """Extra headroom demanded before stepping up, on top of the margin. This is the whole
    anti-oscillation mechanism: a mode entered at SNR ``x`` is only left upward at
    ``thr(next) + margin + up_hysteresis``, and downward only when frames actually fail."""
    up_dwell: int = 1
    """Consecutive clean bursts required before any upshift."""
    max_up_step: int = 2
    """Most modes to climb at once, so a link never leaps onto an untried mode."""
    first_mode_back: int = 2
    """Steps kept in hand by :meth:`first_mode`: how far below the fastest mode one
    measurement supports a session's first burst goes out."""
    thresholds: dict[int, float] = field(default_factory=lambda: dict(AWGN_THRESHOLD_DB))
    modes: list[int] = field(default_factory=usable_modes)
    snr_db: float | None = None
    _smoothed: float | None = None
    _index: int = 0
    """Position in :attr:`modes` — the recommendation is ``modes[_index]``."""
    _clean_run: int = 0
    _clean_since_decay: int = 0
    _ever_failed: bool = False
    _decay_step_db: float = 0.0
    """The last decay step taken since the failure before, which the next one grows on."""

    def __post_init__(self) -> None:
        self._index = min(self._index, len(self.modes) - 1)

    # ── inputs ────────────────────────────────────────────────────────

    def observe(self, snr_db: float | None, ok: int, failed: int, mode: int | None = None) -> None:
        """Feed one burst: mean SNR of its frames, how many decoded / failed, and the mode
        they were sent in (which turns a failure into a measurement — see :meth:`_widen`)."""
        if snr_db is not None:
            self._smoothed = (
                snr_db if self._smoothed is None else 0.7 * self._smoothed + 0.3 * snr_db
            )
            self.snr_db = self._smoothed
        if failed:
            self._widen(snr_db, mode)
            self._ever_failed = True
            self._clean_run = 0
            self._clean_since_decay = 0
            self._decay_step_db = 0.0
            self._step_down()
        elif ok:
            self._clean_run += 1
            self._clean_since_decay += 1
            if not self._ever_failed:
                self.margin_db = max(self.min_margin_db, self.margin_db - self.down_step_db)
            elif self._decay_step_db > 0.0:
                # past the sticky bursts: every clean burst gives back more than the last
                self._decay_step_db = min(
                    self.max_down_step_db, self._decay_step_db * self.decay_growth
                )
                self.margin_db = max(self.min_margin_db, self.margin_db - self._decay_step_db)
            elif self._clean_since_decay >= self.decay_every:
                self._clean_since_decay = 0
                self._decay_step_db = self.down_step_db
                self.margin_db = max(self.min_margin_db, self.margin_db - self.down_step_db)
            self._step_up()

    def _widen(self, snr_db: float | None, mode: int | None) -> None:
        """A failed burst is a measurement, not just a nudge: mode ``m`` failing at SNR ``s``
        says this channel needs more than ``s − threshold[m]`` dB of margin. Jump most of the
        way there instead of creeping up in fixed steps — on a fading channel, where every
        mode costs 6–10 dB more than the AWGN table predicts, creeping means overshooting the
        mode for many bursts first. The jump is capped so one deep fade cannot strand the link.

        Every failure is a measurement, an isolated one included: reading the first as an
        accident (a collision, a missed preamble) and jumping only on a repeat was tried on
        the link bench and cost 11 % on the Moderate channel at 16 dB, where the climb it
        allowed ran into the fades — see ADR-0007."""
        target = self.margin_db + self.up_step_db
        if snr_db is not None and mode is not None and mode in self.thresholds:
            implied = snr_db - self.thresholds[mode] + self.up_step_db
            target = max(target, min(implied, self.margin_db + self.max_jump_db))
        self.margin_db = min(self.max_margin_db, max(self.min_margin_db, target))

    def recommend(self) -> int:
        return self.modes[self._index]

    # ── the faster start (P9-2) ───────────────────────────────────────

    def first_mode(self, snr_db: float) -> int:
        """The mode a session should start at, given one measurement and nothing else:
        the fastest mode that fits under ``snr_db`` with the margin and the hysteresis a
        step up would demand, less one step. The measurement is of a mode-0 frame — the
        most robust there is — and a burst at a fast mode is more exposed to what the
        channel does within a frame, so the first burst keeps :attr:`first_mode_back` steps in hand
        and the climb makes them up in a burst if the channel allows."""
        fit = 0
        for i in range(1, len(self.modes)):
            if self.thresholds[self.modes[i]] + self.margin_db + self.up_hysteresis_db > snr_db:
                break
            fit = i
        return self.modes[max(0, fit - self.first_mode_back)]

    def seed(self, snr_db: float) -> None:
        """Start from a measurement — the connect frame this station decoded — instead of
        from the slowest mode: the smoothed SNR becomes the measurement and the
        recommendation :meth:`first_mode`. Only before anything has been observed; a
        controller that has seen bursts knows more than one frame can tell it."""
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))

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
