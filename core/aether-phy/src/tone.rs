//! The tone floor (ADR-0013): a steady-envelope sixteen-tone FSK frame family under both
//! ladders — the port of `model/aether_model/phy/tone.py` and the tone codec of
//! `frame/codec.py`.
//!
//! One tone at a time, with a continuous phase: the envelope is constant, so a tone frame goes
//! out at the OFDM frames' peak amplitude ([`gain_db`] above their average) and the
//! transmitter's whole peak power reaches the air; and it is detected by energy, so it needs
//! no channel estimate. Three sync blocks of eight symbols a frame — start, after 45 % of the
//! data, end — each a Costas sequence whose pattern names the frame's kind and redundancy
//! version; the codeword is the OFDM frames' (CRC-24, TS 38.212 LDPC with rate matching, the
//! golden-ratio interleaver) ending in four Gray-labelled bits a tone.
//!
//! The numerology, sync patterns and frame kinds are the model's, compiled in from
//! `data/preamble_tables.json`; `tests/model_vectors.rs` checks the tones exactly and the
//! waveform, the detector's offsets and the soft bits to a tolerance.

use std::{f64::consts::PI, sync::Arc};

use aether_fec::{
    CRC24A,
    ldpc::{FecError, NrLdpcCode, select_base_graph, select_lifting_size},
    rate_match::RateMatcher,
};
use rustfft::{Fft, FftPlanner, num_complex::Complex64};

use crate::{codec::coprime_stride, constellation::Complex, tables::TONE};

pub use crate::tables::ToneKind;

/// Symbols in a sync block.
pub const SYNC_SYMBOLS: usize = 8;

/// Power of a tone-floor frame above an OFDM frame's average at the same transmit level: the
/// tone goes out at the OFDM frames' peak. Every SNR the tone floor reports is taken back by
/// this much, so the link layer compares both families in one currency.
#[must_use]
pub fn gain_db() -> f64 {
    TONE.gain_db
}

/// The control frame: acknowledgements and the other control frames on the floor.
#[must_use]
pub fn control_kind() -> &'static ToneKind {
    &TONE.control
}

/// The data kinds, slowest first: the ladder's first rungs.
#[must_use]
pub fn data_kinds() -> &'static [ToneKind] {
    TONE.data
}

/// Every kind: the control frame, then the data kinds.
#[must_use]
pub fn kinds() -> Vec<&'static ToneKind> {
    std::iter::once(control_kind())
        .chain(data_kinds().iter())
        .collect()
}

/// Sample rate of the complex baseband the tone floor is defined on.
#[must_use]
pub fn fs() -> f64 {
    TONE.fs
}

/// Samples in a symbol (40 ms).
#[must_use]
pub fn symbol_samples() -> usize {
    TONE.symbol_samples
}

/// Tones in the family.
#[must_use]
pub fn tones() -> usize {
    TONE.tones
}

/// Coded bits a symbol carries.
#[must_use]
pub fn bits_per_symbol() -> usize {
    TONE.tones.trailing_zeros() as usize
}

/// A symbol's length in seconds.
#[must_use]
pub fn symbol_s() -> f64 {
    TONE.symbol_samples as f64 / TONE.fs
}

/// Tone spacing, the symbol rate: 25 Hz.
#[must_use]
pub fn spacing_hz() -> f64 {
    TONE.fs / TONE.symbol_samples as f64
}

/// Offset of tone `k` from the passband centre.
#[must_use]
pub fn tone_hz(k: usize) -> f64 {
    (k as f64 - (TONE.tones as f64 - 1.0) / 2.0) * spacing_hz()
}

/// Symbol energy over noise density per unit 3 kHz SNR.
#[must_use]
pub fn snr_scale() -> f64 {
    3000.0 * symbol_s()
}

/// How long after a tone frame starts the stream has announced it: its first sync block and
/// the neighbourhood it must beat, plus `receiver_s` for the stream's own lateness (the
/// impulse blanker, the band filter and a block) — 0.54 s with the model's 0.1 s.
#[must_use]
pub fn announce_delay_s(receiver_s: f64) -> f64 {
    let lookahead = TONE.announce_lookahead / TONE.hop_div;
    (SYNC_SYMBOLS + lookahead) as f64 * symbol_s() + receiver_s
}

impl ToneKind {
    /// `K'`: payload plus CRC.
    #[must_use]
    pub fn info_bits(&self) -> usize {
        self.payload_bytes * 8 + CRC24A.width as usize
    }

    /// Coded bits the data symbols carry.
    #[must_use]
    pub fn coded_bits(&self) -> usize {
        self.data_symbols * bits_per_symbol()
    }

    /// The code rate.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.info_bits() as f64 / self.coded_bits() as f64
    }

    /// Base graph chosen by TS 38.212 §7.2.2.
    #[must_use]
    pub fn base_graph(&self) -> u8 {
        select_base_graph(self.payload_bytes * 8, self.rate())
    }

    /// Symbols in the frame: data and three sync blocks.
    #[must_use]
    pub fn symbols(&self) -> usize {
        self.data_symbols + 3 * SYNC_SYMBOLS
    }

    /// The frame's length in samples.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.symbols() * TONE.symbol_samples
    }

    /// The frame's length in seconds.
    #[must_use]
    pub fn duration_s(&self) -> f64 {
        self.symbols() as f64 * symbol_s()
    }

    /// Payload bits per second of air time.
    #[must_use]
    pub fn net_bps(&self) -> f64 {
        8.0 * self.payload_bytes as f64 / self.duration_s()
    }

    /// First symbol of each sync block: start, middle, end. The data is split 45/55 so the
    /// three distances between blocks differ and no shift lines up more than one block.
    #[must_use]
    pub fn block_offsets(&self) -> [usize; 3] {
        let first = self.data_symbols * 9 / 20;
        [0, SYNC_SYMBOLS + first, self.symbols() - SYNC_SYMBOLS]
    }

    /// The sync block's tones at a redundancy version.
    #[must_use]
    pub fn sync(&self, rv: usize) -> &'static [usize; 8] {
        &TONE.sync_patterns[self.patterns[rv]]
    }

    /// Each symbol's role: `None` a data symbol, otherwise the sync tone.
    #[must_use]
    pub fn layout(&self, rv: usize) -> Vec<Option<usize>> {
        let mut out = vec![None; self.symbols()];
        for offset in self.block_offsets() {
            for (j, &tone) in self.sync(rv).iter().enumerate() {
                out[offset + j] = Some(tone);
            }
        }
        out
    }

    /// Redundancy versions the kind signals: one pattern each.
    #[must_use]
    pub fn n_rv(&self) -> usize {
        self.patterns.len()
    }
}

// ── the codec ──────────────────────────────────────────────────────────

/// Tone `t`'s Gray label: neighbouring tones differ in one bit.
const fn gray(t: usize) -> usize {
    t ^ (t >> 1)
}

/// Payload ↔ data tones for one kind, and the soft inverse.
#[derive(Debug, Clone)]
pub struct ToneCodec {
    kind: &'static ToneKind,
    code: NrLdpcCode,
    /// `permutation[k]` is where coded bit `k` is transmitted.
    permutation: Vec<usize>,
    /// The tone whose Gray label is `label`.
    tone_of_label: Vec<usize>,
}

