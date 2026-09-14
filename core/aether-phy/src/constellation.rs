//! Gray-labelled constellations with a max-log LLR demapper.
//!
//! Mappings follow the public definitions in 3GPP TS 38.211 §5.1 for QPSK, 16-QAM and
//! 64-QAM, real ±1 for BPSK, and a Gray-coded circle for 8-PSK. Every constellation has unit
//! mean power, and bit order within a symbol is MSB first: `k = Σ b_i · 2^(m-1-i)`.
//!
//! **LLR convention, used everywhere in Aether: positive means bit 0 is more likely.**
//! `LLR = log P(b=0|y) / P(b=1|y)`, so with max-log and complex noise variance `σ²`
//! (total, both quadratures) `LLR_i = (min_{s∈S₁}|y−s|² − min_{s∈S₀}|y−s|²) / σ²`.

use crate::waveform::Modulation;

/// A complex baseband sample.
pub type Complex = (f64, f64);

/// Mapper and demapper for one modulation.
#[derive(Debug, Clone)]
pub struct Constellation {
    modulation: Modulation,
    bits_per_symbol: usize,
    points: Vec<Complex>,
    /// Per bit position, the point indices whose bit is 0 and whose bit is 1.
    zero_indices: Vec<Vec<usize>>,
    one_indices: Vec<Vec<usize>>,
}

/// Bits of `index`, MSB first.
fn bits_of(index: usize, m: usize) -> Vec<u8> {
    (0..m).map(|i| ((index >> (m - 1 - i)) & 1) as u8).collect()
}

/// Constellation point for every index `0 … 2^m − 1`, following TS 38.211 §5.1.
fn points_for(modulation: Modulation) -> Vec<Complex> {
    let m = modulation.bits_per_symbol();
    let order = modulation.order();
    // bit 0 maps to +1, bit 1 to −1
    let sign = |index: usize, position: usize| -> f64 {
        1.0 - 2.0 * f64::from(bits_of(index, m)[position])
    };
    match modulation {
        Modulation::Bpsk => (0..order).map(|k| (sign(k, 0), 0.0)).collect(),
        Modulation::Qpsk => {
            let scale = 1.0 / 2.0_f64.sqrt();
            (0..order)
                .map(|k| (sign(k, 0) * scale, sign(k, 1) * scale))
                .collect()
        }
        Modulation::Psk8 => {
            // Gray-coded circle: point k sits at angle 2πk/8 and carries label k ^ (k >> 1).
            let mut out = vec![(0.0, 0.0); 8];
            for k in 0..8usize {
                let gray = k ^ (k >> 1);
                let angle = 2.0 * std::f64::consts::PI * k as f64 / 8.0;
                out[gray] = (angle.cos(), angle.sin());
            }
            out
        }
        Modulation::Qam16 => {
            let scale = 1.0 / 10.0_f64.sqrt();
            (0..order)
                .map(|k| {
                    let i = sign(k, 0) * (2.0 - sign(k, 2));
                    let q = sign(k, 1) * (2.0 - sign(k, 3));
                    (i * scale, q * scale)
                })
                .collect()
        }
        Modulation::Qam64 => {
            let scale = 1.0 / 42.0_f64.sqrt();
            (0..order)
                .map(|k| {
                    let i = sign(k, 0) * (4.0 - sign(k, 2) * (2.0 - sign(k, 4)));
                    let q = sign(k, 1) * (4.0 - sign(k, 3) * (2.0 - sign(k, 5)));
                    (i * scale, q * scale)
                })
                .collect()
        }
    }
}

impl Constellation {
    /// Build the mapper for a modulation.
    #[must_use]
    pub fn new(modulation: Modulation) -> Self {
        let m = modulation.bits_per_symbol();
        let points = points_for(modulation);
        let mut zero_indices = vec![Vec::new(); m];
        let mut one_indices = vec![Vec::new(); m];
        for index in 0..points.len() {
            for (position, bit) in bits_of(index, m).into_iter().enumerate() {
                if bit == 0 {
                    zero_indices[position].push(index);
                } else {
                    one_indices[position].push(index);
                }
            }
        }
        Self {
            modulation,
            bits_per_symbol: m,
            points,
            zero_indices,
            one_indices,
        }
    }

