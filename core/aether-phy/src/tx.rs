//! Frame transmitter: a header and coded symbols into complex baseband.
//!
//! A frame is `[SC, SC]` followed by the layout's data symbols. Data symbol `i` is a full
//! pilot symbol when `i` is one of the layout's pilot symbol indices — and in a DATA frame
//! its data carriers then carry the mode and redundancy version as PN chips — and otherwise
//! carries the next `n_data_carriers` constellation symbols, time-major, alongside its comb
//! pilots.
//!
//! Peak reduction (ADR-0004) is *not* applied here yet; see the crate documentation for what
//! that means for comparisons against the reference model.

use crate::{
    constellation::Complex,
    modes::FrameLayout,
    ofdm::OfdmModulator,
    preamble::{FrameHeader, FrameType, Preamble},
    waveform::{WIDE_2300, WaveformParams},
};

/// Assembles frames.
#[derive(Debug)]
pub struct FrameTransmitter {
    params: WaveformParams,
    modulator: OfdmModulator,
    preamble: Preamble,
    n_data_carriers: usize,
}

/// Why a frame could not be assembled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError {
    /// The wrong number of constellation symbols for the layout.
    BadSymbolCount {
        /// What the layout needs.
        expected: usize,
        /// What arrived.
        got: usize,
    },
}

impl core::fmt::Display for TxError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSymbolCount { expected, got } => {
                write!(f, "expected {expected} constellation symbols, got {got}")
            }
        }
    }
}

impl core::error::Error for TxError {}

impl FrameTransmitter {
    /// Build the transmitter.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let modulator = OfdmModulator::new(params);
        let n_data_carriers = modulator.map().data_carriers().len();
        Self {
            params,
            modulator,
            preamble: Preamble::new(params),
            n_data_carriers,
        }
    }

    /// The modulator, for callers that need the carrier map.
    #[must_use]
    pub fn modulator(&self) -> &OfdmModulator {
        &self.modulator
    }

    /// Carrier values for every OFDM symbol of the frame, preamble first.
    ///
    /// # Errors
    /// If `qam` is not exactly the layout's slot count.
    pub fn symbol_values(
        &self,
        header: &FrameHeader,
        layout: &FrameLayout,
        qam: &[Complex],
    ) -> Result<Vec<Vec<Complex>>, TxError> {
        let expected = layout.qam_symbols();
        if qam.len() != expected {
            return Err(TxError::BadSymbolCount {
                expected,
                got: qam.len(),
            });
        }
        let mut symbols = self.preamble.symbols(header);
        let pilots = layout.pilot_symbol_indices();
        let mut position = 0usize;
        let mut pilot_number = 0usize;
        for index in 0..layout.data_symbols {
            if pilots.contains(&index) {
                let chips = match header.frame_type {
                    FrameType::Data => Some(self.preamble.mode_chips(
                        header.mode,
                        pilot_number,
                        header.rv,
                    )),
                    FrameType::Control => None,
                };
                pilot_number += 1;
                symbols.push(self.modulator.pilot_symbol(chips.as_deref()));
            } else {
                let slice = &qam[position..position + self.n_data_carriers];
                position += self.n_data_carriers;
                symbols.push(self.modulator.data_symbol(slice));
            }
        }
        debug_assert_eq!(position, expected);
        Ok(symbols)
    }

    /// Windowed complex-baseband waveform of one frame, `layout.samples() + taper` long.
    ///
    /// # Errors
    /// If `qam` is not exactly the layout's slot count.
    pub fn baseband(
        &self,
        header: &FrameHeader,
        layout: &FrameLayout,
        qam: &[Complex],
    ) -> Result<Vec<Complex>, TxError> {
        let symbols = self.symbol_values(header, layout, qam)?;
        Ok(self.modulator.modulate(&symbols))
    }

    /// The numerology in use.
    #[must_use]
    pub fn params(&self) -> WaveformParams {
        self.params
    }
}

