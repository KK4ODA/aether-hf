//! The rate controller's review, Gate 3: a prototype beside [`RateController`] — the model's
//! `aether_model.link.rate_review.OuterLoopController` (`docs/research/rate-control/
//! 03-candidates.md` §2.4, `04-results.md`), ported so the scenario harness can judge it.
//!
//! Bench-only. [`Controller`](crate::granted::Controller) selects one, and a station never runs
//! one on the air: the default is the current controller.
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

// What each rung's 10 % point moves by on ITU-R F.1487's Good, Moderate and Poor channels, dB
// over its AWGN point: the model's tables (`aether_model.link.rate_review`, from the PHY sweeps),
// which `tests/rate_review_vectors.rs` holds these to.

/// The 2 300 Hz ladder's: by rung, Good, Moderate, Poor (the model's `WIDE_CLASS_PENALTY_DB`).
pub const WIDE_CLASS_PENALTY_DB: [[f64; 3]; 20] = [
    [7.3, 5.8, 4.5],
    [8.5, 7.0, 6.4],
    [7.4, 4.8, 4.2],
    [8.5, 6.0, 6.7],
    [6.6, 4.9, 4.7],
    [8.0, 6.9, 7.1],
    [7.1, 6.3, 4.8],
    [7.4, 7.4, 5.0],
    [7.3, 6.6, 4.1],
    [7.4, 7.4, 5.0],
    [7.3, 7.1, 4.6],
    [7.4, 7.4, 5.0],
    [7.0, 7.6, 5.0],
    [7.4, 7.4, 5.0],
    [7.7, 6.7, 6.5],
    [7.4, 7.4, 5.0],
    [7.8, 9.9, 5.0],
    [7.4, 7.4, 5.0],
    [7.4, 7.4, 5.0],
    [7.9, 7.4, 5.0],
];

/// The 500 Hz ladder's: by rung, Good, Moderate, Poor (the model's `NARROW_CLASS_PENALTY_DB`).
pub const NARROW_CLASS_PENALTY_DB: [[f64; 3]; 15] = [
    [7.3, 5.8, 4.5],
    [8.5, 7.0, 6.4],
    [6.6, 4.1, 3.5],
    [9.4, 6.8, 5.8],
    [10.4, 8.9, 7.3],
    [9.2, 8.7, 5.2],
    [9.1, 8.6, 7.6],
    [9.1, 10.1, 6.6],
    [9.5, 9.8, 6.9],
    [9.1, 9.6, 8.1],
    [10.0, 12.0, 6.9],
    [10.5, 9.8, 6.9],
    [10.1, 9.8, 6.9],
    [9.2, 9.8, 6.9],
    [10.6, 9.8, 6.9],
];

/// The table for a ladder of `rungs` rungs (20 at 2 300 Hz, 15 at 500 Hz); empty for another.
#[must_use]
pub fn class_penalty_db(rungs: usize) -> &'static [[f64; 3]] {
    match rungs {
        n if n == WIDE_CLASS_PENALTY_DB.len() => &WIDE_CLASS_PENALTY_DB,
        n if n == NARROW_CLASS_PENALTY_DB.len() => &NARROW_CLASS_PENALTY_DB,
        _ => &[],
    }
}

/// The chance a frame decodes at `snr_db` on a path not yet measured: the mean of the steady
/// channel's curve and the three fading channels' (`penalties` over the threshold, none for a
/// rung the table lacks).
#[must_use]
pub fn path_decode_probability(
    snr_db: f64,
    threshold_db: f64,
    penalties: Option<&[f64; 3]>,
    slope_db: f64,
) -> f64 {
    let mut sum = decode_probability(snr_db, threshold_db, slope_db);
    let mut n = 1.0;
    if let Some(penalties) = penalties {
        for p in penalties {
            sum += decode_probability(snr_db, threshold_db + p, slope_db);
            n += 1.0;
        }
    }
    sum / n
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

/// A rung's measured delivery, decayed: data bytes delivered, seconds of air spent, frames.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Account {
    /// Data bytes its frames delivered.
    pub bytes: f64,
    /// Seconds of air its frames took.
    pub air_s: f64,
    /// Frames sent at it.
    pub frames: f64,
}

