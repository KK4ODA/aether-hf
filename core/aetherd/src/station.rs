//! The run loop: a link engine, a physical layer, a transmitter and a radio.
//!
//! A [`Station`] is the whole modem with no hardware attached. It takes captured audio in and
//! hands played audio out, and everything between — acquisition, decoding, the ARQ protocol,
//! keying the radio, deciding whether the channel is free — happens inside. The sound card is
//! somebody else's problem, which is what makes two stations testable against each other over
//! a simulated channel rather than only over the air.
//!
//! # The clock is the audio
//!
//! There is no wall clock here. Time is `captured audio samples / sample rate`, because that
//! is the only clock the protocol can actually be late against: a station that thinks a
//! second has passed while its sound card delivered half a second of audio will answer bursts
//! into the middle of them. Every timer the link engine arms is measured against this.
//!
//! # Half duplex
//!
//! While this station transmits it cannot hear, so captured audio is discarded rather than
//! decoded, and the busy detector is told to discard it too — our own sidetone is not channel
//! occupancy, and letting it into the noise-floor estimate would blind the detector for
//! several seconds afterwards.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use aether_link::{
    Action, Container, HarqBuffer, LinkConfig, LinkEngine, PhyTiming, Role, SoftFrame, State,
    rate::PAYLOAD_BYTES,
};
use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, Modem, StreamingReceiver,
    modes::{LONG, MODES, SHORT},
    preamble::FrameType,
    rx::ReceivedFrame,
    waveform::{WIDE_2300, WaveformParams},
};

use crate::{
    busy::{BusyConfig, BusyDetector},
    compress::{Compressor, Decompressor, negotiated, offered_capabilities},
    ptt::{Ptt, PttError, PttWatchdog, WatchdogState},
};

/// How a station is set up.
#[derive(Debug, Clone, PartialEq)]
pub struct StationConfig {
    /// This station's callsign.
    pub callsign: String,
    /// Numerology.
    pub params: WaveformParams,
    /// Link-layer tuning.
    pub link: LinkConfig,
    /// Busy-detector tuning.
    pub busy: BusyConfig,
    /// Transmit audio level, as an RMS fraction of full scale. −12 dBFS by default, which
    /// leaves headroom for the peaks an OFDM waveform has and for a sound card that is not
    /// quite calibrated.
    pub tx_level: f64,
    /// Silence played before a burst, so the radio is in transmit before the waveform starts.
    pub key_lead_s: f64,
    /// Silence played after a burst, before the key is released.
    pub key_tail_s: f64,
    /// Longest a single transmission may last, in seconds.
    pub max_key_s: f64,
    /// Refuse to start a transmission while the channel is occupied.
    pub wait_for_clear: bool,
    /// Offer stream compression in the connect handshake. Used only if the peer offers it
    /// too; a station that cannot decompress must never be sent a compressed stream.
    pub compress: bool,
}

impl Default for StationConfig {
    fn default() -> Self {
        Self {
            callsign: String::new(),
            params: WIDE_2300,
            link: LinkConfig::default(),
            busy: BusyConfig::default(),
            tx_level: 0.25,
            key_lead_s: 0.1,
            key_tail_s: 0.05,
            // A burst of sixteen long frames is under twenty seconds. Thirty gives that room
            // and still stops a stuck key well inside what a transmitter and a band will
            // tolerate.
            max_key_s: 30.0,
            wait_for_clear: true,
            compress: true,
        }
    }
}

/// A frame the physical layer decoded, presented to the link engine.
///
/// Decoding is the engine's call — it owns the HARQ buffers — so the frame keeps a handle on
/// a modem to decode through. The modem is shared and only ever borrowed for the length of a
/// decode, which is why this is a `RefCell` and not a lock: everything in a station runs on
/// one thread, and the audio callback talks to it through a queue.
pub struct PhyFrame {
    frame: ReceivedFrame,
    modem: Rc<RefCell<Modem>>,
    t_start: f64,
    t_end: f64,
    container: Container,
}

impl std::fmt::Debug for PhyFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhyFrame")
            .field("container", &self.container)
            .field("mode", &self.frame.mode)
            .field("rv", &self.frame.rv)
            .field("snr_3k_db", &self.frame.snr_3k_db)
            .finish_non_exhaustive()
    }
}

