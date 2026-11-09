//! A spectrum of what the sound card is delivering, for the panel's display.
//!
//! This is instrumentation, not signal processing: the modem decodes without it, and it
//! reads nothing back. It keeps the last window of captured audio and, when a client asks,
//! windows it and transforms it once. Nothing is computed while nobody is looking, which
//! is why the control API offers it as a method to poll rather than an event to stream —
//! a panel on the Diagnostics tab asks ten times a second; a gateway never asks.
//!
//! The transform covers the audio passband up to [`TOP_HZ`], at the resolution one
//! [`WINDOW`]-sample block gives (about 12 Hz at 48 kHz): enough to see a carrier, a tone,
//! the shape of a burst and the noise floor either side of it, which is what an operator
//! setting a receiver's filter or hunting an interfering signal wants.

use std::{collections::VecDeque, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex64};

/// Samples in one transform.
pub const WINDOW: usize = 4096;

/// The highest frequency reported, in hertz. Nothing this modem cares about is above it.
pub const TOP_HZ: f64 = 4000.0;

/// The analyser: a ring of recent audio and a transform ready to run on it.
pub struct SpectrumAnalyser {
    sample_rate: f64,
    ring: VecDeque<f32>,
    fft: Arc<dyn Fft<f64>>,
    window: Vec<f64>,
}

impl std::fmt::Debug for SpectrumAnalyser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpectrumAnalyser")
            .field("sample_rate", &self.sample_rate)
            .field("held", &self.ring.len())
            .finish_non_exhaustive()
    }
}

/// One spectrum, as the API reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spectrum {
    /// Hertz per bin.
    pub bin_hz: f64,
    /// Power per bin in dBFS, from 0 Hz upwards, [`TOP_HZ`] at most.
    pub bins_db: Vec<f32>,
}

impl Spectrum {
    /// The level around `center_hz`, dBFS: the median of the bins within the reference band
    /// [`passband_edges`] reads its in-band level from, so one look's own level is on the same
    /// footing as the profile's.
    #[must_use]
    pub fn level_around(&self, center_hz: f64) -> f32 {
        reference_level(&self.bins_db, self.bin_hz, center_hz).unwrap_or(f32::NAN)
    }
}

/// The median of the bins within ±[`REFERENCE_HALF_HZ`] of `center_hz`: the in-band reference.
fn reference_level(bins_db: &[f32], bin_hz: f64, center_hz: f64) -> Option<f32> {
    if bins_db.is_empty() || bin_hz <= 0.0 {
        return None;
    }
    let bin_of = |hz: f64| (hz / bin_hz).round() as usize;
    let center = bin_of(center_hz);
    if center >= bins_db.len() {
        return None;
    }
    let half = bin_of(REFERENCE_HALF_HZ).max(1);
    let lo = center.saturating_sub(half);
    let hi = (center + half).min(bins_db.len() - 1);
    let mut middle: Vec<f32> = bins_db[lo..=hi].to_vec();
    middle.sort_by(f32::total_cmp);
    Some(middle[middle.len() / 2])
}

