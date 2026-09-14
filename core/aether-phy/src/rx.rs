//! Frame receiver: carrier-offset correction, channel estimation, equalisation, LLR weights.
//!
//! Given where a frame starts and roughly what its carrier offset is, the receiver
//!
//! 1. removes the offset from the whole frame, then measures the residual from the
//!    pilot-to-pilot phase progression across symbols — the channel cancels between
//!    consecutive symbols, so what is left is the offset — and corrects again. 64-QAM needs
//!    the residual well below 0.1 Hz, which the preamble estimate alone does not deliver;
//! 2. takes the transform of every symbol;
//! 3. for DATA frames reads the mode and redundancy version from the chips on the full pilot
//!    symbols, correlating against every (RV, mode) sequence and reporting the margin over
//!    the runner-up so a caller can retry if the CRC fails;
//! 4. estimates the channel on every symbol — full estimates on the pilot symbols once their
//!    chips are known, comb estimates smoothed over neighbouring symbols and interpolated
//!    across carriers elsewhere;
//! 5. equalises the data carriers and returns the symbols with a **per-symbol** noise
//!    variance.
//!
//! That last point matters more on HF than the interpolation does. Impulsive noise is
//! concentrated in time: one hot sample damages every carrier of the symbol it lands in and
//! none of the others. A single frame-wide variance would average that damage over the clean
//! symbols, which both overstates the noise on those and — far worse — understates it on the
//! damaged one, so the decoder confidently believes exactly the values it should be throwing
//! away. Per symbol, a hit symbol's LLRs shrink on their own and it becomes an erasure.

use crate::{
    constellation::Complex,
    modes::{FrameLayout, LONG, PREAMBLE_SYMBOLS, SHORT},
    ofdm::{DemodError, OfdmDemodulator},
    preamble::{FrameType, N_MODES, N_RV, Preamble, chip_hypothesis},
    tables,
    waveform::{WIDE_2300, WaveformParams},
};

/// Complex helpers, kept local so the crate has no numeric dependency of its own.
mod c {
    use super::Complex;

    pub fn add(a: Complex, b: Complex) -> Complex {
        (a.0 + b.0, a.1 + b.1)
    }
    pub fn sub(a: Complex, b: Complex) -> Complex {
        (a.0 - b.0, a.1 - b.1)
    }
    pub fn mul(a: Complex, b: Complex) -> Complex {
        (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
    }
    pub fn conj(a: Complex) -> Complex {
        (a.0, -a.1)
    }
    pub fn scale(a: Complex, k: f64) -> Complex {
        (a.0 * k, a.1 * k)
    }
    pub fn div(a: Complex, b: Complex) -> Complex {
        let d = b.0 * b.0 + b.1 * b.1;
        if d == 0.0 {
            return (0.0, 0.0);
        }
        ((a.0 * b.0 + a.1 * b.1) / d, (a.1 * b.0 - a.0 * b.1) / d)
    }
    pub fn norm_sq(a: Complex) -> f64 {
        a.0 * a.0 + a.1 * a.1
    }
    pub fn from_angle(theta: f64) -> Complex {
        (theta.cos(), theta.sin())
    }
}

/// Which layout a frame type uses.
#[must_use]
pub fn layout_for(frame_type: FrameType) -> FrameLayout {
    match frame_type {
        FrameType::Data => LONG,
        FrameType::Control => SHORT,
    }
}

/// Where a frame is and what the preamble estimated about it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSync {
    /// Sample index the frame starts at.
    pub start: usize,
    /// Carrier offset the preamble estimated, in hertz.
    pub cfo_hz: f64,
    /// Which container it is.
    pub frame_type: FrameType,
}

/// One demodulated frame.
#[derive(Debug, Clone)]
pub struct ReceivedFrame {
    /// Where it was and what it announced.
    pub sync: FrameSync,
    /// Equalised constellation symbols, time-major.
    pub symbols: Vec<Complex>,
    /// Effective complex noise variance per equalised symbol, for LLR weighting.
    pub noise_var: Vec<f64>,
    /// Mean symbol energy over noise per carrier, estimated from the pilots.
    pub snr_carrier_db: f64,
    /// The same, referenced to a 3 kHz noise bandwidth — the project convention.
    pub snr_3k_db: f64,
    /// Total carrier offset removed: the preamble estimate plus the data-aided residual.
    pub cfo_hz: f64,
    /// Mode read from the pilot chips, or the control mode.
    pub mode: usize,
    /// Redundancy version read from the same chips.
    pub rv: u8,
    /// Second-best chip hypothesis, for a retry.
    pub chip_runner_up: usize,
    /// Best chip metric over the runner-up; below about 1.3 a caller may retry.
    pub mode_confidence: f64,
}

