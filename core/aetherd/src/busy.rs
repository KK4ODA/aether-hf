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
//! smallest block power seen over a window. The reasoning is Martin's (*Noise power spectral
//! density estimation based on optimal smoothing and minimum statistics*, IEEE Trans. Speech
//! and Audio Processing 9(5), 2001): any signal only ever *adds* power, so over a window
//! longer than the longest gap-free stretch of signal, the minimum is the noise alone.
//!
//! That last clause is the whole difficulty. A window of five seconds sees between the
//! syllables of speech and the frames of a burst, and it made the floor climb into the
//! signal on anything that runs longer without a gap: an FT8 transmission is 12.64 s of
//! constant-envelope FSK, and five seconds into one every block in the window *was* the
//! signal, so the floor rose to it, the margin collapsed, and the channel read clear with a
//! strong signal in the passband — busy only at the onset of each period, when the 2.36 s
//! gap had let the floor back down. So there are two separations now, and they are the
//! architecture:
//!
//! * **channel energy** is the latest 25 ms block, and the busy decision is made on it;
//! * **background noise** is the minimum over a **60 s** window of blocks that were
//!   captured while the channel was **not busy**. A block taken while a signal is present is
//!   by definition not noise, and never enters the floor — so a sustained signal raises the
//!   measured energy and leaves the floor where it was, for as long as the window reaches
//!   back before it. Sixty seconds covers FT8, a CW or RTTY exchange and most SSB overs.
//!   Past that the signal has been there for a minute and the floor accepts the window's
//!   plain minimum: at that point it *is* the environment, and refusing to learn it would
//!   leave a floor that once collapsed stuck below everything for ever.
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
//! never steady — so the gate keeps the theory and drops the artefact. The price is that a
//! gap shorter than 200 ms does not show the floor, and the ARQ turnarounds of the modes on
//! the band exceed it. When no block in the window is evidence, the floor learned before is
//! held: a channel that has been busy for five seconds without a quiet moment is still
//! measured against the noise that was there before it. A receiver whose AGC recovers
//! slower than about 15 dB/s (a SLOW setting) ramps gently enough to pass the gate, and
//! its dips will show; the field notes say which settings to use.
//!
//! On top of that:
//!
//! * the threshold has to be exceeded for **three consecutive blocks** (75 ms) before the
//!   channel is called busy. A static crash is one block, a keying click one or two; the
//!   shortest thing on the air that is occupancy — a CW dah at 25 wpm, an Aether preamble of
//!   four 31 ms symbols — is longer. The one-pole smoother this replaces did the opposite:
//!   it stretched a single hot block over four and each of those re-armed the hangover;
//! * a **hangover** keeps the channel marked busy for a moment after the power drops, so the
//!   gaps inside a burst do not read as a free channel — that is the release hysteresis;
//! * a block at **digital silence** never enters the floor. A receiver never delivers it; a
//!   sound card muted around this station's own transmission does, and eight of those in a
//!   row would put the floor at −78 dBFS and everything above it;
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
    /// How far back the floor estimate looks, in seconds. Must cover the longest gap-free
    /// stretch of signal the channel will carry, or the floor climbs into the signal — and
    /// it is the horizon after which a signal that never stops is accepted as the
    /// environment.
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
            // a minute: past an FT8 period (15 s), a CW or RTTY exchange, most SSB overs
            floor_window_s: 60.0,
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
    /// The block level stood at least the threshold above the learned floor, for the
    /// attack's worth of blocks in a row.
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
    /// Raw power per block and whether it is evidence of the noise floor — steady, not
    /// silence, and captured while the channel was not busy — newest last, capped at
    /// `window_blocks`.
    history: std::collections::VecDeque<(f64, bool)>,
    partial: Vec<Complex>,
    /// How many blocks in a row have exceeded the threshold, for the attack qualification.
    over: usize,
    busy_until: f64,
    /// Latest block power, in dB relative to full scale: the channel energy the busy
    /// decision is made on, block by block.
    pub level_db: f64,
    /// Latest floor estimate, in dB relative to full scale.
    pub floor_db: f64,
    /// Blocks discarded because this station was transmitting.
    pub blocks_skipped: usize,
}