impl SpectrumAnalyser {
    /// Build one for audio at `sample_rate`.
    #[must_use]
    pub fn new(sample_rate: f64) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(WINDOW);
        // Hann: a tone stays a tone, and the noise floor beside it is the floor rather than
        // the leakage of the tone
        let window = (0..WINDOW)
            .map(|n| 0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / (WINDOW as f64 - 1.0)).cos())
            .collect();
        Self {
            sample_rate,
            ring: VecDeque::with_capacity(WINDOW),
            fft,
            window,
        }
    }

    /// Remember the newest audio; the oldest is forgotten past one window.
    pub fn push(&mut self, audio: &[f32]) {
        let keep = audio.len().min(WINDOW);
        let excess = (self.ring.len() + keep).saturating_sub(WINDOW);
        self.ring.drain(..excess);
        self.ring.extend(&audio[audio.len() - keep..]);
    }

    /// Whether a whole window has been heard yet.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.ring.len() == WINDOW
    }

    /// How much audio one transform covers, in seconds — about 85 ms at 48 kHz. A spectrum
    /// describes exactly this much of the past, so a caller that wants one of a quiet
    /// channel needs the channel quiet for this long first.
    #[must_use]
    pub fn window_s(&self) -> f64 {
        WINDOW as f64 / self.sample_rate
    }

    /// Transform what is held. `None` until a whole window has been heard.
    #[must_use]
    pub fn compute(&self) -> Option<Spectrum> {
        self.compute_to(TOP_HZ)
    }

    /// [`Self::compute`] up to `top_hz` rather than [`TOP_HZ`]: the receive-filter review's
    /// candidates read the audio chain's own floor between 6 and 10 kHz, where no radio passes
    /// audio (`crate::passband`).
    #[must_use]
    pub fn compute_to(&self, top_hz: f64) -> Option<Spectrum> {
        if !self.ready() {
            return None;
        }
        let mut buffer: Vec<Complex64> = self
            .ring
            .iter()
            .zip(&self.window)
            .map(|(&sample, &w)| Complex64::new(f64::from(sample) * w, 0.0))
            .collect();
        self.fft.process(&mut buffer);
        let bin_hz = self.sample_rate / WINDOW as f64;
        let top = ((top_hz / bin_hz) as usize).min(WINDOW / 2);
        // Scaled so a full-scale sine reads 0 dBFS: the window's coherent gain is its mean
        // (one half for Hann), and a real sine puts half its amplitude in each of two bins.
        let gain = self.window.iter().sum::<f64>() / 2.0;
        let floor = 1e-12;
        let bins_db = buffer[..=top]
            .iter()
            .map(|bin| {
                let amplitude = bin.norm() / gain;
                (20.0 * amplitude.max(floor).log10()) as f32
            })
            .collect();
        Some(Spectrum { bin_hz, bins_db })
    }
}

/// How much a bin must fall below the in-band noise level to count as outside the passband.
const PASSBAND_DROP_DB: f32 = 6.0;
/// Bins in a row that must stay below the drop before an edge is called, so a single noisy
/// bin is not one: about 36 Hz at 12 Hz resolution.
const EDGE_RUN: usize = 3;
/// Where in the profile's bins, lowest first, the floor outside the filter is read: the
/// twentieth. A 3 kHz filter leaves a quarter of the measured range outside it.
const FLOOR_QUANTILE: f64 = 0.05;
/// How far past an edge the profile must stay down, in hertz: the median of the bins beyond
/// the run must be below the drop too. A filter's skirt stays down; a notch the radio's DSP
/// puts in, or a gap a signal left, comes back up within it.
const EDGE_HOLD_HZ: f64 = 150.0;
/// How far either side of the centre the in-band reference noise level is taken from, in
/// hertz: well inside the narrowest filter anyone runs (500 Hz → ±250), so the reference is
/// real in-band noise whatever the filter.
const REFERENCE_HALF_HZ: f64 = 180.0;
/// Quiet spectra kept before the width is trusted: a few seconds of noise at two a second.
/// A few looks are enough, since an edge has to fall halfway to the floor and stay there.
const MIN_OBSERVATIONS: usize = 12;
/// Quiet spectra each bin's noise is read over: about a minute at two a second, so a filter
/// change shows within a minute.
const HISTORY: usize = 120;
/// The share of a bin's looks its noise is read at: the tenth percentile. Band noise is in
/// every look; a signal only in those it was on the air for. On 7074 kHz the FT8 stations stop
/// together for two seconds in every fifteen, so even a bin a signal holds every period is
/// quiet in a look in seven — and an average of the looks, which this replaced, read a band
/// full of FT8 near the middle as a filter 850 Hz wide (the author, beta.130).
const NOISE_QUANTILE: f64 = 0.1;

/// The receiver's passband width in hertz, measured from a noise profile: the span between
/// the bins either side of `center_hz` where the power has fallen `PASSBAND_DROP_DB` below the
/// in-band noise — or, deeper, halfway to the floor outside the filter, whichever is lower —
/// and stays there (`EDGE_HOLD_HZ` past the drop). `None` when there is no
/// window, or the middle of the band is at the very floor (nothing to measure). Full width
/// when nothing drops — a filter wider than the modem, or silence.
#[must_use]
pub fn passband_width_hz(bins_db: &[f32], bin_hz: f64, center_hz: f64) -> Option<f64> {
    passband_edges(bins_db, bin_hz, center_hz).map(|edges| edges.width_hz())
}

