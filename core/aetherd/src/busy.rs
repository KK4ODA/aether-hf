//! Is the channel in use?
//!
//! A station that transmits over someone else's contact is the single most effective way to
//! make a new mode unwelcome on a shared band, so nothing here transmits without asking this
//! first. The answer has to be right in both directions: a detector that cries busy on noise
//! makes the mode unusable, and one that misses a weak signal makes it a nuisance.
//!
//! # How the decision is made
//!
//! Occupancy is a power measurement against a noise floor the detector has to learn, because
//! an HF noise floor moves by tens of dB between bands, hours and antennas — no fixed
//! threshold in dBm or dBFS can work. The floor is tracked by **minimum statistics**: the
//! smallest smoothed power seen over a window of several seconds. The reasoning is Martin's
//! (*Noise power spectral density estimation based on optimal smoothing and minimum
//! statistics*, IEEE Trans. Speech and Audio Processing 9(5), 2001): any signal only ever
//! *adds* power, so over a window longer than a signal's gaps the minimum is the noise alone.
//! A window of a few seconds is long enough to see between syllables of speech and between
//! the frames of a digital burst, and short enough to follow a fade.
//!
//! On top of that:
//!
//! * a **hangover** keeps the channel marked busy for a moment after the power drops, so the
//!   gaps inside a burst do not read as a free channel;
//! * a detected preamble marks the channel busy outright, because the receiver being able to
//!   acquire a frame settles the question better than any power measurement can;
//! * the detector is **told when this station transmits**, and discards those blocks. Its own
//!   sidetone is not occupancy, and letting it into the floor estimate would poison it.

use aether_phy::Complex;

/// Settings for [`BusyDetector`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusyConfig {
    /// Baseband sample rate the blocks arrive at.
    pub fs: f64,
    /// How much audio each measurement covers, in seconds.
    pub block_s: f64,
    /// How far back the floor estimate looks, in seconds. Must cover the longest silence a
    /// signal contains, or the floor creeps up into the signal.
    pub floor_window_s: f64,
    /// How far above the floor counts as occupied, in dB.
    pub threshold_db: f64,
    /// How long the channel stays marked busy after the power falls back.
    pub hang_s: f64,
    /// How long a detected frame keeps the channel marked busy.
    pub frame_hold_s: f64,
}

impl Default for BusyConfig {
    fn default() -> Self {
        Self {
            fs: 8000.0,
            block_s: 0.025,
            // long enough to see between the frames of somebody else's burst and between
            // the syllables of a voice contact
            floor_window_s: 5.0,
            // measured acquisition works well below this, so the frame signal is what catches
            // a weak Aether station; this catches everything else on the channel
            threshold_db: 6.0,
            hang_s: 0.75,
            frame_hold_s: 2.0,
        }
    }
}

/// Channel occupancy from the received baseband.
#[derive(Debug, Clone)]
pub struct BusyDetector {
    config: BusyConfig,
    block_samples: usize,
    window_blocks: usize,
    /// Smoothed power per block, newest last, capped at `window_blocks`.
    history: std::collections::VecDeque<f64>,
    partial: Vec<Complex>,
    smoothed: f64,
    busy_until: f64,
    /// Latest block power, in dB relative to full scale.
    pub level_db: f64,
    /// Latest floor estimate, in dB relative to full scale.
    pub floor_db: f64,
    /// Blocks discarded because this station was transmitting.
    pub blocks_skipped: usize,
}

/// Power floor for the logarithm, so silence gives a very negative number rather than
/// negative infinity.
const FLOOR: f64 = 1e-20;
/// One-pole smoothing of the block power, in blocks. Two blocks is 50 ms: long enough to ride
/// out a single noisy measurement, short enough to catch the start of a burst.
const SMOOTHING: f64 = 2.0;

impl BusyDetector {
    /// Build a detector.
    ///
    /// # Panics
    /// If the block length or the floor window works out to nothing, which would mean a
    /// configuration with a zero sample rate or a zero-length block.
    #[must_use]
    pub fn new(config: BusyConfig) -> Self {
        let block_samples = (config.block_s * config.fs) as usize;
        assert!(
            block_samples > 0,
            "a measurement block must contain samples"
        );
        let window_blocks = (config.floor_window_s / config.block_s) as usize;
        assert!(window_blocks > 0, "the floor window must span a block");
        Self {
            config,
            block_samples,
            window_blocks,
            history: std::collections::VecDeque::with_capacity(window_blocks),
            partial: Vec::with_capacity(block_samples),
            smoothed: 0.0,
            busy_until: f64::NEG_INFINITY,
            level_db: f64::NEG_INFINITY,
            floor_db: f64::NEG_INFINITY,
            blocks_skipped: 0,
        }
    }

