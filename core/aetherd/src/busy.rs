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
//! threshold in dBm or dBFS can work. The floor is the **median** of the block powers seen
//! over a window, taken only over blocks that were not under a signal. The reasoning
//! descends from Martin's minimum statistics (*Noise power spectral density estimation
//! based on optimal smoothing and minimum statistics*, IEEE Trans. Speech and Audio
//! Processing 9(5), 2001): a signal only ever *adds* power, so the quiet blocks are the
//! noise. Martin takes their minimum and corrects its bias; this takes their median, which
//! needs no correction and is the number the margin has to mean something against. On a
//! stormy 40 m evening the noise is not stationary — its envelope surges 8–13 dB above its
//! quietest lulls for a few hundred milliseconds at a time, and the median sits 4 dB above
//! the minimum. A margin over the *minimum* was two decibels over typical noise on such a
//! night, and fired twenty-five times a minute.
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
//! * **background noise** is the median over a **10 s** window of blocks that were
//!   captured while the channel was **not busy**. A block taken while a signal is present is
//!   by definition not noise, and never enters the floor — so a sustained signal raises the
//!   measured energy and leaves the floor where it was. When the window holds no such block
//!   at all, the floor learned before is **held**, for up to a minute: that covers FT8, a
//!   CW or RTTY exchange and most SSB overs. Past the minute the signal has been there
//!   long enough to be the environment, and the floor accepts the median of everything —
//!   refusing to would leave a floor that once collapsed stuck below everything for ever.
//!   Ten seconds is the window because a median follows a change only once half the
//!   window has seen it: a band that goes quiet is reflected in five seconds, not thirty.
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
//! # The shape of the passband
//!
//! A level against a floor is the wrong instrument behind a receiver's AGC. With a strong
//! signal in the passband the receiver turns its gain down, so in the audio the signal sits
//! at the AGC's set point and the noise beside it drops: a strong FT8 station measured
//! exactly 6 dB over the noise between periods, and no margin that rejects storm noise can
//! catch that. What the AGC cannot hide is the **shape**: it scales every frequency in the
//! passband together, so a narrowband signal's peak stays the same distance above the bins
//! beside it. Noise is spectrally flat; an FT8 tone, a CW carrier, a PSK or RTTY signal, the
//! formants of a voice are not. So every 200 ms the detector takes the periodogram of the
//! passband and compares its highest bin to its median bin. Measured: quiet noise sits at
//! 6 dB and never passes 10; storm noise the same; the weakest FT8 station recorded —
//! invisible to the level, 4.5 dB over the floor — sits at 15 dB and above, and a strong one
//! at 19–25. Two consecutive windows over 12 dB is busy, one window keeps it busy once it
//! is, and a block in a peaked window is not noise evidence, which is what keeps a floor
//! from learning a channel that two FT8 stations occupy nine seconds in ten. An OFDM
//! signal like Aether's own is as flat as noise and is caught by the level path, or by
//! decoding it.
//!
//! One limit, measured on a busy FT8 frequency behind an AGC: the passband was occupied
//! 85 % of the time and the rest was the receiver's gain recovering — a ramp, never steady
//! — so the true noise floor was never observable and the level path's floor stayed where
//! it was first learned. That is the level path being blind, not the shape path, which
//! carried the whole channel; the floor reading on such a channel is not to be trusted.
//!
//! On top of that:
//!
//! * the threshold has to be exceeded by **half of the last sixteen blocks** (400 ms) before
//!   the channel is called busy. A static crash is one block; a surge of atmospheric noise
//!   measured 300–400 ms; the shortest occupancy that matters — a CW character, an FT8
//!   period, an SSB syllable, an Aether frame — runs longer and keys at least half the time.
//!   Measured against a stormy band's noise: 0.8 false trips a minute at 6 dB, none at 7;
//!   against CW at 25 wpm, busy for 98 % of the sending. The one-pole smoother this
//!   replaces did the opposite: it stretched a single hot block over four and each of those
//!   re-armed the hangover;
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
    /// How far back the floor estimate looks, in seconds. A median follows a change once
    /// half the window has seen it, so this is the floor's response time doubled. It need
    /// not outlast a signal: blocks under a signal never enter the floor at all.
    pub floor_window_s: f64,
    /// How long the floor is held when no block in the window was free of signal, before
    /// the signal is accepted as the environment. The longest transmission the floor will
    /// see through.
    pub floor_hold_s: f64,
    /// How far above the floor counts as occupied, in dB.
    pub threshold_db: f64,
    /// How long the channel stays marked busy after the power falls back.
    pub hang_s: f64,
    /// How long a detected frame keeps the channel marked busy.
    pub frame_hold_s: f64,
    /// How far the passband's highest spectral bin must stand over its median bin, in dB,
    /// for the shape to count as a signal. Noise measures about 6 and never passed 10 on
    /// any recording; the weakest FT8 station measured 15.
    pub shape_db: f64,
    /// The width of the passband the shape is judged over, in hertz: the waveform's
    /// occupied bandwidth.
    pub passband_hz: f64,
}

