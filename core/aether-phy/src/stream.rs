//! Streaming receiver: feed baseband blocks of any size, get decoded frames out.
//!
//! Wraps the offline detectors and receivers in a rolling buffer so live audio — a sound card,
//! a recording being replayed — can be processed block by block:
//!
//! * blocks are appended to a buffer that keeps at most `max_buffer_s` seconds;
//! * OFDM detection runs over the part of the buffer that has not been searched yet, extended
//!   backwards by the detector's [`stream_lookback`](crate::sync::FrameDetector::stream_lookback),
//!   so a preamble straddling a block boundary is still found once a symbol of what follows
//!   it has arrived;
//! * a detected OFDM frame is decoded only when its last sample — plus the transform window's
//!   margin — is in the buffer; until then it stays pending;
//! * the tone floor (ADR-0013) runs alongside on the same band-limited stream: a
//!   [`ToneStream`] keeps its spectrogram rows by absolute hop, hands over each tone frame a
//!   symbol after it ends, and announces one as arriving once its first sync block is in;
//! * absolute sample indices are kept, so a reported position stays meaningful after the
//!   buffer is trimmed. They refer to the band-limited stream, which lags the raw input by
//!   the filter's group delay, [`StreamingReceiver::delay_samples`].
//!
//! The decoded output is the same as [`Modem::decode_buffer`] on the concatenated input, so
//! real-time behaviour never diverges from the offline model — tests pin that.
//!
//! # Frames, reported early
//!
//! [`take_preambles`](StreamingReceiver::take_preambles) reports a frame the moment
//! acquisition finds it, before its payload has arrived: an OFDM frame by its preamble, a
//! tone frame by its first sync block. That is what the link layer's start-of-frame signal
//! is: without it a receiving station has to treat a whole data frame of silence as the end
//! of a burst, which measurement put at about 13 % of the throughput.

use std::collections::VecDeque;

use crate::{
    Complex,
    blanker::StreamingBlanker,
    fir::Fir,
    modem::{DecodedFrame, Modem, Received},
    preamble::FrameType,
    rx::FrameSync,
    sync::{Acquisition, BankOutput, BankRow, BankState},
    tone::{ToneDetector, ToneStream, ToneSync},
    waveform::{WIDE_2300, WaveformParams},
};

/// A frame acquisition has found whose payload has not fully arrived — the start-of-frame
/// signal: an OFDM frame's preamble, or a tone frame's first sync block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingFrame {
    /// Absolute index of its first sample.
    pub start: usize,
    /// Absolute index one past its last sample.
    pub end: usize,
    /// How far above the threshold that found it acquisition saw it; 1.0 is exactly at it.
    /// An OFDM preamble's threshold sits at the noise maximum, so this is what separates a
    /// real one from a phantom; a tone frame is announced only above a threshold set over
    /// the noise maximum of its first block.
    pub detect_confidence: f64,
    /// Whether it is the tone floor's.
    pub tone: bool,
    /// Whether it is a control frame.
    pub control: bool,
}

/// An OFDM frame acquisition has located, in absolute sample indices.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pending {
    sync: FrameSync,
    /// Absolute index one past the frame's last sample.
    end: usize,
}

/// Feed it baseband, take frames out.
pub struct StreamingReceiver {
    params: WaveformParams,
    modem: Modem,
    blanker: Option<StreamingBlanker>,
    band: Fir<Complex>,
    /// Group delay of the band-limiting filter, in baseband samples. Every absolute index
    /// this receiver reports is in the filtered stream, which lags the input by this much.
    pub delay_samples: usize,
    buffer: Vec<Complex>,
    buffer_start: usize,
    searched: usize,
    pending: Vec<Pending>,
    done: VecDeque<(usize, usize)>,
    announced: VecDeque<usize>,
    fresh: Vec<PendingFrame>,
    max_buffer: usize,
    lookback: usize,
    /// The bank's row for every position from `buffer_start`, each computed once: a row
    /// depends only on the samples of its own reference window and never changes once they
    /// have arrived. The search used to re-run the whole bank over `block + 2·lookback` on
    /// every call, so a 20 ms pass repeated about ninety percent of its correlation work —
    /// the cost ADR-0010 left open, and what put a modest machine 10–20× behind real time.
    rows: VecDeque<BankRow>,
    /// The bank's scratch space.
    bank: BankState,
    /// The tone floor's detector, on the same stream.
    tone: ToneStream,
    /// Samples always kept for the tone floor: its longest frame and half a second — a tone
    /// frame is taken a symbol after it ends and refined against its own samples.
    tone_keep: usize,
    /// Frames decoded since the receiver was built, whether or not their check passed.
    pub frames_decoded: usize,
}

