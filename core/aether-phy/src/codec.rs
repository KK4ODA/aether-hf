//! Payload ↔ constellation-symbol codec for one (mode, layout) pair.
//!
//! ```text
//! bytes ─► bits ─► CRC-24 ─► fillers ─► LDPC ─► rate matching (RV) ─►
//! coprime-stride interleaver ─► Gray-labelled constellation ─► QAM symbols (time-major)
//! ```
//!
//! and the soft inverse, with HARQ-IR combining across redundancy versions.
//!
//! The interleaver is `π(k) = k·p mod E`, with `p` the integer nearest `E/φ` that is coprime
//! with `E`. Consecutive coded bits land about 0.618·E apart in the time-major symbol grid,
//! so a burst in time (a fade) or in frequency (a notch) is scattered over the whole
//! codeword — without the failure mode of a structured interleaver, which is to park a run
//! of coded bits on one carrier.

use aether_fec::{
    CRC24A,
    ldpc::{FecError, NrLdpcCode},
    rate_match::RateMatcher,
};

use crate::{
    constellation::{Complex, Constellation, NoiseVar},
    modes::{FrameLayout, Mode},
};

/// Golden ratio, for the interleaver stride.
const PHI: f64 = 1.618_033_988_749_895;

/// Greatest common divisor.
const fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Stride nearest `E/φ` that is coprime with `E`, searching outward from the ideal.
#[must_use]
pub fn coprime_stride(e: usize) -> usize {
    if e <= 1 {
        return 1;
    }
    let ideal = ((e as f64 / PHI).round() as usize).max(1);
    for delta in 0..e {
        for candidate in [ideal.wrapping_sub(delta), ideal + delta] {
            if candidate >= 1 && candidate < e && gcd(candidate, e) == 1 {
                return candidate;
            }
        }
    }
    1
}

/// Everything needed to turn a payload into symbols and back for one mode and layout.
#[derive(Debug, Clone)]
pub struct FrameCodec {
    mode: Mode,
    layout: FrameLayout,
    /// Payload bytes carried per frame.
    pub payload_bytes: usize,
    /// `K'`, payload plus CRC.
    pub info_bits: usize,
    /// `E`, coded bits after rate matching.
    pub coded_bits: usize,
    code: NrLdpcCode,
    constellation: Constellation,
    /// `permutation[k]` is where coded bit `k` is transmitted.
    permutation: Vec<usize>,
    /// Inverse of `permutation`.
    inverse: Vec<usize>,
    stride: usize,
}

impl FrameCodec {
    /// Build the codec.
    ///
    /// # Errors
    /// If the information block does not fit the selected lifting size.
    pub fn new(mode: Mode, layout: FrameLayout) -> Result<Self, FecError> {
        let payload_bytes = mode.payload_bytes(&layout);
        let info_bits = mode.info_bits(&layout);
        let coded_bits = mode.coded_bits(&layout);
        let code = NrLdpcCode::new(mode.base_graph(&layout), mode.lifting_size(&layout))?;
        if info_bits > code.k {
            return Err(FecError::BadLength {
                expected: code.k,
                got: info_bits,
            });
        }
        let stride = coprime_stride(coded_bits);
        let mut permutation = vec![0usize; coded_bits];
        let mut inverse = vec![0usize; coded_bits];
        for (k, slot) in permutation.iter_mut().enumerate() {
            *slot = (k * stride) % coded_bits;
        }
        for (k, &position) in permutation.iter().enumerate() {
            inverse[position] = k;
        }
        Ok(Self {
            mode,
            layout,
            payload_bytes,
            info_bits,
            coded_bits,
            code,
            constellation: Constellation::new(mode.modulation),
            permutation,
            inverse,
            stride,
        })
    }

    /// The mode this codec serves.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The layout this codec serves.
    #[must_use]
    pub fn layout(&self) -> FrameLayout {
        self.layout
    }

    /// The interleaver stride, for diagnostics and the specification.
    #[must_use]
    pub fn interleaver_stride(&self) -> usize {
        self.stride
    }

    /// Payload bytes → constellation symbols at the given redundancy version.
    ///
    /// # Errors
    /// If the payload is not exactly [`payload_bytes`](Self::payload_bytes) long.
    pub fn encode(&self, payload: &[u8], rv: u8) -> Result<Vec<Complex>, FecError> {
        Ok(self.constellation.map(&self.encode_bits(payload, rv)?))
    }

    /// The interleaved coded bits a payload produces, before they are mapped onto points.
    ///
    /// This is the quantity that has to match the reference model exactly; mapping bits to
    /// constellation points is a separate step with its own test.
    ///
    /// # Errors
    /// If the payload is not exactly [`payload_bytes`](Self::payload_bytes) long.
    pub fn encode_bits(&self, payload: &[u8], rv: u8) -> Result<Vec<u8>, FecError> {
        if payload.len() != self.payload_bytes {
            return Err(FecError::BadLength {
                expected: self.payload_bytes,
                got: payload.len(),
            });
        }
        let mut bits = Vec::with_capacity(payload.len() * 8);
        for &byte in payload {
            for shift in (0..8).rev() {
                bits.push((byte >> shift) & 1);
            }
        }
        let with_crc = CRC24A.attach(&bits);
        debug_assert_eq!(with_crc.len(), self.info_bits);

        let mut info = with_crc;
        info.resize(self.code.k, 0); // fillers
        let codeword = self.code.encode(&info)?;
        let matcher = RateMatcher::new(&self.code, self.info_bits, self.coded_bits, rv)?;
        let selected = matcher.match_bits(&codeword)?;

        let mut interleaved = vec![0u8; self.coded_bits];
        for (k, &bit) in selected.iter().enumerate() {
            interleaved[self.permutation[k]] = bit;
        }
        Ok(interleaved)
    }

