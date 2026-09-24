//! Streaming receiver: feed baseband blocks of any size, get decoded frames out.
//!
//! Wraps the offline detector and receiver in a rolling buffer so live audio — a sound card,
//! a recording being replayed — can be processed block by block:
//!
//! * blocks are appended to a buffer that keeps at most `max_buffer_s` seconds;
//! * detection runs over the part of the buffer that has not been searched yet, extended
//!   backwards by the detector's [`stream_lookback`](crate::sync::FrameDetector::stream_lookback),
//!   so a preamble straddling a block boundary is still found once enough of what follows it
//!   has arrived — a symbol for an ordinary frame, eighteen for a floor one (ADR-0009 §8);
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
    sync::{Acquisition, BankOutput, BankRow, BankState, FLOOR_OVER_ORDINARY},
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
    /// Floor frames the last search saw arriving but could not take as final yet.
    arriving: Vec<PendingFrame>,
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
        let lookback = modem.detector().stream_lookback();
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
            arriving: Vec::new(),
            max_buffer: (max_buffer_s * params.fs_baseband) as usize,
            // a symbol after an ordinary candidate, and far enough past a floor one that
            // nothing still to come could claim it (ADR-0009 §8)
            lookback,
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
    /// into a frame rather than a whole frame later. A floor frame is reported as soon as
    /// its preamble is in, well before acquisition takes it as final; should the final start
    /// differ, it is reported again there.
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
        // a floor frame is taken once nothing still to come could claim it — long before its
        // end — so its span keeps its own body's phantoms out (ADR-0009 §8)
        let found = self
            .modem
            .detector()
            .detect_streaming(region, &output, MAX_FRAMES_PER_SEARCH);
        for acquisition in &found.frames {
            let frame = self.absolute(acquisition, search_start);
            if frame.sync.floor && !self.settle_floor(&frame) {
                continue; // an ordinary frame it clashes with is the stronger
            }
            if self.known(&frame) {
                continue; // a duplicate, or inside a frame already known about
            }
            self.pending.push(frame);
            self.announce(frame);
        }
        self.arriving = found
            .arriving
            .iter()
            .map(|acquisition| self.absolute(acquisition, search_start))
            .collect();
        // a floor frame on its way is announced now, not when it is final: that is up to
        // eighteen symbols in, and the link layer's reply delay expects the signal within ten
        for frame in self.arriving.clone() {
            let explained = self.pending.iter().any(|p| {
                !p.sync.floor
                    && overlap(p, &frame)
                    && frame.sync.timing_peak < FLOOR_OVER_ORDINARY * p.sync.timing_peak
            });
            if !explained && !self.known(&frame) {
                self.announce(frame);
            }
        }
        // the detector needs a symbol after an ordinary candidate and far more after a floor
        // one (ADR-0009 §8), so leave that much unsearched
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
            if frame.end + margin > seen || self.held(&frame) {
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

    /// An acquisition found `base` samples into the stream, with its span, in absolute indices.
    fn absolute(&self, acquisition: &Acquisition, base: usize) -> PendingFrame {
        let span = self.modem.receiver().frame_span(&FrameSync {
            start: 0,
            ..acquisition.sync
        });
        let start = acquisition.sync.start + base;
        PendingFrame {
            sync: FrameSync {
                start,
                ..acquisition.sync
            },
            end: start + span.1,
        }
    }

    /// Whether a frame duplicates, or starts inside, one already pending or handed out.
    fn known(&self, frame: &PendingFrame) -> bool {
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
        if self.announced.contains(&frame.sync.start) {
            return;
        }
        self.announced.push_back(frame.sync.start);
        if self.announced.len() > DONE_MEMORY {
            self.announced.pop_front();
        }
        self.fresh.push(frame);
    }

    /// An ordinary frame a floor frame still arriving would settle away — the phantom a
    /// floor preamble can raise on the ordinary references, complete long before the floor
    /// frame is final — waits for that decision instead of being decoded first.
    fn held(&self, frame: &PendingFrame) -> bool {
        !frame.sync.floor
            && self.arriving.iter().any(|f| {
                overlap(f, frame)
                    && f.sync.timing_peak >= FLOOR_OVER_ORDINARY * frame.sync.timing_peak
            })
    }

    /// A floor candidate, taken while its frame is still arriving, against the pending
    /// ordinary frames it clashes with — above all the phantoms its own body produced before
    /// the floor candidate was final, which offline settles in the same pass. As offline
    /// ([`FrameDetector::detect`]'s settling of the families): the floor frame stays, and
    /// they go, where its statistic is at least [`FLOOR_OVER_ORDINARY`] of theirs; otherwise
    /// it is dropped.
    ///
    /// [`FrameDetector::detect`]: crate::sync::FrameDetector::detect
    fn settle_floor(&mut self, floor: &PendingFrame) -> bool {
        let clashes = |p: &PendingFrame| !p.sync.floor && overlap(p, floor);
        if self.pending.iter().any(|p| {
            clashes(p) && floor.sync.timing_peak < FLOOR_OVER_ORDINARY * p.sync.timing_peak
        }) {
            return false;
        }
        self.pending.retain(|p| !clashes(p));
        true
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

/// Whether two frames' spans overlap.
fn overlap(a: &PendingFrame, b: &PendingFrame) -> bool {
    a.sync.start < b.end && b.sync.start < a.end
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

    /// A narrow floor frame — data (4.2 s) or control (2.2 s) — with a little noise either
    /// side, and its payload.
    fn floor_burst(control: bool) -> (Vec<Complex>, Vec<u8>) {
        use crate::waveform::NARROW_500;
        let mut modem = Modem::new(NARROW_500, true);
        let (burst, payload) = if control {
            let n = modem.payload_bytes(None);
            let payload: Vec<u8> = (0..n)
                .map(|i| (i as u8).wrapping_mul(11).wrapping_add(3))
                .collect();
            (
                modem.control_burst_of(&payload, 0, true).expect("encode"),
                payload,
            )
        } else {
            let mode = modem.modes()[0]; // mode 0 is a floor mode on the narrow air
            let n = modem.payload_bytes(Some(mode));
            let payload: Vec<u8> = (0..n)
                .map(|i| (i as u8).wrapping_mul(5).wrapping_add(1))
                .collect();
            (
                modem.data_burst(&payload, mode, 0).expect("encode"),
                payload,
            )
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
    fn a_floor_frame_streams_the_same_as_one_offline_call() {
        // ADR-0009's floor family carries connect, poll and acknowledgement when a narrow
        // link runs its slowest modes — the one set of modes that holds a link at −10 dB. A
        // floor frame spans 4.2 s (data) or 2.2 s (control), but the streaming search region
        // is a fraction of that, so until the receiver learned to take a floor frame as final
        // once nothing still to come could claim it (ADR-0009 §8), the family decoded
        // offline and never live. Every block size, the daemon's own 20 ms included — and
        // the frame is announced within the ten symbols the link layer's reply delay allows.
        use crate::waveform::NARROW_500;
        for control in [false, true] {
            let (signal, payload) = floor_burst(control);
            let offline = Modem::new(NARROW_500, true).decode_buffer(&signal, 4);
            assert_eq!(offline.len(), 1, "offline must find the floor frame alone");
            // the floor control container holds a byte more than `payload_bytes` sizes, so a
            // control payload comes back zero-padded
            let expected = offline[0].payload.clone().expect("offline decodes it");
            assert_eq!(&expected[..payload.len()], payload.as_slice());
            for block in [160usize, 512, 1600, 4096] {
                let mut rx = StreamingReceiver::new(NARROW_500, 6.0, true);
                let mut got = Vec::new();
                let mut announced_at = None;
                for chunk in signal.chunks(block) {
                    got.extend(rx.feed(chunk));
                    if announced_at.is_none() && !rx.take_preambles().is_empty() {
                        announced_at = Some(rx.samples_seen());
                    }
                }
                let payloads: Vec<Option<Vec<u8>>> =
                    got.iter().map(|f| f.payload.clone()).collect();
                assert_eq!(
                    payloads,
                    vec![Some(expected.clone())],
                    "control {control}, block {block}: streamed differs from offline"
                );
                let start = got[0].frame.sync.start;
                assert_eq!(
                    start, offline[0].frame.sync.start,
                    "control {control}, block {block}: start differs from offline"
                );
                let symbol = NARROW_500.symbol_samples();
                let budget = (rx.modem().air().longest_preamble() + 2) * symbol + block;
                let announced = announced_at.expect("the floor frame was never announced");
                assert!(
                    announced <= start + budget,
                    "control {control}, block {block}: announced {} symbols in",
                    (announced - start) as f64 / symbol as f64
                );
            }
        }
    }
}