impl std::fmt::Debug for StreamingReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamingReceiver")
            .field("samples_seen", &self.samples_seen())
            .field("pending", &self.pending.len())
            .field("frames_decoded", &self.frames_decoded)
            .finish_non_exhaustive()
    }
}

impl Default for StreamingReceiver {
    fn default() -> Self {
        Self::new(WIDE_2300, 6.0, true)
    }
}

/// How many frames one search pass may report.
const MAX_FRAMES_PER_SEARCH: usize = 8;
/// How many finished frames to remember, so a late duplicate detection is recognised.
const DONE_MEMORY: usize = 20;
/// Samples kept beyond the tone floor's longest frame (half a second at 8 kHz).
const TONE_KEEP_EXTRA: usize = 4000;

impl StreamingReceiver {
    /// Build a receiver with a buffer of `max_buffer_s` seconds — at least the tone floor's
    /// longest frame and half a second, which the tone floor needs whatever is asked.
    #[must_use]
    pub fn new(params: WaveformParams, max_buffer_s: f64, blank_impulses: bool) -> Self {
        let modem = Modem::new(params, blank_impulses);
        let band = Fir::new(modem.band_taps().to_vec());
        let blanker = modem.blanker().cloned().map(StreamingBlanker::new);
        // the blanker holds samples back until their window can be centred, so the stream
        // this receiver indexes is later than its input by that much as well as the filter's
        let delay_samples = band.delay()
            + blanker
                .as_ref()
                .map_or(0, StreamingBlanker::latency_samples);
        let bank = modem.detector().bank_state();
        let lookback = modem.detector().stream_lookback();
        let tone_keep = modem
            .air()
            .tone_data()
            .iter()
            .map(|k| k.samples())
            .max()
            .unwrap_or(0)
            + TONE_KEEP_EXTRA;
        let tone = ToneStream::with_detector(ToneDetector::for_kinds(&modem.air().tone_kinds()));
        Self {
            blanker,
            band,
            delay_samples,
            modem,
            rows: VecDeque::new(),
            bank,
            buffer: Vec::new(),
            buffer_start: 0,
            searched: 0,
            pending: Vec::new(),
            done: VecDeque::new(),
            announced: VecDeque::new(),
            fresh: Vec::new(),
            max_buffer: ((max_buffer_s * params.fs_baseband) as usize).max(tone_keep),
            // a symbol after a preamble, so the sidelobe guard can see the real peak
            lookback,
            tone,
            tone_keep,
            frames_decoded: 0,
            params,
        }
    }

    /// Total samples that have entered the filtered stream.
    ///
    /// This is behind the input by [`blanker_latency`](Self::blanker_latency), because the
    /// blanker cannot judge a sample until some of its future has arrived.
    #[must_use]
    pub fn samples_seen(&self) -> usize {
        self.buffer_start + self.buffer.len()
    }

    /// How far the indexed stream runs behind the input, in samples.
    #[must_use]
    pub fn blanker_latency(&self) -> usize {
        self.blanker
            .as_ref()
            .map_or(0, StreamingBlanker::latency_samples)
    }

    /// The modem behind it, for decoding a frame again with a HARQ buffer.
    #[must_use]
    pub fn modem(&self) -> &Modem {
        &self.modem
    }

    /// The modem, mutably.
    pub fn modem_mut(&mut self) -> &mut Modem {
        &mut self.modem
    }

    /// Frames acquisition has found since the last call, whose payloads may not have arrived.
    ///
    /// Each frame is reported once. This is the start-of-frame signal the link layer wants:
    /// it tells a receiving station a burst is still running — roughly two preamble symbols
    /// into an OFDM frame, [`announce_delay_s`](crate::tone::announce_delay_s) into a tone
    /// frame — rather than a whole frame later.
    pub fn take_preambles(&mut self) -> Vec<PendingFrame> {
        std::mem::take(&mut self.fresh)
    }

