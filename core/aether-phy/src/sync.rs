//! Frame acquisition: finding a preamble, its timing, and its carrier offset.
//!
//! Stages, on band-limited complex baseband:
//!
//! 1. **Matched-filter bank (PMF-FFT).** The known two-symbol Schmidl–Cox waveform is split
//!    into segments of [`SEGMENT_LEN`] samples. Each segment is correlated with the signal at
//!    every timing position, and a transform across the segment outputs evaluates the *full*
//!    matched filter for every carrier-offset hypothesis at once. The statistic at a position
//!    is the best bin's normalised correlation, 1.0 being a perfect match. This is the
//!    partial-matched-filter/FFT acquisition used in GNSS and burst modems: it buys the full
//!    processing gain of the preamble with no time/frequency ambiguity — the sequence is PN,
//!    not a chirp, so a frequency shift is not also a time shift.
//! 2. **Fine offset.** At the winning position the segmented filter is evaluated on a much
//!    finer frequency grid, then refined with the full-symbol-lag phase. The half-symbol
//!    Schmidl–Cox estimate is deliberately *not* used: at low SNR its error occasionally
//!    exceeds the refinement's unambiguous range and aliases.
//! 3. **Frame type.** The bank runs against both sequences and the type is whichever wins,
//!    with the ratio of the two peaks as the confidence. Because that decision rides on the
//!    whole preamble's gain it is essentially error-free wherever a frame is detectable at
//!    all. A DATA frame's mode is read later, from the pilot chips.
//!
//! The OFDM floor family of ADR-0009 — eight-symbol preambles of its own sequences, an
//! averaged floor statistic, the contest between the two families — was retired with
//! ADR-0013: below the OFDM tables is the tone floor, found by
//! [`ToneDetector`](crate::tone::ToneDetector).
//!
//! The threshold is set for sensitivity rather than purity: the statistic's noise maximum is
//! around 0.32, the default is [`DEFAULT_MIN_PEAK`], and an occasional false alarm costs only
//! a failed CRC.

use std::{f64::consts::PI, sync::Arc};

use rustfft::{Fft, FftPlanner, num_complex::Complex64};

use crate::{
    constellation::Complex,
    modes::{AirInterface, PREAMBLE_SYMBOLS, air_interface},
    ofdm::{OfdmDemodulator, OfdmModulator},
    preamble::{FrameType, Preamble},
    rx::FrameSync,
    waveform::{WIDE_2300, WaveformParams},
};

/// Samples per partial correlation.
pub const SEGMENT_LEN: usize = 8;
/// Bins in the coarse frequency transform across segments.
pub const FFT_LEN: usize = 256;
/// Default acceptance threshold on the normalised bank peak.
pub const DEFAULT_MIN_PEAK: f64 = 0.36;
/// Default carrier-offset search range, in hertz.
pub const DEFAULT_MAX_CFO_HZ: f64 = 300.0;

/// What acquisition found, with the diagnostics behind the decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Acquisition {
    /// Where the frame is and what type it is, ready for the receiver.
    pub sync: FrameSync,
    /// Normalised matched-filter peak; 1.0 is a perfect match.
    pub timing_peak: f64,
    /// The winning type's peak over the other type's — how sure the type decision is.
    pub type_confidence: f64,
    /// The coarse bin's frequency estimate, before refinement.
    pub coarse_cfo_hz: f64,
}

/// Finds preambles in a buffer.
pub struct FrameDetector {
    params: WaveformParams,
    air: AirInterface,
    /// Accept a candidate only above this normalised peak.
    pub min_timing_peak: f64,
    /// Search this far either side of zero for the carrier offset.
    pub max_cfo_hz: f64,
    /// Give up after examining this many candidates.
    pub max_candidates: usize,
    period: usize,
    min_gap: usize,
    /// Segmented, unit-energy reference waveform per frame type.
    references: Vec<Vec<Complex>>,
    n_segments: usize,
    reference_len: usize,
    bin_hz: Vec<f64>,
    usable_bins: Vec<usize>,
    coarse_fft: Arc<dyn Fft<f64>>,
    fine_fft: Arc<dyn Fft<f64>>,
    fft_offset: usize,
}

impl std::fmt::Debug for FrameDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameDetector")
            .field("min_timing_peak", &self.min_timing_peak)
            .field("max_cfo_hz", &self.max_cfo_hz)
            .finish_non_exhaustive()
    }
}