impl Default for BusyConfig {
    fn default() -> Self {
        Self {
            fs: 8000.0,
            block_s: 0.025,
            // a band that goes quiet is reflected in five seconds
            floor_window_s: 10.0,
            // a minute: past an FT8 period (15 s), a CW or RTTY exchange, most SSB overs
            floor_hold_s: 60.0,
            // measured acquisition works well below this, so the frame signal is what catches
            // a weak Aether station; this catches everything else on the channel
            threshold_db: 6.0,
            hang_s: 0.75,
            frame_hold_s: 2.0,
            shape_db: 12.0,
            passband_hz: 500.0,
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
    /// attack's majority of the last 400 ms.
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
    /// The passband's spectrum was peaked, as a narrowband signal's is and noise's never.
    Shape {
        /// The highest bin over the median bin, dB.
        peak_db: f64,
    },
}

/// Channel occupancy from the received baseband.
#[derive(Clone)]
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
    /// Whether each of the last `ATTACK_WINDOW` blocks exceeded the threshold, newest last.
    over: std::collections::VecDeque<bool>,
    /// When the window last held no signal-free block, if it holds none now: the floor is
    /// held from then, and accepted from everything once the hold has run out.
    held_since: Option<f64>,
    /// Baseband of the last few blocks, for the periodogram; drained every `SHAPE_BLOCKS`.
    shape_buffer: Vec<Complex>,
    /// The planned transform, `SHAPE_FFT` points.
    fft: std::sync::Arc<dyn rustfft::Fft<f64>>,
    /// Whether each of the last two shape windows was peaked, newest last.
    peaked: [bool; 2],
    /// The latest shape reading: the passband's highest bin over its median, dB.
    pub shape_db: f64,
    busy_until: f64,
    /// Latest block power, in dB relative to full scale: the channel energy the busy
    /// decision is made on, block by block.
    pub level_db: f64,
    /// Latest floor estimate, in dB relative to full scale: the median of the last
    /// minute's blocks that were not under a signal — the noise's typical level.
    pub floor_db: f64,
    /// Blocks discarded because this station was transmitting.
    pub blocks_skipped: usize,
}

/// Power floor for the logarithm, so silence gives a very negative number rather than
/// negative infinity.
const FLOOR: f64 = 1e-20;
/// The attack: over the last `ATTACK_WINDOW` blocks (400 ms), at least `ATTACK_MAJORITY`
/// must have exceeded the threshold. A crash is one block and a surge of storm noise a
/// dozen; a CW character keys about half its span, and everything slower keys all of it.
const ATTACK_WINDOW: usize = 16;
/// See [`ATTACK_WINDOW`].
const ATTACK_MAJORITY: usize = 8;
/// How many blocks make one shape window: 200 ms, enough for a steady periodogram and
/// short enough that two of them are the level path's attack.
const SHAPE_BLOCKS: usize = 8;
/// Samples per transform: 50 ms at the baseband rate, 20 Hz bins; four of them are
/// averaged over a window, and the zero padding to `SHAPE_FFT` interpolates the bins.
const SHAPE_SAMPLES: usize = 400;
/// The transform length.
const SHAPE_FFT: usize = 512;
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
            over: std::collections::VecDeque::with_capacity(ATTACK_WINDOW),
            held_since: None,
            shape_buffer: Vec::with_capacity(SHAPE_BLOCKS * block_samples),
            fft: rustfft::FftPlanner::new().plan_fft_forward(SHAPE_FFT),
            peaked: [false, false],
            shape_db: f64::NAN,
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
            // the shape path: every eight blocks, the passband's periodogram
            self.shape_buffer.extend_from_slice(block);
            let peaked_now = if self.shape_buffer.len() >= SHAPE_BLOCKS * self.block_samples {
                // digital silence has no shape: a muted sound card, or nothing on the
                // input, is not judged — its spectrum is numerical noise and can look
                // peaked, and it says nothing about the channel
                let window_power = self
                    .shape_buffer
                    .iter()
                    .map(|&(re, im)| re.mul_add(re, im * im))
                    .sum::<f64>()
                    / self.shape_buffer.len() as f64;
                let peak_db = if window_power > SILENCE {
                    self.passband_peak_db()
                } else {
                    f64::NAN
                };
                self.shape_buffer.clear();
                self.shape_db = peak_db;
                let peaked = peak_db >= self.config.shape_db;
                self.peaked = [self.peaked[1], peaked];
                // attack on two peaked windows in a row; once busy, one is enough to hold
                // it. An FT8 tone hop that straddles a window boundary splits the energy
                // between two bins and that window reads flat for 200 ms — measured, it
                // dropped the channel for a second in the middle of a period, three times
                // a minute. A signal that was there a moment ago and is still peaked in
                // one window of two has not gone anywhere.
                let attack = self.peaked.iter().all(|&p| p);
                let hold = self.busy(now) && self.peaked.iter().any(|&p| p);
                if attack || hold {
                    self.busy_until = self.busy_until.max(now + self.config.hang_s);
                    if attack {
                        self.reason = Some(BusyReason::Shape { peak_db });
                    }
                }
                peaked
            } else {
                self.peaked[1]
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
            self.over.push_back(over);
            if self.over.len() > ATTACK_WINDOW {
                self.over.pop_front();
            }
            if self.over.iter().filter(|&&o| o).count() >= ATTACK_MAJORITY {
                self.busy_until = self.busy_until.max(now + self.config.hang_s);
                self.reason = Some(BusyReason::Level {
                    level_db: self.level_db,
                    floor_db: self.floor_db,
                });
            }
            // evidence of the floor: steady, not a muted card, and not under a signal — by
            // level or by shape; the block that crosses the threshold is itself excluded,
            // whether or not it goes on to make the channel busy
            let evidence = steady && power > SILENCE && !was_busy && !over && !peaked_now;
            self.history.push_back((power, evidence));
            if self.history.len() > self.window_blocks {
                self.history.pop_front();
            }

            let floor = median(
                self.history
                    .iter()
                    .filter(|&&(_, evidence)| evidence)
                    .map(|&(power, _)| power),
            );
            if let Some(floor) = floor {
                self.floor_db = 10.0 * floor.max(FLOOR).log10();
                self.held_since = None;
            } else {
                // No signal-free block in the window: the floor learned before is held —
                // through an FT8 period, an SSB over — until the hold runs out. Then, or
                // when nothing has been learned yet at all, the median of every block,
                // silence excepted: a signal that never stops is the environment, and a
                // floor that will not learn it is a floor that can never recover.
                let since = *self.held_since.get_or_insert(now);
                let unlearned = self.floor_db == f64::NEG_INFINITY;
                if unlearned || now - since >= self.config.floor_hold_s {
                    let all = median(
                        self.history
                            .iter()
                            .map(|&(power, _)| power)
                            .filter(|&p| p > SILENCE),
                    );
                    if let Some(all) = all {
                        self.floor_db = 10.0 * all.max(FLOOR).log10();
                    }
                }
            }
        }
        self.partial.drain(..consumed);
        self.busy(now)
    }
}

