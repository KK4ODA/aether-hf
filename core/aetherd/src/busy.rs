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
//! its dips would show, but the floor follows the radio's gain through them (below).
//!
//! # The radio's gain
//!
//! The steady gate keeps a gain *transient* out of the floor; it cannot keep out a gain *held*
//! down. While a strong signal anywhere in the passband lasts — a carrier beside the channel,
//! a station outside the DSP filter that the AGC still feels — the noise under the cut is
//! quiet, steady and low, every test of evidence passes, and a floor in dBFS learns it. When
//! the AGC lets go, the true noise stands over that floor by the cut, no block is evidence any
//! more, and the channel read busy until the hold ran out: 71 s, 57 times in 108 hours of one
//! station's logs (ADR-0078). So the floor is learned against the radio's gain and drawn at the
//! gain of the moment. The gain is read from the radio's whole audio passband
//! ([`PassbandReference`]): the AGC scales every frequency together, so each spectral bin falls
//! by the cut against its own level on quiet, while a signal only lifts the few bins it holds.
//! A cut lowers the floor at once and the gain coming back raises it at once; the evidence gate
//! cannot stagnate, because the noise under the cut and after it are the same number in the
//! floor's units.
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
//! * a **frame the receiver found** — decoded or not, announced or reported — is no evidence
//!   of the noise: its span is taken out of the floor (`explain`), the part already heard as
//!   well. A weak station's frames sit under the margin and a tone frame's envelope is steady,
//!   so in a session both stations' floors had become the other's signal; and since a decoded
//!   frame's hold is a politeness, not a measurement, the quiet after the frame — in a session
//!   the only quiet there is — counts as noise again (ADR-0076);
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
    /// Where the channel sits in the radio's audio, in hertz: the waveform's centre. The
    /// passband reference leaves the channel out where the passband has room beside it.
    pub centre_hz: f64,
    /// Whether the floor follows the radio's gain.
    pub gain: GainHandling,
}

/// Whether the floor follows the radio's gain (ADR-0078).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GainHandling {
    /// The floor in dBFS, as before ADR-0078: a gain an AGC holds down is learned as the noise,
    /// and the true noise reads busy when it lets go. Kept to replay a recording both ways.
    Plain,
    /// The floor learned against the radio's gain as the passband shows it, and drawn at the
    /// gain of the moment.
    #[default]
    Normalised,
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
            centre_hz: 1500.0,
            gain: GainHandling::Normalised,
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

/// What the detector made of one block: its power and every test that decided whether it was
/// evidence of the noise. A trace of these (`aetherd --busy-trace`) is how a floor that went
/// wrong on the air is taken apart afterwards.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a record of independent tests, one flag each"
)]
pub struct BlockNote {
    /// Where the block ended, on the caller's clock.
    pub end_s: f64,
    /// The block's power, dBFS.
    pub power_db: f64,
    /// Within 3 dB of its neighbours over the last 200 ms.
    pub steady: bool,
    /// At least the threshold over the floor learned before it.
    pub over: bool,
    /// Inside a shape window that read peaked.
    pub peaked: bool,
    /// Inside the hangover of a signal the level or shape path found.
    pub signal: bool,
    /// Inside a frame the receiver found ([`BusyDetector::explain`]).
    pub in_frame: bool,
    /// Taken as evidence of the noise floor.
    pub evidence: bool,
    /// The window held no evidence after it, so the floor was held (or, past the hold,
    /// taken from every block).
    pub held: bool,
    /// The radio's gain as the passband showed it at the block, dB; NaN when there is no
    /// reading (no audio handed over, the warm-up, or too few bins carrying the band's noise).
    pub gain_db: f64,
    /// The gain the floor was drawn at, dB: 0 when the floor is not normalised.
    pub drawn_db: f64,
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
    /// The blocks of the floor window, newest last, capped at `window_blocks`.
    history: std::collections::VecDeque<Block>,
    /// The radio's gain as the noise across its audio passband shows it, when the caller
    /// hands over the audio ([`Self::hear_passband`]).
    reference: Option<PassbandReference>,
    /// The gain the floor is drawn at, dB, never above 0: the reading, or where the reading
    /// includes the channel, the reading held down until it comes back near 0 ([`Self::draw`]).
    drawn_db: f64,
    /// Until when the level or the shape path last found a signal on the channel, with its
    /// hangover: a frame's hold makes the channel busy without measuring anything, and does
    /// not keep the quiet after the frame from the floor (ADR-0076).
    signal_until: f64,
    /// Until when a frame the receiver found or announced occupies the channel
    /// ([`Self::explain`]): its blocks are the frame's, not noise.
    explained_until: f64,
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
    /// The floor in its own units, linear: power, over the radio's gain when the floor is
    /// normalised; 0 before anything is learned.
    floor_q: f64,
    /// Blocks discarded because this station was transmitting.
    pub blocks_skipped: usize,
    /// The latest block, as the detector judged it.
    pub last_block: Option<BlockNote>,
}

/// One block of the floor window.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Block {
    /// Raw power, linear.
    power: f64,
    /// The gain the floor was drawn at for this block, linear; 1 when not normalised.
    gain: f64,
    /// Evidence of the noise floor: steady, not silence, captured while no signal was
    /// measured on the channel and inside no frame the receiver found.
    evidence: bool,
    /// Steady, not peaked, not in a frame, not silence: what a returned noise looks like.
    plain: bool,
    /// Inside a shape window that read peaked.
    peaked: bool,
    /// When the block ended.
    end: f64,
}