/// C1: choose by measured delivery, at the receiver. Per rung, the data its frames delivered
/// per second of the air they took — every copy, every frame never detected — decayed with a
/// half-life of `half_life` frames sent at it, against a prior from the smoothed SNR (the AWGN
/// curve `prior_margin_db` under the reading) worth `prior_frames` frames. The recommendation
/// is the best estimate of the rungs at or under the current one and the rungs above it
/// measured; the rung above is tried — one transmission recommended at it — when its lossless
/// rate beats the current rung's estimate and `probe_every` transmissions have gone since the
/// last try of it; a try that delivered less doubles the wait, one that delivered more moves
/// there and tries the next at once. The prior of a family's rungs is read `family_gain_db`
/// times its surprises under the reading: every frame of the family, not combined, moves the
/// family's offset by `gain × (p − decoded)`. The model's `DeliveryController`.
#[derive(Debug, Clone)]
pub struct DeliveryController {
    thresholds: Vec<f64>,
    modes: Vec<usize>,
    capacity: Vec<usize>,
    frame_s: Vec<f64>,
    floor_modes: usize,
    /// The families' offsets, `[floor, ordinary]`.
    family: [f64; 2],
    family_gain_db: f64,
    /// By rung, its fading channels' 10 % points over its AWGN one: the cautious start's prior.
    penalties: Vec<[f64; 3]>,
    /// The first rung is the four channels' mean's best, and the rung above it is tried at the
    /// next transmission: off, rejected on the harness (`04-results.md` §3.6).
    cautious_start: bool,
    echo_limit: Vec<Option<f64>>,
    echo: Option<f64>,
    smoothed_snr_db: Option<f64>,
    index: usize,
    accounts: Vec<Option<Account>>,
    since_probe: Vec<Option<usize>>,
    probe_wait: Vec<Option<usize>>,
    probing: Option<usize>,
    probe_tries: usize,
    half_life: f64,
    prior_frames: f64,
    probe_every: usize,
    probe_wait_max: usize,
    slope_db: f64,
    prior_margin_db: f64,
    echo_weight: f64,
    echo_min_snr_db: f64,
    /// A control frame's length, `[floor, ordinary]`: with `burst_frames` and `gap_s` the rungs
    /// are ranked per second of a whole exchange ([`value`](Self::value)); `None`, per second of
    /// frame air.
    control_s: Option<[f64; 2]>,
    /// The two gaps of an exchange: to the answer's slot, and to the next key after it.
    gap_s: f64,
    /// By rung, the data frames a full transmission at it carries; empty: none given.
    burst_frames: Vec<usize>,
    /// Whether this station's control frames go on the floor, and whether the other's do, as
    /// the engine last said; `None`: both in the data's family.
    families: Option<(bool, bool)>,
}

