//! Window-method FIR design and a streaming filter.
//!
//! The model designs its filters with `scipy.signal.firwin` and a Kaiser window; this is the
//! same construction, spelled out. There is no Rust dependency here that would be the
//! obvious equivalent of `SciPy`, and the arithmetic is short enough that a transcription is
//! easier to check against the model than a third-party design routine would be — the
//! vectors in `tests/model_vectors.rs` compare tap for tap.
//!
//! Both the design and the filter are deliberately plain: a symmetric windowed sinc, and a
//! direct-form filter that carries its state between blocks so any block size gives the same
//! output as one long call.

/// Modified Bessel function of the first kind, order zero.
///
/// The ascending series, which for the arguments a Kaiser window produces (at most `beta`,
/// about 6.8 for 70 dB of attenuation) reaches double precision well inside the loop bound.
#[must_use]
pub fn bessel_i0(x: f64) -> f64 {
    let half_squared = 0.25 * x * x;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..64 {
        term *= half_squared / (f64::from(k) * f64::from(k));
        sum += term;
        if term < 1e-18 * sum {
            break;
        }
    }
    sum
}

/// Kaiser shape parameter for a given stopband attenuation in dB (Kaiser's own formula, as
/// `scipy.signal.kaiser_beta` states it).
#[must_use]
pub fn kaiser_beta(attenuation_db: f64) -> f64 {
    if attenuation_db > 50.0 {
        0.1102 * (attenuation_db - 8.7)
    } else if attenuation_db > 21.0 {
        0.5842f64.mul_add(
            (attenuation_db - 21.0).powf(0.4),
            0.07886 * (attenuation_db - 21.0),
        )
    } else {
        0.0
    }
}

/// Kaiser window of `n` points.
#[must_use]
pub fn kaiser_window(n: usize, beta: f64) -> Vec<f64> {
    if n == 1 {
        return vec![1.0];
    }
    let alpha = (n - 1) as f64 / 2.0;
    let denominator = bessel_i0(beta);
    (0..n)
        .map(|i| {
            let ratio = (i as f64 - alpha) / alpha;
            bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / denominator
        })
        .collect()
}

/// Normalised sinc, `sin(pi x) / (pi x)`, as `NumPy` defines it.
#[must_use]
pub fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let pi_x = std::f64::consts::PI * x;
        pi_x.sin() / pi_x
    }
}

/// A linear-phase low-pass by the window method: `numtaps` taps, `-6 dB` at `cutoff_hz`,
/// Kaiser-windowed, scaled for unity gain at DC.
///
/// # Panics
/// If `numtaps` is even. An even-length filter has a half-sample group delay, and every
/// caller here relies on the delay being an exact number of samples.
#[must_use]
pub fn firwin_lowpass(numtaps: usize, cutoff_hz: f64, fs: f64, beta: f64) -> Vec<f64> {
    assert!(
        numtaps % 2 == 1,
        "an even-length filter has a half-sample delay"
    );
    let normalised = 2.0 * cutoff_hz / fs; // cutoff relative to Nyquist, as scipy takes it
    let alpha = (numtaps - 1) as f64 / 2.0;
    let window = kaiser_window(numtaps, beta);
    let mut taps: Vec<f64> = (0..numtaps)
        .map(|i| {
            let m = i as f64 - alpha;
            normalised * sinc(normalised * m) * window[i]
        })
        .collect();
    let gain: f64 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= gain;
    }
    taps
}

/// Number of taps for a transition band of `transition_hz`, at `attenuation_db` stopband
/// attenuation — Kaiser's length estimate, rounded up to an odd count.
#[must_use]
pub fn kaiser_numtaps(attenuation_db: f64, transition_hz: f64, fs: f64) -> usize {
    let normalised = 2.0 * std::f64::consts::PI * transition_hz / fs;
    let estimate = ((attenuation_db - 8.0) / (2.285 * normalised)).ceil();
    let n = estimate.max(1.0) as usize;
    n | 1
}

/// What [`Fir`] can filter: real samples, or complex ones as `(re, im)`.
///
/// A trait rather than the arithmetic operators, because [`Complex`](crate::Complex) is a
/// tuple and tuples carry no arithmetic of their own.
pub trait Sample: Copy {
    /// The additive identity.
    #[must_use]
    fn zero() -> Self;
    /// Sum of two samples.
    #[must_use]
    fn add(self, other: Self) -> Self;
    /// Sample scaled by a real factor.
    #[must_use]
    fn scale(self, k: f64) -> Self;
}

impl Sample for f64 {
    fn zero() -> Self {
        0.0
    }

    fn add(self, other: Self) -> Self {
        self + other
    }

    fn scale(self, k: f64) -> Self {
        self * k
    }
}

impl Sample for (f64, f64) {
    fn zero() -> Self {
        (0.0, 0.0)
    }

    fn add(self, other: Self) -> Self {
        (self.0 + other.0, self.1 + other.1)
    }

    fn scale(self, k: f64) -> Self {
        (self.0 * k, self.1 * k)
    }
}