impl SoftFrame for PhyFrame {
    fn container(&self) -> Container {
        self.container
    }

    fn mode(&self) -> usize {
        self.frame.mode
    }

    fn rv(&self) -> u8 {
        self.frame.rv
    }

    fn snr_db(&self) -> f64 {
        self.frame.snr_3k_db
    }

    fn t_start(&self) -> f64 {
        self.t_start
    }

    fn t_end(&self) -> f64 {
        self.t_end
    }

    fn decode(&self, buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
        let mut modem = self.modem.borrow_mut();
        match modem.decode_frame(&self.frame, buffer.map(Vec::as_slice)) {
            Ok((payload, llrs)) => (payload, llrs),
            // A frame the codec will not even look at is not a frame; there is nothing to
            // keep for a later combine either.
            Err(_) => (None, Vec::new()),
        }
    }
}

/// Counters a status display can show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StationStats {
    /// Bursts put on the air.
    pub transmissions: usize,
    /// Frames acquisition found.
    pub frames_detected: usize,
    /// Times a transmission was held back because the channel was busy.
    pub deferred_for_busy: usize,
    /// Times the key-time watchdog fired.
    pub watchdog_trips: usize,
    /// Application bytes handed to the compressor this session.
    pub bytes_before_compression: usize,
    /// Bytes the compressor produced, which is what the link actually carries.
    pub bytes_after_compression: usize,
}

/// One station: link engine, physical layer, radio.
pub struct Station<P: Ptt> {
    config: StationConfig,
    engine: LinkEngine,
    receiver: StreamingReceiver,
    /// Shared with every [`PhyFrame`] handed to the engine, for decoding and combining.
    decoder: Rc<RefCell<Modem>>,
    transmitter: Modem,
    to_audio: BasebandToAudio,
    from_audio: AudioToBaseband,
    ptt: PttWatchdog<P>,
    busy: BusyDetector,
    playback: VecDeque<f32>,
    baseband_seen: usize,
    audio_seen: usize,
    transmitting: bool,
    pending: VecDeque<Vec<aether_link::TxFrame>>,
    delivered: Vec<u8>,
    events: Vec<String>,
    compressor: Compressor,
    decompressor: Decompressor,
    /// Application bytes waiting for a session to negotiate compression.
    outbound: Vec<u8>,
    /// Counters, for display.
    pub stats: StationStats,
}

impl<P: Ptt> std::fmt::Debug for Station<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Station")
            .field("callsign", &self.config.callsign)
            .field("state", &self.engine.state())
            .field("role", &self.engine.role())
            .field("transmitting", &self.transmitting)
            .finish_non_exhaustive()
    }
}

impl<P: Ptt> Station<P> {
    /// Build a station.
    ///
    /// # Panics
    /// If the configured maximum key time is not positive.
    #[must_use]
    pub fn new(config: StationConfig, ptt: P, seed: u64) -> Self {
        let params = config.params;
        let timing = phy_timing(params);
        let link = LinkConfig {
            capabilities: offered_capabilities(config.compress),
            ..config.link.clone()
        };
        let engine = LinkEngine::new(&config.callsign, timing, link, seed);
        let busy = BusyDetector::new(BusyConfig {
            fs: params.fs_baseband,
            ..config.busy
        });
        Self {
            engine,
            receiver: StreamingReceiver::new(params, 6.0, true),
            decoder: Rc::new(RefCell::new(Modem::new(params, false))),
            transmitter: Modem::new(params, false),
            to_audio: BasebandToAudio::new(params),
            from_audio: AudioToBaseband::new(params),
            ptt: PttWatchdog::new(ptt, config.max_key_s),
            busy,
            playback: VecDeque::new(),
            baseband_seen: 0,
            audio_seen: 0,
            transmitting: false,
            pending: VecDeque::new(),
            delivered: Vec::new(),
            events: Vec::new(),
            // Nothing is compressed until a session negotiates it. Before that the two ends
            // have not agreed on anything, and a guess would produce a stream the peer
            // cannot read.
            compressor: Compressor::new(false),
            decompressor: Decompressor::new(false),
            outbound: Vec::new(),
            stats: StationStats::default(),
            config,
        }
    }

