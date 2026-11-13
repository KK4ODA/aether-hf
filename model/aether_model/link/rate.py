"""The rate controller (ADR-0094, ADR-0099): each station chooses the rung its own data goes at.

Per rung, the data its frames delivered per second of the air they took — every copy, every
frame never detected — decayed, against a prior from the smoothed SNR the other station reports;
the choice is the best estimate per second of a whole exchange, and the rung above is tried when
it could beat it. Nothing recommends a rung to the other station: one decision-maker, at the
sender (the rate controller's second review, ``docs/research/rate-control-2/``).

The tables below are the PHY's measurements: each rung's AWGN 10 % point
(``bench/baselines/phy_fer.csv``, ``phy_fer_500.csv``), its payload and frame length, the echo a
rung cannot carry (ADR-0087), and the rungs worth offering at all (:func:`usable_modes`).
"""

from __future__ import annotations

import math
from collections.abc import Mapping
from dataclasses import dataclass, field

from aether_model.link.frames import DATA_HEADER

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
    4: -11.1,
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
entry is measured: the tone floor's five (rungs 0–4: its own two, ADR-0013, the same frames
and thresholds as on the wide ladder, and the four-tone middle kinds, ADR-0015 and ADR-0081)
by ``tools/bench_tone.py`` (``bench/baselines/tone_floor.csv``); the OFDM modes (rungs 5–14,
OFDM modes 3–12) by ``tools/bench_phy.py --bandwidth 500`` (``bench/baselines/
phy_fer_500.csv``; written by ``tools/update_rate_table.py --bandwidth 500 --apply``). Its
control mode, QPSK ½ (rung 5), sits at −5.2 dB against the wide table's BPSK ⅕ at −5.1
because twelve carriers carry ≈ 6.8 dB more per carrier than fifty-seven; below it is the
tone floor, which reaches −19 dB where the OFDM floor it replaced (ADR-0009) reached −12. QPSK
⅓ on the ordinary frame was rung 4 until tone4x100-93 took its place (ADR-0081)."""

ECHO_LIMIT: dict[int, float] = {5: 0.0073}
"""The echo share (:func:`~aether_model.phy.tone.echo_share`) above which each rung of the
2 300 Hz ladder that has one is kept off it (ADR-0087), from ``tools/bench_echo.py --limits``
(``bench/baselines/echo.csv``; a test holds the two to each other): tone100-153, whose 100-baud
symbols a 7 ms echo breaks. A rung without one is carried on some measured path that reads
nearly as much echo as any path that refuses it — tone100-105's sixteen tones over 1 600 Hz
carry a 6 ms path at a reading of 0.011."""

NARROW_ECHO_LIMIT: dict[int, float] = {3: 0.0038, 4: 0.0038}
"""The 500 Hz ladder's :data:`ECHO_LIMIT`: the four-tone kinds' 10 ms symbols at their two higher
code rates, refused by two rays 5 ms apart (a reading of 0.004) and carried 4 ms apart (0.003).
Rung 2 and the OFDM rungs have none; the climb stops under rung 3 all the same."""

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
    4: 93,
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

NARROW_FRAME_S: dict[int, float] = {m: (5.36 if m < 5 else 1.054) for m in range(15)}
"""Air time of each narrow rung's DATA frame: 134 slots of 40 ms on the tone floor, its
middle kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy of
the frames; tested against them)."""


WIDE_PRIOR_MARGIN_DB = 1.0
"""How far under the reading the prior is read on the 2 300 Hz air (ADR-0111): there a rung's 10 %
point arrives 0.2-0.8 dB over its AWGN point by the reading on a clean path, and the 3 dB of
:attr:`RateController.prior_margin_db` left a strong path one or two rungs low for the whole of a
short transfer (``docs/research/rate-control-3/01-evidence.md`` §2-3). The 500 Hz air keeps the 3:
its rungs pay the larger fading penalty, and with less caution a 7 ms NVIS echo kept a session
going back to OFDM (``04-results.md`` §5.1-5.2)."""
WIDE_FAMILY_GAIN_DB = 1.0
"""How far a frame moves its family's offset on the 2 300 Hz air (ADR-0111), twice
:attr:`RateController.family_gain_db`: with the smaller margin, a family that fails is walked
down twice as fast (``04-results.md`` §5.2)."""


def usable_modes(
    thresholds: Mapping[int, float] = AWGN_THRESHOLD_DB,
    payload: Mapping[int, float] = PAYLOAD_BYTES,
    frame_s: Mapping[int, float] | None = FRAME_S,
) -> list[int]:
    """Modes on the throughput/threshold Pareto front, ascending. ``payload`` is a frame's PHY
    payload in bytes; given ``frame_s`` (air time per mode) the comparison is bytes per second,
    which is what tells the floor's long frames from the ordinary ones.

    A mode is worth the data a DATA frame carries in it, after the frame's 3-byte header
    (ADR-0073). Counted by the payload, the 500 Hz air's QPSK ⅓ (15 bytes in 1.054 s) beat
    tone4x100-75 (75 bytes in 5.36 s), 114 bit/s to 112, and was offered from 7 dB further up;
    after the header it carries 91 bit/s to the tone rung's 107."""

    def worth(m: int) -> float:
        return max(0.0, payload[m] - DATA_HEADER) / (frame_s[m] if frame_s is not None else 1.0)

    out = []
    for m, thr in thresholds.items():
        dominated = any(
            worth(o) >= worth(m) and thresholds[o] <= thr and o != m for o in thresholds
        )
        if not dominated:
            out.append(m)
    return sorted(out)


