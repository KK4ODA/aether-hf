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
    partial_share: float = 0.5
    """A burst that lost fewer than this share of its frames, and decoded some, failed in part
    (ADR-0084): the margin widens by the share lost — the jump to the margin the failure
    implies too — the rung stays unless the SNR no longer carries it, and the climb waits. On a
    fading path a lone lost frame is the operating point HARQ pays for, not a cliff: widening
    in full on one frame of six took a 1 kB session on ITU Poor at −6 dB from rung 3 to rung 1
    for its next burst. A burst lost in the main is a failure as before, and so is any on the
    first OFDM rung (:meth:`_at_edge`): the floor below it carries an exchange better than the
    rung does once it loses frames — at 500 Hz a fifth of rung 5's at 0 dB on Moderate and Poor,
    where holding the rung cost 3 %; at 2 300 Hz, on WC4Y's Test of 2026-10-23, rung 7 decoded
    one frame in eight on its own while every tone frame decoded (ADR-0093)."""
    echo_limit: dict[int, float] = field(default_factory=dict)
    """The echo share above which a rung is kept off the ladder (ADR-0087): the tone floor's
    40 ms symbols carry a path whose echo smears the fast kinds' 10 ms ones and outruns OFDM's
    5 ms prefix, and the SNR such a path reads cannot say so — the floor's reading saturates
    near −3 dB whatever the SNR, as a quiet path at −3 dB reads, so on ITU's disturbed NVIS
    (7 ms) the 500 Hz air went first to rung 4, which decodes only on a frame's third copy, then
    to rung 3, then down to the rungs the path carries. :data:`ECHO_LIMIT`,
    :data:`NARROW_ECHO_LIMIT`; empty, no rung has one."""
    echo_weight: float = 0.4
    """How much each reading moves :attr:`echo`: a frame's reading spreads by a factor of two
    or so about its path's, and a session's first data goes on one or two of them."""
    echo_min_snr_db: float = -6.0
    """The weakest frame whose echo reading counts: below it the noise in the bins the reading
    weighs is as large as a 4 ms echo's spill, and at those SNRs the rungs an echo refuses are
    not on offer."""
    echo: float | None = None
    """The path's echo as the frames have read it, smoothed (:meth:`observe_echo`)."""
    reseed_margin_db: float = 3.0
    """How far above a lower-bound seed (:meth:`seed`) the first ordinary measurement must read
    to replace it. The tone floor's estimate is exact to about +10 dB on AWGN; a few decibels
    between two frames' readings is their estimates' own spread and the fade between them, and
    replacing a seed on that would only take the larger of two noisy numbers. Beyond it the
    seed was the floor's ceiling, not the path's."""
    floor_modes: int = 6
    """How many of the ladder's leading rungs are the floor's — the tone floor, ADR-0013, and
    on the 2 300 Hz air its fast kinds, ADR-0014 — whose frames are five times as long as an
    OFDM frame: see :meth:`first_mode` and :meth:`_at_edge`. The first OFDM rung is held to the
    learned margin like any other: the 2 300 Hz air capped it at 1 dB against the floor until
    ADR-0093 (ADR-0013 §4), which held a weak path on a rung that lost most of its frames."""
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
        if failed and ok and failed < self.partial_share * (ok + failed) and not self._at_edge():
            self._widen(snr_db, mode, failed / (ok + failed))
            self._ever_failed = True
            self._clean_run = 0
            self._clean_since_decay = 0
            self._decay_step_db = 0.0
            if not self._fits(self._index):
                self._step_down()
        elif failed:
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

    def observe_echo(self, share: float | None, snr_db: float | None) -> None:
        """A frame's echo reading (:func:`~aether_model.phy.tone.echo_share`, ADR-0087), read
        at ``snr_db``. A recommendation the reading now rules out moves to the fastest rung
        below that it allows and the SNR carries."""
        if share is None or snr_db is None or snr_db < self.echo_min_snr_db:
            return
        w = self.echo_weight
        self.echo = share if self.echo is None else (1.0 - w) * self.echo + w * share
        if not self._echo_out(self._index):
            return
        for i in range(self._index - 1, -1, -1):
            if not self._echo_out(i) and (self._fits(i) or i == 0):
                self._index = i
                return
        self._index = 0

    def _echo_out(self, index: int) -> bool:
        """Whether the path's echo keeps the rung at ``index`` off the ladder."""
        limit = self.echo_limit.get(self.modes[index])
        return limit is not None and self.echo is not None and self.echo > limit

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

    def _at_edge(self) -> bool:
        """Whether the recommendation is the first OFDM rung, above the floor: where a
        failure, in part or not, goes back to the floor (:attr:`partial_share`)."""
        return self.floor_modes > 0 and self._index == self._first_ordinary()

    def _widen(self, snr_db: float | None, mode: int | None, share: float = 1.0) -> None:
        """A failed burst is a measurement, not just a nudge: mode ``m`` failing at SNR ``s``
        says this channel needs more than ``s − threshold[m]`` dB of margin. Jump most of the
        way there instead of creeping up in fixed steps — on a fading channel, where every
        mode costs 6–10 dB more than the AWGN table predicts, creeping means overshooting the
        mode for many bursts first. The jump is capped so one deep fade cannot strand the link.

        Every failure is a measurement, an isolated one included: reading the first as an
        accident (a collision, a missed preamble) and jumping only on a repeat was tried on
        the link bench and cost 11 % on the Moderate channel at 16 dB, where the climb it
        allowed ran into the fades — see ADR-0007. A burst that failed in part moves the margin
        by the ``share`` of it that failed (:attr:`partial_share`)."""
        target = self.margin_db + self.up_step_db * share
        if snr_db is not None and mode is not None and mode in self.thresholds:
            implied = snr_db - self.thresholds[mode] + self.up_step_db
            target = max(target, min(implied, self.margin_db + self.max_jump_db * share))
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
        and the climb makes them up in a burst if the channel allows.

        On the floor the steps in hand are the decibels they keep, not the rungs: the first
        burst goes at the fastest rung the measurement clears by as much as the steps below
        the fit would have kept at the fit's own edge. The floor's top rung fits a measurement
        up to six or seven decibels above it, because the rung above is the first OFDM one, and
        counted in rungs the steps took a session the call had measured with ten decibels to
        spare down to half the rate — WC4Y's Test of 2026-10-11, its message at tone-36 on a
        path that carried tone4x100-75 in every burst after (ADR-0066). Among the OFDM rungs,
        a decibel or two apart, the rungs are what ADR-0008 measured and stay the rule."""
        ordinary = self._first_ordinary()
        fit = 0
        for i in range(1, len(self.modes)):
            if self._required(i) > snr_db or self._echo_out(i):
                break
            fit = i
        # the steps in hand stay in the family the measurement fits: a session the SNR puts
        # on an OFDM rung does not start on the floor, five times slower, for its caution
        lowest = ordinary if fit >= ordinary else 0
        back = max(lowest, fit - self.first_mode_back)
        if fit < ordinary:
            keep = self._required(fit) - self._required(back)
            for i in range(fit, back, -1):
                if self._required(i) + keep <= snr_db:
                    return self.modes[i]
        return self.modes[back]

    def _required(self, index: int) -> float:
        """The SNR a step up to the rung at ``index`` asks for: its threshold, the margin it is
        held to and the hysteresis."""
        return self.thresholds[self.modes[index]] + self.margin_db + self.up_hysteresis_db

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

    def _fits(self, index: int, extra_db: float = 0.0) -> bool:
        if self._echo_out(index):
            return False
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