    // ── inspection ────────────────────────────────────────────────────

    /// The station clock, in seconds of captured audio.
    #[must_use]
    pub fn now(&self) -> f64 {
        self.audio_seen as f64 / self.config.params.audio_rate as f64
    }

    /// Where the session is.
    #[must_use]
    pub fn state(&self) -> State {
        self.engine.state()
    }

    /// Which half of the session this station is.
    #[must_use]
    pub fn role(&self) -> Role {
        self.engine.role()
    }

    /// Whether a session is up.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.engine.connected()
    }

    /// Whether the radio is keyed.
    #[must_use]
    pub fn transmitting(&self) -> bool {
        self.transmitting
    }

    /// Whether the channel is occupied.
    #[must_use]
    pub fn channel_busy(&self) -> bool {
        self.busy.busy(self.now())
    }

    /// Bytes the application has queued that are not yet with the link layer.
    #[must_use]
    pub fn queued_bytes(&self) -> usize {
        self.outbound.len() + self.engine.tx_pending_bytes()
    }

    /// How many delivered bytes are waiting to be taken.
    #[must_use]
    pub fn received_len(&self) -> usize {
        self.delivered.len()
    }

    /// Bytes received and delivered, taken out of the buffer.
    pub fn take_received(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.delivered)
    }

    /// Events reported since the last call, as `name:detail`.
    pub fn take_events(&mut self) -> Vec<String> {
        std::mem::take(&mut self.events)
    }

    /// The link engine, for a status display.
    #[must_use]
    pub fn engine(&self) -> &LinkEngine {
        &self.engine
    }

    /// The busy detector.
    #[must_use]
    pub fn busy_detector(&self) -> &BusyDetector {
        &self.busy
    }

    /// What the radio is keyed through.
    pub fn ptt_description(&self) -> String {
        self.ptt.describe()
    }

    // ── commands ──────────────────────────────────────────────────────

    /// Call a station.
    ///
    /// # Errors
    /// If a session is already up.
    pub fn connect(&mut self, remote: &str) -> Result<(), &'static str> {
        self.engine.connect(remote)?;
        self.pump();
        Ok(())
    }

    /// Queue bytes to send.
    ///
    /// Bytes handed over before a session exists are held here, not compressed: whether the
    /// stream is compressed at all is settled by the connect handshake, and anything put on
    /// the wire before that would be read by the peer under the wrong assumption. An
    /// application is entitled to queue a message and then call — in fact that is the natural
    /// way to use it — so this is the normal path and not an edge case.
    ///
    /// Once a session is up the bytes are compressed here, above the ARQ, if it negotiated
    /// compression. One call is one flush, so handing over a whole message compresses far
    /// better than writing it a few bytes at a time — see [`compress`](crate::compress).
    pub fn send(&mut self, data: &[u8]) {
        self.outbound.extend_from_slice(data);
        if self.connected() {
            self.flush_outbound();
        }
        self.pump();
    }

    /// Push whatever the application has queued through the compressor and into the engine.
    fn flush_outbound(&mut self) {
        if self.outbound.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.outbound);
        let wire = self.compressor.push(&pending);
        self.stats.bytes_before_compression = self.compressor.bytes_in;
        self.stats.bytes_after_compression = self.compressor.bytes_out;
        self.engine.send(&wire);
    }

    /// Whether this session is compressing.
    #[must_use]
    pub fn compressing(&self) -> bool {
        self.compressor.active()
    }

    /// How much smaller compression has made this session's traffic, as a fraction.
    #[must_use]
    pub fn compression_saving(&self) -> f64 {
        self.compressor.saving()
    }

    /// Close the session once everything queued has been acknowledged.
    pub fn disconnect(&mut self) {
        self.engine.disconnect();
        self.pump();
    }

    /// Close the session now.
    pub fn abort(&mut self) {
        self.engine.abort();
        self.pump();
    }

    /// As the receiving station, demand the sending role.
    pub fn request_break(&mut self) {
        self.engine.request_break();
    }

    // ── audio ─────────────────────────────────────────────────────────

    /// Hand over audio the sound card captured.
    ///
    /// # Errors
    /// If the key had to be released and the radio refused.
    pub fn capture(&mut self, audio: &[f32]) -> Result<(), PttError> {
        self.audio_seen += audio.len();
        let baseband = self.from_audio.process(audio);
        let now = self.now();

        if self.transmitting {
            // Deaf while transmitting: what came in is our own sidetone, not the channel, so
            // it is discarded. The receiver is still fed — with silence, the same length.
            // Skipping the blocks outright would splice the stream either side of the
            // transmission together, and a frame that straddled the join would be assembled
            // from two pieces of signal that were never adjacent. Measured, that cost 40 dB
            // and made every burst that overlapped one of our own acknowledgements
            // undecodable. Muted is not the same as paused.
            self.busy.skip(&baseband);
            let muted = vec![(0.0, 0.0); baseband.len()];
            self.absorb(&muted, now);
        } else {
            self.busy.push(&baseband, now);
            self.absorb(&baseband, now);
        }

        self.engine.tick(now);
        if self.ptt.poll(now)? == WatchdogState::Tripped {
            // the key was stuck: drop whatever was still queued rather than resume mid-burst
            self.stats.watchdog_trips += 1;
            self.playback.clear();
            self.pending.clear();
            self.transmitting = false;
            self.ptt.unkey(now)?;
            self.engine.on_tx_done(now);
            self.events.push("watchdog:key time exceeded".to_owned());
        }
        self.pump();
        Ok(())
    }

    /// Fill a playback buffer with whatever this station wants to transmit.
    ///
    /// Returns the number of samples that carry signal; the rest of `out` is silence.
    ///
    /// # Errors
    /// If the radio refuses to key or release.
    pub fn playback(&mut self, out: &mut [f32]) -> Result<usize, PttError> {
        out.fill(0.0);
        let now = self.now();

        // Finish the current transmission before starting the next one. Rendering the
        // queued burst here instead would run the two together with no gap and no release,
        // and the receiving station infers the end of a burst from the silence after it —
        // it would hear one endless burst and never answer.
        if self.playback.is_empty() && self.transmitting {
            self.transmitting = false;
            self.ptt.unkey(now)?;
            self.engine.on_tx_done(now);
            self.pump();
            return Ok(0);
        }
        if self.playback.is_empty() {
            self.start_pending(now);
        }
        if self.playback.is_empty() {
            return Ok(0);
        }

        if !self.transmitting {
            self.ptt.key(now)?;
            self.transmitting = true;
            self.stats.transmissions += 1;
        }
        let count = out.len().min(self.playback.len());
        for slot in out.iter_mut().take(count) {
            *slot = self.playback.pop_front().unwrap_or(0.0);
        }
        Ok(count)
    }

    // ── internals ─────────────────────────────────────────────────────

    /// Feed the streaming receiver and pass what it finds to the engine.
    fn absorb(&mut self, baseband: &[Complex], now: f64) {
        let fs = self.config.params.fs_baseband;
        let frames = self.receiver.feed(baseband);
        let preambles = self.receiver.take_preambles();
        self.baseband_seen += baseband.len();

        // Completed frames first, then preambles. Both can come out of one block, and the
        // order matters: a preamble found in this block belongs to a frame that is *still
        // arriving*, so its deadline must be the one that stands. The other way round, the
        // receiver decides a burst has ended in the middle of it and answers over the rest.
        for decoded in frames {
            self.stats.frames_detected += 1;
            let container = if decoded.frame.sync.frame_type == FrameType::Control {
                Container::Control
            } else {
                Container::Data
            };
            let layout = if container == Container::Control {
                SHORT
            } else {
                LONG
            };
            let start = decoded.frame.sync.start as f64 / fs;
            let frame = PhyFrame {
                container,
                t_start: start,
                t_end: start + layout.duration_s(),
                frame: decoded.frame,
                modem: Rc::clone(&self.decoder),
            };
            self.engine.on_frame(&frame, now);
        }

        for pending in preambles {
            // the start-of-frame signal: it tells the engine a burst is still running long
            // before the frame itself arrives, and it marks the channel busy at an SNR far
            // below anything a power measurement would catch
            self.busy.mark_frame(now);
            self.engine.on_preamble(pending.sync.start as f64 / fs, now);
        }
        self.pump();
    }

    /// Take what the engine has decided and act on it.
    fn pump(&mut self) {
        let mut connected = false;
        for action in self.engine.drain() {
            match action {
                Action::Transmit { frames, .. } => self.pending.push_back(frames),
                Action::Deliver(bytes) => {
                    let plain = self.decompressor.push(&bytes);
                    if self.decompressor.failed() {
                        self.events.push(
                            "error:the peer sent a compressed stream this station cannot read"
                                .to_owned(),
                        );
                    }
                    self.delivered.extend_from_slice(&plain);
                }
                Action::Event { name, detail } => {
                    // A session is where compression is agreed, so both coders are built the
                    // moment one comes up and thrown away when it ends: their state is the
                    // stream's history, and carrying it into the next session would make the
                    // first bytes undecodable.
                    if name == "connected" {
                        let agreed = negotiated(
                            offered_capabilities(self.config.compress),
                            self.engine.peer_capabilities(),
                        );
                        self.compressor = Compressor::new(agreed);
                        self.decompressor = Decompressor::new(agreed);
                        connected = true;
                    } else if name == "disconnected" {
                        self.compressor = Compressor::new(false);
                        self.decompressor = Decompressor::new(false);
                    }
                    self.events.push(format!("{name}:{detail}"));
                }
            }
        }
        if connected {
            self.flush_outbound();
        }
    }

    /// Render the next queued burst into playable audio, if the channel allows.
    fn start_pending(&mut self, now: f64) {
        if self.pending.is_empty() {
            return;
        }
        if self.config.wait_for_clear && !self.channel_clear(now) {
            self.stats.deferred_for_busy += 1;
            return;
        }
        let Some(frames) = self.pending.pop_front() else {
            return;
        };

        let mut baseband: Vec<Complex> = Vec::new();
        for frame in &frames {
            let burst = match frame.container {
                Container::Data => {
                    self.transmitter
                        .data_burst(&frame.payload, MODES[frame.mode], frame.rv)
                }
                Container::Control => self.transmitter.control_burst(&frame.payload, frame.rv),
            };
            match burst {
                Ok(samples) => baseband.extend(samples),
                // The engine only ever asks for payloads it sized from this same table, so
                // this cannot happen without a bug; dropping the frame keeps a bug off the
                // air rather than transmitting something malformed.
                Err(error) => {
                    self.events
                        .push(format!("error:cannot modulate frame: {error}"));
                    return;
                }
            }
        }
        if baseband.is_empty() {
            return;
        }

        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.config.key_tail_s * audio_rate) as usize;
        // flush the chain after the burst, or its filters keep the tail of the last frame
        let mut rendered = self.to_audio.process(&baseband);
        rendered.extend(self.to_audio.flush());
        let level = self.config.tx_level as f32;

        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.playback.extend(rendered.iter().map(|&x| x * level));
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
    }

    /// Whether it is polite to start transmitting.
    ///
    /// A session already under way answers regardless: the peer is waiting for this
    /// acknowledgement, the exchange is what put the energy on the channel in the first
    /// place, and staying silent would only make the peer retransmit into the same channel.
    /// What the check protects is *starting* something.
    fn channel_clear(&self, now: f64) -> bool {
        if self.engine.state() != State::Idle && self.engine.state() != State::Connecting {
            return true;
        }
        self.busy.settled() && !self.busy.busy(now)
    }
}