/// Turns a located frame into symbols and LLR weights.
#[derive(Debug)]
pub struct FrameReceiver {
    params: WaveformParams,
    demodulator: OfdmDemodulator,
    preamble: Preamble,
    /// How far each per-symbol noise estimate is pulled back toward the frame-wide one.
    /// Zero trusts fifteen pilots completely, one ignores them; a half flags a damaged symbol
    /// without letting pilot noise alone condemn a clean one.
    pub noise_shrinkage: f64,
    /// A symbol's variance is never allowed below this multiple of the frame value, so an
    /// unluckily quiet pilot set cannot make the decoder over-trust a symbol.
    pub noise_floor_fraction: f64,
}

impl Default for FrameReceiver {
    fn default() -> Self {
        Self::new(WIDE_2300)
    }
}

impl FrameReceiver {
    /// Build it.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        Self {
            params,
            demodulator: OfdmDemodulator::new(params),
            preamble: Preamble::new(params),
            noise_shrinkage: 0.5,
            noise_floor_fraction: 0.25,
        }
    }

    /// How far into a symbol period the transform window is taken.
    ///
    /// A streaming caller needs this to know when a frame's last symbol is really complete.
    #[must_use]
    pub fn fft_offset(&self) -> usize {
        self.demodulator.fft_offset()
    }

    /// Where a frame starts and ends in the sample stream.
    #[must_use]
    pub fn frame_span(&self, sync: &FrameSync) -> (usize, usize) {
        let layout = layout_for(sync.frame_type);
        (sync.start, sync.start + layout.samples())
    }

    /// Demodulate and equalise one frame.
    ///
    /// `hypothesis` overrides the chip decision with a specific `(mode, rv)` index — used for
    /// the runner-up retry after a CRC failure.
    ///
    /// # Errors
    /// If the frame runs past the end of the buffer.
    ///
    /// # Panics
    /// If the chip metrics are not finite, which would mean the input contained NaN.
    #[allow(clippy::too_many_lines)]
    pub fn receive(
        &self,
        samples: &[Complex],
        sync: &FrameSync,
        hypothesis: Option<usize>,
    ) -> Result<ReceivedFrame, DemodError> {
        let layout = layout_for(sync.frame_type);
        let period = self.params.symbol_samples();
        let n_sym = layout.total_symbols();
        let pre = PREAMBLE_SYMBOLS;
        let map = self.demodulator.map();
        let pilot_carriers = map.pilot_carriers().to_vec();
        let data_carriers = map.data_carriers().to_vec();
        let n_carriers = map.n_carriers();

        let (start, end) = self.frame_span(sync);
        // One extra period so the last symbol's transform window is inside the slice; the
        // demodulator re-checks each symbol, so a short tail surfaces there rather than here.
        let segment_end = (end + period).min(samples.len());
        if start >= segment_end {
            return Err(DemodError::OutOfRange {
                start,
                available: samples.len(),
            });
        }
        let segment = &samples[start..segment_end];

        // 1. remove the preamble's offset estimate, then the data-aided residual
        let demodulate = |cfo_hz: f64| -> Result<Vec<Vec<Complex>>, DemodError> {
            let rotated: Vec<Complex> = segment
                .iter()
                .enumerate()
                .map(|(index, &sample)| {
                    let t = index as f64 / self.params.fs_baseband;
                    c::mul(
                        sample,
                        c::from_angle(-2.0 * core::f64::consts::PI * cfo_hz * t),
                    )
                })
                .collect();
            (0..n_sym)
                .map(|i| self.demodulator.carriers(&rotated, i * period))
                .collect()
        };

        let raw = demodulate(sync.cfo_hz)?;
        let pilot_reference: Vec<Complex> = pilot_carriers
            .iter()
            .map(|&p| map.pilot_sequence()[p])
            .collect();
        let mut progression = (0.0, 0.0);
        for symbol in pre + 1..n_sym {
            for (slot, &carrier) in pilot_carriers.iter().enumerate() {
                let reference = pilot_reference[slot];
                let term = c::mul(
                    c::mul(raw[symbol][carrier], c::conj(raw[symbol - 1][carrier])),
                    c::mul(reference, c::conj(reference)),
                );
                progression = c::add(progression, term);
            }
        }
        let residual = progression.1.atan2(progression.0)
            / (2.0 * core::f64::consts::PI * self.params.symbol_period_s());
        let cfo_total = sync.cfo_hz + residual;
        let raw = demodulate(cfo_total)?;

        // 2. comb estimates on every symbol after the preamble, smoothed over +-1 symbol
        let mut comb = vec![vec![(0.0, 0.0); pilot_carriers.len()]; n_sym];
        for symbol in pre..n_sym {
            for (slot, &carrier) in pilot_carriers.iter().enumerate() {
                comb[symbol][slot] = c::div(raw[symbol][carrier], pilot_reference[slot]);
            }
        }
        let mut smoothed = comb.clone();
        for (symbol, row) in smoothed.iter_mut().enumerate().skip(pre) {
            let low = symbol.saturating_sub(1).max(pre);
            let high = (symbol + 1).min(n_sym - 1);
            let count = (high - low + 1) as f64;
            for (slot, value) in row.iter_mut().enumerate() {
                let mut sum = (0.0, 0.0);
                for neighbour in &comb[low..=high] {
                    sum = c::add(sum, neighbour[slot]);
                }
                *value = c::scale(sum, 1.0 / count);
            }
        }
        let interpolate = |row: &[Complex]| -> Vec<Complex> {
            let mut out = vec![(0.0, 0.0); n_carriers];
            for (carrier, slot) in out.iter_mut().enumerate() {
                // locate the bracketing pilots
                let upper = pilot_carriers.partition_point(|&p| p < carrier);
                if upper < pilot_carriers.len() && pilot_carriers[upper] == carrier {
                    *slot = row[upper];
                    continue;
                }
                let hi = upper.min(pilot_carriers.len() - 1);
                let lo = hi.saturating_sub(1);
                let (x0, x1) = (pilot_carriers[lo] as f64, pilot_carriers[hi] as f64);
                let t = if (x1 - x0).abs() < f64::EPSILON {
                    0.0
                } else {
                    (carrier as f64 - x0) / (x1 - x0)
                };
                *slot = (
                    row[lo].0 + t * (row[hi].0 - row[lo].0),
                    row[lo].1 + t * (row[hi].1 - row[lo].1),
                );
            }
            out
        };

        let pilot_symbols: Vec<usize> = layout
            .pilot_symbol_indices()
            .into_iter()
            .map(|i| pre + i)
            .collect();
        let data_symbols: Vec<usize> = (pre..n_sym)
            .filter(|s| !pilot_symbols.contains(s))
            .collect();

        // 3. mode and redundancy version from the chips on the pilot symbols
        let (mut mode, mut rv, mut runner_up, mut confidence) = (0usize, 0u8, 0usize, 1.0f64);
        if sync.frame_type == FrameType::Data {
            let mut observed: Vec<Complex> = Vec::new();
            for &symbol in &pilot_symbols {
                let channel = interpolate(&smoothed[symbol]);
                for &carrier in &data_carriers {
                    observed.push(c::mul(raw[symbol][carrier], c::conj(channel[carrier])));
                }
            }
            let energy: f64 = observed.iter().map(|&v| c::norm_sq(v)).sum::<f64>().sqrt();
            if energy > 0.0 {
                for value in &mut observed {
                    *value = c::scale(*value, 1.0 / energy);
                }
            }
            let used = observed.len();
            let mut metrics = vec![0.0f64; N_RV * N_MODES];
            for (index, sequence) in tables::MODE_CHIPS.iter().enumerate() {
                let mut accumulator = (0.0, 0.0);
                for (slot, &chip) in sequence.iter().take(used).enumerate() {
                    accumulator = c::add(accumulator, c::scale(observed[slot], chip));
                }
                metrics[index] = c::norm_sq(accumulator).sqrt() / (used as f64).sqrt();
            }
            let mut order: Vec<usize> = (0..metrics.len()).collect();
            order.sort_by(|&a, &b| metrics[b].partial_cmp(&metrics[a]).expect("finite metrics"));
            let best = order[0];
            runner_up = order[1];
            confidence = metrics[best] / metrics[runner_up].max(1e-12);
            let (m, r) = chip_hypothesis(hypothesis.unwrap_or(best));
            mode = m;
            rv = r;
        }

        // 4. known carrier values per symbol, then the channel estimates
        let mut known = vec![vec![(0.0, 0.0); n_carriers]; n_sym];
        let mut is_pilot_symbol = vec![false; n_sym];
        for (pilot_number, &symbol) in pilot_symbols.iter().enumerate() {
            known[symbol].copy_from_slice(map.pilot_sequence());
            if sync.frame_type == FrameType::Data {
                let chips = self.preamble.mode_chips(mode, pilot_number, rv);
                for (&carrier, &chip) in data_carriers.iter().zip(&chips) {
                    known[symbol][carrier] = (chip, 0.0);
                }
            }
            is_pilot_symbol[symbol] = true;
        }
        for &symbol in &data_symbols {
            for (slot, &carrier) in pilot_carriers.iter().enumerate() {
                known[symbol][carrier] = pilot_reference[slot];
            }
        }

        let mut channel = vec![vec![(0.0, 0.0); n_carriers]; n_sym];
        for symbol in pre..n_sym {
            if is_pilot_symbol[symbol] {
                let direct: Vec<Complex> = (0..n_carriers)
                    .map(|carrier| c::div(raw[symbol][carrier], known[symbol][carrier]))
                    .collect();
                // three-tap smoothing across carriers, edges left alone
                for carrier in 0..n_carriers {
                    channel[symbol][carrier] = if carrier == 0 || carrier == n_carriers - 1 {
                        direct[carrier]
                    } else {
                        let mut sum = c::scale(direct[carrier], 0.5);
                        sum = c::add(sum, c::scale(direct[carrier - 1], 0.25));
                        c::add(sum, c::scale(direct[carrier + 1], 0.25))
                    };
                }
            } else {
                channel[symbol] = interpolate(&smoothed[symbol]);
            }
        }

        // 5. noise variance, per symbol and frame-wide
        let bias = 3.0 / 2.0; // undo the three-tap averaging bias
        let mut per_symbol = Vec::with_capacity(data_symbols.len());
        let mut total = 0.0f64;
        let mut count = 0usize;
        for &symbol in &data_symbols {
            let mut sum = 0.0;
            for (slot, &carrier) in pilot_carriers.iter().enumerate() {
                let expected = c::mul(channel[symbol][carrier], known[symbol][carrier]);
                sum += c::norm_sq(c::sub(raw[symbol][carrier], expected));
                let _ = slot;
            }
            let mean = sum / pilot_carriers.len() as f64;
            per_symbol.push(mean * bias);
            total += sum;
            count += pilot_carriers.len();
        }
        let sigma2 = if count == 0 {
            0.0
        } else {
            total / count as f64 * bias
        };
        let floor = sigma2 * self.noise_floor_fraction;
        let sigma2_symbol: Vec<f64> = per_symbol
            .iter()
            .map(|&value| {
                (self.noise_shrinkage * sigma2 + (1.0 - self.noise_shrinkage) * value).max(floor)
            })
            .collect();

        let signal_power: f64 = data_symbols
            .iter()
            .flat_map(|&s| (0..n_carriers).map(move |carrier| (s, carrier)))
            .map(|(s, carrier)| c::norm_sq(channel[s][carrier]))
            .sum::<f64>()
            / (data_symbols.len() * n_carriers) as f64;
        let snr_carrier = signal_power / sigma2.max(1e-12);
        let bandwidth_ratio = self.params.occupied_bandwidth_hz() / 3000.0;

        // 6. equalise the data carriers, time-major
        let mut symbols = Vec::with_capacity(layout.qam_symbols());
        let mut noise_var = Vec::with_capacity(layout.qam_symbols());
        for (position, &symbol) in data_symbols.iter().enumerate() {
            for &carrier in &data_carriers {
                let h = channel[symbol][carrier];
                symbols.push(c::div(raw[symbol][carrier], h));
                noise_var.push(sigma2_symbol[position] / c::norm_sq(h).max(1e-9));
            }
        }

        Ok(ReceivedFrame {
            sync: *sync,
            symbols,
            noise_var,
            snr_carrier_db: 10.0 * snr_carrier.log10(),
            snr_3k_db: 10.0 * (snr_carrier * bandwidth_ratio).log10(),
            cfo_hz: cfo_total,
            mode,
            rv,
            chip_runner_up: runner_up,
            mode_confidence: confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::FrameCodec,
        constellation::NoiseVar,
        modes::{CONTROL_MODE, MODES},
        preamble::FrameHeader,
        tx::FrameTransmitter,
    };

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 43) % 256) as u8).collect()
    }

    /// Transmit a frame into a buffer with lead-in silence, as a receiver would see it.
    fn transmit(mode_index: usize, rv: u8, lead: usize) -> (Vec<Complex>, Vec<u8>, FrameSync) {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(MODES[mode_index], LONG).expect("codec");
        let data = payload(codec.payload_bytes);
        let qam = codec.encode(&data, rv).expect("encode");
        let header = FrameHeader::new(FrameType::Data, mode_index, rv).expect("header");
        let frame = tx.baseband(&header, &LONG, &qam).expect("baseband");
        let mut buffer = vec![(0.0, 0.0); lead];
        buffer.extend_from_slice(&frame);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 600));
        (
            buffer,
            data,
            FrameSync {
                start: lead,
                cfo_hz: 0.0,
                frame_type: FrameType::Data,
            },
        )
    }

    #[test]
    fn a_clean_frame_reports_the_mode_and_redundancy_version_it_was_sent_with() {
        // Reading (mode, rv) off the pilot chips is the receiver's job and has to work for
        // every combination, including the redundancy versions that are not decodable alone.
        for mode_index in [0usize, 4, 8, 13] {
            for rv in 0..4u8 {
                let (buffer, _, sync) = transmit(mode_index, rv, 500);
                let rx = FrameReceiver::default();
                let frame = rx.receive(&buffer, &sync, None).expect("receive");
                assert_eq!(frame.mode, mode_index, "mode of {mode_index}/rv{rv}");
                assert_eq!(frame.rv, rv, "rv of {mode_index}/rv{rv}");
                assert!(
                    frame.mode_confidence > 2.0,
                    "{mode_index}/rv{rv}: chip confidence {}",
                    frame.mode_confidence
                );
                assert!(
                    frame.snr_carrier_db > 30.0,
                    "{mode_index}/rv{rv}: clean SNR reads {}",
                    frame.snr_carrier_db
                );
            }
        }
    }

    #[test]
    fn a_clean_first_transmission_decodes_for_every_mode() {
        // Redundancy version 0 is the one that carries the systematic bits and is meant to
        // stand alone; the others are parity and only make sense combined (see below).
        for (mode_index, mode) in MODES.iter().enumerate() {
            let (buffer, data, sync) = transmit(mode_index, 0, 500);
            let rx = FrameReceiver::default();
            let frame = rx.receive(&buffer, &sync, None).expect("receive");
            let codec = FrameCodec::new(*mode, LONG).expect("codec");
            let (decoded, _) = codec
                .decode(
                    &frame.symbols,
                    NoiseVar::PerSymbol(&frame.noise_var),
                    0,
                    None,
                )
                .expect("decode");
            assert_eq!(decoded.as_deref(), Some(&data[..]), "{}", mode.name());
        }
    }

    #[test]
    fn a_later_redundancy_version_decodes_once_combined_with_the_first() {
        // The protocol never decodes an RV on its own except RV 0: a retransmission is
        // combined with what the receiver already holds. This walks that path through the
        // receiver rather than straight from the codec.
        let mode_index = 4;
        let codec = FrameCodec::new(MODES[mode_index], LONG).expect("codec");
        let mut buffer_llrs: Option<Vec<f64>> = None;
        let mut payload_out = None;
        for rv in 0..2u8 {
            let (buffer, data, sync) = transmit(mode_index, rv, 500);
            let rx = FrameReceiver::default();
            let frame = rx.receive(&buffer, &sync, None).expect("receive");
            assert_eq!(frame.rv, rv);
            let (decoded, llrs) = codec
                .decode(
                    &frame.symbols,
                    NoiseVar::PerSymbol(&frame.noise_var),
                    frame.rv,
                    buffer_llrs.as_deref(),
                )
                .expect("decode");
            buffer_llrs = Some(llrs);
            if rv == 1 {
                payload_out = decoded.map(|d| (d, data));
            }
        }
        let (decoded, expected) = payload_out.expect("the combined decode produced nothing");
        assert_eq!(
            decoded, expected,
            "combining RV0 and RV1 did not recover the payload"
        );
    }

    #[test]
    fn a_carrier_offset_is_measured_and_removed() {
        let (buffer, data, mut sync) = transmit(4, 0, 500);
        let offset = 37.5;
        let rotated: Vec<Complex> = buffer
            .iter()
            .enumerate()
            .map(|(index, &sample)| {
                let t = index as f64 / WIDE_2300.fs_baseband;
                c::mul(
                    sample,
                    c::from_angle(2.0 * core::f64::consts::PI * offset * t),
                )
            })
            .collect();
        // hand the receiver a deliberately imperfect preamble estimate
        sync.cfo_hz = offset - 1.5;
        let rx = FrameReceiver::default();
        let frame = rx.receive(&rotated, &sync, None).expect("receive");
        assert!(
            (frame.cfo_hz - offset).abs() < 0.1,
            "residual not removed: {} vs {offset}",
            frame.cfo_hz
        );
        let codec = FrameCodec::new(MODES[4], LONG).expect("codec");
        let (decoded, _) = codec
            .decode(
                &frame.symbols,
                NoiseVar::PerSymbol(&frame.noise_var),
                0,
                None,
            )
            .expect("decode");
        assert_eq!(decoded.as_deref(), Some(&data[..]));
    }

    #[test]
    fn a_control_frame_needs_no_chips() {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(CONTROL_MODE, SHORT).expect("codec");
        let data = payload(codec.payload_bytes);
        let qam = codec.encode(&data, 0).expect("encode");
        let frame = tx
            .baseband(&FrameHeader::control(), &SHORT, &qam)
            .expect("baseband");
        let mut buffer = vec![(0.0, 0.0); 400];
        buffer.extend_from_slice(&frame);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 600));
        let sync = FrameSync {
            start: 400,
            cfo_hz: 0.0,
            frame_type: FrameType::Control,
        };

        let rx = FrameReceiver::default();
        let received = rx.receive(&buffer, &sync, None).expect("receive");
        assert_eq!(received.symbols.len(), SHORT.qam_symbols());
        let (decoded, _) = codec
            .decode(
                &received.symbols,
                NoiseVar::PerSymbol(&received.noise_var),
                0,
                None,
            )
            .expect("decode");
        assert_eq!(decoded.as_deref(), Some(&data[..]));
    }

    #[test]
    fn the_runner_up_hypothesis_can_be_forced() {
        let (buffer, _, sync) = transmit(4, 1, 500);
        let rx = FrameReceiver::default();
        let frame = rx.receive(&buffer, &sync, None).expect("receive");
        let forced = rx
            .receive(&buffer, &sync, Some(frame.chip_runner_up))
            .expect("receive");
        let (mode, rv) = chip_hypothesis(frame.chip_runner_up);
        assert_eq!((forced.mode, forced.rv), (mode, rv));
    }

    #[test]
    fn a_damaged_symbol_gets_a_higher_noise_variance_than_its_neighbours() {
        // one hot sample inside one symbol: the per-symbol estimate must notice
        let (mut buffer, _, sync) = transmit(4, 0, 500);
        let victim = 500 + 10 * WIDE_2300.symbol_samples() + WIDE_2300.symbol_samples() / 2;
        buffer[victim] = (buffer[victim].0 + 60.0, buffer[victim].1);

        let rx = FrameReceiver::default();
        let frame = rx.receive(&buffer, &sync, None).expect("receive");
        let per_carrier = WIDE_2300.n_data_carriers();
        let means: Vec<f64> = frame
            .noise_var
            .chunks_exact(per_carrier)
            .map(|chunk| chunk.iter().sum::<f64>() / chunk.len() as f64)
            .collect();
        let worst = means.iter().copied().fold(f64::MIN, f64::max);
        let median = {
            let mut sorted = means.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
            sorted[sorted.len() / 2]
        };
        assert!(
            worst > median * 5.0,
            "damaged symbol not distrusted: {worst} vs {median}"
        );
    }

    #[test]
    fn a_frame_past_the_end_of_the_buffer_is_an_error() {
        let rx = FrameReceiver::default();
        let sync = FrameSync {
            start: 0,
            cfo_hz: 0.0,
            frame_type: FrameType::Data,
        };
        assert!(rx.receive(&[(0.0, 0.0); 100], &sync, None).is_err());
    }
}