/// A gain this far under 0 dB is a cut: the bins' levels on quiet do not learn from it.
const CUT_DB: f64 = 3.0;
/// Where the gain reading includes the channel, how near 0 dB it must come before the floor
/// follows it back up, dB ([`BusyDetector::draw`]).
const GAIN_RETURN_DB: f64 = 3.0;
/// A move of the gain reading larger than this is followed at once, dB; a smaller one is eased
/// toward by [`GAIN_EASE`] a block. The reading wanders about a decibel either way on steady
/// noise, and a floor drawn at every wander judged a signal at the threshold against a moving
/// line (C-rtty-+0-slow: busy 0.25 s later than the plain floor's).
const GAIN_STEP_DB: f64 = 2.0;
/// The share of the way to the reading the drawn gain moves each block within
/// [`GAIN_STEP_DB`]: a second's time constant.
const GAIN_EASE: f64 = 0.025;

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
/// A narrowband peak in the passband counts as a signal only if the peak also stands within
/// this many dB of the learned noise floor. A real signal — even one an AGC has held down to
/// a few decibels over the noise — has its peak at or above the floor; a receiver spur or a
/// local birdie sits well below it, yet is just as "peaked" against the passband's own
/// median, which the band filter's near-zero skirt bins drag down. Judging the peak against
/// that median alone called any faint narrowband thing on an otherwise-quiet channel busy —
/// a station's own spur held the indicator on with nothing on the air, whatever the dial.
/// Measured across recordings: a genuine narrowband signal's peak sat about 5 dB below a
/// (signal-contaminated) floor estimate and a spur about 12 dB below, so the gate splits
/// them with margin either way.
const SHAPE_PEAK_FLOOR_MARGIN_DB: f64 = 10.0;

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
            reference: None,
            drawn_db: 0.0,
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
            signal_until: f64::NEG_INFINITY,
            explained_until: f64::NEG_INFINITY,
            level_db: f64::NEG_INFINITY,
            floor_db: f64::NEG_INFINITY,
            floor_q: 0.0,
            blocks_skipped: 0,
            last_block: None,
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

    /// How far the radio's gain stands under its level on quiet, in dB, as the passband
    /// reference shows it: about 0 when nothing holds it down, −20 when an AGC has cut it by
    /// 20 dB. `None` before the detector has a reference and a level on quiet to measure it
    /// against, or when its floor is not normalised.
    #[must_use]
    pub fn gain_db(&self) -> Option<f64> {
        if !self.normalising() {
            return None;
        }
        Some(self.reference.as_ref()?.latest()?.min(0.0))
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

    /// The longest a signal can have been on the channel before this detector calls it
    /// busy, in seconds. The shape path is the slower: it attacks on two peaked windows in a
    /// row, and a signal that starts inside a window may leave that one reading flat, so three
    /// windows; the level path needs half its attack window for a signal that stays on and
    /// all of it for one keyed half the time. One block more for the one still being
    /// gathered. Audio further back than this, with the channel still reading clear, held
    /// nothing either path could see — what a caller measuring the noise *between* signals
    /// has to wait for before it trusts a window (`Station::sample_passband`).
    #[must_use]
    pub fn attack_s(&self) -> f64 {
        ((3 * SHAPE_BLOCKS).max(ATTACK_WINDOW) + 1) as f64 * self.config.block_s
    }

    /// Mark the channel busy because a frame was **decoded**.
    ///
    /// A decoded frame is there whatever the power measurement says, and it works below the
    /// threshold, which is the case that matters for politeness. An acquisition alone is
    /// not called here: see the module notes.
    ///
    /// The attack's votes go: they are the frame's own blocks, now explained. Kept, the next
    /// block found a majority of them over the threshold and named the busy channel energy
    /// again, and an answer that waits only for energy no frame accounts for — a `DISC_ACK` —
    /// waited out the hangover of the very `DISC` it answered, into its retry (the
    /// three-clients scenario, 2–3 s at a client's leaving; ADR-0053).
    pub fn mark_frame(&mut self, now: f64, detect_confidence: f64) {
        self.busy_until = self.busy_until.max(now + self.config.frame_hold_s);
        self.reason = Some(BusyReason::Frame { detect_confidence });
        self.over.clear();
    }

    /// A frame the receiver found or announced occupies `from..until` on the detector's clock:
    /// its blocks are the frame's energy, not the noise's, whatever the power measurement made
    /// of them — those already taken in are taken back out of the floor, those still to come
    /// stay out.
    ///
    /// A weak station's frames sit under the margin — at 0 dB in 3 kHz a 500 Hz signal lifts
    /// the passband's power by 3 dB — and a tone frame's envelope is constant, so steady: on
    /// WC4Y's Test of 2026-10-17 both stations' floors became the other's signal, KK4ODA-1's
    /// −18 dBFS against a real −28 and WC4Y's −12 against −19, and the panel drew signal as
    /// noise (13-wc4y-beta111.md §13.2). A detection that turns out to be noise only keeps a
    /// stretch of noise out of the floor, which then holds; nothing is marked busy by this.
    pub fn explain(&mut self, from: f64, until: f64) {
        self.explained_until = self.explained_until.max(until);
        let mut changed = false;
        for entry in self.history.iter_mut().rev() {
            if entry.end <= from {
                break;
            }
            changed |= entry.evidence;
            entry.evidence = false;
            entry.plain = false;
        }
        if changed {
            self.refloor();
        }
    }

    /// The floor from the evidence the window holds now; held as it is when there is none.
    fn refloor(&mut self) {
        if let Some(q) = self.evidence_floor() {
            self.floor_q = q;
            let gain = self.history.back().map_or(1.0, |b| b.gain);
            self.floor_db = 10.0 * (q * gain).max(FLOOR).log10();
        }
    }

    /// The median of the window's evidence, in the floor's own units: linear power, over the
    /// radio's gain when the floor is normalised.
    fn evidence_floor(&self) -> Option<f64> {
        median(
            self.history
                .iter()
                .filter(|b| b.evidence)
                .map(|b| b.power / b.gain),
        )
    }

    /// Whether the floor is learned against the passband reference (C2).
    fn normalising(&self) -> bool {
        self.config.gain == GainHandling::Normalised && self.reference.is_some()
    }

    /// The gain the floor is drawn at for a block whose gain reads `measured` (dB, `None` with no
    /// reading): the reading, never above 0 dB — a signal only adds power, so a reading over
    /// the bins' levels on quiet is a signal, not more gain — and held where it was while there
    /// is no reading. A move of more than [`GAIN_STEP_DB`] is taken at once, a smaller one
    /// eased toward, so the floor does not wander with the reading. Where the reading includes the channel (a 2300 Hz channel fills the
    /// passband), a wide signal in the channel lifts it exactly as the gain coming back would,
    /// so the floor follows it down at once but back up only once it is within
    /// [`GAIN_RETURN_DB`] of 0: a signal in the channel under a cut stays over the floor, at the
    /// price of a slow AGC's ramp back read as busy until it is nearly done.
    fn draw(&self, measured: Option<f64>) -> f64 {
        let Some(g) = measured.map(|g| g.min(0.0)) else {
            return self.drawn_db;
        };
        let holds = self
            .reference
            .as_ref()
            .is_some_and(PassbandReference::includes_channel);
        let target = if holds && g > self.drawn_db && g < -GAIN_RETURN_DB {
            self.drawn_db
        } else {
            g
        };
        if (target - self.drawn_db).abs() > GAIN_STEP_DB {
            target
        } else {
            (target - self.drawn_db).mul_add(GAIN_EASE, self.drawn_db)
        }
    }

    /// Hand over the radio's audio as captured, before its baseband goes to [`Self::push`]:
    /// the noise across the radio's whole audio passband follows its gain, whatever a
    /// narrowband signal on top does.
    pub fn hear_passband(&mut self, audio: &[f32], audio_rate: f64, now: f64) {
        let block_s = self.config.block_s;
        let (centre, half) = (self.config.centre_hz, self.config.passband_hz / 2.0);
        let occupied = (centre - half, centre + half);
        let heard = (
            centre - half - CHANNEL_GUARD_HZ,
            centre + half + CHANNEL_GUARD_HZ,
        );
        self.reference
            .get_or_insert_with(|| PassbandReference::new(audio_rate, block_s, occupied, heard))
            .push(audio, now);
    }

    /// Discard audio captured while this station was transmitting.
    ///
    /// A half-duplex station hears its own sidetone; letting that into the floor estimate
    /// would raise the floor by tens of dB and blind the detector for the whole window.
    ///
    /// The attack's votes and the shape path's window go too: they describe the channel
    /// before this station keyed, and a block heard after it would otherwise be judged
    /// together with them. A peer's signal that ended just before the key went down left
    /// eight votes over the threshold, and the first quiet block after the station's own
    /// deafness made the channel busy on them, with nothing on the air.
    pub fn skip(&mut self, samples: &[Complex]) {
        self.partial.clear();
        self.over.clear();
        self.shape_buffer.clear();
        self.peaked = [false, false];
        if let Some(reference) = self.reference.as_mut() {
            reference.skip();
        }
        self.blocks_skipped += samples.len() / self.block_samples.max(1);
    }

    /// Feed received baseband, and get the occupancy as of the end of it.
    pub fn push(&mut self, samples: &[Complex], now: f64) -> bool {
        self.partial.extend_from_slice(samples);
        let mut consumed = 0;
        while self.partial.len() - consumed >= self.block_samples {
            let block = &self.partial[consumed..consumed + self.block_samples];
            consumed += self.block_samples;
            // where this block ended and began, on the caller's clock: `now` is the end of
            // what was fed
            let block_end = now - (self.partial.len() - consumed) as f64 / self.config.fs;
            let block_start = block_end - self.config.block_s;
            let power = block
                .iter()
                .map(|&(re, im)| re.mul_add(re, im * im))
                .sum::<f64>()
                / self.block_samples as f64;
            let measured = self.reference.as_ref().and_then(|r| r.at(block_end));
            if self.normalising() {
                self.drawn_db = self.draw(measured);
            }
            let gain = 10f64.powf(self.drawn_db / 10.0);
            // the floor learned before this block, at this block's gain
            if self.normalising() && self.floor_q > 0.0 {
                self.floor_db = 10.0 * (self.floor_q * gain).max(FLOOR).log10();
            }

            let steady = self.steady_with(power);
            // the shape path: a narrowband signal an AGC held down out of the level reading
            self.shape_buffer.extend_from_slice(block);
            let peaked_now = self.note_shape(now);

            // The separation the whole detector rests on: the busy decision is made on this
            // block's energy against the floor learned *before* it, and the block only
            // becomes floor evidence if the channel was not busy when it was taken. A block
            // captured under a signal is not noise, however long the signal lasts.
            self.level_db = 10.0 * power.max(FLOOR).log10();
            let was_signal = block_end < self.signal_until;
            let excess = self.level_db - self.floor_db;
            self.excess_peak_db = self.excess_peak_db.max(excess);
            let over = self.floor_db.is_finite() && excess >= self.config.threshold_db;
            self.over.push_back(over);
            if self.over.len() > ATTACK_WINDOW {
                self.over.pop_front();
            }
            if self.over.iter().filter(|&&o| o).count() >= ATTACK_MAJORITY {
                self.busy_until = self.busy_until.max(now + self.config.hang_s);
                self.signal_until = self.signal_until.max(now + self.config.hang_s);
                self.reason = Some(BusyReason::Level {
                    level_db: self.level_db,
                    floor_db: self.floor_db,
                });
            }
            // evidence of the floor: steady, not a muted card, not under a signal — by level
            // or by shape — and inside no frame the receiver found (`explain`); the block that
            // crosses the threshold is itself excluded, whether or not it goes on to make the
            // channel busy. A decoded frame's hold is not a signal measured: the quiet after the
            // frame is the noise, and in a session it is the only quiet there is (ADR-0076)
            let in_frame = block_start < self.explained_until;
            let plain = steady && power > SILENCE && !peaked_now && !in_frame;
            let evidence = plain && !was_signal && !over;
            if let Some(reference) = self.reference.as_mut() {
                reference.learn(block_end, evidence);
            }
            self.history.push_back(Block {
                power,
                gain,
                evidence,
                plain,
                peaked: peaked_now,
                end: block_end,
            });
            if self.history.len() > self.window_blocks {
                self.history.pop_front();
            }

            let held = !self.settle_floor(now);
            self.last_block = Some(BlockNote {
                end_s: block_end,
                power_db: self.level_db,
                steady,
                over,
                peaked: peaked_now,
                signal: was_signal,
                in_frame,
                evidence,
                held,
                gain_db: measured.unwrap_or(f64::NAN),
                drawn_db: self.drawn_db,
            });
        }
        self.partial.drain(..consumed);
        self.busy(now)
    }

    /// Steady: the last few blocks and one of `power` stayed within a few dB — a receiver's AGC
    /// stepping down and ramping back never is.
    fn steady_with(&self, power: f64) -> bool {
        self.history.len() + 1 >= STEADY_BLOCKS && {
            let recent = self
                .history
                .iter()
                .rev()
                .take(STEADY_BLOCKS - 1)
                .map(|b| b.power)
                .chain(std::iter::once(power));
            let (low, high) = recent.fold((f64::INFINITY, 0.0_f64), |(lo, hi), p| {
                (lo.min(p), hi.max(p))
            });
            high <= low.max(FLOOR) * 10f64.powf(STEADY_RANGE_DB / 10.0)
        }
    }

    /// The floor from the window as it stands at `now`; whether the window held evidence for it.
    fn settle_floor(&mut self, now: f64) -> bool {
        let learned = if let Some(floor) = self.evidence_floor() {
            self.floor_q = floor;
            self.held_since = None;
            true
        } else {
            // No signal-free block in the window: the floor learned before is held — through an
            // FT8 period, an SSB over — until the hold runs out. Then, or when nothing has been
            // learned yet at all, the median of every block, silence excepted: a signal that
            // never stops is the environment, and a floor that will not learn it is a floor that
            // can never recover.
            let since = *self.held_since.get_or_insert(now);
            if self.floor_q <= 0.0 || now - since >= self.config.floor_hold_s {
                let all = median(
                    self.history
                        .iter()
                        .filter(|b| b.power > SILENCE)
                        .map(|b| b.power / b.gain),
                );
                if let Some(all) = all {
                    self.floor_q = all;
                }
                // what is there now is the environment, the gain included
                self.rebase();
            }
            false
        };
        if self.floor_q > 0.0 {
            let gain = 10f64.powf(self.drawn_db / 10.0);
            self.floor_db = 10.0 * (self.floor_q * gain).max(FLOOR).log10();
        }
        learned
    }

    /// Take the gain the floor is drawn at now as the normal one, the floor's units moving
    /// with it, so the floor in dBFS is unchanged: the hold has run out and what is there is the
    /// environment, the radio's gain included. Without it a reading stuck under 0 dB — the
    /// baselines learned before an attenuator went in, say — would, where the reading includes
    /// the channel, keep the floor from following the gain back up after every cut.
    fn rebase(&mut self) {
        if !self.normalising() || self.drawn_db >= 0.0 {
            return;
        }
        let g = 10f64.powf(self.drawn_db / 10.0);
        for b in &mut self.history {
            b.gain /= g;
        }
        self.floor_q *= g;
        if let Some(reference) = self.reference.as_mut() {
            reference.rebase(self.drawn_db);
        }
        self.drawn_db = 0.0;
    }
}

