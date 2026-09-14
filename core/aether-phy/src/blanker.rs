//! Impulsive-noise blanker for the receiver front end (roadmap P2-5).
//!
//! HF is full of impulsive noise — ignition, switching supplies, plasma televisions, static
//! crashes. It is the one impairment OFDM handles *worse* than a single-carrier waveform: the
//! FFT spreads a single hot sample across all 57 carriers of the symbol it lands in, so one
//! microsecond of interference damages a whole 31 ms symbol. The defence is to remove the
//! impulse in the time domain, before the FFT ever sees it.
//!
//! Two things have to be true for that to work, and both dictate where this sits in the chain:
//!
//! * **Blank before band-limiting.** The receive filter smears an impulse into a long ringing
//!   tail; once that has happened there is no longer a small set of hot samples to remove.
//!   This runs on the raw stream, ahead of the band-limiting filter.
//! * **Judge against a robust envelope.** The threshold has to come from a statistic the
//!   impulses themselves cannot drag upward, or a strong burst raises the bar until it no
//!   longer trips it. A moving *median* of the envelope is used, not a moving mean.
//!
//! Blanking is not free: zeroing a sample removes signal along with noise, and blanking too
//! eagerly is its own impairment. The default threshold is set where clean Gaussian noise is
//! essentially never blanked, so on a quiet channel the blanker does nothing at all.
//!
//! What blanking leaves behind is handled downstream: the receiver estimates noise variance
//! per OFDM symbol, so a symbol that still took damage gets its LLRs scaled down and
//! effectively becomes an erasure the decoder can work around.

use crate::Complex;

/// `median(|x|) / sigma` for a complex Gaussian with per-component variance `sigma²`, which
/// is `sqrt(2 ln 2)`. Converts a robust median envelope into the RMS the threshold is set
/// from.
const RAYLEIGH_MEDIAN: f64 = 1.177_410_022_515_474_7;

/// What to do with a sample that exceeds the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlankMode {
    /// Replace it with zero. The decoder handles the hole better than it handles the impulse.
    #[default]
    Blank,
    /// Keep its phase and pull its magnitude down to the threshold.
    Clip,
}

/// A blanked block, and where it was blanked.
#[derive(Debug, Clone)]
pub struct BlankerResult {
    /// The samples, with impulses removed.
    pub samples: Vec<Complex>,
    /// True where a sample was removed.
    pub blanked: Vec<bool>,
}

impl BlankerResult {
    /// Fraction of samples removed.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        if self.blanked.is_empty() {
            return 0.0;
        }
        self.blanked.iter().filter(|&&b| b).count() as f64 / self.blanked.len() as f64
    }
}

/// Median-referenced impulse blanker.
///
/// `threshold_sigma` is in units of the local RMS envelope. For complex Gaussian noise the
/// envelope is Rayleigh, so the probability a *clean* sample is blanked is `exp(-k²)`:
/// 1.2e-4 at k = 3, 4.8e-6 at k = 3.5, 1.1e-7 at k = 4. The default of 3.5 costs a quiet
/// channel essentially nothing while still catching bursts tens of dB above the noise.
#[derive(Debug, Clone)]
pub struct NoiseBlanker {
    threshold_sigma: f64,
    window: usize,
    segment_span: usize,
    mode: BlankMode,
}

impl Default for NoiseBlanker {
    fn default() -> Self {
        Self::new(3.5, 101, 9, BlankMode::Blank)
    }
}

impl NoiseBlanker {
    /// Build a blanker.
    ///
    /// # Panics
    /// If `threshold_sigma` is not positive.
    #[must_use]
    pub fn new(threshold_sigma: f64, window: usize, segment_span: usize, mode: BlankMode) -> Self {
        assert!(threshold_sigma > 0.0, "threshold must be positive");
        Self {
            threshold_sigma,
            window: window.max(3),
            segment_span: segment_span.max(1) | 1, // odd, so the window is centred
            mode,
        }
    }

    /// Samples per segment of the envelope estimate.
    #[must_use]
    pub fn window(&self) -> usize {
        self.window
    }

    /// Longest burst the reference can survive: it has to stay a minority of the segments the
    /// running median looks at. About 57 ms at 8 kHz with the defaults.
    #[must_use]
    pub fn robust_span_samples(&self) -> usize {
        self.window * self.segment_span / 2
    }

