//! Complex baseband (8 kHz) ↔ real audio (48 kHz) at the configured centre frequency.
//!
//! Transmit: baseband → band-limiting FIR → mix up to `centre_hz` → take the real part →
//! ×6 interpolation FIR → audio. Receive is the mirror image: ÷6 decimation FIR → mix down →
//! the same band-limiting FIR, which also removes the image at `−2 × centre` → baseband.
//!
//! Both directions are streaming objects with filter state and phase-continuous mixers, so
//! any block size gives the same output as one long block. Group delays are fixed and known
//! ([`BasebandToAudio::tx_delay_samples`], [`AudioToBaseband::rx_delay_samples`], both at the
//! audio rate); frame synchronisation absorbs them.

use crate::{
    Complex,
    fir::{Fir, firwin_lowpass, kaiser_beta, kaiser_numtaps},
    waveform::WaveformParams,
};

/// Stopband attenuation both filters are designed to, in dB.
const ATTENUATION_DB: f64 = 70.0;
/// Transition width of the baseband band-limiting filter, in Hz.
const BAND_TRANSITION_HZ: f64 = 240.0;

/// Linear-phase low-pass for the complex baseband: flat over every active carrier.
#[must_use]
pub fn band_limit_taps(params: &WaveformParams) -> Vec<f64> {
    let edge = params.occupied_bandwidth_hz() / 2.0 + params.subcarrier_spacing_hz();
    let cutoff = edge + BAND_TRANSITION_HZ / 2.0;
    let numtaps = kaiser_numtaps(ATTENUATION_DB, BAND_TRANSITION_HZ, params.fs_baseband);
    firwin_lowpass(
        numtaps,
        cutoff,
        params.fs_baseband,
        kaiser_beta(ATTENUATION_DB),
    )
}

/// Low-pass at the audio rate for ×R interpolation and ÷R decimation.
///
/// The passband reaches the top of the occupied audio band (centre, plus half the bandwidth,
/// plus one subcarrier spacing) and the stopband starts at the first zero-stuffing image.
/// The length is a multiple of `2R` plus one, so the group delay is a whole number of
/// samples at both rates.
#[must_use]
pub fn resample_taps(params: &WaveformParams) -> Vec<f64> {
    let r = params.resample_factor();
    let audio_rate = params.audio_rate as f64;
    let top =
        params.centre_hz + params.occupied_bandwidth_hz() / 2.0 + params.subcarrier_spacing_hz();
    // symmetric about baseband Nyquist: what the passband leaves is what the image takes
    let transition = 2.0 * (params.fs_baseband / 2.0 - top);
    let estimate = kaiser_numtaps(ATTENUATION_DB, transition, audio_rate);
    let numtaps = estimate.div_ceil(2 * r) * (2 * r) + 1;
    let mut taps = firwin_lowpass(
        numtaps,
        params.fs_baseband / 2.0,
        audio_rate,
        kaiser_beta(ATTENUATION_DB),
    );
    // interpolation gain: zero-stuffing divides the amplitude by R
    for tap in &mut taps {
        *tap *= r as f64;
    }
    taps
}

/// Baseband to the real audio a sound card plays.
#[derive(Debug, Clone)]
pub struct BasebandToAudio {
    params: WaveformParams,
    band: Fir<Complex>,
    interpolate: Fir<f64>,
    n0: usize,
}

impl BasebandToAudio {
    /// Build the chain for a numerology.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        Self {
            band: Fir::new(band_limit_taps(&params)),
            interpolate: Fir::new(resample_taps(&params)),
            params,
            n0: 0,
        }
    }

    /// Group delay from baseband in to audio out, in audio samples.
    #[must_use]
    pub fn tx_delay_samples(&self) -> usize {
        self.band.delay() * self.params.resample_factor() + self.interpolate.delay()
    }

    /// Convert a block of baseband samples to audio.
    pub fn process(&mut self, baseband: &[Complex]) -> Vec<f32> {
        let filtered = self.band.process(baseband);
        let r = self.params.resample_factor();
        let mut stuffed = vec![0.0f64; filtered.len() * r];
        for (i, &(re, im)) in filtered.iter().enumerate() {
            let t = (self.n0 + i) as f64 / self.params.fs_baseband;
            let phase = 2.0 * std::f64::consts::PI * self.params.centre_hz * t;
            // Re{x e^{jωt}}, written out rather than through a complex multiply
            stuffed[i * r] = re.mul_add(phase.cos(), -(im * phase.sin()));
        }
        self.n0 += filtered.len();
        self.interpolate
            .process(&stuffed)
            .into_iter()
            .map(|x| x as f32)
            .collect()
    }
}