/// Where [`passband_width_hz`] puts the passband's edges, and the levels it drew them from:
/// what a trace of the estimate reports (`aetherd --passband-trace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PassbandEdges {
    /// The low edge, hertz: 0 when nothing drops on the way down.
    pub low_hz: f64,
    /// The high edge, hertz: the top of the measured range when nothing drops on the way up.
    pub high_hz: f64,
    /// The in-band reference level around the centre, dBFS.
    pub reference_db: f32,
    /// The level an edge is drawn under, dBFS.
    pub threshold_db: f32,
    /// The floor the threshold was drawn toward, dBFS.
    pub floor_db: f32,
}

impl PassbandEdges {
    /// The span between the edges, hertz.
    #[must_use]
    pub fn width_hz(&self) -> f64 {
        (self.high_hz - self.low_hz).max(0.0)
    }
}

/// The passband's edges in a noise profile, as [`passband_width_hz`] draws them.
#[must_use]
pub fn passband_edges(bins_db: &[f32], bin_hz: f64, center_hz: f64) -> Option<PassbandEdges> {
    if bins_db.is_empty() || bin_hz <= 0.0 {
        return None;
    }
    let bin_of = |hz: f64| (hz / bin_hz).round() as usize;
    let center = bin_of(center_hz);
    // in-band reference: the median over ±REFERENCE_HALF_HZ of the centre, so one loud bin
    // (a birdie, a carrier) does not lift it
    let reference = reference_level(bins_db, bin_hz, center_hz)?;
    if !reference.is_finite() {
        return None;
    }
    // the floor the audio falls to outside the radio's filter: the profile's low end over the
    // whole measured range. A filter's edge falls most of the way to it; signals in the
    // passband lift its middle a few decibels at most, and the band noise either side of them
    // stays far above it. Where the band noise is barely over the floor, the plain drop under
    // the middle is the test, as it always was
    let mut sorted: Vec<f32> = bins_db.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[((sorted.len() - 1) as f64 * FLOOR_QUANTILE).round() as usize];
    let threshold = (reference - PASSBAND_DROP_DB).min(f32::midpoint(reference, floor));
    let hold = bin_of(EDGE_HOLD_HZ).max(EDGE_RUN);
    // whether the profile stays down beyond `start`, going `step`: the median of the next
    // `hold` bins under the drop too, or too few bins left to say (the measured range ends)
    let stays_down = |start: isize, step: isize| -> bool {
        let mut beyond: Vec<f32> = (0..hold as isize)
            .map(|i| start + step * i)
            .filter(|&j| j >= 0 && (j as usize) < bins_db.len())
            .map(|j| bins_db[j as usize])
            .collect();
        if beyond.len() < EDGE_RUN {
            return true;
        }
        beyond.sort_by(f32::total_cmp);
        beyond[beyond.len() / 2] < threshold
    };
    // scan outward for the first run of EDGE_RUN bins below the threshold that stays down
    let edge = |step: isize| -> usize {
        let mut index = center as isize;
        let mut below = 0usize;
        while index >= 0 && (index as usize) < bins_db.len() {
            if bins_db[index as usize] < threshold {
                below += 1;
                if below >= EDGE_RUN {
                    let run_start = index - step * (EDGE_RUN as isize - 1);
                    if stays_down(run_start, step) {
                        return (index + step.signum() * (EDGE_RUN as isize - 1))
                            .clamp(0, bins_db.len() as isize - 1)
                            as usize;
                    }
                    // a dip that comes back up: a notch or a gap, not the filter's edge
                    below = 0;
                }
            } else {
                below = 0;
            }
            index += step;
        }
        // no drop found in this direction: the passband runs off the end of what we measure
        if step > 0 { bins_db.len() - 1 } else { 0 }
    };
    let high = edge(1);
    let low = edge(-1);
    Some(PassbandEdges {
        low_hz: low as f64 * bin_hz,
        high_hz: high.max(low) as f64 * bin_hz,
        reference_db: reference,
        threshold_db: threshold,
        floor_db: floor,
    })
}

