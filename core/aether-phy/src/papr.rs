//! Peak reduction for the transmitter (ADR-0004).
//!
//! OFDM adds 57 carriers with independent phases, so its envelope is close to Rayleigh and
//! its peaks run 10–12 dB above the mean. An amateur transmitter cannot pass that: driven to
//! a peak the rig will not clip, the *average* power — which is what carries the link — sits
//! more than 10 dB below what the finals can do. Trading a little distortion for a lower
//! peak-to-average ratio buys real transmit power, and on HF that is the difference between
//! a contact and a retry.
//!
//! # Clip and filter
//!
//! The method is Armstrong's (*Electronics Letters* 38(5), 2002): clip the envelope to a
//! target above the mean, then run the band-limiting filter, which removes the out-of-band
//! splatter clipping created — and, in doing so, restores part of the peak, which is why it
//! takes several passes. The filter is the *same* one the passband chain already applies, so
//! nothing here can emit energy the transmitter would not.
//!
//! # Why two targets
//!
//! Peak reduction is bought with in-band distortion, and how much a mode can absorb depends
//! on whether its constellation carries information in amplitude. This was measured the wrong
//! way round at first — tested at comfortable signal-to-noise ratios it looked as though only
//! 64-QAM cared. Re-measured at each mode's *own* threshold, 16-QAM 3/4 went from 0 % to 45 %
//! frame errors at the 5 dB target. So the split is by constellation, not by speed.

use crate::{
    Complex,
    fir::Fir,
    passband::band_limit_taps,
    waveform::{WIDE_2300, WaveformParams},
};

/// Clip target for the constant-modulus modes: BPSK, QPSK and 8-PSK.
///
/// Measured (`bench/baselines/papr.csv`): EVM −22.8 dB, far more than they need — 8-PSK 1/2
/// decodes 20 out of 20 at its threshold — and worth +1.0 … +1.7 dB of delivered power. These
/// are the modes a weak link actually runs on, so this is where the gain matters most.
pub const CLIP_TARGET_DB: f64 = 5.0;

/// Clip target for the amplitude-modulated constellations: 16-QAM and 64-QAM.
///
/// Their outer points sit where the clipper works hardest. At the 5 dB target, 16-QAM 3/4
/// goes from 0 % to 45 % frame errors at its threshold; at 7 dB (EVM −31.9 dB) it is clean
/// again, and the modes still gain +0.25 … +0.50 dB.
pub const CLIP_TARGET_DENSE_DB: f64 = 7.0;

/// How many clip-and-filter passes to run. Each one undoes part of the previous pass's peak
/// reduction, so the gain per pass falls away; four reaches the target on every mode.
pub const DEFAULT_ITERATIONS: usize = 4;

/// Clip target for a mode.
///
/// PSK rides through clipping; QAM does not, because its outer points are where the clipper
/// works hardest and its decisions depend on amplitude.
#[must_use]
pub fn clip_target_db(bits_per_symbol: usize) -> f64 {
    if bits_per_symbol >= 4 {
        CLIP_TARGET_DENSE_DB
    } else {
        CLIP_TARGET_DB
    }
}

/// Peak-to-average power ratio of a burst, in dB.
#[must_use]
pub fn papr_db(samples: &[Complex]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let power = |&(re, im): &Complex| re.mul_add(re, im * im);
    let mean = samples.iter().map(power).sum::<f64>() / samples.len() as f64;
    if mean <= 0.0 {
        return 0.0;
    }
    let peak = samples.iter().map(power).fold(0.0, f64::max);
    10.0 * (peak / mean).log10()
}