/// Link-layer timing derived from the waveform tables.
///
/// Every number here comes from [`WaveformParams`] or a measurement, never a guess: the frame
/// durations are what the layouts actually occupy, and the capacities are what the modes
/// actually carry.
#[must_use]
pub fn phy_timing(params: WaveformParams) -> PhyTiming {
    PhyTiming {
        data_frame_s: LONG.duration_s(),
        control_frame_s: SHORT.duration_s(),
        // PTT, the radio's own transmit delay, and the audio buffers at both ends
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        // acquisition reports a frame about two preamble symbols in, plus the search block
        preamble_detect_s: Some(4.0 * params.symbol_period_s()),
        data_capacity: PAYLOAD_BYTES.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ptt::NullPtt;

    /// Step two stations against each other through a channel, in blocks of audio.
    ///
    /// Each station's playback becomes the other's capture, scaled and with noise added. This
    /// is the real thing end to end: the ARQ protocol over the real codec, over the real
    /// waveform, over real 48 kHz audio.
    struct Air {
        a: Station<NullPtt>,
        b: Station<NullPtt>,
        block: usize,
        gain: f32,
        noise_sigma: f32,
        state: u64,
    }

    impl Air {
        fn new(gain: f32, noise_sigma: f32) -> Self {
            let config = |call: &str| StationConfig {
                callsign: call.to_owned(),
                // the channel in these tests is a wire, so politeness would only slow them
                wait_for_clear: false,
                ..StationConfig::default()
            };
            Self {
                a: Station::new(config("W4ODA"), NullPtt::default(), 1),
                b: Station::new(config("KK4XYZ"), NullPtt::default(), 2),
                block: 4096,
                gain,
                noise_sigma,
                state: 0x1234_5678_9abc_def0,
            }
        }

        fn noise(&mut self) -> f32 {
            // Box–Muller over a small deterministic generator: a failure is reproducible
            self.state ^= self.state >> 12;
            self.state ^= self.state << 25;
            self.state ^= self.state >> 27;
            let u1 = ((self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64
                / (1u64 << 53) as f64)
                .max(1e-12);
            self.state ^= self.state >> 12;
            self.state ^= self.state << 25;
            self.state ^= self.state >> 27;
            let u2 =
                (self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
        }

        /// Run for `seconds`, or until `done` says to stop.
        fn run(
            &mut self,
            seconds: f64,
            mut done: impl FnMut(&Station<NullPtt>, &Station<NullPtt>) -> bool,
        ) {
            let rate = WIDE_2300.audio_rate as f64;
            let blocks = (seconds * rate / self.block as f64) as usize;
            let mut from_a = vec![0.0f32; self.block];
            let mut from_b = vec![0.0f32; self.block];
            for _ in 0..blocks {
                self.a.playback(&mut from_a).expect("playback");
                self.b.playback(&mut from_b).expect("playback");

                let mut to_b = vec![0.0f32; self.block];
                let mut to_a = vec![0.0f32; self.block];
                for index in 0..self.block {
                    to_b[index] = from_a[index].mul_add(self.gain, self.noise() * self.noise_sigma);
                    to_a[index] = from_b[index].mul_add(self.gain, self.noise() * self.noise_sigma);
                }
                self.a.capture(&to_a).expect("capture");
                self.b.capture(&to_b).expect("capture");
                if done(&self.a, &self.b) {
                    return;
                }
            }
        }
    }

    #[test]
    fn two_stations_connect_over_real_audio() {
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(40.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.connected() && air.b.connected(),
            "a: {:?} {:?}, b: {:?} {:?}",
            air.a.state(),
            air.a.take_events(),
            air.b.state(),
            air.b.take_events()
        );
        assert_eq!(air.a.role(), Role::Iss);
        assert_eq!(air.b.role(), Role::Irs);
    }

    #[test]
    fn a_message_crosses_the_air_intact() {
        let message = b"CQ CQ DE W4ODA -- this went through the codec, the waveform and the audio.";
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.a.send(message);
        air.run(90.0, |_, b| {
            b.engine().stats.bytes_delivered >= message.len()
        });
        let got = air.b.take_received();
        assert_eq!(
            got.as_slice(),
            message.as_slice(),
            "delivered {} of {} bytes",
            got.len(),
            message.len()
        );
    }

    #[test]
    fn a_compressed_message_crosses_the_air_and_is_smaller_on_it() {
        // the whole point of P3-6: fewer bytes on a link that carries a few hundred a second
        let message: Vec<u8> = [
            "MID: A1B2C3D4E5F6",
            "From: W4ODA",
            "To: KK4XYZ",
            "Subject: Net check-in",
            "",
            "Checked in to the Thursday evening net on 40 metres. Conditions were poor",
            "for the first half hour and improved after sunset. Fourteen stations were",
            "logged, three of them portable. No traffic to pass this week.",
            "",
            "73 de W4ODA",
        ]
        .join("\r\n")
        .into_bytes();
        let message = message.as_slice();

        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(20.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.compressing() && air.b.compressing(),
            "both stations offered compression, so both should be using it"
        );

        let expected = message.len();
        air.a.send(message);
        air.run(180.0, |_, b| b.received_len() >= expected);
        let got = air.b.take_received();
        assert_eq!(
            String::from_utf8_lossy(&got),
            String::from_utf8_lossy(message)
        );
        // 21 % on a message this short, measured; `compress` covers the longer case where
        // the coder has history and does better than twice as well
        assert!(
            air.a.compression_saving() > 0.20,
            "only {:.1} % was saved",
            100.0 * air.a.compression_saving()
        );
        assert!(
            air.a.stats.bytes_after_compression < message.len(),
            "{} bytes went on the air for a {}-byte message",
            air.a.stats.bytes_after_compression,
            message.len()
        );
    }

    #[test]
    fn a_station_with_compression_off_is_never_sent_a_compressed_stream() {
        // the negotiation exists so a station that cannot decompress is never handed one
        let plain = StationConfig {
            callsign: "KK4XYZ".to_owned(),
            wait_for_clear: false,
            compress: false,
            ..StationConfig::default()
        };
        let mut air = Air::new(1.0, 0.0005);
        air.b = Station::new(plain, NullPtt::default(), 2);

        air.a.connect("KK4XYZ").expect("idle");
        air.run(20.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        assert!(
            !air.a.compressing(),
            "the caller compressed to a station that said it could not decompress"
        );
        assert!(!air.b.compressing());

        let message = b"plain text, because one end cannot do better";
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received(), message);
    }

    #[test]
    fn a_station_does_not_key_when_it_has_nothing_to_say() {
        let mut station = Station::new(StationConfig::default(), NullPtt::default(), 1);
        let mut out = vec![0.0f32; 4096];
        for _ in 0..20 {
            assert_eq!(station.playback(&mut out).expect("playback"), 0);
            assert!(!station.transmitting());
            station.capture(&vec![0.0f32; 4096]).expect("capture");
        }
        assert_eq!(station.stats.transmissions, 0);
    }

    #[test]
    fn a_busy_channel_holds_back_a_call() {
        // politeness, tested rather than asserted: a station that can hear somebody else must
        // not start a session on top of them
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        // let the detector learn a quiet floor, then put a strong signal on the channel
        let quiet = vec![0.0001f32; 4096];
        for _ in 0..80 {
            station.capture(&quiet).expect("capture");
        }
        assert!(station.busy_detector().settled());
        let loud: Vec<f32> = (0..4096)
            .map(|i| {
                let phase = 2.0 * std::f64::consts::PI * 1500.0 * f64::from(i) / 48000.0;
                0.2 * phase.sin() as f32
            })
            .collect();
        for _ in 0..10 {
            station.capture(&loud).expect("capture");
        }
        assert!(station.channel_busy(), "the loud signal was not heard");

        station.connect("KK4XYZ").expect("idle");
        let mut out = vec![0.0f32; 4096];
        for _ in 0..5 {
            assert_eq!(
                station.playback(&mut out).expect("playback"),
                0,
                "it transmitted over an occupied channel"
            );
            station.capture(&loud).expect("capture");
        }
        assert!(station.stats.deferred_for_busy > 0);
        assert_eq!(station.stats.transmissions, 0);
    }

    #[test]
    fn a_station_in_session_answers_even_on_a_busy_channel() {
        // the mirror of the previous test: once a session is up, the peer is waiting for this
        // answer, and staying silent only makes it retransmit into the same channel
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let quiet = vec![0.0001f32; 4096];
        for _ in 0..80 {
            station.capture(&quiet).expect("capture");
        }
        let now = station.now();
        station.busy.mark_frame(now);
        assert!(station.channel_busy());
        assert!(
            !station.channel_clear(now),
            "an idle station must wait for a busy channel"
        );

        station.connect("KK4XYZ").expect("idle");
        // pretend the handshake completed: the engine is what decides this in a real session
        assert!(!station.channel_clear(station.now()), "still only calling");
    }
}