impl BusyDetector {
    /// The passband's highest spectral bin over its median bin, in dB, from the blocks
    /// gathered since the last window: four 50 ms transforms averaged, so a bin is
    /// judged on 200 ms and not on one noisy periodogram.
    fn passband_peak_db(&self) -> f64 {
        let bin_hz = self.config.fs / SHAPE_FFT as f64;
        let half = (self.config.passband_hz / 2.0 / bin_hz).round() as usize;
        let mut power = vec![0.0f64; SHAPE_FFT];
        let mut scratch = vec![rustfft::num_complex::Complex64::default(); SHAPE_FFT];
        for chunk in self.shape_buffer.chunks(SHAPE_SAMPLES) {
            if chunk.len() < SHAPE_SAMPLES {
                break;
            }
            for (slot, (i, &(re, im))) in scratch.iter_mut().zip(chunk.iter().enumerate()) {
                // a Hann window, so a strong bin does not leak into its neighbours' median
                let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / SHAPE_SAMPLES as f64).cos();
                *slot = rustfft::num_complex::Complex64::new(re * w, im * w);
            }
            for slot in scratch.iter_mut().skip(SHAPE_SAMPLES) {
                *slot = rustfft::num_complex::Complex64::default();
            }
            self.fft.process(&mut scratch);
            for (p, v) in power.iter_mut().zip(&scratch) {
                *p += v.norm_sqr();
            }
        }
        // the passband straddles DC in baseband: the bins from -half..=half, wrapping
        let band: Vec<f64> = (0..=half)
            .map(|k| power[k])
            .chain((1..=half).map(|k| power[SHAPE_FFT - k]))
            .collect();
        match median(band.iter().copied()) {
            Some(mid) if mid > 0.0 => {
                let peak = band.iter().copied().fold(0.0, f64::max);
                10.0 * (peak / mid).log10()
            }
            _ => f64::NAN,
        }
    }
}