/// The audio a sound card captured, back to complex baseband.
#[derive(Debug, Clone)]
pub struct AudioToBaseband {
    params: WaveformParams,
    decimate: Fir<f64>,
    band: Fir<Complex>,
    phase: usize,
    n0: usize,
}

impl AudioToBaseband {
    /// Build the chain for a numerology.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let r = params.resample_factor() as f64;
        let mut taps = resample_taps(&params);
        for tap in &mut taps {
            *tap /= r; // decimation keeps the rate's own gain, unlike interpolation
        }
        Self {
            decimate: Fir::new(taps),
            band: Fir::new(band_limit_taps(&params)),
            params,
            phase: 0,
            n0: 0,
        }
    }

    /// Group delay from audio in to baseband out, in audio samples.
    #[must_use]
    pub fn rx_delay_samples(&self) -> usize {
        self.decimate.delay() + self.band.delay() * self.params.resample_factor()
    }

    /// Convert a block of audio to baseband.
    pub fn process(&mut self, audio: &[f32]) -> Vec<Complex> {
        let wide: Vec<f64> = audio.iter().map(|&x| f64::from(x)).collect();
        let filtered = self.decimate.process(&wide);
        let r = self.params.resample_factor();

        // the decimation phase carries across blocks, so a block whose length is not a
        // multiple of R does not drop or repeat a sample at the join
        let mut low = Vec::with_capacity(filtered.len().div_ceil(r));
        let mut index = self.phase;
        while index < filtered.len() {
            low.push(filtered[index]);
            index += r;
        }
        self.phase = index - filtered.len();

        let mixed: Vec<Complex> = low
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let t = (self.n0 + i) as f64 / self.params.fs_baseband;
                let phase = -2.0 * std::f64::consts::PI * self.params.centre_hz * t;
                (x * phase.cos(), x * phase.sin())
            })
            .collect();
        self.n0 += low.len();
        // ×2 undoes the halving that taking the real part cost on the way out
        self.band
            .process(&mixed)
            .into_iter()
            .map(|(re, im)| (2.0 * re, 2.0 * im))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::WIDE_2300;

    /// A tone at baseband offset `offset_hz`, `n` samples long.
    fn tone(offset_hz: f64, n: usize, fs: f64) -> Vec<Complex> {
        (0..n)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * offset_hz * i as f64 / fs;
                (phase.cos(), phase.sin())
            })
            .collect()
    }

    #[test]
    fn both_filters_are_symmetric_and_odd_length() {
        let params = WIDE_2300;
        for taps in [band_limit_taps(&params), resample_taps(&params)] {
            assert_eq!(taps.len() % 2, 1, "an even filter has a half-sample delay");
            for (a, b) in taps.iter().zip(taps.iter().rev()) {
                assert!((a - b).abs() < 1e-15, "not linear phase");
            }
        }
        // the interpolation filter's length must also be a multiple of 2R plus one, so the
        // delay is whole at baseband too
        let r = params.resample_factor();
        assert_eq!((resample_taps(&params).len() - 1) % (2 * r), 0);
    }

    #[test]
    fn a_round_trip_returns_the_signal_it_was_given() {
        // The chain is transparent up to its known delay and one constant phase: the two
        // mixers count samples at opposite ends of the chain, so the round trip rotates the
        // signal by 2π·centre·delay. That is a channel phase like any other and frame
        // synchronisation removes it — what has to hold is that nothing else changed.
        let params = WIDE_2300;
        let signal = tone(500.0, 4000, params.fs_baseband);
        let mut tx = BasebandToAudio::new(params);
        let audio = tx.process(&signal);
        let mut rx = AudioToBaseband::new(params);
        let back = rx.process(&audio);

        let r = params.resample_factor();
        let delay = (tx.tx_delay_samples() + rx.rx_delay_samples()) / r;
        assert_eq!(
            (tx.tx_delay_samples() + rx.rx_delay_samples()) % r,
            0,
            "the delay must be whole at baseband"
        );
        let start = delay + 200; // clear of the filters' transients at either end
        let count = 2000;
        assert!(back.len() > start + count, "too short: {}", back.len());

        // the single rotation, estimated over the whole comparison window
        let (mut cr, mut ci) = (0.0, 0.0);
        for i in 0..count {
            let (re, im) = back[start + i];
            let (wr, wi) = signal[start - delay + i];
            cr += re * wr + im * wi;
            ci += im * wr - re * wi;
        }
        let magnitude = cr.hypot(ci);
        assert!(magnitude > 0.0, "no correlation at all");
        let (pr, pi) = (cr / magnitude, ci / magnitude);

        let mut worst = 0.0f64;
        for i in 0..count {
            let (wr, wi) = signal[start - delay + i];
            let expected = (wr * pr - wi * pi, wr * pi + wi * pr);
            let (re, im) = back[start + i];
            worst = worst.max((re - expected.0).hypot(im - expected.1));
        }
        // well under the 0.39 a single sample of slip at this tone would cost
        assert!(worst < 0.05, "round trip error {worst} after de-rotation");
    }

    #[test]
    fn block_size_does_not_change_either_direction() {
        let params = WIDE_2300;
        let signal = tone(300.0, 3000, params.fs_baseband);

        let reference_audio = BasebandToAudio::new(params).process(&signal);
        for chunk in [1usize, 17, 512] {
            let mut tx = BasebandToAudio::new(params);
            let mut got = Vec::new();
            for block in signal.chunks(chunk) {
                got.extend(tx.process(block));
            }
            assert_eq!(got.len(), reference_audio.len(), "chunk {chunk}");
            for (a, b) in got.iter().zip(&reference_audio) {
                assert!((a - b).abs() < 1e-6, "chunk {chunk} changed the audio");
            }
        }

        let reference_bb = AudioToBaseband::new(params).process(&reference_audio);
        for chunk in [1usize, 17, 512] {
            let mut rx = AudioToBaseband::new(params);
            let mut got = Vec::new();
            for block in reference_audio.chunks(chunk) {
                got.extend(rx.process(block));
            }
            assert_eq!(got.len(), reference_bb.len(), "chunk {chunk}");
            for (a, b) in got.iter().zip(&reference_bb) {
                assert!(
                    (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12,
                    "chunk {chunk} changed the baseband"
                );
            }
        }
    }

    #[test]
    fn the_transmitted_audio_stays_inside_the_channel() {
        // an operator's transmitter has to fit the filter it is fed through: nothing of
        // consequence outside the occupied band, in either direction
        let params = WIDE_2300;
        let n = 8192;
        let mut wide = Vec::new();
        for offset in [-1100.0, -500.0, 0.0, 500.0, 1100.0] {
            wide.push(tone(offset, n, params.fs_baseband));
        }
        let combined: Vec<Complex> = (0..n)
            .map(|i| {
                wide.iter()
                    .fold((0.0, 0.0), |acc, t| (acc.0 + t[i].0, acc.1 + t[i].1))
            })
            .collect();
        let audio = BasebandToAudio::new(params).process(&combined);

        // power at a frequency, by direct correlation — no FFT needed for a handful of bins
        let power_at = |f: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &x) in audio.iter().enumerate() {
                let phase = -2.0 * std::f64::consts::PI * f * i as f64 / params.audio_rate as f64;
                re += f64::from(x) * phase.cos();
                im += f64::from(x) * phase.sin();
            }
            (re * re + im * im) / (audio.len() * audio.len()) as f64
        };
        let in_band = power_at(params.centre_hz);
        for out in [
            params.centre_hz - 1600.0,
            params.centre_hz + 1600.0,
            params.centre_hz + 4000.0,
        ] {
            let leak = power_at(out);
            assert!(
                leak < in_band * 1e-5,
                "leakage at {out} Hz is {:.1} dB down, not enough",
                10.0 * (in_band / leak).log10()
            );
        }
    }
}
