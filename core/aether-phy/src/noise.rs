//! The noise across the band, read where the signal is (ADR-0104).
//!
//! A radio's receive filter shapes the band's noise as it shapes the signal: a carrier or a
//! tone its skirt cuts by 20 dB has 20 dB less noise too, and keeps its SNR as long as the
//! audio chain's own floor is further down. The receivers took the noise as flat across their
//! band, so a carrier or a tone in the skirt read as one 20 dB under the others: the OFDM
//! decoder all but discarded it, and the tone decoder took a symbol sent on it for the uncut
//! tones' noise. Each receiver now reads the noise per pilot carrier or per tone, and
//! [`shaped_levels`] turns those readings into a shape — dropped where they differ by no more
//! than their own estimates scatter, so the receiver on a band nothing shapes is the flat one
//! it was. The model's `aether_model.phy.noise` is the reference.

/// Noise levels in nepers across the band, one a pilot carrier or a tone, from their raw
/// readings `level`: smoothed over three neighbours (the ends repeated), and pulled toward
/// `centre` by an empirical-Bayes factor, `1 − scatter / var(level)` at least 0, where `scatter`
/// is the variance each reading's own estimate has. Fewer than three readings give `centre`
/// everywhere.
#[must_use]
pub fn shaped_levels(level: &[f64], scatter: f64, centre: f64) -> Vec<f64> {
    let n = level.len();
    if n < 3 {
        return vec![centre; n];
    }
    let mean = level.iter().sum::<f64>() / n as f64;
    let spread = level.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
    let keep = (1.0 - scatter / spread.max(1e-12)).max(0.0);
    (0..n)
        .map(|i| {
            let before = level[i.saturating_sub(1)];
            let after = level[(i + 1).min(n - 1)];
            let smooth = 0.25 * before + 0.5 * level[i] + 0.25 * after;
            centre + keep * (smooth - centre)
        })
        .collect()
}

/// Linear interpolation at `x` through the points `(xs, ys)` (`xs` increasing), flat outside
/// them, as `NumPy`'s `interp`.
#[must_use]
pub fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    let last = xs.len() - 1;
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[last] {
        return ys[last];
    }
    let upper = xs.partition_point(|&v| v <= x);
    let (j, k) = (upper - 1, upper);
    let slope = (ys[k] - ys[j]) / (xs[k] - xs[j]);
    slope * (x - xs[j]) + ys[j]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_that_differ_by_their_own_scatter_are_flat() {
        let level = [0.1, -0.1, 0.05, -0.05, 0.0];
        let out = shaped_levels(&level, 1.0, 0.0);
        assert!(out.iter().all(|&v| v.abs() < 1e-15), "{out:?}");
    }

    #[test]
    fn a_shape_well_above_the_scatter_is_kept_and_smoothed() {
        let level = [0.0, 0.0, 0.0, -4.0, -4.0];
        let out = shaped_levels(&level, 1e-6, -1.6);
        assert!((out[0] - 0.0).abs() < 1e-3, "{out:?}");
        assert!((out[2] + 1.0).abs() < 1e-3, "{out:?}");
        assert!((out[4] + 4.0).abs() < 1e-3, "{out:?}");
    }

    #[test]
    fn interpolation_is_numpys() {
        let xs = [0.0, 4.0, 8.0, 11.0];
        let ys = [1.0, 3.0, 2.0, 5.0];
        for (x, want) in [
            (-1.0, 1.0),
            (4.0, 3.0),
            (6.0, 2.5),
            (10.0, 4.0),
            (12.0, 5.0),
        ] {
            let got = interp(x, &xs, &ys);
            assert!((got - want).abs() < 1e-12, "at {x}: {got} vs {want}");
        }
    }
}
