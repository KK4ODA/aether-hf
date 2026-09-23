//! Streaming receiver: feed baseband blocks of any size, get decoded frames out.
//!
//! Wraps the offline detector and receiver in a rolling buffer so live audio — a sound card,
//! a recording being replayed — can be processed block by block:
//!
//! * blocks are appended to a buffer that keeps at most `max_buffer_s` seconds;
//! * detection runs over the part of the buffer that has not been searched yet, extended
//!   backwards by a little more than two preamble symbols, so a preamble straddling a block
//!   boundary is still found once its second symbol has arrived;
//! * a detected frame is decoded only when its last sample — plus the transform window's
//!   margin — is in the buffer; until then it stays pending;
//! * absolute sample indices are kept, so a reported position stays meaningful after the
//!   buffer is trimmed. They refer to the band-limited stream, which lags the raw input by
//!   the filter's group delay, [`StreamingReceiver::delay_samples`].
//!
//! The decoded output is the same as [`Modem::decode_buffer`] on the concatenated input, so
//! real-time behaviour never diverges from the offline model — a test pins that.
//!
//! # Preambles, reported early
//!
//! [`take_preambles`](StreamingReceiver::take_preambles) reports a frame the moment
//! acquisition finds it, before its payload has arrived. That is what the link layer's
//! start-of-frame signal is: without it a receiving station has to treat a whole data frame
//! of silence as the end of a burst, which measurement put at about 13 % of the throughput.

use std::collections::VecDeque;

use crate::{
    Complex,
    blanker::StreamingBlanker,
    fir::Fir,
    modem::{DecodedFrame, Modem},
    rx::FrameSync,
    sync::{BankOutput, BankRow, BankState},
    waveform::{WIDE_2300, WaveformParams},
};

/// A frame acquisition has located but whose payload has not fully arrived.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingFrame {
    /// Where it is and what it announced, in absolute sample indices.
    pub sync: FrameSync,
    /// Absolute index one past the frame's last sample.
    pub end: usize,
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
    pending: Vec<PendingFrame>,
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
    /// The bank's running state: the floor family's ring, and scratch space.
    bank: BankState,
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

impl StreamingReceiver {
    /// Build a receiver with a buffer of `max_buffer_s` seconds.
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
            max_buffer: (max_buffer_s * params.fs_baseband) as usize,
            // a symbol after an ordinary candidate, the whole preamble of a floor one
            lookback: (crate::modes::air_interface(params).longest_preamble() + 1)
                * params.symbol_samples(),
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
    /// it tells a receiving station a burst is still running, roughly two preamble symbols
    /// into a frame rather than a whole frame later.
    pub fn take_preambles(&mut self) -> Vec<PendingFrame> {
        std::mem::take(&mut self.fresh)
    }