/// Power floor for the logarithm, so silence gives a very negative number rather than
/// negative infinity.
const FLOOR: f64 = 1e-20;
/// How many consecutive blocks must exceed the threshold before the channel is busy: 75 ms.
/// A static crash or a keying click is a block or two; the shortest occupancy that matters
/// — a CW dah, an Aether preamble of four 31 ms symbols — is longer.
const ATTACK_BLOCKS: usize = 3;
/// How many blocks the detector listens for before its floor means anything: 2.5 s, the
/// half-window the five-second design waited for.
const SETTLE_BLOCKS: usize = 100;
/// A block below this power is digital silence — a muted sound card, never a receiver — and
/// is not evidence of the noise floor. −70 dBFS.
const SILENCE: f64 = 1e-7;
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
            over: 0,
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
        // a few seconds of listening: the window is a minute long, and waiting for half of
        // it before saying anything would keep every start silent for thirty seconds
        self.history.len() >= SETTLE_BLOCKS
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

            // steady: the last few blocks, this one included, stayed within a few dB — a
            // receiver's AGC stepping down and ramping back never is
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
            // The separation the whole detector rests on: the busy decision is made on this
            // block's energy against the floor learned *before* it, and the block only
            // becomes floor evidence if the channel was not busy when it was taken. A block
            // captured under a signal is not noise, however long the signal lasts.
            self.level_db = 10.0 * power.max(FLOOR).log10();
            let was_busy = self.busy(now);
            let excess = self.level_db - self.floor_db;
            self.excess_peak_db = self.excess_peak_db.max(excess);
            let over = self.floor_db.is_finite() && excess >= self.config.threshold_db;
            self.over = if over { self.over + 1 } else { 0 };
            if self.over >= ATTACK_BLOCKS {
                self.busy_until = self.busy_until.max(now + self.config.hang_s);
                self.reason = Some(BusyReason::Level {
                    level_db: self.level_db,
                    floor_db: self.floor_db,
                });
            }
            // evidence of the floor: steady, not a muted card, and not under a signal — the
            // block that crosses the threshold is itself excluded, whether or not it goes
            // on to make the channel busy
            let evidence = steady && power > SILENCE && !was_busy && !over;
            self.history.push_back((power, evidence));
            if self.history.len() > self.window_blocks {
                self.history.pop_front();
            }

            let floor = self
                .history
                .iter()
                .filter(|&&(_, evidence)| evidence)
                .map(|&(power, _)| power)
                .fold(f64::INFINITY, f64::min);
            if floor.is_finite() {
                self.floor_db = 10.0 * floor.max(FLOOR).log10();
            } else if self.history.len() >= self.window_blocks
                || self.floor_db == f64::NEG_INFINITY
            {
                // No evidence in the whole window: either nothing has been heard yet, or
                // the channel has been busy for the whole minute. Then the plain minimum,
                // silence excepted — a signal that never stops is the environment, and a
                // floor that will not learn it is a floor that can never recover.
                let lowest = self
                    .history
                    .iter()
                    .map(|&(power, _)| power)
                    .filter(|&p| p > SILENCE)
                    .fold(f64::INFINITY, f64::min);
                if lowest.is_finite() {
                    self.floor_db = 10.0 * lowest.max(FLOOR).log10();
                }
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

    /// Block-by-block record of a scripted run: for every block, the stage, the time, the
    /// level, the floor and the busy state — enough to hold the floor to a shape, not only
    /// the busy state to a value.
    type Trace = Vec<(&'static str, f64, f64, f64, bool)>;

    fn traced(detector: &mut BusyDetector, stages: &[Stage<'static>], seed: u64) -> Trace {
        let fs = detector.config().fs;
        let block = detector.config().block_s;
        let per_block = (block * fs) as usize;
        let mut now = 0.0;
        let mut trace = Vec::new();
        let mut n = 0u64;
        for (name, blocks, sigma_of) in stages {
            for index in 0..*blocks {
                let samples = noise(per_block, sigma_of(index), seed.wrapping_add(n));
                n += 1;
                now += block;
                let busy = detector.push(&samples, now);
                trace.push((*name, now, detector.level_db, detector.floor_db, busy));
            }
        }
        trace
    }

    /// Blocks for a number of seconds at the default block length.
    const fn secs(s: f64) -> usize {
        (s / 0.025) as usize
    }

    /// The share of blocks in a stage that were busy, 0..1.
    fn busy_share(trace: &Trace, stage: &str) -> f64 {
        let blocks: Vec<_> = trace.iter().filter(|r| r.0 == stage).collect();
        blocks.iter().filter(|r| r.4).count() as f64 / blocks.len().max(1) as f64
    }

    /// The floor, dBFS, at the last block of a stage.
    fn floor_at_end(trace: &Trace, stage: &str) -> f64 {
        trace.iter().rev().find(|r| r.0 == stage).expect("stage present").3
    }

    #[test]
    fn case_1_stationary_noise_never_reads_busy() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![("noise", secs(90.0), Box::new(|_| 0.01))];
        let trace = traced(&mut detector, &stages, 3);
        let after_settle: Vec<_> = trace.iter().filter(|r| r.1 > 3.0).collect();
        let busy = after_settle.iter().filter(|r| r.4).count();
        assert_eq!(busy, 0, "stationary noise lit the indicator {busy} blocks of {}", after_settle.len());
        // and the floor sits where the noise is: within the block-to-block spread of it
        let level: Vec<f64> = after_settle.iter().map(|r| r.2).collect();
        let median = {
            let mut s = level.clone();
            s.sort_by(f64::total_cmp);
            s[s.len() / 2]
        };
        let floor = floor_at_end(&trace, "noise");
        assert!(
            (median - floor) < 2.5 && (median - floor) > 0.0,
            "the floor should sit just under the noise's median: median {median:.1}, floor {floor:.1}"
        );
    }

    #[test]
    fn case_2_a_single_short_spike_does_not_trigger_busy() {
        // one block, +20 dB: a static crash. The attack needs three in a row.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(10.0), Box::new(|_| 0.01)),
            ("spike", 1, Box::new(|_| 0.1)),
            ("noise after", secs(5.0), Box::new(|_| 0.01)),
        ];
        let trace = traced(&mut detector, &stages, 5);
        assert!(!trace.iter().any(|r| r.4), "a single spike lit the indicator");
        let before = floor_at_end(&trace, "noise");
        let after = floor_at_end(&trace, "noise after");
        assert!((before - after).abs() < 0.5, "the spike moved the floor {before:.1} -> {after:.1}");
    }

    #[test]
    fn case_3_repeated_impulses_do_not_chatter() {
        // a crash every 300 ms for ten seconds, each one or two blocks long
        let mut detector = BusyDetector::new(BusyConfig::default());
        let mut stages: Vec<Stage<'static>> = vec![("noise", secs(10.0), Box::new(|_| 0.01))];
        for _ in 0..33 {
            stages.push(("crash", 2, Box::new(|_| 0.08)));
            stages.push(("between", 10, Box::new(|_| 0.01)));
        }
        stages.push(("noise after", secs(5.0), Box::new(|_| 0.01)));
        let trace = traced(&mut detector, &stages, 9);
        let transitions = trace.windows(2).filter(|w| w[0].4 != w[1].4).count();
        assert_eq!(
            transitions, 0,
            "two-block impulses caused {transitions} busy transitions: chatter"
        );
        let before = floor_at_end(&trace, "noise");
        let after = floor_at_end(&trace, "noise after");
        assert!((before - after).abs() < 1.0, "the impulses moved the floor {before:.1} -> {after:.1}");
    }

    #[test]
    fn case_4_a_signal_rising_through_the_threshold_activates_busy() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(10.0), Box::new(|_| 0.01)),
            // +15 dB over 6 s
            ("ramp", secs(6.0), Box::new(|i| 0.01 * 10f64.powf(i as f64 / secs(6.0) as f64 * 15.0 / 20.0))),
            ("hold", secs(4.0), Box::new(|_| 0.01 * 10f64.powf(15.0 / 20.0))),
        ];
        let trace = traced(&mut detector, &stages, 13);
        let first = trace.iter().find(|r| r.4).expect("the ramp lights it");
        assert_eq!(first.0, "ramp");
        assert!(
            first.2 - first.3 >= 6.0 - 0.3,
            "lit at only {:+.1} dB over the floor",
            first.2 - first.3
        );
        assert!(busy_share(&trace, "hold") > 0.99, "not held busy through the hold");
    }

    #[test]
    fn case_5_a_continuous_strong_signal_stays_busy_and_the_floor_stays_put() {
        // 45 s of a +12 dB signal: busy throughout, and the floor may not climb toward it
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(15.0), Box::new(|_| 0.01)),
            ("signal", secs(45.0), Box::new(|_| 0.01 * 10f64.powf(12.0 / 20.0))),
        ];
        let trace = traced(&mut detector, &stages, 17);
        let floor_before = floor_at_end(&trace, "noise");
        let signal_blocks: Vec<_> = trace.iter().filter(|r| r.0 == "signal" && r.1 > 15.5).collect();
        let busy = signal_blocks.iter().filter(|r| r.4).count();
        assert_eq!(
            busy,
            signal_blocks.len(),
            "busy dropped out {} blocks of {} under a continuous signal",
            signal_blocks.len() - busy,
            signal_blocks.len()
        );
        let floor_worst = signal_blocks.iter().map(|r| r.3).fold(f64::MIN, f64::max);
        assert!(
            floor_worst - floor_before < 1.0,
            "the floor climbed {:.1} dB toward a signal it should never learn",
            floor_worst - floor_before
        );
    }

    #[test]
    fn case_6_an_ft8_burst_is_busy_for_its_whole_length() {
        // 12.64 s of constant-envelope signal at +10 dB, as an FT8 transmission is: busy from
        // the attack to the end of it, not only at the onset
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(15.0), Box::new(|_| 0.01)),
            ("ft8", secs(12.64), Box::new(|_| 0.01 * 10f64.powf(10.0 / 20.0))),
            ("gap", secs(2.36), Box::new(|_| 0.01)),
        ];
        let trace = traced(&mut detector, &stages, 19);
        let ft8: Vec<_> = trace.iter().filter(|r| r.0 == "ft8" && r.1 > 15.2).collect();
        let busy = ft8.iter().filter(|r| r.4).count();
        assert_eq!(busy, ft8.len(), "busy cleared {} blocks into an FT8 period", ft8.len() - busy);
        // the floor did not follow the signal
        let floor_before = floor_at_end(&trace, "noise");
        let floor_end = floor_at_end(&trace, "ft8");
        assert!(
            (floor_end - floor_before).abs() < 1.0,
            "the floor moved {floor_before:.1} -> {floor_end:.1} under FT8"
        );
    }

    #[test]
    fn case_7_when_the_signal_stops_busy_clears_and_the_floor_recovers() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(15.0), Box::new(|_| 0.01)),
            ("signal", secs(20.0), Box::new(|_| 0.01 * 10f64.powf(12.0 / 20.0))),
            ("after", secs(10.0), Box::new(|_| 0.01)),
        ];
        let trace = traced(&mut detector, &stages, 23);
        let after: Vec<_> = trace.iter().filter(|r| r.0 == "after").collect();
        // busy clears within the hangover
        let cleared = after.iter().position(|r| !r.4).expect("busy never cleared");
        assert!(
            cleared as f64 * 0.025 <= detector.config().hang_s + 0.1,
            "took {:.2} s to clear after the signal stopped",
            cleared as f64 * 0.025
        );
        assert!(after.iter().skip(cleared).all(|r| !r.4), "busy came back on noise");
        // and the floor is where it was before the signal
        let before = floor_at_end(&trace, "noise");
        let recovered = floor_at_end(&trace, "after");
        assert!((before - recovered).abs() < 1.0, "floor {before:.1} -> {recovered:.1}");
    }

    #[test]
    fn case_8_repeated_ft8_cycles_do_not_ratchet_the_floor() {
        // ten 15 s cycles of 12.64 s on, 2.36 s off, at +10 dB: the floor must stay at the
        // noise the whole way, and every period must be busy end to end
        let mut detector = BusyDetector::new(BusyConfig::default());
        let mut stages: Vec<Stage<'static>> = vec![("noise", secs(15.0), Box::new(|_| 0.01))];
        for _ in 0..10 {
            stages.push(("ft8", secs(12.64), Box::new(|_| 0.01 * 10f64.powf(10.0 / 20.0))));
            stages.push(("gap", secs(2.36), Box::new(|_| 0.01)));
        }
        let trace = traced(&mut detector, &stages, 29);
        let floor_before = floor_at_end(&trace, "noise");
        let floors: Vec<f64> = trace.iter().filter(|r| r.1 > 15.0).map(|r| r.3).collect();
        let worst = floors.iter().copied().fold(f64::MIN, f64::max);
        assert!(
            worst - floor_before < 1.0,
            "the floor ratcheted up to {worst:.1} from {floor_before:.1} over ten cycles"
        );
        // the on periods: busy, apart from the attack at the start of each
        let on: Vec<_> = trace.iter().filter(|r| r.0 == "ft8").collect();
        let share = on.iter().filter(|r| r.4).count() as f64 / on.len() as f64;
        assert!(share > 0.98, "busy only {:.0}% of the FT8 periods", share * 100.0);
        // the gaps: busy only for the hangover at the start of each
        let off: Vec<_> = trace.iter().filter(|r| r.0 == "gap").collect();
        let share_off = off.iter().filter(|r| r.4).count() as f64 / off.len() as f64;
        assert!(
            share_off < 0.4,
            "busy {:.0}% of the gaps, more than the hangover accounts for",
            share_off * 100.0
        );
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
