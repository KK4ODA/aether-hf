//! Receiver-side rate control.
//!
//! The receiving station knows two things the sender does not: the SNR it measures on every
//! frame, and whether those frames decoded. It turns that into a mode recommendation with
//! three nested loops.
//!
//! * **Inner loop** — the fastest mode whose measured AWGN threshold, plus a margin, fits
//!   under the smoothed SNR. Thresholds come from the PHY sweep; every entry is measured.
//! * **Outer loop** — the margin itself adapts. A failed burst is treated as a
//!   *measurement*: mode `m` dying at SNR `s` says this channel needs more than
//!   `s − threshold[m]` dB, so the margin jumps toward that figure rather than creeping up in
//!   fixed steps. On a fading channel, where every mode costs 6–10 dB more than the AWGN
//!   table predicts, creeping means overshooting the mode for many bursts first.
//! * **Hysteresis** — the decision is a state machine, not a lookup. Stepping *up* needs a
//!   clean burst and an extra `up_hysteresis_db` of headroom; stepping *down* happens
//!   immediately on any failure. Fast down, slow up: what stops a link parked on a mode
//!   boundary from oscillating and losing a burst to every change.
//!
//! Modes that another mode beats on *both* throughput and threshold are never recommended.

/// Minimum usable SNR (3 kHz, 10 % frame error rate) per rung of the 2 300 Hz ladder on
/// AWGN.
///
/// Every entry is measured: the tone floor's six (rungs 0–5: its own two, ADR-0013, and the
/// fast kinds, ADR-0014) by `tools/bench_tone.py` (`bench/baselines/tone_floor.csv`), at
/// equal peak power and so in the OFDM frames' reference; the OFDM modes (rungs 6–19, OFDM
/// modes 0–13) by
/// `bench_phy.py` (`bench/baselines/phy_fer_awgn14.csv`). Interpolated guesses used to sit
/// here and were optimistic by up to 1.4 dB on the 64-QAM modes, which the rate controller
/// had no way to discover except by losing frames. Mirrors the model; the vector test pins
/// it.
pub const AWGN_THRESHOLD_DB: [f64; 20] = [
    -19.0, -17.3, -16.0, -14.2, -13.1, -11.2, -5.1, -3.2, -1.8, -0.4, 1.4, 2.9, 4.7, 6.9, 6.0, 8.9,
    9.9, 13.9, 15.6, 16.9,
];

/// The tone floor's control frame's 10 % FER point on AWGN (ADR-0013,
/// `bench/baselines/tone_floor.csv`) — the same frame on both airs.
pub const TONE_CONTROL_THRESHOLD_DB: f64 = -19.5;

/// The 2 300 Hz control frames' 10 % FER points on AWGN, indexed by family (ordinary,
/// floor): the control mode on the SHORT layout, and the tone floor's control frame — what
/// the lossy pipe ([`crate::sim`]) judges a control frame by; the rate controller never
/// reads it. Mirrors the model; the vector test pins it.
pub const CONTROL_THRESHOLD_DB: [f64; 2] = [-5.1, TONE_CONTROL_THRESHOLD_DB];

/// The two 500 Hz control frames' 10 % FER points on AWGN, indexed by family: the ordinary
/// SHORT frame at the control mode (`bench/baselines/floor_500.csv`) and the tone floor's.
/// Mirrors the model.
pub const NARROW_CONTROL_THRESHOLD_DB: [f64; 2] = [-4.5, TONE_CONTROL_THRESHOLD_DB];

/// `PhyTiming::floor_margin_db` of the 2 300 Hz air (ADR-0013 §4, the link bench): its first
/// OFDM rung spreads a frame over 2.3 kHz and, with HARQ, stays productive on a fading path
/// a decibel above its 10 % point — held to that against the floor, a quarter of its rate,
/// however wide the learned margin. The 500 Hz air's first rung has a fifth of that
/// diversity, and has no cap.
pub const WIDE_FLOOR_MARGIN_DB: f64 = 1.0;

/// Payload bytes per frame of each rung of the 2 300 Hz ladder.
pub const PAYLOAD_BYTES: [usize; 20] = [
    24, 36, 51, 75, 105, 153, 26, 46, 70, 95, 144, 193, 217, 291, 291, 389, 438, 585, 658, 732,
];