impl ToneCodec {
    /// Build the codec.
    ///
    /// # Errors
    /// If the information block does not fit the selected lifting size.
    pub fn new(kind: &'static ToneKind) -> Result<Self, FecError> {
        let bg = kind.base_graph();
        let z = select_lifting_size(bg, kind.info_bits())?;
        let code = NrLdpcCode::new(bg, z)?;
        if kind.info_bits() > code.k {
            return Err(FecError::BadLength {
                expected: code.k,
                got: kind.info_bits(),
            });
        }
        let n = kind.coded_bits();
        let stride = coprime_stride(n);
        let permutation = (0..n).map(|k| (k * stride) % n).collect();
        let mut tone_of_label = vec![0usize; tones()];
        for t in 0..tones() {
            tone_of_label[gray(t)] = t;
        }
        Ok(Self {
            kind,
            code,
            permutation,
            tone_of_label,
        })
    }

    /// The kind this codec serves.
    #[must_use]
    pub fn kind(&self) -> &'static ToneKind {
        self.kind
    }

    /// The data symbols' tones.
    ///
    /// # Errors
    /// If the payload is not the kind's length.
    pub fn encode(&self, payload: &[u8], rv: u8) -> Result<Vec<usize>, FecError> {
        let kind = self.kind;
        if payload.len() != kind.payload_bytes {
            return Err(FecError::BadLength {
                expected: kind.payload_bytes,
                got: payload.len(),
            });
        }
        let mut bits = Vec::with_capacity(payload.len() * 8);
        for &byte in payload {
            for shift in (0..8).rev() {
                bits.push((byte >> shift) & 1);
            }
        }
        let mut info = CRC24A.attach(&bits);
        info.resize(self.code.k, 0);
        let codeword = self.code.encode(&info)?;
        let matcher = RateMatcher::new(&self.code, kind.info_bits(), kind.coded_bits(), rv)?;
        let selected = matcher.match_bits(&codeword)?;
        let mut interleaved = vec![0u8; kind.coded_bits()];
        for (k, &bit) in selected.iter().enumerate() {
            interleaved[self.permutation[k]] = bit;
        }
        let m = bits_per_symbol();
        Ok(interleaved
            .chunks_exact(m)
            .map(|group| {
                let label = group
                    .iter()
                    .fold(0usize, |acc, &b| (acc << 1) | usize::from(b & 1));
                self.tone_of_label[label]
            })
            .collect())
    }

    /// Soft bits (positive = 0) of the data symbols, in transmission order → payload, or
    /// `None` if the check fails, and the codeword's LLR buffer, which a retransmission at
    /// another redundancy version combines with (HARQ-IR).
    ///
    /// # Errors
    /// If `llr` is not the kind's coded-bit count.
    pub fn decode(
        &self,
        llr: &[f64],
        rv: u8,
        buffer: Option<&[f64]>,
    ) -> Result<(Option<Vec<u8>>, Vec<f64>), FecError> {
        let kind = self.kind;
        if llr.len() != kind.coded_bits() {
            return Err(FecError::BadLength {
                expected: kind.coded_bits(),
                got: llr.len(),
            });
        }
        let llr_e: Vec<f64> = self.permutation.iter().map(|&p| llr[p]).collect();
        let matcher = RateMatcher::new(&self.code, kind.info_bits(), kind.coded_bits(), rv)?;
        let full = matcher.recover(&llr_e, buffer)?;
        let decoded = self.code.decode(&full, 30, 0.8)?;
        let block = &decoded.bits[..kind.info_bits()];
        // the all-zero word passes every linear code and its CRC: refused, as FrameCodec does
        if !CRC24A.check(block) || block.iter().all(|&bit| bit == 0) {
            return Ok((None, full));
        }
        let payload = block[..block.len() - CRC24A.width as usize]
            .chunks_exact(8)
            .map(|chunk| chunk.iter().fold(0u8, |acc, &b| (acc << 1) | (b & 1)))
            .collect();
        Ok((Some(payload), full))
    }
}

// ── modulation ─────────────────────────────────────────────────────────

/// Continuous-phase FSK: the frequency glides between tones over the ramp on a raised
/// cosine, the phase accumulates, and only the frame's own fade in and out touches the
/// amplitude — a constant envelope, at `gain_db` above unit power.
#[must_use]
pub fn modulate(tones: &[usize], gain_db: f64) -> Vec<Complex> {
    let n = TONE.symbol_samples;
    let hz: Vec<f64> = tones.iter().map(|&t| tone_hz(t)).collect();
    let mut freq: Vec<f64> = hz.iter().flat_map(|&f| std::iter::repeat_n(f, n)).collect();
    let ramp = TONE.ramp_samples;
    if ramp > 0 && tones.len() > 1 {
        let shape: Vec<f64> = (0..ramp)
            .map(|k| 0.5 - 0.5 * (PI * (k as f64 + 0.5) / ramp as f64).cos())
            .collect();
        for i in 1..tones.len() {
            if tones[i] == tones[i - 1] {
                continue;
            }
            let start = i * n - ramp / 2;
            let (a, b) = (hz[i - 1], hz[i]);
            for (k, &s) in shape.iter().enumerate() {
                freq[start + k] = a + (b - a) * s;
            }
        }
    }
    let scale = 2.0 * PI / TONE.fs;
    let gain = 10f64.powf(gain_db / 20.0);
    let mut accumulated = 0.0f64;
    let mut out = Vec::with_capacity(freq.len());
    for &f in &freq {
        let phase = scale * accumulated;
        out.push((phase.cos() * gain, phase.sin() * gain));
        accumulated += f;
    }
    let edge = TONE.edge_samples;
    if edge > 0 && out.len() >= 2 * edge {
        let len = out.len();
        for k in 0..edge {
            let w = 0.5 - 0.5 * (PI * (k as f64 + 0.5) / edge as f64).cos();
            out[k].0 *= w;
            out[k].1 *= w;
            out[len - 1 - k].0 *= w;
            out[len - 1 - k].1 *= w;
        }
    }
    out
}

/// The whole frame's tones: the sync blocks with the data between them.
///
/// # Panics
/// If `data` is shorter than the kind's data symbols.
#[must_use]
pub fn frame_tones(kind: &ToneKind, data: &[usize], rv: usize) -> Vec<usize> {
    let mut data = data.iter();
    kind.layout(rv)
        .into_iter()
        .map(|role| role.unwrap_or_else(|| *data.next().expect("enough data tones")))
        .collect()
}

/// A frame as the transmitter sends it: [`gain_db`] above an OFDM frame.
///
/// # Errors
/// If the payload is not the kind's length.
pub fn burst(codec: &ToneCodec, payload: &[u8], rv: u8) -> Result<Vec<Complex>, FecError> {
    let data = codec.encode(payload, rv)?;
    Ok(modulate(
        &frame_tones(codec.kind(), &data, usize::from(rv)),
        gain_db(),
    ))
}

// ── demodulation ───────────────────────────────────────────────────────

