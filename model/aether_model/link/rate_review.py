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


WAIT_TOLERANCE_S = 1e-9
"""Six transmissions' air is a try's wait exactly when six such transmissions went: a sum that
lands on it counts as there whatever its last bit (:meth:`DeliveryController._wait_s`)."""


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
    every frame never detected — decayed with a half-life of :attr:`half_life` of its own
    frames' air, whatever rung that air went at, against a prior from the smoothed SNR worth
    :attr:`prior_frames` frames. The recommendation is the best estimate of the current rung
    and its usable neighbours. The neighbour above is tried — one transmission recommended at
    it — when its lossless rate beats the current rung's estimate and the air of
    :attr:`probe_every` full transmissions at it has gone since the last try of it, or at once
    when it is of the same family, its estimate beats the current rung's, no try of it has
    failed since it last carried, and every frame of the last transmission at the current rung
    decoded alone (:meth:`_clean_climb`); a try that delivered less than the rung it went up
    from doubles the wait.

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
    the prior is the mean of the four channels' curves at the reading when
    :attr:`prior_margin_db` is ``None``."""
    prior_margin_db: float | None = 3.0
    """The prior is the AWGN curve this far under the reading (:func:`prior_probability`): on
    the real modem it beat the four channels' mean, ×0.942 of the current controller's session
    time against ×1.095 (``04-results.md`` §2)."""
    family_gain_db: float = 0.5
    """How far a frame moves its family's offset, per unit of surprise: the prior of every rung of
    a family is read that far under the reading, the offset moved by every frame of the family
    (not combined) by ``gain × (p − decoded)`` — up by nearly the gain for a failure the prior
    was sure of, down by nearly it for a success it ruled out. Without it a rung never sent at
    keeps the optimistic prior whatever its family's rungs have shown: on the harness's NVIS
    paths C1 stepped down from a failing OFDM rung to the OFDM rung below, unmeasured, and the
    one below that (``04-results.md`` §3.4). ``0`` turns it off."""
    _family: dict[bool, float] = field(default_factory=dict)
    """The families' offsets, by whether the family is the floor's."""
    control_s: dict[bool, float] = field(default_factory=dict)
    """A control frame's length, by whether it is the floor's. With :attr:`burst_frames` and
    :attr:`gap_s` the rungs are ranked per second of a whole exchange (:meth:`value`): a full
    transmission's data frames, its header, the answer and the two gaps. The header and the
    answer go in the families the two stations ask of each other (:attr:`families`), whatever
    the data's rung — so on a lopsided path whose answers go on the floor, the 500 Hz air's
    first OFDM rung (six 1.05 s frames and a 3.2 s answer) carries less than the floor's top
    (five 5.36 s frames) with nothing lost, where per second of frame air it looked 24 % better
    (``04-results.md`` §3.10). Empty: per second of frame air, as before."""
    gap_s: float = 0.0
    """The two gaps of an exchange: to the answer's slot, and to the next key after it."""
    burst_frames: dict[int, int] = field(default_factory=dict)
    """Per rung, the data frames a full transmission at it carries (``burst_capacity``)."""
    families: tuple[bool, bool] | None = None
    """Whether this station's control frames go on the floor, and whether the other's do (what
    each asked of the other), as the engine last said; ``None``: both in the data's family."""
    cautious_start: bool = False
    """The first rung is the four channels' mean's best (:func:`path_decode_probability`), not
    the optimistic prior's, and the rung above it is tried at the next transmission. Built for
    the harness's disturbed NVIS, where a first transmission at OFDM 5 cost 24 OFDM frames before
    its blocks could go on the floor, and rejected there (``04-results.md`` §3.6): it took that
    scenario from 0.76 to 1.03 of the current controller's delivery and cost more everywhere a
    climb is the job — the median regret +22.6 → +28.7 %, three weak paths under 0.85, one Test
    failed. Off; kept until the decision ADR says what goes."""
    _accounts: dict[int, _Account] = field(default_factory=dict)
    _since_probe: dict[int, float] = field(default_factory=dict)
    """Seconds of data air observed since each rung's last try (:meth:`_wait_s`)."""
    _probe_wait: dict[int, int] = field(default_factory=dict)
    _probing: int | None = None
    """The rung being tried — what is recommended until a transmission carrying it is seen."""
    _probe_tries: int = 0
    _try_frames: int = 0
    """Frames at the rung being tried since the try began: a try is decided once they outweigh
    the prior (:meth:`_try_over`)."""
    _clean_at: int | None = None
    """The rung the last transmission's frames went at, when every one of them there decoded
    alone (:meth:`_clean_climb`)."""

    # ── what the engine feeds it ──────────────────────────────────────

    def observe(self, snr_db: float | None, ok: int, failed: int, mode: int | None = None) -> None:
        """A burst's counts: only the SNR is used (``observe_frames`` carries the rest); a
        silent slot (``None, 0, 1``) is no news of any rung."""
        self.observe_snr(snr_db)

    def observe_frames(self, frames: list[FrameOutcome], snr_db: float | None) -> None:
        """A transmission's data frames: each moves its family's offset and its rung's account,
        a try ends, and the recommendation is the best estimate around the current rung."""
        self.observe_snr(snr_db)
        if not frames:
            return
        if self.family_gain_db and self.snr_db is not None:
            for f in frames:
                if f.combined or f.mode not in self.thresholds:
                    continue
                floor = f.mode < self.floor_modes
                p = self._prior_p(f.mode, self.snr_db)
                offset = self._family.get(floor, 0.0) + self.family_gain_db * (
                    p - (1.0 if f.decoded else 0.0)
                )
                self._family[floor] = min(20.0, max(-5.0, offset))
        decay = 0.5 ** (1.0 / self.half_life)
        for f in frames:
            for mode, old in self._accounts.items():
                if mode == f.mode:
                    continue
                # a rung's account ages with the air sent at any rung, in frames of its own:
                # aged only by its own frames, three copies of one block sent two minutes
                # earlier let an OFDM rung back in on an NVIS path, and it delivered two blocks
                # of the next seventeen frames (04-results.md §3.24)
                aged = 0.5 ** (f.air_s / (self.half_life * self.frame_air_s.get(mode, 1.0)))
                old.bytes *= aged
                old.air_s *= aged
                old.frames *= aged
            acc = self._accounts.setdefault(f.mode, _Account())
            acc.bytes *= decay
            acc.air_s *= decay
            acc.frames *= decay
            acc.air_s += f.air_s
            acc.frames += 1.0
            if f.decoded:
                acc.bytes += f.capacity
        sent = {f.mode for f in frames}
        air = 0.0
        for f in frames:
            air += f.air_s  # added in order, as the port adds (Python's sum() compensates)
        for mode in self.modes:
            self._since_probe[mode] = self._since_probe.get(mode, 0.0) + air
        here = self.modes[self._index]
        at_here = [f for f in frames if f.mode == here]
        clean = bool(at_here) and all(f.decoded and not f.combined for f in at_here)
        self._clean_at = here if clean else None
        for mode in sent - {self._probing, here}:
            # frames at a rung above, sent for another reason (a copy still at its first rung,
            # the Test's measurement): news of it, as a try would be
            if mode > here:
                self._judge_try(mode)
        if self._probing is not None:
            self._probe_tries += 1
            self._try_frames += sum(1 for f in frames if f.mode == self._probing)
            if self._try_over():
                self._end_probe(self._try_frames > 0)
        self._decide()

    # ── the estimate ──────────────────────────────────────────────────

    def estimate(self, mode: int) -> float:
        """Data bytes a second of air at ``mode`` delivers, measured and prior together."""
        cap = float(self.capacity.get(mode, 0))
        air = self.frame_air_s.get(mode, 1.0)
        prior = 0.0 if self.snr_db is None else cap * self._prior_p(mode, self.snr_db)
        acc = self._accounts.get(mode, _Account())
        w = self.prior_frames
        return (acc.bytes + w * prior) / (acc.air_s + w * air)

    def value(self, mode: int) -> float:
        """What the rungs are ranked by: :meth:`estimate` per second of a full exchange at
        ``mode`` (:attr:`control_s`), or of its frames' air when the engine gave no lengths."""
        return self._priced(mode, self.estimate(mode))

    def _priced(self, mode: int, per_air: float) -> float:
        n = self.burst_frames.get(mode)
        if not n or not self.control_s:
            return per_air
        floor = mode < self.floor_modes
        own, other = self.families if self.families is not None else (floor, floor)
        over = self.control_s[own] + self.control_s[other] + self.gap_s
        air = n * self.frame_air_s.get(mode, 1.0)
        return per_air * air / (air + over)

    def _prior_p(self, mode: int, snr_db: float) -> float:
        """The prior's chance a frame at ``mode`` decodes at ``snr_db``, its family's offset
        under the reading."""
        offset = self._family.get(mode < self.floor_modes, 0.0)
        return prior_probability(
            snr_db - offset,
            self.thresholds[mode],
            self.penalties.get(mode, ()),
            self.slope_db,
            self.prior_margin_db,
        )

    def lossless(self, mode: int) -> float:
        return float(self.capacity.get(mode, 0)) / self.frame_air_s.get(mode, 1.0)

    def recommend(self) -> int:
        return self._probing if self._probing is not None else self.modes[self._index]

    def _usable(self, index: int) -> bool:
        return 0 <= index < len(self.modes) and not self._echo_out(index)

    def _try_over(self) -> bool:
        """A try is decided once its own frames outweigh the prior — more than
        :attr:`prior_frames` of them — or, a frame or none a transmission, after twice the
        transmissions that takes. The sender tries with one frame a transmission, and one frame
        is a coin toss: one lucky decode put seventeen frames on OFDM over a 7 ms echo, two
        unlucky ones shut rung 7 from a low start for a whole session (``04-results.md``
        §3.14)."""
        enough = int(self.prior_frames) + 1
        return self._try_frames >= enough or self._probe_tries >= 2 * enough

    def _cancel_try(self) -> None:
        self._probing = None
        self._probe_tries = 0
        self._try_frames = 0

    def _end_probe(self, measured: bool) -> None:
        mode = self._probing
        assert mode is not None
        self._cancel_try()
        if not measured:
            return
        if self._judge_try(mode):
            self._index = self.modes.index(mode)
            if self._usable(self._index + 1):
                # a climb that works goes on: the rung above is tried at the next transmission
                above = self.modes[self._index + 1]
                self._since_probe[above] = self._wait_s(above)

    def _wait_s(self, mode: int) -> float:
        """The air to let go by before trying ``mode``: its wait, in full transmissions at
        ``mode``. Counted in transmissions of whatever was sent, the wait was six floor
        transmissions (about 160 s) before a try of OFDM and under a minute above it: from a low
        start, or after a stall put the session on the floor, the way back took minutes
        (``04-results.md`` §3.16)."""
        n = self.burst_frames.get(mode, 1)
        return self._probe_wait.get(mode, self.probe_every) * (n * self.frame_air_s.get(mode, 1.0))

    def _judge_try(self, mode: int) -> bool:
        """What frames at ``mode``, above the current rung, said: whether it now beats the
        current rung. If not, the next try of it waits twice as long."""
        here = self.modes[self._index]
        if self.value(mode) > self.value(here):
            self._probe_wait[mode] = self.probe_every
            return True
        wait = self._probe_wait.get(mode, self.probe_every)
        self._probe_wait[mode] = min(self.probe_wait_max, 2 * wait)
        self._since_probe[mode] = 0.0
        return False

    def _decide(self) -> None:
        # down to any rung (the estimate of one never sent is the prior); up only to a rung
        # whose own frames outweigh its prior — a try's, or frames the sender put there itself
        here = [
            i
            for i in range(len(self.modes))
            if self._usable(i)
            and (
                i <= self._index
                or self._accounts.get(self.modes[i], _Account()).frames > self.prior_frames
            )
        ]
        if not here:
            self._index = 0
            self._cancel_try()
            return
        self._index = max(here, key=lambda i: (self.value(self.modes[i]), -i))
        up = self._index + 1
        if self._probing is not None:
            if not self._usable(up) or self.modes[up] != self._probing:
                self._cancel_try()  # the choice moved from under the try
            return
        if not self._usable(up):
            return
        mode = self.modes[up]
        if self._priced(mode, self.lossless(mode)) <= self.value(self.modes[self._index]):
            return
        waited = self._since_probe.get(mode, 0.0) >= self._wait_s(mode) - WAIT_TOLERANCE_S
        if waited or self._clean_climb(mode):
            self._probing = mode
            self._since_probe[mode] = 0.0

    def _clean_climb(self, mode: int) -> bool:
        """Whether the rung above, ``mode``, is tried without its wait: of the current rung's
        family (a try of it is one frame a transmission at the sender, where a try across the
        floor's edge takes the transmission and its leftovers, §3.22), rated above the current
        rung, no try of it failed since it last carried, and every frame of the last
        transmission at the current rung decoded alone. On a strong path the wait — six full
        transmissions — was most of a session: 36 of 36 frames decoded at rung 12 while the
        reading rose and rated rung 14 higher, and the try came in the last transmission
        (``04-results.md`` §3.24, K5)."""
        here = self.modes[self._index]
        return (
            self._clean_at == here
            and (mode < self.floor_modes) == (here < self.floor_modes)
            and self._probe_wait.get(mode, self.probe_every) == self.probe_every
            and self.value(mode) > self.value(here)
        )

    def observe_echo(self, share: float | None, snr_db: float | None) -> None:
        """The echo reading moves the smoothed echo as the current controller's does (ADR-0087);
        a try of a rung it now rules out ends, and a choice it rules out goes to the best
        estimate below that it allows."""
        if share is None or snr_db is None or snr_db < self.echo_min_snr_db:
            return
        w = self.echo_weight
        self.echo = share if self.echo is None else (1.0 - w) * self.echo + w * share
        if self._probing is not None and self._echo_out(self.modes.index(self._probing)):
            self._cancel_try()
        if self._echo_out(self._index):
            below = [i for i in range(self._index) if self._usable(i)]
            self._index = max(below, key=lambda i: (self.value(self.modes[i]), -i)) if below else 0

    def silent(self, steps: int) -> None:
        """A transmission of this station's went unanswered (C1-S): the rung steps down, as the
        engine steps its own (R9), to a rung the echo allows, and a try ends. Stepped by the
        ladder's index alone it landed on the rungs a 7 ms echo rules out (ADR-0087), and blocks
        cut there stayed for their four copies (``04-results.md`` §3.10)."""
        self._cancel_try()
        below = [i for i in range(max(0, self._index - steps) + 1) if self._usable(i)]
        self._index = below[-1] if below else 0

    # ── the start ─────────────────────────────────────────────────────

    def first_mode(self, snr_db: float) -> int:
        """The rung a session starts at from one reading, no steps in hand (C4): the best by the
        four channels' mean when :attr:`cautious_start`, by the optimistic prior otherwise."""
        saved = self.snr_db
        self.snr_db = snr_db
        value = self._start_estimate if self.cautious_start else self.value
        best = max(
            (i for i in range(len(self.modes)) if not self._echo_out(i)),
            key=lambda i: (value(self.modes[i]), -i),
            default=0,
        )
        self.snr_db = saved
        return self.modes[best]

    def _start_estimate(self, mode: int) -> float:
        """:meth:`estimate` with the four channels' mean as the prior."""
        assert self.snr_db is not None
        cap = float(self.capacity.get(mode, 0))
        air = self.frame_air_s.get(mode, 1.0)
        offset = self._family.get(mode < self.floor_modes, 0.0)
        p = path_decode_probability(
            self.snr_db - offset, self.thresholds[mode], self.penalties.get(mode, ()), self.slope_db
        )
        acc = self._accounts.get(mode, _Account())
        w = self.prior_frames
        return (acc.bytes + w * cap * p) / (acc.air_s + w * air)

    def seed(self, snr_db: float, *, lower_bound: bool = False) -> None:
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))
        if self.cautious_start and self._usable(self._index + 1):
            above = self.modes[self._index + 1]
            self._since_probe[above] = self._wait_s(above)


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
    prior_margin_db: float | None = 3.0
    """As :attr:`DeliveryController.prior_margin_db`: ×0.886 of the current controller's session
    time against ×1.000 with the four channels' mean."""
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

    def observe_echo(self, share: float | None, snr_db: float | None) -> None:
        """The echo reading moves the smoothed echo as the current controller's does (ADR-0087);
        a choice it now rules out is chosen again among the rungs it allows."""
        if share is None or snr_db is None or snr_db < self.echo_min_snr_db:
            return
        w = self.echo_weight
        self.echo = share if self.echo is None else (1.0 - w) * self.echo + w * share
        if self._echo_out(self._index):
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