/// Air time of each wide rung's DATA frame: 134 slots of 40 ms on the tone floor, its fast
/// kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy of the
/// frames; the vector test pins it) — what [`usable_modes_by_rate`] needs to compare the
/// floor's long frames with the ordinary ones.
pub const FRAME_S: [f64; 20] = [
    5.36, 5.36, 5.36, 5.36, 5.36, 5.36, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054,
    1.054, 1.054, 1.054, 1.054, 1.054, 1.054,
];

/// The 500 Hz waveform's ladder (P7-0, ADR-0013, ADR-0015), 3 kHz-referenced like the wide
/// one, so the two read as an operator would compare them: the tone floor's two rungs (the
/// same frames and thresholds as on the wide ladder) and its two four-tone middle kinds,
/// measured by `tools/bench_tone.py` (`bench/baselines/tone_floor.csv`), then the OFDM modes
/// from QPSK ⅓, measured by `tools/bench_phy.py --bandwidth 500` into
/// `bench/baselines/phy_fer_500.csv`, written by `tools/update_rate_table.py --bandwidth 500
/// --apply` into the model, and mirrored here (the vector test pins it).
pub const NARROW_AWGN_THRESHOLD_DB: [f64; 15] = [
    -19.0, -17.3, -14.3, -13.0, -6.0, -5.2, -3.6, -2.1, 0.5, -0.1, 2.0, 3.5, 6.9, 8.8, 10.4,
];

/// Payload bytes per frame of each rung of the 500 Hz ladder: the tone floor's frames are
/// five times as long as the ordinary ones, which is why [`usable_modes_by_rate`] needs
/// [`NARROW_FRAME_S`] to compare them.
pub const NARROW_PAYLOAD_BYTES: [usize; 15] = [
    24, 36, 51, 75, 15, 25, 34, 39, 53, 53, 71, 81, 109, 123, 137,
];

/// Air time of each narrow rung's DATA frame: 134 slots of 40 ms on the tone floor, its
/// middle kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy
/// of the frames; the vector test pins it).
pub const NARROW_FRAME_S: [f64; 15] = [
    5.36, 5.36, 5.36, 5.36, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054,
    1.054,
];

/// Rungs on the throughput/threshold Pareto front, ascending, for the wide ladder.
///
/// A rung another rung beats on both counts is never worth choosing; rung 13 (8-PSK 2/3) is in
/// that position, beaten by rung 14 (16-QAM 1/2) on the same payload at a lower threshold.
#[must_use]
pub fn usable_modes() -> Vec<usize> {
    usable_modes_by_rate(&AWGN_THRESHOLD_DB, &PAYLOAD_BYTES, &FRAME_S)
}

/// Modes on the throughput/threshold Pareto front of any table, ascending, comparing
/// payload bytes per frame — right when every frame is the same length.
#[must_use]
pub fn usable_modes_of(thresholds: &[f64], payload: &[usize]) -> Vec<usize> {
    let frame_s = vec![1.0; thresholds.len()];
    usable_modes_by_rate(thresholds, payload, &frame_s)
}

/// Modes on the throughput/threshold Pareto front of any table, ascending, comparing bytes
/// per second — what tells the floor's long frames from the ordinary ones.
#[must_use]
pub fn usable_modes_by_rate(thresholds: &[f64], payload: &[usize], frame_s: &[f64]) -> Vec<usize> {
    let worth = |m: usize| payload[m] as f64 / frame_s[m];
    (0..thresholds.len())
        .filter(|&m| {
            !(0..thresholds.len()).any(|other| {
                other != m && worth(other) >= worth(m) && thresholds[other] <= thresholds[m]
            })
        })
        .collect()
}