/// Energy of every tone in every symbol of a frame starting at `start` with carrier offset
/// `cfo_hz`, symbol-major (`symbols × tones`); white noise of unit power per sample gives one.
/// `None` if the frame runs past the buffer.
#[must_use]
pub fn tone_energies(
    samples: &[Complex],
    start: usize,
    cfo_hz: f64,
    symbols: usize,
) -> Option<Vec<f64>> {
    let n = TONE.symbol_samples;
    if start + symbols * n > samples.len() {
        return None;
    }
    let tones = TONE.tones;
    // the references: exp(−2πj·(f_k + cfo)·t)
    let mut reference = vec![(0.0f64, 0.0f64); tones * n];
    for k in 0..tones {
        let omega = -2.0 * PI * (tone_hz(k) + cfo_hz);
        for i in 0..n {
            let phase = omega * (i as f64 / TONE.fs);
            reference[k * n + i] = (phase.cos(), phase.sin());
        }
    }
    let mut out = vec![0.0f64; symbols * tones];
    for s in 0..symbols {
        let segment = &samples[start + s * n..start + (s + 1) * n];
        for k in 0..tones {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &(x, y)) in segment.iter().enumerate() {
                let (c, d) = reference[k * n + i];
                re += x * c - y * d;
                im += x * d + y * c;
            }
            out[s * tones + k] = (re * re + im * im) / n as f64;
        }
    }
    Some(out)
}

/// The index of the largest value, the first among equals.
fn argmax(values: &[f64]) -> usize {
    let mut best = 0;
    for (i, &v) in values.iter().enumerate() {
        if v > values[best] {
            best = i;
        }
    }
    best
}

/// The median, as `NumPy` takes it: the mean of the two middle values of an even count.
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        values[n / 2]
    } else {
        f64::midpoint(values[n / 2 - 1], values[n / 2])
    }
}

/// The noise energy per bin: the median of the bins that should hold only noise — every tone
/// but the sync tone in a sync symbol, every tone but the strongest in a data symbol — over
/// ln 2, the median of an exponential. A silent symbol (the receiver muted while its station
/// transmitted) measures nothing and is left out: counted, a frame half under the station's
/// own transmission read its noise as zero and its SNR as 290 dB.
#[must_use]
pub fn noise_level(energies: &[f64], layout: &[Option<usize>]) -> f64 {
    let tones = TONE.tones;
    let mut bins = Vec::with_capacity(energies.len());
    for (s, role) in layout.iter().enumerate() {
        let row = &energies[s * tones..(s + 1) * tones];
        if silent(row) {
            continue;
        }
        let skip = role.unwrap_or_else(|| argmax(row));
        bins.extend(
            row.iter()
                .enumerate()
                .filter(|&(k, _)| k != skip)
                .map(|(_, &e)| e),
        );
    }
    if bins.is_empty() {
        return 1e-30;
    }
    (median(&mut bins) / std::f64::consts::LN_2).max(1e-30)
}

/// Whether a symbol's row holds no energy at all: a receiver hears exact silence only while
/// it is muted — its own transmission — and there every tone ties, so it is no evidence of
/// anything.
fn silent(row: &[f64]) -> bool {
    row.iter().all(|&e| e <= 0.0)
}

/// Whether `tone` is the strongest in `row` — the first among equals, as `NumPy`'s argmax —
/// and the row holds energy at all.
fn strongest(row: &[f64], tone: usize) -> bool {
    argmax(row) == tone && row[tone] > 0.0
}

/// The modified Bessel function I0 by its power series, for the small arguments the metric
/// takes it at (at most 3, where the series is exact to the last bit in twenty terms).
fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let mut term = 1.0f64;
    let mut sum = 1.0f64;
    for k in 1..40 {
        term *= q / f64::from(k * k);
        sum += term;
        if term < sum * 1e-18 {
            break;
        }
    }
    sum
}

/// Log-likelihood of each tone in each data symbol (data symbols × tones), up to a
/// per-symbol constant: `log I0(2·sqrt(E·s)/σ)`, with `s` the symbol SNR the sync blocks
/// measure, interpolated between the three blocks.
#[must_use]
pub fn symbol_metrics(energies: &[f64], layout: &[Option<usize>], noise: f64) -> Vec<f64> {
    let tones = TONE.tones;
    let e: Vec<f64> = energies.iter().map(|v| v / noise).collect();
    let sync: Vec<(usize, usize)> = layout
        .iter()
        .enumerate()
        .filter_map(|(s, role)| role.map(|t| (s, t)))
        .collect();
    let mut centres = Vec::new();
    let mut levels = Vec::new();
    for block in sync.chunks(SYNC_SYMBOLS) {
        let centre = block.iter().map(|&(s, _)| s as f64).sum::<f64>() / block.len() as f64;
        let level = block
            .iter()
            .map(|&(s, t)| (e[s * tones + t] - 1.0).max(0.0))
            .sum::<f64>()
            / block.len() as f64;
        centres.push(centre);
        levels.push(level);
    }
    let interp = |x: f64| -> f64 {
        if x <= centres[0] {
            return levels[0];
        }
        let last = centres.len() - 1;
        if x >= centres[last] {
            return levels[last];
        }
        for j in 0..last {
            if x >= centres[j] && x <= centres[j + 1] {
                let slope = (levels[j + 1] - levels[j]) / (centres[j + 1] - centres[j]);
                return slope * (x - centres[j]) + levels[j];
            }
        }
        levels[last]
    };
    let mut out = Vec::new();
    for (s, role) in layout.iter().enumerate() {
        if role.is_some() {
            continue;
        }
        let snr = interp(s as f64).max(0.1);
        for k in 0..tones {
            let arg = 2.0 * (e[s * tones + k] * snr).sqrt();
            out.push(if arg > 3.0 {
                arg - 0.5 * (2.0 * PI * arg).ln() + 1.0 / (8.0 * arg)
            } else {
                bessel_i0(arg).ln()
            });
        }
    }
    out
}

/// `log Σ exp(values)`, the largest factored out.
fn log_sum_exp(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let top = values.clone().fold(f64::NEG_INFINITY, f64::max);
    top + values.map(|v| (v - top).exp()).sum::<f64>().ln()
}

/// Soft bits (positive = 0) from the per-tone metrics: log-sum over the tones labelled 0 less
/// the log-sum over those labelled 1, bit by bit, most significant first.
#[must_use]
pub fn bit_llrs(metrics: &[f64]) -> Vec<f64> {
    let tones = TONE.tones;
    let bits = bits_per_symbol();
    let mut out = Vec::with_capacity(metrics.len() / tones * bits);
    for row in metrics.chunks_exact(tones) {
        for b in 0..bits {
            let shift = bits - 1 - b;
            let zero = (0..tones).filter(|&t| (gray(t) >> shift) & 1 == 0);
            let one = (0..tones).filter(|&t| (gray(t) >> shift) & 1 == 1);
            out.push(log_sum_exp(zero.map(|t| row[t])) - log_sum_exp(one.map(|t| row[t])));
        }
    }
    out
}

/// SNR (3 kHz, the OFDM frames' reference) from a measured symbol energy over noise density.
#[must_use]
pub fn snr_from_es(es_over_n0: f64) -> f64 {
    10.0 * (es_over_n0.max(1e-3) / snr_scale()).log10() - gain_db()
}

/// A tone frame's soft information, ready to decode or to combine.
#[derive(Debug, Clone)]
pub struct ToneFrame {
    /// Its kind.
    pub kind: &'static ToneKind,
    /// The redundancy version its pattern named.
    pub rv: u8,
    /// Soft bits of the data symbols, in transmission order.
    pub llr: Vec<f64>,
    /// SNR (3 kHz) in the OFDM frames' reference: what this path gives an ordinary frame.
    pub snr_db: f64,
}

