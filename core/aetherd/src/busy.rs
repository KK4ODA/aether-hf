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
//! The minimum is taken only over blocks where the level was **steady** — the raw block
//! powers within 3 dB over the last 200 ms. A receiver's AGC cuts its gain in a millisecond when
//! something strong appears anywhere in its passband (an adjacent station, a crash outside
//! the audio filter, nothing this detector can see) and lets it back over the next half
//! second; a two-minute recording of an idle band through an FTDX10 on AGC AUTO showed
//! seven such dips of 4–18 dB, and a plain minimum held each one for the whole window,
//! which drew the noise floor as a train of square pits on the panel and put the busy
//! threshold a decibel above the real noise. Noise is stationary and the gaps in a signal
//! are stationary at the floor; a gain transient is a step down and a ramp back, and is
//! never steady — so the gate keeps the theory and drops the artefact. It is judged on the
//! raw block powers, not the smoothed level: the smoother turns a step-and-ramp into a
//! rounded valley whose bottom looks steady for a few blocks, and lags a burst's end by
//! half a dozen blocks, which would hide a short gap. The price is that a gap shorter than
//! 200 ms does not show the floor — which is no worse than before, since the smoothed
//! level the old minimum was taken over needed about that long to fall to the floor after
//! a burst, and the ARQ turnarounds of the modes on the band exceed it. When no block in the window was steady, the floor learned before is
//! held: a channel that has been busy for five seconds without a quiet moment is still
//! measured against the noise that was there before it. A receiver whose AGC recovers
//! slower than about 15 dB/s (a SLOW setting) ramps gently enough to pass the gate, and
//! its dips will show; the field notes say which settings to use.
//!
//! On top of that:
//!
//! * a **hangover** keeps the channel marked busy for a moment after the power drops, so the
//!   gaps inside a burst do not read as a free channel;
//! * a **decoded frame** marks the channel busy outright — a frame that decoded is there,
//!   whatever the power measurement says. A merely *acquired* preamble does not: on a real
//!   band the acquisition false-alarms many times a minute, and its confidence overlaps
//!   between a phantom and a weak real frame (measured on the OTA-2 recordings: phantoms
//!   to 1.54 over their threshold, a real frame at 1.38), so no gate on it is clean. The
//!   only evidence a phantom can never produce is a decode;
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

/// Why the channel was last marked busy.
///
/// The busy state is one number, `busy_until`, extended by more than one path; when the
/// indicator lights with nothing 6 dB above the floor, the question is always which path
/// did it, and this is the answer. It is reported with the state and logged on every
/// transition (`field/OTA-2-FINDINGS.md`: the busy indicator was on for seconds while the
/// operator watched a level that never crossed the threshold).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BusyReason {
    /// The smoothed level stood at least the threshold above the learned floor.
    Level {
        /// The level at the moment, dBFS.
        level_db: f64,
        /// The floor at the moment, dBFS.
        floor_db: f64,
    },
    /// A frame was acquired or decoded, at this acquisition confidence (1.0 is exactly at
    /// the detector's threshold; a decoded frame reports its own).
    Frame {
        /// Acquisition peak over the threshold that accepted it.
        detect_confidence: f64,
    },
}

impl BusyReason {
    /// A line for a log: the numbers that decided it.
    #[must_use]
    pub fn describe(&self, threshold_db: f64) -> String {
        match self {
            Self::Level { level_db, floor_db } => format!(
                "level {level_db:.1} dBFS is {:+.1} dB over the floor {floor_db:.1} (threshold {threshold_db:.1})",
                level_db - floor_db
            ),
            Self::Frame { detect_confidence } => {
                format!("a frame acquired at confidence {detect_confidence:.2}")
            }
        }
    }
}

/// Channel occupancy from the received baseband.
#[derive(Debug, Clone)]
pub struct BusyDetector {
    config: BusyConfig,
    block_samples: usize,
    window_blocks: usize,
    /// Why `busy_until` was last extended, for the operator and the log.
    reason: Option<BusyReason>,
    /// The largest level-over-floor seen since the last time it was read: the decision is
    /// made forty times a second on a 50 ms quantity and a display samples it twice a second,
    /// so the excursions that trip the threshold are the ones a display never shows.
    excess_peak_db: f64,
    /// Raw power per block and whether the level was steady when it was measured, newest
    /// last, capped at `window_blocks`.
    history: std::collections::VecDeque<(f64, bool)>,
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
/// How many blocks the level must have been steady over for a block to count toward the
/// floor: 200 ms. A receiver's AGC recovers at 40–55 dB/s on the rig this was measured
/// on, and holds the cut gain for about 100 ms first; eight blocks see through both, with
/// room for a slower rig.
const STEADY_BLOCKS: usize = 8;
/// How much the raw block power may vary over those blocks and still count as steady, in
/// dB. HF noise over 200 ms stayed within it two blocks in three on the recording; an AGC
/// ramp of 15 dB/s or faster crosses it.
const STEADY_RANGE_DB: f64 = 3.0;

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
            reason: None,
            excess_peak_db: f64::NEG_INFINITY,
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