/// Tuning for [`RateController`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateConfig {
    /// Starting margin over a mode's threshold, in dB.
    pub margin_db: f64,
    /// Floor on the margin.
    pub min_margin_db: f64,
    /// Ceiling on the margin.
    pub max_margin_db: f64,
    /// Margin increase per failed burst when the failing mode is not reported.
    pub up_step_db: f64,
    /// Cap on a single targeted increase, so one deep fade cannot strand the link.
    pub max_jump_db: f64,
    /// Margin decrease at the first decay step after a failure, applied once `decay_every`
    /// clean bursts have gone by.
    pub down_step_db: f64,
    /// How much each further clean burst's decay step grows on the last (1.0 keeps the
    /// step fixed and the old every-`decay_every` cadence): once the stickiness has run
    /// its course the margin comes back at an accelerating rate, capped at
    /// `max_down_step_db`. A learned penalty is given up slowly at first and quickly once
    /// clean burst follows clean burst — what a fade that has passed, or a collision that
    /// was never a fade, looks like (ADR-0007).
    pub decay_growth: f64,
    /// Cap on a single decay step.
    pub max_down_step_db: f64,
    /// Clean bursts before the first decay step after a failure.
    pub decay_every: usize,
    /// Extra headroom demanded before stepping up — the anti-oscillation mechanism.
    pub up_hysteresis_db: f64,
    /// Consecutive clean bursts required before any upshift.
    pub up_dwell: usize,
    /// Most modes to climb at once, so a link never leaps onto an untried mode.
    pub max_up_step: usize,
    /// Steps kept in hand by [`RateController::first_mode`]: how far below the fastest mode
    /// one measurement supports a session's first burst goes out.
    pub first_mode_back: usize,
    /// How far above a lower-bound seed ([`RateController::seed`]) the first ordinary
    /// measurement must read to replace it. The tone floor's estimate is exact to about
    /// +10 dB on AWGN; a few decibels between two frames' readings is their estimates' own
    /// spread and the fade between them, and replacing a seed on that would only take the
    /// larger of two noisy numbers. Beyond it the seed was the floor's ceiling, not the
    /// path's (ADR-0016).
    pub reseed_margin_db: f64,
}

impl Default for RateConfig {
    fn default() -> Self {
        Self {
            margin_db: 3.0,
            min_margin_db: 1.5,
            max_margin_db: 12.0,
            up_step_db: 1.5,
            max_jump_db: 3.0,
            down_step_db: 0.25,
            decay_growth: 2.0,
            max_down_step_db: 1.0,
            decay_every: 3,
            up_hysteresis_db: 1.5,
            up_dwell: 1,
            max_up_step: 2,
            first_mode_back: 2,
            reseed_margin_db: 3.0,
        }
    }
}

/// Chooses the mode the sender should use.
#[derive(Debug, Clone)]
pub struct RateController {
    config: RateConfig,
    thresholds: Vec<f64>,
    modes: Vec<usize>,
    margin_db: f64,
    smoothed_snr_db: Option<f64>,
    index: usize,
    clean_run: usize,
    clean_since_decay: usize,
    ever_failed: bool,
    /// The last decay step taken since the failure before, which the next one grows on.
    decay_step_db: f64,
    /// How many of the ladder's leading rungs are the floor's — the tone floor, ADR-0013 —
    /// whose frames carry a quarter of the first OFDM rung's rate or less. The step across
    /// that boundary is not a step between neighbours a third apart in rate, which is what
    /// the learned margin was built for: see [`floor_margin_db`](Self::floor_margin_db).
    floor_modes: usize,
    /// The most margin the first OFDM rung is held to against the floor, however wide the
    /// learned margin has grown, on an air whose first rung stays productive on a fading
    /// path below it (`PhyTiming::floor_margin_db`); `None` leaves the learned margin in
    /// charge.
    floor_margin_db: Option<f64>,
    /// Failed bursts in a row on the first OFDM rung ([`step_down`](Self::step_down)).
    boundary_failures: usize,
    /// The seed was a lower bound ([`seed`](Self::seed)): the first clean burst measured on an
    /// ordinary frame seeds again.
    reseed_pending: bool,
}

impl Default for RateController {
    fn default() -> Self {
        Self::new(RateConfig::default())
    }
}

impl RateController {
    /// The modes this controller recommends from, ascending: the throughput/threshold
    /// Pareto front of its table.
    #[must_use]
    pub fn modes(&self) -> &[usize] {
        &self.modes
    }

    /// Build one for the wide waveform's ladder.
    #[must_use]
    pub fn new(config: RateConfig) -> Self {
        Self::for_table_timed(config, &AWGN_THRESHOLD_DB, &PAYLOAD_BYTES, &FRAME_S)
    }

    /// The same controller for an air whose first `floor_modes` rungs are the floor's, the
    /// first OFDM rung's margin capped at `floor_margin_db` against it (ADR-0013 §4).
    #[must_use]
    pub fn with_floor(mut self, floor_modes: usize, floor_margin_db: Option<f64>) -> Self {
        self.floor_modes = floor_modes;
        self.floor_margin_db = floor_margin_db;
        self
    }