/// Soft bits and SNR of a frame of `kind` at a known start and carrier offset; `None` if it
/// runs past the buffer.
#[must_use]
pub fn demodulate(
    samples: &[Complex],
    kind: &'static ToneKind,
    rv: u8,
    start: usize,
    cfo_hz: f64,
) -> Option<ToneFrame> {
    let layout = kind.layout(usize::from(rv));
    let energies = tone_energies(samples, start, cfo_hz, kind.symbols())?;
    let noise = noise_level(&energies, &layout);
    let llr = bit_llrs(&symbol_metrics(&energies, &layout, noise));
    let tones = TONE.tones;
    let held: Vec<f64> = layout
        .iter()
        .enumerate()
        .filter_map(|(s, role)| role.map(|t| energies[s * tones + t]))
        .collect();
    let es = held.iter().sum::<f64>() / held.len() as f64 / noise - 1.0;
    Some(ToneFrame {
        kind,
        rv,
        llr,
        snr_db: snr_from_es(es),
    })
}

// ── acquisition ────────────────────────────────────────────────────────

/// A tone frame found in a buffer.
#[derive(Debug, Clone, Copy)]
pub struct ToneSync {
    /// Where the frame starts.
    pub start: usize,
    /// Its carrier offset.
    pub cfo_hz: f64,
    /// Its kind.
    pub kind: &'static ToneKind,
    /// The redundancy version its pattern named.
    pub rv: u8,
    /// Mean normalised energy of the 24 sync tones: 1 on noise, 1 + Es/N0 on a frame.
    pub statistic: f64,
}

impl ToneSync {
    /// How far above the acquisition threshold the frame was found; 1.0 is exactly at it.
    #[must_use]
    pub fn detect_confidence(&self) -> f64 {
        self.statistic / TONE.threshold
    }
}

/// Finds tone frames: a spectrogram at a quarter-symbol hop with bins a quarter of the tone
/// spacing and, for every start, carrier-offset bin and pattern, the mean of the 24 sync
/// tones' ratios to the other fifteen tones in their symbols, each clipped; a candidate is
/// refined and kept if half its sync tones are the strongest in their symbols.
pub struct ToneDetector {
    hop: usize,
    nfft: usize,
    bin_hz: f64,
    cfo_bins: usize,
    /// Tone `k`'s bin, as an offset from the band's lowest column.
    tone_col: Vec<usize>,
    /// Columns in the band a row keeps: the tones, and the offset search either side.
    band: usize,
    fft: Arc<dyn Fft<f64>>,
    /// Every (kind, redundancy version) the patterns can name.
    hypotheses: Vec<(&'static ToneKind, u8)>,
}

impl std::fmt::Debug for ToneDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToneDetector")
            .field("hop", &self.hop)
            .field("cfo_bins", &self.cfo_bins)
            .finish_non_exhaustive()
    }
}

impl Default for ToneDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl ToneDetector {
    /// The detector for every kind.
    ///
    /// # Panics
    /// If the compiled-in sync patterns are not the length the model exported.
    #[must_use]
    pub fn new() -> Self {
        // the patterns are compiled in as eight-symbol arrays; the model's export must agree
        assert_eq!(
            TONE.sync_symbols, SYNC_SYMBOLS,
            "the model's sync blocks are a different length"
        );
        let n = TONE.symbol_samples;
        let nfft = n * TONE.bin_div;
        let bin_hz = TONE.fs / nfft as f64;
        let cfo_bins = (TONE.max_cfo_hz / bin_hz).round() as usize;
        let centre = (TONE.tones - 1) * TONE.bin_div / 2;
        // tone k sits at bin k·bin_div − centre; the band runs from the lowest tone less the
        // offset search to the highest plus it
        let lowest = centre + cfo_bins; // −(tone 0's bin − cfo_bins)
        let tone_col = (0..TONE.tones)
            .map(|k| k * TONE.bin_div + lowest - centre - cfo_bins)
            .collect();
        let band = 2 * (centre + cfo_bins) + 1;
        let mut hypotheses = Vec::new();
        for kind in kinds() {
            for rv in 0..kind.n_rv() {
                hypotheses.push((kind, rv as u8));
            }
        }
        Self {
            hop: n / TONE.hop_div,
            nfft,
            bin_hz,
            cfo_bins,
            tone_col,
            band,
            fft: FftPlanner::new().plan_fft_forward(nfft),
            hypotheses,
        }
    }

    /// Samples between spectrogram rows.
    #[must_use]
    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Width of a carrier-offset bin, in hertz.
    #[must_use]
    pub fn bin_hz(&self) -> f64 {
        self.bin_hz
    }

    /// Offset bins searched either side of zero.
    #[must_use]
    pub fn cfo_bins(&self) -> usize {
        self.cfo_bins
    }

    /// Every (kind, redundancy version) hypothesis, in the model's order.
    #[must_use]
    pub fn hypotheses(&self) -> &[(&'static ToneKind, u8)] {
        &self.hypotheses
    }

    /// The acquisition threshold on the statistic.
    #[must_use]
    pub fn threshold(&self) -> f64 {
        TONE.threshold
    }

    /// One spectrogram row's band: the energy of every bin of the band in the symbol-long
    /// window at `start`, unit for unit noise power.
    fn spectrum_row(&self, samples: &[Complex], start: usize) -> Vec<f64> {
        let n = TONE.symbol_samples;
        let mut buffer = vec![Complex64::new(0.0, 0.0); self.nfft];
        for (slot, &(re, im)) in buffer.iter_mut().zip(&samples[start..start + n]) {
            *slot = Complex64::new(re, im);
        }
        self.fft.process(&mut buffer);
        let half = self.band / 2;
        (0..self.band)
            .map(|c| {
                let bin = (c + self.nfft - half) % self.nfft;
                buffer[bin].norm_sqr() / n as f64
            })
            .collect()
    }

    /// One row of ratios (`tones × offsets`, tone-major) from a spectrum row: every tone's
    /// energy at every offset over the mean of the other fifteen there, clipped.
    fn ratio_row(&self, spectrum: &[f64]) -> Vec<f64> {
        let tones = TONE.tones;
        let width = 2 * self.cfo_bins + 1;
        let mut out = vec![0.0f64; tones * width];
        for c in 0..width {
            let total: f64 = (0..tones).map(|k| spectrum[self.tone_col[k] + c]).sum();
            for k in 0..tones {
                let e = spectrum[self.tone_col[k] + c];
                let others = (total - e) / (tones - 1) as f64;
                out[k * width + c] = (e / others.max(1e-30)).min(TONE.clip);
            }
        }
        out
    }

    /// The ratio row of the symbol-long window at `start`.
    #[must_use]
    pub fn ratios_at(&self, samples: &[Complex], start: usize) -> Vec<f64> {
        self.ratio_row(&self.spectrum_row(samples, start))
    }

    /// Every hop's ratio row of a buffer.
    fn ratio_rows(&self, samples: &[Complex]) -> Vec<Vec<f64>> {
        let n = TONE.symbol_samples;
        if samples.len() < n {
            return Vec::new();
        }
        (0..=(samples.len() - n) / self.hop)
            .map(|h| self.ratios_at(samples, h * self.hop))
            .collect()
    }

    /// The statistic of hypothesis `(kind, rv)` at start row `h`: the best offset's mean
    /// ratio over the sync tones of the first block only, or of all three.
    fn statistic<'a, R>(
        &self,
        rows: &R,
        h: usize,
        kind: &ToneKind,
        rv: usize,
        first_only: bool,
    ) -> Option<(f64, usize)>
    where
        R: Fn(usize) -> Option<&'a [f64]>,
    {
        let q = TONE.hop_div;
        let width = 2 * self.cfo_bins + 1;
        let offsets = kind.block_offsets();
        let blocks = if first_only {
            &offsets[..1]
        } else {
            &offsets[..]
        };
        let mut acc = vec![0.0f64; width];
        for &o in blocks {
            for (j, &tone) in kind.sync(rv).iter().enumerate() {
                let row = rows(h + q * (o + j))?;
                for (slot, &v) in acc.iter_mut().zip(&row[tone * width..(tone + 1) * width]) {
                    *slot += v;
                }
            }
        }
        let count = (blocks.len() * SYNC_SYMBOLS) as f64;
        for slot in &mut acc {
            *slot /= count;
        }
        let c = argmax(&acc);
        Some((acc[c], c))
    }