    /// Feed a block of baseband and take out whatever finished, in the order the frames
    /// start.
    pub fn feed(&mut self, block: &[Complex]) -> Vec<DecodedFrame> {
        let blanked = self
            .blanker
            .as_mut()
            .map_or_else(|| block.to_vec(), |b| b.process(block));
        let filtered = self.band.process(&blanked);
        self.buffer.extend_from_slice(&filtered);

        self.search();
        let mut out = self.harvest();
        out.extend(self.tone_floor());
        self.trim();
        out.sort_by_key(|d| d.frame.start());
        out
    }

    /// The bank's row for every position whose reference window has arrived and has not
    /// been computed yet. Each is computed exactly once; the search then reads rows instead
    /// of re-running the bank over its whole lookback on every block.
    fn extend_rows(&mut self, reference_len: usize) {
        let seen = self.samples_seen();
        if seen < reference_len {
            return;
        }
        // the last position whose reference window is complete
        let last = seen - reference_len;
        if self.buffer_start + self.rows.len() > last {
            return;
        }
        // the normalisation floor, as the offline bank takes it from its region: this
        // buffer. It only bites on a window sixty decibels under the mean, so rows computed
        // against slightly different buffers agree wherever anything can be detected.
        let mean_power = self
            .buffer
            .iter()
            .map(|&(re, im)| re * re + im * im)
            .sum::<f64>()
            / self.buffer.len().max(1) as f64;
        let floor = 1e-3 * mean_power.sqrt() * (reference_len as f64).sqrt();
        let detector = self.modem.detector();
        while self.buffer_start + self.rows.len() <= last {
            let relative = self.rows.len();
            let window: f64 = self.buffer[relative..relative + reference_len]
                .iter()
                .map(|&(re, im)| re * re + im * im)
                .sum();
            let energy = window.max(1e-30).sqrt().max(floor);
            let row = detector.bank_row(&mut self.bank, &self.buffer, relative, energy);
            self.rows.push_back(row);
        }
    }

    /// Search the part of the buffer nothing has looked at yet for OFDM preambles.
    fn search(&mut self) {
        let symbol = self.params.symbol_samples();
        let reference_len = self.modem.detector().reference_len();
        self.extend_rows(reference_len);
        let search_start = self
            .buffer_start
            .max(self.searched.saturating_sub(self.lookback));
        let offset = search_start - self.buffer_start;
        if self.buffer.len() < offset + 4 * symbol {
            return;
        }
        let region = &self.buffer[offset..];
        // the bank over exactly this region — the rows already computed for its positions
        let positions = region.len().saturating_sub(reference_len) + 1;
        let output = BankOutput::from_rows(self.rows.iter().skip(offset).take(positions));
        let found = self
            .modem
            .detector()
            .detect_with(region, &output, MAX_FRAMES_PER_SEARCH);
        let air = self.modem.air();
        for acquisition in &found {
            let frame = self.absolute(acquisition, search_start);
            if self.known(&frame) {
                continue; // a duplicate, or inside a frame already known about
            }
            self.pending.push(frame);
            self.announce(PendingFrame {
                start: frame.sync.start,
                end: frame.end,
                detect_confidence: frame.sync.detect_confidence(&air),
                tone: false,
                control: frame.sync.frame_type == FrameType::Control,
            });
        }
        // the detector needs a symbol after a candidate, so leave that much unsearched
        self.searched = self
            .searched
            .max(self.samples_seen().saturating_sub(self.lookback));
    }

    /// Decode every pending OFDM frame whose samples have all arrived.
    fn harvest(&mut self) -> Vec<DecodedFrame> {
        // the transform window reaches a little past a frame's nominal last sample
        let margin = self.modem.receiver().fft_offset() + self.params.symbol_samples();
        let seen = self.samples_seen();
        self.pending.sort_by_key(|p| p.sync.start);

        let mut out = Vec::new();
        let mut still = Vec::new();
        for frame in std::mem::take(&mut self.pending) {
            if frame.end + margin > seen {
                still.push(frame);
                continue;
            }
            self.done.push_back((frame.sync.start, frame.end));
            if self.done.len() > DONE_MEMORY {
                self.done.pop_front();
            }
            let local = FrameSync {
                start: frame.sync.start - self.buffer_start,
                ..frame.sync
            };
            let decoded = self.modem.decode_sync(&self.buffer, &local, None);
            // A frame that will not demodulate — it ran off the end after all, or its
            // length does not match the layout — is simply not a frame. Acquisition
            // occasionally locks onto noise, and there is nothing to report about it.
            if let Ok(mut decoded) = decoded {
                if let Received::Ofdm(received) = &mut decoded.frame {
                    received.sync = frame.sync; // report absolute positions
                }
                out.push(decoded);
                self.frames_decoded += 1;
            }
        }
        self.pending = still;
        out
    }