    /// Why the channel was last marked busy, if it ever was.
    #[must_use]
    pub fn reason(&self) -> Option<BusyReason> {
        self.reason
    }

    /// The largest level-over-floor since the last reset — what the threshold was actually
    /// tested against in the meantime, which a reading sampled twice a second cannot show.
    #[must_use]
    pub fn excess_peak_db(&self) -> f64 {
        self.excess_peak_db
    }

    /// Start a new peak interval; the publisher calls it after each reading goes out.
    pub fn reset_excess_peak(&mut self) {
        self.excess_peak_db = f64::NEG_INFINITY;
    }

    /// Whether enough audio has gone by for the floor estimate to mean anything.
    ///
    /// Before this a caller should treat the channel as busy: not knowing is not the same as
    /// knowing it is clear, and the safe reading of "I have not listened yet" is to wait.
    #[must_use]
    pub fn settled(&self) -> bool {
        self.history.len() >= self.window_blocks / 2
    }

    /// Mark the channel busy because a frame was **decoded**.
    ///
    /// A decoded frame is there whatever the power measurement says, and it works below the
    /// threshold, which is the case that matters for politeness. An acquisition alone is
    /// not called here: see the module notes.
    pub fn mark_frame(&mut self, now: f64, detect_confidence: f64) {
        self.busy_until = self.busy_until.max(now + self.config.frame_hold_s);
        self.reason = Some(BusyReason::Frame { detect_confidence });
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
            // steady: the last few blocks, this one included, stayed within a few dB
            let steady = self.history.len() + 1 >= STEADY_BLOCKS && {
                let recent = self
                    .history
                    .iter()
                    .rev()
                    .take(STEADY_BLOCKS - 1)
                    .map(|&(block, _)| block)
                    .chain(std::iter::once(power));
                let (low, high) = recent.fold((f64::INFINITY, 0.0_f64), |(lo, hi), p| {
                    (lo.min(p), hi.max(p))
                });
                high <= low.max(FLOOR) * 10f64.powf(STEADY_RANGE_DB / 10.0)
            };
            self.history.push_back((power, steady));
            if self.history.len() > self.window_blocks {
                self.history.pop_front();
            }

            let floor = self
                .history
                .iter()
                .filter(|&&(_, steady)| steady)
                .map(|&(power, _)| power)
                .fold(f64::INFINITY, f64::min);
            self.level_db = 10.0 * self.smoothed.max(FLOOR).log10();
            if floor.is_finite() {
                self.floor_db = 10.0 * floor.max(FLOOR).log10();
            } else if self.floor_db == f64::NEG_INFINITY {
                // nothing steady yet: the plain minimum, until there is
                let lowest = self
                    .history
                    .iter()
                    .map(|&(power, _)| power)
                    .fold(f64::INFINITY, f64::min)
                    .max(FLOOR);
                self.floor_db = 10.0 * lowest.log10();
            }
            let excess = self.level_db - self.floor_db;
            self.excess_peak_db = self.excess_peak_db.max(excess);
            if excess >= self.config.threshold_db {
                self.busy_until = self.busy_until.max(now + self.config.hang_s);
                self.reason = Some(BusyReason::Level {
                    level_db: self.level_db,
                    floor_db: self.floor_db,
                });
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

    /// A scripted channel, block by block, with every busy transition recorded alongside
    /// the numbers that decided it. This is the controlled reproduction: noise only, a
    /// signal ramping up through the threshold and back down, impulses shorter than a
    /// block, a step in the floor, and a transmit gap — each stage checked against what
    /// the detector must and must not say.
    /// One stage of a script: its name, how many blocks it lasts, and the noise sigma at
    /// each block of it.
    type Stage<'a> = (&'a str, usize, Box<dyn Fn(usize) -> f64>);
    /// A busy transition: the stage it happened in, when, the new state, and the excess.
    type Transition = (String, f64, bool, f64);

    fn scripted(detector: &mut BusyDetector, stages: &[Stage<'_>], seed: u64) -> Vec<Transition> {
        let fs = detector.config().fs;
        let block = detector.config().block_s;
        let per_block = (block * fs) as usize;
        let mut now = 0.0;
        let mut was = false;
        let mut log = Vec::new();
        let mut n = 0u64;
        for (name, blocks, sigma_of) in stages {
            for index in 0..*blocks {
                let samples = noise(per_block, sigma_of(index), seed.wrapping_add(n));
                n += 1;
                now += block;
                let busy = detector.push(&samples, now);
                if busy != was {
                    log.push(((*name).to_owned(), now, busy, detector.excess_db()));
                    was = busy;
                }
            }
        }
        log
    }

    #[test]
    fn the_level_path_fires_only_when_the_level_really_clears_the_threshold() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let quiet = 0.01;
        let stages: Vec<Stage<'_>> = vec![
            // 8 s of noise alone: nothing may fire once the floor has settled
            ("noise", 320, Box::new(move |_| quiet)),
            // a signal ramping 0 -> +12 dB over 4 s: must fire, and not before ~+6
            ("ramp up", 160, Box::new(move |i| quiet * 10f64.powf((i as f64 / 160.0) * 12.0 / 20.0))),
            // holding at +12 dB
            ("hold", 40, Box::new(move |_| quiet * 10f64.powf(12.0 / 20.0))),
            // ramping back down over 4 s: must clear after the hangover
            ("ramp down", 160, Box::new(move |i| quiet * 10f64.powf((1.0 - i as f64 / 160.0) * 12.0 / 20.0))),
            ("noise again", 200, Box::new(move |_| quiet)),
        ];
        let log = scripted(&mut detector, &stages, 7);
        let stage_of = |t: f64| -> &str {
            let b = (t / 0.025).round() as usize;
            if b <= 320 { "noise" } else if b <= 480 { "ramp up" } else if b <= 520 { "hold" } else if b <= 680 { "ramp down" } else { "noise again" }
        };
        for (stage, at, busy, excess) in &log {
            eprintln!("BUSY {} at {at:.2}s in '{stage}' | excess {excess:+.1} dB | threshold 6.0", if *busy { "OFF -> ON " } else { "ON  -> OFF" });
        }
        assert!(
            !log.iter().any(|(s, t, busy, _)| *busy && s == "noise" && *t > 2.5),
            "noise alone lit the indicator: {log:?}"
        );
        let first_on = log.iter().find(|(_, _, busy, _)| *busy).expect("the ramp must light it");
        assert_eq!(stage_of(first_on.1), "ramp up", "lit outside the ramp: {log:?}");
        assert!(
            first_on.3 >= 6.0 - 0.6,
            "lit at only {:+.1} dB over the floor, below the 6 dB threshold",
            first_on.3
        );
        assert!(
            log.iter().any(|(s, _, busy, _)| !*busy && (s == "ramp down" || s == "noise again")),
            "never cleared after the signal fell: {log:?}"
        );
        assert!(!detector.busy(0.025 * 880.0), "still busy on noise at the end");
    }