    /// How many of the first block's tones at offset bin `c` are the strongest in their
    /// symbols — a silent symbol (the receiver muted) ties every tone and is no evidence.
    fn first_hits<'a, R>(&self, rows: &R, h: usize, kind: &ToneKind, rv: usize, c: usize) -> usize
    where
        R: Fn(usize) -> Option<&'a [f64]>,
    {
        let width = 2 * self.cfo_bins + 1;
        kind.sync(rv)
            .iter()
            .enumerate()
            .filter(|&(j, &tone)| {
                rows(h + TONE.hop_div * j).is_some_and(|row| {
                    let column: Vec<f64> = (0..TONE.tones).map(|k| row[k * width + c]).collect();
                    strongest(&column, tone)
                })
            })
            .count()
    }

    /// The best hypothesis at start row `h` among `hypotheses`: statistic, hypothesis index,
    /// offset bin (signed). A first block counts only with enough of its tones strongest.
    fn best<'a, R>(
        &self,
        rows: &R,
        h: usize,
        hypotheses: &[usize],
        first_only: bool,
    ) -> Option<(f64, usize, i64)>
    where
        R: Fn(usize) -> Option<&'a [f64]>,
    {
        let mut best: Option<(f64, usize, i64)> = None;
        for &i in hypotheses {
            let (kind, rv) = self.hypotheses[i];
            let Some((v, c)) = self.statistic(rows, h, kind, usize::from(rv), first_only) else {
                continue;
            };
            if best.is_some_and(|b| v <= b.0) {
                continue;
            }
            if first_only
                && self.first_hits(rows, h, kind, usize::from(rv), c) < TONE.min_first_hits
            {
                continue;
            }
            best = Some((v, i, c as i64 - self.cfo_bins as i64));
        }
        best
    }

    /// Frames in a buffer, earliest first: peaks of the statistic above the threshold,
    /// strongest first, each refined and kept if [`confirmed`](Self::confirmed); a kept frame
    /// rules out every other start inside its span.
    #[must_use]
    pub fn detect(&self, samples: &[Complex], max_frames: usize) -> Vec<ToneSync> {
        let rows = self.ratio_rows(samples);
        let lookup = |h: usize| rows.get(h).map(Vec::as_slice);
        let all: Vec<usize> = (0..self.hypotheses.len()).collect();
        let mut stat = vec![f64::NEG_INFINITY; rows.len()];
        let mut hyp = vec![0usize; rows.len()];
        let mut cbin = vec![0i64; rows.len()];
        for h in 0..rows.len() {
            if let Some((v, i, c)) = self.best(&lookup, h, &all, false) {
                stat[h] = v;
                hyp[h] = i;
                cbin[h] = c;
            }
        }
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by(|&a, &b| stat[b].total_cmp(&stat[a]).then(a.cmp(&b)));
        let mut found = Vec::new();
        let mut taken = vec![false; rows.len()];
        let q = TONE.hop_div;
        for p in order {
            if stat[p] < TONE.threshold || found.len() >= max_frames {
                break;
            }
            if taken[p] {
                continue;
            }
            let (kind, rv) = self.hypotheses[hyp[p]];
            let sync = self.refine(
                samples,
                kind,
                rv,
                p * self.hop,
                cbin[p] as f64 * self.bin_hz,
            );
            if !self.confirmed(samples, &sync) {
                // a coincidence of data symbols with a pattern: rule out its neighbourhood
                let lo = p.saturating_sub(q);
                let hi = (p + q + 1).min(taken.len());
                taken[lo..hi].fill(true);
                continue;
            }
            // nothing else starts inside it; a frame right after it may start a hop or two
            // before its nominal end, where the peak's rounding puts it
            let span = q * kind.symbols() - q / 2;
            let lo = (p + 1).saturating_sub(span);
            let hi = (p + span).min(taken.len());
            taken[lo..hi].fill(true);
            found.push(sync);
        }
        found.sort_by_key(|s| s.start);
        found
    }

    /// Mean energy of the 24 sync tones alone at `start`, for each offset in `cfos`;
    /// `-inf` where the frame would leave the buffer.
    #[must_use]
    pub fn sync_energies(
        &self,
        samples: &[Complex],
        kind: &ToneKind,
        rv: u8,
        start: i64,
        cfos: &[f64],
    ) -> Vec<f64> {
        let n = TONE.symbol_samples;
        if start < 0 || start as usize + kind.samples() > samples.len() {
            return vec![f64::NEG_INFINITY; cfos.len()];
        }
        let start = start as usize;
        let layout = kind.layout(usize::from(rv));
        let t: Vec<f64> = (0..n).map(|i| i as f64 / TONE.fs).collect();
        let shifts: Vec<Vec<(f64, f64)>> = cfos
            .iter()
            .map(|&f| {
                t.iter()
                    .map(|&ti| {
                        let phase = -2.0 * PI * ti * f;
                        (phase.cos(), phase.sin())
                    })
                    .collect()
            })
            .collect();
        let mut sums = vec![0.0f64; cfos.len()];
        let mut count = 0usize;
        for (s, role) in layout.iter().enumerate() {
            let Some(tone) = *role else { continue };
            count += 1;
            let omega = -2.0 * PI * tone_hz(tone);
            let segment = &samples[start + s * n..start + (s + 1) * n];
            let base: Vec<(f64, f64)> = segment
                .iter()
                .zip(&t)
                .map(|(&(x, y), &ti)| {
                    let phase = omega * ti;
                    let (c, d) = (phase.cos(), phase.sin());
                    (x * c - y * d, x * d + y * c)
                })
                .collect();
            for (slot, shift) in sums.iter_mut().zip(&shifts) {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (&(a, b), &(c, d)) in base.iter().zip(shift) {
                    re += a * c - b * d;
                    im += a * d + b * c;
                }
                *slot += re * re + im * im;
            }
        }
        sums.iter().map(|&s| s / count as f64 / n as f64).collect()
    }

    /// Mean normalised energy of the sync tones of a frame placed at `start`/`cfo`.
    #[must_use]
    pub fn sync_statistic(
        &self,
        samples: &[Complex],
        kind: &ToneKind,
        rv: u8,
        start: usize,
        cfo: f64,
    ) -> f64 {
        let layout = kind.layout(usize::from(rv));
        let Some(e) = tone_energies(samples, start, cfo, kind.symbols()) else {
            return f64::NEG_INFINITY;
        };
        let held: Vec<f64> = layout
            .iter()
            .enumerate()
            .filter_map(|(s, role)| role.map(|t| e[s * TONE.tones + t]))
            .collect();
        held.iter().sum::<f64>() / held.len() as f64 / noise_level(&e, &layout)
    }

    /// Per sync block, the symbols of a frame placed at `start`/`cfo` whose own tone is the
    /// strongest of the sixteen; a silent symbol (the receiver muted) is no evidence. All
    /// zero if the frame runs past the buffer.
    #[must_use]
    pub fn block_hits(
        &self,
        samples: &[Complex],
        kind: &ToneKind,
        rv: u8,
        start: usize,
        cfo: f64,
    ) -> [usize; 3] {
        let Some(e) = tone_energies(samples, start, cfo, kind.symbols()) else {
            return [0; 3];
        };
        let tones = TONE.tones;
        let sync = kind.sync(usize::from(rv));
        kind.block_offsets().map(|o| {
            sync.iter()
                .enumerate()
                .filter(|&(j, &t)| strongest(&e[(o + j) * tones..(o + j + 1) * tones], t))
                .count()
        })
    }

    /// Sync symbols of a frame placed at `start`/`cfo` whose own tone is the strongest
    /// ([`block_hits`](Self::block_hits), summed).
    #[must_use]
    pub fn sync_hits(
        &self,
        samples: &[Complex],
        kind: &ToneKind,
        rv: u8,
        start: usize,
        cfo: f64,
    ) -> usize {
        self.block_hits(samples, kind, rv, start, cfo).iter().sum()
    }

    /// Whether a refined candidate is a frame: at least `min_hits` of its sync tones are the
    /// strongest in their symbols — which data coincidences with a pattern never reach — and
    /// at least `min_block_hits` of them in a second block. A hypothesis a block-spacing off
    /// a strong frame has eight hits in one block and chance in the others: taken as frames
    /// end, it came first and blocked the real frame (found by two daemons over `[sim]`,
    /// the first block in the silence a station keeps while it transmits).
    #[must_use]
    pub fn confirmed(&self, samples: &[Complex], sync: &ToneSync) -> bool {
        let mut hits = self.block_hits(samples, sync.kind, sync.rv, sync.start, sync.cfo_hz);
        hits.sort_unstable();
        hits.iter().sum::<usize>() >= TONE.min_hits && hits[1] >= TONE.min_block_hits
    }

    /// Timing and offset on the sync tones' own energy: a grid of two hops and two bins
    /// either side at half a hop and half a bin — the clipped statistic is flat across a
    /// strong frame's neighbourhood — then to the sample and a fraction of a hertz.
    #[must_use]
    pub fn refine(
        &self,
        samples: &[Complex],
        kind: &'static ToneKind,
        rv: u8,
        start: usize,
        cfo: f64,
    ) -> ToneSync {
        let half = (self.hop / 2) as i64;
        let cfos: Vec<f64> = (0..9)
            .map(|k| cfo + self.bin_hz * (-2.0 + 0.5 * f64::from(k)))
            .collect();
        let starts: Vec<i64> = (-4..=4).map(|k| start as i64 + k * half).collect();
        let mut best = (f64::NEG_INFINITY, starts[0], cfos[0]);
        for &s in &starts {
            for (j, v) in self
                .sync_energies(samples, kind, rv, s, &cfos)
                .into_iter()
                .enumerate()
            {
                if v > best.0 {
                    best = (v, s, cfos[j]);
                }
            }
        }
        let (_, mut start, cfo) = best;
        for (lo, hi, step) in [(-half, half, 4i64), (-3, 3, 1)] {
            let centre = start;
            let mut top = (f64::NEG_INFINITY, centre);
            let mut s = centre + lo;
            while s <= centre + hi {
                let v = self.sync_energies(samples, kind, rv, s, &[cfo])[0];
                if v > top.0 {
                    top = (v, s);
                }
                s += step;
            }
            start = top.1;
        }
        let offsets: Vec<f64> = (0..21)
            .map(|k| cfo + self.bin_hz * (-0.5 + 0.05 * f64::from(k)))
            .collect();
        let energy = self.sync_energies(samples, kind, rv, start, &offsets);
        let k = argmax(&energy);
        let mut cfo = offsets[k];
        if k > 0 && k + 1 < offsets.len() {
            let (a, b, c) = (energy[k - 1], energy[k], energy[k + 1]);
            let denom = a - 2.0 * b + c;
            if denom < 0.0 {
                cfo += 0.5 * (a - c) / denom * (offsets[1] - offsets[0]);
            }
        }
        let start = start.max(0) as usize;
        ToneSync {
            start,
            cfo_hz: cfo,
            kind,
            rv,
            statistic: self.sync_statistic(samples, kind, rv, start, cfo),
        }
    }
}