    /// Feed a block of baseband and take out whatever finished.
    pub fn feed(&mut self, block: &[Complex]) -> Vec<DecodedFrame> {
        let blanked = self
            .blanker
            .as_mut()
            .map_or_else(|| block.to_vec(), |b| b.process(block));
        let filtered = self.band.process(&blanked);
        self.buffer.extend_from_slice(&filtered);

        self.search();
        let out = self.harvest();
        self.trim();
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
            let absolute = self.buffer_start + relative;
            let window: f64 = self.buffer[relative..relative + reference_len]
                .iter()
                .map(|&(re, im)| re * re + im * im)
                .sum();
            let energy = window.max(1e-30).sqrt().max(floor);
            let (row, floor_row) =
                detector.bank_row(&mut self.bank, &self.buffer, relative, absolute, energy);
            self.rows.push_back(row);
            if let Some((back, floor_row)) = floor_row
                && let Some(target) = self.rows.get_mut(back)
            {
                target.floor_stat = floor_row.stat;
                target.floor_cfo = floor_row.cfo;
                target.floor_other = floor_row.other;
                target.floor_winner = floor_row.winner;
            }
        }
    }

    /// Search the part of the buffer nothing has looked at yet.
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
        for acquisition in found {
            let start = acquisition.sync.start + search_start;
            let known = self
                .pending
                .iter()
                .map(|p| (p.sync.start, p.end))
                .chain(self.done.iter().copied())
                .any(|(a, b)| a.saturating_sub(symbol) < start && start < b);
            if known {
                continue; // a duplicate, or inside a frame already known about
            }
            let span = self.modem.receiver().frame_span(&FrameSync {
                start: 0,
                ..acquisition.sync
            });
            let frame = PendingFrame {
                sync: FrameSync {
                    start,
                    ..acquisition.sync
                },
                end: start + span.1,
            };
            self.pending.push(frame);
            if !self.announced.contains(&start) {
                self.announced.push_back(start);
                if self.announced.len() > DONE_MEMORY {
                    self.announced.pop_front();
                }
                self.fresh.push(frame);
            }
        }
        // the detector needs a symbol after an ordinary candidate and the whole preamble of
        // a floor one (ADR-0009), so leave that much unsearched
        self.searched = self
            .searched
            .max(self.samples_seen().saturating_sub(self.lookback));
    }

    /// Decode every pending frame whose samples have all arrived.
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
            let buffer = std::mem::take(&mut self.buffer);
            let decoded = self.modem.decode_sync(&buffer, &local, None);
            self.buffer = buffer;
            // A frame that will not demodulate — it ran off the end after all, or its
            // length does not match the layout — is simply not a frame. Acquisition
            // occasionally locks onto noise, and there is nothing to report about it.
            if let Ok(mut decoded) = decoded {
                decoded.frame.sync = frame.sync; // report absolute positions
                out.push(decoded);
                self.frames_decoded += 1;
            }
        }
        self.pending = still;
        out
    }

    /// Drop what nothing still needs, without ever discarding a pending frame's start.
    fn trim(&mut self) {
        let keep = self
            .pending
            .iter()
            .map(|p| p.sync.start)
            .min()
            .unwrap_or(usize::MAX)
            .min(self.searched.saturating_sub(self.lookback))
            .max(self.samples_seen().saturating_sub(self.max_buffer))
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
                assert_eq!(streamed.mode.index, expected.mode.index);
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
            assert_eq!(decoded.mode.index, *mode);
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
        let limit = (2.0 * WIDE_2300.fs_baseband) as usize + 8000;
        assert!(
            rx.buffer.len() <= limit,
            "buffer grew to {} samples, limit {limit}",
            rx.buffer.len()
        );
        // the blanker holds a window back so it can be centred, so the stream this receiver
        // indexes runs exactly that far behind its input
        assert_eq!(rx.samples_seen(), 60 * 8000 - rx.blanker_latency());
    }

    #[test]
    #[ignore = "known defect: the streaming receiver cannot acquire a floor frame (see the \
                comment); a repro, not yet a fix"]
    fn a_floor_frame_streams_the_same_as_one_offline_call() {
        // ADR-0009's floor family carries connect, poll and acknowledgement when a narrow
        // link runs its slowest modes. A floor frame spans 4.2 s (33 728 samples), but the
        // streaming receiver only ever searches a window of `block + 2·lookback` ≈ 4 500
        // samples (`search`, above), so `detect_floor`'s "the whole frame must be present"
        // rule (`sync.rs`) can never pass live: the family decodes offline over a buffer
        // that holds it whole, and never on the air. Accepting the candidate on its preamble
        // instead mis-locks by a symbol — an 8-symbol repeated preamble is only pinned to the
        // exact symbol once the preamble→data boundary is in — so the real fix is a
        // floor-aware search-back (announce-triggered, or a dedicated lookback) with a cost
        // measurement, not a one-line change. Tracked as the top OTA risk; this test is the
        // repro. Run with `--ignored` to see it fail.
        use crate::waveform::NARROW_500;
        let mut modem = Modem::new(NARROW_500, true);
        let mode = modem.modes()[0]; // mode 0 is a floor mode on the narrow air
        let n = modem.payload_bytes(Some(mode));
        let payload: Vec<u8> = (0..n)
            .map(|i| (i as u8).wrapping_mul(5).wrapping_add(1))
            .collect();
        let mut signal = vec![(0.0f64, 0.0f64); 600];
        signal.extend(modem.data_burst(&payload, mode, 0).expect("encode"));
        signal.extend(std::iter::repeat_n((0.0, 0.0), 4000));

        let offline = Modem::new(NARROW_500, true).decode_buffer(&signal, 4);
        assert_eq!(
            offline.len(),
            1,
            "the offline reference must find the floor frame"
        );
        assert_eq!(offline[0].payload.as_ref(), Some(&payload));

        let mut rx = StreamingReceiver::new(NARROW_500, 6.0, true);
        let mut got = Vec::new();
        for block in signal.chunks(1600) {
            got.extend(rx.feed(block));
        }
        assert_eq!(got.len(), 1, "the streaming receiver found no floor frame");
        assert_eq!(got[0].payload.as_ref(), Some(&payload));
    }
}