    /// Constellation symbols → payload, or `None` if the CRC fails.
    ///
    /// `buffer` carries the full-codeword LLRs of an earlier transmission of the same block;
    /// pass the returned buffer back with the next redundancy version to combine them.
    ///
    /// # Errors
    /// If `symbols` is not the layout's slot count.
    pub fn decode(
        &self,
        symbols: &[Complex],
        noise_var: NoiseVar<'_>,
        rv: u8,
        buffer: Option<&[f64]>,
    ) -> Result<(Option<Vec<u8>>, Vec<f64>), FecError> {
        let expected = self.layout.qam_symbols();
        if symbols.len() != expected {
            return Err(FecError::BadLength {
                expected,
                got: symbols.len(),
            });
        }
        let interleaved = self.constellation.llr(symbols, noise_var);
        let mut llr_e = vec![0.0f64; self.coded_bits];
        for (k, slot) in llr_e.iter_mut().enumerate() {
            *slot = interleaved[self.permutation[k]];
        }
        let matcher = RateMatcher::new(&self.code, self.info_bits, self.coded_bits, rv)?;
        let full = matcher.recover(&llr_e, buffer)?;
        let decoded = self.code.decode(&full, 25, 0.8)?;

        let block = &decoded.bits[..self.info_bits];
        if !CRC24A.check(block) {
            return Ok((None, full));
        }
        // The all-zero word is a codeword of every linear code and its CRC is zero, so a
        // decoder fed noise (a false detection, a frame read at the wrong start) converges to
        // it and "passes". No frame of ours is all zeros — the link layer never assigns
        // session 0 and its other frames have a non-zero kind — so the block is refused.
        if block.iter().all(|&bit| bit == 0) {
            return Ok((None, full));
        }
        let payload_bits = &block[..block.len() - CRC24A.width as usize];
        let payload = payload_bits
            .chunks_exact(8)
            .map(|chunk| chunk.iter().fold(0u8, |acc, &b| (acc << 1) | (b & 1)))
            .collect();
        Ok((Some(payload), full))
    }

    /// Deinterleave order, exposed for tests and diagnostics.
    #[must_use]
    pub fn inverse_permutation(&self) -> &[usize] {
        &self.inverse
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::{CONTROL_MODE, LONG, MODES, SHORT};

    #[test]
    fn the_stride_is_coprime_and_near_the_golden_ratio() {
        for e in [1176usize, 2352, 3528, 4704, 7056, 420] {
            let p = coprime_stride(e);
            assert_eq!(gcd(p, e), 1, "stride {p} shares a factor with {e}");
            let ideal = e as f64 / PHI;
            assert!(
                (p as f64 - ideal).abs() < 16.0,
                "stride {p} is far from {ideal}"
            );
        }
    }

    #[test]
    fn the_interleaver_is_a_permutation() {
        let codec = FrameCodec::new(MODES[4], LONG).unwrap();
        let mut seen = vec![false; codec.coded_bits];
        for &position in &codec.permutation {
            assert!(!seen[position], "the interleaver is not injective");
            seen[position] = true;
        }
        assert!(seen.into_iter().all(|s| s));
        for (k, &position) in codec.permutation.iter().enumerate() {
            assert_eq!(codec.inverse[position], k);
        }
    }

    #[test]
    fn every_mode_round_trips_a_payload() {
        for &m in &MODES {
            let codec = FrameCodec::new(m, LONG).unwrap();
            let payload: Vec<u8> = (0..codec.payload_bytes)
                .map(|i| ((i * 37) % 256) as u8)
                .collect();
            let symbols = codec.encode(&payload, 0).unwrap();
            assert_eq!(symbols.len(), LONG.qam_symbols(), "{}", m.name());
            let (decoded, _) = codec
                .decode(&symbols, NoiseVar::Uniform(0.005), 0, None)
                .unwrap();
            assert_eq!(decoded.as_deref(), Some(&payload[..]), "{}", m.name());
        }
    }

    #[test]
    fn the_control_frame_round_trips_on_the_short_layout() {
        let codec = FrameCodec::new(CONTROL_MODE, SHORT).unwrap();
        let payload = vec![1u8, 2, 3, 4, 5, 6, 7];
        let symbols = codec.encode(&payload, 0).unwrap();
        let (decoded, _) = codec
            .decode(&symbols, NoiseVar::Uniform(0.01), 0, None)
            .unwrap();
        assert_eq!(decoded.as_deref(), Some(&payload[..]));
    }

    #[test]
    fn a_corrupted_frame_reports_failure_rather_than_garbage() {
        let codec = FrameCodec::new(MODES[10], LONG).unwrap();
        let payload: Vec<u8> = (0..codec.payload_bytes).map(|i| (i % 251) as u8).collect();
        let mut symbols = codec.encode(&payload, 0).unwrap();
        for symbol in symbols.iter_mut().step_by(2) {
            *symbol = (-symbol.1, symbol.0); // rotate a quarter turn: heavy damage
        }
        let (decoded, _) = codec
            .decode(&symbols, NoiseVar::Uniform(0.05), 0, None)
            .unwrap();
        assert!(
            decoded.is_none(),
            "the CRC must catch this rather than return wrong bytes"
        );
    }

    #[test]
    fn wrong_payload_length_is_an_error() {
        let codec = FrameCodec::new(MODES[0], LONG).unwrap();
        assert!(matches!(
            codec.encode(&[1, 2, 3], 0),
            Err(FecError::BadLength { .. })
        ));
        assert!(matches!(
            codec.decode(&[(0.0, 0.0)], NoiseVar::Uniform(1.0), 0, None),
            Err(FecError::BadLength { .. })
        ));
    }
}