    /// Which modulation this maps.
    #[must_use]
    pub fn modulation(&self) -> Modulation {
        self.modulation
    }

    /// Bits per constellation symbol.
    #[must_use]
    pub fn bits_per_symbol(&self) -> usize {
        self.bits_per_symbol
    }

    /// The constellation points, indexed by their bit label.
    #[must_use]
    pub fn points(&self) -> &[Complex] {
        &self.points
    }

    /// Map a flat bit slice (MSB first per symbol) onto constellation symbols.
    ///
    /// # Panics
    /// If the length is not a multiple of the bits per symbol.
    #[must_use]
    pub fn map(&self, bits: &[u8]) -> Vec<Complex> {
        let m = self.bits_per_symbol;
        assert!(bits.len() % m == 0, "bit count must be a multiple of {m}");
        bits.chunks_exact(m)
            .map(|chunk| {
                let index = chunk
                    .iter()
                    .fold(0usize, |acc, &b| (acc << 1) | usize::from(b & 1));
                self.points[index]
            })
            .collect()
    }

    /// Nearest-point decisions, as a flat bit slice.
    #[must_use]
    pub fn hard(&self, symbols: &[Complex]) -> Vec<u8> {
        let mut out = Vec::with_capacity(symbols.len() * self.bits_per_symbol);
        for &(yi, yq) in symbols {
            let mut best = 0usize;
            let mut best_d2 = f64::INFINITY;
            for (index, &(pi, pq)) in self.points.iter().enumerate() {
                let d2 = (yi - pi).powi(2) + (yq - pq).powi(2);
                if d2 < best_d2 {
                    best_d2 = d2;
                    best = index;
                }
            }
            out.extend(bits_of(best, self.bits_per_symbol));
        }
        out
    }

    /// Max-log LLRs, flat, `m` per symbol, positive meaning bit 0.
    ///
    /// `noise_var` is the total complex noise variance, either one value for every symbol or
    /// one per symbol — the latter is what per-carrier equalisation produces, and is how a
    /// faded carrier gets discounted instead of trusted.
    ///
    /// # Panics
    /// If a per-symbol `noise_var` is not the same length as `symbols`.
    #[must_use]
    pub fn llr(&self, symbols: &[Complex], noise_var: NoiseVar<'_>) -> Vec<f64> {
        if let NoiseVar::PerSymbol(values) = noise_var {
            assert_eq!(values.len(), symbols.len(), "one noise variance per symbol");
        }
        let m = self.bits_per_symbol;
        let mut out = Vec::with_capacity(symbols.len() * m);
        let mut distances = vec![0.0f64; self.points.len()];
        for (symbol_index, &(yi, yq)) in symbols.iter().enumerate() {
            for (slot, &(pi, pq)) in distances.iter_mut().zip(&self.points) {
                *slot = (yi - pi).powi(2) + (yq - pq).powi(2);
            }
            let nv = match noise_var {
                NoiseVar::Uniform(value) => value,
                NoiseVar::PerSymbol(values) => values[symbol_index],
            };
            for position in 0..m {
                let min_one = self.one_indices[position]
                    .iter()
                    .map(|&i| distances[i])
                    .fold(f64::INFINITY, f64::min);
                let min_zero = self.zero_indices[position]
                    .iter()
                    .map(|&i| distances[i])
                    .fold(f64::INFINITY, f64::min);
                out.push((min_one - min_zero) / nv);
            }
        }
        out
    }

    /// Smallest distance between any two points — the constellation's error resilience.
    #[must_use]
    pub fn min_distance(&self) -> f64 {
        let mut best = f64::INFINITY;
        for (i, &(ai, aq)) in self.points.iter().enumerate() {
            for &(bi, bq) in &self.points[i + 1..] {
                best = best.min(((ai - bi).powi(2) + (aq - bq).powi(2)).sqrt());
            }
        }
        best
    }