    /// Local RMS envelope, estimated as a *median of segment medians*.
    ///
    /// A single moving median is robust only to impulses that are a minority inside its own
    /// window. A static crash lasting tens of milliseconds is longer than any window short
    /// enough to track fading, and inside such a burst the local median *is* the burst — the
    /// threshold rises with it and the blanker sails straight past the thing it exists to
    /// catch. Taking the median of per-segment medians pushes the robustness out to
    /// [`robust_span_samples`](Self::robust_span_samples) while keeping the estimator cheap
    /// and still fast enough to follow fading, which moves at 0.1–1 Hz.
    ///
    /// # Panics
    /// Never for a non-empty input: the branch that indexes the smoothed levels is only
    /// reached when there are at least two segments to smooth.
    #[must_use]
    pub fn envelope_rms(&self, samples: &[Complex]) -> Vec<f64> {
        let magnitude: Vec<f64> = samples.iter().map(|&(re, im)| re.hypot(im)).collect();
        let n = magnitude.len();
        if n == 0 {
            return Vec::new();
        }
        let window = self.window.min(n);
        let segments = n / window;
        let level: Vec<f64> = if segments < 2 {
            vec![median(&mut magnitude.clone()); n]
        } else {
            let per_segment: Vec<f64> = (0..segments)
                .map(|s| median(&mut magnitude[s * window..(s + 1) * window].to_vec()))
                .collect();
            let smoothed = median_filter(&per_segment, self.segment_span.min(segments));
            let mut level: Vec<f64> = Vec::with_capacity(n);
            for value in &smoothed {
                level.extend(std::iter::repeat_n(*value, window));
            }
            // the ragged tail keeps the last segment's level
            let last = *level.last().expect("segments >= 2");
            level.resize(n, last);
            level
        };
        level
            .into_iter()
            .map(|l| l / RAYLEIGH_MEDIAN * std::f64::consts::SQRT_2)
            .collect()
    }

    /// Remove impulses from a block.
    #[must_use]
    pub fn process(&self, samples: &[Complex]) -> BlankerResult {
        if samples.is_empty() {
            return BlankerResult {
                samples: Vec::new(),
                blanked: Vec::new(),
            };
        }
        let rms = self.envelope_rms(samples);
        let mut out = samples.to_vec();
        let mut blanked = vec![false; samples.len()];
        for (index, sample) in out.iter_mut().enumerate() {
            let threshold = self.threshold_sigma * rms[index];
            let magnitude = sample.0.hypot(sample.1);
            if threshold > 0.0 && magnitude > threshold {
                blanked[index] = true;
                *sample = match self.mode {
                    BlankMode::Blank => (0.0, 0.0),
                    BlankMode::Clip => {
                        let scale = threshold / magnitude;
                        (sample.0 * scale, sample.1 * scale)
                    }
                };
            }
        }
        BlankerResult {
            samples: out,
            blanked,
        }
    }
}

/// Median of a slice, by the convention `NumPy` uses: the mean of the two middle values when
/// the count is even. Sorts in place, so the caller passes a copy it does not need.
fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        f64::midpoint(values[mid - 1], values[mid])
    }
}