    /// How many of the ladder's leading rungs are the floor's.
    #[must_use]
    pub fn floor_modes(&self) -> usize {
        self.floor_modes
    }

    /// The cap on the first OFDM rung's margin, if the air has one.
    #[must_use]
    pub fn floor_margin_db(&self) -> Option<f64> {
        self.floor_margin_db
    }

    /// Build one for any mode table: its thresholds decide when to step, its payloads
    /// decide which modes another mode beats on both counts and are never recommended.
    ///
    /// # Panics
    /// If the table is empty or the two slices disagree in length.
    #[must_use]
    pub fn for_table(config: RateConfig, thresholds: &[f64], payload: &[usize]) -> Self {
        let frame_s = vec![1.0; thresholds.len()];
        Self::for_table_timed(config, thresholds, payload, &frame_s)
    }

    /// The same for a table whose frames differ in length — the floor's are five times the
    /// ordinary ones (ADR-0013): modes are compared by bytes per second.
    ///
    /// # Panics
    /// If the table's columns differ in length or are empty.
    #[must_use]
    pub fn for_table_timed(
        config: RateConfig,
        thresholds: &[f64],
        payload: &[usize],
        frame_s: &[f64],
    ) -> Self {
        assert!(
            !thresholds.is_empty()
                && thresholds.len() == payload.len()
                && thresholds.len() == frame_s.len(),
            "a mode table has one threshold, one payload and one air time per mode"
        );
        Self {
            margin_db: config.margin_db,
            config,
            thresholds: thresholds.to_vec(),
            modes: usable_modes_by_rate(thresholds, payload, frame_s),
            smoothed_snr_db: None,
            index: 0,
            clean_run: 0,
            clean_since_decay: 0,
            ever_failed: false,
            decay_step_db: 0.0,
            floor_modes: 6,
            floor_margin_db: None,
            boundary_failures: 0,
            reseed_pending: false,
        }
    }

    /// The mode to use.
    #[must_use]
    pub fn recommend(&self) -> usize {
        self.modes[self.index]
    }

    /// The mode a session should start at, given one measurement and nothing else: the
    /// fastest mode that fits under `snr_db` with the margin and the hysteresis a step
    /// up would demand, less one step. The measurement is of a mode-0 frame — the most
    /// robust there is — and a burst at a fast mode is more exposed to what the channel
    /// does within a frame, so the first burst keeps `first_mode_back` steps in hand and
    /// the climb makes them up in a burst if the channel allows (P9-2, ADR-0008). The steps
    /// in hand stay in the family the measurement fits: a session the SNR puts on an OFDM
    /// rung does not start on the floor, five times slower, for its caution (ADR-0013).
    #[must_use]
    pub fn first_mode(&self, snr_db: f64) -> usize {
        let ordinary = self.first_ordinary();
        let mut fit = 0;
        for index in 1..self.modes.len() {
            if self.thresholds[self.modes[index]]
                + self.margin(index)
                + self.config.up_hysteresis_db
                > snr_db
            {
                break;
            }
            fit = index;
        }
        let lowest = if fit >= ordinary { ordinary } else { 0 };
        self.modes[fit.saturating_sub(self.config.first_mode_back).max(lowest)]
    }

    /// Index in [`modes`](Self::modes) of the first rung above the floor.
    fn first_ordinary(&self) -> usize {
        self.modes
            .iter()
            .position(|&m| m >= self.floor_modes)
            .unwrap_or(0)
    }

    /// The margin a rung is held to: the learned one, capped at `floor_margin_db` for the
    /// first OFDM rung, whose alternative is the floor.
    fn margin(&self, index: usize) -> f64 {
        match self.floor_margin_db {
            Some(cap) if index == self.first_ordinary() => self.margin_db.min(cap),
            _ => self.margin_db,
        }
    }

