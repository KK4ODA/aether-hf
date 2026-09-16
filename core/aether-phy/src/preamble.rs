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
    modes::{FrameLayout, air_interface},
    ofdm::CarrierMap,
    tables,
    waveform::{WIDE_2300, WaveformParams},
};

/// Largest cosine allowed between any two preamble sequences of one waveform. No length-6
/// sequence is orthogonal to both ordinary ones (there are only 64), so the floor family's
/// twelve-carrier sequences (ADR-0009) sit at 0.236 against them at 500 Hz.
pub const SC_SEPARATION: f64 = 0.3;

/// Redundancy versions signalled per frame.
pub const N_RV: usize = tables::N_RV;
/// Most modes any air interface may signal: the wide table's fourteen. The narrow table
/// has ten, and each air interface's chip set is indexed by its own count
/// ([`Preamble::chip_index`]).
pub const N_MODES: usize = 14;
/// Chips carried across the full pilot symbols of a wide frame; a preamble knows its own
/// ([`Preamble::n_chips`]).
pub const CHIP_LENGTH: usize = 168;

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

/// Index of the chip sequence carrying `(mode, rv)` in a table of `n_modes` modes.
///
/// RV 0 occupies the first `n_modes` sequences, so RV-0 frames are unchanged from the design
/// before redundancy versions were signalled — which is why the golden vectors still hold.
#[must_use]
pub const fn chip_index(mode: usize, rv: u8, n_modes: usize) -> usize {
    rv as usize * n_modes + mode
}

/// Inverse of [`chip_index`].
#[must_use]
pub const fn chip_hypothesis(index: usize, n_modes: usize) -> (usize, u8) {
    (index % n_modes, (index / n_modes) as u8)
}

