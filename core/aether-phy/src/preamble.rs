//! Frame preamble and in-frame signalling.
//!
//! The preamble is two identical Schmidl–Cox symbols: a PN sequence on the *even* carriers
//! only, odd carriers zero, so the useful part of each symbol is two identical halves and the
//! two symbols repeat one another. A receiver finds the frame and estimates carrier offset
//! from that structure alone (Schmidl & Cox, *IEEE Trans. Commun.*, 1997).
//!
//! **The frame type is the choice of sequence.** DATA and CONTROL use different ones, so the
//! type decision carries the full processing gain of the preamble instead of depending on a
//! header symbol that would be unreadable where the robust modes operate.
//!
//! **Mode and redundancy version ride on the pilot symbols.** A DATA frame puts ±1 chips —
//! one sequence per (RV, mode) pair — on the data carriers of its full pilot symbols. The
//! comb pilots there stay known, so a receiver estimates the channel from them, correlates
//! the chips, and only then treats those symbols as fully known. Carrying the RV outside the
//! codeword is what lets a receiver soft-combine a retransmission with a first transmission
//! whose sequence number it never decoded.
//!
//! Zadoff–Chu is deliberately not used in the preamble: a ZC chirp shifted in frequency is,
//! up to phase, the same chirp shifted in time, so a matched filter could not tell a carrier
//! offset from a timing error. A PN sequence decorrelates under either.
//!
//! The sequences themselves are air-interface constants compiled in from the reference model
//! (see `build.rs`) rather than regenerated here.

use crate::{
    constellation::Complex,
    ofdm::CarrierMap,
    tables,
    waveform::{WIDE_2300, WaveformParams},
};

/// Redundancy versions signalled per frame.
pub const N_RV: usize = tables::N_RV;
/// Modes signalled per frame.
pub const N_MODES: usize = tables::N_MODES;
/// Chips carried across the full pilot symbols of a frame.
pub const CHIP_LENGTH: usize = tables::CHIP_LENGTH;

/// Which container a frame is, signalled by the preamble sequence itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// Carries user data and the connection handshake.
    Data,
    /// Carries acknowledgements and other control frames.
    Control,
}

/// What the preamble and pilot chips of a frame announce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Container type.
    pub frame_type: FrameType,
    /// Mode index, for DATA frames.
    pub mode: usize,
    /// Redundancy version, for DATA frames.
    pub rv: u8,
}

impl FrameHeader {
    /// Build a header, checking the mode and redundancy version are in range.
    ///
    /// # Errors
    /// If the mode or redundancy version is out of range.
    pub fn new(frame_type: FrameType, mode: usize, rv: u8) -> Result<Self, HeaderError> {
        if mode >= N_MODES {
            return Err(HeaderError::BadMode(mode));
        }
        if usize::from(rv) >= N_RV {
            return Err(HeaderError::BadRedundancyVersion(rv));
        }
        Ok(Self {
            frame_type,
            mode,
            rv,
        })
    }

    /// A control frame, which always uses the control mode and RV 0.
    #[must_use]
    pub const fn control() -> Self {
        Self {
            frame_type: FrameType::Control,
            mode: 0,
            rv: 0,
        }
    }
}

/// A header that cannot be signalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// Mode index outside the table.
    BadMode(usize),
    /// Redundancy version outside the signalled range.
    BadRedundancyVersion(u8),
}

impl core::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMode(m) => write!(f, "mode index must be 0..{N_MODES}, got {m}"),
            Self::BadRedundancyVersion(rv) => {
                write!(f, "redundancy version must be 0..{N_RV}, got {rv}")
            }
        }
    }
}

impl core::error::Error for HeaderError {}

/// Index of the chip sequence carrying `(mode, rv)`.
///
/// RV 0 occupies the first `N_MODES` sequences, so RV-0 frames are unchanged from the design
/// before redundancy versions were signalled — which is why the golden vectors still hold.
#[must_use]
pub const fn chip_index(mode: usize, rv: u8) -> usize {
    rv as usize * N_MODES + mode
}

/// Inverse of [`chip_index`].
#[must_use]
pub const fn chip_hypothesis(index: usize) -> (usize, u8) {
    (index % N_MODES, (index / N_MODES) as u8)
}

/// The preamble sequences for one numerology.
#[derive(Debug, Clone)]
pub struct Preamble {
    n_carriers: usize,
    even: Vec<usize>,
    scale: f64,
    n_data_carriers: usize,
}