/// Tracks the receiver's passband from the noise it delivers, so a radio filter set narrower
/// than the modem's bandwidth — the mismatch that carries only the middle of every burst and
/// looks like a dead band — is caught without asking the rig anything. Band noise fills the
/// whole SSB passband; a filter confines it, and the confinement is in the audio whatever the
/// radio, the mode or the keying. Fed audio the busy detector calls quiet, not transmitting;
/// each bin's noise is its low percentile over the last minute of it ([`NOISE_QUANTILE`]), so
/// signals that come and go — a band of FT8 — never stand in for the noise.
#[derive(Debug, Clone, Default)]
pub struct PassbandMonitor {
    /// The last [`HISTORY`] quiet spectra, dBFS per bin, oldest first.
    history: VecDeque<Vec<f32>>,
    bin_hz: f64,
    seen: usize,
}

impl PassbandMonitor {
    /// A fresh monitor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep a spectrum of quiet audio among the last [`HISTORY`].
    pub fn observe(&mut self, spectrum: &Spectrum) {
        if self
            .history
            .front()
            .is_some_and(|first| first.len() != spectrum.bins_db.len())
            || (self.bin_hz - spectrum.bin_hz).abs() > 1e-9
        {
            self.history.clear();
        }
        self.bin_hz = spectrum.bin_hz;
        self.history.push_back(spectrum.bins_db.clone());
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.seen = self.seen.saturating_add(1);
    }

    /// The noise profile: each bin's [`NOISE_QUANTILE`] over the spectra kept.
    fn profile(&self) -> Vec<f32> {
        let looks = self.history.len();
        let Some(first) = self.history.front() else {
            return Vec::new();
        };
        let at = ((looks - 1) as f64 * NOISE_QUANTILE).round() as usize;
        let mut column = Vec::with_capacity(looks);
        (0..first.len())
            .map(|bin| {
                column.clear();
                column.extend(self.history.iter().map(|spectrum| spectrum[bin]));
                *column.select_nth_unstable_by(at, f32::total_cmp).1
            })
            .collect()
    }

    /// The receiver's passband width in hertz around `center_hz`, once enough quiet audio
    /// has been kept for each bin's noise to be read.
    #[must_use]
    pub fn width_hz(&self, center_hz: f64) -> Option<f64> {
        if self.history.len() < MIN_OBSERVATIONS {
            return None;
        }
        passband_width_hz(&self.profile(), self.bin_hz, center_hz)
    }

    /// Where the passband's edges are around `center_hz`, once the width can be read
    /// ([`Self::width_hz`]).
    #[must_use]
    pub fn edges(&self, center_hz: f64) -> Option<PassbandEdges> {
        if self.history.len() < MIN_OBSERVATIONS {
            return None;
        }
        passband_edges(&self.profile(), self.bin_hz, center_hz)
    }

    /// The noise profile the edges are drawn from, with its bin width: `None` before the first
    /// look. A recording keeps it at its start and end.
    #[must_use]
    pub fn noise_profile(&self) -> Option<(f64, Vec<f32>)> {
        (!self.history.is_empty()).then(|| (self.bin_hz, self.profile()))
    }

    /// How many quiet spectra are kept now.
    #[must_use]
    pub fn looks(&self) -> usize {
        self.history.len()
    }