/// In-band error power after removing the best complex gain, in dB.
///
/// A power amplifier compresses, and a pure gain change is not distortion; what is left after
/// the best-fit gain is taken out is what the receiver will see as noise.
///
/// # Panics
/// If the two bursts are different lengths, or the reference carries no power.
#[must_use]
pub fn evm_db(reference: &[Complex], distorted: &[Complex]) -> f64 {
    assert_eq!(reference.len(), distorted.len(), "bursts differ in length");
    // alpha = <ref, got> / <ref, ref>, the least-squares complex gain
    let mut cross = (0.0, 0.0);
    let mut energy = 0.0;
    for (&(rr, ri), &(gr, gi)) in reference.iter().zip(distorted) {
        cross.0 += rr.mul_add(gr, ri * gi);
        cross.1 += rr.mul_add(gi, -(ri * gr));
        energy += rr.mul_add(rr, ri * ri);
    }
    assert!(energy > 0.0, "the reference carries no power");
    let alpha = (cross.0 / energy, cross.1 / energy);

    let mut error = 0.0;
    let mut scaled = 0.0;
    for (&(rr, ri), &(gr, gi)) in reference.iter().zip(distorted) {
        let fitted = (
            alpha.0.mul_add(rr, -(alpha.1 * ri)),
            alpha.0.mul_add(ri, alpha.1 * rr),
        );
        error += (gr - fitted.0).mul_add(gr - fitted.0, (gi - fitted.1) * (gi - fitted.1));
        scaled += fitted.0.mul_add(fitted.0, fitted.1 * fitted.1);
    }
    10.0 * (error / scaled).log10()
}

/// Iterative clip-and-filter peak reduction.
#[derive(Debug, Clone)]
pub struct ClipAndFilter {
    /// Peak-to-average target, in dB.
    pub target_papr_db: f64,
    /// How many clip-then-filter passes to run.
    pub iterations: usize,
    taps: Vec<f64>,
}

impl ClipAndFilter {
    /// Build a peak reducer for a numerology.
    #[must_use]
    pub fn new(params: WaveformParams, target_papr_db: f64) -> Self {
        Self {
            target_papr_db,
            iterations: DEFAULT_ITERATIONS,
            taps: band_limit_taps(&params),
        }
    }

    /// The same reducer with a different pass count.
    #[must_use]
    pub fn with_iterations(mut self, iterations: usize) -> Self {
        self.iterations = iterations;
        self
    }

    /// One pass of the band-limiting filter with its constant group delay taken out.
    ///
    /// This is what a streaming transmitter does — the filter is already in the chain — so
    /// nothing here depends on seeing the whole burst at once.
    fn filter(&self, samples: &[Complex]) -> Vec<Complex> {
        let mut fir = Fir::<Complex>::new(self.taps.clone());
        let delay = fir.delay();
        let mut padded = samples.to_vec();
        padded.extend(std::iter::repeat_n((0.0, 0.0), delay));
        let filtered = fir.process(&padded);
        filtered[delay..].to_vec()
    }

    /// Reduce the peaks of a burst, keeping its average power.
    #[must_use]
    pub fn process(&self, samples: &[Complex]) -> Vec<Complex> {
        if samples.is_empty() {
            return Vec::new();
        }
        let power = |&(re, im): &Complex| re.mul_add(re, im * im);
        let mean_power = samples.iter().map(power).sum::<f64>() / samples.len() as f64;
        if mean_power <= 0.0 {
            return samples.to_vec();
        }
        let threshold = (mean_power * 10.0f64.powf(self.target_papr_db / 10.0)).sqrt();

        let mut out = samples.to_vec();
        for _ in 0..self.iterations {
            for sample in &mut out {
                let magnitude = sample.0.hypot(sample.1);
                if magnitude > threshold {
                    let scale = threshold / magnitude;
                    *sample = (sample.0 * scale, sample.1 * scale);
                }
            }
            out = self.filter(&out);
        }
        // restore the average power, so what changed is the peak and nothing else
        let after = out.iter().map(power).sum::<f64>() / out.len() as f64;
        let scale = (mean_power / after.max(1e-30)).sqrt();
        for sample in &mut out {
            *sample = (sample.0 * scale, sample.1 * scale);
        }
        out
    }
}

