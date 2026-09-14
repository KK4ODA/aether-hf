//! Frame layouts and the mode table — everything derives from [`WaveformParams`].
//!
//! A *frame* is a preamble (two Schmidl–Cox symbols whose PN sequence encodes the frame
//! type) followed by `data_symbols` OFDM symbols, every `pilot_symbol_period`-th of which is
//! a full pilot symbol. A *mode* is a (modulation, code rate) pair; with a layout it fixes
//! the coded bits, information bits and payload bytes of a frame.
//!
//! Base-graph choice follows the public 5G rule (TS 38.212 §7.2.2), one code block per frame.

use aether_fec::{
    CRC24A,
    ldpc::{select_base_graph, select_lifting_size},
};

use crate::waveform::{Modulation, WIDE_2300, WaveformParams};

/// Symbols of preamble ahead of every frame.
pub const PREAMBLE_SYMBOLS: usize = 2;

/// How many OFDM symbols a frame carries, and which of them are full pilot symbols.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLayout {
    /// `"long"` or `"short"`.
    pub name: &'static str,
    /// Symbols after the preamble.
    pub data_symbols: usize,
    /// The numerology the layout is measured against.
    pub waveform: WaveformParams,
}

/// Data frames: 34 symbols, about 1.05 s.
pub const LONG: FrameLayout = FrameLayout {
    name: "long",
    data_symbols: 32,
    waveform: WIDE_2300,
};
/// Control frames (ACK, connect, ping): 14 symbols, about 0.43 s.
pub const SHORT: FrameLayout = FrameLayout {
    name: "short",
    data_symbols: 12,
    waveform: WIDE_2300,
};

impl FrameLayout {
    /// Indices, within the data symbols, that are full pilot symbols.
    #[must_use]
    pub fn pilot_symbol_indices(&self) -> Vec<usize> {
        let period = self.waveform.pilot_symbol_period;
        if period == 0 {
            return Vec::new();
        }
        (0..self.data_symbols).step_by(period).collect()
    }

    /// How many full pilot symbols the frame has.
    #[must_use]
    pub fn n_pilot_symbols(&self) -> usize {
        self.pilot_symbol_indices().len()
    }

    /// Data symbols that carry payload rather than pilots.
    #[must_use]
    pub fn n_payload_symbols(&self) -> usize {
        self.data_symbols - self.n_pilot_symbols()
    }

    /// Data-carrier slots available for coded bits in one frame.
    #[must_use]
    pub fn qam_symbols(&self) -> usize {
        self.n_payload_symbols() * self.waveform.n_data_carriers()
    }

    /// Preamble plus data symbols.
    #[must_use]
    pub fn total_symbols(&self) -> usize {
        PREAMBLE_SYMBOLS + self.data_symbols
    }

    /// Frame duration in seconds.
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        self.total_symbols() as f64 * self.waveform.symbol_period_s()
    }

    /// Frame length in baseband samples.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.total_symbols() * self.waveform.symbol_samples()
    }
}

/// One (modulation, code rate) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    /// Position in [`MODES`], and the value signalled in the pilot chips.
    pub index: usize,
    /// Constellation.
    pub modulation: Modulation,
    /// Code rate numerator.
    pub rate_num: usize,
    /// Code rate denominator.
    pub rate_den: usize,
}

impl Mode {
    /// Name as it appears in the mode table, e.g. `QAM16-3/4`.
    #[must_use]
    pub fn name(&self) -> String {
        format!(
            "{}-{}/{}",
            self.modulation.name(),
            self.rate_num,
            self.rate_den
        )
    }

    /// Code rate as a float.
    #[must_use]
    pub fn code_rate(&self) -> f64 {
        self.rate_num as f64 / self.rate_den as f64
    }

    /// Coded bits carried by one frame.
    #[must_use]
    pub fn coded_bits(&self, layout: &FrameLayout) -> usize {
        layout.qam_symbols() * self.modulation.bits_per_symbol()
    }

    /// `K'` — payload plus CRC, rounded down to a whole number of payload bytes.
    #[must_use]
    pub fn info_bits(&self, layout: &FrameLayout) -> usize {
        let raw = self.coded_bits(layout) * self.rate_num / self.rate_den;
        let payload_bytes = (raw - CRC24A.width as usize) / 8;
        payload_bytes * 8 + CRC24A.width as usize
    }