const FRAME_TYPES: [FrameType; 2] = [FrameType::Data, FrameType::Control];
/// How much finer the refinement grid is than the coarse one.
const FINE_FACTOR: usize = 16;

impl Default for FrameDetector {
    fn default() -> Self {
        Self::new(WIDE_2300)
    }
}

/// A unit-energy two-symbol reference waveform, segmented for the bank.
fn reference_waveform(
    modulator: &OfdmModulator,
    values: Vec<Complex>,
    period: usize,
) -> Vec<Complex> {
    let waveform = modulator.modulate(&[values.clone(), values]);
    let mut wave: Vec<Complex> = waveform[..2 * period].to_vec();
    let energy: f64 = wave
        .iter()
        .map(|&(re, im)| re * re + im * im)
        .sum::<f64>()
        .sqrt();
    if energy > 0.0 {
        for sample in &mut wave {
            sample.0 /= energy;
            sample.1 /= energy;
        }
    }
    let segments = wave.len() / SEGMENT_LEN;
    wave.truncate(segments * SEGMENT_LEN);
    wave
}

impl FrameDetector {
    /// Build the detector for a numerology.
    #[must_use]
    pub fn new(params: WaveformParams) -> Self {
        let modulator = OfdmModulator::new(params);
        let preamble = Preamble::new(params);
        let period = params.symbol_samples();
        let air = air_interface(params);

        let references: Vec<Vec<Complex>> = FRAME_TYPES
            .iter()
            .map(|&ft| reference_waveform(&modulator, preamble.sc_values(ft), period))
            .collect();
        let reference_len = references[0].len();
        let n_segments = reference_len / SEGMENT_LEN;

        // Bin frequencies of a transform whose samples are SEGMENT_LEN apart.
        let spacing = params.fs_baseband / SEGMENT_LEN as f64;
        let bin_hz: Vec<f64> = (0..FFT_LEN)
            .map(|k| {
                let signed = if k <= FFT_LEN / 2 {
                    k as f64
                } else {
                    k as f64 - FFT_LEN as f64
                };
                signed * spacing / FFT_LEN as f64
            })
            .collect();
        let usable_bins: Vec<usize> = (0..FFT_LEN)
            .filter(|&k| bin_hz[k].abs() <= DEFAULT_MAX_CFO_HZ)
            .collect();

        let mut planner = FftPlanner::new();
        Self {
            params,
            air,
            min_timing_peak: air.acquisition_threshold,
            max_cfo_hz: DEFAULT_MAX_CFO_HZ,
            max_candidates: 16,
            period,
            min_gap: 4 * period,
            references,
            n_segments,
            reference_len,
            bin_hz,
            usable_bins,
            coarse_fft: planner.plan_fft_forward(FFT_LEN),
            fine_fft: planner.plan_fft_forward(FINE_FACTOR * FFT_LEN),
            fft_offset: OfdmDemodulator::new(params).fft_offset(),
        }
    }

    /// Length of the reference waveform the bank correlates against.
    #[must_use]
    pub fn reference_len(&self) -> usize {
        self.reference_len
    }

    /// The segmented partial correlations of one reference at one position, written into
    /// `out` (one entry per segment) so the bank allocates nothing per position.
    fn partials_into(
        samples: &[Complex],
        position: usize,
        reference: &[Complex],
        out: &mut [Complex64],
    ) {
        for (segment, slot) in out.iter_mut().enumerate() {
            let mut accumulator = Complex64::new(0.0, 0.0);
            for offset in 0..SEGMENT_LEN {
                let sample = samples[position + segment * SEGMENT_LEN + offset];
                let r = reference[segment * SEGMENT_LEN + offset];
                // correlate: sample * conj(reference)
                accumulator += Complex64::new(
                    sample.0 * r.0 + sample.1 * r.1,
                    sample.1 * r.0 - sample.0 * r.1,
                );
            }
            *slot = accumulator;
        }
    }

    /// The running state for a bank fed one stream of positions in order.
    #[must_use]
    pub fn bank_state(&self) -> BankState {
        BankState {
            spectrum: vec![Complex64::new(0.0, 0.0); FFT_LEN],
            parts: vec![Complex64::new(0.0, 0.0); self.n_segments],
        }
    }