impl Preamble {
    /// Build it.
    ///
    /// # Panics
    /// If the carriers this numerology calls even are not the ones the exported sequences
    /// were generated for. That would mean the two implementations disagree about the carrier
    /// map, and every preamble this produced would be undetectable by the other — worth
    /// failing loudly at construction rather than silently on the air.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let map = CarrierMap::new(params);
        let even: Vec<usize> = (0..map.n_carriers())
            .filter(|&c| map.bins()[c].rem_euclid(2) == 0)
            .collect();
        assert_eq!(
            even.len(),
            tables::SC_LENGTH,
            "the exported Schmidl-Cox sequence does not fit this carrier map"
        );
        assert!(
            even == tables::EVEN_CARRIERS,
            "this carrier map disagrees with the one the sequences were generated for"
        );
        let scale = (map.n_carriers() as f64 / even.len() as f64).sqrt();
        Self {
            n_carriers: map.n_carriers(),
            even,
            scale,
            n_data_carriers: map.data_carriers().len(),
        }
    }

    /// Carrier indices the Schmidl–Cox sequence occupies.
    #[must_use]
    pub fn even_carriers(&self) -> &[usize] {
        &self.even
    }

    /// Chips carried across all the full pilot symbols of a frame.
    #[must_use]
    pub const fn n_chips(&self) -> usize {
        CHIP_LENGTH
    }

    /// Carrier values of each Schmidl–Cox symbol for a frame type: unit mean power over the
    /// active carriers, zero on the odd ones.
    #[must_use]
    pub fn sc_values(&self, frame_type: FrameType) -> Vec<Complex> {
        let signs: &[f64] = match frame_type {
            FrameType::Data => &tables::SC_DATA,
            FrameType::Control => &tables::SC_CONTROL,
        };
        let mut out = vec![(0.0, 0.0); self.n_carriers];
        for (&carrier, &sign) in self.even.iter().zip(signs) {
            out[carrier] = (sign * self.scale, 0.0);
        }
        out
    }

    /// The two preamble symbols of a frame — identical by construction.
    #[must_use]
    pub fn symbols(&self, header: &FrameHeader) -> Vec<Vec<Complex>> {
        let values = self.sc_values(header.frame_type);
        vec![values.clone(), values]
    }

    /// Chips for the data carriers of one full pilot symbol of a DATA frame.
    ///
    /// # Panics
    /// If the pilot symbol index runs past the chip sequence.
    #[must_use]
    pub fn mode_chips(&self, mode: usize, pilot_symbol_index: usize, rv: u8) -> Vec<f64> {
        let sequence = &tables::MODE_CHIPS[chip_index(mode, rv)];
        let start = pilot_symbol_index * self.n_data_carriers;
        assert!(
            start + self.n_data_carriers <= sequence.len(),
            "more pilot symbols than the chip sequence covers"
        );
        sequence[start..start + self.n_data_carriers].to_vec()
    }
}

impl Default for Preamble {
    fn default() -> Self {
        Self::new(WIDE_2300)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_frame_types_use_well_separated_sequences() {
        let pre = Preamble::default();
        let a = pre.sc_values(FrameType::Data);
        let b = pre.sc_values(FrameType::Control);
        let dot: f64 = a.iter().zip(&b).map(|(x, y)| x.0 * y.0 + x.1 * y.1).sum();
        let energy: f64 = a.iter().map(|x| x.0 * x.0 + x.1 * x.1).sum();
        assert!((dot / energy).abs() < 0.3, "correlation {}", dot / energy);
    }

    #[test]
    fn the_preamble_sits_only_on_even_carriers() {
        let pre = Preamble::default();
        let values = pre.sc_values(FrameType::Data);
        let map = CarrierMap::new(WIDE_2300);
        for (carrier, value) in values.iter().enumerate() {
            let occupied = value.0.abs() > 0.0 || value.1.abs() > 0.0;
            assert_eq!(
                occupied,
                map.bins()[carrier].rem_euclid(2) == 0,
                "carrier {carrier}"
            );
        }
    }

    #[test]
    fn the_preamble_carries_unit_mean_power() {
        let pre = Preamble::default();
        let values = pre.sc_values(FrameType::Data);
        let total: f64 = values.iter().map(|v| v.0 * v.0 + v.1 * v.1).sum();
        assert!((total / values.len() as f64 - 1.0).abs() < 1e-12);
    }

    #[test]
    fn both_preamble_symbols_are_identical() {
        let pre = Preamble::default();
        let header = FrameHeader::new(FrameType::Data, 4, 1).unwrap();
        let symbols = pre.symbols(&header);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0], symbols[1]);
    }

    #[test]
    fn every_chip_sequence_is_unit_magnitude_and_well_separated() {
        let pre = Preamble::default();
        let sequences: Vec<Vec<f64>> = (0..N_RV)
            .flat_map(|rv| {
                (0..N_MODES).map(move |mode| {
                    let pre = Preamble::default();
                    (0..4)
                        .flat_map(|s| pre.mode_chips(mode, s, rv as u8))
                        .collect::<Vec<f64>>()
                })
            })
            .collect();
        assert_eq!(sequences.len(), N_RV * N_MODES);
        for sequence in &sequences {
            assert_eq!(sequence.len(), CHIP_LENGTH);
            assert!(sequence.iter().all(|&c| (c.abs() - 1.0).abs() < 1e-12));
        }
        let mut worst: f64 = 0.0;
        for (i, a) in sequences.iter().enumerate() {
            for b in &sequences[i + 1..] {
                let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
                worst = worst.max((dot / CHIP_LENGTH as f64).abs());
            }
        }
        assert!(worst <= 0.2, "worst pairwise correlation {worst}");
        let _ = pre.n_chips();
    }

    #[test]
    fn chip_index_and_hypothesis_are_inverses() {
        for rv in 0..N_RV as u8 {
            for mode in 0..N_MODES {
                assert_eq!(chip_hypothesis(chip_index(mode, rv)), (mode, rv));
            }
        }
        // RV 0 occupies the first N_MODES slots, keeping earlier frames unchanged
        assert_eq!(chip_index(5, 0), 5);
    }

    #[test]
    fn out_of_range_headers_are_rejected() {
        assert!(FrameHeader::new(FrameType::Data, N_MODES, 0).is_err());
        assert!(FrameHeader::new(FrameType::Data, 0, N_RV as u8).is_err());
        assert!(FrameHeader::new(FrameType::Data, N_MODES - 1, N_RV as u8 - 1).is_ok());
        assert_eq!(FrameHeader::control().frame_type, FrameType::Control);
    }
}