    /// Start from a measurement — the connect frame this station decoded — instead of
    /// from the slowest mode: the smoothed SNR becomes the measurement and the
    /// recommendation [`first_mode`](Self::first_mode). Only before anything has been
    /// observed; a controller that has seen bursts knows more than one frame can tell it.
    ///
    /// `lower_bound`: the measurement was a tone-floor frame's, which a strong path does not
    /// show — the floor's estimate is exact to about +10 dB on AWGN and saturates near +17,
    /// and on a dispersive path it reads a few decibels whatever the SNR, the echo's spill
    /// into the next symbol counting as noise (ADR-0016). Calls start on the floor, so a
    /// strong path's session would start many rungs low and climb two a burst; instead the
    /// first clean burst measured on an ordinary frame seeds again, upward only.
    pub fn seed(&mut self, snr_db: f64, lower_bound: bool) {
        if self.smoothed_snr_db.is_some() {
            return;
        }
        self.smoothed_snr_db = Some(snr_db);
        let first = self.first_mode(snr_db);
        self.index = self.modes.iter().position(|&m| m == first).unwrap_or(0);
        self.reseed_pending = lower_bound;
    }

    /// The smoothed SNR, once anything has been measured.
    #[must_use]
    pub fn snr_db(&self) -> Option<f64> {
        self.smoothed_snr_db
    }

    /// The current margin over a mode's threshold.
    #[must_use]
    pub fn margin_db(&self) -> f64 {
        self.margin_db
    }

    /// Feed one burst: the mean SNR of its frames, how many decoded and failed, and the mode
    /// they were sent in — which is what turns a failure into a measurement.
    pub fn observe(&mut self, snr_db: Option<f64>, ok: usize, failed: usize, mode: Option<usize>) {
        if self.reseed_pending
            && ok > 0
            && failed == 0
            && let (Some(value), Some(mode)) = (snr_db, mode)
            && mode >= self.floor_modes
        {
            // the first measurement a strong path can show: start again from it — upward, and
            // only past the estimates' own spread — as an ordinary connect frame would have
            self.reseed_pending = false;
            if self
                .smoothed_snr_db
                .is_none_or(|seed| value > seed + self.config.reseed_margin_db)
            {
                self.smoothed_snr_db = Some(value);
                let first = self.first_mode(value);
                let at = self.modes.iter().position(|&m| m == first).unwrap_or(0);
                self.index = self.index.max(at);
                return;
            }
        }
        if let Some(value) = snr_db {
            self.smoothed_snr_db = Some(
                self.smoothed_snr_db
                    .map_or(value, |prev| 0.7 * prev + 0.3 * value),
            );
        }
        if failed > 0 {
            self.widen(snr_db, mode);
            self.ever_failed = true;
            self.clean_run = 0;
            self.clean_since_decay = 0;
            self.decay_step_db = 0.0;
            self.step_down();
        } else if ok > 0 {
            self.boundary_failures = 0;
            self.clean_run += 1;
            self.clean_since_decay += 1;
            if !self.ever_failed {
                self.margin_db =
                    (self.margin_db - self.config.down_step_db).max(self.config.min_margin_db);
            } else if self.decay_step_db > 0.0 {
                // past the sticky bursts: every clean burst gives back more than the last
                self.decay_step_db = (self.decay_step_db * self.config.decay_growth)
                    .min(self.config.max_down_step_db);
                self.margin_db =
                    (self.margin_db - self.decay_step_db).max(self.config.min_margin_db);
            } else if self.clean_since_decay >= self.config.decay_every {
                self.clean_since_decay = 0;
                self.decay_step_db = self.config.down_step_db;
                self.margin_db =
                    (self.margin_db - self.config.down_step_db).max(self.config.min_margin_db);
            }
            self.step_up();
        }
    }

    /// A failed burst is a measurement, not just a nudge.
    fn widen(&mut self, snr_db: Option<f64>, mode: Option<usize>) {
        let mut target = self.margin_db + self.config.up_step_db;
        if let (Some(snr), Some(mode)) = (snr_db, mode) {
            if let Some(&threshold) = self.thresholds.get(mode) {
                let implied = snr - threshold + self.config.up_step_db;
                target = target.max(implied.min(self.margin_db + self.config.max_jump_db));
            }
        }
        self.margin_db = target.clamp(self.config.min_margin_db, self.config.max_margin_db);
    }

    /// Whether the mode at `index` fits under the smoothed SNR with the margin and `extra`.
    fn fits(&self, index: usize, extra_db: f64) -> bool {
        match self.smoothed_snr_db {
            None => index == 0,
            Some(snr) => self.thresholds[self.modes[index]] + self.margin(index) + extra_db <= snr,
        }
    }

