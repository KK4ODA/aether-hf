//! 3GPP TS 38.212 §5.3.2 LDPC: base graphs 1 and 2, systematic encoder, layered decoder.
//!
//! A direct port of the reference model's `fec/nr_ldpc.py`, sharing its base-graph tables
//! (see `build.rs`) and checked against it bit-for-bit on encoding.
//!
//! The parity-check matrix is a base graph of `rows × cols` blocks, each block either zero
//! or a `Z × Z` cyclic permutation `P^s`. Nothing here materialises a matrix: a block row is
//! a list of `(column, shift)` pairs, and applying `P^s` is a rotation.

use crate::tables;

/// One base-graph entry as generated from the specification tables: `(row, column, shifts
/// for each of the eight lifting sets)`.
type BaseGraphEntry = (u16, u16, [u16; 8]);

/// LLR assigned to filler bits (known zeros) at the decoder input.
pub const FILLER_LLR: f64 = 1e3;

/// Table 5.3.2-1: lifting sizes `Z` grouped by set index `i_LS`.
pub const LIFTING_SETS: [&[usize]; 8] = [
    &[2, 4, 8, 16, 32, 64, 128, 256],
    &[3, 6, 12, 24, 48, 96, 192, 384],
    &[5, 10, 20, 40, 80, 160, 320],
    &[7, 14, 28, 56, 112, 224],
    &[9, 18, 36, 72, 144, 288],
    &[11, 22, 44, 88, 176, 352],
    &[13, 26, 52, 104, 208],
    &[15, 30, 60, 120, 240],
];

/// Every valid lifting size, ascending.
#[must_use]
pub fn all_lifting_sizes() -> Vec<usize> {
    let mut sizes: Vec<usize> = LIFTING_SETS
        .iter()
        .flat_map(|s| s.iter().copied())
        .collect();
    sizes.sort_unstable();
    sizes
}

/// Set index `i_LS` of a lifting size, or `None` if it is not one.
#[must_use]
pub fn lifting_set_index(z: usize) -> Option<usize> {
    LIFTING_SETS.iter().position(|set| set.contains(&z))
}

/// `K_b` per TS 38.212 §5.2.2 for a code block of `info_len` bits (including CRC).
#[must_use]
pub fn kb_for(bg: u8, info_len: usize) -> usize {
    if bg == 1 {
        return 22;
    }
    match info_len {
        n if n > 640 => 10,
        n if n > 560 => 9,
        n if n > 192 => 8,
        _ => 6,
    }
}

/// Smallest `Z` with `K_b·Z >= info_len` (§5.2.2).
///
/// # Errors
/// If `info_len` exceeds what the base graph can carry.
pub fn select_lifting_size(bg: u8, info_len: usize) -> Result<usize, FecError> {
    let kb = kb_for(bg, info_len);
    all_lifting_sizes()
        .into_iter()
        .find(|z| kb * z >= info_len)
        .ok_or(FecError::BlockTooLong {
            info_len,
            max: kb * 384,
        })
}

/// Base graph selection per TS 38.212 §7.2.2.
#[must_use]
pub fn select_base_graph(payload_bits: usize, rate: f64) -> u8 {
    if payload_bits <= 292 || (payload_bits <= 3824 && rate <= 0.67) || rate <= 0.25 {
        2
    } else {
        1
    }
}

/// Anything that can go wrong constructing or driving a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FecError {
    /// The base graph number was neither 1 nor 2.
    BadBaseGraph(u8),
    /// The lifting size is not in Table 5.3.2-1.
    BadLiftingSize(usize),
    /// A slice was not the length the code requires.
    BadLength {
        /// What was expected.
        expected: usize,
        /// What arrived.
        got: usize,
    },
    /// More information bits than the base graph can carry.
    BlockTooLong {
        /// Information bits requested.
        info_len: usize,
        /// Most this base graph supports.
        max: usize,
    },
    /// A redundancy version outside 0..=3.
    BadRedundancyVersion(u8),
}

impl core::fmt::Display for FecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadBaseGraph(n) => write!(f, "base graph must be 1 or 2, got {n}"),
            Self::BadLiftingSize(z) => write!(f, "{z} is not a lifting size (Table 5.3.2-1)"),
            Self::BadLength { expected, got } => write!(f, "expected {expected} bits, got {got}"),
            Self::BlockTooLong { info_len, max } => {
                write!(
                    f,
                    "{info_len} information bits exceed the base graph maximum of {max}"
                )
            }
            Self::BadRedundancyVersion(rv) => {
                write!(f, "redundancy version must be 0..3, got {rv}")
            }
        }
    }
}

impl core::error::Error for FecError {}

