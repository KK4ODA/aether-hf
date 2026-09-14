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

/// Minimum usable SNR (3 kHz, 10 % frame error rate) per mode on AWGN.
///
/// Every entry is measured (`bench/baselines/phy_fer_awgn14.csv`). Interpolated guesses used
/// to sit here and were optimistic by up to 1.4 dB on the 64-QAM modes, which the rate
/// controller had no way to discover except by losing frames.
pub const AWGN_THRESHOLD_DB: [f64; 14] = [
    -5.1, -3.2, -1.8, -0.4, 1.4, 2.9, 4.7, 6.9, 6.0, 8.9, 9.9, 13.9, 15.6, 16.9,
];

/// Payload bytes per frame for each mode, on the LONG layout.
pub const PAYLOAD_BYTES: [usize; 14] = [
    26, 46, 70, 95, 144, 193, 217, 291, 291, 389, 438, 585, 658, 732,
];

/// Modes on the throughput/threshold Pareto front, ascending.
///
/// A mode another mode beats on both counts is never worth choosing; mode 7 (8-PSK 2/3) is in
/// that position, beaten by mode 8 (16-QAM 1/2) on the same payload at a lower threshold.
#[must_use]
pub fn usable_modes() -> Vec<usize> {
    (0..AWGN_THRESHOLD_DB.len())
        .filter(|&m| {
            !(0..AWGN_THRESHOLD_DB.len()).any(|other| {
                other != m
                    && PAYLOAD_BYTES[other] >= PAYLOAD_BYTES[m]
                    && AWGN_THRESHOLD_DB[other] <= AWGN_THRESHOLD_DB[m]
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
    /// Margin decrease per decay step.
    pub down_step_db: f64,
    /// Clean bursts per decay step, once a failure has taught the margin something.
    pub decay_every: usize,
    /// Extra headroom demanded before stepping up — the anti-oscillation mechanism.
    pub up_hysteresis_db: f64,
    /// Consecutive clean bursts required before any upshift.
    pub up_dwell: usize,
    /// Most modes to climb at once, so a link never leaps onto an untried mode.
    pub max_up_step: usize,
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
            decay_every: 3,
            up_hysteresis_db: 1.5,
            up_dwell: 1,
            max_up_step: 2,
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
}

impl Default for RateController {
    fn default() -> Self {
        Self::new(RateConfig::default())
    }
}

impl RateController {
    /// Build one.
    #[must_use]
    pub fn new(config: RateConfig) -> Self {
        Self {
            margin_db: config.margin_db,
            config,
            thresholds: AWGN_THRESHOLD_DB.to_vec(),
            modes: usable_modes(),
            smoothed_snr_db: None,
            index: 0,
            clean_run: 0,
            clean_since_decay: 0,
            ever_failed: false,
        }
    }

    /// The mode to use.
    #[must_use]
    pub fn recommend(&self) -> usize {
        self.modes[self.index]
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
            self.step_down();
        } else if ok > 0 {
            self.clean_run += 1;
            self.clean_since_decay += 1;
            let period = if self.ever_failed {
                self.config.decay_every
            } else {
                1
            };
            if self.clean_since_decay >= period {
                self.clean_since_decay = 0;
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
            Some(snr) => self.thresholds[self.modes[index]] + self.margin_db + extra_db <= snr,
        }
    }

    /// A failure: fall to the fastest mode the SNR and margin still support, but always at
    /// least one step — a failure at the bottom of the table is still evidence.
    fn step_down(&mut self) {
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
    fn mode_seven_is_dominated_and_never_recommended() {
        let modes = usable_modes();
        assert!(!modes.contains(&7), "{modes:?}");
        assert_eq!(modes.len(), 13);
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
    fn a_clean_link_converges_in_a_few_bursts() {
        for snr in [0.0, 8.0, 14.0, 20.0] {
            let final_mode = settle(snr);
            let mut rc = RateController::default();
            let mut bursts = 0;
            for index in 1..=20 {
                rc.observe(Some(snr), 6, 0, None);
                if rc.recommend() == final_mode {
                    bursts = index;
                    break;
                }
            }
            assert!(
                bursts > 0 && bursts <= 8,
                "snr {snr}: {bursts} bursts to settle"
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
        // mode 4 needs +1.4 dB and failed at +12: this channel costs about 11 dB more
        rc.observe(Some(12.0), 0, 4, Some(4));
        assert!(rc.margin_db() - start > rc.config.up_step_db);
        assert!(rc.margin_db() - start <= rc.config.max_jump_db);
    }

    #[test]
    fn a_learned_margin_is_sticky_then_decays() {
        let mut rc = RateController::default();
        rc.observe(Some(12.0), 0, 4, Some(4));
        let learned = rc.margin_db();
        for _ in 0..rc.config.decay_every - 1 {
            rc.observe(Some(12.0), 6, 0, Some(4));
        }
        assert!(
            (rc.margin_db() - learned).abs() < 1e-12,
            "gave up too early"
        );
        rc.observe(Some(12.0), 6, 0, Some(4));
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
    fn with_no_measurement_it_recommends_the_most_robust_mode() {
        let rc = RateController::default();
        assert_eq!(rc.recommend(), usable_modes()[0]);
        assert_eq!(rc.snr_db(), None);
    }
}