    /// The transform of the partials now in `state.parts`, and its strongest usable bin:
    /// the bin's magnitude and index.
    fn top_bin(&self, state: &mut BankState) -> (f64, usize) {
        state.spectrum[..self.n_segments].copy_from_slice(&state.parts);
        state.spectrum[self.n_segments..].fill(Complex64::new(0.0, 0.0));
        self.coarse_fft.process(&mut state.spectrum);
        let mut top = 0.0f64;
        let mut top_bin = 0usize;
        for &bin in &self.usable_bins {
            let magnitude = state.spectrum[bin].norm();
            if magnitude > top {
                top = magnitude;
                top_bin = bin;
            }
        }
        (top, top_bin)
    }

    /// One position's bank result. `position` indexes `samples` (its reference window must
    /// be in them) and `energy` is the reference window's normalisation, already floored.
    pub fn bank_row(
        &self,
        state: &mut BankState,
        samples: &[Complex],
        position: usize,
        energy: f64,
    ) -> BankRow {
        let mut best = [0.0f64; FRAME_TYPES.len()];
        let mut best_cfo = [0.0f64; FRAME_TYPES.len()];
        for (type_index, reference) in self.references.iter().enumerate() {
            Self::partials_into(samples, position, reference, &mut state.parts);
            // Every bin of the transform is a sum of the same partial correlations with
            // unit-magnitude phases, so no bin can exceed the sum of their magnitudes.
            // Where that bound is already under the threshold there is nothing to find and
            // the transform can be skipped. This is exact rather than a heuristic - it can
            // never discard a position that would have passed - and the bound is loose
            // (about eight times the peak on noise), so it skips only the quietest
            // positions: worth about 38 % of the bank's time, not an order of magnitude.
            let bound: f64 = state.parts.iter().map(|p| p.norm()).sum();
            if bound / energy < self.min_timing_peak {
                continue;
            }
            let (top, top_bin) = self.top_bin(state);
            best[type_index] = top / energy;
            best_cfo[type_index] = self.bin_hz[top_bin];
        }
        let (win, lose) = if best[0] >= best[1] { (0, 1) } else { (1, 0) };
        BankRow {
            peak: best[win],
            cfo: best_cfo[win],
            other: best[lose],
            winner: win,
        }
    }

    /// The matched-filter bank: for every start position, the best normalised peak over both
    /// frame types, the offset of the winning bin, the other type's peak, and which type won.
    /// Each position is one [`bank_row`](Self::bank_row); the streaming receiver computes
    /// the same rows once each and keeps them, rather than calling this over its whole
    /// lookback.
    #[must_use]
    pub fn bank(&self, samples: &[Complex]) -> BankOutput {
        if samples.len() < self.reference_len {
            return BankOutput::default();
        }
        let positions = samples.len() - self.reference_len + 1;
        // running energy over the reference window, for normalisation
        let mut cumulative = vec![0.0f64; samples.len() + 1];
        for (index, &(re, im)) in samples.iter().enumerate() {
            cumulative[index + 1] = cumulative[index] + re * re + im * im;
        }
        let mean_power = cumulative[samples.len()] / samples.len() as f64;
        let floor = 1e-3 * mean_power.sqrt() * (self.reference_len as f64).sqrt();

        let mut state = self.bank_state();
        let mut output = BankOutput::sized(positions);
        for position in 0..positions {
            let energy = (cumulative[position + self.reference_len] - cumulative[position])
                .max(1e-30)
                .sqrt()
                .max(floor);
            let row = self.bank_row(&mut state, samples, position, energy);
            output.set(position, &row);
        }
        output
    }