/// One `(base graph, Z)` instance: systematic encoder, parity check and layered decoder.
#[derive(Debug, Clone)]
pub struct NrLdpcCode {
    bg_number: u8,
    z: usize,
    kb: usize,
    rows: usize,
    /// Information bits `K = K_b·Z`.
    pub k: usize,
    /// Full codeword length `cols·Z`.
    pub n_full: usize,
    /// Circular-buffer length `(cols − 2)·Z`.
    pub n_cb: usize,
    /// Per block row, the `(column, shift)` pairs of its non-zero blocks.
    block_rows: Vec<Vec<(usize, usize)>>,
    /// Effective shift of parity column `K_b` after summing block rows 0..4.
    p1_shift: usize,
}

/// Rotate `P^s · v`: element `r` of the result is `v[(r + s) mod Z]`.
fn rotate_into(dst: &mut [u8], src: &[u8], shift: usize) {
    let z = src.len();
    for (r, slot) in dst.iter_mut().enumerate() {
        *slot ^= src[(r + shift) % z];
    }
}

impl NrLdpcCode {
    /// Build the code for a base graph and lifting size.
    ///
    /// # Errors
    /// If the base graph or lifting size is not valid.
    pub fn new(bg: u8, z: usize) -> Result<Self, FecError> {
        let (rows, cols, entries): (usize, usize, &[BaseGraphEntry]) = match bg {
            1 => (tables::BG1_ROWS, tables::BG1_COLS, &tables::BG1_ENTRIES),
            2 => (tables::BG2_ROWS, tables::BG2_COLS, &tables::BG2_ENTRIES),
            other => return Err(FecError::BadBaseGraph(other)),
        };
        let i_ls = lifting_set_index(z).ok_or(FecError::BadLiftingSize(z))?;
        let kb = if bg == 1 { 22 } else { 10 };

        let mut block_rows: Vec<Vec<(usize, usize)>> = vec![Vec::new(); rows];
        for &(row, col, shifts) in entries {
            block_rows[row as usize].push((col as usize, shifts[i_ls] as usize % z));
        }

        let mut code = Self {
            bg_number: bg,
            z,
            kb,
            rows,
            k: kb * z,
            n_full: cols * z,
            n_cb: (cols - 2) * z,
            block_rows,
            p1_shift: 0,
        };
        code.p1_shift = code.compute_p1_shift();
        Ok(code)
    }

    /// Base graph number, 1 or 2.
    #[must_use]
    pub fn base_graph(&self) -> u8 {
        self.bg_number
    }

    /// Lifting size `Z`.
    #[must_use]
    pub fn z(&self) -> usize {
        self.z
    }

    /// The three entries of column `K_b` in block rows 0..4 have shifts of which two are
    /// equal and cancel over GF(2), leaving a single `P^s`. Find `s` numerically rather than
    /// hard-coding which row carries the odd one, so both base graphs and all eight lifting
    /// sets are handled by the same code.
    fn compute_p1_shift(&self) -> usize {
        let z = self.z;
        let mut unit = vec![0u8; z];
        unit[0] = 1;
        let mut acc = vec![0u8; z];
        for row in &self.block_rows[..4] {
            for &(col, shift) in row {
                if col == self.kb {
                    rotate_into(&mut acc, &unit, shift);
                }
            }
        }
        let position = acc
            .iter()
            .position(|&b| b != 0)
            .expect("summed column K_b must be a single cyclic shift");
        debug_assert_eq!(acc.iter().filter(|&&b| b != 0).count(), 1);
        (z - position) % z
    }

    /// XOR of `P^s · c_j` over the entries of block row `i` whose column satisfies `keep`.
    fn row_sum<F: Fn(usize) -> bool>(&self, blocks: &[u8], row: usize, keep: F) -> Vec<u8> {
        let z = self.z;
        let mut acc = vec![0u8; z];
        for &(col, shift) in &self.block_rows[row] {
            if keep(col) {
                rotate_into(&mut acc, &blocks[col * z..(col + 1) * z], shift);
            }
        }
        acc
    }