    /// How many spectra have been folded in, for a test that must tell a monitor kept away
    /// from signals from one that stopped listening altogether.
    #[cfg(test)]
    pub(crate) fn observations(&self) -> usize {
        self.seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat noise spectrum band-limited to `[lo, hi]` Hz: full noise inside, a floor
    /// outside, as a receiver's filter delivers it.
    fn filtered_noise(bin_hz: f64, lo: f64, hi: f64) -> Vec<f32> {
        let bins = (TOP_HZ / bin_hz) as usize + 1;
        (0..bins)
            .map(|k| {
                let hz = k as f64 * bin_hz;
                if hz >= lo && hz <= hi { -40.0 } else { -95.0 }
            })
            .collect()
    }

    #[test]
    fn the_passband_width_is_read_from_where_the_noise_drops() {
        let bin_hz = 48_000.0 / WINDOW as f64;
        // a 2.4 kHz SSB filter around 1500 Hz: ~300..2700
        let wide = filtered_noise(bin_hz, 300.0, 2700.0);
        let w = passband_width_hz(&wide, bin_hz, 1500.0).expect("a width");
        assert!((w - 2400.0).abs() < 120.0, "wide filter read as {w} Hz");
        // a 500 Hz filter around 1500 Hz: ~1250..1750
        let narrow = filtered_noise(bin_hz, 1250.0, 1750.0);
        let n = passband_width_hz(&narrow, bin_hz, 1500.0).expect("a width");
        assert!((n - 500.0).abs() < 120.0, "narrow filter read as {n} Hz");
        assert!(
            n < 2300.0 - 400.0,
            "a 500 Hz filter must read narrow: {n} Hz"
        );
    }

    #[test]
    fn a_flat_spectrum_with_no_drop_reads_wide_not_narrow() {
        let bin_hz = 48_000.0 / WINDOW as f64;
        let flat = vec![-40.0f32; (TOP_HZ / bin_hz) as usize + 1];
        // nothing drops, so it runs to the ends: wider than any modem bandwidth, never a
        // false "too narrow"
        let w = passband_width_hz(&flat, bin_hz, 1500.0).expect("a width");
        assert!(w > 2300.0, "flat noise read as {w} Hz, would false-warn");
    }

    #[test]
    fn a_notch_in_the_passband_is_not_its_edge() {
        // a radio's manual notch, 60 Hz wide and deep, at 1800 Hz in a 2.4 kHz filter: the
        // profile comes back up past it, so the edge is the filter's, not the notch's
        let bin_hz = 48_000.0 / WINDOW as f64;
        let mut notched = filtered_noise(bin_hz, 300.0, 2700.0);
        for (k, bin) in notched.iter_mut().enumerate() {
            if (k as f64 * bin_hz - 1800.0).abs() <= 30.0 {
                *bin = -80.0;
            }
        }
        let w = passband_width_hz(&notched, bin_hz, 1500.0).expect("a width");
        assert!((w - 2400.0).abs() < 120.0, "notched filter read as {w} Hz");
    }

    /// Band noise in a filter `[lo, hi]` Hz, each bin's power drawn afresh as noise is (an
    /// exponential, so a single look varies by several decibels), and, in `busy` of the looks,
    /// FT8-like signals 15 dB over the noise on a share `held` of the bins between 900 and
    /// 2100 Hz — a band of FT8 thickest near the middle, gone in the period's quiet gap.
    fn ft8_band(
        bin_hz: f64,
        lo: f64,
        hi: f64,
        looks: usize,
        busy: f64,
        held: f64,
    ) -> Vec<Spectrum> {
        let mut state = 0x9E37_79B9_7F4A_7C15_u64;
        let mut uniform = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        };
        let bins = (TOP_HZ / bin_hz) as usize + 1;
        (0..looks)
            .map(|_| {
                let signals = uniform() < busy;
                let bins_db = (0..bins)
                    .map(|k| {
                        let hz = k as f64 * bin_hz;
                        let level = if hz >= lo && hz <= hi { -40.0 } else { -95.0 };
                        let noise = level + 10.0 * (-uniform().ln()).log10();
                        if signals && (900.0..=2100.0).contains(&hz) && uniform() < held {
                            (level + 15.0).max(noise) as f32
                        } else {
                            noise as f32
                        }
                    })
                    .collect();
                Spectrum { bin_hz, bins_db }
            })
            .collect()
    }

