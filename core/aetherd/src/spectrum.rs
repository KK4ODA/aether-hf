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

#[cfg(test)]
mod tests {
    use super::*;

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