    /// Systematic encoding of `K = K_b·Z` information bits (fillers already zero).
    ///
    /// # Errors
    /// If `info` is not exactly `K` bits.
    pub fn encode(&self, info: &[u8]) -> Result<Vec<u8>, FecError> {
        if info.len() != self.k {
            return Err(FecError::BadLength {
                expected: self.k,
                got: info.len(),
            });
        }
        let (z, kb) = (self.z, self.kb);
        let mut cw = vec![0u8; self.n_full];
        cw[..self.k].copy_from_slice(info);

        // p1: sum block rows 0..4 over the information columns, then undo the single shift.
        let mut s0123 = vec![0u8; z];
        for row in 0..4 {
            let partial = self.row_sum(&cw, row, |col| col < kb);
            for (slot, value) in s0123.iter_mut().zip(partial) {
                *slot ^= value;
            }
        }
        // seg[kb] = P^{-s} · s0123, i.e. rotate by the recovered shift
        for r in 0..z {
            cw[kb * z + r] = s0123[(r + z - self.p1_shift % z) % z];
        }

        // Core parities p2..p4 by substitution: with p1 known, each of rows 0..4 introduces
        // exactly one remaining unknown, and that unknown always has shift zero.
        let mut known = vec![false; kb + 4];
        for slot in known.iter_mut().take(kb + 1) {
            *slot = true;
        }
        for _ in 0..3 {
            for row in 0..4 {
                let unknown: Vec<usize> = self.block_rows[row]
                    .iter()
                    .map(|&(col, _)| col)
                    .filter(|&col| col < kb + 4 && !known[col])
                    .collect();
                if unknown.len() == 1 {
                    let target = unknown[0];
                    let value = self.row_sum(&cw, row, |col| col < kb + 4 && known[col]);
                    cw[target * z..(target + 1) * z].copy_from_slice(&value);
                    known[target] = true;
                    break;
                }
            }
        }

        // Extension rows: block row i >= 4 has exactly one identity entry, at column K_b + i.
        let mut known_any = vec![false; self.n_full / z];
        for (col, slot) in known_any.iter_mut().enumerate() {
            *slot = col < kb + 4 && known[col];
        }
        for row in 4..self.rows {
            let target = kb + row;
            let value = self.row_sum(&cw, row, |col| known_any[col]);
            cw[target * z..(target + 1) * z].copy_from_slice(&value);
            known_any[target] = true;
        }
        Ok(cw)
    }

    /// Whether every parity check is satisfied.
    ///
    /// # Errors
    /// If `codeword` is not exactly `n_full` bits.
    pub fn syndrome_ok(&self, codeword: &[u8]) -> Result<bool, FecError> {
        if codeword.len() != self.n_full {
            return Err(FecError::BadLength {
                expected: self.n_full,
                got: codeword.len(),
            });
        }
        Ok((0..self.rows).all(|row| {
            self.row_sum(codeword, row, |_| true)
                .iter()
                .all(|&b| b == 0)
        }))
    }

    /// Layered normalised min-sum decoding of full-codeword LLRs.
    ///
    /// `llr` is `n_full` long: zero for punctured bits, [`FILLER_LLR`] for fillers. Returns
    /// the hard decisions, whether every check was satisfied, and the iterations used.
    ///
    /// # Errors
    /// If `llr` is not exactly `n_full` long.
    pub fn decode(&self, llr: &[f64], max_iter: usize, alpha: f64) -> Result<Decoded, FecError> {
        if llr.len() != self.n_full {
            return Err(FecError::BadLength {
                expected: self.n_full,
                got: llr.len(),
            });
        }
        let z = self.z;
        let mut post = llr.to_vec();
        // check-to-variable messages, one Z-vector per edge
        let mut messages: Vec<Vec<f64>> = self
            .block_rows
            .iter()
            .map(|row| vec![0.0; row.len() * z])
            .collect();

        let mut iterations = max_iter;
        let mut converged = false;
        for iteration in 1..=max_iter {
            for (row_index, row) in self.block_rows.iter().enumerate() {
                let degree = row.len();
                let mut incoming = vec![0.0f64; degree * z];
                for (edge, &(col, shift)) in row.iter().enumerate() {
                    for r in 0..z {
                        let variable = col * z + (r + shift) % z;
                        incoming[edge * z + r] = post[variable] - messages[row_index][edge * z + r];
                    }
                }
                // per check r: smallest and second smallest magnitude, and the sign product
                for r in 0..z {
                    let (mut min1, mut min2, mut argmin, mut sign) =
                        (f64::INFINITY, f64::INFINITY, 0, 1.0f64);
                    for edge in 0..degree {
                        let value = incoming[edge * z + r];
                        let magnitude = value.abs();
                        if value < 0.0 {
                            sign = -sign;
                        }
                        if magnitude < min1 {
                            min2 = min1;
                            min1 = magnitude;
                            argmin = edge;
                        } else if magnitude < min2 {
                            min2 = magnitude;
                        }
                    }
                    for edge in 0..degree {
                        let value = incoming[edge * z + r];
                        let own_sign = if value < 0.0 { -1.0 } else { 1.0 };
                        let magnitude = if edge == argmin { min2 } else { min1 };
                        let c2v = alpha * magnitude * sign * own_sign;
                        messages[row_index][edge * z + r] = c2v;
                        let (col, shift) = row[edge];
                        post[col * z + (r + shift) % z] = value + c2v;
                    }
                }
            }
            let hard: Vec<u8> = post.iter().map(|&v| u8::from(v < 0.0)).collect();
            if self.syndrome_ok(&hard)? {
                iterations = iteration;
                converged = true;
                break;
            }
        }
        let bits = post.iter().map(|&v| u8::from(v < 0.0)).collect();
        Ok(Decoded {
            bits,
            converged,
            iterations,
        })
    }
}