    /// Payload bytes carried by one frame.
    #[must_use]
    pub fn payload_bytes(&self, layout: &FrameLayout) -> usize {
        (self.info_bits(layout) - CRC24A.width as usize) / 8
    }

    /// Base graph chosen by TS 38.212 §7.2.2.
    #[must_use]
    pub fn base_graph(&self, layout: &FrameLayout) -> u8 {
        select_base_graph(self.payload_bytes(layout) * 8, self.code_rate())
    }

    /// Lifting size for this mode's information block.
    ///
    /// # Panics
    /// If the information block does not fit the base graph, which the fixed mode table
    /// never produces.
    #[must_use]
    pub fn lifting_size(&self, layout: &FrameLayout) -> usize {
        select_lifting_size(self.base_graph(layout), self.info_bits(layout))
            .expect("mode table fits its base graph")
    }

    /// Payload bits per second of frame air time, ignoring ACK turnaround.
    #[must_use]
    pub fn net_bit_rate(&self, layout: &FrameLayout) -> f64 {
        8.0 * self.payload_bytes(layout) as f64 / layout.duration_s()
    }
}

const fn mode(index: usize, modulation: Modulation, rate_num: usize, rate_den: usize) -> Mode {
    Mode {
        index,
        modulation,
        rate_num,
        rate_den,
    }
}

/// The mode table, ordered from most robust to fastest; the rate controller steps along it.
pub const MODES: [Mode; 14] = [
    mode(0, Modulation::Bpsk, 1, 5),
    mode(1, Modulation::Bpsk, 1, 3),
    mode(2, Modulation::Bpsk, 1, 2),
    mode(3, Modulation::Qpsk, 1, 3),
    mode(4, Modulation::Qpsk, 1, 2),
    mode(5, Modulation::Qpsk, 2, 3),
    mode(6, Modulation::Psk8, 1, 2),
    mode(7, Modulation::Psk8, 2, 3),
    mode(8, Modulation::Qam16, 1, 2),
    mode(9, Modulation::Qam16, 2, 3),
    mode(10, Modulation::Qam16, 3, 4),
    mode(11, Modulation::Qam64, 2, 3),
    mode(12, Modulation::Qam64, 3, 4),
    mode(13, Modulation::Qam64, 5, 6),
];

/// Every control frame (ACK, connect, ping) uses the most robust mode on the SHORT layout.
pub const CONTROL_MODE: Mode = MODES[0];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_are_the_specification() {
        assert_eq!(LONG.total_symbols(), 34);
        assert_eq!(LONG.samples(), 8432);
        assert_eq!(LONG.qam_symbols(), 1176);
        assert_eq!(LONG.pilot_symbol_indices(), vec![0, 8, 16, 24]);
        assert_eq!(SHORT.total_symbols(), 14);
        assert_eq!(SHORT.qam_symbols(), 420);
        assert_eq!(SHORT.pilot_symbol_indices(), vec![0, 8]);
        assert!((LONG.duration_s() - 1.054).abs() < 1e-9);
    }

    #[test]
    fn payload_grows_monotonically_with_mode() {
        let payloads: Vec<usize> = MODES.iter().map(|m| m.payload_bytes(&LONG)).collect();
        assert!(payloads.windows(2).all(|w| w[0] <= w[1]), "{payloads:?}");
        assert_eq!(payloads[0], 26);
        assert_eq!(payloads[13], 732);
    }

    #[test]
    fn coded_bits_are_the_slots_times_the_constellation() {
        for m in &MODES {
            assert_eq!(m.coded_bits(&LONG), 1176 * m.modulation.bits_per_symbol());
        }
    }

    #[test]
    fn every_mode_fits_its_lifting_size() {
        for m in &MODES {
            let z = m.lifting_size(&LONG);
            let kb = if m.base_graph(&LONG) == 1 { 22 } else { 10 };
            assert!(kb * z >= m.info_bits(&LONG), "{}", m.name());
        }
    }

    #[test]
    fn the_control_mode_carries_seven_bytes() {
        assert_eq!(CONTROL_MODE.payload_bytes(&SHORT), 7);
    }

    #[test]
    fn mode_names_match_the_table() {
        assert_eq!(MODES[0].name(), "BPSK-1/5");
        assert_eq!(MODES[10].name(), "QAM16-3/4");
        assert_eq!(MODES[13].name(), "QAM64-5/6");
    }
}
