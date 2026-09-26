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
        let top = ((TOP_HZ / bin_hz) as usize).min(WINDOW / 2);
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
/// How far either side of the centre the in-band reference noise level is taken from, in
/// hertz: well inside the narrowest filter anyone runs (500 Hz → ±250), so the reference is
/// real in-band noise whatever the filter.
const REFERENCE_HALF_HZ: f64 = 180.0;
/// Quiet spectra to fold in before the width estimate is trusted; at about two a second
/// that is a few seconds of noise.
const MIN_OBSERVATIONS: usize = 12;
/// The noise profile's exponential-moving-average weight for each new spectrum: enough to
/// follow a filter change in a second or two, smooth enough that one noisy window does not
/// move the edges.
const PASSBAND_EMA_ALPHA: f32 = 0.2;

/// The receiver's passband width in hertz, measured from a smoothed noise spectrum: the span
/// between the bins either side of `center_hz` where the power has fallen `PASSBAND_DROP_DB`
/// below the in-band noise and stays there. `None` when there is no window, or the middle of
/// the band is at the very floor (nothing to measure). Full width when nothing drops — a
/// filter wider than the modem, or silence — so it can only ever report *narrower* than the
/// truth, never a false narrowing.
#[must_use]
pub fn passband_width_hz(bins_db: &[f32], bin_hz: f64, center_hz: f64) -> Option<f64> {
    if bins_db.is_empty() || bin_hz <= 0.0 {
        return None;
    }
    let bin_of = |hz: f64| (hz / bin_hz).round() as usize;
    let center = bin_of(center_hz);
    if center >= bins_db.len() {
        return None;
    }
    // in-band reference: the median over ±REFERENCE_HALF_HZ of the centre, so one loud bin
    // (a birdie, a carrier) does not lift it
    let half = bin_of(REFERENCE_HALF_HZ).max(1);
    let lo = center.saturating_sub(half);
    let hi = (center + half).min(bins_db.len() - 1);
    let mut middle: Vec<f32> = bins_db[lo..=hi].to_vec();
    middle.sort_by(f32::total_cmp);
    let reference = middle[middle.len() / 2];
    if !reference.is_finite() {
        return None;
    }
    let threshold = reference - PASSBAND_DROP_DB;
    // scan outward for the first run of EDGE_RUN bins below the threshold
    let edge = |step: isize| -> usize {
        let mut index = center as isize;
        let mut below = 0usize;
        while index >= 0 && (index as usize) < bins_db.len() {
            if bins_db[index as usize] < threshold {
                below += 1;
                if below >= EDGE_RUN {
                    // the edge is where the run began
                    return (index + step.signum() * (EDGE_RUN as isize - 1))
                        .clamp(0, bins_db.len() as isize - 1) as usize;
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
    Some((high.saturating_sub(low)) as f64 * bin_hz)
}

/// Tracks the receiver's passband from the noise it delivers, so a radio filter set narrower
/// than the modem's bandwidth — the mismatch that carries only the middle of every burst and
/// looks like a dead band — is caught without asking the rig anything. Band noise fills the
/// whole SSB passband; a filter confines it, and the confinement is in the audio whatever the
/// radio, the mode or the keying. Fed only quiet audio (no signal, not transmitting), so what
/// it measures is the receiver's own shape, not a narrowband signal that happens to be on.
#[derive(Debug, Clone, Default)]
pub struct PassbandMonitor {
    /// Exponential moving average of the power spectrum, dBFS per bin; empty until the first.
    profile: Vec<f32>,
    bin_hz: f64,
    seen: usize,
}

impl PassbandMonitor {
    /// A fresh monitor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold a spectrum of quiet audio into the running noise profile.
    pub fn observe(&mut self, spectrum: &Spectrum) {
        self.bin_hz = spectrum.bin_hz;
        if self.profile.len() != spectrum.bins_db.len() {
            self.profile.clone_from(&spectrum.bins_db);
            self.seen = 1;
            return;
        }
        for (average, &now) in self.profile.iter_mut().zip(&spectrum.bins_db) {
            *average += PASSBAND_EMA_ALPHA * (now - *average);
        }
        self.seen = self.seen.saturating_add(1);
    }

    /// The receiver's passband width in hertz around `center_hz`, once enough quiet audio
    /// has been folded in for the estimate to be steady.
    #[must_use]
    pub fn width_hz(&self, center_hz: f64) -> Option<f64> {
        if self.seen < MIN_OBSERVATIONS {
            return None;
        }
        passband_width_hz(&self.profile, self.bin_hz, center_hz)
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