    /// Carrier offset at a known preamble position: the segmented filter on a fine grid,
    /// then the full-symbol-lag phase to refine it.
    ///
    /// # Panics
    /// If the buffer does not hold two whole symbols from `start`.
    #[must_use]
    pub fn fine_cfo(&self, samples: &[Complex], start: usize, frame_type: FrameType) -> f64 {
        let type_index = FRAME_TYPES
            .iter()
            .position(|&t| t == frame_type)
            .expect("known type");
        let reference = &self.references[type_index];
        let preamble_symbols = PREAMBLE_SYMBOLS;
        assert!(
            start + preamble_symbols * self.period <= samples.len(),
            "fine_cfo needs the whole preamble from the start position"
        );

        let reps = preamble_symbols / 2;
        let n_fine = FINE_FACTOR * FFT_LEN;
        let mut spectrum = vec![Complex64::new(0.0, 0.0); n_fine];
        for segment in 0..reps * self.n_segments {
            let mut accumulator = Complex64::new(0.0, 0.0);
            for offset in 0..SEGMENT_LEN {
                let sample = samples[start + segment * SEGMENT_LEN + offset];
                let r = reference[(segment % self.n_segments) * SEGMENT_LEN + offset];
                accumulator += Complex64::new(
                    sample.0 * r.0 + sample.1 * r.1,
                    sample.1 * r.0 - sample.0 * r.1,
                );
            }
            spectrum[segment] = accumulator;
        }
        self.fine_fft.process(&mut spectrum);

        let spacing = self.params.fs_baseband / SEGMENT_LEN as f64;
        let mut best = 0.0f64;
        let mut coarse = 0.0f64;
        for (k, value) in spectrum.iter().enumerate() {
            let signed = if k <= n_fine / 2 {
                k as f64
            } else {
                k as f64 - n_fine as f64
            };
            let frequency = signed * spacing / n_fine as f64;
            if frequency.abs() > self.max_cfo_hz {
                continue;
            }
            let magnitude = value.norm();
            if magnitude > best {
                best = magnitude;
                coarse = frequency;
            }
        }

        // Refine with the phase between consecutive identical preamble symbols.
        let rotate = |sample: Complex, t: f64| {
            let phase = -2.0 * PI * coarse * t;
            Complex64::new(
                sample.0 * phase.cos() - sample.1 * phase.sin(),
                sample.0 * phase.sin() + sample.1 * phase.cos(),
            )
        };
        let mut correlation = Complex64::new(0.0, 0.0);
        for lag in 0..preamble_symbols - 1 {
            for index in 0..self.period {
                let i0 = start + lag * self.period + index;
                let i1 = i0 + self.period;
                let a = rotate(samples[i0], i0 as f64 / self.params.fs_baseband);
                let b = rotate(samples[i1], i1 as f64 / self.params.fs_baseband);
                correlation += a.conj() * b;
            }
        }
        let residual = correlation.im.atan2(correlation.re) * self.params.fs_baseband
            / (2.0 * PI * self.period as f64);
        coarse + residual
    }

    /// How far a streaming receiver searches back behind what it has already searched, so
    /// that every candidate is decided in a region that holds all it depends on: a symbol
    /// past the preamble.
    #[must_use]
    pub fn stream_lookback(&self) -> usize {
        (PREAMBLE_SYMBOLS + 1) * self.period
    }

    // ── full acquisition ──────────────────────────────────────────────

    /// Find up to `max_frames` preambles in a buffer.
    ///
    /// # Panics
    /// If the bank produced a non-finite statistic, which would mean the input contained NaN.
    #[must_use]
    pub fn detect(&self, samples: &[Complex], max_frames: usize) -> Vec<Acquisition> {
        let output = self.bank(samples);
        self.detect_with(samples, &output, max_frames)
    }

    /// [`detect`](Self::detect) over a bank already computed for `samples` — for a streaming
    /// receiver, the rows it keeps, a row per position each computed once rather than the
    /// bank re-run over the whole lookback on every block.
    ///
    /// # Panics
    /// As [`detect`](Self::detect).
    #[must_use]
    pub fn detect_with(
        &self,
        samples: &[Complex],
        output: &BankOutput,
        max_frames: usize,
    ) -> Vec<Acquisition> {
        let mut ordinary: Vec<Acquisition> = Vec::new();
        if output.peak.is_empty() {
            return ordinary;
        }
        let mut eligible: Vec<bool> = output
            .peak
            .iter()
            .map(|&p| p >= self.min_timing_peak)
            .collect();

        // The two preamble symbols are identical, so a preamble preceded by silence also
        // produces a strong sidelobe one symbol early. Never accept a peak until the
        // statistic one symbol later exists, so the real peak can win.
        let tail = eligible.len().saturating_sub(self.period);
        for slot in &mut eligible[tail..] {
            *slot = false;
        }

        for _ in 0..self.max_candidates {
            if ordinary.len() >= max_frames {
                break;
            }
            let Some(start) = eligible
                .iter()
                .enumerate()
                .filter(|&(_, &ok)| ok)
                .max_by(|a, b| {
                    output.peak[a.0]
                        .partial_cmp(&output.peak[b.0])
                        .expect("finite peaks")
                })
                .map(|(index, _)| index)
            else {
                break;
            };
            let reject_low = start.saturating_sub(self.period);
            let reject_high = (start + self.period).min(eligible.len());

            let later = start + self.period;
            if later < output.peak.len() && output.peak[later] > output.peak[start] {
                // this was the early sidelobe; the real one is a symbol later
                eligible[reject_low..reject_high].fill(false);
                continue;
            }
            if start + 4 * self.period + self.fft_offset > samples.len() {
                // too close to the end to be usable yet
                eligible[reject_low..reject_high].fill(false);
                continue;
            }
            let frame_type = FRAME_TYPES[output.winner[start]];
            let cfo_hz = self.fine_cfo(samples, start, frame_type);
            ordinary.push(Acquisition {
                sync: FrameSync {
                    start,
                    cfo_hz,
                    frame_type,
                    timing_peak: output.peak[start],
                    type_confidence: output.peak[start] / output.other[start].max(1e-12),
                },
                timing_peak: output.peak[start],
                type_confidence: output.peak[start] / output.other[start].max(1e-12),
                coarse_cfo_hz: output.cfo[start],
            });

            // Nothing else can start inside this frame: strong data symbols correlate with
            // the reference well enough to pass the threshold on their own.
            let span = match frame_type {
                FrameType::Data => self.air.long.samples(),
                FrameType::Control => self.air.short.samples(),
            };
            let low = start.saturating_sub(self.min_gap);
            let high = (start + span).min(eligible.len());
            eligible[low..high].fill(false);
        }

        ordinary.sort_by_key(|acquisition| acquisition.sync.start);
        ordinary
    }
}