impl BusyDetector {
    /// The passband's highest spectral bin over its median bin, in dB, from the blocks
    /// gathered since the last window: four 50 ms transforms averaged, so a bin is
    /// judged on 200 ms and not on one noisy periodogram.
    /// Judge the passband's shape once the running buffer (filled by the caller) holds a
    /// whole window — eight blocks, 200 ms — and otherwise carry the last verdict forward.
    /// Returns whether this window was peaked as a narrowband signal's spectrum is, and — on
    /// an attack, or while it holds — marks the channel busy. See [`SHAPE_PEAK_FLOOR_MARGIN_DB`]
    /// for why the peak is weighed against the learned floor, not the passband's own median.
    fn note_shape(&mut self, now: f64) -> bool {
        if self.shape_buffer.len() < SHAPE_BLOCKS * self.block_samples {
            return self.peaked[1];
        }
        let window_power = self
            .shape_buffer
            .iter()
            .map(|&(re, im)| re.mul_add(re, im * im))
            .sum::<f64>()
            / self.shape_buffer.len() as f64;
        let (peak_db, peaked) = if window_power > SILENCE {
            let (peak_over_median, peak_share) = self.passband_shape();
            // The peak over the passband's own median says how narrowband it is; that alone
            // cannot tell a real weak signal from a spur. Its absolute level, weighed against
            // the learned noise floor, can: a signal an AGC has held down still stands at the
            // floor, a spur sits below it. Before the floor is known there is nothing to
            // weigh it against, so nothing is called busy on shape yet.
            let peak_abs_db = 10.0 * window_power.log10() + peak_share;
            let over_floor = self.floor_db.is_finite()
                && peak_abs_db >= self.floor_db - SHAPE_PEAK_FLOOR_MARGIN_DB;
            (
                peak_over_median,
                peak_over_median >= self.config.shape_db && over_floor,
            )
        } else {
            // digital silence has no shape: a muted sound card, or nothing on the input, is
            // numerical noise and can look peaked, and it says nothing about the channel
            (f64::NAN, false)
        };
        self.shape_buffer.clear();
        self.shape_db = peak_db;
        self.peaked = [self.peaked[1], peaked];
        // attack on two peaked windows in a row; once busy, one is enough to hold it. An FT8
        // tone hop that straddles a window boundary splits its energy between two bins and
        // that window reads flat for 200 ms — measured, it dropped the channel for a second
        // in the middle of a period. A signal peaked in one window of two has not gone.
        let attack = self.peaked.iter().all(|&p| p);
        let hold = self.busy(now) && self.peaked.iter().any(|&p| p);
        if attack || hold {
            self.busy_until = self.busy_until.max(now + self.config.hang_s);
            self.signal_until = self.signal_until.max(now + self.config.hang_s);
            if attack {
                self.reason = Some(BusyReason::Shape { peak_db });
            }
        }
        peaked
    }

    /// The passband's shape, from its periodogram: the highest bin over the *median* bin
    /// (how far a narrowband peak stands above the noise between the bins — noise sits near
    /// 6 dB, a signal far higher), and the highest bin over the *whole passband's* power
    /// (the peak's share of the energy: near unity for a lone tone, `1/bins` for spread
    /// noise), which the caller turns into the peak's absolute level to weigh against the
    /// learned floor. Both in dB; `(NaN, -inf)` when the passband holds no power.
    fn passband_shape(&self) -> (f64, f64) {
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
                let w = 0.5
                    - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / SHAPE_SAMPLES as f64).cos();
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
        let sum: f64 = band.iter().sum();
        match median(band.iter().copied()) {
            Some(mid) if mid > 0.0 && sum > 0.0 => {
                let peak = band.iter().copied().fold(0.0, f64::max);
                (10.0 * (peak / mid).log10(), 10.0 * (peak / sum).log10())
            }
            _ => (f64::NAN, f64::NEG_INFINITY),
        }
    }
}

/// The radio's gain, from the noise across its audio passband.
///
/// A receiver's AGC scales everything in its passband by one factor. Each spectral bin of the
/// passband is measured against its own level on quiet, and the gain is a low quantile of
/// those differences across the bins: a cut moves every bin down together, while a signal on
/// top only ever *raises* the bins it holds — up to three in four of them may hold one and the
/// reading stays on the noise. Each bin is judged against itself, so a radio's filter skirt,
/// a sound card's tilt or a quiet edge of the band costs nothing.
///
/// A bin's level on quiet is learned only while the gain reads uncut and the channel's own
/// block was evidence of the noise ([`Self::learn`]): a cut is never learned as the normal
/// level, and neither is a wide signal over the channel. Where the passband has room beside
/// the channel the channel's bins are left out of the reading; a 2300 Hz channel fills the
/// passband, and there the reading includes it ([`Self::includes_channel`]).
#[derive(Clone)]
pub struct PassbandReference {
    rate: f64,
    block: usize,
    pending: Vec<f32>,
    fft: std::sync::Arc<dyn rustfft::Fft<f64>>,
    window: Vec<f64>,
    /// Every bin of [`REFERENCE_BAND_HZ`].
    bins: Vec<usize>,
    /// Which of `bins` the gain is read from: those beside the channel, or all of them.
    gain_bins: Vec<bool>,
    /// Which of `bins` lie in the channel's occupied band: the noise the floor is of.
    channel_bins: Vec<bool>,
    includes_channel: bool,
    /// Each bin's power in the last [`GAIN_SMOOTH`] blocks, linear, newest last.
    recent: std::collections::VecDeque<Vec<f64>>,
    /// Each bin's level on quiet, dB: a quantile tracker of the bin's own level.
    baseline: Vec<f64>,
    /// The bins' levels through the warm-up, until the baselines are set from them.
    warm: Vec<Vec<f64>>,
    /// The last few raw gain readings, for the median that smooths them.
    raw: std::collections::VecDeque<f64>,
    /// Readings, newest last: two seconds of them.
    readings: std::collections::VecDeque<GainReading>,
}

