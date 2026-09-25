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
    0: -19.0,
    1: -17.3,
    2: -16.0,
    3: -14.2,
    4: -13.1,
    5: -11.2,
    6: -5.1,
    7: -3.2,
    8: -1.8,
    9: -0.4,
    10: 1.4,
    11: 2.9,
    12: 4.7,
    13: 6.9,
    14: 6.0,
    15: 8.9,
    16: 9.9,
    17: 13.9,
    18: 15.6,
    19: 16.9,
}
"""Minimum usable SNR (3 kHz, FER ≤ 10 %) per rung of the 2 300 Hz ladder on AWGN. **Every
rung is measured**: the tone floor's six (rungs 0–5: its own two, ADR-0013, and the fast
kinds, ADR-0014) by ``tools/bench_tone.py`` (``bench/baselines/tone_floor.csv``), at equal
peak power and so in the OFDM frames' reference; the OFDM modes (rungs 6–19, OFDM modes
0–13) by ``bench_phy.py`` (``bench/baselines/phy_fer_awgn14.csv``, the current air
interface including ADR-0004 peak reduction) — regenerate with
``tools/update_rate_table.py --apply``. The interpolated guesses the OFDM entries replaced
were optimistic by up to 1.4 dB on the 64-QAM modes, which the rate controller had no way to
discover except by losing frames."""

PAYLOAD_BYTES: dict[int, float] = {
    0: 24,
    1: 36,
    2: 51,
    3: 75,
    4: 105,
    5: 153,
    6: 26,
    7: 46,
    8: 70,
    9: 95,
    10: 144,
    11: 193,
    12: 217,
    13: 291,
    14: 291,
    15: 389,
    16: 438,
    17: 585,
    18: 658,
    19: 732,
}
"""Payload bytes per frame of each rung of the 2 300 Hz ladder."""

FRAME_S: dict[int, float] = {m: (5.36 if m < 6 else 1.054) for m in range(20)}
"""Air time of each wide rung's DATA frame: 134 slots of 40 ms on the tone floor, its fast
kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy of the
frames; tested against them) — what :func:`usable_modes` needs to compare the floor's long
frames with the ordinary ones."""


NARROW_AWGN_THRESHOLD_DB: dict[int, float] = {
    0: -19.0,
    1: -17.3,
    2: -14.3,
    3: -13.0,
    4: -6.0,
    5: -5.2,
    6: -3.6,
    7: -2.1,
    8: 0.5,
    9: -0.1,
    10: 2.0,
    11: 3.5,
    12: 6.9,
    13: 8.8,
    14: 10.4,
}
"""The 500 Hz waveform's table (P7-0), 3 kHz-referenced like the wide one, so the two read
as an operator would compare them: the same transmitter power into the same noise. Every
entry is measured: the tone floor's four (rungs 0–3: its own two, ADR-0013, the same frames
and thresholds as on the wide ladder, and the four-tone middle kinds, ADR-0015) by
``tools/bench_tone.py`` (``bench/baselines/tone_floor.csv``); the OFDM modes (rungs 4–14,
OFDM modes 2–12) by ``tools/bench_phy.py --bandwidth 500`` (``bench/baselines/
phy_fer_500.csv``; written by ``tools/update_rate_table.py --bandwidth 500 --apply``). Its
control mode, QPSK ½ (rung 5), sits at −5.2 dB against the wide table's BPSK ⅕ at −5.1
because twelve carriers carry ≈ 6.8 dB more per carrier than fifty-seven; below it is QPSK ⅓
on the ordinary frame (rung 4), and below that the tone floor, which reaches −19 dB where the
OFDM floor it replaced (ADR-0009) reached −12."""

TONE_CONTROL_THRESHOLD_DB = -19.5
"""The tone floor's control frame's 10 % FER point on AWGN (ADR-0013,
``bench/baselines/tone_floor.csv``) — the same frame on both airs."""

CONTROL_THRESHOLD_DB: dict[bool, float] = {False: -5.1, True: TONE_CONTROL_THRESHOLD_DB}
"""The 2 300 Hz control frames' 10 % FER points on AWGN, keyed by family: the control mode
on the SHORT layout, and the tone floor's control frame — what the lossy pipe
(:mod:`aether_model.link.sim`) judges a control frame by; the rate controller never reads
it. ``tools/bench_floor.py --bandwidth 2300`` measures the ordinary one
(``bench/baselines/floor_2300.csv``)."""