    #[test]
    fn a_band_full_of_ft8_is_not_a_narrow_filter_and_a_narrow_filter_still_is() {
        // the author on 7074 kHz with a 3 kHz filter, beta.130: the average of the looks read
        // 850 Hz. Signals on most bins near the middle, in most looks, must not stand in for
        // the noise; a 500 Hz filter under the same traffic must still read narrow
        let bin_hz = 48_000.0 / WINDOW as f64;
        let mut monitor = PassbandMonitor::new();
        for (look, spectrum) in ft8_band(bin_hz, 200.0, 3200.0, HISTORY, 0.85, 0.85)
            .iter()
            .enumerate()
        {
            monitor.observe(spectrum);
            // from the first reading on, not only once a minute of looks is kept
            if look + 1 >= MIN_OBSERVATIONS {
                let wide = monitor.width_hz(1500.0).expect("a width");
                assert!(
                    wide > 2300.0 - 400.0,
                    "a 3 kHz filter in an FT8 band read as {wide} Hz after {} looks",
                    look + 1
                );
            }
        }
        let mut monitor = PassbandMonitor::new();
        for spectrum in ft8_band(bin_hz, 1250.0, 1750.0, HISTORY, 0.85, 0.85) {
            monitor.observe(&spectrum);
        }
        let narrow = monitor.width_hz(1500.0).expect("a width");
        assert!(
            (narrow - 500.0).abs() < 150.0,
            "a 500 Hz filter read as {narrow} Hz"
        );
    }

    #[test]
    fn the_monitor_waits_for_enough_noise_before_it_estimates() {
        let bin_hz = 48_000.0 / WINDOW as f64;
        let spectrum = Spectrum {
            bin_hz,
            bins_db: filtered_noise(bin_hz, 1250.0, 1750.0),
        };
        let mut monitor = PassbandMonitor::new();
        monitor.observe(&spectrum);
        assert!(monitor.width_hz(1500.0).is_none(), "one look is not enough");
        for _ in 0..MIN_OBSERVATIONS {
            monitor.observe(&spectrum);
        }
        let w = monitor.width_hz(1500.0).expect("a width now");
        assert!((w - 500.0).abs() < 120.0, "monitor read {w} Hz");
    }

    #[test]
    fn a_tone_shows_as_a_peak_at_its_frequency_and_at_its_level() {
        let fs = 48_000.0;
        let mut analyser = SpectrumAnalyser::new(fs);
        assert!(analyser.compute().is_none(), "nothing heard yet");
        // fed in blocks the size the daemon uses, with a half-scale 1500 Hz sine
        let block = 960;
        let samples: Vec<f32> = (0..6 * WINDOW)
            .map(|n| 0.5 * (std::f64::consts::TAU * 1500.0 * n as f64 / fs).sin() as f32)
            .collect();
        for chunk in samples.chunks(block) {
            analyser.push(chunk);
        }
        let spectrum = analyser.compute().expect("a window was heard");
        assert_eq!(
            spectrum.bins_db.len(),
            (TOP_HZ / spectrum.bin_hz) as usize + 1
        );
        let (peak, &level) = spectrum
            .bins_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .expect("bins");
        let at_hz = peak as f64 * spectrum.bin_hz;
        assert!(
            (at_hz - 1500.0).abs() <= spectrum.bin_hz,
            "peak at {at_hz} Hz"
        );
        // half scale is -6 dBFS; the tone is not on a bin centre, so allow the scallop
        assert!(
            (-6.0 - f64::from(level)).abs() < 2.0,
            "peak reads {level} dBFS"
        );
        // and the floor beside it is a floor: far below the tone
        let far = spectrum.bins_db[(500.0 / spectrum.bin_hz) as usize];
        assert!(
            level - far > 60.0,
            "floor {far} dBFS beside a {level} dBFS tone"
        );
    }

    #[test]
    fn only_the_newest_window_is_kept() {
        let mut analyser = SpectrumAnalyser::new(48_000.0);
        analyser.push(&vec![1.0; 3 * WINDOW]);
        assert!(analyser.ready());
        assert_eq!(analyser.ring.len(), WINDOW);
        analyser.push(&[0.0; 100]);
        assert_eq!(analyser.ring.len(), WINDOW);
        assert_eq!(analyser.ring.back(), Some(&0.0));
    }
}
