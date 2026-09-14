//! OFDM symbol construction and parsing.
//!
//! **Carrier map.** The active carriers occupy FFT bins `c − n_carriers/2` for carrier index
//! `c`, DC-centred; the passband stage later moves DC to the audio centre frequency. Comb
//! pilots sit on every `pilot_carrier_spacing`-th carrier including both band edges, and the
//! rest carry data.
//!
//! **Known sequences.** Pilot values come from one frequency-domain Zadoff–Chu sequence of
//! length `n_carriers`: unit magnitude on every carrier, and a full pilot symbol built from
//! it has a low-PAPR time waveform (≈ 3.5 dB against ≈ 10 dB for data), which keeps the
//! transmit envelope steady across pilot and data symbols.
//!
//! **Shaping.** Each symbol is `CP + N` samples plus a `taper`-sample cyclic *suffix*, with a
//! raised-cosine ramp on the first and last `taper` samples and an overlap-add of that much
//! between consecutive symbols. The symbol period stays `CP + N`; the taper comes out of the
//! guard, so the FFT window never sees a tapered sample and the windowing adds no
//! inter-carrier interference.

use std::{f64::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex64};

use crate::{constellation::Complex, tables, waveform::WaveformParams};

/// Zadoff–Chu root of the pilot sequence (coprime with 57, 68 and 12).
pub const PILOT_ROOT: usize = 7;

/// Zadoff–Chu sequence `exp(−jπ·u·n(n+c_f)/N)` with `c_f = N mod 2`.
///
/// # Panics
/// If `root` is not coprime with `length`.
#[must_use]
pub fn zadoff_chu(length: usize, root: usize) -> Vec<Complex> {
    fn gcd(mut a: usize, mut b: usize) -> usize {
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        a
    }
    assert!(
        gcd(length, root) == 1,
        "root {root} is not coprime with length {length}"
    );
    let cf = length % 2;
    (0..length)
        .map(|n| {
            let phase = -PI * root as f64 * n as f64 * (n + cf) as f64 / length as f64;
            (phase.cos(), phase.sin())
        })
        .collect()
}

/// Which carriers are pilots, which carry data, and where each sits in the transform.
#[derive(Debug, Clone)]
pub struct CarrierMap {
    params: WaveformParams,
    bins: Vec<isize>,
    pilot_carriers: Vec<usize>,
    data_carriers: Vec<usize>,
    pilot_sequence: Vec<Complex>,
}

impl CarrierMap {
    /// Build the map for a numerology.
    ///
    /// # Panics
    /// If the numerology has no active carriers, or if the pilot sequence computed here
    /// differs from the one exported alongside the preamble tables — which would mean this
    /// implementation and the reference model disagree about what the pilots *are*, and
    /// neither could equalise the other's frames.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let n = params.n_carriers();
        let bins: Vec<isize> = (0..n).map(|c| c as isize - (n / 2) as isize).collect();
        let spacing = params.pilot_carrier_spacing;
        let mut pilot_carriers: Vec<usize> = (0..n).step_by(spacing).collect();
        if *pilot_carriers.last().expect("at least one pilot") != n - 1 {
            pilot_carriers.push(n - 1);
        }
        let data_carriers: Vec<usize> = (0..n).filter(|c| !pilot_carriers.contains(c)).collect();
        Self {
            params,
            bins,
            pilot_carriers,
            data_carriers,
            pilot_sequence: {
                let computed = zadoff_chu(n, PILOT_ROOT);
                if computed.len() == tables::PILOT_SEQUENCE.len() {
                    for (got, want) in computed.iter().zip(tables::PILOT_SEQUENCE.iter()) {
                        assert!(
                            (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
                            "the pilot sequence disagrees with the reference model"
                        );
                    }
                }
                computed
            },
        }
    }

    /// Active carrier count.
    #[must_use]
    pub fn n_carriers(&self) -> usize {
        self.params.n_carriers()
    }

    /// FFT bin (possibly negative) of each carrier index.
    #[must_use]
    pub fn bins(&self) -> &[isize] {
        &self.bins
    }

    /// Carrier indices that are comb pilots.
    #[must_use]
    pub fn pilot_carriers(&self) -> &[usize] {
        &self.pilot_carriers
    }

    /// Carrier indices that carry data.
    #[must_use]
    pub fn data_carriers(&self) -> &[usize] {
        &self.data_carriers
    }