// ── streaming ──────────────────────────────────────────────────────────

/// A tone frame whose first sync block is in and whose end is not: evidence that a frame of
/// `kind` is arriving, `start` to a hop (absolute samples).
#[derive(Debug, Clone, Copy)]
pub struct ToneArrival {
    /// Where it starts.
    pub start: usize,
    /// Its kind.
    pub kind: &'static ToneKind,
    /// The redundancy version its first block named.
    pub rv: u8,
    /// The first block's statistic.
    pub statistic: f64,
}

impl ToneArrival {
    /// Where it ends.
    #[must_use]
    pub fn end(&self) -> usize {
        self.start + self.kind.samples()
    }

    /// How far above the announcement threshold the first block was.
    #[must_use]
    pub fn detect_confidence(&self) -> f64 {
        self.statistic / TONE.announce_threshold
    }
}

/// A candidate: start row, statistic, hypothesis, offset bin.
type Candidate = (i64, f64, usize, i64);

/// The streaming counterpart of [`ToneDetector::detect`]: fed the receiver's rolling buffer,
/// it keeps the spectrogram's ratio rows by absolute hop — each computed once — evaluates
/// every hypothesis at every start once its rows are in (each frame length on its own
/// frontier, so a control frame is not held back for a data frame's worth of rows), takes a
/// frame a symbol after it ends, and announces one once its first sync block is in.
pub struct ToneStream {
    det: ToneDetector,
    /// Ratio rows from absolute hop `first_row` on.
    rows: std::collections::VecDeque<Vec<f64>>,
    first_row: i64,
    next_hop: i64,
    /// Per frame length (hops from a frame's first sync row to its last): its hypotheses,
    /// and the next start not yet evaluated.
    groups: Vec<(i64, Vec<usize>, i64)>,
    next_first: i64,
    first_hops: i64,
    candidates: Vec<Candidate>,
    firsts: Vec<Candidate>,
    taken: std::collections::VecDeque<(usize, usize)>,
    /// Frames announced and not yet final.
    pub arriving: Vec<ToneArrival>,
}

impl std::fmt::Debug for ToneStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToneStream")
            .field("next_hop", &self.next_hop)
            .field("candidates", &self.candidates.len())
            .field("arriving", &self.arriving.len())
            .finish_non_exhaustive()
    }
}