    /// The tone floor on the same stream: the tone frames now final, decoded, and the ones
    /// arriving announced.
    fn tone_floor(&mut self) -> Vec<DecodedFrame> {
        let found = self.tone.feed(&self.buffer, self.buffer_start);
        let mut out = Vec::new();
        for sync in found {
            let local = ToneSync {
                start: sync.start - self.buffer_start,
                ..sync
            };
            if let Ok(mut decoded) = self.modem.decode_tone(&self.buffer, &local) {
                if let Received::Tone(_, at) = &mut decoded.frame {
                    *at = sync; // report absolute positions
                }
                out.push(decoded);
                self.frames_decoded += 1;
            }
        }
        for arrival in self.tone.arriving.clone() {
            self.announce(PendingFrame {
                start: arrival.start,
                end: arrival.end(),
                detect_confidence: arrival.detect_confidence(),
                tone: true,
                control: arrival.kind.control,
            });
        }
        out
    }

    /// An acquisition found `base` samples into the stream, with its span, in absolute indices.
    fn absolute(&self, acquisition: &Acquisition, base: usize) -> Pending {
        let span = self.modem.receiver().frame_span(&FrameSync {
            start: 0,
            ..acquisition.sync
        });
        let start = acquisition.sync.start + base;
        Pending {
            sync: FrameSync {
                start,
                ..acquisition.sync
            },
            end: start + span.1,
        }
    }

    /// Whether a frame duplicates, or starts inside, one already pending or handed out.
    fn known(&self, frame: &Pending) -> bool {
        let symbol = self.params.symbol_samples();
        let start = frame.sync.start;
        self.pending
            .iter()
            .map(|p| (p.sync.start, p.end))
            .chain(self.done.iter().copied())
            .any(|(a, b)| a.saturating_sub(symbol) < start && start < b)
    }

    /// Report a frame to [`take_preambles`](Self::take_preambles), once per start.
    fn announce(&mut self, frame: PendingFrame) {
        if self.announced.contains(&frame.start) {
            return;
        }
        self.announced.push_back(frame.start);
        if self.announced.len() > DONE_MEMORY {
            self.announced.pop_front();
        }
        self.fresh.push(frame);
    }