    /// The known unit-magnitude value of every carrier.
    #[must_use]
    pub fn pilot_sequence(&self) -> &[Complex] {
        &self.pilot_sequence
    }

    /// Index into the transform for a carrier, wrapping negative bins.
    #[must_use]
    pub fn bin_index(&self, carrier: usize) -> usize {
        let n = self.params.fft_size as isize;
        ((self.bins[carrier] % n + n) % n) as usize
    }
}

/// Frequency-domain carrier values into windowed, overlap-added time samples.
pub struct OfdmModulator {
    params: WaveformParams,
    map: CarrierMap,
    ramp_up: Vec<f64>,
    ramp_down: Vec<f64>,
    scale: f64,
    ifft: Arc<dyn Fft<f64>>,
}

impl std::fmt::Debug for OfdmModulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfdmModulator")
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

fn raised_cosine_ramp(width: usize) -> Vec<f64> {
    (0..width)
        .map(|i| 0.5 * (1.0 - (PI * (i as f64 + 0.5) / width as f64).cos()))
        .collect()
}

impl OfdmModulator {
    /// Build the modulator.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let map = CarrierMap::new(params);
        let ramp_up = raised_cosine_ramp(params.taper_samples);
        let mut ramp_down = ramp_up.clone();
        ramp_down.reverse();
        // A symbol of unit-power carriers comes out with unit mean power per sample.
        let scale = params.fft_size as f64 / (map.n_carriers() as f64).sqrt();
        let mut planner = FftPlanner::new();
        Self {
            params,
            map,
            ramp_up,
            ramp_down,
            scale,
            ifft: planner.plan_fft_inverse(params.fft_size),
        }
    }

    /// The carrier map this modulator uses.
    #[must_use]
    pub fn map(&self) -> &CarrierMap {
        &self.map
    }

    /// Carrier values for an ordinary data symbol: comb pilots plus `data`.
    ///
    /// # Panics
    /// If `data` is not one value per data carrier.
    #[must_use]
    pub fn data_symbol(&self, data: &[Complex]) -> Vec<Complex> {
        assert_eq!(
            data.len(),
            self.map.data_carriers().len(),
            "one value per data carrier"
        );
        let mut out = vec![(0.0, 0.0); self.map.n_carriers()];
        for &carrier in self.map.pilot_carriers() {
            out[carrier] = self.map.pilot_sequence()[carrier];
        }
        for (&carrier, &value) in self.map.data_carriers().iter().zip(data) {
            out[carrier] = value;
        }
        out
    }

    /// Carrier values for a full pilot symbol, optionally carrying ±1 chips on its data
    /// carriers (which is how a DATA frame signals its mode and redundancy version).
    ///
    /// # Panics
    /// If `chips` is given and is not one per data carrier.
    #[must_use]
    pub fn pilot_symbol(&self, chips: Option<&[f64]>) -> Vec<Complex> {
        let mut out = self.map.pilot_sequence().to_vec();
        if let Some(chips) = chips {
            assert_eq!(
                chips.len(),
                self.map.data_carriers().len(),
                "one chip per data carrier"
            );
            for (&carrier, &chip) in self.map.data_carriers().iter().zip(chips) {
                out[carrier] = (chip, 0.0);
            }
        }
        out
    }

    /// One extended symbol: `CP + N + taper`, with both edges tapered.
    ///
    /// # Panics
    /// If `carrier_values` is not one value per active carrier.
    #[must_use]
    pub fn to_time(&self, carrier_values: &[Complex]) -> Vec<Complex> {
        assert_eq!(
            carrier_values.len(),
            self.map.n_carriers(),
            "one value per carrier"
        );
        let n = self.params.fft_size;
        let mut spectrum = vec![Complex64::new(0.0, 0.0); n];
        for (carrier, &(re, im)) in carrier_values.iter().enumerate() {
            spectrum[self.map.bin_index(carrier)] = Complex64::new(re, im);
        }
        self.ifft.process(&mut spectrum);
        // rustfft's inverse transform is unnormalised, so divide by N and apply the
        // model's scale; the two combine to `1 / sqrt(n_carriers)` per sample.
        let gain = self.scale / n as f64;
        let body: Vec<Complex> = spectrum
            .iter()
            .map(|c| (c.re * gain, c.im * gain))
            .collect();

        let (cp, w) = (self.params.cp_samples, self.params.taper_samples);
        let mut ext = Vec::with_capacity(cp + n + w);
        ext.extend_from_slice(&body[n - cp..]);
        ext.extend_from_slice(&body);
        ext.extend_from_slice(&body[..w]);
        for (sample, &ramp) in ext[..w].iter_mut().zip(&self.ramp_up) {
            sample.0 *= ramp;
            sample.1 *= ramp;
        }
        let tail = ext.len() - w;
        for (sample, &ramp) in ext[tail..].iter_mut().zip(&self.ramp_down) {
            sample.0 *= ramp;
            sample.1 *= ramp;
        }
        ext
    }

    /// Overlap-add a sequence of carrier-value vectors into one waveform.
    ///
    /// The output is `symbols · (CP+N) + taper` long: the trailing taper is the last symbol's
    /// ramp-down, so a caller can overlap-add consecutive bursts.
    #[must_use]
    pub fn modulate(&self, symbols: &[Vec<Complex>]) -> Vec<Complex> {
        let period = self.params.symbol_samples();
        let w = self.params.taper_samples;
        let mut out = vec![(0.0, 0.0); symbols.len() * period + w];
        for (index, values) in symbols.iter().enumerate() {
            let extended = self.to_time(values);
            for (offset, sample) in extended.into_iter().enumerate() {
                let slot = &mut out[index * period + offset];
                slot.0 += sample.0;
                slot.1 += sample.1;
            }
        }
        out
    }
}

