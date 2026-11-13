//! The rate controller (ADR-0094, ADR-0099): each station chooses the rung its own data goes at.
//!
//! Per rung, the data its frames delivered per second of the air they took — every copy, every
//! frame never detected — decayed, against a prior from the smoothed SNR the other station
//! reports; the choice is the best estimate per second of a whole exchange, and the rung above
//! is tried when it could beat it. Nothing recommends a rung to the other station: one
//! decision-maker, at the sender (the rate controller's second review,
//! `docs/research/rate-control-2/`). The model's `aether_model.link.rate`.
//!
//! The tables are the PHY's measurements: each rung's AWGN 10 % point, its payload and frame
//! length, the echo a rung cannot carry (ADR-0087), and the rungs worth offering at all.

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

/// The echo share (`aether_phy::tone::echo_share`) above which each rung of the 2 300 Hz
/// ladder that has one is kept off it (ADR-0087), from `tools/bench_echo.py --limits`
/// (`bench/baselines/echo.csv`): tone100-153, whose 100-baud symbols a 7 ms echo breaks. A rung
/// without one is carried on some measured path that reads nearly as much echo as any path that
/// refuses it. Mirrors the model; the vector test pins it.
pub const ECHO_LIMIT: [Option<f64>; 20] = [
    None,
    None,
    None,
    None,
    None,
    Some(0.0073),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

/// The 500 Hz ladder's [`ECHO_LIMIT`]: the four-tone kinds' 10 ms symbols at their two higher
/// code rates, refused by two rays 5 ms apart and carried 4 ms apart. Rung 2 and the OFDM rungs
/// have none; the climb stops under rung 3 all the same. Mirrors the model.
pub const NARROW_ECHO_LIMIT: [Option<f64>; 15] = [
    None,
    None,
    None,
    Some(0.0038),
    Some(0.0038),
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
    None,
];

/// The tone floor's control frame's 10 % FER point on AWGN (ADR-0013,
/// `bench/baselines/tone_floor.csv`) — the same frame on both airs.
pub const TONE_CONTROL_THRESHOLD_DB: f64 = -19.5;

/// The 2 300 Hz control frames' 10 % FER points on AWGN, indexed by family (ordinary,
/// floor): the control mode on the SHORT layout, and the tone floor's control frame — what the
/// engine asks for the other station's control frames on the floor by (07-protocol-8.md §7),
/// and the model's lossy pipe judges a control frame by; the rate controller never reads it.
/// Mirrors the model; the vector test pins it.
pub const CONTROL_THRESHOLD_DB: [f64; 2] = [-5.1, TONE_CONTROL_THRESHOLD_DB];

/// The two 500 Hz control frames' 10 % FER points on AWGN, indexed by family: the ordinary
/// SHORT frame at the control mode (`bench/baselines/floor_500.csv`) and the tone floor's.
/// Mirrors the model.
pub const NARROW_CONTROL_THRESHOLD_DB: [f64; 2] = [-4.5, TONE_CONTROL_THRESHOLD_DB];

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

/// The 500 Hz waveform's ladder (P7-0, ADR-0013, ADR-0015, ADR-0081), 3 kHz-referenced like
/// the wide one, so the two read as an operator would compare them: the tone floor's two rungs
/// (the same frames and thresholds as on the wide ladder) and its three four-tone middle kinds,
/// measured by `tools/bench_tone.py` (`bench/baselines/tone_floor.csv`), then the OFDM modes
/// from QPSK ½, measured by `tools/bench_phy.py --bandwidth 500` into
/// `bench/baselines/phy_fer_500.csv`, written by `tools/update_rate_table.py --bandwidth 500
/// --apply` into the model, and mirrored here (the vector test pins it).
pub const NARROW_AWGN_THRESHOLD_DB: [f64; 15] = [
    -19.0, -17.3, -14.3, -13.0, -11.1, -5.2, -3.6, -2.1, 0.5, -0.1, 2.0, 3.5, 6.9, 8.8, 10.4,
];

/// Payload bytes per frame of each rung of the 500 Hz ladder: the tone floor's frames are
/// five times as long as the ordinary ones, which is why [`usable_modes_by_rate`] needs
/// [`NARROW_FRAME_S`] to compare them.
pub const NARROW_PAYLOAD_BYTES: [usize; 15] = [
    24, 36, 51, 75, 93, 25, 34, 39, 53, 53, 71, 81, 109, 123, 137,
];

/// Air time of each narrow rung's DATA frame: 134 slots of 40 ms on the tone floor, its
/// middle kinds included, 34 symbols of 31 ms on the ordinary layout (the link layer's copy
/// of the frames; the vector test pins it).
pub const NARROW_FRAME_S: [f64; 15] = [
    5.36, 5.36, 5.36, 5.36, 5.36, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054, 1.054,
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
/// per second — what tells the floor's long frames from the ordinary ones. `payload` is a
/// frame's PHY payload; a mode is worth the data a DATA frame carries in it after its 3-byte
/// header (ADR-0073): counted by the payload, the 500 Hz air's QPSK ⅓ (15 bytes in 1.054 s)
/// beat tone4x100-75 (75 bytes in 5.36 s), 114 bit/s to 112, and was offered from 7 dB further
/// up; after the header it carries 91 bit/s to the tone rung's 107.
#[must_use]
pub fn usable_modes_by_rate(thresholds: &[f64], payload: &[usize], frame_s: &[f64]) -> Vec<usize> {
    let worth =
        |m: usize| payload[m].saturating_sub(crate::frames::DATA_HEADER) as f64 / frame_s[m];
    (0..thresholds.len())
        .filter(|&m| {
            !(0..thresholds.len()).any(|other| {
                other != m && worth(other) >= worth(m) && thresholds[other] <= thresholds[m]
            })
        })
        .collect()
}

/// Six transmissions' air is a try's wait exactly when six such transmissions went: a sum that
/// lands on it counts as there whatever its last bit (the model's `WAIT_TOLERANCE_S`).
const WAIT_TOLERANCE_S: f64 = 1e-9;

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

/// Bench only: the third rate review's candidate switches
/// (`docs/research/rate-control-3/03-candidates.md` §2), all off in a station on the air. The
/// model's `RateController` fields of the same names.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct RateReview {
    /// G3: a rung's prior counts in full while the rung has no frames of its own and less as
    /// they come in, until it counts for nothing once the rung has a whole try's worth.
    pub prior_steps_aside: bool,
    /// G4: a lost try none of whose frames failed does not double the wait before the next try
    /// of its rung.
    pub clean_try_keeps_wait: bool,
    /// G5: while a rung is tried, its account is not aged by the air the other rungs send beside
    /// it.
    pub try_unaged: bool,
    /// G1: the prior's margin under the reading, dB, in place of today's 3 (`None`: today's).
    pub prior_margin_db: Option<f64>,
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
/// half-life of `half_life` of its own frames' air, whatever rung that air went at, against a
/// prior from the smoothed SNR (the AWGN curve `prior_margin_db` under the reading) worth
/// `prior_frames` frames. The recommendation is the best estimate of the rungs at or under the
/// current one and the rungs above it measured; the rung above is tried — one transmission
/// recommended at it — when its lossless rate beats the current rung's estimate and
/// `probe_every` transmissions have gone since the last try of it, or at once after a clean
/// transmission ([`clean_climb`](Self::clean_climb)); a try that delivered less doubles the
/// wait, one that delivered more moves there and tries the next at once. The prior of a family's rungs is read `family_gain_db`
/// times its surprises under the reading: every frame of the family, not combined, moves the
/// family's offset by `gain × (p − decoded)`. The OFDM family is refused as a whole
/// ([`refuses`](Self::refuses), D3) when the choice has gone to the floor after OFDM frames that
/// delivered nothing. The model's `RateController`.
#[derive(Debug, Clone)]
pub struct RateController {
    thresholds: Vec<f64>,
    modes: Vec<usize>,
    capacity: Vec<usize>,
    frame_s: Vec<f64>,
    floor_modes: usize,
    /// The families' offsets, `[floor, ordinary]`.
    family: [f64; 2],
    family_gain_db: f64,
    echo_limit: Vec<Option<f64>>,
    echo: Option<f64>,
    smoothed_snr_db: Option<f64>,
    index: usize,
    accounts: Vec<Option<Account>>,
    /// Seconds of data air observed since each rung's last try ([`wait_s`](Self::wait_s)).
    since_probe: Vec<Option<f64>>,
    probe_wait: Vec<Option<usize>>,
    probing: Option<usize>,
    probe_tries: usize,
    /// Frames at the rung being tried since the try began: a try is decided once they outweigh
    /// the prior ([`try_over`](Self::try_over)).
    try_frames: usize,
    /// Of `try_frames`, those that did not decode ([`RateReview::clean_try_keeps_wait`]).
    try_lost: usize,
    /// The review's candidate switches, all off on the air.
    review: RateReview,
    /// The rung the last transmission's frames went at, when every one of them there decoded
    /// alone ([`clean_climb`](Self::clean_climb)).
    clean_at: Option<usize>,
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
    /// The last OFDM frames observed delivered nothing ([`judge_family`](Self::judge_family)).
    ofdm_failed: bool,
}

impl RateController {
    /// One for an air's table.
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
        let n = thresholds.len();
        Self {
            thresholds: thresholds.to_vec(),
            modes: usable_modes_by_rate(thresholds, payload, frame_s),
            capacity: capacity.to_vec(),
            frame_s: frame_s.to_vec(),
            floor_modes,
            family: [0.0, 0.0],
            family_gain_db: 0.5,
            echo_limit: echo_limit.to_vec(),
            echo: None,
            smoothed_snr_db: None,
            index: 0,
            accounts: vec![None; n],
            since_probe: vec![None; n],
            probe_wait: vec![None; n],
            probing: None,
            probe_tries: 0,
            try_frames: 0,
            try_lost: 0,
            review: RateReview::default(),
            clean_at: None,
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
            ofdm_failed: false,
        }
    }

    /// The same controller with the review's candidate switches set (benches only).
    #[must_use]
    pub const fn with_review(mut self, review: RateReview) -> Self {
        if let Some(margin) = review.prior_margin_db {
            self.prior_margin_db = margin;
        }
        self.review = review;
        self
    }

    /// D3: whether `mode`'s family is refused on this path. The OFDM family is, when every rung
    /// of it is over its echo limit, or when the choice is a floor rung and the last OFDM frames
    /// delivered nothing (the model's `refuses`). The floor's family is never refused.
    #[must_use]
    pub fn refuses(&self, mode: usize) -> bool {
        if mode < self.floor_modes {
            return false;
        }
        if self.ofdm_failed && self.modes[self.index] < self.floor_modes {
            return true;
        }
        let mut ofdm = (0..self.modes.len()).filter(|&i| self.modes[i] >= self.floor_modes);
        let mut any = false;
        let all_out = ofdm.all(|i| {
            any = true;
            self.echo_out(i)
        });
        any && all_out
    }

    /// D3 from outcomes: the OFDM frames of a transmission delivered nothing, or some did.
    fn judge_family(&mut self, frames: &[FrameOutcome]) {
        let mut ofdm = frames
            .iter()
            .filter(|f| f.mode >= self.floor_modes)
            .peekable();
        if ofdm.peek().is_some() {
            self.ofdm_failed = !ofdm.any(|f| f.decoded);
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
        let w = self.prior_weight(&acc);
        (acc.bytes + w * prior) / (acc.air_s + w * air)
    }

    /// How many frames the prior counts as beside a rung's own
    /// ([`RateReview::prior_steps_aside`]; the model's `_prior_weight`).
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn prior_weight(&self, acc: &Account) -> f64 {
        let w = self.prior_frames;
        if !self.review.prior_steps_aside {
            return w;
        }
        let whole = (self.prior_frames as usize + 1) as f64;
        w * (1.0 - acc.frames / whole).max(0.0)
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

    /// A transmission of this station's went unanswered (C1 at the sender): the choice steps
    /// down, as the engine steps its own (R9), to a rung the echo allows, and a try ends.
    pub fn silent(&mut self, steps: usize) {
        self.cancel_try();
        let to = self.index.saturating_sub(steps);
        self.index = (0..=to).rev().find(|&i| self.usable(i)).unwrap_or(0);
    }

    /// The smoothed SNR.
    #[must_use]
    pub const fn snr_db(&self) -> Option<f64> {
        self.smoothed_snr_db
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

    fn since(&self, mode: usize) -> f64 {
        self.since_probe.get(mode).copied().flatten().unwrap_or(0.0)
    }

    /// The air to let go by before trying `mode`: its wait, in full transmissions at `mode`
    /// (the model's `_wait_s`). Counted in transmissions of whatever was sent, it was six floor
    /// transmissions before a try of OFDM.
    #[allow(clippy::cast_precision_loss)]
    fn wait_s(&self, mode: usize) -> f64 {
        let n = self.burst_frames.get(mode).copied().unwrap_or(1);
        self.wait(mode) as f64 * (n as f64 * self.frame_s.get(mode).copied().unwrap_or(1.0))
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
            let unaged = if self.review.try_unaged {
                self.probing
            } else {
                None
            };
            for (mode, old) in self.accounts.iter_mut().enumerate() {
                if mode == f.mode || Some(mode) == unaged {
                    continue;
                }
                if let Some(old) = old {
                    // a rung's account ages with the air sent at any rung, in frames of its own
                    // (the model's `observe_frames`)
                    let aged = 0.5_f64.powf(f.air_s / (self.half_life * self.frame_s[mode]));
                    old.bytes *= aged;
                    old.air_s *= aged;
                    old.frames *= aged;
                }
            }
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
        self.judge_family(frames);
        let mut sent: Vec<usize> = frames.iter().map(|f| f.mode).collect();
        sent.sort_unstable();
        sent.dedup();
        let air = frames.iter().map(|f| f.air_s).sum::<f64>();
        for i in 0..self.modes.len() {
            let mode = self.modes[i];
            self.since_probe[mode] = Some(self.since(mode) + air);
        }
        let here = self.modes[self.index];
        let mut at_here = frames.iter().filter(|f| f.mode == here).peekable();
        let clean = at_here.peek().is_some() && at_here.all(|f| f.decoded && !f.combined);
        self.clean_at = clean.then_some(here);
        for &mode in &sent {
            // frames at a rung above, sent for another reason (a copy still at its first rung,
            // the Test's measurement): news of it, as a try would be
            if Some(mode) != self.probing && mode != here && mode > here {
                let clean = frames.iter().filter(|f| f.mode == mode).all(|f| f.decoded);
                self.judge_try(mode, clean);
            }
        }
        if let Some(probe) = self.probing {
            self.probe_tries += 1;
            self.try_frames += frames.iter().filter(|f| f.mode == probe).count();
            self.try_lost += frames
                .iter()
                .filter(|f| f.mode == probe && !f.decoded)
                .count();
            if self.try_over() {
                self.end_probe(self.try_frames > 0);
            }
        }
        self.decide();
    }

    /// A try is decided once its own frames outweigh the prior — more than `prior_frames` of
    /// them — or, a frame or none a transmission, after twice the transmissions that takes: one
    /// frame is a coin toss either way (the model's `_try_over`).
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    const fn try_over(&self) -> bool {
        let enough = self.prior_frames as usize + 1;
        self.try_frames >= enough || self.probe_tries >= 2 * enough
    }

    const fn cancel_try(&mut self) {
        self.probing = None;
        self.probe_tries = 0;
        self.try_frames = 0;
        self.try_lost = 0;
    }

    fn end_probe(&mut self, measured: bool) {
        let Some(mode) = self.probing else {
            return;
        };
        let clean = self.try_lost == 0;
        self.cancel_try();
        if !measured {
            return;
        }
        if self.judge_try(mode, clean) {
            self.index = self
                .modes
                .iter()
                .position(|&m| m == mode)
                .unwrap_or(self.index);
            if self.usable(self.index + 1) {
                // a climb that works goes on: the rung above is tried at the next transmission
                let above = self.modes[self.index + 1];
                self.since_probe[above] = Some(self.wait_s(above));
            }
        }
    }

    /// What frames at `mode`, above the current rung, said: whether it now beats the current
    /// rung. If not, the next try of it waits twice as long — unless every one of the frames
    /// decoded and [`RateReview::clean_try_keeps_wait`] is on.
    fn judge_try(&mut self, mode: usize, clean: bool) -> bool {
        let here = self.modes[self.index];
        if self.value(mode) > self.value(here) {
            self.probe_wait[mode] = Some(self.probe_every);
            return true;
        }
        let wait = self.wait(mode);
        self.probe_wait[mode] = Some(if self.review.clean_try_keeps_wait && clean {
            wait
        } else {
            self.probe_wait_max.min(2 * wait)
        });
        self.since_probe[mode] = Some(0.0);
        false
    }

    fn decide(&mut self) {
        // down to any rung (the estimate of one never sent is the prior); up only to a rung
        // whose own frames outweigh its prior — a try's, or frames the sender put there itself
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.modes.len() {
            if !self.usable(i) {
                continue;
            }
            if i > self.index && self.account(self.modes[i]).frames <= self.prior_frames {
                continue;
            }
            let value = self.value(self.modes[i]);
            if best.is_none_or(|(_, b)| value > b) {
                best = Some((i, value));
            }
        }
        let Some((index, _)) = best else {
            self.index = 0;
            self.cancel_try();
            return;
        };
        self.index = index;
        let up = self.index + 1;
        if let Some(probe) = self.probing {
            if !(self.usable(up) && self.modes[up] == probe) {
                self.cancel_try(); // the choice moved from under the try
            }
            return;
        }
        if !self.usable(up) {
            return;
        }
        let mode = self.modes[up];
        if self.priced(mode, self.lossless(mode)) <= self.value(self.modes[self.index]) {
            return;
        }
        let waited = self.since(mode) >= self.wait_s(mode) - WAIT_TOLERANCE_S;
        if waited || self.clean_climb(mode) {
            self.probing = Some(mode);
            self.since_probe[mode] = Some(0.0);
        }
    }

    /// Whether the rung above, `mode`, is tried without its wait: of the current rung's family,
    /// rated above the current rung, no try of it failed since it last carried, and every frame
    /// of the last transmission at the current rung decoded alone (the model's `_clean_climb`).
    fn clean_climb(&self, mode: usize) -> bool {
        let here = self.modes[self.index];
        self.clean_at == Some(here)
            && (mode < self.floor_modes) == (here < self.floor_modes)
            && self.wait(mode) == self.probe_every
            && self.value(mode) > self.value(here)
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
            self.cancel_try();
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

    /// The rung a session starts at from one reading, no steps in hand: the optimistic prior's
    /// best. (A cautious start, by the four fading channels' mean, was rejected on the harness
    /// and deleted: `04-results.md` §3.6.)
    #[must_use]
    pub fn first_mode(&self, snr_db: f64) -> usize {
        let mut best: Option<(usize, f64)> = None;
        for i in 0..self.modes.len() {
            if self.echo_out(i) {
                continue;
            }
            let value = self.priced(self.modes[i], self.estimate_at(self.modes[i], Some(snr_db)));
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

    /// A step down after silence passes the rungs the echo rules out, as the model's does.
    #[test]
    fn a_silent_step_lands_on_a_rung_the_echo_allows() {
        let payload = [10, 20, 30, 40, 50, 60];
        let limit = [None, None, None, Some(0.0038), Some(0.0038), None];
        let mut ctl = RateController::for_table(
            &[-10.0, -8.0, -6.0, -4.0, -2.0, 0.0],
            &payload,
            &payload,
            &[1.0; 6],
            5,
            &limit,
        );
        assert_eq!(ctl.modes(), &[0, 1, 2, 3, 4, 5]);
        ctl.observe_echo(Some(0.012), 0.0);
        ctl.index = 5;
        ctl.silent(1);
        assert_eq!(ctl.recommend(), 2);
        ctl.echo = None;
        ctl.index = 5;
        ctl.silent(1);
        assert_eq!(ctl.recommend(), 4);
    }

    /// D3 (ADR-0099 §2.3): OFDM frames that delivered nothing refuse the family once the
    /// choice is a floor rung; an OFDM frame that delivers lifts it; an echo over every OFDM
    /// rung's limit refuses it whatever the choice. The model's `test_d3_*`.
    #[test]
    fn d3_refuses_the_ofdm_family_by_outcomes_and_by_the_echo() {
        let payload = [10, 20, 30, 40, 50, 60];
        let limit = [None, None, None, Some(0.004), Some(0.004), Some(0.004)];
        let mut ctl = RateController::for_table(
            &[-10.0, -8.0, -6.0, -4.0, -2.0, 0.0],
            &payload,
            &payload,
            &[1.0; 6],
            3,
            &limit,
        );
        let frames = |mode: usize, decoded: bool| {
            vec![
                FrameOutcome {
                    mode,
                    air_s: 1.0,
                    capacity: payload[mode],
                    detected: true,
                    decoded,
                    combined: false,
                };
                3
            ]
        };
        ctl.seed(6.0);
        ctl.observe_frames(&frames(4, false), Some(6.0));
        ctl.index = 2;
        assert!(ctl.refuses(3) && ctl.refuses(5) && !ctl.refuses(2));
        ctl.index = 4;
        assert!(!ctl.refuses(4), "the choice on OFDM refuses nothing");
        ctl.observe_frames(&frames(3, true), Some(6.0));
        ctl.index = 2;
        assert!(!ctl.refuses(4), "an OFDM frame that delivers lifts it");
        ctl.observe_echo(Some(0.012), 6.0);
        assert!(ctl.refuses(5), "an echo over every OFDM rung's limit");
    }
}