    /// Drop what nothing still needs, without ever discarding a pending frame's start or the
    /// tone floor's longest frame.
    fn trim(&mut self) {
        let seen = self.samples_seen();
        let keep = self
            .pending
            .iter()
            .map(|p| p.sync.start)
            .min()
            .unwrap_or(usize::MAX)
            .min(self.searched.saturating_sub(self.lookback))
            .min(seen.saturating_sub(self.tone_keep))
            .max(seen.saturating_sub(self.max_buffer))
            .max(self.buffer_start);
        let drop = keep - self.buffer_start;
        if drop > 0 {
            self.buffer.drain(..drop);
            // the rows run from `buffer_start` too, so they go with it
            self.rows.drain(..drop.min(self.rows.len()));
            self.buffer_start += drop;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::MODES;

    fn burst(modem: &mut Modem, payloads: &[(usize, Vec<u8>)]) -> Vec<Complex> {
        let mut out = vec![(0.0, 0.0); 600];
        for (mode_index, payload) in payloads {
            out.extend(
                modem
                    .data_burst(payload, MODES[*mode_index], 0)
                    .expect("encode"),
            );
        }
        out.extend(std::iter::repeat_n((0.0, 0.0), 4000));
        out
    }

    fn payload_for(mode_index: usize, seed: u8) -> Vec<u8> {
        let n = Modem::default().payload_bytes(Some(MODES[mode_index]));
        (0..n)
            .map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn streaming_gives_the_same_frames_as_one_offline_call() {
        // the property the whole design rests on: real-time behaviour must not diverge from
        // the offline model, or every measurement taken offline stops meaning anything
        let mut modem = Modem::default();
        let payloads = vec![(2, payload_for(2, 1)), (4, payload_for(4, 9))];
        let signal = burst(&mut modem, &payloads);

        let offline = Modem::default().decode_buffer(&signal, 8);
        assert_eq!(
            offline.len(),
            2,
            "the offline reference itself must find both"
        );

        for chunk in [512usize, 1024, 4096] {
            let mut rx = StreamingReceiver::default();
            let mut got = Vec::new();
            for block in signal.chunks(chunk) {
                got.extend(rx.feed(block));
            }
            assert_eq!(got.len(), offline.len(), "block size {chunk}: frame count");
            for (streamed, expected) in got.iter().zip(&offline) {
                assert_eq!(
                    streamed.payload, expected.payload,
                    "block size {chunk}: payload"
                );
                assert_eq!(
                    streamed.frame.ofdm().map(|f| f.mode),
                    expected.frame.ofdm().map(|f| f.mode)
                );
            }
        }
    }

    #[test]
    fn every_payload_comes_back_intact() {
        let mut modem = Modem::default();
        let payloads = vec![(0, payload_for(0, 3)), (5, payload_for(5, 44))];
        let signal = burst(&mut modem, &payloads);

        let mut rx = StreamingReceiver::default();
        let mut got = Vec::new();
        for block in signal.chunks(777) {
            got.extend(rx.feed(block));
        }
        assert_eq!(got.len(), 2);
        for (decoded, (mode, payload)) in got.iter().zip(&payloads) {
            assert_eq!(decoded.payload.as_ref(), Some(payload), "mode {mode}");
            assert_eq!(decoded.frame.ofdm().map(|f| f.mode), Some(*mode));
        }
    }

    #[test]
    fn a_preamble_is_reported_long_before_its_payload() {
        // the start-of-frame signal: worth about 13 % of the link's throughput, because
        // without it a receiver waits a whole frame of silence to know a burst has ended
        let mut modem = Modem::default();
        let payloads = vec![(4, payload_for(4, 5))];
        let signal = burst(&mut modem, &payloads);

        let mut rx = StreamingReceiver::default();
        let mut announced_at = None;
        let mut decoded_at = None;
        let chunk = 256;
        for (index, block) in signal.chunks(chunk).enumerate() {
            let frames = rx.feed(block);
            if announced_at.is_none() && !rx.take_preambles().is_empty() {
                announced_at = Some(index * chunk);
            }
            if decoded_at.is_none() && !frames.is_empty() {
                decoded_at = Some(index * chunk);
            }
        }
        let announced = announced_at.expect("the preamble was never reported");
        let decoded = decoded_at.expect("the frame never decoded");
        assert!(
            announced < decoded,
            "announced at {announced}, decoded at {decoded}"
        );
        let frame_samples = crate::modes::LONG.samples();
        assert!(
            decoded - announced > frame_samples / 2,
            "only {} samples of warning out of a {frame_samples}-sample frame",
            decoded - announced
        );
    }

    #[test]
    fn a_preamble_is_reported_once() {
        let mut modem = Modem::default();
        let signal = burst(&mut modem, &[(4, payload_for(4, 2))]);
        let mut rx = StreamingReceiver::default();
        let mut announcements = 0;
        for block in signal.chunks(300) {
            rx.feed(block);
            announcements += rx.take_preambles().len();
        }
        assert_eq!(announcements, 1, "the same frame was announced twice");
    }

    #[test]
    fn silence_produces_nothing() {
        let mut rx = StreamingReceiver::default();
        let silence = vec![(0.0, 0.0); 40_000];
        for block in silence.chunks(1000) {
            assert!(rx.feed(block).is_empty());
        }
        assert!(rx.take_preambles().is_empty());
        assert_eq!(rx.frames_decoded, 0);
    }

    #[test]
    fn the_buffer_stays_bounded_over_a_long_stream() {
        // an always-on receiver runs for days; the buffer must not be a slow leak
        let mut rx = StreamingReceiver::new(WIDE_2300, 2.0, true);
        let silence = vec![(0.0, 0.0); 8000];
        for _ in 0..60 {
            rx.feed(&silence);
        }
        // what was asked for, or the tone floor's longest frame and half a second where
        // that is more (ADR-0013): a tone frame is refined against its own samples
        let limit = ((2.0 * WIDE_2300.fs_baseband) as usize).max(rx.tone_keep) + 8000;
        assert_eq!(rx.tone_keep, 134 * 320 + 4000);
        assert!(
            rx.buffer.len() <= limit,
            "buffer grew to {} samples, limit {limit}",
            rx.buffer.len()
        );
        // the blanker holds a window back so it can be centred, so the stream this receiver
        // indexes runs exactly that far behind its input
        assert_eq!(rx.samples_seen(), 60 * 8000 - rx.blanker_latency());
    }

    /// A tone-floor frame — a data rung or the control frame — with a little noise either
    /// side, and its payload.
    fn tone_burst(params: WaveformParams, control: bool) -> (Vec<Complex>, Vec<u8>) {
        let mut modem = Modem::new(params, true);
        let (burst, payload) = if control {
            let payload: Vec<u8> = (0..7u8)
                .map(|i| i.wrapping_mul(11).wrapping_add(3))
                .collect();
            (
                modem.control_burst_of(&payload, 0, true).expect("encode"),
                payload,
            )
        } else {
            let payload: Vec<u8> = (0..modem.rung_payload_bytes(1))
                .map(|i| (i as u8).wrapping_mul(5).wrapping_add(1))
                .collect();
            (modem.rung_burst(&payload, 1, 0).expect("encode"), payload)
        };
        // a little noise, not digital silence: a receiver never delivers exact zeros
        let mut state = 0x2545_f491_u64;
        let mut noise = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f64 / u64::MAX as f64 - 0.5) * 0.02
        };
        let mut signal: Vec<Complex> = (0..600).map(|_| (noise(), noise())).collect();
        signal.extend(burst);
        signal.extend((0..4000).map(|_| (noise(), noise())));
        (signal, payload)
    }

    #[test]
    fn a_tone_frame_streams_the_same_as_one_offline_call() {
        // ADR-0013: the tone floor is taken a symbol after its frame ends, from the same
        // band-limited stream the offline detector reads — the same payload at the same
        // start, at every block size, the daemon's own 20 ms included — and announced once
        // its first sync block is in, long before
        use crate::waveform::NARROW_500;
        for params in [WIDE_2300, NARROW_500] {
            for control in [false, true] {
                let (signal, payload) = tone_burst(params, control);
                let offline = Modem::new(params, true).decode_buffer(&signal, 4);
                assert_eq!(offline.len(), 1, "offline must find the tone frame alone");
                assert_eq!(offline[0].payload.as_deref(), Some(payload.as_slice()));
                let symbol = crate::tone::symbol_samples();
                for block in [160usize, 512, 1600, 4096] {
                    let mut rx = StreamingReceiver::new(params, 6.0, true);
                    let mut got = Vec::new();
                    let mut announced = Vec::new();
                    for chunk in signal.chunks(block) {
                        got.extend(rx.feed(chunk));
                        for pending in rx.take_preambles() {
                            announced.push((pending, rx.samples_seen()));
                        }
                    }
                    let what = format!("{params:?} control {control}, block {block}");
                    assert_eq!(got.len(), 1, "{what}: {} frames", got.len());
                    assert_eq!(got[0].payload, offline[0].payload, "{what}: payload");
                    let start = got[0].frame.start();
                    assert_eq!(start, offline[0].frame.start(), "{what}: start");
                    assert_eq!(announced.len(), 1, "{what}: announced {announced:?}");
                    let (pending, at) = announced[0];
                    assert!(pending.tone && pending.control == control, "{what}");
                    assert!(pending.start.abs_diff(start) <= symbol / 4, "{what}");
                    assert!(pending.detect_confidence >= 1.0, "{what}");
                    let budget = (crate::tone::SYNC_SYMBOLS + 4) * symbol + block;
                    assert!(
                        at <= start + budget,
                        "{what}: announced {} symbols in",
                        (at - start) as f64 / symbol as f64
                    );
                }
            }
        }
    }

    #[test]
    fn an_ofdm_burst_raises_no_tone_frame_and_a_tone_frame_no_ofdm_one() {
        // the two families share the stream: neither detector may take the other's frames
        let mut modem = Modem::default();
        let ofdm = burst(
            &mut modem,
            &[(3, payload_for(3, 7)), (9, payload_for(9, 2))],
        );
        let mut rx = StreamingReceiver::default();
        let got: Vec<DecodedFrame> = ofdm.chunks(160).flat_map(|c| rx.feed(c)).collect();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|f| !f.frame.is_tone() && f.ok()));
        let (tone, _) = tone_burst(WIDE_2300, false);
        let mut rx = StreamingReceiver::default();
        let got: Vec<DecodedFrame> = tone.chunks(160).flat_map(|c| rx.feed(c)).collect();
        assert_eq!(got.len(), 1);
        assert!(got[0].frame.is_tone() && got[0].ok());
    }
}
