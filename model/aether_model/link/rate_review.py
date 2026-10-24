"""The rate controller's review, Gate 3: prototypes beside :class:`~aether_model.link.rate.
RateController` (``docs/research/rate-control/03-candidates.md``).

Bench-only. :attr:`~aether_model.link.granted.GrantConfig.controller` selects one; a station
never runs one on the air. Each prototype answers what the engine asks of the current
controller — :meth:`recommend`, :meth:`first_mode`, :meth:`seed`, the echo, the thresholds —
and takes, in place of a burst's counts, what each data frame of it came to
(:class:`FrameOutcome`), frames never detected included: the job is the rung that delivers the
most data per second of air, and a frame never heard is air spent for nothing.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from itertools import pairwise

from aether_model.link.rate import AWGN_THRESHOLD_DB, RateController

# What each rung's 10 % point moves by on ITU-R F.1487's Good, Moderate and Poor channels, dB over
# its AWGN point (bench/baselines/phy_fer.csv, phy_fer_500.csv and tone_floor.csv through
# tools/bench_link.channel_thresholds, which fills a rung not swept from its family's mean): the
# prototypes' prior for a path not yet measured is the mean of the four channels' curves.
WIDE_CLASS_PENALTY_DB: dict[int, tuple[float, float, float]] = {
    0: (7.3, 5.8, 4.5),
    1: (8.5, 7.0, 6.4),
    2: (7.4, 4.8, 4.2),
    3: (8.5, 6.0, 6.7),
    4: (6.6, 4.9, 4.7),
    5: (8.0, 6.9, 7.1),
    6: (7.1, 6.3, 4.8),
    7: (7.4, 7.4, 5.0),
    8: (7.3, 6.6, 4.1),
    9: (7.4, 7.4, 5.0),
    10: (7.3, 7.1, 4.6),
    11: (7.4, 7.4, 5.0),
    12: (7.0, 7.6, 5.0),
    13: (7.4, 7.4, 5.0),
    14: (7.7, 6.7, 6.5),
    15: (7.4, 7.4, 5.0),
    16: (7.8, 9.9, 5.0),
    17: (7.4, 7.4, 5.0),
    18: (7.4, 7.4, 5.0),
    19: (7.9, 7.4, 5.0),
}
NARROW_CLASS_PENALTY_DB: dict[int, tuple[float, float, float]] = {
    0: (7.3, 5.8, 4.5),
    1: (8.5, 7.0, 6.4),
    2: (6.6, 4.1, 3.5),
    3: (9.4, 6.8, 5.8),
    4: (10.4, 8.9, 7.3),
    5: (9.2, 8.7, 5.2),
    6: (9.1, 8.6, 7.6),
    7: (9.1, 10.1, 6.6),
    8: (9.5, 9.8, 6.9),
    9: (9.1, 9.6, 8.1),
    10: (10.0, 12.0, 6.9),
    11: (10.5, 9.8, 6.9),
    12: (10.1, 9.8, 6.9),
    13: (9.2, 9.8, 6.9),
    14: (10.6, 9.8, 6.9),
}

CLASS_PENALTY_DB: dict[int, dict[int, tuple[float, float, float]]] = {
    len(WIDE_CLASS_PENALTY_DB): WIDE_CLASS_PENALTY_DB,
    len(NARROW_CLASS_PENALTY_DB): NARROW_CLASS_PENALTY_DB,
}
"""The table for a ladder, by its number of rungs (20 at 2 300 Hz, 15 at 500 Hz)."""


@dataclass(frozen=True)
class FrameOutcome:
    """One data frame of a transmission, as the receiving station saw it."""

    mode: int
    air_s: float
    """Its air time: a frame never detected took its slot all the same."""
    capacity: int
    """The data bytes a frame at this rung carries: what it is worth when it arrives."""
    detected: bool
    """Reported by the PHY at all; ``False`` for a frame the header counted and nothing heard."""
    decoded: bool
    """Delivered by this copy, alone or combined with earlier ones."""
    combined: bool
    """Combined with copies held: a rescue when :attr:`decoded`."""
    snr_db: float | None
    """Its measured SNR, when it decoded or the PHY trusts the measurement."""
    measure: bool = False
    """A measurement frame (ADR-0077): sent once, never combined."""


def decode_probability(snr_db: float, threshold_db: float, slope_db: float) -> float:
    """The prior's chance that a frame decodes at ``snr_db``: a logistic through 90 % at the
    rung's AWGN 10 % point, ``slope_db`` wide. On a fading path the real curve is shallower and
    further up; the prior is only where a rung not yet measured starts."""
    x = (snr_db - threshold_db) / slope_db
    if x < -40:
        return 0.0
    return 1.0 / (1.0 + math.exp(-x) / 9.0)


def path_decode_probability(
    snr_db: float, threshold_db: float, penalties: tuple[float, ...], slope_db: float
) -> float:
    """The chance a frame decodes at ``snr_db`` on a path not yet measured: the mean of the
    steady channel's curve and the three fading channels' (``penalties`` over the threshold)."""
    points = (0.0, *penalties)
    return sum(decode_probability(snr_db, threshold_db + p, slope_db) for p in points) / len(points)


def prior_probability(
    snr_db: float,
    threshold_db: float,
    penalties: tuple[float, ...],
    slope_db: float,
    margin_db: float | None,
) -> float:
    """The prototypes' prior: with ``margin_db`` the AWGN curve that far under the reading (an
    optimistic prior — a rung above found wanting costs one measured transmission, a rung under
    the path's best costs every wait for a try), else the four channels' mean."""
    if margin_db is not None:
        return decode_probability(snr_db - margin_db, threshold_db, slope_db)
    return path_decode_probability(snr_db, threshold_db, penalties, slope_db)


@dataclass
class _Account:
    """A rung's measured delivery, decayed: data bytes delivered and seconds of air spent."""

    bytes: float = 0.0
    air_s: float = 0.0
    frames: float = 0.0


