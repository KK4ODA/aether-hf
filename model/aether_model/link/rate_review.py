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
    prior_margin_db: float = 3.0
    """How far under the reading the prior puts a rung's AWGN curve: what a path not yet measured
    is assumed to cost — the current controller's starting margin."""
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
            prior = cap * decode_probability(
                self.snr_db - self.prior_margin_db, self.thresholds[mode], self.slope_db
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
        here = self.modes[self._index]
        if self.estimate(mode) > self.estimate(here):
            self._index = self.modes.index(mode)
            self._probe_wait[mode] = self.probe_every
            if self._usable(self._index + 1):
                # a climb that works goes on: the rung above is tried at the next transmission
                above = self.modes[self._index + 1]
                self._since_probe[above] = self._probe_wait.get(above, self.probe_every)
        else:
            wait = self._probe_wait.get(mode, self.probe_every)
            self._probe_wait[mode] = min(self.probe_wait_max, 2 * wait)

    def _decide(self) -> None:
        # down to any rung (the estimate of one never sent is the prior), up only by a try
        here = [i for i in range(self._index + 1) if self._usable(i)]
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
    start_offset_db: float = 3.0
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
        p = decode_probability(snr - self.offset(mode), self.thresholds[mode], self.slope_db)
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