/// What [`FrameDetector::bank`] produced, one entry per start position.
#[derive(Debug, Clone, Default)]
pub struct BankOutput {
    /// Best normalised peak over both frame types.
    pub peak: Vec<f64>,
    /// Carrier offset of the winning bin, in hertz.
    pub cfo: Vec<f64>,
    /// The losing frame type's peak at the same position.
    pub other: Vec<f64>,
    /// Which frame type won: an index into the detector's type list.
    pub winner: Vec<usize>,
}

impl BankOutput {
    /// Zeroed output for `positions` positions.
    #[must_use]
    pub fn sized(positions: usize) -> Self {
        Self {
            peak: vec![0.0; positions],
            cfo: vec![0.0; positions],
            other: vec![0.0; positions],
            winner: vec![0; positions],
        }
    }

    /// The output over a run of rows, in order.
    pub fn from_rows<'a>(rows: impl Iterator<Item = &'a BankRow>) -> Self {
        let mut out = Self::default();
        for row in rows {
            out.peak.push(row.peak);
            out.cfo.push(row.cfo);
            out.other.push(row.other);
            out.winner.push(row.winner);
        }
        out
    }

    fn set(&mut self, position: usize, row: &BankRow) {
        self.peak[position] = row.peak;
        self.cfo[position] = row.cfo;
        self.other[position] = row.other;
        self.winner[position] = row.winner;
    }
}

/// One position's bank result.
#[derive(Debug, Clone, Copy, Default)]
pub struct BankRow {
    /// Best normalised peak over both frame types.
    pub peak: f64,
    /// Carrier offset of the winning bin, in hertz.
    pub cfo: f64,
    /// The losing frame type's peak.
    pub other: f64,
    /// Which frame type won.
    pub winner: usize,
}