NARROW_CONTROL_THRESHOLD_DB: dict[bool, float] = {False: -4.5, True: TONE_CONTROL_THRESHOLD_DB}
"""The two 500 Hz control frames' 10 % FER points on AWGN: the ordinary SHORT frame at the
control mode (``bench/baselines/floor_500.csv``), and the tone floor's. Until 2026-09-23
the pipe judged every control frame at data mode 0's threshold: an ordinary acknowledgement
that needs −4.5 dB went through the pipe eight decibels below where the modem could decode
it."""

NARROW_PAYLOAD_BYTES: dict[int, float] = {
    0: 24,
    1: 36,
    2: 51,
    3: 75,
    4: 15,
    5: 25,
    6: 34,
    7: 39,
    8: 53,
    9: 53,
    10: 71,
    11: 81,
    12: 109,
    13: 123,
    14: 137,
}
"""Payload bytes per frame of each rung of the 500 Hz ladder: the tone floor's frames are
five times as long as the ordinary ones, which is why :func:`usable_modes` needs
:data:`NARROW_FRAME_S` to compare them."""

NARROW_FRAME_S: dict[int, float] = {m: (5.36 if m < 4 else 1.054) for m in range(15)}
"""Air time of each narrow rung's DATA frame: 134 slots of 40 ms on the tone floor, its
middle kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy of
the frames; tested against them)."""


