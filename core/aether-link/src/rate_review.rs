//! The rate controller's review, Gate 3: a prototype beside [`RateController`] — the model's
//! `aether_model.link.rate_review.OuterLoopController` (`docs/research/rate-control/
//! 03-candidates.md` §2.4, `04-results.md`), ported so the scenario harness can judge it.
//!
//! Bench-only. [`Controller`](crate::granted::Controller) selects it, and a station never runs
//! it on the air: the default is the current controller.
//!
//! [`RateController`]: crate::rate::RateController

use crate::rate::usable_modes_by_rate;

/// One data frame of a transmission, as the receiving station saw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameOutcome {
    /// The rung it was read at (or sent at, for one never detected).
    pub mode: usize,
    /// Its air time: a frame never detected took its slot all the same.
    pub air_s: f64,
    /// The data bytes a frame at this rung carries.
    pub capacity: usize,
    /// Reported by the PHY at all.
    pub detected: bool,
    /// Delivered by this copy, alone or combined with earlier ones.
    pub decoded: bool,
    /// Combined with copies held.
    pub combined: bool,
}

/// The prior's chance that a frame decodes at `snr_db`: a logistic through 90 % at the rung's
/// AWGN 10 % point, `slope_db` wide.
#[must_use]
pub fn decode_probability(snr_db: f64, threshold_db: f64, slope_db: f64) -> f64 {
    let x = (snr_db - threshold_db) / slope_db;
    if x < -40.0 {
        return 0.0;
    }
    1.0 / (1.0 + (-x).exp() / 9.0)
}

/// C2: outer-loop adaptation to a derived target. One offset per family (the tone floor,
/// OFDM), moved per frame sent at the rung recommended — up by `step_db` when its copy failed
/// or was never detected, down by `step_db · T / (1 − T)` when it decoded, where `T`, the error
/// rate at which a rung and the one below deliver alike, is derived from the family's rate
/// ratios — and the rung with the most expected delivery: its payload rate times its chance of
/// decoding at the reading less its family's offset and the prior's margin.
#[derive(Debug, Clone)]
pub struct OuterLoopController {
    thresholds: Vec<f64>,
    modes: Vec<usize>,
    capacity: Vec<usize>,
    frame_s: Vec<f64>,
    floor_modes: usize,
    echo_limit: Vec<Option<f64>>,
    echo: Option<f64>,
    smoothed_snr_db: Option<f64>,
    index: usize,
    /// Offsets by family: `[floor, ordinary]`; `None` until moved.
    offset: [Option<f64>; 2],
    target: [f64; 2],
    step_db: f64,
    slope_db: f64,
    start_offset_db: f64,
    prior_margin_db: f64,
    echo_weight: f64,
    echo_min_snr_db: f64,
}

impl OuterLoopController {
    /// One for an air's table: the AWGN thresholds, the data bytes a frame carries and its air
    /// time, by rung.
    ///
    /// # Panics
    /// If the table's columns differ in length or are empty.
    #[must_use]
    pub fn for_table(
        thresholds: &[f64],
        payload: &[usize],
        capacity: &[usize],
        frame_s: &[f64],
        floor_modes: usize,
        echo_limit: &[Option<f64>],
    ) -> Self {
        assert!(
            !thresholds.is_empty()
                && thresholds.len() == payload.len()
                && thresholds.len() == capacity.len()
                && thresholds.len() == frame_s.len(),
            "a mode table has one threshold, payload, capacity and air time per mode"
        );
        let mut c = Self {
            thresholds: thresholds.to_vec(),
            modes: usable_modes_by_rate(thresholds, payload, frame_s),
            capacity: capacity.to_vec(),
            frame_s: frame_s.to_vec(),
            floor_modes,
            echo_limit: echo_limit.to_vec(),
            echo: None,
            smoothed_snr_db: None,
            index: 0,
            offset: [None, None],
            target: [0.1, 0.1],
            step_db: 0.5,
            slope_db: 0.5,
            start_offset_db: 0.0,
            prior_margin_db: 3.0,
            echo_weight: 0.4,
            echo_min_snr_db: -6.0,
        };
        for floor in [true, false] {
            let rates: Vec<f64> = c
                .modes
                .iter()
                .filter(|&&m| c.is_floor(m) == floor)
                .map(|&m| c.lossless(m))
                .collect();
            let shares: Vec<f64> = rates
                .windows(2)
                .filter(|w| w[1] > 0.0)
                .map(|w| 1.0 - w[0] / w[1])
                .collect();
            if !shares.is_empty() {
                #[allow(clippy::cast_precision_loss)]
                let mean = shares.iter().sum::<f64>() / shares.len() as f64;
                c.target[usize::from(!floor)] = mean.clamp(0.05, 0.5);
            }
        }
        c
    }

    const fn is_floor(&self, mode: usize) -> bool {
        mode < self.floor_modes
    }