    /// A failure: fall to the fastest mode the SNR and margin still support, but always at
    /// least one step — a failure at the bottom of the table is still evidence. The one
    /// exception is a single failed burst on the first OFDM rung of an air that caps its
    /// margin (`floor_margin_db`) while the SNR still carries the rung: the floor below is a
    /// quarter of the rate, so one lost burst is not worth leaving for it; a second one in a
    /// row is.
    fn step_down(&mut self) {
        let ordinary = self.first_ordinary();
        let at_boundary = self.floor_margin_db.is_some() && self.index == ordinary;
        self.boundary_failures = if at_boundary {
            self.boundary_failures + 1
        } else {
            0
        };
        if at_boundary && self.boundary_failures < 2 && self.fits(ordinary, 0.0) {
            return;
        }
        self.boundary_failures = 0;
        let mut target = self.index.saturating_sub(1);
        for candidate in (0..self.index).rev() {
            if self.fits(candidate, 0.0) {
                target = candidate;
                break;
            }
        }
        self.index = target;
    }

    fn step_up(&mut self) {
        if self.clean_run < self.config.up_dwell {
            return;
        }
        let mut target = self.index;
        for candidate in self.index + 1..self.modes.len() {
            if !self.fits(candidate, self.config.up_hysteresis_db) {
                break;
            }
            target = candidate;
        }
        if target > self.index {
            self.index = target.min(self.index + self.config.max_up_step);
            self.clean_run = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(snr_db: f64) -> usize {
        let mut rc = RateController::default();
        for _ in 0..20 {
            rc.observe(Some(snr_db), 6, 0, None);
        }
        rc.recommend()
    }

    /// Drive the controller against a channel that fails any mode needing more than
    /// `snr_db − 1` dB, returning the sequence of recommendations.
    fn boundary_track(rc: &mut RateController, snr_db: f64, bursts: usize) -> Vec<usize> {
        let mut track = Vec::with_capacity(bursts);
        for _ in 0..bursts {
            let mode = rc.recommend();
            track.push(mode);
            let failed = usize::from(AWGN_THRESHOLD_DB[mode] > snr_db - 1.0) * 3;
            rc.observe(Some(snr_db), 6 - failed, failed, Some(mode));
        }
        track
    }

    #[test]
    fn the_dominated_rungs_are_never_recommended() {
        // 8-PSK 2/3, OFDM mode 7, six rungs up the ladder since the fast kinds (ADR-0014);
        // and BPSK 1/5, OFDM mode 0, which the fastest tone kind beats on rate and threshold
        let modes = usable_modes();
        assert!(!modes.contains(&13) && !modes.contains(&6), "{modes:?}");
        assert_eq!(modes.len(), 18);
        assert!(modes.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_recommendation_climbs_with_snr() {
        let floor = usable_modes()[0];
        let mut previous = 0usize;
        for snr in (-6..22).step_by(2) {
            let mode = settle(f64::from(snr));
            assert!(mode >= previous, "snr {snr}: {mode} after {previous}");
            assert!(
                mode == floor || AWGN_THRESHOLD_DB[mode] <= f64::from(snr),
                "snr {snr}: recommended {mode}, which needs {}",
                AWGN_THRESHOLD_DB[mode]
            );
            previous = mode;
        }
    }

    #[test]
    fn a_seed_from_the_tone_floor_is_a_lower_bound() {
        // ADR-0016: calls start on the tone floor, whose SNR estimate reads low on a strong
        // path; a controller seeded from it seeds again from the first clean burst measured
        // on an ordinary frame — upward, past the estimates' own spread, and once
        let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-9);
        let mut rc = RateController::default();
        let ordinary = rc.modes()[rc.first_ordinary()];
        rc.seed(4.0, true);
        assert_eq!(rc.recommend(), rc.first_mode(4.0));
        rc.observe(Some(20.0), 6, 0, Some(0)); // a floor burst: a lower bound again
        assert!(close(rc.snr_db(), 0.7 * 4.0 + 0.3 * 20.0));
        rc.observe(Some(20.0), 3, 3, Some(ordinary)); // a failure says nothing of the path
        assert!(rc.recommend() < rc.first_mode(20.0));
        rc.observe(Some(20.0), 6, 0, Some(ordinary));
        assert!(close(rc.snr_db(), 20.0));
        assert_eq!(rc.recommend(), rc.first_mode(20.0));
        rc.observe(Some(10.0), 6, 0, Some(ordinary)); // once: then smoothed as ever
        assert!(close(rc.snr_db(), 0.7 * 20.0 + 0.3 * 10.0));

        let mut high = RateController::default(); // upward only
        high.seed(15.0, true);
        high.observe(Some(10.0), 6, 0, Some(ordinary));
        assert!(close(high.snr_db(), 0.7 * 15.0 + 0.3 * 10.0));
        let mut near = RateController::default(); // within the spread: smoothed, not replaced
        let margin = RateConfig::default().reseed_margin_db;
        near.seed(10.0, true);
        near.observe(Some(10.0 + margin), 6, 0, Some(ordinary));
        assert!(close(near.snr_db(), 0.7 * 10.0 + 0.3 * (10.0 + margin)));
        let mut plain = RateController::default(); // an ordinary frame's is not a lower bound
        plain.seed(4.0, false);
        plain.observe(Some(20.0), 6, 0, Some(ordinary));
        assert!(close(plain.snr_db(), 0.7 * 4.0 + 0.3 * 20.0));
    }

    #[test]
    fn a_clean_link_converges_in_a_few_bursts() {
        // from the bottom of the ladder two things pace the climb and nothing else: the step,
        // `max_up_step` usable modes a burst, and the margin, which gives up `down_step_db` a
        // clean burst before the first failure; one burst more for the first measurement
        let config = RateConfig::default();
        let decay = ((config.margin_db - config.min_margin_db) / config.down_step_db).ceil();
        for snr in [0.0, 8.0, 14.0, 20.0] {
            let final_mode = settle(snr);
            let mut rc = RateController::default();
            let position = rc
                .modes()
                .iter()
                .position(|&m| m == final_mode)
                .expect("a usable mode");
            let climb = position.div_ceil(config.max_up_step);
            let bound = climb.max(decay as usize) + 1;
            let mut bursts = 0;
            for index in 1..=30 {
                rc.observe(Some(snr), 6, 0, None);
                if rc.recommend() == final_mode {
                    bursts = index;
                    break;
                }
            }
            assert!(
                bursts > 0 && bursts <= bound,
                "snr {snr}: {bursts} bursts to settle, {bound} allowed"
            );
        }
    }

    #[test]
    fn it_does_not_oscillate_at_a_mode_boundary() {
        let mut rc = RateController::default();
        let track = boundary_track(&mut rc, 9.0, 40);
        let tail = &track[20..];
        assert!(tail.iter().all(|&m| m == tail[0]), "{track:?}");
        assert!(AWGN_THRESHOLD_DB[tail[0]] <= 9.0);
    }

    #[test]
    fn it_backs_off_fast_when_the_channel_collapses() {
        let mut rc = RateController::default();
        for _ in 0..20 {
            rc.observe(Some(18.0), 6, 0, None);
        }
        let high = rc.recommend();
        let track = boundary_track(&mut rc, 4.0, 12);
        assert_eq!(track[0], high);
        let sustainable = track
            .iter()
            .position(|&m| AWGN_THRESHOLD_DB[m] <= 3.0)
            .expect("never reached a sustainable mode");
        assert!(sustainable <= 4, "{track:?}");
        assert!(track[7..].iter().all(|&m| m == track[7]), "{track:?}");
    }

    #[test]
    fn a_failure_widens_the_margin_by_more_than_a_fixed_step() {
        let mut rc = RateController::default();
        let start = rc.margin_db();
        // rung 6 (QPSK ½) needs +1.4 dB and failed at +12: this channel costs about 11 dB more
        rc.observe(Some(12.0), 0, 4, Some(6));
        assert!(rc.margin_db() - start > rc.config.up_step_db);
        assert!(rc.margin_db() - start <= rc.config.max_jump_db);
    }

    #[test]
    fn a_learned_margin_is_sticky_then_decays() {
        let mut rc = RateController::default();
        rc.observe(Some(12.0), 0, 4, Some(6));
        let learned = rc.margin_db();
        for _ in 0..rc.config.decay_every - 1 {
            rc.observe(Some(12.0), 6, 0, Some(6));
        }
        assert!(
            (rc.margin_db() - learned).abs() < 1e-12,
            "gave up too early"
        );
        rc.observe(Some(12.0), 6, 0, Some(6));
        assert!(rc.margin_db() < learned, "never decays");
    }

    #[test]
    fn before_any_failure_the_margin_decays_freely() {
        let mut rc = RateController::default();
        for _ in 0..3 {
            rc.observe(Some(10.0), 6, 0, Some(0));
        }
        assert!(rc.margin_db() < RateConfig::default().margin_db - 2.0 * rc.config.down_step_db);
    }

    #[test]
    fn the_first_mode_keeps_a_step_in_hand() {
        let mut rc = RateController::default();
        // far below every mode but the slowest: the slowest
        assert_eq!(rc.first_mode(-25.0), usable_modes()[0]);
        let config = RateConfig::default();
        // within the family the fit is in: a fit on the OFDM rungs does not start on the floor
        let ordinary = rc.first_ordinary();
        for snr in [4.0, 9.0, 15.0, 20.0] {
            let modes = rc.modes().to_vec();
            let top = modes
                .iter()
                .rposition(|&m| {
                    AWGN_THRESHOLD_DB[m] + rc.margin_db() + config.up_hysteresis_db <= snr
                })
                .expect("something fits");
            let lowest = if top >= ordinary { ordinary } else { 0 };
            assert_eq!(
                rc.first_mode(snr),
                modes[top.saturating_sub(config.first_mode_back).max(lowest)],
                "{snr}"
            );
        }
        // seeding places the controller there and takes the measurement, once
        rc.seed(15.0, false);
        assert_eq!(rc.recommend(), rc.first_mode(15.0));
        rc.seed(2.0, false);
        assert_eq!(rc.snr_db(), Some(15.0));
    }

    #[test]
    fn the_floor_boundary_is_crossed_by_what_the_rungs_are_worth() {
        // ADR-0013 §4, ADR-0014: the floor's frames are five times as long as an OFDM frame;
        // the first OFDM rung is the first *usable* one — the wide ladder's BPSK 1/5 is beaten
        // by its fastest tone kind and never recommended
        let config = RateConfig::default();
        let rc = RateController::default(); // the wide ladder: rungs 0–5 are the floor
        assert_eq!(rc.floor_modes(), 6);
        assert_eq!(&rc.modes()[..6], &[0, 1, 2, 3, 4, 5]);
        let first = rc.modes()[rc.first_ordinary()];
        let second = rc.modes()[rc.first_ordinary() + 1];
        let fits_second = AWGN_THRESHOLD_DB[second] + rc.margin_db() + config.up_hysteresis_db;
        assert_eq!(
            rc.first_mode(fits_second),
            first,
            "not two steps down, on the floor"
        );
        assert!(
            rc.first_mode(-12.0) < 6,
            "nothing above the floor fits: the floor"
        );

        let wound = |cap: Option<f64>| {
            let mut rc = RateController::default().with_floor(6, cap);
            rc.seed(AWGN_THRESHOLD_DB[first] + 1.5, false);
            rc.index = rc
                .modes
                .iter()
                .position(|&m| m == first)
                .expect("the first OFDM rung");
            rc.margin_db = 8.0; // a fading channel's learned margin
            rc
        };
        let mut capped = wound(Some(1.0));
        let snr = capped.snr_db();
        capped.observe(snr, 0, 6, Some(first));
        assert_eq!(
            capped.recommend(),
            first,
            "one lost burst on the capped rung: held"
        );
        capped.observe(snr, 0, 6, Some(first));
        assert!(capped.recommend() < first, "two in a row: the floor");
        for _ in 0..3 {
            capped.observe(snr, 6, 0, None);
        }
        assert!(
            capped.recommend() < first,
            "the cap and the hysteresis are not met there"
        );
        let enough = AWGN_THRESHOLD_DB[first] + 1.0 + config.up_hysteresis_db + 0.5;
        for _ in 0..6 {
            capped.observe(Some(enough), 6, 0, None);
        }
        assert!(
            capped.recommend() >= first,
            "they are here, whatever the learned margin"
        );

        // uncapped the learned margin decides, as between any two rungs
        let mut plain = wound(None);
        let snr = plain.snr_db();
        plain.observe(snr, 0, 6, Some(first));
        assert!(plain.recommend() < first);
    }

    #[test]
    fn with_no_measurement_it_recommends_the_most_robust_mode() {
        let rc = RateController::default();
        assert_eq!(rc.recommend(), usable_modes()[0]);
        assert_eq!(rc.snr_db(), None);
    }
}