/// The preamble sequences for one numerology.
#[derive(Debug, Clone)]
pub struct Preamble {
    n_carriers: usize,
    even: Vec<usize>,
    scale: f64,
    n_data_carriers: usize,
    n_modes: usize,
    tables: &'static tables::Tables,
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
        let air = air_interface(params);
        let tables = tables::for_bandwidth(params.bandwidth.hz())
            .expect("every air interface has its tables exported");
        let map = CarrierMap::new(params);
        let even: Vec<usize> = (0..map.n_carriers())
            .filter(|&c| map.bins()[c].rem_euclid(2) == 0)
            .collect();
        assert_eq!(
            even.len(),
            tables.sc_length,
            "the exported Schmidl-Cox sequence does not fit this carrier map"
        );
        assert!(
            even == tables.even_carriers,
            "this carrier map disagrees with the one the sequences were generated for"
        );
        assert_eq!(
            tables.n_modes,
            air.n_modes(),
            "the chip set covers the mode table"
        );
        // the model exports the bound and the threshold with the sequences; the air
        // interface here states them too, and the two must not drift apart
        assert!(
            (tables.chip_correlation_bound - air.chip_correlation_bound).abs() < 1e-12
                && (tables.acquisition_threshold - air.acquisition_threshold).abs() < 1e-12,
            "the air interface's bound or threshold differs from the model's export"
        );
        // the same for the floor family (ADR-0009), present or absent on both sides
        assert!(
            tables.floor_modes == air.floor_modes
                && tables.control_mode_index == air.control_mode_index
                && (tables.floor_acquisition_threshold - air.floor_acquisition_threshold).abs()
                    < 1e-12
                && (tables.floor_sc_length > 0) == air.floor_long.is_some(),
            "the air interface's floor family differs from the model's export"
        );
        if let Some(floor_long) = air.floor_long {
            assert_eq!(
                tables.floor_sc_length,
                map.n_carriers(),
                "a floor preamble sequence covers every carrier"
            );
            assert_eq!(
                tables.floor_chip_length,
                floor_long.n_pilot_symbols() * map.data_carriers().len(),
                "the floor chips cover every full pilot symbol of the floor data frame"
            );
        }
        let scale = (map.n_carriers() as f64 / even.len() as f64).sqrt();
        let this = Self {
            n_carriers: map.n_carriers(),
            even,
            scale,
            n_data_carriers: map.data_carriers().len(),
            n_modes: air.n_modes(),
            tables,
        };
        // every pair of preamble sequences of this waveform must be well separated
        let mut sequences = vec![
            this.sc_values_of(FrameType::Data, false),
            this.sc_values_of(FrameType::Control, false),
        ];
        if air.floor_long.is_some() {
            sequences.push(this.sc_values_of(FrameType::Data, true));
            sequences.push(this.sc_values_of(FrameType::Control, true));
        }
        for (i, a) in sequences.iter().enumerate() {
            for b in &sequences[i + 1..] {
                let dot: f64 = a.iter().zip(b).map(|(x, y)| x.0 * y.0 + x.1 * y.1).sum();
                let norm = |s: &[Complex]| s.iter().map(|v| v.0 * v.0 + v.1 * v.1).sum::<f64>();
                assert!(
                    dot.abs() / (norm(a) * norm(b)).sqrt() <= SC_SEPARATION,
                    "preamble PN sequences correlate too strongly"
                );
            }
        }
        this
    }

    /// Carrier indices the Schmidl–Cox sequence occupies.
    #[must_use]
    pub fn even_carriers(&self) -> &[usize] {
        &self.even
    }

    /// Chips carried across all the full pilot symbols of a frame.
    #[must_use]
    pub const fn n_chips(&self) -> usize {
        self.tables.chip_length
    }

    /// Modes this air interface's chips can signal.
    #[must_use]
    pub const fn n_modes(&self) -> usize {
        self.n_modes
    }

    /// (mode, RV) chip sequences in all: [`N_RV`] × [`Self::n_modes`].
    #[must_use]
    pub const fn n_sequences(&self) -> usize {
        N_RV * self.n_modes
    }

    /// Index of the chip sequence carrying `(mode, rv)` on this air interface.
    #[must_use]
    pub const fn chip_index(&self, mode: usize, rv: u8) -> usize {
        chip_index(mode, rv, self.n_modes)
    }

    /// The `(mode, rv)` a chip-sequence index means on this air interface.
    #[must_use]
    pub const fn chip_hypothesis(&self, index: usize) -> (usize, u8) {
        chip_hypothesis(index, self.n_modes)
    }

    /// The whole chip sequence with this index, all pilot symbols end to end.
    ///
    /// # Panics
    /// If the index is past the last sequence.
    #[must_use]
    pub fn chip_sequence(&self, index: usize) -> &'static [f64] {
        assert!(
            index < self.n_sequences(),
            "chip sequence {index} does not exist"
        );
        let length = self.tables.chip_length;
        &self.tables.mode_chips[index * length..(index + 1) * length]
    }

    /// Carrier values of each ordinary Schmidl–Cox symbol for a frame type: unit mean power
    /// over the active carriers, zero on the odd ones.
    #[must_use]
    pub fn sc_values(&self, frame_type: FrameType) -> Vec<Complex> {
        self.sc_values_of(frame_type, false)
    }

    /// Carrier values of each preamble symbol of a frame type and family: the ordinary
    /// sequence on the even carriers, or the floor family's on every carrier (ADR-0009),
    /// unit mean power either way.
    ///
    /// # Panics
    /// If the floor family is asked of an air that has none.
    #[must_use]
    pub fn sc_values_of(&self, frame_type: FrameType, floor: bool) -> Vec<Complex> {
        if floor {
            let signs: &[f64] = match frame_type {
                FrameType::Data => self.tables.floor_sc_data,
                FrameType::Control => self.tables.floor_sc_control,
            };
            assert_eq!(signs.len(), self.n_carriers, "this air has no floor family");
            return signs.iter().map(|&sign| (sign, 0.0)).collect();
        }
        let signs: &[f64] = match frame_type {
            FrameType::Data => self.tables.sc_data,
            FrameType::Control => self.tables.sc_control,
        };
        let mut out = vec![(0.0, 0.0); self.n_carriers];
        for (&carrier, &sign) in self.even.iter().zip(signs) {
            out[carrier] = (sign * self.scale, 0.0);
        }
        out
    }

    /// The two preamble symbols of an ordinary frame — identical by construction.
    #[must_use]
    pub fn symbols(&self, header: &FrameHeader) -> Vec<Vec<Complex>> {
        let values = self.sc_values(header.frame_type);
        vec![values.clone(), values]
    }

    /// The preamble symbols of a frame on a layout: `layout.preamble_symbols` copies of the
    /// type's sequence of the layout's family.
    #[must_use]
    pub fn symbols_for(&self, header: &FrameHeader, layout: &FrameLayout) -> Vec<Vec<Complex>> {
        let values = self.sc_values_of(header.frame_type, layout.is_floor());
        vec![values; layout.preamble_symbols]
    }

    /// Chips a DATA frame on a layout carries: the ordinary set's length on the ordinary
    /// layouts, every full pilot symbol's data carriers on a floor layout.
    #[must_use]
    pub const fn n_chips_for(&self, layout: &FrameLayout) -> usize {
        if layout.is_floor() {
            self.tables.floor_chip_length
        } else {
            self.tables.chip_length
        }
    }

    /// The whole chip sequence with this index on a layout, all pilot symbols end to end.
    ///
    /// # Panics
    /// If the index is past the last sequence, or the floor family is asked of an air
    /// without one.
    #[must_use]
    pub fn chip_sequence_for(&self, index: usize, layout: &FrameLayout) -> &'static [f64] {
        if !layout.is_floor() {
            return self.chip_sequence(index);
        }
        assert!(
            index < self.n_sequences(),
            "chip sequence {index} does not exist"
        );
        let length = self.tables.floor_chip_length;
        assert!(length > 0, "this air has no floor family");
        &self.tables.floor_mode_chips[index * length..(index + 1) * length]
    }

    /// Chips for the data carriers of one full pilot symbol of an ordinary DATA frame.
    ///
    /// # Panics
    /// If the pilot symbol index runs past the chip sequence.
    #[must_use]
    pub fn mode_chips(&self, mode: usize, pilot_symbol_index: usize, rv: u8) -> Vec<f64> {
        // any ordinary layout selects the ordinary chip set
        self.mode_chips_for(mode, pilot_symbol_index, rv, &crate::modes::LONG)
    }

    /// Chips for the data carriers of one full pilot symbol of a DATA frame on a layout.
    ///
    /// # Panics
    /// If the pilot symbol index runs past the chip sequence.
    #[must_use]
    pub fn mode_chips_for(
        &self,
        mode: usize,
        pilot_symbol_index: usize,
        rv: u8,
        layout: &FrameLayout,
    ) -> Vec<f64> {
        let sequence = self.chip_sequence_for(self.chip_index(mode, rv), layout);
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

    fn chip_sets_are_unit_magnitude_and_well_separated(pre: &Preamble, bound: f64) {
        let sequences: Vec<Vec<f64>> = (0..N_RV)
            .flat_map(|rv| {
                (0..pre.n_modes()).map(move |mode| {
                    (0..4)
                        .flat_map(|s| pre.mode_chips(mode, s, rv as u8))
                        .collect::<Vec<f64>>()
                })
            })
            .collect();
        assert_eq!(sequences.len(), pre.n_sequences());
        for sequence in &sequences {
            assert_eq!(sequence.len(), pre.n_chips());
            assert!(sequence.iter().all(|&c| (c.abs() - 1.0).abs() < 1e-12));
        }
        let mut worst: f64 = 0.0;
        for (i, a) in sequences.iter().enumerate() {
            for b in &sequences[i + 1..] {
                let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
                worst = worst.max((dot / pre.n_chips() as f64).abs());
            }
        }
        assert!(worst <= bound + 1e-12, "worst pairwise correlation {worst}");
    }

    #[test]
    fn every_chip_sequence_is_unit_magnitude_and_well_separated() {
        let wide = Preamble::default();
        assert_eq!(
            (wide.n_chips(), wide.n_modes(), wide.n_sequences()),
            (168, 14, 56)
        );
        chip_sets_are_unit_magnitude_and_well_separated(&wide, 0.2);
        // the narrow waveform: 32 chips, fifty-two sequences, a looser bound; its floor
        // frames carry 128 chips at the wide bound (ADR-0009)
        let narrow = Preamble::new(crate::waveform::NARROW_500);
        assert_eq!(
            (narrow.n_chips(), narrow.n_modes(), narrow.n_sequences()),
            (32, 13, 52)
        );
        chip_sets_are_unit_magnitude_and_well_separated(&narrow, 0.25);
        let floor = crate::modes::NARROW_FLOOR_LONG;
        assert_eq!(narrow.n_chips_for(&floor), 128);
        for index in 0..narrow.n_sequences() {
            let seq = narrow.chip_sequence_for(index, &floor);
            assert_eq!(seq.len(), 128);
            assert!(seq.iter().all(|c| (c.abs() - 1.0).abs() < 1e-12));
        }
        // the floor preamble sequences occupy every carrier, unit power each
        let sc = narrow.sc_values_of(FrameType::Data, true);
        assert_eq!(sc.len(), 12);
        assert!(
            sc.iter()
                .all(|v| (v.0.abs() - 1.0).abs() < 1e-12 && v.1 == 0.0)
        );
    }

    #[test]
    fn the_narrow_preamble_keeps_the_frame_types_apart_on_six_carriers() {
        let pre = Preamble::new(crate::waveform::NARROW_500);
        assert_eq!(pre.even_carriers().len(), 6);
        let a = pre.sc_values(FrameType::Data);
        let b = pre.sc_values(FrameType::Control);
        let dot: f64 = a.iter().zip(&b).map(|(x, y)| x.0 * y.0 + x.1 * y.1).sum();
        assert!(
            dot.abs() < 1e-12,
            "the same seeds come out orthogonal at length 6"
        );
    }

    #[test]
    fn chip_index_and_hypothesis_are_inverses() {
        for pre in [
            Preamble::default(),
            Preamble::new(crate::waveform::NARROW_500),
        ] {
            for rv in 0..N_RV as u8 {
                for mode in 0..pre.n_modes() {
                    assert_eq!(pre.chip_hypothesis(pre.chip_index(mode, rv)), (mode, rv));
                }
            }
        }
        // RV 0 occupies the first N_MODES slots, keeping earlier frames unchanged
        assert_eq!(chip_index(5, 0, N_MODES), 5);
        assert_eq!(chip_index(5, 1, 10), 15);
    }

    #[test]
    fn out_of_range_headers_are_rejected() {
        assert!(FrameHeader::new(FrameType::Data, N_MODES, 0).is_err());
        assert!(FrameHeader::new(FrameType::Data, 0, N_RV as u8).is_err());
        assert!(FrameHeader::new(FrameType::Data, N_MODES - 1, N_RV as u8 - 1).is_ok());
        assert_eq!(FrameHeader::control().frame_type, FrameType::Control);
    }
}