/// One reading of the passband: the gain and the bins' levels it came from.
#[derive(Clone)]
struct GainReading {
    /// The end of the audio it covers.
    end: f64,
    /// The gain against the bins' levels on quiet, dB; `None` while warming up or when too few
    /// bins carry the band's noise.
    gain_db: Option<f64>,
    /// Each bin's level, dB.
    levels: Vec<f64>,
    /// Whether the baselines have learned from it.
    learned: bool,
}

/// The audio band the reference is taken over, Hz: inside any SSB filter of 2.4 kHz.
const REFERENCE_BAND_HZ: (f64, f64) = (300.0, 2700.0);
/// How far either side of the channel's occupied band the channel's band filter still lets
/// energy through, Hz: a subcarrier and the filter's transition band. The reference leaves
/// it out, so a signal in the channel cannot lift the reference the floor is drawn at.
const CHANNEL_GUARD_HZ: f64 = 280.0;
/// The fewest bins outside the channel the gain is read from; with fewer (a 2300 Hz channel
/// fills a 2.4 kHz passband) it is read from every bin.
const MIN_REFERENCE_BINS: usize = 20;
/// The quantile both of each bin's level that its baseline tracks and of the bins' differences
/// that the gain reads. The same on both sides, so noise alone reads 0 dB with no correction;
/// signals may lift three bins in four, and a baseline is left on the noise by a signal there
/// three-quarters of the time.
const GAIN_QUANTILE: f64 = 0.25;
/// How many blocks a bin's level is averaged over: 100 ms, which puts a bin's level within
/// about 2 dB on noise and the quantile across thirty bins within about half a decibel.
const GAIN_SMOOTH: usize = 4;
/// How many raw gain readings the reading is the median of: 75 ms. One reading wanders ±2 dB
/// on noise alone, and the floor's dead band ([`BusyDetector::draw`]) takes the rest; more
/// readings put the reading further behind the radio, and a fast AGC's dip after a static crash
/// was read after the noise was back (D-crashes-fast: 1.9 s of false busy with five).
const GAIN_READINGS: usize = 3;
/// A baseline's step down per reading it learns from, dB (0.8 dB/s); its step up is a third of
/// that, which is what puts it at [`GAIN_QUANTILE`].
const BASELINE_STEP_DB: f64 = 0.02;
/// The first readings are a warm-up: no gain is read, and at its end each bin's level on quiet is
/// set from them ([`PassbandReference::warm_up`]): two seconds.
const WARM_READINGS: usize = 80;
/// A bin carries the band's noise, and so follows the gain, when its level on quiet is within
/// this of the channel's, dB, either way. A bin outside a narrow radio filter holds only the
/// sound card's own noise, which no AGC moves; a bin far above the channel's has learned a
/// signal that was there more often than not, and would read its absence as a cut.
const INFORM_DB: f64 = 10.0;
/// The fewest bins carrying the band's noise a gain is read from.
const MIN_INFORMED_BINS: usize = 10;

impl PassbandReference {
    /// A reference for audio at `rate`, read every `block_s`, for a channel whose occupied band
    /// is `occupied` (Hz) and which the band filter hears out to `heard` (Hz).
    #[must_use]
    pub fn new(rate: f64, block_s: f64, occupied: (f64, f64), heard: (f64, f64)) -> Self {
        let block = ((rate * block_s) as usize).max(16);
        let bin_hz = rate / block as f64;
        // four-term Blackman-Harris: sidelobes 92 dB down. A carrier 40 dB over the noise in
        // 3 kHz stands 59 dB over a 40 Hz bin's, and through a Hann window its skirt lifted
        // every bin within 400 Hz of it — half of one side of the reference
        let window = (0..block)
            .map(|i| {
                let x = 2.0 * std::f64::consts::PI * i as f64 / block as f64;
                0.01168f64.mul_add(
                    -(3.0 * x).cos(),
                    0.14128f64.mul_add((2.0 * x).cos(), 0.48829f64.mul_add(-x.cos(), 0.35875)),
                )
            })
            .collect();
        let bins: Vec<usize> = ((REFERENCE_BAND_HZ.0 / bin_hz).ceil() as usize
            ..=((REFERENCE_BAND_HZ.1 / bin_hz).floor() as usize).min(block / 2))
            .collect();
        let hz = |k: usize| k as f64 * bin_hz;
        let beside: Vec<bool> = bins
            .iter()
            .map(|&k| hz(k) < heard.0 || hz(k) > heard.1)
            .collect();
        let includes_channel = beside.iter().filter(|&&b| b).count() < MIN_REFERENCE_BINS;
        let gain_bins = if includes_channel {
            vec![true; bins.len()]
        } else {
            beside
        };
        let channel_bins = bins
            .iter()
            .map(|&k| hz(k) >= occupied.0 && hz(k) <= occupied.1)
            .collect();
        Self {
            rate,
            block,
            pending: Vec::with_capacity(block),
            fft: rustfft::FftPlanner::new().plan_fft_forward(block),
            window,
            bins,
            gain_bins,
            channel_bins,
            includes_channel,
            recent: std::collections::VecDeque::with_capacity(GAIN_SMOOTH),
            baseline: Vec::new(),
            warm: Vec::with_capacity(WARM_READINGS),
            raw: std::collections::VecDeque::with_capacity(GAIN_READINGS),
            readings: std::collections::VecDeque::new(),
        }
    }

    /// Whether the gain is read from the channel's own bins too: a 2300 Hz channel fills the
    /// passband, so a wide signal in it lifts the reading as the gain coming back would.
    #[must_use]
    pub fn includes_channel(&self) -> bool {
        self.includes_channel
    }

    /// Feed captured audio ending at `now`.
    pub fn push(&mut self, audio: &[f32], now: f64) {
        self.pending.extend_from_slice(audio);
        let mut taken = 0;
        let mut spectrum = vec![rustfft::num_complex::Complex64::default(); self.block];
        while self.pending.len() - taken >= self.block {
            for ((slot, &x), &w) in spectrum
                .iter_mut()
                .zip(&self.pending[taken..taken + self.block])
                .zip(&self.window)
            {
                *slot = rustfft::num_complex::Complex64::new(f64::from(x) * w, 0.0);
            }
            taken += self.block;
            let end = now - (self.pending.len() - taken) as f64 / self.rate;
            self.fft.process(&mut spectrum);
            if self.recent.len() == GAIN_SMOOTH {
                self.recent.pop_front();
            }
            self.recent
                .push_back(self.bins.iter().map(|&k| spectrum[k].norm_sqr()).collect());
            if self.recent.len() < GAIN_SMOOTH {
                continue;
            }
            let levels: Vec<f64> = (0..self.bins.len())
                .map(|i| {
                    let mean = self.recent.iter().map(|p| p[i]).sum::<f64>() / GAIN_SMOOTH as f64;
                    10.0 * mean.max(FLOOR).log10()
                })
                .collect();
            let gain_db = if self.baseline.is_empty() {
                self.warm_up(&levels);
                None
            } else if let Some(g) = self.gain_of(&levels) {
                if self.raw.len() == GAIN_READINGS {
                    self.raw.pop_front();
                }
                self.raw.push_back(g);
                median(self.raw.iter().copied())
            } else {
                self.raw.clear();
                None
            };
            self.readings.push_back(GainReading {
                end,
                gain_db,
                levels,
                learned: false,
            });
        }
        self.pending.drain(..taken);
        if let Some(newest) = self.readings.back().map(|r| r.end) {
            while self.readings.front().is_some_and(|r| r.end < newest - 2.0) {
                self.readings.pop_front();
            }
        }
    }