def usable_modes(
    thresholds: Mapping[int, float] = AWGN_THRESHOLD_DB,
    payload: Mapping[int, float] = PAYLOAD_BYTES,
    frame_s: Mapping[int, float] | None = FRAME_S,
) -> list[int]:
    """Modes on the throughput/threshold Pareto front, ascending. ``payload`` is bytes per
    frame; given ``frame_s`` (air time per mode) the comparison is bytes per second, which
    is what tells the floor's long frames from the ordinary ones."""

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
    reseed_margin_db: float = 3.0
    """How far above a lower-bound seed (:meth:`seed`) the first ordinary measurement must read
    to replace it. The tone floor's estimate is exact to about +10 dB on AWGN; a few decibels
    between two frames' readings is their estimates' own spread and the fade between them, and
    replacing a seed on that would only take the larger of two noisy numbers. Beyond it the
    seed was the floor's ceiling, not the path's."""
    floor_modes: int = 6
    """How many of the ladder's leading rungs are the floor's — the tone floor, ADR-0013, and
    on the 2 300 Hz air its fast kinds, ADR-0014 — whose frames are five times as long as an
    OFDM frame: see :meth:`first_mode` and :attr:`floor_margin_db`."""
    floor_margin_db: float | None = None
    """The most margin the first OFDM rung is held to against the floor, however wide the
    learned margin has grown — on an air whose first rung stays productive on a fading path
    below the margin the learned one would demand (``PhyTiming.floor_margin_db``). The
    learned margin prices a channel's fading among OFDM rungs a third apart in rate; the floor
    is a quarter of the rate, and a rung that loses half its frames but gets the rest through
    with HARQ is still worth twice the floor. The 2 300 Hz air's first rung spreads a frame
    over 2.3 kHz: on the fading bench, before the floor existed, it carried every session
    from a decibel above its 10 % point up on every class. The 500 Hz air's has a fifth of
    that diversity and does not; ``None``, the learned margin, is the rule there."""
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
    _boundary_failures: int = 0
    """Failed bursts in a row on the first OFDM rung (:meth:`_step_down`)."""
    _reseed_pending: bool = False
    """The seed was a lower bound (:meth:`seed`): the first clean burst measured on an ordinary
    frame seeds again."""

    def __post_init__(self) -> None:
        self._index = min(self._index, len(self.modes) - 1)

    # ── inputs ────────────────────────────────────────────────────────

    def observe(self, snr_db: float | None, ok: int, failed: int, mode: int | None = None) -> None:
        """Feed one burst: mean SNR of its frames, how many decoded / failed, and the mode
        they were sent in (which turns a failure into a measurement — see :meth:`_widen`)."""
        if (
            self._reseed_pending
            and snr_db is not None
            and ok
            and not failed
            and mode is not None
            and mode >= self.floor_modes
        ):
            # the first measurement a strong path can show: start again from it — upward, and
            # only past the estimates' own spread — as an ordinary connect frame would have
            self._reseed_pending = False
            if self._smoothed is None or snr_db > self._smoothed + self.reseed_margin_db:
                self._smoothed = self.snr_db = snr_db
                self._index = max(self._index, self.modes.index(self.first_mode(snr_db)))
                return
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
            self._boundary_failures = 0
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

    def observe_snr(self, snr_db: float | None) -> None:
        """Take a burst's SNR and nothing else: for a burst sent faster than this station asked
        for, whose failure is the sender's choice and no news to this controller (ADR-0020) —
        the Test's ladder climbing past what the path carries, or a burst sent before the advice
        to go slower reached the sender. What decoded in it is still a measurement of the
        path."""
        if snr_db is None:
            return
        self._smoothed = snr_db if self._smoothed is None else 0.7 * self._smoothed + 0.3 * snr_db
        self.snr_db = self._smoothed

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
        ordinary = self._first_ordinary()
        fit = 0
        for i in range(1, len(self.modes)):
            if self.thresholds[self.modes[i]] + self._margin(i) + self.up_hysteresis_db > snr_db:
                break
            fit = i
        # the steps in hand stay in the family the measurement fits: a session the SNR puts
        # on an OFDM rung does not start on the floor, five times slower, for its caution
        lowest = ordinary if fit >= ordinary else 0
        return self.modes[max(lowest, fit - self.first_mode_back)]

    def _first_ordinary(self) -> int:
        """Index in :attr:`modes` of the first rung above the floor."""
        return next((i for i, m in enumerate(self.modes) if m >= self.floor_modes), 0)

    def seed(self, snr_db: float, *, lower_bound: bool = False) -> None:
        """Start from a measurement — the connect frame this station decoded — instead of
        from the slowest mode: the smoothed SNR becomes the measurement and the
        recommendation :meth:`first_mode`. Only before anything has been observed; a
        controller that has seen bursts knows more than one frame can tell it.

        ``lower_bound``: the measurement was a tone-floor frame's, which a strong path does
        not show — the floor's estimate is exact to about +10 dB on AWGN and saturates near
        +17, and on a dispersive path it reads a few decibels whatever the SNR, the echo's
        spill into the next symbol counting as noise (ADR-0016). Calls start on the floor, so
        a strong path's session would start many rungs low and climb two a burst; instead the
        first clean burst measured on an ordinary frame seeds again, upward only."""
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))
        self._reseed_pending = lower_bound

    # ── the state machine ─────────────────────────────────────────────

    def _margin(self, index: int) -> float:
        """The margin a rung is held to: the learned one, capped at :attr:`floor_margin_db`
        for the first OFDM rung, whose alternative is the floor."""
        if self.floor_margin_db is not None and index == self._first_ordinary():
            return min(self.margin_db, self.floor_margin_db)
        return self.margin_db

    def _fits(self, index: int, extra_db: float = 0.0) -> bool:
        if self.snr_db is None:
            return index == 0
        return self.thresholds[self.modes[index]] + self._margin(index) + extra_db <= self.snr_db

    def _step_down(self) -> None:
        """A failure: fall to the fastest mode the current SNR and margin still support, but
        always at least one step — a failure at the bottom of the table is still evidence.
        The one exception is a single failed burst on the first OFDM rung of an air that caps
        its margin (:attr:`floor_margin_db`) while the SNR still carries the rung: the floor
        below is a quarter of the rate, so one lost burst is not worth leaving for it; a
        second one in a row is."""
        ordinary = self._first_ordinary()
        at_boundary = self.floor_margin_db is not None and self._index == ordinary
        self._boundary_failures = self._boundary_failures + 1 if at_boundary else 0
        if at_boundary and self._boundary_failures < 2 and self._fits(ordinary):
            return
        self._boundary_failures = 0
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