impl std::fmt::Debug for BusyDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BusyDetector")
            .field("level_db", &self.level_db)
            .field("floor_db", &self.floor_db)
            .field("shape_db", &self.shape_db)
            .field("busy_until", &self.busy_until)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// The median of some powers, or `None` of none. Linear, so it is the median block and
/// not a mean that a surge would pull up.
fn median(powers: impl Iterator<Item = f64>) -> Option<f64> {
    let mut sorted: Vec<f64> = powers.collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(sorted[sorted.len() / 2])
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
            (median - floor).abs() < 1.0,
            "the floor is the noise's typical level: median {median:.1}, floor {floor:.1}"
        );
    }

    #[test]
    fn case_2_a_single_short_spike_does_not_trigger_busy() {
        // one block, +20 dB: a static crash. The attack needs half of 400 ms.
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
        // busy clears within the hangover, plus the half of the attack window that is
        // still over the threshold when the signal stops: the release latency
        let cleared = after.iter().position(|r| !r.4).expect("busy never cleared");
        let drain_s = (ATTACK_WINDOW - ATTACK_MAJORITY) as f64 * 0.025;
        assert!(
            cleared as f64 * 0.025 <= detector.config().hang_s + drain_s + 0.05,
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

    /// Noise plus a tone at `offset_hz` inside the passband, the tone `db` over the noise's
    /// power: what a narrowband signal looks like in baseband.
    fn tone_over_noise(n: usize, sigma: f64, db: f64, offset_hz: f64, seed: u64, phase0: f64) -> Vec<Complex> {
        let amp = sigma * 10f64.powf(db / 20.0) * std::f64::consts::SQRT_2;
        noise(n, sigma, seed)
            .into_iter()
            .enumerate()
            .map(|(i, (re, im))| {
                let ph = phase0 + 2.0 * std::f64::consts::PI * offset_hz * i as f64 / 8000.0;
                (re + amp * ph.cos(), im + amp * ph.sin())
            })
            .collect()
    }

    #[test]
    fn a_narrowband_signal_too_weak_for_the_level_is_caught_by_its_shape() {
        // An FT8 station behind the receiver's AGC measured 4.5 dB over the noise between
        // periods — under any margin that rejects storm noise — and 15 dB peaked in the
        // passband. The level path cannot see it; the shape path must.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let fs = detector.config().fs;
        let block = (detector.config().block_s * fs) as usize;
        let mut now = 0.0;
        for i in 0..secs(10.0) {
            now += 0.025;
            detector.push(&noise(block, 0.01, 40 + i as u64), now);
        }
        assert!(!detector.busy(now), "noise alone is quiet");
        let floor_before = detector.floor_db;
        // +4 dB of tone at +60 Hz: a 3 s "period" of it
        let mut phase = 0.0;
        for i in 0..secs(3.0) {
            now += 0.025;
            let samples = tone_over_noise(block, 0.01, 4.0, 60.0, 900 + i as u64, phase);
            phase += 2.0 * std::f64::consts::PI * 60.0 * block as f64 / fs;
            detector.push(&samples, now);
        }
        assert!(
            detector.level_db - floor_before < 6.0,
            "the test is meant to be under the level margin: {:+.1} dB",
            detector.level_db - floor_before
        );
        assert!(
            detector.shape_db >= detector.config().shape_db,
            "the passband should read peaked: {:.1} dB",
            detector.shape_db
        );
        assert!(detector.busy(now), "a narrowband signal under the level margin is busy by shape");
        assert!(matches!(detector.reason(), Some(BusyReason::Shape { .. })));
        assert!(
            (detector.floor_db - floor_before).abs() < 1.0,
            "and the floor did not learn it: {floor_before:.1} -> {:.1}",
            detector.floor_db
        );
    }

    #[test]
    fn one_flat_window_inside_a_narrowband_signal_does_not_drop_busy() {
        // an FT8 tone hop straddling a window boundary reads flat for one window; the
        // signal is still there and busy must not blink
        let mut detector = BusyDetector::new(BusyConfig::default());
        let fs = detector.config().fs;
        let block = (detector.config().block_s * fs) as usize;
        let mut now = 0.0;
        for i in 0..secs(10.0) {
            now += 0.025;
            detector.push(&noise(block, 0.01, 300 + i as u64), now);
        }
        let mut phase = 0.0;
        let mut feed = |d: &mut BusyDetector, now: &mut f64, blocks: usize, db: f64, seed: u64| {
            for i in 0..blocks {
                *now += 0.025;
                d.push(&tone_over_noise(block, 0.01, db, 60.0, seed + i as u64, phase), *now);
                phase += 2.0 * std::f64::consts::PI * 60.0 * block as f64 / fs;
            }
        };
        feed(&mut detector, &mut now, SHAPE_BLOCKS * 4, 8.0, 2000); // four peaked windows
        assert!(detector.busy(now), "busy on the signal");
        feed(&mut detector, &mut now, SHAPE_BLOCKS, -30.0, 3000); // one window of noise alone
        assert!(detector.busy(now), "one flat window must not drop it");
        feed(&mut detector, &mut now, SHAPE_BLOCKS * 2, 8.0, 4000);
        assert!(detector.busy(now), "and the signal carries on");
    }

    #[test]
    fn flat_noise_at_any_level_is_not_peaked() {
        // the shape of noise: about 6 dB peak over median in the passband, never 12
        let mut detector = BusyDetector::new(BusyConfig::default());
        let block = (detector.config().block_s * detector.config().fs) as usize;
        let mut worst = f64::MIN;
        let mut now = 0.0;
        for i in 0..secs(30.0) {
            now += 0.025;
            // the level wanders +-6 dB block to block, as storm noise does
            let sigma = 0.01 * 10f64.powf(((i % 7) as f64 - 3.0) * 2.0 / 20.0);
            detector.push(&noise(block, sigma, 70 + i as u64), now);
            if detector.shape_db.is_finite() {
                worst = worst.max(detector.shape_db);
            }
        }
        assert!(
            worst < detector.config().shape_db,
            "flat noise read as peaked: {worst:.1} dB against {}",
            detector.config().shape_db
        );
        assert!(!matches!(detector.reason(), Some(BusyReason::Shape { .. })));
    }

    #[test]
    fn two_alternating_stations_leave_the_floor_at_the_gaps() {
        // Two FT8 stations on alternate periods, each 12.6 s, with 2.4 s of noise between:
        // the channel is occupied nine seconds in ten. The strong one is +6 dB by level
        // (what the AGC leaves of a strong signal), the weak one +4. Both are peaked, so
        // neither is floor evidence, the floor stays at the gaps, and both periods are busy.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let fs = detector.config().fs;
        let block = (detector.config().block_s * fs) as usize;
        let mut now = 0.0;
        let mut seed = 0u64;
        let feed_noise = |d: &mut BusyDetector, now: &mut f64, s: f64, seed: &mut u64| {
            for _ in 0..secs(s) {
                *now += 0.025;
                *seed += 1;
                d.push(&noise(block, 0.01, 1000 + *seed), *now);
            }
        };
        let feed_tone = |d: &mut BusyDetector, now: &mut f64, s: f64, db: f64, hz: f64, seed: &mut u64| -> f64 {
            let mut phase = 0.0;
            let mut busy_blocks = 0usize;
            let n = secs(s);
            for _ in 0..n {
                *now += 0.025;
                *seed += 1;
                d.push(&tone_over_noise(block, 0.01, db, hz, 5000 + *seed, phase), *now);
                phase += 2.0 * std::f64::consts::PI * hz * block as f64 / fs;
                if d.busy(*now) {
                    busy_blocks += 1;
                }
            }
            busy_blocks as f64 / n as f64
        };
        feed_noise(&mut detector, &mut now, 10.0, &mut seed);
        let floor_before = detector.floor_db;
        let mut worst_floor = f64::MIN;
        let mut strong_share = 0.0;
        let mut weak_share = 0.0;
        for _ in 0..3 {
            strong_share += feed_tone(&mut detector, &mut now, 12.6, 6.0, 80.0, &mut seed);
            worst_floor = worst_floor.max(detector.floor_db);
            feed_noise(&mut detector, &mut now, 2.4, &mut seed);
            weak_share += feed_tone(&mut detector, &mut now, 12.6, 4.0, -120.0, &mut seed);
            worst_floor = worst_floor.max(detector.floor_db);
            feed_noise(&mut detector, &mut now, 2.4, &mut seed);
        }
        assert!(
            worst_floor - floor_before < 1.5,
            "the floor climbed into the stations: {floor_before:.1} -> {worst_floor:.1}"
        );
        assert!(strong_share / 3.0 > 0.9, "the strong station busy only {:.0}%", strong_share / 3.0 * 100.0);
        assert!(weak_share / 3.0 > 0.9, "the weak station busy only {:.0}%", weak_share / 3.0 * 100.0);
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