/// The bank's scratch space, so a position allocates nothing; one per stream, from
/// [`FrameDetector::bank_state`].
#[derive(Debug, Clone)]
pub struct BankState {
    spectrum: Vec<Complex64>,
    parts: Vec<Complex64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        codec::FrameCodec,
        constellation::NoiseVar,
        modes::{LONG, MODES, SHORT},
        preamble::FrameHeader,
        rx::FrameReceiver,
        tx::FrameTransmitter,
    };

    fn frame_at(lead: usize, mode_index: usize, rv: u8) -> (Vec<Complex>, Vec<u8>) {
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(MODES[mode_index], LONG).expect("codec");
        let payload: Vec<u8> = (0..codec.payload_bytes)
            .map(|i| ((i * 17) % 256) as u8)
            .collect();
        let qam = codec.encode(&payload, rv).expect("encode");
        let header = FrameHeader::new(FrameType::Data, mode_index, rv).expect("header");
        let frame = tx.baseband(&header, &LONG, &qam).expect("baseband");
        let mut buffer = vec![(0.0, 0.0); lead];
        buffer.extend_from_slice(&frame);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 2000));
        (buffer, payload)
    }

    fn rotate(samples: &[Complex], cfo_hz: f64) -> Vec<Complex> {
        samples
            .iter()
            .enumerate()
            .map(|(index, &(re, im))| {
                let t = index as f64 / WIDE_2300.fs_baseband;
                let phase = 2.0 * PI * cfo_hz * t;
                (
                    re * phase.cos() - im * phase.sin(),
                    re * phase.sin() + im * phase.cos(),
                )
            })
            .collect()
    }

    #[test]
    fn a_clean_frame_is_found_at_the_right_sample() {
        let detector = FrameDetector::default();
        for lead in [400usize, 1000, 1731] {
            let (buffer, _) = frame_at(lead, 4, 0);
            let found = detector.detect(&buffer, 2);
            assert_eq!(found.len(), 1, "lead {lead}: found {} frames", found.len());
            assert_eq!(found[0].sync.start, lead, "lead {lead}");
            assert!(found[0].timing_peak > 0.9, "peak {}", found[0].timing_peak);
            assert_eq!(found[0].sync.frame_type, FrameType::Data);
            assert!(
                found[0].type_confidence > 2.0,
                "confidence {}",
                found[0].type_confidence
            );
        }
    }

    #[test]
    fn the_frame_type_is_read_from_the_preamble() {
        let detector = FrameDetector::default();
        let tx = FrameTransmitter::default();
        let codec = FrameCodec::new(crate::modes::CONTROL_MODE, SHORT).expect("codec");
        let payload = vec![1u8, 2, 3, 4, 5, 6, 7];
        let qam = codec.encode(&payload, 0).expect("encode");
        let frame = tx
            .baseband(&FrameHeader::control(), &SHORT, &qam)
            .expect("baseband");
        let mut buffer = vec![(0.0, 0.0); 500];
        buffer.extend_from_slice(&frame);
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 2000));

        let found = detector.detect(&buffer, 2);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sync.frame_type, FrameType::Control);
        assert_eq!(found[0].sync.start, 500);
    }

    #[test]
    fn a_carrier_offset_is_estimated_across_the_search_range() {
        let detector = FrameDetector::default();
        for offset in [-250.0, -97.5, -12.0, 0.0, 33.0, 128.0, 250.0] {
            let (buffer, _) = frame_at(600, 0, 0);
            let rotated = rotate(&buffer, offset);
            let found = detector.detect(&rotated, 1);
            assert_eq!(found.len(), 1, "offset {offset}");
            assert_eq!(found[0].sync.start, 600, "offset {offset}");
            assert!(
                (found[0].sync.cfo_hz - offset).abs() < 1.0,
                "offset {offset}: estimated {}",
                found[0].sync.cfo_hz
            );
        }
    }

    #[test]
    fn acquisition_feeds_the_receiver_end_to_end() {
        // the whole chain in Rust: find the frame, demodulate it, decode it
        let detector = FrameDetector::default();
        let receiver = FrameReceiver::default();
        let (buffer, payload) = frame_at(777, 8, 0);
        let rotated = rotate(&buffer, 41.0);

        let found = detector.detect(&rotated, 1);
        assert_eq!(found.len(), 1);
        let frame = receiver
            .receive(&rotated, &found[0].sync, None)
            .expect("receive");
        assert_eq!(frame.mode, 8);
        let codec = FrameCodec::new(MODES[8], LONG).expect("codec");
        let (decoded, _) = codec
            .decode(
                &frame.symbols,
                NoiseVar::PerSymbol(&frame.noise_var),
                0,
                None,
            )
            .expect("decode");
        assert_eq!(decoded.as_deref(), Some(&payload[..]));
    }

    #[test]
    fn silence_produces_no_detections() {
        let detector = FrameDetector::default();
        let quiet = vec![(0.0, 0.0); 20_000];
        assert!(detector.detect(&quiet, 4).is_empty());
    }

    #[test]
    fn a_buffer_shorter_than_the_reference_is_handled() {
        let detector = FrameDetector::default();
        assert!(detector.detect(&[(0.0, 0.0); 10], 1).is_empty());
        assert!(detector.bank(&[(0.0, 0.0); 10]).peak.is_empty());
    }

    #[test]
    fn two_frames_in_one_buffer_are_both_found() {
        let detector = FrameDetector::default();
        let (first, _) = frame_at(400, 4, 0);
        let (second, _) = frame_at(0, 2, 1);
        let mut buffer = first[..400 + LONG.samples() + 8].to_vec();
        buffer.extend(std::iter::repeat_n((0.0, 0.0), 300));
        let offset = buffer.len();
        buffer.extend_from_slice(&second);

        let found = detector.detect(&buffer, 4);
        assert_eq!(
            found.len(),
            2,
            "found {:?}",
            found.iter().map(|f| f.sync.start).collect::<Vec<_>>()
        );
        assert_eq!(found[0].sync.start, 400);
        assert_eq!(found[1].sync.start, offset);
    }
}