    #[test]
    fn an_impulse_shorter_than_a_block_is_a_hangover_not_a_lockup() {
        // A static crash: one block, 20 dB up. It legitimately trips the threshold — the
        // detector cannot know it will not last — but the cost is bounded to the hangover,
        // and the floor must not move. A display sampling twice a second will almost never
        // show the block that did it, which is why the reason and the peak are reported.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let quiet = 0.01;
        let stages: Vec<Stage<'_>> = vec![
            ("noise", 320, Box::new(move |_| quiet)),
            ("impulse", 1, Box::new(move |_| quiet * 10.0)),
            ("noise", 120, Box::new(move |_| quiet)),
        ];
        let floor_before = {
            let mut d = BusyDetector::new(BusyConfig::default());
            feed(&mut d, 8.0, quiet, 7, 0.0);
            d.floor_db
        };
        let log = scripted(&mut detector, &stages, 7);
        let on = log.iter().filter(|(_, _, busy, _)| *busy).count();
        assert_eq!(on, 1, "the impulse lights it exactly once: {log:?}");
        let off = log.iter().find(|(_, _, busy, _)| !*busy).expect("and it clears");
        let lit = log.iter().find(|(_, _, busy, _)| *busy).expect("lit");
        let held = off.1 - lit.1;
        // the smoother (one pole, two blocks) keeps the level over the threshold for a few
        // blocks after a 20 dB impulse, and each of those re-arms the hangover: measured,
        // 100 ms on top of it. Bounded and explicable; anything much beyond it is not.
        let smoother_tail_s = 0.15;
        assert!(
            held <= detector.config().hang_s + smoother_tail_s,
            "an impulse held the channel {held:.2} s, longer than the hangover and the              smoother's tail"
        );
        assert!(
            (detector.floor_db - floor_before).abs() < 1.0,
            "the impulse moved the floor: {} -> {}",
            floor_before,
            detector.floor_db
        );
        assert!(matches!(detector.reason(), Some(BusyReason::Level { .. })));
    }

    #[test]
    fn a_step_down_in_the_noise_does_not_leave_the_channel_reading_busy() {
        // The floor drops when the band goes quieter; the level drops with it, so nothing
        // should be busy. A floor that failed to follow — or followed an artefact — would
        // read the new quiet as +N dB over an old floor, or the old level as +N over a
        // collapsed one. Neither may happen.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'_>> = vec![
            ("loud noise", 320, Box::new(|_| 0.03)),
            ("quieter noise", 320, Box::new(|_| 0.01)),
            ("loud again", 320, Box::new(|_| 0.03)),
        ];
        let log = scripted(&mut detector, &stages, 11);
        // the step back up is a genuine +9.5 dB over a floor learned on the quiet: it may
        // fire, and must clear within the window once the floor catches up
        let noise_lit: Vec<_> = log.iter().filter(|(s, _, b, _)| *b && s != "loud again").collect();
        assert!(noise_lit.is_empty(), "stationary noise lit the indicator: {noise_lit:?}");
        assert!(!detector.busy(0.025 * 960.0 + 1.0), "still busy long after the floor caught up");
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

    /// Feed `seconds` of noise whose amplitude follows `gain(t)`, block by block.
    fn feed_shaped(
        detector: &mut BusyDetector,
        seconds: f64,
        sigma: f64,
        seed: u64,
        t0: f64,
        gain: impl Fn(f64) -> f64,
    ) -> f64 {
        let fs = detector.config().fs;
        let block = detector.config().block_s;
        let per_block = (block * fs) as usize;
        let blocks = (seconds / block) as usize;
        let mut now = t0;
        for index in 0..blocks {
            let g = gain(index as f64 * block);
            let samples = noise(per_block, sigma * g, seed.wrapping_add(index as u64));
            now += block;
            detector.push(&samples, now);
        }
        now
    }

    #[test]
    fn a_receiver_gain_dip_does_not_pull_the_floor_down() {
        // what an FTDX10 on AGC AUTO did seven times in two minutes of an idle band: the
        // gain cut 4–18 dB in a millisecond by something outside the audio passband, held
        // for about 100 ms, and let back at 40–55 dB/s. A plain minimum held each dip for
        // the whole window and drew square pits on the panel; the busy threshold sat a
        // decibel above the real noise for five seconds after each one.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 11, 0.0);
        let floor_before = detector.floor_db;
        let level_before = detector.level_db;
        assert!((level_before - floor_before).abs() < 2.0);
        // −18 dB for 100 ms, then back at 50 dB/s: 0.46 s of ramp
        let dip = |t: f64| -> f64 {
            let db = if t < 0.1 {
                -18.0
            } else {
                (-18.0 + 50.0 * (t - 0.1)).min(0.0)
            };
            10f64.powf(db / 20.0)
        };
        let now = feed_shaped(&mut detector, 0.6, 0.01, 12, now, dip);
        let now = feed(&mut detector, 1.0, 0.01, 13, now);
        assert!(
            (detector.floor_db - floor_before).abs() < 1.0,
            "the gain dip moved the floor from {floor_before:.1} to {:.1} dB",
            detector.floor_db
        );
        assert!(
            !detector.busy(now),
            "an idle channel read busy after a gain dip: {:.1} dB over the floor",
            detector.excess_db()
        );
        // and a real drop in the noise — another band, another antenna — is followed
        let now = feed(&mut detector, 6.0, 0.001, 14, now);
        assert!(
            detector.floor_db < floor_before - 15.0,
            "the floor did not follow a quieter band: {:.1} dB",
            detector.floor_db
        );
        assert!(!detector.busy(now));
    }

    #[test]
    fn the_floor_is_still_learned_in_the_gaps_of_a_busy_channel() {
        // somebody else's ARQ session: bursts 20 dB up with turnarounds of a fifth of a
        // second, for longer than the window. The floor must come from the gaps, and the
        // channel must read busy throughout
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 6.0, 0.01, 21, 0.0);
        let floor_before = detector.floor_db;
        let mut now = now;
        for burst in 0..8u64 {
            now = feed(&mut detector, 1.0, 0.1, 100 + burst, now);
            assert!(detector.busy(now), "burst {burst} read clear");
            now = feed(&mut detector, 0.2, 0.01, 200 + burst, now);
            assert!(detector.busy(now), "the gap after burst {burst} read clear");
        }
        assert!(
            (detector.floor_db - floor_before).abs() < 1.5,
            "the floor crept from {floor_before:.1} to {:.1} dB under the bursts",
            detector.floor_db
        );
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
        detector.mark_frame(now, 3.0);
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