impl Default for FrameTransmitter {
    fn default() -> Self {
        Self::new(WIDE_2300)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::FrameCodec,
        modes::{CONTROL_MODE, LONG, MODES, SHORT},
        ofdm::OfdmDemodulator,
    };

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 29) % 256) as u8).collect()
    }

    #[test]
    fn a_frame_is_the_expected_length() {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(MODES[4], LONG).unwrap();
        let qam = codec.encode(&payload(codec.payload_bytes), 0).unwrap();
        let header = FrameHeader::new(FrameType::Data, 4, 0).unwrap();
        let waveform = tx.baseband(&header, &LONG, &qam).unwrap();
        assert_eq!(waveform.len(), LONG.samples() + WIDE_2300.taper_samples);
    }

    #[test]
    fn the_frame_has_the_right_symbol_structure() {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(MODES[8], LONG).unwrap();
        let qam = codec.encode(&payload(codec.payload_bytes), 2).unwrap();
        let header = FrameHeader::new(FrameType::Data, 8, 2).unwrap();
        let symbols = tx.symbol_values(&header, &LONG, &qam).unwrap();
        assert_eq!(symbols.len(), LONG.total_symbols());
        assert_eq!(
            symbols[0], symbols[1],
            "the two preamble symbols are identical"
        );
        // every symbol carries one value per active carrier
        assert!(symbols.iter().all(|s| s.len() == WIDE_2300.n_carriers()));
    }

    #[test]
    fn demodulating_a_frame_returns_the_transmitted_carrier_values() {
        let tx = FrameTransmitter::default();
        let demodulator = OfdmDemodulator::new(WIDE_2300);
        let codec = FrameCodec::new(MODES[4], LONG).unwrap();
        let qam = codec.encode(&payload(codec.payload_bytes), 0).unwrap();
        let header = FrameHeader::new(FrameType::Data, 4, 0).unwrap();
        let expected = tx.symbol_values(&header, &LONG, &qam).unwrap();
        let waveform = tx.baseband(&header, &LONG, &qam).unwrap();
        let period = WIDE_2300.symbol_samples();

        // the last symbol has no successor to overlap with, so stop one short
        for (index, want) in expected.iter().enumerate().take(LONG.total_symbols() - 1) {
            let got = demodulator
                .carriers(&waveform, index * period)
                .expect("in range");
            for (carrier, (g, w)) in got.iter().zip(want).enumerate() {
                assert!(
                    (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9,
                    "symbol {index}, carrier {carrier}"
                );
            }
        }
    }

    #[test]
    fn a_control_frame_carries_no_chips() {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(CONTROL_MODE, SHORT).unwrap();
        let qam = codec.encode(&payload(codec.payload_bytes), 0).unwrap();
        let symbols = tx
            .symbol_values(&FrameHeader::control(), &SHORT, &qam)
            .unwrap();
        let map = tx.modulator().map();
        // a control frame's pilot symbols carry the plain pilot sequence on every carrier
        let pilot_symbol = &symbols[2]; // first data symbol, which is a pilot symbol
        for &carrier in map.data_carriers() {
            let expected = map.pilot_sequence()[carrier];
            assert!((pilot_symbol[carrier].0 - expected.0).abs() < 1e-12);
            assert!((pilot_symbol[carrier].1 - expected.1).abs() < 1e-12);
        }
    }

    #[test]
    fn a_data_frame_puts_its_chips_on_the_pilot_symbols() {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(MODES[6], LONG).unwrap();
        let qam = codec.encode(&payload(codec.payload_bytes), 3).unwrap();
        let header = FrameHeader::new(FrameType::Data, 6, 3).unwrap();
        let symbols = tx.symbol_values(&header, &LONG, &qam).unwrap();
        let map = tx.modulator().map();
        let preamble = Preamble::default();
        for (pilot_number, &index) in LONG.pilot_symbol_indices().iter().enumerate() {
            let symbol = &symbols[2 + index];
            let chips = preamble.mode_chips(6, pilot_number, 3);
            for (&carrier, &chip) in map.data_carriers().iter().zip(&chips) {
                assert!(
                    (symbol[carrier].0 - chip).abs() < 1e-12,
                    "pilot symbol {index}"
                );
                assert!(symbol[carrier].1.abs() < 1e-12);
            }
        }
    }

    #[test]
    fn the_wrong_symbol_count_is_an_error() {
        let tx = FrameTransmitter::default();
        let header = FrameHeader::new(FrameType::Data, 0, 0).unwrap();
        assert!(matches!(
            tx.baseband(&header, &LONG, &[(0.0, 0.0); 3]),
            Err(TxError::BadSymbolCount { .. })
        ));
    }
}