    /// The gain these levels show against the baselines, dB: the quantile of the differences
    /// over the bins that carry the band's noise.
    fn gain_of(&self, levels: &[f64]) -> Option<f64> {
        let channel = quantile(
            self.baseline
                .iter()
                .zip(&self.channel_bins)
                .filter(|&(_, &c)| c)
                .map(|(&b, _)| b),
            0.5,
        )?;
        let mut differences: Vec<f64> = levels
            .iter()
            .zip(&self.baseline)
            .zip(&self.gain_bins)
            .filter(|&((_, &b), &used)| used && (b - channel).abs() <= INFORM_DB)
            .map(|((&x, &b), _)| x - b)
            .collect();
        if differences.len() < MIN_INFORMED_BINS {
            return None;
        }
        differences.sort_by(f64::total_cmp);
        Some(differences[(differences.len() as f64 * GAIN_QUANTILE) as usize])
    }

    /// Gather the warm-up's levels, and once there are [`WARM_READINGS`] of them set each bin's
    /// level on quiet to the [`GAIN_QUANTILE`] of its own: a static crash or a burst in the first
    /// seconds lifts a few of them and moves no quantile. Started from the first reading alone,
    /// a crash there left every bin 9 dB high, every reading after it said the gain was cut by 9,
    /// and the bins, which learn only from uncut readings, never came down (the held-out draw's
    /// `D-crashes-off`: busy 47 % of the time with no AGC at all).
    fn warm_up(&mut self, levels: &[f64]) {
        self.warm.push(levels.to_vec());
        if self.warm.len() < WARM_READINGS {
            return;
        }
        self.baseline = (0..self.bins.len())
            .map(|i| quantile(self.warm.iter().map(|l| l[i]), GAIN_QUANTILE).unwrap_or(0.0))
            .collect();
        self.warm = Vec::new();
    }

    /// Move each baseline a step toward the level it saw: down by [`BASELINE_STEP_DB`] when
    /// under it, up by a third of that when over it — the steps that settle a tracker on the
    /// [`GAIN_QUANTILE`] of what it sees.
    fn step_baselines(&mut self, levels: &[f64]) {
        let down = BASELINE_STEP_DB;
        let up = down * GAIN_QUANTILE / (1.0 - GAIN_QUANTILE);
        for (b, &x) in self.baseline.iter_mut().zip(levels) {
            *b += if x < *b { -down } else { up };
        }
    }

    /// The block of the channel ending at `t` was `quiet` — evidence of the noise floor: when
    /// the gain read then is uncut, the bins' levels then are the noise's, and each baseline
    /// takes a step toward its own.
    pub fn learn(&mut self, t: f64, quiet: bool) {
        let Some(index) = self.index_at(t) else {
            return;
        };
        let reading = &mut self.readings[index];
        let uncut = reading.gain_db.is_some_and(|g| g >= -CUT_DB);
        if reading.learned || !quiet || !uncut {
            return;
        }
        reading.learned = true;
        let levels = reading.levels.clone();
        self.step_baselines(&levels);
    }

    /// Take the gain of the moment as the normal one: every baseline moves by `gain_db`, so the
    /// gain reads 0 dB now. What the floor does when it accepts the environment.
    pub fn rebase(&mut self, gain_db: f64) {
        for b in &mut self.baseline {
            *b += gain_db;
        }
        for r in &mut self.readings {
            r.gain_db = r.gain_db.map(|g| g - gain_db);
        }
        for g in &mut self.raw {
            *g -= gain_db;
        }
    }

    /// This station transmitted: the audio gathered before it is not averaged with what comes
    /// after. The baselines and the readings stay.
    pub fn skip(&mut self) {
        self.pending.clear();
        self.recent.clear();
        self.raw.clear();
    }

    /// The newest gain reading, dB.
    #[must_use]
    pub fn latest(&self) -> Option<f64> {
        self.readings.back().and_then(|r| r.gain_db)
    }

    /// The gain reading for audio ending at `t`, dB: the latest that ended by then, give or
    /// take half a block; the earliest kept when none did.
    #[must_use]
    pub fn at(&self, t: f64) -> Option<f64> {
        self.index_at(t).and_then(|i| self.readings[i].gain_db)
    }

    fn index_at(&self, t: f64) -> Option<usize> {
        let slack = 0.5 * self.block as f64 / self.rate;
        self.readings
            .iter()
            .rposition(|r| r.end <= t + slack)
            .or_else(|| (!self.readings.is_empty()).then_some(0))
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

/// The `q` quantile of some values, or `None` of none.
fn quantile(values: impl Iterator<Item = f64>, q: f64) -> Option<f64> {
    let mut sorted: Vec<f64> = values.collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    Some(sorted[((sorted.len() as f64 * q) as usize).min(sorted.len() - 1)])
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
            (
                "ramp up",
                160,
                Box::new(move |i| quiet * 10f64.powf((i as f64 / 160.0) * 12.0 / 20.0)),
            ),
            // holding at +12 dB
            (
                "hold",
                40,
                Box::new(move |_| quiet * 10f64.powf(12.0 / 20.0)),
            ),
            // ramping back down over 4 s: must clear after the hangover
            (
                "ramp down",
                160,
                Box::new(move |i| quiet * 10f64.powf((1.0 - i as f64 / 160.0) * 12.0 / 20.0)),
            ),
            ("noise again", 200, Box::new(move |_| quiet)),
        ];
        let log = scripted(&mut detector, &stages, 7);
        let stage_of = |t: f64| -> &str {
            let b = (t / 0.025).round() as usize;
            if b <= 320 {
                "noise"
            } else if b <= 480 {
                "ramp up"
            } else if b <= 520 {
                "hold"
            } else if b <= 680 {
                "ramp down"
            } else {
                "noise again"
            }
        };
        for (stage, at, busy, excess) in &log {
            eprintln!(
                "BUSY {} at {at:.2}s in '{stage}' | excess {excess:+.1} dB | threshold 6.0",
                if *busy { "OFF -> ON " } else { "ON  -> OFF" }
            );
        }
        assert!(
            !log.iter()
                .any(|(s, t, busy, _)| *busy && s == "noise" && *t > 2.5),
            "noise alone lit the indicator: {log:?}"
        );
        let first_on = log
            .iter()
            .find(|(_, _, busy, _)| *busy)
            .expect("the ramp must light it");
        assert_eq!(
            stage_of(first_on.1),
            "ramp up",
            "lit outside the ramp: {log:?}"
        );
        assert!(
            first_on.3 >= 6.0 - 0.6,
            "lit at only {:+.1} dB over the floor, below the 6 dB threshold",
            first_on.3
        );
        assert!(
            log.iter()
                .any(|(s, _, busy, _)| !*busy && (s == "ramp down" || s == "noise again")),
            "never cleared after the signal fell: {log:?}"
        );
        assert!(
            !detector.busy(0.025 * 880.0),
            "still busy on noise at the end"
        );
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
        let noise_lit: Vec<_> = log
            .iter()
            .filter(|(s, _, b, _)| *b && s != "loud again")
            .collect();
        assert!(
            noise_lit.is_empty(),
            "stationary noise lit the indicator: {noise_lit:?}"
        );
        assert!(
            !detector.busy(0.025 * 960.0 + 1.0),
            "still busy long after the floor caught up"
        );
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
        trace
            .iter()
            .rev()
            .find(|r| r.0 == stage)
            .expect("stage present")
            .3
    }

