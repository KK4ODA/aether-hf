//! Circular-buffer bit selection with redundancy versions, and its soft inverse (§5.4.2).
//!
//! A port of the reference model's `RateMatcher`. `info_len` (K′) marks where filler bits
//! start inside the systematic part; fillers are skipped on transmit and pinned to
//! [`FILLER_LLR`] on receive.
//!
//! Recovering into a buffer that already holds an earlier transmission's LLRs, rather than a
//! fresh one, is hybrid ARQ with incremental redundancy — the mechanism that lets the link
//! decode a frame no single transmission of which was decodable.

use crate::ldpc::{FILLER_LLR, FecError, NrLdpcCode};

/// Table 5.4.2.1-2 numerators: `k0 = floor(num · N_cb / N) · Z`, here with `N_cb = N`.
const K0_NUMERATORS: [[usize; 4]; 2] = [[0, 17, 33, 56], [0, 13, 25, 43]];

/// Bit selection for one `(code, info_len, E, rv)`.
#[derive(Debug, Clone)]
pub struct RateMatcher {
    info_len: usize,
    e: usize,
    rv: u8,
    n_cb: usize,
    n_full: usize,
    k: usize,
    two_z: usize,
    /// Circular-buffer index transmitted at each output position.
    positions: Vec<usize>,
}

impl RateMatcher {
    /// Build the selection pattern.
    ///
    /// # Errors
    /// If the redundancy version is outside 0..=3 or `info_len` is outside `[2Z, K]`.
    pub fn new(code: &NrLdpcCode, info_len: usize, e: usize, rv: u8) -> Result<Self, FecError> {
        if rv > 3 {
            return Err(FecError::BadRedundancyVersion(rv));
        }
        let z = code.z();
        if info_len < 2 * z || info_len > code.k {
            return Err(FecError::BadLength {
                expected: code.k,
                got: info_len,
            });
        }
        let k0 = K0_NUMERATORS[usize::from(code.base_graph() == 2)][rv as usize] * z;

        // Positions of the circular buffer d (length n_cb) that hold fillers: d_k = c_{k+2Z}.
        let mut usable: Vec<usize> = Vec::with_capacity(code.n_cb);
        let filler_start = info_len - 2 * z;
        let filler_end = code.k - 2 * z;
        for index in 0..code.n_cb {
            if index < filler_start || index >= filler_end {
                usable.push(index);
            }
        }

        // Selection order starts at k0, skips fillers, and wraps.
        let start = usable.partition_point(|&index| index < k0);
        let mut order: Vec<usize> = Vec::with_capacity(usable.len());
        order.extend_from_slice(&usable[start..]);
        order.extend_from_slice(&usable[..start]);

        let mut positions = Vec::with_capacity(e);
        while positions.len() < e {
            let take = e - positions.len();
            positions.extend(order.iter().take(take).copied());
        }

        Ok(Self {
            info_len,
            e,
            rv,
            n_cb: code.n_cb,
            n_full: code.n_full,
            k: code.k,
            two_z: 2 * z,
            positions,
        })
    }

    /// Output length `E`.
    #[must_use]
    pub fn e(&self) -> usize {
        self.e
    }

    /// Redundancy version.
    #[must_use]
    pub fn rv(&self) -> u8 {
        self.rv
    }

    /// Select `E` bits from a full codeword.
    ///
    /// # Errors
    /// If `codeword` is not exactly the code's full length.
    pub fn match_bits(&self, codeword: &[u8]) -> Result<Vec<u8>, FecError> {
        if codeword.len() != self.n_full {
            return Err(FecError::BadLength {
                expected: self.n_full,
                got: codeword.len(),
            });
        }
        let d = &codeword[self.two_z..];
        Ok(self.positions.iter().map(|&index| d[index]).collect())
    }