/// Result of [`NrLdpcCode::decode`].
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Hard decisions over the full codeword.
    pub bits: Vec<u8>,
    /// Whether every parity check was satisfied when decoding stopped.
    pub converged: bool,
    /// Iterations actually used.
    pub iterations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifting_sizes_are_the_spec_table() {
        assert_eq!(all_lifting_sizes().len(), 51);
        assert_eq!(lifting_set_index(30), Some(7));
        assert_eq!(lifting_set_index(384), Some(1));
        assert_eq!(lifting_set_index(5), Some(2));
        assert_eq!(lifting_set_index(31), None);
    }

    #[test]
    fn lifting_size_selection_follows_kb() {
        assert_eq!(kb_for(1, 1000), 22);
        assert_eq!(kb_for(2, 700), 10);
        assert_eq!(kb_for(2, 600), 9);
        assert_eq!(kb_for(2, 300), 8);
        assert_eq!(kb_for(2, 100), 6);
        let z = select_lifting_size(2, 232).unwrap();
        assert!(kb_for(2, 232) * z >= 232);
    }

    #[test]
    fn every_encoding_satisfies_its_own_parity_checks() {
        for (bg, z) in [(1u8, 32usize), (2, 30), (2, 52), (1, 176)] {
            let code = NrLdpcCode::new(bg, z).unwrap();
            let info: Vec<u8> = (0..code.k)
                .map(|i| u8::from((i * 7 + bg as usize) % 5 < 2))
                .collect();
            let cw = code.encode(&info).unwrap();
            assert_eq!(cw.len(), code.n_full);
            assert_eq!(
                &cw[..code.k],
                &info[..],
                "encoding must be systematic (bg{bg}, z{z})"
            );
            assert!(code.syndrome_ok(&cw).unwrap(), "bg{bg}, z{z}");
        }
    }

    #[test]
    fn a_flipped_bit_breaks_the_syndrome() {
        let code = NrLdpcCode::new(2, 30).unwrap();
        let mut cw = code.encode(&vec![1u8; code.k]).unwrap();
        assert!(code.syndrome_ok(&cw).unwrap());
        cw[17] ^= 1;
        assert!(!code.syndrome_ok(&cw).unwrap());
    }

    #[test]
    fn decoding_a_clean_codeword_returns_it_unchanged() {
        let code = NrLdpcCode::new(2, 30).unwrap();
        let info: Vec<u8> = (0..code.k).map(|i| u8::from(i % 3 == 0)).collect();
        let cw = code.encode(&info).unwrap();
        let llr: Vec<f64> = cw
            .iter()
            .map(|&b| if b == 1 { -4.0 } else { 4.0 })
            .collect();
        let decoded = code.decode(&llr, 25, 0.8).unwrap();
        assert!(decoded.converged);
        assert_eq!(
            decoded.iterations, 1,
            "a clean codeword needs no correction"
        );
        assert_eq!(decoded.bits, cw);
    }

    #[test]
    fn decoding_corrects_errors() {
        let code = NrLdpcCode::new(2, 52).unwrap();
        let info: Vec<u8> = (0..code.k).map(|i| u8::from((i * 11) % 7 < 3)).collect();
        let cw = code.encode(&info).unwrap();
        let mut llr: Vec<f64> = cw
            .iter()
            .map(|&b| if b == 1 { -2.0 } else { 2.0 })
            .collect();
        for position in (0..llr.len()).step_by(37) {
            llr[position] = -llr[position]; // flip the sign: a hard error
        }
        let decoded = code.decode(&llr, 40, 0.8).unwrap();
        assert!(decoded.converged, "decoder failed to converge");
        assert_eq!(&decoded.bits[..code.k], &info[..]);
    }

    #[test]
    fn wrong_lengths_are_errors_not_panics() {
        let code = NrLdpcCode::new(2, 30).unwrap();
        assert!(matches!(
            code.encode(&[0, 1]),
            Err(FecError::BadLength { .. })
        ));
        assert!(matches!(
            code.syndrome_ok(&[0, 1]),
            Err(FecError::BadLength { .. })
        ));
        assert!(matches!(
            code.decode(&[0.0], 5, 0.8),
            Err(FecError::BadLength { .. })
        ));
        assert!(matches!(
            NrLdpcCode::new(3, 30),
            Err(FecError::BadBaseGraph(3))
        ));
        assert!(matches!(
            NrLdpcCode::new(2, 31),
            Err(FecError::BadLiftingSize(31))
        ));
    }
}