@dataclass
class DeliveryController(RateController):
    """C1-R: choose by measured delivery, at the receiver (``03-candidates.md`` §2.2).

    Per rung, the data its frames delivered per second of the air they took — every copy,
    every frame never detected — decayed with a half-life of :attr:`half_life` frames sent at
    it, against a prior from the smoothed SNR worth :attr:`prior_frames` frames. The
    recommendation is the best estimate of the current rung and its usable neighbours. The
    neighbour above is tried — one transmission recommended at it — when its lossless rate
    beats the current rung's estimate and :attr:`probe_every` transmissions have gone since the
    last try of it; a try that delivered less than the rung it went up from doubles the wait.

    It inherits :class:`RateController` for what the engine reads of it (the ladder, the
    thresholds, the echo exclusions, the SNR) and replaces the margin, the hysteresis, the
    dwell, the stride, the decay, the partial rule, the edge rule and the re-seed."""

    capacity: dict[int, int] = field(default_factory=dict)
    frame_air_s: dict[int, float] = field(default_factory=dict)
    half_life: float = 12.0
    prior_frames: float = 2.0
    probe_every: int = 6
    probe_wait_max: int = 48
    slope_db: float = 0.5
    penalties: dict[int, tuple[float, ...]] = field(default_factory=dict)
    """Per rung, its fading channels' 10 % points over its AWGN one (:data:`CLASS_PENALTY_DB`):
    the prior is the mean of the four channels' curves at the reading, unless
    :attr:`prior_margin_db` is set."""
    prior_margin_db: float | None = None
    _accounts: dict[int, _Account] = field(default_factory=dict)
    _since_probe: dict[int, int] = field(default_factory=dict)
    _probe_wait: dict[int, int] = field(default_factory=dict)
    _probing: int | None = None
    """The rung being tried — what is recommended until a transmission carrying it is seen."""
    _probe_tries: int = 0

    # ── what the engine feeds it ──────────────────────────────────────

    def observe(self, snr_db: float | None, ok: int, failed: int, mode: int | None = None) -> None:
        """A burst's counts: only the SNR is used (``observe_frames`` carries the rest); a
        silent slot (``None, 0, 1``) is no news of any rung."""
        self.observe_snr(snr_db)

    def observe_frames(self, frames: list[FrameOutcome], snr_db: float | None) -> None:
        """A transmission's data frames: each rung's account takes its frames, a try ends, and
        the recommendation is the best estimate around the current rung."""
        self.observe_snr(snr_db)
        if not frames:
            return
        decay = 0.5 ** (1.0 / self.half_life)
        for f in frames:
            acc = self._accounts.setdefault(f.mode, _Account())
            acc.bytes *= decay
            acc.air_s *= decay
            acc.frames *= decay
            acc.air_s += f.air_s
            acc.frames += 1.0
            if f.decoded:
                acc.bytes += f.capacity
        sent = {f.mode for f in frames}
        for mode in self.modes:
            self._since_probe[mode] = self._since_probe.get(mode, 0) + 1
        here = self.modes[self._index]
        for mode in sent - {self._probing, here}:
            # frames at a rung above, sent for another reason (a copy still at its first rung,
            # the Test's measurement): news of it, as a try would be
            if mode > here:
                self._judge_try(mode)
        if self._probing is not None:
            self._probe_tries += 1
            if self._probing in sent or self._probe_tries >= 2:
                self._end_probe(self._probing in sent)
        if self._probing is None:
            self._decide()

    # ── the estimate ──────────────────────────────────────────────────

    def estimate(self, mode: int) -> float:
        """Data bytes a second of air at ``mode`` delivers, measured and prior together."""
        cap = float(self.capacity.get(mode, 0))
        air = self.frame_air_s.get(mode, 1.0)
        if self.snr_db is None:
            prior = 0.0
        else:
            prior = cap * prior_probability(
                self.snr_db,
                self.thresholds[mode],
                self.penalties.get(mode, ()),
                self.slope_db,
                self.prior_margin_db,
            )
        acc = self._accounts.get(mode, _Account())
        w = self.prior_frames
        return (acc.bytes + w * prior) / (acc.air_s + w * air)

    def lossless(self, mode: int) -> float:
        return float(self.capacity.get(mode, 0)) / self.frame_air_s.get(mode, 1.0)

    def recommend(self) -> int:
        return self._probing if self._probing is not None else self.modes[self._index]

    def _usable(self, index: int) -> bool:
        return 0 <= index < len(self.modes) and not self._echo_out(index)

    def _end_probe(self, measured: bool) -> None:
        mode, self._probing = self._probing, None
        assert mode is not None
        self._probe_tries = 0
        if not measured:
            return
        if self._judge_try(mode):
            self._index = self.modes.index(mode)
            if self._usable(self._index + 1):
                # a climb that works goes on: the rung above is tried at the next transmission
                above = self.modes[self._index + 1]
                self._since_probe[above] = self._probe_wait.get(above, self.probe_every)

    def _judge_try(self, mode: int) -> bool:
        """What frames at ``mode``, above the current rung, said: whether it now beats the
        current rung. If not, the next try of it waits twice as long."""
        here = self.modes[self._index]
        if self.estimate(mode) > self.estimate(here):
            self._probe_wait[mode] = self.probe_every
            return True
        wait = self._probe_wait.get(mode, self.probe_every)
        self._probe_wait[mode] = min(self.probe_wait_max, 2 * wait)
        self._since_probe[mode] = 0
        return False

    def _decide(self) -> None:
        # down to any rung (the estimate of one never sent is the prior); up only to a rung
        # measured — by a try, or by frames the sender put there itself
        here = [
            i
            for i in range(len(self.modes))
            if self._usable(i)
            and (i <= self._index or self._accounts.get(self.modes[i], _Account()).frames >= 1.0)
        ]
        if not here:
            self._index = 0
            return
        self._index = max(here, key=lambda i: (self.estimate(self.modes[i]), -i))
        up = self._index + 1
        if not self._usable(up):
            return
        mode = self.modes[up]
        if self.lossless(mode) <= self.estimate(self.modes[self._index]):
            return
        if self._since_probe.get(mode, 0) >= self._probe_wait.get(mode, self.probe_every):
            self._probing = mode
            self._since_probe[mode] = 0

    def silent(self, steps: int) -> None:
        """A transmission of this station's went unanswered (C1-S): the rung steps down, as the
        engine steps its own (R9), and a try ends."""
        self._probing = None
        self._probe_tries = 0
        self._index = max(0, self._index - steps)

    # ── the start ─────────────────────────────────────────────────────

    def first_mode(self, snr_db: float) -> int:
        """The rung the prior puts highest at ``snr_db``, no steps in hand (C4)."""
        saved = self.snr_db
        self.snr_db = snr_db
        best = max(
            (i for i in range(len(self.modes)) if not self._echo_out(i)),
            key=lambda i: (self.estimate(self.modes[i]), -i),
            default=0,
        )
        self.snr_db = saved
        return self.modes[best]

    def seed(self, snr_db: float, *, lower_bound: bool = False) -> None:
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))