    /// Whether every nearest-neighbour pair differs in exactly one bit.
    #[must_use]
    pub fn is_gray(&self) -> bool {
        let min = self.min_distance();
        for (i, &(ai, aq)) in self.points.iter().enumerate() {
            for (j, &(bi, bq)) in self.points.iter().enumerate() {
                if i == j {
                    continue;
                }
                let d = ((ai - bi).powi(2) + (aq - bq).powi(2)).sqrt();
                if (d - min).abs() < 1e-9 && (i ^ j).count_ones() != 1 {
                    return false;
                }
            }
        }
        true
    }

    /// Mean power of the constellation, which every mapping normalises to one.
    #[must_use]
    pub fn mean_power(&self) -> f64 {
        self.points.iter().map(|&(i, q)| i * i + q * q).sum::<f64>() / self.points.len() as f64
    }
}

/// How the demapper is told the noise level.
#[derive(Debug, Clone, Copy)]
pub enum NoiseVar<'a> {
    /// One variance for every symbol.
    Uniform(f64),
    /// One variance per symbol, as per-carrier equalisation produces.
    PerSymbol(&'a [f64]),
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Modulation; 5] = [
        Modulation::Bpsk,
        Modulation::Qpsk,
        Modulation::Psk8,
        Modulation::Qam16,
        Modulation::Qam64,
    ];

    #[test]
    fn every_constellation_has_unit_mean_power() {
        for m in ALL {
            let c = Constellation::new(m);
            assert!(
                (c.mean_power() - 1.0).abs() < 1e-12,
                "{}: {}",
                m.name(),
                c.mean_power()
            );
        }
    }

    #[test]
    fn every_constellation_is_gray_labelled() {
        for m in ALL {
            assert!(
                Constellation::new(m).is_gray(),
                "{} is not Gray-labelled",
                m.name()
            );
        }
    }

    #[test]
    fn mapping_then_hard_decision_round_trips() {
        for m in ALL {
            let c = Constellation::new(m);
            let bits: Vec<u8> = (0..m.bits_per_symbol() * 64)
                .map(|i| u8::from((i * 7) % 5 < 2))
                .collect();
            assert_eq!(c.hard(&c.map(&bits)), bits, "{}", m.name());
        }
    }

    #[test]
    fn llr_sign_agrees_with_the_hard_decision_on_a_clean_symbol() {
        for m in ALL {
            let c = Constellation::new(m);
            let bits: Vec<u8> = (0..m.bits_per_symbol() * 32)
                .map(|i| u8::from((i * 11) % 3 == 0))
                .collect();
            let symbols = c.map(&bits);
            let llrs = c.llr(&symbols, NoiseVar::Uniform(0.01));
            for (llr, bit) in llrs.iter().zip(&bits) {
                // positive LLR means bit 0
                assert_eq!(*llr > 0.0, *bit == 0, "{}", m.name());
            }
        }
    }

    #[test]
    fn bpsk_llr_is_four_re_y_over_sigma_squared() {
        let c = Constellation::new(Modulation::Bpsk);
        let noise = 0.3;
        for &y in &[-1.5, -0.2, 0.0, 0.4, 2.0] {
            let got = c.llr(&[(y, 0.0)], NoiseVar::Uniform(noise))[0];
            assert!((got - 4.0 * y / noise).abs() < 1e-9, "y={y}: {got}");
        }
    }

    #[test]
    fn per_symbol_noise_discounts_the_noisier_symbol() {
        let c = Constellation::new(Modulation::Qpsk);
        let symbols = c.map(&[0, 0, 0, 0]);
        let llrs = c.llr(&symbols, NoiseVar::PerSymbol(&[0.1, 10.0]));
        assert!(llrs[0].abs() > llrs[2].abs() * 10.0, "{llrs:?}");
    }

    #[test]
    fn qpsk_points_are_the_spec_formula() {
        let c = Constellation::new(Modulation::Qpsk);
        let s = 1.0 / 2.0_f64.sqrt();
        assert!((c.points()[0].0 - s).abs() < 1e-12 && (c.points()[0].1 - s).abs() < 1e-12);
        assert!((c.points()[3].0 + s).abs() < 1e-12 && (c.points()[3].1 + s).abs() < 1e-12);
    }
}