    #[test]
    fn case_1_stationary_noise_never_reads_busy() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![("noise", secs(90.0), Box::new(|_| 0.01))];
        let trace = traced(&mut detector, &stages, 3);
        let after_settle: Vec<_> = trace.iter().filter(|r| r.1 > 3.0).collect();
        let busy = after_settle.iter().filter(|r| r.4).count();
        assert_eq!(
            busy,
            0,
            "stationary noise lit the indicator {busy} blocks of {}",
            after_settle.len()
        );
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
        assert!(
            !trace.iter().any(|r| r.4),
            "a single spike lit the indicator"
        );
        let before = floor_at_end(&trace, "noise");
        let after = floor_at_end(&trace, "noise after");
        assert!(
            (before - after).abs() < 0.5,
            "the spike moved the floor {before:.1} -> {after:.1}"
        );
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
        assert!(
            (before - after).abs() < 1.0,
            "the impulses moved the floor {before:.1} -> {after:.1}"
        );
    }

    #[test]
    fn case_4_a_signal_rising_through_the_threshold_activates_busy() {
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(10.0), Box::new(|_| 0.01)),
            // +15 dB over 6 s
            (
                "ramp",
                secs(6.0),
                Box::new(|i| 0.01 * 10f64.powf(i as f64 / secs(6.0) as f64 * 15.0 / 20.0)),
            ),
            (
                "hold",
                secs(4.0),
                Box::new(|_| 0.01 * 10f64.powf(15.0 / 20.0)),
            ),
        ];
        let trace = traced(&mut detector, &stages, 13);
        let first = trace.iter().find(|r| r.4).expect("the ramp lights it");
        assert_eq!(first.0, "ramp");
        assert!(
            first.2 - first.3 >= 6.0 - 0.3,
            "lit at only {:+.1} dB over the floor",
            first.2 - first.3
        );
        assert!(
            busy_share(&trace, "hold") > 0.99,
            "not held busy through the hold"
        );
    }

    #[test]
    fn case_5_a_continuous_strong_signal_stays_busy_and_the_floor_stays_put() {
        // 45 s of a +12 dB signal: busy throughout, and the floor may not climb toward it
        let mut detector = BusyDetector::new(BusyConfig::default());
        let stages: Vec<Stage<'static>> = vec![
            ("noise", secs(15.0), Box::new(|_| 0.01)),
            (
                "signal",
                secs(45.0),
                Box::new(|_| 0.01 * 10f64.powf(12.0 / 20.0)),
            ),
        ];
        let trace = traced(&mut detector, &stages, 17);
        let floor_before = floor_at_end(&trace, "noise");
        let signal_blocks: Vec<_> = trace
            .iter()
            .filter(|r| r.0 == "signal" && r.1 > 15.5)
            .collect();
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
            (
                "ft8",
                secs(12.64),
                Box::new(|_| 0.01 * 10f64.powf(10.0 / 20.0)),
            ),
            ("gap", secs(2.36), Box::new(|_| 0.01)),
        ];
        let trace = traced(&mut detector, &stages, 19);
        let ft8: Vec<_> = trace
            .iter()
            .filter(|r| r.0 == "ft8" && r.1 > 15.2)
            .collect();
        let busy = ft8.iter().filter(|r| r.4).count();
        assert_eq!(
            busy,
            ft8.len(),
            "busy cleared {} blocks into an FT8 period",
            ft8.len() - busy
        );
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
            (
                "signal",
                secs(20.0),
                Box::new(|_| 0.01 * 10f64.powf(12.0 / 20.0)),
            ),
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
        assert!(
            after.iter().skip(cleared).all(|r| !r.4),
            "busy came back on noise"
        );
        // and the floor is where it was before the signal
        let before = floor_at_end(&trace, "noise");
        let recovered = floor_at_end(&trace, "after");
        assert!(
            (before - recovered).abs() < 1.0,
            "floor {before:.1} -> {recovered:.1}"
        );
    }

    #[test]
    fn case_8_repeated_ft8_cycles_do_not_ratchet_the_floor() {
        // ten 15 s cycles of 12.64 s on, 2.36 s off, at +10 dB: the floor must stay at the
        // noise the whole way, and every period must be busy end to end
        let mut detector = BusyDetector::new(BusyConfig::default());
        let mut stages: Vec<Stage<'static>> = vec![("noise", secs(15.0), Box::new(|_| 0.01))];
        for _ in 0..10 {
            stages.push((
                "ft8",
                secs(12.64),
                Box::new(|_| 0.01 * 10f64.powf(10.0 / 20.0)),
            ));
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
        assert!(
            share > 0.98,
            "busy only {:.0}% of the FT8 periods",
            share * 100.0
        );
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
    fn tone_over_noise(
        n: usize,
        sigma: f64,
        db: f64,
        offset_hz: f64,
        seed: u64,
        phase0: f64,
    ) -> Vec<Complex> {
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
        assert!(
            detector.busy(now),
            "a narrowband signal under the level margin is busy by shape"
        );
        assert!(matches!(detector.reason(), Some(BusyReason::Shape { .. })));
        assert!(
            (detector.floor_db - floor_before).abs() < 1.0,
            "and the floor did not learn it: {floor_before:.1} -> {:.1}",
            detector.floor_db
        );
    }

    #[test]
    fn a_faint_spur_far_below_the_floor_is_not_busy_though_it_is_peaked() {
        // A local birdie or a receiver spur sits well below the band noise, but on an
        // otherwise-quiet passband its peak stands tens of dB over the median, so the shape
        // path called the channel busy with nothing on the air — a station's own spur held
        // the indicator on whatever the dial (a real recording read 50-67 dB peaked at
        // -52 dBFS). A real signal's peak stands at or above the noise floor even when an
        // AGC has held it down; a spur's is far below it, and that is what the gate weighs.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let fs = detector.config().fs;
        let block = (detector.config().block_s * fs) as usize;
        let mut now = 0.0;
        // learn the band-noise floor from a real, audible band
        for i in 0..secs(10.0) {
            now += 0.025;
            detector.push(&noise(block, 0.1, 40 + i as u64), now);
        }
        assert!(
            detector.settled() && !detector.busy(now),
            "quiet band noise is clear"
        );
        let floor = detector.floor_db;
        // then a near-silent passband with a faint tone: strongly peaked against the quiet
        // median, but its absolute level is ~15 dB below the floor just learned
        let mut phase = 0.0;
        for i in 0..secs(3.0) {
            now += 0.025;
            detector.push(
                &tone_over_noise(block, 0.003, 15.0, 60.0, 900 + i as u64, phase),
                now,
            );
            phase += 2.0 * std::f64::consts::PI * 60.0 * block as f64 / fs;
        }
        assert!(
            detector.shape_db >= detector.config().shape_db,
            "the spur really is narrowband/peaked against the median: {:.1} dB",
            detector.shape_db
        );
        assert!(
            detector.level_db < floor - SHAPE_PEAK_FLOOR_MARGIN_DB,
            "the spur is meant to sit well below the floor: level {:.1}, floor {floor:.1}",
            detector.level_db
        );
        assert!(
            !detector.busy(now),
            "a faint spur below the floor was called busy on shape alone"
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
                d.push(
                    &tone_over_noise(block, 0.01, db, 60.0, seed + i as u64, phase),
                    *now,
                );
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
    fn a_signal_is_called_busy_within_the_attack_time_wherever_it_starts() {
        // `attack_s` is how long the station waits past a window of audio before it takes
        // that window for noise: a signal that could outlast it unseen would put the start
        // of a burst into the receiver's noise profile, and a tone frame's 400 Hz there
        // reads as a narrow filter. So it has to bound both paths from every phase of
        // the detector's blocks and shape windows: a flat signal 12 dB up, caught by level
        // as an OFDM burst is, and a tone under the level margin, caught by shape.
        let mut settled = BusyDetector::new(BusyConfig::default());
        let fs = settled.config().fs;
        let block = (settled.config().block_s * fs) as usize;
        let quiet_until = feed(&mut settled, 10.0, 0.01, 40, 0.0);
        assert!(settled.settled() && !settled.busy(quiet_until));
        let attack = settled.attack_s();
        for lead in (0..SHAPE_BLOCKS * block).step_by(block / 2) {
            for flat in [true, false] {
                let mut detector = settled.clone();
                // noise up to where the signal starts, `lead` samples into the next window
                let mut now = quiet_until + lead as f64 / fs;
                detector.push(&noise(lead, 0.01, 7_000 + lead as u64), now);
                let start = now;
                let mut phase = 0.0;
                let mut n = 0u64;
                while !detector.busy(now) {
                    assert!(
                        now - start < 2.0,
                        "never called busy (flat {flat}, lead {lead})"
                    );
                    let samples = if flat {
                        noise(block, 0.01 * 10f64.powf(12.0 / 20.0), 8_000 + n)
                    } else {
                        tone_over_noise(block, 0.01, 4.0, 60.0, 8_000 + n, phase)
                    };
                    phase += 2.0 * std::f64::consts::PI * 60.0 * block as f64 / fs;
                    n += 1;
                    now += block as f64 / fs;
                    detector.push(&samples, now);
                }
                assert!(
                    now - start <= attack + 1e-9,
                    "a {} signal starting {lead} samples into a window was called busy \
                     {:.3} s after it began, past the {attack:.3} s the detector promises",
                    if flat { "flat" } else { "narrowband" },
                    now - start
                );
            }
        }
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
        let feed_tone = |d: &mut BusyDetector,
                         now: &mut f64,
                         s: f64,
                         db: f64,
                         hz: f64,
                         seed: &mut u64|
         -> f64 {
            let mut phase = 0.0;
            let mut busy_blocks = 0usize;
            let n = secs(s);
            for _ in 0..n {
                *now += 0.025;
                *seed += 1;
                d.push(
                    &tone_over_noise(block, 0.01, db, hz, 5000 + *seed, phase),
                    *now,
                );
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
        assert!(
            strong_share / 3.0 > 0.9,
            "the strong station busy only {:.0}%",
            strong_share / 3.0 * 100.0
        );
        assert!(
            weak_share / 3.0 > 0.9,
            "the weak station busy only {:.0}%",
            weak_share / 3.0 * 100.0
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
    fn a_signal_heard_before_our_own_transmission_does_not_make_the_channel_busy_after_it() {
        // A peer's signal ended just before this station keyed: the attack's votes from it
        // were still in the window when the station's own transmission went to `skip`, and
        // the first quiet block after the deafness found eight of sixteen over the threshold
        // and turned the channel busy with nothing on the air. Likewise a narrowband signal's
        // peaked shape window.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 8.0, 0.01, 6, 0.0);
        assert!(!detector.busy(now));
        // the peer, 12 dB over the noise, for a second — busy, as it should be
        let now = feed(&mut detector, 1.0, 0.04, 77, now);
        assert!(
            detector.busy(now),
            "a signal 12 dB over the noise read clear"
        );
        // this station transmits for three seconds, longer than any hang
        let sidetone = noise((3.0 * detector.config().fs) as usize, 1.0, 9);
        detector.skip(&sidetone);
        let now = now + 3.0;
        assert!(
            !detector.busy(now),
            "the peer's busy outlasted the transmission"
        );
        // the quiet channel after it
        let block = detector.config().block_s;
        let per_block = (block * detector.config().fs) as usize;
        let mut t = now;
        for index in 0..8 {
            t += block;
            detector.push(&noise(per_block, 0.01, 500 + index), t);
            assert!(
                !detector.busy(t),
                "block {index} after the transmission read busy ({:?}) on votes from before it",
                detector.reason()
            );
        }
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
    fn a_decoded_frame_explains_the_energy_that_was_its_own() {
        // A frame loud enough to make the channel busy by its energy is decoded as it ends.
        // The quiet after it is the frame's hangover, not energy of something unknown: an
        // answer that waits only for unexplained energy (a DISC_ACK) must not wait it out.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let now = feed(&mut detector, 6.0, 0.01, 1, 0.0);
        let now = feed(&mut detector, 0.5, 0.05, 2, now);
        assert!(matches!(detector.reason(), Some(BusyReason::Level { .. })));
        detector.mark_frame(now, 2.0);
        let now = feed(&mut detector, 0.1, 0.01, 3, now);
        assert!(
            detector.busy(now),
            "the frame holds the channel for its hold"
        );
        assert!(
            matches!(detector.reason(), Some(BusyReason::Frame { .. })),
            "the frame's own blocks named it energy again: {:?}",
            detector.reason()
        );
    }

    /// A session as the busy detector of a station in it hears it: the other station's
    /// frames, a weak and steady signal 4.5 dB over the passband's noise (a 500 Hz signal at
    /// about 0 dB in 3 kHz), 5.36 s each with 0.7 s of quiet between, each decoded at its end
    /// and — with `explained` — announced a third of a second in, as the receiver does. The
    /// floor at the end, in dB over the noise's own.
    fn floor_after_a_session(explained: bool) -> f64 {
        let config = BusyConfig::default();
        let mut detector = BusyDetector::new(config);
        let sigma = 0.01;
        let mut now = feed(&mut detector, 8.0, sigma, 1, 0.0);
        let noise_floor = detector.floor_db;
        let signal = sigma * (1.0 + 10f64.powf(4.5 / 10.0)).sqrt();
        for frame in 0..30u64 {
            let start = now;
            let end = start + 5.36;
            let announced = start + 0.32;
            let mut told = false;
            while now < end {
                if explained && !told && now >= announced {
                    detector.explain(start, end);
                    told = true;
                }
                now = feed(
                    &mut detector,
                    config.block_s,
                    signal,
                    100 + frame * 1000 + (now * 40.0) as u64,
                    now,
                );
            }
            if explained {
                detector.explain(start, end);
            }
            detector.mark_frame(now + 0.15, 40.0);
            now = feed(&mut detector, 0.7, sigma, 7 + frame, now);
        }
        detector.floor_db - noise_floor
    }

    #[test]
    fn a_weak_stations_frames_do_not_become_the_noise_floor() {
        // ADR-0076: on WC4Y's Test of 2026-10-17 both stations' floors became the other's
        // signal (KK4ODA-1 −18 dBFS against a real −28), and with them the threshold and the
        // panel's readings. Frames the receiver found are no noise, and the quiet after a
        // decoded frame is, its hold notwithstanding
        let without = floor_after_a_session(false);
        let with = floor_after_a_session(true);
        assert!(
            without > 3.0,
            "unexplained, the frames ratchet the floor up: {without:.1} dB — the test does not tell"
        );
        assert!(
            with.abs() < 1.0,
            "the floor learned the frames: {with:.1} dB over the noise"
        );
    }

    /// The radio's audio as an AGC delivers it, fed to a detector the way `Station::capture`
    /// does: 20 ms at a time, the whole passband to [`BusyDetector::hear_passband`] and the
    /// channel's baseband, from the same front end, to [`BusyDetector::push`].
    ///
    /// The audio is white noise (the band: −29 dBFS in a 500 Hz channel, where the author's
    /// station reads −27) and, from `on` to `off`, a carrier at `carrier_hz` 30 dB over the noise
    /// in 3 kHz. The AGC is the harness's model of an FTDX10 on AUTO in DATA (beta.17's
    /// measurement): it holds the gain to keep the carrier's peaks at 12 dB over the noise —
    /// 21 dB down here, as the author's −27 went to −47 — holds 100 ms after the carrier goes,
    /// and comes back at 50 dB/s.
    /// Returns, every 20 ms: the time, whether the channel read busy, and the floor.
    fn agc_held_by_a_carrier(
        detector: &mut BusyDetector,
        seconds: f64,
        carrier_hz: f64,
        on: f64,
        off: f64,
    ) -> Vec<(f64, bool, f64)> {
        let params = aether_phy::waveform::NARROW_500;
        let rate = params.audio_rate as f64;
        let mut front = aether_phy::AudioToBaseband::new(params);
        let sigma = 0.18;
        // white noise at 48 kHz holds a sixteenth of its power in 3 kHz
        let noise_3k = sigma * (3000.0 / (rate / 2.0)).sqrt();
        let amplitude = noise_3k * 10f64.powf(30.0 / 20.0) * std::f64::consts::SQRT_2;
        let threshold = noise_3k * 10f64.powf(12.0 / 20.0);
        let (mut gain_db, mut held) = (0.0f64, 0.0f64);
        let chunk = (0.02 * rate) as usize;
        let mut out = Vec::new();
        let mut n = 0usize;
        let mut seed = 1u64;
        while (n as f64) / rate < seconds {
            let samples = noise(chunk, sigma, seed);
            // odd seeds: the generator's state is the seed with its low bit set
            seed += 2;
            let mut audio = Vec::with_capacity(chunk);
            for (re, _) in samples {
                let t = n as f64 / rate;
                let carrier = if (on..off).contains(&t) {
                    amplitude * (2.0 * std::f64::consts::PI * carrier_hz * t).sin()
                } else {
                    0.0
                };
                // the AGC's limit: what keeps the carrier's peak at the threshold
                let limit = if (on..off).contains(&t) {
                    20.0 * (threshold / amplitude).log10()
                } else {
                    0.0
                };
                if limit < gain_db {
                    (gain_db, held) = (limit, 0.1);
                } else if held > 0.0 {
                    held -= 1.0 / rate;
                } else {
                    gain_db = (gain_db + 50.0 / rate).min(0.0).min(limit);
                }
                audio.push(((re + carrier) * 10f64.powf(gain_db / 20.0)) as f32);
                n += 1;
            }
            let now = n as f64 / rate;
            let baseband = front.process(&audio);
            detector.hear_passband(&audio, rate, now);
            let busy = detector.push(&baseband, now);
            out.push((now, busy, detector.floor_db));
        }
        out
    }

    #[test]
    fn a_gain_held_down_by_a_signal_outside_the_channel_reads_clear_when_it_lets_go() {
        // The author's FTDX10 on AGC AUTO, 2026-10-17: a strong signal held the gain down, the
        // floor fell from −27 to −47 dBFS on the depressed noise, and when the AGC let go the
        // true noise read busy by level for 71 s — the floor window and its hold (ADR-0078).
        // The carrier here is at 700 Hz, outside the 500 Hz channel, for 20 s.
        let mut detector = BusyDetector::new(BusyConfig::default());
        let trace = agc_held_by_a_carrier(&mut detector, 90.0, 700.0, 20.0, 40.0);
        let before = trace
            .iter()
            .rev()
            .find(|r| r.0 < 19.0)
            .expect("the minute before")
            .2;
        assert!(
            !trace.iter().any(|r| r.0 > 5.0 && r.0 < 40.0 && r.1),
            "busy with nothing in the channel, before the carrier or under it"
        );
        let still_busy: Vec<_> = trace.iter().filter(|r| r.0 >= 43.0 && r.1).collect();
        assert!(
            still_busy.is_empty(),
            "the channel read busy {:.1} s after the carrier had gone (last at {:.1} s): the \
             floor had learned the noise the AGC held down",
            still_busy.len() as f64 * 0.02,
            still_busy.last().map_or(0.0, |r| r.0)
        );
        let after = trace.iter().find(|r| r.0 >= 43.0).expect("after").2;
        assert!(
            (after - before).abs() < 3.0,
            "the floor is back at the noise: {before:.1} before, {after:.1} after"
        );
    }

    /// White noise at 48 kHz through `params`' front end and an AGC (beta.17's: a step down, a
    /// 100 ms hold, 50 dB/s back), 20 ms a call: `scene(t)` gives what is added to the noise at
    /// `t` (relative to the noise's RMS) and the gain the AGC would cut to, dB (0 for none).
    /// Returns (time, busy) after every call.
    fn agc_scene(
        params: aether_phy::waveform::WaveformParams,
        seconds: f64,
        mut scene: impl FnMut(f64, f64) -> (f64, f64),
    ) -> Vec<(f64, bool)> {
        let rate = params.audio_rate as f64;
        let mut detector = BusyDetector::new(BusyConfig {
            fs: params.fs_baseband,
            passband_hz: params.occupied_bandwidth_hz(),
            centre_hz: params.centre_hz,
            ..BusyConfig::default()
        });
        let mut front = aether_phy::AudioToBaseband::new(params);
        let sigma = 0.18;
        let (mut gain_db, mut held) = (0.0f64, 0.0f64);
        let chunk = (0.02 * rate) as usize;
        let mut out = Vec::new();
        let mut n = 0usize;
        let mut seed = 1u64;
        while (n as f64) / rate < seconds {
            let noise = noise(chunk, sigma, seed);
            // odd seeds: the generator's state is the seed with its low bit set
            seed += 2;
            let mut audio = Vec::with_capacity(chunk);
            for (re, im) in noise {
                let t = n as f64 / rate;
                let (added, limit) = scene(t, im / sigma);
                if limit < gain_db {
                    (gain_db, held) = (limit, 0.1);
                } else if held > 0.0 {
                    held -= 1.0 / rate;
                } else {
                    gain_db = (gain_db + 50.0 / rate).min(0.0).min(limit);
                }
                audio.push(((re + added * sigma) * 10f64.powf(gain_db / 20.0)) as f32);
                n += 1;
            }
            let now = n as f64 / rate;
            let baseband = front.process(&audio);
            detector.hear_passband(&audio, rate, now);
            out.push((now, detector.push(&baseband, now)));
        }
        out
    }

    /// The share of the trace's blocks in `from..until` that read busy.
    fn busy_in(trace: &[(f64, bool)], from: f64, until: f64) -> f64 {
        let blocks: Vec<bool> = trace
            .iter()
            .filter(|r| r.0 >= from && r.0 < until)
            .map(|r| r.1)
            .collect();
        blocks.iter().filter(|&&b| b).count() as f64 / blocks.len().max(1) as f64
    }

    #[test]
    fn a_wide_signal_in_a_2300_hz_channel_under_a_cut_still_reads_busy() {
        // A 2300 Hz channel fills the radio's passband, so the gain is read from the channel's own
        // bins: a wide signal in it lifts the reading exactly as the gain coming back would. A
        // signal the audio never carries (outside the DSP filter) holds the AGC 25 dB down from
        // 20 s to 60 s; a wide signal 9 dB over the noise fills the channel from 35 s to 45 s. It
        // must read busy, and the channel clear again within 3 s of its end and of the gain's
        // return (ADR-0078 §2.3).
        let trace = agc_scene(aether_phy::waveform::WIDE_2300, 75.0, |t, other| {
            let wide = if (35.0..45.0).contains(&t) {
                other * 10f64.powf(9.0 / 20.0)
            } else {
                0.0
            };
            let limit = if (20.0..60.0).contains(&t) {
                -25.0
            } else {
                0.0
            };
            (wide, limit)
        });
        let caught = busy_in(&trace, 35.5, 45.0);
        assert!(
            caught > 0.9,
            "the wide signal under the cut read busy {caught:.2} of its length"
        );
        for (from, until, what) in [
            (5.0, 35.0, "before the signal"),
            (48.0, 60.0, "under the cut after it"),
            (63.0, 75.0, "after the gain came back"),
        ] {
            let busy = busy_in(&trace, from, until);
            assert!(busy == 0.0, "busy {busy:.2} of the time {what}");
        }
    }

    #[test]
    fn a_crash_in_the_first_moment_leaves_no_false_cut() {
        // A static crash 30 dB over the noise in the first tenth of a second, then quiet noise
        // with no AGC. Each bin's level on quiet is set from the whole warm-up, so the crash
        // moves none of them; set from the first reading, it left them 9 dB high, the gain read
        // −9 dB with nothing cut, and the channel read busy for a minute (ADR-0078 §3.3).
        let trace = agc_scene(aether_phy::waveform::NARROW_500, 40.0, |t, other| {
            let crash = if (0.03..0.09).contains(&t) {
                other * 10f64.powf(30.0 / 20.0)
            } else {
                0.0
            };
            (crash, 0.0)
        });
        let busy = busy_in(&trace, 3.0, 40.0);
        assert!(
            busy == 0.0,
            "busy {busy:.2} of the time after a crash at the start"
        );
    }

    /// The gain reading after four seconds of noise to learn from, then one second of the same
    /// noise through a gain of `gain_db` with a tone of `tone_db` over the noise at `tone_hz`.
    fn reading_after(gain_db: f64, tone_hz: f64, tone_db: f64, occupied: (f64, f64)) -> f64 {
        let rate = 48_000.0;
        let heard = (occupied.0 - CHANNEL_GUARD_HZ, occupied.1 + CHANNEL_GUARD_HZ);
        let mut reference = PassbandReference::new(rate, 0.025, occupied, heard);
        let sigma = 0.05;
        let amplitude = sigma * 10f64.powf(tone_db / 20.0) * std::f64::consts::SQRT_2;
        let mut n = 0usize;
        let mut readings = Vec::new();
        for chunk in 0..250u64 {
            let learning = chunk < 200;
            let gain = if learning {
                1.0
            } else {
                10f64.powf(gain_db / 20.0)
            };
            // odd seeds: the generator's state is the seed with its low bit set
            let audio: Vec<f32> = noise(960, sigma, 2 * chunk + 61)
                .into_iter()
                .map(|(re, _)| {
                    let t = n as f64 / rate;
                    n += 1;
                    let tone = if tone_hz > 0.0 && !learning {
                        amplitude * (2.0 * std::f64::consts::PI * tone_hz * t).sin()
                    } else {
                        0.0
                    };
                    ((re + tone) * gain) as f32
                })
                .collect();
            let now = n as f64 / rate;
            reference.push(&audio, now);
            reference.learn(now, learning);
            if chunk >= 225 {
                readings.push(reference.latest().expect("a reading"));
            }
        }
        median(readings.into_iter()).expect("readings")
    }

    #[test]
    fn the_gain_reading_follows_the_gain_and_not_a_signal_on_top_of_it() {
        // a gain cut moves it by the cut, at once; a strong narrowband signal outside the
        // channel, or a signal inside it when there is room to leave the channel out, does not
        // move it at all (ADR-0078)
        let narrow = (1250.0, 1750.0);
        let quiet = reading_after(0.0, 0.0, 0.0, narrow);
        assert!(quiet.abs() < 1.0, "noise alone read {quiet:.1} dB");
        let cut = reading_after(-20.0, 0.0, 0.0, narrow);
        assert!((cut + 20.0).abs() < 1.0, "a 20 dB cut read as {cut:.1} dB");
        for (hz, place) in [(700.0, "at 700 Hz"), (1500.0, "in the channel")] {
            let carrier = reading_after(0.0, hz, 30.0, narrow);
            assert!(
                (carrier - quiet).abs() < 1.0,
                "a carrier {place} moved it {:.1} dB",
                carrier - quiet
            );
            let both = reading_after(-20.0, hz, 30.0, narrow);
            assert!(
                (both + 20.0).abs() < 1.0,
                "a 20 dB cut with a carrier {place} read as {both:.1} dB"
            );
        }
        // a 2300 Hz channel fills the passband: the reading includes it, and still reads a cut
        let heard = |(low, high): (f64, f64)| (low - CHANNEL_GUARD_HZ, high + CHANNEL_GUARD_HZ);
        let wide = (360.0, 2640.0);
        assert!(PassbandReference::new(48_000.0, 0.025, wide, heard(wide)).includes_channel());
        assert!(!PassbandReference::new(48_000.0, 0.025, narrow, heard(narrow)).includes_channel());
        let cut = reading_after(-20.0, 0.0, 0.0, wide);
        assert!(
            (cut + 20.0).abs() < 1.0,
            "a 20 dB cut at 2300 Hz read as {cut:.1} dB"
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