impl DeliveryController {
    /// One for an air's table, as [`OuterLoopController::for_table`].
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
        penalties: &[[f64; 3]],
        echo_limit: &[Option<f64>],
    ) -> Self {
        assert!(
            !thresholds.is_empty()
                && thresholds.len() == payload.len()
                && thresholds.len() == capacity.len()
                && thresholds.len() == frame_s.len(),
            "a mode table has one threshold, payload, capacity and air time per mode"
        );
        let n = thresholds.len();
        Self {
            thresholds: thresholds.to_vec(),
            modes: usable_modes_by_rate(thresholds, payload, frame_s),
            capacity: capacity.to_vec(),
            frame_s: frame_s.to_vec(),
            floor_modes,
            family: [0.0, 0.0],
            family_gain_db: 0.5,
            penalties: penalties.to_vec(),
            cautious_start: false,
            echo_limit: echo_limit.to_vec(),
            echo: None,
            smoothed_snr_db: None,
            index: 0,
            accounts: vec![None; n],
            since_probe: vec![None; n],
            probe_wait: vec![None; n],
            probing: None,
            probe_tries: 0,
            half_life: 12.0,
            prior_frames: 2.0,
            probe_every: 6,
            probe_wait_max: 48,
            slope_db: 0.5,
            prior_margin_db: 3.0,
            echo_weight: 0.4,
            echo_min_snr_db: -6.0,
            control_s: None,
            gap_s: 0.0,
            burst_frames: Vec::new(),
            families: None,
        }
    }

    /// Ranks the rungs per second of a whole exchange: a full transmission's data frames
    /// (`burst_frames`, by rung), its header, the answer (`control_floor_s` or
    /// `control_ordinary_s` by their families) and the two gaps (`gap_s`).
    pub fn price_exchanges(
        &mut self,
        control_floor_s: f64,
        control_ordinary_s: f64,
        gap_s: f64,
        burst_frames: &[usize],
    ) {
        self.control_s = Some([control_floor_s, control_ordinary_s]);
        self.gap_s = gap_s;
        self.burst_frames = burst_frames.to_vec();
    }

    /// The families both stations' control frames go in now: this station's own, and the one
    /// it asks of the other.
    pub const fn set_families(&mut self, own_floor: bool, other_floor: bool) {
        self.families = Some((own_floor, other_floor));
    }

    /// The families the engine last said both stations' control frames go in.
    #[must_use]
    pub const fn families(&self) -> Option<(bool, bool)> {
        self.families
    }

    /// What the rungs are ranked by: [`estimate`](Self::estimate) per second of a full
    /// exchange at `mode`, or of its frames' air when no lengths were given.
    #[must_use]
    pub fn value(&self, mode: usize) -> f64 {
        self.priced(mode, self.estimate(mode))
    }

    #[allow(clippy::cast_precision_loss)]
    fn priced(&self, mode: usize, per_air: f64) -> f64 {
        let n = self.burst_frames.get(mode).copied().unwrap_or(0);
        let Some(control) = self.control_s else {
            return per_air;
        };
        if n == 0 {
            return per_air;
        }
        let floor = mode < self.floor_modes;
        let (own, other) = self.families.unwrap_or((floor, floor));
        let len = |fl: bool| control[usize::from(!fl)];
        let over = len(own) + len(other) + self.gap_s;
        let air = n as f64 * self.frame_s.get(mode).copied().unwrap_or(1.0);
        per_air * air / (air + over)
    }

    /// Data bytes a second of a rung's frames carries, nothing lost.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn lossless(&self, mode: usize) -> f64 {
        self.capacity.get(mode).copied().unwrap_or(0) as f64
            / self.frame_s.get(mode).copied().unwrap_or(1.0)
    }

    /// Data bytes a second of air at `mode` delivers, measured and prior together.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn estimate(&self, mode: usize) -> f64 {
        self.estimate_at(mode, self.smoothed_snr_db)
    }

    #[allow(clippy::cast_precision_loss)]
    fn estimate_at(&self, mode: usize, snr_db: Option<f64>) -> f64 {
        let cap = self.capacity.get(mode).copied().unwrap_or(0) as f64;
        let air = self.frame_s.get(mode).copied().unwrap_or(1.0);
        let prior = snr_db.map_or(0.0, |snr| cap * self.prior_p(mode, snr));
        let acc = self.account(mode);
        let w = self.prior_frames;
        (acc.bytes + w * prior) / (acc.air_s + w * air)
    }

    /// The prior's chance a frame at `mode` decodes at `snr_db`, its family's offset under the
    /// reading.
    fn prior_p(&self, mode: usize, snr_db: f64) -> f64 {
        let offset = self.family[usize::from(mode >= self.floor_modes)];
        decode_probability(
            snr_db - offset - self.prior_margin_db,
            self.thresholds[mode],
            self.slope_db,
        )
    }

    /// The families' offsets, `[floor, ordinary]`.
    #[must_use]
    pub const fn family_offsets(&self) -> [f64; 2] {
        self.family
    }

    /// A rung's account so far (zero for one never sent at).
    #[must_use]
    pub fn account(&self, mode: usize) -> Account {
        self.accounts
            .get(mode)
            .copied()
            .flatten()
            .unwrap_or_default()
    }

    /// The usable rungs, ascending.
    #[must_use]
    pub fn modes(&self) -> &[usize] {
        &self.modes
    }

    /// The rung to use: the one being tried, else the choice.
    #[must_use]
    pub fn recommend(&self) -> usize {
        self.probing.unwrap_or(self.modes[self.index])
    }

    /// The choice, whatever is being tried.
    #[must_use]
    pub fn choice(&self) -> usize {
        self.modes[self.index]
    }

    /// The rung being tried, if one is.
    #[must_use]
    pub const fn probing(&self) -> Option<usize> {
        self.probing
    }

    /// The smoothed SNR.
    #[must_use]
    pub const fn snr_db(&self) -> Option<f64> {
        self.smoothed_snr_db
    }

    /// What a recording calls the margin: the prior's.
    #[must_use]
    pub const fn margin_db(&self) -> f64 {
        self.prior_margin_db
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

    /// A burst's counts: only its SNR is read; a silent slot is no news.
    pub fn observe(&mut self, snr_db: Option<f64>) {
        self.observe_snr(snr_db);
    }

    fn usable(&self, index: usize) -> bool {
        index < self.modes.len() && !self.echo_out(index)
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

    fn since(&self, mode: usize) -> usize {
        self.since_probe.get(mode).copied().flatten().unwrap_or(0)
    }

    fn wait(&self, mode: usize) -> usize {
        self.probe_wait
            .get(mode)
            .copied()
            .flatten()
            .unwrap_or(self.probe_every)
    }

    /// A transmission's data frames: each rung's account takes its frames, a try ends, and the
    /// recommendation is the best estimate around the current rung.
    pub fn observe_frames(&mut self, frames: &[FrameOutcome], snr_db: Option<f64>) {
        self.observe_snr(snr_db);
        if frames.is_empty() {
            return;
        }
        if self.family_gain_db != 0.0
            && let Some(snr) = self.smoothed_snr_db
        {
            for f in frames {
                if f.combined || f.mode >= self.thresholds.len() {
                    continue;
                }
                let p = self.prior_p(f.mode, snr);
                let slot = usize::from(f.mode >= self.floor_modes);
                let offset = self.family[slot]
                    + self.family_gain_db * (p - if f.decoded { 1.0 } else { 0.0 });
                self.family[slot] = offset.clamp(-5.0, 20.0);
            }
        }
        let decay = 0.5_f64.powf(1.0 / self.half_life);
        #[allow(clippy::cast_precision_loss)]
        for f in frames {
            let Some(slot) = self.accounts.get_mut(f.mode) else {
                continue;
            };
            let acc = slot.get_or_insert_with(Account::default);
            acc.bytes *= decay;
            acc.air_s *= decay;
            acc.frames *= decay;
            acc.air_s += f.air_s;
            acc.frames += 1.0;
            if f.decoded {
                acc.bytes += f.capacity as f64;
            }
        }
        let mut sent: Vec<usize> = frames.iter().map(|f| f.mode).collect();
        sent.sort_unstable();
        sent.dedup();
        for i in 0..self.modes.len() {
            let mode = self.modes[i];
            self.since_probe[mode] = Some(self.since(mode) + 1);
        }
        let here = self.modes[self.index];
        for &mode in &sent {
            // frames at a rung above, sent for another reason (a copy still at its first rung,
            // the Test's measurement): news of it, as a try would be
            if Some(mode) != self.probing && mode != here && mode > here {
                self.judge_try(mode);
            }
        }
        if let Some(probe) = self.probing {
            self.probe_tries += 1;
            let measured = sent.contains(&probe);
            if measured || self.probe_tries >= 2 {
                self.end_probe(measured);
            }
        }
        if self.probing.is_none() {
            self.decide();
        }
    }

    fn end_probe(&mut self, measured: bool) {
        let Some(mode) = self.probing.take() else {
            return;
        };
        self.probe_tries = 0;
        if !measured {
            return;
        }
        if self.judge_try(mode) {
            self.index = self
                .modes
                .iter()
                .position(|&m| m == mode)
                .unwrap_or(self.index);
            if self.usable(self.index + 1) {
                // a climb that works goes on: the rung above is tried at the next transmission
                let above = self.modes[self.index + 1];
                self.since_probe[above] = Some(self.wait(above));
            }
        }
    }

    /// What frames at `mode`, above the current rung, said: whether it now beats the current
    /// rung. If not, the next try of it waits twice as long.
    fn judge_try(&mut self, mode: usize) -> bool {
        let here = self.modes[self.index];
        if self.value(mode) > self.value(here) {
            self.probe_wait[mode] = Some(self.probe_every);
            return true;
        }
        let wait = self.wait(mode);
        self.probe_wait[mode] = Some(self.probe_wait_max.min(2 * wait));
        self.since_probe[mode] = Some(0);
        false
    }

    fn decide(&mut self) {
        // down to any rung (the estimate of one never sent is the prior); up only to a rung
        // measured — by a try, or by frames the sender put there itself
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.modes.len() {
            if !self.usable(i) {
                continue;
            }
            if i > self.index && self.account(self.modes[i]).frames < 1.0 {
                continue;
            }
            let value = self.value(self.modes[i]);
            if best.is_none_or(|(_, b)| value > b) {
                best = Some((i, value));
            }
        }
        let Some((index, _)) = best else {
            self.index = 0;
            return;
        };
        self.index = index;
        let up = self.index + 1;
        if !self.usable(up) {
            return;
        }
        let mode = self.modes[up];
        if self.priced(mode, self.lossless(mode)) <= self.value(self.modes[self.index]) {
            return;
        }
        if self.since(mode) >= self.wait(mode) {
            self.probing = Some(mode);
            self.since_probe[mode] = Some(0);
        }
    }

    /// A frame's echo reading (ADR-0087): a try of a rung it rules out ends, and a choice it
    /// rules out goes to the best estimate below that it allows.
    pub fn observe_echo(&mut self, share: Option<f64>, snr_db: f64) {
        let Some(share) = share else {
            return;
        };
        if snr_db < self.echo_min_snr_db {
            return;
        }
        let w = self.echo_weight;
        self.echo = Some(self.echo.map_or(share, |prev| (1.0 - w) * prev + w * share));
        if let Some(probe) = self.probing
            && let Some(at) = self.modes.iter().position(|&m| m == probe)
            && self.echo_out(at)
        {
            self.probing = None;
            self.probe_tries = 0;
        }
        if self.echo_out(self.index) {
            let mut best: Option<(usize, f64)> = None;
            for i in 0..self.index {
                if !self.usable(i) {
                    continue;
                }
                let value = self.value(self.modes[i]);
                if best.is_none_or(|(_, b)| value > b) {
                    best = Some((i, value));
                }
            }
            self.index = best.map_or(0, |(i, _)| i);
        }
    }

    /// [`estimate`](Self::estimate) with the four channels' mean as the prior.
    #[allow(clippy::cast_precision_loss)]
    fn start_estimate(&self, mode: usize, snr_db: f64) -> f64 {
        let cap = self.capacity.get(mode).copied().unwrap_or(0) as f64;
        let air = self.frame_s.get(mode).copied().unwrap_or(1.0);
        let offset = self.family[usize::from(mode >= self.floor_modes)];
        let p = path_decode_probability(
            snr_db - offset,
            self.thresholds[mode],
            self.penalties.get(mode),
            self.slope_db,
        );
        let acc = self.account(mode);
        let w = self.prior_frames;
        (acc.bytes + w * cap * p) / (acc.air_s + w * air)
    }

    /// The rung a session starts at from one reading, no steps in hand: the four channels'
    /// mean's best (the cautious start), or the optimistic prior's.
    #[must_use]
    pub fn first_mode(&self, snr_db: f64) -> usize {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.modes.len() {
            if self.echo_out(i) {
                continue;
            }
            let value = if self.cautious_start {
                self.start_estimate(self.modes[i], snr_db)
            } else {
                self.priced(self.modes[i], self.estimate_at(self.modes[i], Some(snr_db)))
            };
            if best.is_none_or(|(_, b)| value > b) {
                best = Some((i, value));
            }
        }
        self.modes[best.map_or(0, |(i, _)| i)]
    }

    /// Start from the connect frame's reading, before anything has been observed.
    pub fn seed(&mut self, snr_db: f64) {
        if self.smoothed_snr_db.is_some() {
            return;
        }
        self.smoothed_snr_db = Some(snr_db);
        let first = self.first_mode(snr_db);
        self.index = self.modes.iter().position(|&m| m == first).unwrap_or(0);
        if self.cautious_start && self.usable(self.index + 1) {
            let above = self.modes[self.index + 1];
            self.since_probe[above] = Some(self.wait(above));
        }
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