/// A direct-form FIR that keeps its state between calls.
///
/// The state is the tail of the previous input, so feeding a signal in blocks of any size
/// gives exactly the same output as feeding it in one call.
#[derive(Debug, Clone)]
pub struct Fir<T> {
    taps: Vec<f64>,
    history: Vec<T>,
}

impl<T: Sample> Fir<T> {
    /// Build a filter from its taps.
    ///
    /// # Panics
    /// If `taps` is empty.
    #[must_use]
    pub fn new(taps: Vec<f64>) -> Self {
        assert!(!taps.is_empty(), "a filter needs at least one tap");
        let history = vec![T::zero(); taps.len() - 1];
        Self { taps, history }
    }

    /// Group delay in samples. Only meaningful for the symmetric filters designed here.
    #[must_use]
    pub fn delay(&self) -> usize {
        (self.taps.len() - 1) / 2
    }

    /// The taps.
    #[must_use]
    pub fn taps(&self) -> &[f64] {
        &self.taps
    }

    /// Filter a block, carrying state across the boundary.
    pub fn process(&mut self, block: &[T]) -> Vec<T> {
        if block.is_empty() {
            return Vec::new();
        }
        let tail = self.history.len();
        // one contiguous buffer of [previous tail | this block], so the convolution never
        // has to special-case the join
        let mut buffer = Vec::with_capacity(tail + block.len());
        buffer.extend_from_slice(&self.history);
        buffer.extend_from_slice(block);
        let out: Vec<T> = (0..block.len())
            .map(|n| {
                let mut acc = T::zero();
                for (k, &tap) in self.taps.iter().enumerate() {
                    acc = acc.add(buffer[tail + n - k].scale(tap));
                }
                acc
            })
            .collect();
        self.history.copy_from_slice(&buffer[buffer.len() - tail..]);
        out
    }

    /// Forget the state, as if nothing had been filtered yet.
    pub fn reset(&mut self) {
        self.history.fill(T::zero());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bessel_matches_known_values() {
        // I0(0) = 1, and a few tabulated points
        assert!((bessel_i0(0.0) - 1.0).abs() < 1e-15);
        assert!((bessel_i0(1.0) - 1.266_065_877_752_008_3).abs() < 1e-12);
        assert!((bessel_i0(5.0) - 27.239_871_823_604_44).abs() < 1e-10);
    }

    #[test]
    fn kaiser_beta_matches_the_seventy_db_case() {
        // the value the model's filters are designed at
        assert!((kaiser_beta(70.0) - 0.1102 * 61.3).abs() < 1e-12);
        assert!(
            kaiser_beta(10.0) == 0.0,
            "below 21 dB the window is rectangular"
        );
    }

    #[test]
    fn a_lowpass_passes_dc_and_stops_the_stopband() {
        let fs = 8000.0;
        let taps = firwin_lowpass(101, 1000.0, fs, kaiser_beta(70.0));
        let response = |f: f64| {
            let alpha = (taps.len() - 1) as f64 / 2.0;
            let (mut re, mut im) = (0.0, 0.0);
            for (n, &tap) in taps.iter().enumerate() {
                let phase = -2.0 * std::f64::consts::PI * f * (n as f64 - alpha) / fs;
                re += tap * phase.cos();
                im += tap * phase.sin();
            }
            re.hypot(im)
        };
        assert!((response(0.0) - 1.0).abs() < 1e-12, "unity at DC");
        assert!(response(500.0) > 0.99, "flat in the passband");
        assert!(
            (response(1000.0) - 0.5).abs() < 0.05,
            "half amplitude at cutoff"
        );
        assert!(response(2000.0) < 1e-3, "70 dB down in the stopband");
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let taps = firwin_lowpass(65, 1000.0, 8000.0, kaiser_beta(70.0));
        let signal: Vec<f64> = (0..1000).map(|i| (0.01 * f64::from(i)).sin()).collect();

        let mut whole = Fir::<f64>::new(taps.clone());
        let reference = whole.process(&signal);

        for chunk in [1usize, 7, 64, 333] {
            let mut streaming = Fir::<f64>::new(taps.clone());
            let mut got = Vec::new();
            for block in signal.chunks(chunk) {
                got.extend(streaming.process(block));
            }
            assert_eq!(got.len(), reference.len());
            for (a, b) in got.iter().zip(&reference) {
                assert!(
                    (a - b).abs() < 1e-12,
                    "block size {chunk} changed the output"
                );
            }
        }
    }

    #[test]
    fn an_impulse_returns_the_taps() {
        let taps = firwin_lowpass(21, 1000.0, 8000.0, kaiser_beta(70.0));
        let mut fir = Fir::<f64>::new(taps.clone());
        let mut impulse = vec![0.0; 40];
        impulse[0] = 1.0;
        let out = fir.process(&impulse);
        for (n, &tap) in taps.iter().enumerate() {
            assert!((out[n] - tap).abs() < 1e-15);
        }
    }
}