/// A one-dimensional median filter with edge values extended, matching
/// [`scipy.ndimage.median_filter`] with `mode="nearest"`.
///
/// [`scipy.ndimage.median_filter`]: https://docs.scipy.org/doc/scipy/reference/generated/scipy.ndimage.median_filter.html
fn median_filter(values: &[f64], size: usize) -> Vec<f64> {
    if values.is_empty() || size <= 1 {
        return values.to_vec();
    }
    // scipy centres an even-sized window with the extra sample on the left
    let origin = size / 2;
    let last = values.len() - 1;
    (0..values.len())
        .map(|i| {
            let mut window: Vec<f64> = (0..size)
                .map(|k| {
                    let index = (i + k).saturating_sub(origin).min(last);
                    values[index]
                })
                .collect();
            median(&mut window)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic complex Gaussian noise, by Box–Muller over a small generator, so a test
    /// failure is always reproducible.
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

    #[test]
    fn a_quiet_channel_is_left_alone() {
        // the whole point of the 3.5-sigma default: on clean noise the blanker does nothing
        let samples = noise(20_000, 1.0, 7);
        let result = NoiseBlanker::default().process(&samples);
        assert!(
            result.fraction() < 1e-3,
            "blanked {:.4} of a clean channel",
            result.fraction()
        );
    }

    #[test]
    fn a_single_impulse_is_removed() {
        let mut samples = noise(4000, 1.0, 11);
        samples[2000] = (400.0, 0.0);
        let result = NoiseBlanker::default().process(&samples);
        assert!(result.blanked[2000], "the impulse survived");
        assert_eq!(result.samples[2000], (0.0, 0.0));
    }

    #[test]
    fn a_sustained_burst_is_removed_too() {
        // This is why the reference is a median of segment medians rather than one moving
        // median: a burst longer than a single window *is* the local median, so a plain
        // moving median raises its own threshold and misses exactly the case that matters.
        let mut samples = noise(40_000, 1.0, 13);
        let burst = 20_000..20_300; // 300 samples, three times the 101-sample window
        for index in burst.clone() {
            samples[index] = (60.0, -40.0);
        }
        let result = NoiseBlanker::default().process(&samples);
        let caught = burst.clone().filter(|&i| result.blanked[i]).count();
        assert!(
            caught > burst.len() * 9 / 10,
            "caught only {caught} of {} burst samples",
            burst.len()
        );
    }

    #[test]
    fn a_burst_longer_than_the_robust_span_is_beyond_it() {
        // The limit is documented rather than hidden: past `robust_span_samples` the burst is
        // the majority of what the reference looks at, and no median can tell them apart.
        let blanker = NoiseBlanker::default();
        let span = blanker.robust_span_samples();
        assert_eq!(span, 101 * 9 / 2);
        let mut samples = noise(4 * span, 1.0, 17);
        for sample in &mut samples[span..3 * span] {
            *sample = (50.0, 0.0);
        }
        let result = blanker.process(&samples);
        let caught = (span..3 * span).filter(|&i| result.blanked[i]).count();
        assert!(
            caught < span, // less than half of it
            "a burst twice the robust span should defeat the reference, caught {caught}"
        );
    }

    #[test]
    fn clipping_keeps_the_phase() {
        let mut samples = noise(4000, 1.0, 19);
        samples[1000] = (300.0, 300.0);
        let blanker = NoiseBlanker::new(3.5, 101, 9, BlankMode::Clip);
        let result = blanker.process(&samples);
        let (re, im) = result.samples[1000];
        assert!(result.blanked[1000]);
        assert!((re - im).abs() < 1e-9, "the phase moved: ({re}, {im})");
        assert!(re.hypot(im) < 300.0f64.hypot(300.0), "nothing was removed");
    }

    #[test]
    fn a_streamed_signal_is_blanked_the_same_whatever_the_block_size() {
        // This is the property the streaming wrapper exists for. Without it the reference is
        // a statistic of whatever block the sound card happened to deliver, so the start of
        // every burst is judged against the silence in front of it and blanked, and how much
        // goes depends on where the buffer boundaries landed — measured at 20 dB of
        // signal-to-noise ratio on a clean channel.
        let mut samples = noise(30_000, 0.01, 23);
        // a strong signal that starts abruptly, as a burst on a quiet channel does
        for (index, sample) in samples.iter_mut().enumerate().take(20_000).skip(8_000) {
            let phase = 0.7 * index as f64;
            *sample = (phase.cos(), phase.sin());
        }
        samples[14_000] = (60.0, 0.0); // and one genuine impulse inside it

        let reference = {
            let mut blanker = StreamingBlanker::default();
            let mut out = blanker.process(&samples);
            out.extend(blanker.flush());
            out
        };
        assert_eq!(reference.len(), samples.len());

        for chunk in [251usize, 683, 1024, 4096] {
            let mut blanker = StreamingBlanker::default();
            let mut out = Vec::new();
            for block in samples.chunks(chunk) {
                out.extend(blanker.process(block));
            }
            out.extend(blanker.flush());
            assert_eq!(out.len(), samples.len(), "block size {chunk}: length");
            for (index, (got, want)) in out.iter().zip(&reference).enumerate() {
                assert!(
                    (got.0 - want.0).abs() < 1e-12 && (got.1 - want.1).abs() < 1e-12,
                    "block size {chunk} changed sample {index}"
                );
            }
        }
    }

    #[test]
    fn a_burst_that_starts_abruptly_is_not_mistaken_for_an_impulse() {
        let mut samples = noise(30_000, 0.01, 29);
        let signal = 8_000..20_000;
        for (index, sample) in samples
            .iter_mut()
            .enumerate()
            .take(signal.end)
            .skip(signal.start)
        {
            let phase = 0.7 * index as f64;
            *sample = (phase.cos(), phase.sin());
        }
        let mut blanker = StreamingBlanker::default();
        let mut out = Vec::new();
        for block in samples.chunks(683) {
            out.extend(blanker.process(block));
        }
        out.extend(blanker.flush());

        let removed = signal.clone().filter(|&i| out[i] == (0.0, 0.0)).count();
        assert!(
            removed < signal.len() / 200,
            "blanked {removed} of {} signal samples",
            signal.len()
        );
    }

    #[test]
    fn the_streaming_wrapper_holds_back_exactly_its_stated_latency() {
        let mut blanker = StreamingBlanker::default();
        let latency = blanker.latency_samples();
        let samples = noise(10_000, 0.01, 31);
        let emitted: usize = samples.chunks(1000).map(|b| blanker.process(b).len()).sum();
        assert_eq!(
            emitted,
            samples.len() - latency,
            "held back {} rather than {latency}",
            samples.len() - emitted
        );
        assert_eq!(blanker.flush().len(), latency);
    }

    #[test]
    fn an_empty_block_is_not_an_error() {
        let result = NoiseBlanker::default().process(&[]);
        assert!(result.samples.is_empty());
        assert!(result.fraction() == 0.0, "an empty block blanks nothing");
    }
}

/// A [`NoiseBlanker`] for a live stream.
///
/// Blanking a live stream is not the same job as blanking a buffer, and doing it block by
/// block is wrong in a way that is easy to miss: the reference is a *local* statistic, so a
/// block that is half silence and half signal has a reference taken from the silence, and the
/// blanker removes the start of every burst it hears. Worse, *which* samples it removes then
/// depends on where the sound card happened to put its buffer boundaries — a receiver whose
/// output depends on its buffer size cannot be measured at all. On a clean channel at the
/// daemon's own block size that cost a frame 20 dB of signal-to-noise ratio: the blanker
/// doing far more damage than the impulses it exists to remove.
///
/// The fix is to give the streaming path the same view the offline one has: a window centred
/// on each sample, with real signal on both sides of it. That means holding samples back
/// until their future has arrived, so this introduces a fixed latency of
/// [`latency_samples`](Self::latency_samples) — one robust span, about 57 ms at 8 kHz with the
/// default settings. Everything downstream sees a stream that is simply late by that much.
#[derive(Debug, Clone)]
pub struct StreamingBlanker {
    blanker: NoiseBlanker,
    /// Retained samples: history, then what is about to be emitted, then the lookahead.
    pending: Vec<Complex>,
    /// How far into `pending` has already been emitted.
    emitted: usize,
    span: usize,
    history: usize,
    /// Stream index of `pending[0]`, so the segment grid can be kept aligned to the stream.
    absolute: usize,
}

impl Default for StreamingBlanker {
    fn default() -> Self {
        Self::new(NoiseBlanker::default())
    }
}

impl StreamingBlanker {
    /// Wrap a blanker for streaming use.
    #[must_use]
    pub fn new(blanker: NoiseBlanker) -> Self {
        let span = blanker.robust_span_samples().max(1);
        Self {
            blanker,
            pending: Vec::new(),
            emitted: 0,
            span,
            // A centred window needs one span on each side, but the reference is a median of
            // *segment* medians and the segment grid is laid out from the start of whatever
            // buffer it is given: judged over a buffer barely wider than the window itself,
            // every segment is an edge segment. Carrying several spans of history makes the
            // statistics the same whatever size the sound card hands over — measured, that is
            // the difference between a clean frame and one 18 dB down at small block sizes.
            history: 4 * span,
            absolute: 0,
        }
    }

    /// How far behind its input the output runs, in samples.
    #[must_use]
    pub fn latency_samples(&self) -> usize {
        self.span
    }

    /// The blanker being applied.
    #[must_use]
    pub fn blanker(&self) -> &NoiseBlanker {
        &self.blanker
    }

    /// Feed a block; get back the samples whose windows are now complete.
    pub fn process(&mut self, block: &[Complex]) -> Vec<Complex> {
        self.pending.extend_from_slice(block);
        if self.pending.len() < self.emitted + 2 * self.span {
            return Vec::new(); // not enough future yet to judge anything new
        }
        let result = self.blanker.process(&self.pending);
        let emit_to = self.pending.len() - self.span;
        let out = result.samples[self.emitted..emit_to].to_vec();

        // Drop history only down to a segment boundary of the *stream*. The envelope
        // estimate lays its segments out from the start of the buffer it is handed, so a
        // buffer that starts mid-segment produces different medians — and the blanked output
        // would then depend on the size of the blocks the sound card happened to deliver.
        let window = self.blanker.window();
        let wanted = emit_to.saturating_sub(self.history);
        let keep_from = wanted - (self.absolute + wanted) % window;
        self.pending.drain(..keep_from);
        self.absolute += keep_from;
        self.emitted = emit_to - keep_from;
        out
    }

    /// Emit everything still held back, judged against whatever context exists.
    ///
    /// For the end of a recording. A live receiver never calls this.
    pub fn flush(&mut self) -> Vec<Complex> {
        if self.pending.len() <= self.emitted {
            self.pending.clear();
            self.emitted = 0;
            return Vec::new();
        }
        let result = self.blanker.process(&self.pending);
        let out = result.samples[self.emitted..].to_vec();
        self.absolute += self.pending.len();
        self.pending.clear();
        self.emitted = 0;
        out
    }
}