    /// The settings in use.
    #[must_use]
    pub fn config(&self) -> BusyConfig {
        self.config
    }

    /// Change how far above the floor counts as occupied.
    ///
    /// Safe to do while running: the floor estimate and its history are unaffected, only the
    /// line drawn across them.
    pub fn set_threshold_db(&mut self, threshold_db: f64) {
        self.config.threshold_db = threshold_db;
    }

    /// Whether the channel is occupied, as of `now`.
    #[must_use]
    pub fn busy(&self, now: f64) -> bool {
        now < self.busy_until
    }

    /// How far the current level sits above the learned floor, in dB.
    #[must_use]
    pub fn excess_db(&self) -> f64 {
        self.level_db - self.floor_db
    }

    /// Whether enough audio has gone by for the floor estimate to mean anything.
    ///
    /// Before this a caller should treat the channel as busy: not knowing is not the same as
    /// knowing it is clear, and the safe reading of "I have not listened yet" is to wait.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.history.len() >= self.window_blocks / 2
    }

    /// Mark the channel busy because a frame was detected.
    ///
    /// The receiver acquiring a preamble is stronger evidence than any power measurement, and
    /// it works below the threshold, which is the case that matters for politeness.
    pub fn mark_frame(&mut self, now: f64) {
        self.busy_until = self.busy_until.max(now + self.config.frame_hold_s);
    }

    /// Discard audio captured while this station was transmitting.
    ///
    /// A half-duplex station hears its own sidetone; letting that into the floor estimate
    /// would raise the floor by tens of dB and blind the detector for the whole window.
    pub fn skip(&mut self, samples: &[Complex]) {
        self.partial.clear();
        self.blocks_skipped += samples.len() / self.block_samples.max(1);
    }

    /// Feed received baseband, and get the occupancy as of the end of it.
    pub fn push(&mut self, samples: &[Complex], now: f64) -> bool {
        self.partial.extend_from_slice(samples);
        let mut consumed = 0;
        while self.partial.len() - consumed >= self.block_samples {
            let block = &self.partial[consumed..consumed + self.block_samples];
            consumed += self.block_samples;
            let power = block
                .iter()
                .map(|&(re, im)| re.mul_add(re, im * im))
                .sum::<f64>()
                / self.block_samples as f64;

            // one-pole smoothing, started at the first block rather than from zero so the
            // estimate does not have to climb out of silence it never heard
            self.smoothed = if self.history.is_empty() {
                power
            } else {
                self.smoothed + (power - self.smoothed) / SMOOTHING
            };
            self.history.push_back(self.smoothed);
            if self.history.len() > self.window_blocks {
                self.history.pop_front();
            }

            let floor = self
                .history
                .iter()
                .copied()
                .fold(f64::INFINITY, f64::min)
                .max(FLOOR);
            self.level_db = 10.0 * self.smoothed.max(FLOOR).log10();
            self.floor_db = 10.0 * floor.log10();
            if self.level_db - self.floor_db >= self.config.threshold_db {
                self.busy_until = self.busy_until.max(now + self.config.hang_s);
            }
        }
        self.partial.drain(..consumed);
        self.busy(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic complex Gaussian noise, so a failure is always reproducible.
    fn noise(n: usize, sigma: f64, seed: u64) -> Vec<Complex> {
        let mut state = seed | 1;
        let mut next = || {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            let value = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
            ((value >> 11) as f64 / (1u64 << 53) as f64).mul_add(1.0 - 1e-12, 1e-12)
        };
        (0..n)
            .map(|_| {
                let (u1, u2) = (next(), next());
                let r = sigma * (-2.0 * u1.ln()).sqrt();
                let theta = 2.0 * std::f64::consts::PI * u2;
                (r * theta.cos(), r * theta.sin())
            })
            .collect()
    }

    /// Feed `seconds` of noise at `sigma`, a block at a time, and return the final verdict.
    fn feed(detector: &mut BusyDetector, seconds: f64, sigma: f64, seed: u64, t0: f64) -> f64 {
        let fs = detector.config().fs;
        let block = detector.config().block_s;
        let per_block = (block * fs) as usize;
        let blocks = (seconds / block) as usize;
        let mut now = t0;
        for index in 0..blocks {
            let samples = noise(per_block, sigma, seed.wrapping_add(index as u64));
            now += block;
            detector.push(&samples, now);
        }
        now
    }

    #[test]
    fn a_quiet_channel_reads_clear() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 1, 0.0);
        assert!(detector.settled());
        assert!(
            !detector.busy(now),
            "called a quiet channel busy at {:.1} dB over a {:.1} dB floor",
            detector.level_db,
            detector.floor_db
        );
    }

    #[test]
    fn a_signal_well_above_the_floor_reads_busy() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 2, 0.0);
        assert!(!detector.busy(now));
        // 20 dB above the noise it has been listening to
        let now = feed(&mut detector, 0.5, 0.1, 99, now);
        assert!(
            detector.busy(now),
            "missed a signal {:.1} dB over the floor",
            detector.excess_db()
        );
    }

    #[test]
    fn the_floor_follows_the_band_rather_than_a_fixed_level() {
        // the whole reason the floor is learned: an HF noise floor moves by tens of dB
        // between bands and hours, and a detector with a fixed threshold is useless on one
        // of them
        for sigma in [0.001, 0.01, 0.1] {
            let mut detector = BusyDetector::new(BusyConfig::default());
            let now = feed(&mut detector, 8.0, sigma, 5, 0.0);
            assert!(
                !detector.busy(now),
                "sigma {sigma}: quiet channel read busy at {:.1} dB",
                detector.excess_db()
            );
            let now = feed(&mut detector, 0.5, sigma * 10.0, 77, now);
            assert!(
                detector.busy(now),
                "sigma {sigma}: missed a signal 20 dB up"
            );
        }
    }

    #[test]
    fn the_hangover_covers_the_gaps_inside_a_burst() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 3, 0.0);
        let now = feed(&mut detector, 0.3, 0.1, 31, now);
        assert!(detector.busy(now));
        // the quarter-second gap between two frames of somebody else's burst
        assert!(
            detector.busy(now + 0.25),
            "the channel read clear in the gap between frames"
        );
    }

    #[test]
    fn a_detected_frame_marks_the_channel_busy_on_its_own() {
        // acquisition works far below the power threshold, and a receiver that can decode a
        // station is the strongest possible evidence that the station is there
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 4, 0.0);
        assert!(!detector.busy(now));
        detector.mark_frame(now);
        assert!(detector.busy(now));
        assert!(detector.busy(now + 1.5));
        assert!(!detector.busy(now + 2.5), "it never let go");
    }

    #[test]
    fn our_own_transmission_does_not_reach_the_floor_estimate() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 6, 0.0);
        let floor_before = detector.floor_db;
        let level_before = detector.level_db;

        // a loud transmission of our own, handed to `skip` rather than `push`
        let sidetone = noise(8000, 1.0, 123);
        detector.skip(&sidetone);
        assert!(detector.blocks_skipped > 0);

        assert!(
            (detector.floor_db - floor_before).abs() < 1e-12,
            "our own sidetone moved the floor"
        );
        assert!((detector.level_db - level_before).abs() < 1e-12);
        assert!(!detector.busy(now + 1.0));
    }

    #[test]
    fn nothing_is_claimed_before_the_detector_has_listened() {
        let detector = BusyDetector::new(BusyConfig::default());
        assert!(
            !detector.settled(),
            "a detector that has heard nothing must not claim to know the floor"
        );
    }

    #[test]
    fn blocks_are_assembled_across_ragged_feeds() {
        // a sound card hands over whatever buffer size it likes, and that must not change
        // what the detector concludes
        let quiet = BusyConfig::default();
        let mut whole = BusyDetector::new(quiet);
        let mut ragged = BusyDetector::new(quiet);
        let samples = noise(8000, 0.02, 8);

        whole.push(&samples, 1.0);
        let mut offset = 0;
        for size in [37usize, 512, 1, 999, 128] {
            let end = (offset + size).min(samples.len());
            ragged.push(&samples[offset..end], 1.0);
            offset = end;
        }
        ragged.push(&samples[offset..], 1.0);

        assert!(
            (whole.level_db - ragged.level_db).abs() < 1e-9,
            "{} vs {}",
            whole.level_db,
            ragged.level_db
        );
        assert!((whole.floor_db - ragged.floor_db).abs() < 1e-9);
    }
}