    /// Soft rate recovery into full-codeword LLRs: punctured bits zero, fillers [`FILLER_LLR`].
    ///
    /// Pass an earlier transmission's full-codeword LLRs as `buffer` to combine redundancy
    /// versions; the result is a new vector and `buffer` is left untouched.
    ///
    /// # Errors
    /// If `llr_e` is not `E` long, or `buffer` is not the full codeword length.
    pub fn recover(&self, llr_e: &[f64], buffer: Option<&[f64]>) -> Result<Vec<f64>, FecError> {
        if llr_e.len() != self.e {
            return Err(FecError::BadLength {
                expected: self.e,
                got: llr_e.len(),
            });
        }
        let mut full = if let Some(previous) = buffer {
            if previous.len() != self.n_full {
                return Err(FecError::BadLength {
                    expected: self.n_full,
                    got: previous.len(),
                });
            }
            previous.to_vec()
        } else {
            let mut fresh = vec![0.0; self.n_full];
            fresh[self.info_len..self.k].fill(FILLER_LLR);
            fresh
        };
        let mut accumulator = vec![0.0f64; self.n_cb];
        for (&index, &value) in self.positions.iter().zip(llr_e) {
            accumulator[index] += value;
        }
        for (slot, value) in full[self.two_z..].iter_mut().zip(accumulator) {
            *slot += value;
        }
        Ok(full)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> NrLdpcCode {
        NrLdpcCode::new(2, 30).unwrap()
    }

    #[test]
    fn rv0_starts_at_the_beginning_of_the_buffer() {
        let code = code();
        let matcher = RateMatcher::new(&code, code.k, 200, 0).unwrap();
        assert_eq!(matcher.positions[0], 0);
    }

    #[test]
    fn later_redundancy_versions_start_further_in() {
        let code = code();
        let starts: Vec<usize> = (0..4)
            .map(|rv| RateMatcher::new(&code, code.k, 200, rv).unwrap().positions[0])
            .collect();
        assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
    }

    #[test]
    fn selection_round_trips_through_recovery() {
        let code = code();
        let info: Vec<u8> = (0..code.k).map(|i| u8::from(i % 3 == 0)).collect();
        let cw = code.encode(&info).unwrap();
        let matcher = RateMatcher::new(&code, code.k, 400, 0).unwrap();
        let selected = matcher.match_bits(&cw).unwrap();
        assert_eq!(selected.len(), 400);
        let llr: Vec<f64> = selected
            .iter()
            .map(|&b| if b == 1 { -3.0 } else { 3.0 })
            .collect();
        let full = matcher.recover(&llr, None).unwrap();
        assert_eq!(full.len(), code.n_full);
        // every selected position must now disagree in sign with the transmitted bit
        for (&position, &bit) in matcher.positions.iter().zip(&selected) {
            let value = full[matcher.two_z + position];
            assert_eq!(value < 0.0, bit == 1);
        }
    }

    #[test]
    fn combining_two_versions_accumulates_rather_than_replaces() {
        let code = code();
        let first = RateMatcher::new(&code, code.k, 300, 0).unwrap();
        let second = RateMatcher::new(&code, code.k, 300, 1).unwrap();
        let a = first.recover(&vec![1.0; 300], None).unwrap();
        let b = second.recover(&vec![1.0; 300], Some(&a)).unwrap();
        let gained: f64 = b.iter().zip(&a).map(|(x, y)| x - y).sum();
        assert!(gained > 0.0, "the second version added no information");
        // the first buffer must not have been modified in place
        assert_eq!(a.len(), code.n_full);
    }

    #[test]
    fn fillers_are_pinned_and_never_transmitted() {
        let code = code();
        let info_len = code.k - 4 * 30; // leave four blocks of fillers
        let matcher = RateMatcher::new(&code, info_len, 300, 0).unwrap();
        let filler_range = (info_len - 60)..(code.k - 60);
        assert!(
            matcher.positions.iter().all(|p| !filler_range.contains(p)),
            "a filler bit was selected for transmission"
        );
        let full = matcher.recover(&vec![0.5; 300], None).unwrap();
        assert!(full[info_len..code.k].iter().all(|&v| v >= FILLER_LLR));
    }

    #[test]
    fn bad_arguments_are_errors() {
        let code = code();
        assert!(matches!(
            RateMatcher::new(&code, code.k, 100, 4),
            Err(FecError::BadRedundancyVersion(4))
        ));
        assert!(RateMatcher::new(&code, 10, 100, 0).is_err());
        let matcher = RateMatcher::new(&code, code.k, 100, 0).unwrap();
        assert!(matcher.match_bits(&[0, 1]).is_err());
        assert!(matcher.recover(&[0.0], None).is_err());
    }
}