# What each rung's 10 % point moves by on ITU-R F.1487's Good, Moderate and Poor channels, dB over
# its AWGN point (bench/baselines/phy_fer.csv, phy_fer_500.csv and tone_floor.csv through
# tools/bench_link.channel_thresholds, which fills a rung not swept from its family's mean): the
# controller's prior for a path not yet measured is the mean of the four channels' curves.
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
lands on it counts as there whatever its last bit (:meth:`RateController._wait_s`)."""


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
    """The prior: with ``margin_db`` the AWGN curve that far under the reading (an
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
class RateController:
    """The rate controller (C1, ADR-0094; the one controller, ADR-0099): the rung this
    station's own data goes at.

    Per rung, the data its frames delivered per second of the air they took — every copy,
    every frame never detected — decayed with a half-life of :attr:`half_life` of its own
    frames' air, whatever rung that air went at, against a prior from the smoothed SNR the other
    station reports, worth :attr:`prior_frames` frames. The choice is the best estimate, per
    second of a whole exchange, of the current rung and its usable neighbours. The neighbour
    above is tried — one frame a transmission at it — when its lossless rate beats the current
    rung's estimate and the air of :attr:`probe_every` full transmissions at it has gone since
    the last try of it, or at once when it is of the same family, its estimate beats the current
    rung's, no try of it has failed since it last carried, and every frame of the last
    transmission at the current rung decoded alone (:meth:`_clean_climb`); a try that delivered
    less than the rung it went up from doubles the wait. The OFDM family is refused as a whole
    (:meth:`refuses`, D3) when the choice has gone to the floor after OFDM frames that delivered
    nothing."""

    thresholds: dict[int, float] = field(default_factory=lambda: dict(AWGN_THRESHOLD_DB))
    """Each rung's AWGN 10 % point: where the prior puts its curve."""
    modes: list[int] = field(default_factory=usable_modes)
    """The rungs on offer, ascending (:func:`usable_modes`): the choice is ``modes[_index]``."""
    floor_modes: int = 6
    """How many of the ladder's leading rungs are the floor's — the tone floor, ADR-0013, and on
    the 2 300 Hz air its fast kinds, ADR-0014 — whose frames are five times as long as an OFDM
    frame."""
    echo_limit: dict[int, float] = field(default_factory=dict)
    """The echo share above which a rung is kept off the ladder (ADR-0087): the tone floor's
    40 ms symbols carry a path whose echo smears the fast kinds' 10 ms ones and outruns OFDM's
    5 ms prefix, and the SNR such a path reads cannot say so — the floor's reading saturates
    near −3 dB whatever the SNR. :data:`ECHO_LIMIT`, :data:`NARROW_ECHO_LIMIT`; empty, no rung
    has one."""
    echo_weight: float = 0.4
    """How much each reading moves :attr:`echo`: a frame's reading spreads by a factor of two
    or so about its path's, and a session's first data goes on one or two of them."""
    echo_min_snr_db: float = -6.0
    """The weakest frame whose echo reading counts: below it the noise in the bins the reading
    weighs is as large as a 4 ms echo's spill, and at those SNRs the rungs an echo refuses are
    not on offer."""
    echo: float | None = None
    """The path's echo as the frames have read it, smoothed (:meth:`observe_echo`)."""
    snr_db: float | None = None
    """The smoothed reading the prior is drawn at: how the other station hears this one."""
    _smoothed: float | None = None
    _index: int = 0
    """Position in :attr:`modes` — the choice is ``modes[_index]``."""
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
    the real modem it beat the four channels' mean, ×0.942 of the old controller's session
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
    _ofdm_failed: bool = False
    """The last OFDM frames observed delivered nothing (:meth:`_judge_family`)."""

    def __post_init__(self) -> None:
        self._index = min(self._index, len(self.modes) - 1)

    # ── what the engine feeds it ──────────────────────────────────────

    def observe_snr(self, snr_db: float | None) -> None:
        """A reading of how the other station hears this one, smoothed 0.7/0.3."""
        if snr_db is None:
            return
        self._smoothed = snr_db if self._smoothed is None else 0.7 * self._smoothed + 0.3 * snr_db
        self.snr_db = self._smoothed

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
        self._judge_family(frames)
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

    def _judge_family(self, frames: list[FrameOutcome]) -> None:
        """D3 from outcomes: the OFDM frames of a transmission delivered nothing, or some did."""
        ofdm = [f for f in frames if f.mode >= self.floor_modes]
        if ofdm:
            self._ofdm_failed = not any(f.decoded for f in ofdm)

    def refuses(self, mode: int) -> bool:
        """D3: whether ``mode``'s family is refused on this path. The OFDM family is, when every
        rung of it is over its echo limit, or when the choice is a floor rung and the last OFDM
        frames delivered nothing: the account has judged every OFDM rung worse than the floor
        on what OFDM just did. (D3 as first written refused the family only on its most robust
        rung's failure, and capped the faster rungs at it: C1 left OFDM for the floor from a
        faster rung, never sending at the most robust one, so the rule never fired, and C1
        climbs only to a rung its own frames have measured, so the cap changed nothing.) The
        floor's family is never refused: there is nothing under it."""
        if mode < self.floor_modes:
            return False
        if self._ofdm_failed and self.modes[self._index] < self.floor_modes:
            return True
        ofdm = [i for i, m in enumerate(self.modes) if m >= self.floor_modes]
        return bool(ofdm) and all(self._echo_out(i) for i in ofdm)

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
        """The echo reading moves the smoothed echo (ADR-0087);
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

    def _echo_out(self, index: int) -> bool:
        """Whether the path's echo keeps the rung at ``index`` off the ladder."""
        limit = self.echo_limit.get(self.modes[index])
        return limit is not None and self.echo is not None and self.echo > limit

    def silent(self, steps: int) -> None:
        """A transmission of this station's went unanswered: the rung steps down, as the
        engine steps its own (R9), to a rung the echo allows, and a try ends. Stepped by the
        ladder's index alone it landed on the rungs a 7 ms echo rules out (ADR-0087), and blocks
        cut there stayed for their four copies (``04-results.md`` §3.10)."""
        self._cancel_try()
        below = [i for i in range(max(0, self._index - steps) + 1) if self._usable(i)]
        self._index = below[-1] if below else 0

    # ── the start ─────────────────────────────────────────────────────

    def first_mode(self, snr_db: float) -> int:
        """The rung a session starts at from one reading, no steps in hand (C4): the best by the
        optimistic prior. (A cautious start, by the four channels' mean, was rejected on the
        harness and deleted: ``04-results.md`` §3.6.)"""
        saved = self.snr_db
        self.snr_db = snr_db
        best = max(
            (i for i in range(len(self.modes)) if not self._echo_out(i)),
            key=lambda i: (self.value(self.modes[i]), -i),
            default=0,
        )
        self.snr_db = saved
        return self.modes[best]

    def seed(self, snr_db: float, *, lower_bound: bool = False) -> None:
        if self._smoothed is not None:
            return
        self._smoothed = self.snr_db = snr_db
        self._index = self.modes.index(self.first_mode(snr_db))