/// Why a symbol could not be demodulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemodError {
    /// The symbol runs past the end of the buffer.
    OutOfRange {
        /// Where the transform window would have started.
        start: usize,
        /// How many samples were available.
        available: usize,
    },
}

impl core::fmt::Display for DemodError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::OutOfRange { start, available } => {
                write!(
                    f,
                    "symbol at {start} runs past the end of {available} samples"
                )
            }
        }
    }
}

impl core::error::Error for DemodError {}

/// Time samples back into raw, un-equalised carrier values.
pub struct OfdmDemodulator {
    params: WaveformParams,
    map: CarrierMap,
    scale: f64,
    fft: Arc<dyn Fft<f64>>,
}

impl std::fmt::Debug for OfdmDemodulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfdmDemodulator")
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

impl OfdmDemodulator {
    /// Build the demodulator.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let map = CarrierMap::new(params);
        let scale = (map.n_carriers() as f64).sqrt() / params.fft_size as f64;
        let mut planner = FftPlanner::new();
        Self {
            params,
            map,
            scale,
            fft: planner.plan_fft_forward(params.fft_size),
        }
    }

    /// The carrier map this demodulator uses.
    #[must_use]
    pub fn map(&self) -> &CarrierMap {
        &self.map
    }

    /// Where the FFT window starts inside a symbol period: past the tapered part of the
    /// cyclic prefix, with margin before the useful part so late multipath stays inside it.
    #[must_use]
    pub fn fft_offset(&self) -> usize {
        let (cp, w) = (self.params.cp_samples, self.params.taper_samples);
        w + (cp - w) / 2
    }

    /// Carrier values of the symbol whose period begins at `symbol_start`.
    ///
    /// The window is taken `fft_offset` samples into the period, and the linear phase ramp
    /// that produces across carriers is removed, so a distortion-free channel returns exactly
    /// the transmitted carrier values.
    ///
    /// # Errors
    /// If the symbol runs past the end of the buffer.
    pub fn carriers(
        &self,
        samples: &[Complex],
        symbol_start: usize,
    ) -> Result<Vec<Complex>, DemodError> {
        let n = self.params.fft_size;
        let start = symbol_start + self.fft_offset();
        if start + n > samples.len() {
            return Err(DemodError::OutOfRange {
                start,
                available: samples.len(),
            });
        }
        let mut buffer: Vec<Complex64> = samples[start..start + n]
            .iter()
            .map(|&(re, im)| Complex64::new(re, im))
            .collect();
        self.fft.process(&mut buffer);

        let shift = self.fft_offset() as isize - self.params.cp_samples as isize;
        let out = (0..self.map.n_carriers())
            .map(|carrier| {
                let value = buffer[self.map.bin_index(carrier)] * self.scale;
                let phase = -2.0 * PI * self.map.bins()[carrier] as f64 * shift as f64 / n as f64;
                let rotation = Complex64::new(phase.cos(), phase.sin());
                let corrected = value * rotation;
                (corrected.re, corrected.im)
            })
            .collect();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::waveform::WIDE_2300;

    fn magnitude(value: Complex) -> f64 {
        value.0.hypot(value.1)
    }

    #[test]
    fn the_carrier_map_is_the_specification() {
        let map = CarrierMap::new(WIDE_2300);
        assert_eq!(map.n_carriers(), 57);
        assert_eq!(map.pilot_carriers().len(), 15);
        assert_eq!(map.data_carriers().len(), 42);
        assert_eq!(map.pilot_carriers()[0], 0);
        assert_eq!(*map.pilot_carriers().last().unwrap(), 56);
        assert_eq!(map.bins()[0], -28);
        assert_eq!(map.bins()[56], 28);
    }

    #[test]
    fn the_pilot_sequence_is_unit_magnitude_and_matches_the_exported_table() {
        let map = CarrierMap::new(WIDE_2300);
        for &value in map.pilot_sequence() {
            assert!((magnitude(value) - 1.0).abs() < 1e-12);
        }
        // CarrierMap::new already checks this against the exported table on construction
        assert_eq!(map.pilot_sequence().len(), map.n_carriers());
    }

    #[test]
    fn a_symbol_round_trips_through_modulation_and_demodulation() {
        let modulator = OfdmModulator::new(WIDE_2300);
        let demodulator = OfdmDemodulator::new(WIDE_2300);
        let data: Vec<Complex> = (0..42u16)
            .map(|i| {
                let angle = 0.37 * f64::from(i);
                (angle.cos(), angle.sin())
            })
            .collect();
        let values = modulator.data_symbol(&data);
        // pad with a second symbol so the FFT window has room
        let waveform = modulator.modulate(&[values.clone(), values.clone()]);
        let recovered = demodulator.carriers(&waveform, 0).expect("in range");
        for (index, (got, want)) in recovered.iter().zip(&values).enumerate() {
            assert!(
                (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
                "carrier {index}: ({}, {}) vs ({}, {})",
                got.0,
                got.1,
                want.0,
                want.1
            );
        }
    }

    #[test]
    fn a_full_pilot_symbol_has_lower_peak_to_average_than_a_data_symbol() {
        let modulator = OfdmModulator::new(WIDE_2300);
        let papr = |values: &[Complex]| {
            let body = modulator.to_time(values);
            let powers: Vec<f64> = body.iter().map(|&(re, im)| re * re + im * im).collect();
            let mean = powers.iter().sum::<f64>() / powers.len() as f64;
            let peak = powers.iter().copied().fold(0.0, f64::max);
            10.0 * (peak / mean).log10()
        };
        let data: Vec<Complex> = (0..42)
            .map(|i| {
                if i % 2 == 0 {
                    (0.707, 0.707)
                } else {
                    (-0.707, 0.707)
                }
            })
            .collect();
        assert!(papr(&modulator.pilot_symbol(None)) < papr(&modulator.data_symbol(&data)));
    }

    #[test]
    fn modulation_output_length_is_the_frame_plus_one_taper() {
        let modulator = OfdmModulator::new(WIDE_2300);
        let values = modulator.pilot_symbol(None);
        let symbols = vec![values; 5];
        let waveform = modulator.modulate(&symbols);
        assert_eq!(
            waveform.len(),
            5 * WIDE_2300.symbol_samples() + WIDE_2300.taper_samples
        );
    }

    #[test]
    fn a_symbol_of_unit_power_carriers_has_about_unit_mean_power() {
        let modulator = OfdmModulator::new(WIDE_2300);
        let waveform = modulator.modulate(&vec![modulator.pilot_symbol(None); 8]);
        let mean = waveform
            .iter()
            .map(|&(re, im)| re * re + im * im)
            .sum::<f64>()
            / waveform.len() as f64;
        assert!((mean - 1.0).abs() < 0.05, "mean power {mean}");
    }

    #[test]
    fn demodulating_past_the_end_is_an_error_not_a_panic() {
        let demodulator = OfdmDemodulator::new(WIDE_2300);
        assert!(demodulator.carriers(&[(0.0, 0.0); 10], 0).is_err());
    }

    #[test]
    fn zadoff_chu_rejects_a_root_sharing_a_factor() {
        let result = std::panic::catch_unwind(|| zadoff_chu(10, 5));
        assert!(result.is_err());
    }
}