impl Default for ToneStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ToneStream {
    /// A stream with a fresh detector.
    #[must_use]
    pub fn new() -> Self {
        let det = ToneDetector::new();
        let q = TONE.hop_div as i64;
        let mut groups: Vec<(i64, Vec<usize>, i64)> = Vec::new();
        for (i, (kind, _)) in det.hypotheses().iter().enumerate() {
            let length = q * (kind.symbols() as i64 - 1);
            if let Some(group) = groups.iter_mut().find(|g| g.0 == length) {
                group.1.push(i);
            } else {
                groups.push((length, vec![i], 0));
            }
        }
        Self {
            det,
            rows: std::collections::VecDeque::new(),
            first_row: 0,
            next_hop: 0,
            groups,
            next_first: 0,
            first_hops: q * (SYNC_SYMBOLS as i64 - 1),
            candidates: Vec::new(),
            firsts: Vec::new(),
            taken: std::collections::VecDeque::new(),
            arriving: Vec::new(),
        }
    }

    /// The detector behind it.
    #[must_use]
    pub fn detector(&self) -> &ToneDetector {
        &self.det
    }

    fn row(&self, h: i64) -> Option<&[f64]> {
        let index = usize::try_from(h - self.first_row).ok()?;
        self.rows.get(index).map(Vec::as_slice)
    }

    /// Frames final in `buffer` (whose first sample is absolute index `abs0`), with absolute
    /// starts; [`arriving`](Self::arriving) holds the frames announced and not yet final.
    pub fn feed(&mut self, buffer: &[Complex], abs0: usize) -> Vec<ToneSync> {
        let n = TONE.symbol_samples as i64;
        let hop = self.det.hop() as i64;
        let abs0_i = abs0 as i64;
        let end = abs0_i + buffer.len() as i64;
        let ceil = (abs0_i + hop - 1) / hop;
        if ceil > self.next_hop {
            // the buffer was trimmed past rows never computed: start again from there
            self.next_hop = ceil;
        }
        if self.rows.is_empty() {
            self.first_row = self.next_hop;
        }
        while self.next_hop * hop + n <= end {
            let a = (self.next_hop * hop - abs0_i) as usize;
            let row = self.det.ratios_at(buffer, a);
            if self.first_row + self.rows.len() as i64 != self.next_hop {
                // a gap: rows before it can no longer be used together with these
                self.rows.clear();
                self.first_row = self.next_hop;
            }
            self.rows.push_back(row);
            self.next_hop += 1;
        }
        let top = self.next_hop;
        let oldest = if self.rows.is_empty() {
            top
        } else {
            self.first_row
        };
        // every hypothesis at every start whose rows are all in
        let groups = self.groups.clone();
        let mut new_candidates = Vec::new();
        for (index, (length, hyps, next)) in groups.iter().enumerate() {
            let last_full = top - 1 - length;
            let mut h = (*next).max(oldest);
            while h <= last_full {
                if let Some((v, i, c)) =
                    self.det
                        .best(&|r| self.row(r as i64), h as usize, hyps, false)
                    && v >= TONE.threshold
                {
                    new_candidates.push((h, v, i, c));
                }
                h += 1;
            }
            self.groups[index].2 = (*next).max(last_full + 1);
        }
        self.candidates.extend(new_candidates);
        // the first blocks, for the frames arriving
        let last_first = top - 1 - self.first_hops;
        let all: Vec<usize> = (0..self.det.hypotheses().len()).collect();
        let mut h = self.next_first.max(oldest);
        let mut new_firsts = Vec::new();
        while h <= last_first {
            if let Some((v, i, c)) = self
                .det
                .best(&|r| self.row(r as i64), h as usize, &all, true)
                && v >= TONE.announce_threshold
            {
                new_firsts.push((h, v, i, c));
            }
            h += 1;
        }
        self.firsts.extend(new_firsts);
        self.next_first = self.next_first.max(last_first + 1);
        let found = self.settle(buffer, abs0);
        self.announce();
        // drop the rows no start still to be evaluated needs
        let need = self
            .groups
            .iter()
            .map(|g| g.2)
            .chain(std::iter::once(self.next_first))
            .chain(self.candidates.iter().map(|c| c.0))
            .chain(self.firsts.iter().map(|c| c.0))
            .min()
            .unwrap_or(top);
        while self.first_row < need && !self.rows.is_empty() {
            self.rows.pop_front();
            self.first_row += 1;
        }
        found
    }

    fn frontier_of(&self, kind: &ToneKind) -> i64 {
        let length = TONE.hop_div as i64 * (kind.symbols() as i64 - 1);
        self.groups
            .iter()
            .find(|g| g.0 == length)
            .map_or(i64::MAX, |g| g.2)
    }

    /// The candidates now final, each refined and confirmed, with absolute starts.
    fn settle(&mut self, buffer: &[Complex], abs0: usize) -> Vec<ToneSync> {
        let lookahead = TONE.lookahead as i64;
        let hop = self.det.hop();
        let mut snapshot = self.candidates.clone();
        snapshot.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        let mut keep = Vec::new();
        let mut found = Vec::new();
        for &(h, v, i, c) in &snapshot {
            let (kind, rv) = self.det.hypotheses()[i];
            if h + lookahead >= self.frontier_of(kind) {
                keep.push((h, v, i, c));
                continue;
            }
            let start = h as usize * hop;
            let near = snapshot
                .iter()
                .filter(|x| (x.0 - h).abs() <= lookahead)
                .map(|x| x.1)
                .fold(f64::NEG_INFINITY, f64::max);
            if v < near || self.overlaps((start, start + kind.samples())) || start < abs0 {
                continue;
            }
            let sync =
                self.det
                    .refine(buffer, kind, rv, start - abs0, c as f64 * self.det.bin_hz());
            if !self.det.confirmed(buffer, &sync) {
                continue;
            }
            let sync = ToneSync {
                start: sync.start + abs0,
                ..sync
            };
            self.taken
                .push_back((sync.start, sync.start + kind.samples()));
            if self.taken.len() > 16 {
                self.taken.pop_front();
            }
            found.push(sync);
        }
        self.candidates = keep;
        found
    }

    fn announce(&mut self) {
        let lookahead = TONE.announce_lookahead as i64;
        let hop = self.det.hop();
        let mut snapshot = self.firsts.clone();
        snapshot.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.total_cmp(&b.1))
                .then(a.2.cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        let mut keep = Vec::new();
        for &(h, v, i, c) in &snapshot {
            if h + lookahead >= self.next_first {
                keep.push((h, v, i, c));
                continue;
            }
            let (kind, rv) = self.det.hypotheses()[i];
            let start = h as usize * hop;
            let near = snapshot
                .iter()
                .filter(|x| (x.0 - h).abs() <= lookahead)
                .map(|x| x.1)
                .fold(f64::NEG_INFINITY, f64::max);
            if v < near || self.overlaps((start, start + kind.samples())) {
                continue;
            }
            // inside a frame already arriving: its own middle or end sync block, which
            // repeats the first — never a frame of its own, in a half-duplex burst
            if self
                .arriving
                .iter()
                .any(|a| (a.start as i64 - hop as i64) < start as i64 && start + hop < a.end())
            {
                continue;
            }
            self.arriving.push(ToneArrival {
                start,
                kind,
                rv,
                statistic: v,
            });
        }
        self.firsts = keep;
        // an announcement lasts until its frame is taken or its end has passed
        let now = self.next_hop.max(0) as usize * hop;
        let arriving = std::mem::take(&mut self.arriving);
        self.arriving = arriving
            .into_iter()
            .filter(|a| a.end() + hop > now && !self.overlaps((a.start + hop, a.end() - hop)))
            .collect();
    }