impl Default for ClipAndFilter {
    fn default() -> Self {
        Self::new(WIDE_2300, CLIP_TARGET_DB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{modes::LONG, ofdm::OfdmModulator, waveform::WIDE_2300};

    /// A burst of OFDM symbols with deterministic pseudo-random carrier values, which is
    /// close enough to Gaussian for its envelope to behave like the real thing.
    fn ofdm_burst(symbols: usize) -> Vec<Complex> {
        let modulator = OfdmModulator::new(WIDE_2300);
        let n_data = WIDE_2300.n_data_carriers();
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut values: Vec<Vec<Complex>> = Vec::with_capacity(symbols);
        for _ in 0..symbols {
            let mut data: Vec<Complex> = Vec::with_capacity(n_data);
            for _ in 0..n_data {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                let unit =
                    (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
                let angle = 2.0 * std::f64::consts::PI * unit;
                data.push((angle.cos(), angle.sin()));
            }
            values.push(modulator.data_symbol(&data));
        }
        modulator.modulate(&values)
    }

    #[test]
    fn the_targets_are_the_ones_adr_0004_settled_on() {
        assert!((clip_target_db(1) - CLIP_TARGET_DB).abs() < 1e-12, "BPSK");
        assert!((clip_target_db(2) - CLIP_TARGET_DB).abs() < 1e-12, "QPSK");
        assert!((clip_target_db(3) - CLIP_TARGET_DB).abs() < 1e-12, "8-PSK");
        assert!(
            (clip_target_db(4) - CLIP_TARGET_DENSE_DB).abs() < 1e-12,
            "16-QAM"
        );
        assert!(
            (clip_target_db(6) - CLIP_TARGET_DENSE_DB).abs() < 1e-12,
            "64-QAM"
        );
    }

    #[test]
    fn raw_ofdm_really_does_have_the_peaks_this_exists_to_remove() {
        let burst = ofdm_burst(LONG.data_symbols);
        let raw = papr_db(&burst);
        assert!(
            raw > 9.0,
            "the test signal is not representative: {raw:.1} dB"
        );
    }

    #[test]
    fn clipping_reaches_its_target() {
        let burst = ofdm_burst(LONG.data_symbols);
        for target in [CLIP_TARGET_DB, CLIP_TARGET_DENSE_DB] {
            let reduced = ClipAndFilter::new(WIDE_2300, target).process(&burst);
            let got = papr_db(&reduced);
            // the filter restores part of the peak on the last pass, so the target is
            // approached rather than met exactly; a dB of slack is what four passes leave
            assert!(
                got < target + 1.0,
                "target {target} dB, reached {got:.2} dB from {:.2} dB",
                papr_db(&burst)
            );
            assert!(got < papr_db(&burst) - 2.0, "it barely did anything");
        }
    }

    #[test]
    fn average_power_is_preserved() {
        // otherwise a comparison between clipped and unclipped is measuring the gain change
        let burst = ofdm_burst(8);
        let reduced = ClipAndFilter::default().process(&burst);
        let mean = |x: &[Complex]| {
            x.iter().map(|&(r, i)| r.mul_add(r, i * i)).sum::<f64>() / x.len() as f64
        };
        let (before, after) = (mean(&burst), mean(&reduced));
        assert!(
            (after / before - 1.0).abs() < 1e-9,
            "power moved by {:.3} dB",
            10.0 * (after / before).log10()
        );
    }

    #[test]
    fn the_distortion_it_costs_is_the_measured_amount() {
        // The number that decided ADR-0004: what a mode has to absorb in exchange for the
        // peak reduction. If this moves, the mode thresholds move with it.
        let burst = ofdm_burst(LONG.data_symbols);
        let five = ClipAndFilter::new(WIDE_2300, CLIP_TARGET_DB).process(&burst);
        let seven = ClipAndFilter::new(WIDE_2300, CLIP_TARGET_DENSE_DB).process(&burst);
        let (evm_five, evm_seven) = (evm_db(&burst, &five), evm_db(&burst, &seven));
        assert!(
            evm_five < -18.0,
            "5 dB target costs {evm_five:.1} dB of EVM, more than the modes were measured at"
        );
        assert!(
            evm_seven < evm_five,
            "the gentler target should distort less: {evm_seven:.1} vs {evm_five:.1}"
        );
    }

    #[test]
    fn an_empty_or_silent_burst_is_left_alone() {
        assert!(ClipAndFilter::default().process(&[]).is_empty());
        let silence = vec![(0.0, 0.0); 100];
        assert_eq!(ClipAndFilter::default().process(&silence), silence);
        assert!((papr_db(&silence) - 0.0).abs() < 1e-12);
    }

    #[test]
    fn evm_ignores_a_pure_gain_change() {
        // a compressing amplifier changes the gain; that is not distortion and must not be
        // counted as any
        let burst = ofdm_burst(4);
        let louder: Vec<Complex> = burst.iter().map(|&(r, i)| (0.5 * r, 0.5 * i)).collect();
        assert!(
            evm_db(&burst, &louder) < -200.0,
            "a gain change was counted as distortion: {}",
            evm_db(&burst, &louder)
        );
    }
}