@dataclass
class OuterLoopController(RateController):
    """C2-R: outer-loop adaptation to a derived target (``03-candidates.md`` §2.4).

    One offset per family (the tone floor, OFDM), moved per frame sent at the rung recommended:
    up by :attr:`step_db` when its copy failed or was never detected, down by
    ``step_db · T / (1 − T)`` when it decoded — OLLA's fixed point, where the share failing is
    ``T``. The target ``T`` is not tuned: it is the error rate at which a rung and the one below
    it deliver alike, ``1 − R_{r−1}/R_r``, averaged over the family's neighbours. The rung is the
    one with the most expected delivery, its payload rate times its chance of decoding at the
    reading less its family's offset, read off a logistic through each rung's AWGN 10 % point.

    It replaces the margin and its schedule, the hysteresis, the dwell, the stride, the partial
    rule, the edge rule and the re-seed."""

    capacity: dict[int, int] = field(default_factory=dict)
    frame_air_s: dict[int, float] = field(default_factory=dict)
    step_db: float = 0.5
    slope_db: float = 0.5
    start_offset_db: float = 0.0
    penalties: dict[int, tuple[float, ...]] = field(default_factory=dict)
    """As :attr:`DeliveryController.penalties`: the curve a rung's chance is read off is the
    mean of the four channels', its family's offset under it."""
    prior_margin_db: float | None = None
    """As :attr:`DeliveryController.prior_margin_db`."""
    _offset: dict[bool, float] = field(default_factory=dict)
    _target: dict[bool, float] = field(default_factory=dict)

    def __post_init__(self) -> None:
        super().__post_init__()
        for floor in (True, False):
            rungs = [m for m in self.modes if self._is_floor(m) == floor]
            rates = [self.lossless(m) for m in rungs]
            shares = [1.0 - lo / hi for lo, hi in pairwise(rates) if hi > 0]
            self._target[floor] = min(0.5, max(0.05, sum(shares) / len(shares))) if shares else 0.1

    def _is_floor(self, mode: int) -> bool:
        return mode < self.floor_modes

    def lossless(self, mode: int) -> float:
        return float(self.capacity.get(mode, 0)) / self.frame_air_s.get(mode, 1.0)

    def offset(self, mode: int) -> float:
        return self._offset.get(self._is_floor(mode), self.start_offset_db)

    def expected(self, mode: int, snr_db: float | None = None) -> float:
        snr = self.snr_db if snr_db is None else snr_db
        if snr is None:
            return 0.0
        p = prior_probability(
            snr - self.offset(mode),
            self.thresholds[mode],
            self.penalties.get(mode, ()),
            self.slope_db,
            self.prior_margin_db,
        )
        return self.lossless(mode) * p

    def observe(self, snr_db: float | None, ok: int, failed: int, mode: int | None = None) -> None:
        self.observe_snr(snr_db)

    def observe_frames(self, frames: list[FrameOutcome], snr_db: float | None) -> None:
        asked = self.recommend()
        self.observe_snr(snr_db)
        for f in frames:
            if f.mode != asked or f.combined:
                continue  # a copy HARQ combined says nothing of the first copy's error rate
            floor = self._is_floor(f.mode)
            target = self._target[floor]
            offset = self._offset.get(floor, self.start_offset_db)
            if f.decoded:
                offset -= self.step_db * target / (1.0 - target)
            else:
                offset += self.step_db
            self._offset[floor] = min(20.0, max(-5.0, offset))
        self._choose()

    def _choose(self) -> None:
        usable = [i for i in range(len(self.modes)) if not self._echo_out(i)]
        if not usable or self.snr_db is None:
            self._index = 0
            return
        self._index = max(usable, key=lambda i: (self.expected(self.modes[i]), -i))

    def first_mode(self, snr_db: float) -> int:
        usable = [i for i in range(len(self.modes)) if not self._echo_out(i)]
        best = max(usable, key=lambda i: (self.expected(self.modes[i], snr_db), -i), default=0)
        return self.modes[best]

    def seed(self, snr_db: float, *, lower_bound: bool = False) -> None:
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))


CONTROLLERS = ("current", "delivery", "olla", "delivery-s")
"""The controllers :attr:`~aether_model.link.granted.GrantConfig.controller` names."""


def default_thresholds() -> dict[int, float]:
    return dict(AWGN_THRESHOLD_DB)