    /// Whether `span` overlaps a frame taken by more than the hop or two a coarse start is
    /// off by — back-to-back frames of a burst touch.
    fn overlaps(&self, span: (usize, usize)) -> bool {
        let tol = TONE.hop_div / 2 * self.det.hop();
        self.taken
            .iter()
            .any(|&(a, b)| span.0 + tol < b && a + tol < span.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec(kind: &'static ToneKind) -> ToneCodec {
        ToneCodec::new(kind).expect("codec")
    }

    fn payload(kind: &ToneKind, seed: u8) -> Vec<u8> {
        (0..kind.payload_bytes)
            .map(|i| (i as u8).wrapping_mul(37).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn the_kinds_are_the_models() {
        assert_eq!(tones(), 16);
        assert_eq!(symbol_samples(), 320);
        assert!((spacing_hz() - 25.0).abs() < 1e-12);
        assert_eq!(control_kind().payload_bytes, 7);
        assert_eq!(control_kind().symbols(), 80);
        assert_eq!(
            data_kinds()
                .iter()
                .map(|k| k.payload_bytes)
                .collect::<Vec<_>>(),
            vec![24, 36]
        );
        assert!(data_kinds().iter().all(|k| k.symbols() == 134));
        assert_eq!(control_kind().block_offsets(), [0, 33, 72]);
        assert_eq!(data_kinds()[0].block_offsets(), [0, 57, 126]);
        assert!((announce_delay_s(0.1) - 0.54).abs() < 1e-12);
    }

    #[test]
    fn every_kind_round_trips_clean() {
        for kind in kinds() {
            let codec = codec(kind);
            let data = payload(kind, 3);
            let x = burst(&codec, &data, 0).expect("burst");
            assert_eq!(x.len(), kind.samples());
            let frame = demodulate(&x, kind, 0, 0, 0.0).expect("demodulate");
            let (decoded, _) = codec.decode(&frame.llr, 0, None).expect("decode");
            assert_eq!(decoded.as_deref(), Some(&data[..]), "{}", kind.name);
        }
    }

    #[test]
    fn the_envelope_is_constant_at_the_gain() {
        let kind = &data_kinds()[1];
        let x = burst(&codec(kind), &payload(kind, 9), 0).expect("burst");
        let gain = 10f64.powf(gain_db() / 20.0);
        let edge = TONE.edge_samples;
        for &(re, im) in &x[edge..x.len() - edge] {
            assert!(((re * re + im * im).sqrt() - gain).abs() < 1e-9);
        }
    }

    #[test]
    fn the_detector_names_kind_rv_start_and_offset() {
        let det = ToneDetector::new();
        for kind in kinds() {
            for rv in 0..kind.n_rv() {
                let x = burst(&codec(kind), &payload(kind, 5), rv as u8).expect("burst");
                let lead = 1234 + 97 * rv;
                let cfo = 31.5 - 20.0 * rv as f64;
                let mut y: Vec<Complex> = vec![(0.0, 0.0); lead];
                y.extend_from_slice(&x);
                y.extend(std::iter::repeat_n((0.0, 0.0), 2000));
                for (i, s) in y.iter_mut().enumerate() {
                    let phase = 2.0 * PI * cfo * i as f64 / TONE.fs;
                    let (c, d) = (phase.cos(), phase.sin());
                    *s = (s.0 * c - s.1 * d, s.0 * d + s.1 * c);
                    // a little deterministic hiss, so ratios are finite
                    s.0 += 1e-3 * ((i * 7919) % 101) as f64 / 101.0;
                    s.1 += 1e-3 * ((i * 104_729) % 97) as f64 / 97.0;
                }
                let found = det.detect(&y, 4);
                assert_eq!(found.len(), 1, "{} rv {rv}", kind.name);
                let s = found[0];
                assert_eq!((s.kind.name, s.rv), (kind.name, rv as u8));
                assert!(
                    s.start.abs_diff(lead) <= 4,
                    "{} start {}",
                    kind.name,
                    s.start
                );
                assert!((s.cfo_hz - cfo).abs() < 0.5, "cfo {}", s.cfo_hz);
            }
        }
    }

    #[test]
    fn silence_under_a_frame_is_left_out_of_its_noise() {
        // a frame half under the receiver's own transmission read its noise as zero from the
        // silent symbols and its SNR as 290 dB, which a rate controller would believe
        let kind = &data_kinds()[0];
        let x = burst(&codec(kind), &payload(kind, 4), 0).expect("burst");
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f64 / u64::MAX as f64 - 0.5) * 0.4
        };
        let y: Vec<Complex> = x
            .iter()
            .map(|&(re, im)| (re + noise(), im + noise()))
            .collect();
        let whole = demodulate(&y, kind, 0, 0, 0.0).expect("whole");
        let mut muted = y.clone();
        let half = muted.len() / 2;
        muted[half..].fill((0.0, 0.0));
        let part = demodulate(&muted, kind, 0, 0, 0.0).expect("half");
        assert!(
            (part.snr_db - whole.snr_db).abs() < 4.0,
            "{} against {}",
            part.snr_db,
            whole.snr_db
        );
    }

    #[test]
    fn the_stream_takes_frames_a_symbol_after_they_end_and_announces_them_early() {
        let kinds = [&data_kinds()[0], &data_kinds()[0], control_kind()];
        let mut y: Vec<Complex> = vec![(0.0, 0.0); 3000];
        let mut starts = Vec::new();
        for (i, kind) in kinds.iter().enumerate() {
            starts.push(y.len());
            y.extend(burst(&codec(kind), &payload(kind, i as u8), 0).expect("burst"));
        }
        y.extend(std::iter::repeat_n((0.0, 0.0), 4000));
        for (i, s) in y.iter_mut().enumerate() {
            s.0 += 1e-3 * ((i * 7919) % 101) as f64 / 101.0;
            s.1 += 1e-3 * ((i * 104_729) % 97) as f64 / 97.0;
        }
        let mut stream = ToneStream::new();
        let mut found = Vec::new();
        let mut announced = Vec::new();
        let mut buffer: Vec<Complex> = Vec::new();
        let mut abs0 = 0usize;
        for chunk in y.chunks(160) {
            buffer.extend_from_slice(chunk);
            let seen = abs0 + buffer.len();
            for sync in stream.feed(&buffer, abs0) {
                found.push((
                    sync.start,
                    sync.kind.name,
                    seen - (sync.start + sync.kind.samples()),
                ));
            }
            for a in &stream.arriving {
                if !announced.iter().any(|&(s, _)| s == a.start) {
                    announced.push((a.start, seen - a.start));
                }
            }
            if buffer.len() > 56_000 {
                let drop = buffer.len() - 56_000;
                buffer.drain(..drop);
                abs0 += drop;
            }
        }
        assert_eq!(found.len(), 3, "{found:?}");
        for ((start, name, late), (&truth, kind)) in found.iter().zip(starts.iter().zip(kinds)) {
            assert_eq!(*name, kind.name);
            assert!(start.abs_diff(truth) <= 4, "{start} against {truth}");
            assert!(*late <= 480, "taken {late} samples after its end");
        }
        assert_eq!(announced.len(), 3, "{announced:?}");
        for ((start, after), &truth) in announced.iter().zip(&starts) {
            assert!(start.abs_diff(truth) <= 80);
            assert!(*after <= 4000, "announced {after} samples in");
        }
    }
}