    /// Data bytes a second of a rung's frames carries, nothing lost.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn lossless(&self, mode: usize) -> f64 {
        self.capacity.get(mode).copied().unwrap_or(0) as f64
            / self.frame_s.get(mode).copied().unwrap_or(1.0)
    }

    /// The family's offset, dB.
    #[must_use]
    pub fn offset(&self, mode: usize) -> f64 {
        self.offset[usize::from(!self.is_floor(mode))].unwrap_or(self.start_offset_db)
    }

    /// The target error rate of a rung's family.
    #[must_use]
    pub fn target(&self, floor: bool) -> f64 {
        self.target[usize::from(!floor)]
    }

    /// The two families' offsets as moved so far, `[floor, ordinary]`: `None` for one never moved.
    #[must_use]
    pub const fn offsets(&self) -> [Option<f64>; 2] {
        self.offset
    }

    fn expected(&self, mode: usize, snr_db: Option<f64>) -> f64 {
        let Some(snr) = snr_db.or(self.smoothed_snr_db) else {
            return 0.0;
        };
        let p = decode_probability(
            snr - self.offset(mode) - self.prior_margin_db,
            self.thresholds[mode],
            self.slope_db,
        );
        self.lossless(mode) * p
    }

    /// The usable rungs, ascending.
    #[must_use]
    pub fn modes(&self) -> &[usize] {
        &self.modes
    }

    /// The rung to use.
    #[must_use]
    pub fn recommend(&self) -> usize {
        self.modes[self.index]
    }

    /// The smoothed SNR.
    #[must_use]
    pub const fn snr_db(&self) -> Option<f64> {
        self.smoothed_snr_db
    }

    /// What a recording calls the margin: the recommended rung's family's offset.
    #[must_use]
    pub fn margin_db(&self) -> f64 {
        self.offset(self.recommend())
    }

    /// The path's echo as the frames have read it, smoothed.
    #[must_use]
    pub const fn echo(&self) -> Option<f64> {
        self.echo
    }

    /// The threshold of a rung, if the table has one.
    #[must_use]
    pub fn threshold_db(&self, mode: usize) -> Option<f64> {
        self.thresholds.get(mode).copied()
    }

    /// Take an SNR reading and nothing else.
    pub fn observe_snr(&mut self, snr_db: Option<f64>) {
        if let Some(value) = snr_db {
            self.smoothed_snr_db = Some(
                self.smoothed_snr_db
                    .map_or(value, |prev| 0.7 * prev + 0.3 * value),
            );
        }
    }

    /// A transmission's data frames: each frame at the rung recommended, not combined, moves its
    /// family's offset; then the rung is chosen again.
    pub fn observe_frames(&mut self, frames: &[FrameOutcome], snr_db: Option<f64>) {
        let asked = self.recommend();
        self.observe_snr(snr_db);
        for f in frames {
            if f.mode != asked || f.combined {
                continue;
            }
            let floor = self.is_floor(f.mode);
            let target = self.target(floor);
            let slot = usize::from(!floor);
            let mut offset = self.offset[slot].unwrap_or(self.start_offset_db);
            if f.decoded {
                offset -= self.step_db * target / (1.0 - target);
            } else {
                offset += self.step_db;
            }
            self.offset[slot] = Some(offset.clamp(-5.0, 20.0));
        }
        self.choose();
    }

    /// A frame's echo reading (ADR-0087): a choice it now rules out is chosen again.
    pub fn observe_echo(&mut self, share: Option<f64>, snr_db: f64) {
        let Some(share) = share else {
            return;
        };
        if snr_db < self.echo_min_snr_db {
            return;
        }
        let w = self.echo_weight;
        self.echo = Some(self.echo.map_or(share, |prev| (1.0 - w) * prev + w * share));
        if self.echo_out(self.index) {
            self.choose();
        }
    }

    fn echo_out(&self, index: usize) -> bool {
        match (
            self.echo_limit.get(self.modes[index]).copied().flatten(),
            self.echo,
        ) {
            (Some(limit), Some(echo)) => echo > limit,
            _ => false,
        }
    }

    /// The best usable index by expected delivery at `snr_db` (the smoothed SNR if `None`), the
    /// lower of two alike.
    fn best(&self, snr_db: Option<f64>) -> Option<usize> {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.modes.len() {
            if self.echo_out(i) {
                continue;
            }
            let value = self.expected(self.modes[i], snr_db);
            if best.is_none_or(|(_, b)| value > b) {
                best = Some((i, value));
            }
        }
        best.map(|(i, _)| i)
    }

    fn choose(&mut self) {
        self.index = if self.smoothed_snr_db.is_none() {
            0
        } else {
            self.best(None).unwrap_or(0)
        };
    }

    /// The rung a session starts at from one reading.
    #[must_use]
    pub fn first_mode(&self, snr_db: f64) -> usize {
        self.modes[self.best(Some(snr_db)).unwrap_or(0)]
    }

    /// Start from the connect frame's reading, before anything has been observed.
    pub fn seed(&mut self, snr_db: f64) {
        if self.smoothed_snr_db.is_some() {
            return;
        }
        self.smoothed_snr_db = Some(snr_db);
        let first = self.first_mode(snr_db);
        self.index = self.modes.iter().position(|&m| m == first).unwrap_or(0);
    }

    /// A burst's counts: only its SNR is read; a silent slot is no news.
    pub fn observe(&mut self, snr_db: Option<f64>) {
        self.observe_snr(snr_db);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prior_passes_ninety_per_cent_at_the_threshold() {
        assert!((decode_probability(-3.2, -3.2, 0.5) - 0.9).abs() < 1e-12);
        assert!(decode_probability(-6.2, -3.2, 0.5) < 0.05);
        assert!(decode_probability(0.0, -3.2, 0.5) > 0.99);
        assert!(decode_probability(-50.0, -3.2, 0.5).abs() < f64::MIN_POSITIVE);
    }
}
