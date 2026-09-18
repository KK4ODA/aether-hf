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

mod fieldtest;
pub use fieldtest::{Rung, Step, TestPlan, TestRun, Transfer};

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use aether_link::{
    Action, Container, HarqBuffer, LinkConfig, LinkEngine, PhyTiming, Role, SoftFrame, State,
    frames::{
        ConnectBody, DataHeader, DataKind, ProbeBody, decode_data, encode_data, pack_callsign,
        unpack_callsign, with_bandwidth,
    },
    rate::PAYLOAD_BYTES,
};
use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, Modem, StreamingReceiver,
    preamble::FrameType,
    rx::ReceivedFrame,
    waveform::{WIDE_2300, WaveformParams},
};

use crate::{
    busy::{BusyConfig, BusyDetector},
    compress::{Compressor, Decompressor, negotiated, offered_capabilities},
    cwid::CwId,
    ptt::{Ptt, PttError, PttWatchdog, WatchdogState},
    spectrum::{Spectrum, SpectrumAnalyser},
};

/// How a station is set up.
///
/// Each flag is a setting the operator names in the configuration file; a bit set would
/// make the file's keys and the struct's fields stop matching.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct StationConfig {
    /// This station's callsign.
    pub callsign: String,
    /// Numerology.
    pub params: WaveformParams,
    /// Answer calls but never make one, and never beacon (§97.221(c): an automatically
    /// controlled station may use 500 Hz outside the automatic sub-bands only to respond).
    pub answer_only: bool,
    /// Link-layer tuning.
    pub link: LinkConfig,
    /// Busy-detector tuning.
    pub busy: BusyConfig,
    /// Transmit level, as a fraction of full scale: the amplitude of a sine that has the
    /// same RMS as the data waveform, which is what the tune tone plays. The audio's RMS is
    /// therefore `tx_level / √2` — 0.25 is −15 dBFS RMS with peaks around −6 dBFS — which
    /// leaves headroom for a sound card that is not quite calibrated. Measured, not assumed:
    /// the simulated channel's SNR is set against this.
    pub tx_level: f64,
    /// Silence played before a burst, so the radio is in transmit before the waveform starts.
    pub key_lead_s: f64,
    /// Silence played after a burst, before the key is released — on top of the playback
    /// lead, which the tail always covers: see [`Station::new`].
    pub key_tail_s: f64,
    /// How far ahead of the audio clock the daemon keeps playback queued, in seconds. A
    /// sample handed to the sound card now leaves it this much later; the engine's timers
    /// are told, so a reply is waited for from when the burst really ended.
    pub playback_lead_s: f64,
    /// Longest a single transmission may last, in seconds.
    pub max_key_s: f64,
    /// Refuse to start a transmission while the channel is occupied.
    pub wait_for_clear: bool,
    /// Offer stream compression in the connect handshake. Used only if the peer offers it
    /// too; a station that cannot decompress must never be sent a compressed stream.
    pub compress: bool,
    /// Identify in Morse at the end of a transmission, and how. `None` for not at all, which
    /// is the default: Aether's own frames carry both callsigns, and whether that satisfies
    /// the local rules is something only the operator knows.
    pub cw_id: Option<CwId>,
    /// Where session recordings go. `None` means recording is refused.
    pub record_dir: Option<std::path::PathBuf>,
    /// Record every session on its own: start when one connects, stop when it ends. For a
    /// gateway, which nobody is watching, and for field validation, which wants all of them.
    pub record_auto: bool,
    /// What every recording that starts on its own should say about the station: the band,
    /// the antenna, whatever the operator would have written down had they been there.
    pub record_notes: String,
    /// Longest a station may transmit without a Morse identifier, in seconds. Ignored when
    /// `cw_id` is `None`. Ten minutes is the common regulatory figure.
    pub cw_id_interval_s: f64,
    /// Who and where, for the field log a recording becomes.
    pub operator: crate::config::OperatorSection,
}

impl Default for StationConfig {
    fn default() -> Self {
        Self {
            callsign: String::new(),
            params: WIDE_2300,
            answer_only: false,
            link: LinkConfig::default(),
            busy: BusyConfig::default(),
            tx_level: 0.25,
            key_lead_s: 0.1,
            key_tail_s: 0.05,
            playback_lead_s: 0.0,
            // A burst of sixteen long frames is under twenty seconds. Thirty gives that room
            // and still stops a stuck key well inside what a transmitter and a band will
            // tolerate.
            max_key_s: 30.0,
            wait_for_clear: true,
            compress: true,
            cw_id: None,
            record_dir: None,
            record_auto: false,
            record_notes: String::new(),
            cw_id_interval_s: 600.0,
            operator: crate::config::OperatorSection::default(),
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

    fn floor(&self) -> bool {
        self.frame.sync.floor
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

/// Something waiting to be transmitted.
enum Outgoing {
    /// Link-layer frames, to be modulated.
    Frames(Vec<aether_link::TxFrame>),
    /// A drive-setting burst: the real waveform, modulated exactly as traffic is, so the
    /// rig's ALC sees the peaks traffic will actually present it with. The operator may cut
    /// it short like a tune tone.
    Drive(Vec<aether_link::TxFrame>),
    /// Audio to play as it is: a keying test, or a tune tone.
    Audio {
        samples: Vec<f32>,
        /// Whether it is silence. Keying an SSB transmitter with no audio puts nothing on
        /// the air, so a keying test need not wait for the channel; a tone must.
        silent: bool,
        /// Whether it is a tune tone, which the operator may cut short.
        tone: bool,
    },
}

/// How much later than the playback lead a sound card's capture of this station's own
/// burst may still be arriving, over and above the lead itself: device buffering both ways.
const CAPTURE_LAG_ALLOWANCE_S: f64 = 0.15;

/// The Morse identifier's amplitude as a fraction of the transmit level: a little below the
/// data waveform's peak, because it is an identifier and not the signal, and scaled with
/// the drive so an operator who sets the level by the tone has set the identifier too.
const CW_ID_RELATIVE_LEVEL: f64 = 0.8;

/// What the sound card is delivering, over the last few seconds.
///
/// Setup, not propagation, is what defeats most new users of an HF data mode
/// (`COMMUNITY-CONCERNS.md`), and the audio level is the setting they get wrong most. A
/// meter that says "too quiet", "clipping" or "good" is worth more than any amount of
/// documentation about what a sound card mixer should look like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelReading {
    /// RMS over the window, in dB relative to full scale.
    pub rms_dbfs: f64,
    /// The loudest single sample, in dB relative to full scale.
    pub peak_dbfs: f64,
    /// Fraction of samples at or beyond full scale.
    pub clipping: f64,
    /// Whether enough audio has been seen for the reading to mean anything.
    pub settled: bool,
}

impl LevelReading {
    /// What an operator should do about it, in a sentence.
    ///
    /// The thresholds are the usual ones for a sound-card interface: clipping is never
    /// acceptable, a peak within a few dB of full scale leaves no headroom for a stronger
    /// station, and an RMS below about −40 dBFS is inside the card's own noise.
    #[must_use]
    pub fn advice(&self) -> &'static str {
        if !self.settled {
            "Still listening."
        } else if self.clipping > 0.0005 {
            "Clipping. Turn the radio's audio output or the sound card's input down."
        } else if self.peak_dbfs > -3.0 {
            "Almost clipping. Turn the input down a little to leave headroom."
        } else if self.rms_dbfs < -40.0 {
            "Very quiet. Turn the radio's audio output or the sound card's input up."
        } else {
            "Good."
        }
    }
}

/// A rolling meter over the last few seconds of captured audio.
#[derive(Debug, Clone)]
struct LevelMeter {
    window: usize,
    /// Per-block (RMS², peak, clipped count, length), newest last.
    blocks: VecDeque<(f64, f64, usize, usize)>,
    held: usize,
}

impl LevelMeter {
    fn new(sample_rate: f64, seconds: f64) -> Self {
        Self {
            window: (sample_rate * seconds) as usize,
            blocks: VecDeque::new(),
            held: 0,
        }
    }

    fn push(&mut self, audio: &[f32]) {
        if audio.is_empty() {
            return;
        }
        let mut power = 0.0f64;
        let mut peak = 0.0f64;
        let mut clipped = 0usize;
        for &sample in audio {
            let x = f64::from(sample).abs();
            power += x * x;
            peak = peak.max(x);
            if x >= 0.99 {
                clipped += 1;
            }
        }
        self.blocks.push_back((power, peak, clipped, audio.len()));
        self.held += audio.len();
        while self.held > self.window
            && let Some((_, _, _, length)) = self.blocks.pop_front()
        {
            self.held -= length;
        }
    }

    fn reading(&self) -> LevelReading {
        let floor = 1e-10f64;
        let mut power = 0.0;
        let mut peak = 0.0f64;
        let mut clipped = 0usize;
        for &(p, pk, c, _) in &self.blocks {
            power += p;
            peak = peak.max(pk);
            clipped += c;
        }
        let count = self.held.max(1) as f64;
        LevelReading {
            rms_dbfs: 10.0 * (power / count).max(floor).log10(),
            peak_dbfs: 20.0 * peak.max(floor).log10(),
            clipping: clipped as f64 / count,
            settled: self.held >= self.window / 2,
        }
    }
}

/// How far above its acceptance threshold acquisition has to have seen a frame before the
/// detection is worth believing on its own.
///
/// Set from the 300-frame OTA-2 record (`field/OTA-2-FINDINGS.md`), where a crowded 40 m
/// produced 13.6 false acquisitions a minute: every frame that decoded cleared its
/// threshold by a wide margin, and the noise triggers sat just above it, which is what a
/// threshold set on the statistic's maximum over *noise* implies. This is deliberately
/// only a reporting gate — raising the detector's own threshold is ADR business, because
/// it would cost weak-signal frames that do decode.
pub const DETECT_CONFIDENCE_TRUSTED: f64 = 1.3;

/// The carrier offset worth reporting. It is real when the frame decoded, or when
/// acquisition itself was confident enough to trust; `None` for a probable noise trigger,
/// whose offset is the correlator locking onto noise, not a real frequency error. Nothing
/// in the protocol reads carrier offset — this only keeps a phantom number off the panel
/// and out of the sidecar, where one once sent an on-air analysis chasing a rig fault that
/// did not exist.
///
/// `mode_confidence` is read from the pilot chips, which only a DATA frame carries, so a
/// CONTROL frame always reports 1.0 there and this used to suppress the offset of every
/// one of them — including the connect, poll and acknowledgement frames the link cannot do
/// without. `detect_confidence` is defined for both, so it is what decides.
#[must_use]
pub fn reported_cfo(
    decoded: bool,
    confidence: f64,
    detect_confidence: f64,
    cfo_hz: f64,
) -> Option<f64> {
    let sure = confidence >= aether_phy::modem::MODE_RETRY_CONFIDENCE
        || detect_confidence >= DETECT_CONFIDENCE_TRUSTED;
    (decoded || sure).then_some(cfo_hz)
}

/// What the physical layer made of one frame: for a display, for the list of stations
/// heard, and for a recording's sidecar.
///
/// The callsigns are read off the frame itself where it carries them — a beacon, a
/// connect request or its answer — and otherwise attributed to the other end of the
/// session when the frame belongs to it (same session id). A frame from nobody this
/// station can name has neither.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameReport {
    /// Station time, seconds.
    pub t_s: f64,
    /// `data`, `control`, `beacon`, `connect` (a request), `answer`, `probe` or
    /// `probe-answer`.
    pub kind: &'static str,
    /// Mode index.
    pub mode: usize,
    /// Redundancy version.
    pub rv: u8,
    /// SNR, dB, 3 kHz reference.
    pub snr_db: f64,
    /// Carrier offset removed, hertz — positive means the other station is high.
    pub cfo_hz: f64,
    /// How sure the receiver was of the mode chips (below about 1.3 is a guess). Only a
    /// DATA frame carries chips, so a CONTROL frame always reports 1.0 here — read
    /// `detect_confidence` for those.
    pub confidence: f64,
    /// How far above its acceptance threshold acquisition saw the preamble: 1.0 is exactly
    /// at the threshold. Defined for every frame type, so this is the one number that
    /// separates a real control frame from a noise trigger.
    pub detect_confidence: f64,
    /// Whether the payload decoded.
    pub decoded: bool,
    /// Payload bytes, when it did.
    pub bytes: usize,
    /// Who sent it, when the frame says or the session implies.
    pub from: Option<String>,
    /// Who it was addressed to, when the frame says.
    pub to: Option<String>,
    /// A control frame's fields, spelled out.
    pub control: Option<String>,
}

/// A session's account, kept from the moment it comes up to the moment it ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkAccount {
    /// Station time when the session came up.
    pub started_s: f64,
    /// Application bytes handed to the link this session, before compression.
    pub bytes_sent: usize,
    /// Application bytes delivered this session, after decompression.
    pub bytes_received: usize,
}

/// The dial frequency, asked of the radio now and then rather than on every frame.
#[derive(Debug, Clone, Copy, Default)]
struct FrequencyCache {
    value: Option<u64>,
    next_ask_s: f64,
}

/// How far back the throughput reading looks, seconds.
const THROUGHPUT_WINDOW_S: f64 = 30.0;

/// The most constellation points kept from a frame, for the display.
const CONSTELLATION_POINTS: usize = 1024;

/// The most frame reports held between two calls of `take_frame_reports`.
const MAX_UNTAKEN_REPORTS: usize = 1024;

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
    /// Morse identifiers sent.
    pub cw_ids: usize,
    /// Beacons transmitted.
    pub beacons_sent: usize,
    /// Beacons heard from other stations.
    pub beacons_heard: usize,
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
    /// Audio waiting for the sound card, at unit level; the transmit level is applied as
    /// it leaves, which is what lets the level change under a tone that is playing.
    playback: VecDeque<f32>,
    baseband_seen: usize,
    audio_seen: usize,
    transmitting: bool,
    /// Whether what is playing is a tune tone, which `tune_stop` may cut short — and
    /// nothing else may: a burst cut short is a session broken.
    playing_test: bool,
    /// Largest sample magnitude handed to the sound card during the transmission now
    /// running, after the transmit level is applied — what the rig's ALC is actually being
    /// shown.
    tx_peak_running: f32,
    /// The same for the last transmission that finished, which is the one worth reporting:
    /// a peak is only meaningful over a whole burst.
    tx_peak_last: Option<f32>,
    /// The audio clock when a held-back burst was last reported to the engine, while one
    /// is held.
    held_since: Option<f64>,
    /// Until when the busy detector ignores what the sound card delivers: the card's
    /// capture runs a lead behind its playback, so the end of this station's own burst
    /// arrives after the key is released, and read as channel it would hold the next burst
    /// back for its own echo.
    deaf_until: f64,
    pending: VecDeque<Outgoing>,
    meter: LevelMeter,
    /// The session recording in progress, if one is.
    recording: Option<crate::record::Recording>,
    /// Notes the operator gave for the next automatic recording, if any.
    record_notes: Option<String>,
    /// The Test session running, or the last one run (P6-7).
    test: Option<TestRun>,
    /// Callsigns given while a session was up, to take effect when it ends.
    pending_callsigns: Option<Vec<String>>,
    delivered: Vec<u8>,
    events: Vec<String>,
    /// Frames the physical layer reported since a client last took them.
    reports: Vec<FrameReport>,
    /// The last frame, and its equalised constellation, for the diagnostics display.
    last_frame: Option<FrameReport>,
    last_symbols: Vec<Complex>,
    /// What the sound card is delivering, for the spectrum display.
    spectrum: SpectrumAnalyser,
    /// Until when a burst is known to be arriving: a preamble was found and its frame
    /// has not finished, or a frame just finished and the next may follow.
    rx_until: f64,
    /// The session's account, while one is up.
    link: Option<LinkAccount>,
    /// Bytes that crossed the air, with when, for the throughput reading.
    moved: VecDeque<(f64, usize)>,
    /// The engine's acknowledged-plus-delivered count when `moved` was last fed.
    crossed: usize,
    frequency: FrequencyCache,
    compressor: Compressor,
    decompressor: Decompressor,
    /// Application bytes waiting for a session to negotiate compression.
    outbound: Vec<u8>,
    /// When the last Morse identifier went out, in station time.
    last_cw_id: Option<f64>,
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
    pub fn new(mut config: StationConfig, ptt: P, seed: u64) -> Self {
        // The key is released when this station's queue runs dry, and at that moment the
        // sound card still holds a playback lead's worth of what was queued last. Unless
        // the tail of silence is at least that long, what the card is still holding is
        // the end of the burst, and the radio drops out of transmit with the last fifth
        // of a second of every frame still to play — seen on a rig's meters as a spike as
        // the transmission ends, and on the air as a frame nobody can decode. The bench
        // never showed it: the simulated channel carries audio whether keyed or not.
        config.key_tail_s += config.playback_lead_s;
        let params = config.params;
        let mut timing = phy_timing(params);
        timing.tx_latency_s = config.key_lead_s + config.playback_lead_s + config.key_tail_s;
        let link = LinkConfig {
            // what this station offers, and the bandwidth it transmits in — stated, not
            // negotiated: a call claiming another bandwidth than it arrived in is ignored
            capabilities: with_bandwidth(
                offered_capabilities(config.compress),
                params.bandwidth.hz(),
            ),
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
            playing_test: false,
            tx_peak_running: 0.0,
            tx_peak_last: None,
            held_since: None,
            deaf_until: f64::NEG_INFINITY,
            pending: VecDeque::new(),
            meter: LevelMeter::new(params.audio_rate as f64, 3.0),
            recording: None,
            record_notes: None,
            test: None,
            pending_callsigns: None,
            delivered: Vec::new(),
            events: Vec::new(),
            reports: Vec::new(),
            last_frame: None,
            last_symbols: Vec::new(),
            spectrum: SpectrumAnalyser::new(params.audio_rate as f64),
            rx_until: f64::NEG_INFINITY,
            link: None,
            moved: VecDeque::new(),
            crossed: 0,
            frequency: FrequencyCache::default(),
            // Nothing is compressed until a session negotiates it. Before that the two ends
            // have not agreed on anything, and a guess would produce a stream the peer
            // cannot read.
            compressor: Compressor::new(false),
            decompressor: Decompressor::new(false),
            outbound: Vec::new(),
            last_cw_id: None,
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

    /// The waveform this station runs.
    #[must_use]
    pub fn params(&self) -> WaveformParams {
        self.config.params
    }

    /// Frames reported since the last call.
    pub fn take_frame_reports(&mut self) -> Vec<FrameReport> {
        std::mem::take(&mut self.reports)
    }

    /// The last frame the physical layer found, decoded or not.
    #[must_use]
    pub fn last_frame(&self) -> Option<&FrameReport> {
        self.last_frame.as_ref()
    }

    /// The last frame's equalised constellation, thinned to at most
    /// [`CONSTELLATION_POINTS`] points, with the frame it came from.
    #[must_use]
    pub fn constellation(&self) -> Option<(&FrameReport, &[Complex])> {
        self.last_frame
            .as_ref()
            .map(|frame| (frame, self.last_symbols.as_slice()))
    }

    /// A spectrum of the last window of captured audio, once one has been heard.
    #[must_use]
    pub fn spectrum(&self) -> Option<Spectrum> {
        self.spectrum.compute()
    }

    /// Whether a burst is arriving right now.
    #[must_use]
    pub fn receiving(&self) -> bool {
        !self.transmitting && self.now() < self.rx_until
    }

    /// The session's account, while one is up.
    #[must_use]
    pub fn link(&self) -> Option<LinkAccount> {
        self.link
    }

    /// Payload bytes that crossed the air in either direction — acknowledged by the
    /// other station, or received from it — over the last [`THROUGHPUT_WINDOW_S`] seconds
    /// of the session, as bits per second. Zero when no session is up.
    #[must_use]
    pub fn throughput_bps(&self) -> f64 {
        let Some(link) = self.link else { return 0.0 };
        let now = self.now();
        let since = now - THROUGHPUT_WINDOW_S;
        let bytes: usize = self
            .moved
            .iter()
            .filter(|(t, _)| *t >= since)
            .map(|(_, n)| n)
            .sum();
        let span = (now - link.started_s).clamp(1.0, THROUGHPUT_WINDOW_S);
        bytes as f64 * 8.0 / span
    }

    /// The dial frequency, when the keying interface can ask the radio.
    ///
    /// Asked at most every ten seconds — every minute after the radio did not answer —
    /// and never while transmitting, so a display that polls does not turn into a
    /// stream of CAT traffic; a station keyed by a serial line has no answer at all.
    pub fn frequency_hz(&mut self) -> Option<u64> {
        let now = self.now();
        if self.transmitting || now < self.frequency.next_ask_s {
            return self.frequency.value;
        }
        let answer = self.ptt.inner_mut().frequency_hz();
        self.frequency.next_ask_s = now + if answer.is_some() { 10.0 } else { 60.0 };
        if answer.is_some() {
            self.frequency.value = answer;
        }
        self.frequency.value
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

    /// Why this station cannot transmit, when its keying interface could not be opened.
    #[must_use]
    pub fn ptt_fault(&self) -> Option<String> {
        self.ptt.fault()
    }

    /// Whether the keying interface can tune the radio (CAT or `rigctld`).
    #[must_use]
    pub fn can_tune(&self) -> bool {
        self.ptt.can_tune()
    }

    // ── commands ──────────────────────────────────────────────────────

    /// Call a station, as this station's first callsign.
    ///
    /// # Errors
    /// If a session is already up.
    pub fn connect(&mut self, remote: &str) -> Result<(), &'static str> {
        self.connect_as(remote, None)
    }

    /// Call a station as one of this station's callsigns.
    ///
    /// # Errors
    /// If a session is already up, or the callsign is not one of this station's.
    pub fn connect_as(&mut self, remote: &str, as_call: Option<&str>) -> Result<(), &'static str> {
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and makes none");
        }
        self.engine.connect_as(remote, as_call)?;
        self.pump();
        Ok(())
    }

    /// Change the callsigns this station answers to; the first is the one it calls as.
    ///
    /// The configuration file names the station, and that is what it answers to until a
    /// host program says otherwise: the operator's callsign lives in the host in practice
    /// (Winlink Express, Pat and `VarAC` each send `MYCALL`, and none of them can be told the
    /// modem has a callsign of its own). Given while a session is up, the change waits for
    /// the session to end — the callsign is in every frame's addressing — and the result says
    /// so.
    ///
    /// # Errors
    /// If the list is empty or a callsign is one the air interface cannot carry.
    pub fn set_callsigns(&mut self, calls: &[String]) -> Result<bool, &'static str> {
        if self.engine.state() == State::Idle {
            self.engine.set_callsigns(calls)?;
            self.pending_callsigns = None;
            return Ok(true);
        }
        // validated now, so a host is not told OK for a list the engine will refuse later
        let mut probe = LinkEngine::new(
            &self.engine.my_call,
            self.engine.timing().clone(),
            self.config.link.clone(),
            0,
        );
        probe.set_callsigns(calls)?;
        self.pending_callsigns = Some(probe.callsigns);
        Ok(false)
    }

    /// The callsign this station runs under right now, and every one it answers to.
    #[must_use]
    pub fn callsigns(&self) -> &[String] {
        &self.engine.callsigns
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
        if let Some(link) = &mut self.link {
            link.bytes_sent += pending.len();
        }
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

    /// Stop transmitting and release the radio, now.
    ///
    /// For shutdown. A gateway is stopped by a service manager sending a signal, and a
    /// station whose process is killed mid-burst leaves the transmitter keyed until somebody
    /// notices — which on an unattended station could be a very long time. Whatever else
    /// fails on the way out, the key has to come up.
    ///
    /// # Errors
    /// If the radio refuses to release. The local state is cleared either way.
    pub fn shut_down(&mut self) -> Result<(), PttError> {
        self.playback.clear();
        self.pending.clear();
        self.transmitting = false;
        // a recording left open would have a header from its last patch; closing it here
        // is cheap and the sidecar gets the counters
        self.stop_recording();
        self.ptt.unkey(self.now())
    }

    /// As the receiving station, demand the sending role.
    pub fn request_break(&mut self) {
        self.engine.request_break();
    }

    /// Apply the settings that can change without a restart.
    ///
    /// A sound card is opened once and a control socket is bound once, so most of a
    /// configuration cannot be changed under a running station. These few can, and applying
    /// them here is what makes `config.set` mean something before the next restart rather
    /// than after it.
    pub fn apply_live(&mut self, config: &crate::config::Config) {
        // applied as audio leaves, so it takes effect on a tone already playing
        self.config.tx_level = config.audio.tx_level;
        self.config.max_key_s = config.radio.max_key_s;
        self.config.wait_for_clear = config.radio.wait_for_clear;
        self.config.link.max_mode = config.radio.fastest_mode();
        self.config.answer_only = config.radio.answer_only;
        self.ptt.max_key_s = config.radio.max_key_s;
        self.busy.set_threshold_db(config.radio.busy_threshold_db);
        self.config.record_auto = config.record.auto;
        self.config.record_notes.clone_from(&config.record.notes);
        self.config.operator.clone_from(&config.operator);
    }

    /// Tune the radio, when the keying interface can ask it to. Refused during a session
    /// or while transmitting: the other station is on the frequency this one would leave.
    ///
    /// # Errors
    /// With the reason, in a sentence for the operator.
    pub fn tune_to(&mut self, hz: u64) -> Result<(), String> {
        if self.transmitting {
            return Err("the transmitter is keyed".into());
        }
        if self.engine.state() != State::Idle {
            return Err("a session is running".into());
        }
        self.ptt
            .inner_mut()
            .set_frequency_hz(hz)
            .map_err(|e| match e {
                crate::ptt::PttError::Backend(message) => message,
                other @ crate::ptt::PttError::WatchdogTripped => other.to_string(),
            })?;
        // what the radio now reads, and a fresh reading soon after
        self.frequency.value = Some(hz);
        self.frequency.next_ask_s = self.now() + 2.0;
        self.note("tune", &format!("dial set to {hz} Hz"));
        Ok(())
    }

    /// Transmit one unproto beacon: this station's callsign, addressed to nobody.
    ///
    /// It is how an operator answers "can anybody hear me?" without arranging a contact
    /// first. Sent at the most robust mode, because the point is to be heard by somebody who
    /// cannot yet hear anything else — and never while a session is running, which would put
    /// a frame into the middle of somebody's transfer.
    ///
    /// # Errors
    /// If a session is up, or the callsign cannot be packed.
    pub fn beacon(&mut self) -> Result<(), &'static str> {
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and sends no beacon");
        }
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        let body = pack_callsign(&self.engine.my_call).map_err(|_| "the callsign will not pack")?;
        let header = DataHeader {
            kind: DataKind::Beacon,
            seq: 0,
            session: 0,
        };
        // beacons go out at the control mode: the slowest ordinary mode whose frame carries
        // a callsign (mode 0 is a floor mode on the narrow air, ADR-0009)
        let mode = self.transmitter.air().control_mode().index;
        let capacity = self.engine.timing().capacity(mode);
        let payload =
            encode_data(&header, &body, capacity).map_err(|_| "the beacon will not fit")?;
        self.pending
            .push_back(Outgoing::Frames(vec![aether_link::TxFrame {
                container: Container::Data,
                payload,
                mode,
                rv: 0,
                floor: false,
            }]));
        self.stats.beacons_sent += 1;
        Ok(())
    }

    /// Ask a station whether it hears this one, and how well, without a session
    /// (ADR-0006): one probe frame, one answer carrying the SNR the probe arrived at,
    /// and a `probe` event with both directions of the path — or `no answer`.
    ///
    /// Sending a probe is a call, so an answer-only station may not; answering one is a
    /// response, which it may, and does on its own.
    ///
    /// # Errors
    /// If the station is answer-only, a session is up, a probe is already out, or the
    /// callsign is not one of this station's.
    pub fn probe(&mut self, remote: &str, as_call: Option<&str>) -> Result<(), &'static str> {
        if self.config.answer_only {
            return Err("this station is answer-only: it answers probes and sends none");
        }
        self.engine.probe(remote, as_call)?;
        self.pump();
        Ok(())
    }

    /// Key the transmitter with no audio for a few seconds, so an operator can see the rig
    /// go into transmit and the interface's PTT light come on.
    ///
    /// Whether the *radio* keyed is something only the operator can see, and the point of
    /// the test is to let them look — so it keys now, not when the channel next clears: an
    /// SSB transmitter keyed with no audio radiates nothing, and an operator watching a PTT
    /// light cannot be told "accepted" and then kept waiting. Bounded, and refused during a
    /// session.
    ///
    /// # Errors
    /// If a session is running, or the duration is outside what is sensible for a test.
    pub fn key_test(&mut self, seconds: f64) -> Result<(), &'static str> {
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        if !(0.2..=5.0).contains(&seconds) {
            return Err("a keying test lasts between 0.2 and 5 seconds");
        }
        let samples = (seconds * self.config.params.audio_rate as f64) as usize;
        self.pending.push_back(Outgoing::Audio {
            samples: vec![0.0; samples],
            silent: true,
            tone: false,
        });
        Ok(())
    }

    /// Key the transmitter and play a steady tone at the configured level, so an operator
    /// can set their drive by watching the rig's ALC.
    ///
    /// This is what a radio's own "tune" button does with the difference that the audio is
    /// this modem's, at this modem's level, through this modem's sound card — which is
    /// exactly the path that has to be right. Bounded at ten seconds because a carrier is a
    /// carrier, and refused during a session or while the channel is busy — refused rather
    /// than deferred, because an operator with a hand on the drive control is waiting for
    /// it now, and a tone that starts on its own a minute later would surprise them.
    ///
    /// # Errors
    /// If a session is running, the channel is not known to be clear, or the duration is
    /// outside what is sensible.
    pub fn tune(&mut self, seconds: f64) -> Result<(), &'static str> {
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        if !(0.5..=10.0).contains(&seconds) {
            return Err("a tune tone lasts between 0.5 and 10 seconds");
        }
        if self.config.wait_for_clear && !self.busy.settled() {
            return Err(
                "the busy detector is still learning the noise floor; try again in a few seconds",
            );
        }
        if self.config.wait_for_clear && !self.channel_clear(self.now()) {
            return Err("the channel is busy");
        }
        let rate = self.config.params.audio_rate as f64;
        let tone = crate::cwid::CwId {
            // a steady tone is one very long dah; the shaped edges keep it from clicking
            wpm: 1.2 / (seconds / 3.0),
            tone_hz: 1500.0,
            // full scale here: the transmit level is applied as the audio leaves
            level: 1.0,
        };
        self.pending.push_back(Outgoing::Audio {
            samples: tone.audio("T", rate),
            silent: false,
            tone: true,
        });
        Ok(())
    }

    /// Send a few real bursts so the operator can set drive against the waveform the
    /// station actually transmits.
    ///
    /// A tune tone is a sine, and the daemon scales the data waveform to the *tone's RMS* —
    /// but the waveform is OFDM, so its peaks land far above anything the tone reaches.
    /// Measured at the sound card, which is where it matters: **+5.8 dB** at the floor
    /// control mode and **+7.0 dB** at the fastest, against a tone whose peak is exactly
    /// the transmit level. (ADR-0004's 5.9/7.6 dB are baseband PAPR, before the passband
    /// conversion regrows the peaks; these numbers are the end-to-end ones.)
    ///
    /// An ALC responds to peaks, so drive set by ear on the tone is six or seven decibels
    /// into limiting on real traffic, which smears the constellation and costs far more
    /// than it saves (OTA-2, finding 1: a 26 dB channel demodulating at 7 dB).
    ///
    /// So the two jobs are separated. [`tune`](Self::tune) stays a steady carrier, which is
    /// what an antenna tuner needs; this sends the real thing, at the fastest mode the
    /// station is allowed to use, because that is the worst case its ALC will ever see.
    /// Set the drive here and every slower mode has margin in hand.
    ///
    /// The bursts carry filler, not protocol: a station that decodes one finds a data frame
    /// for a session it does not have and ignores it, as it would any stray frame.
    ///
    /// # Errors
    /// If a session is running, the channel is not known to be clear, the count is outside
    /// what is sensible, or this build has no waveform for the configured bandwidth.
    pub fn set_drive(&mut self, bursts: usize) -> Result<(), &'static str> {
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        if !(1..=10).contains(&bursts) {
            return Err("a drive check is between 1 and 10 bursts");
        }
        if self.config.wait_for_clear && !self.busy.settled() {
            return Err(
                "the busy detector is still learning the noise floor; try again in a few seconds",
            );
        }
        if self.config.wait_for_clear && !self.channel_clear(self.now()) {
            return Err("the channel is busy");
        }
        let air = self.air();
        let index = self.config.link.max_mode.min(air.n_modes() - 1);
        let mode = air.modes[index];
        let bytes = mode.payload_bytes(&air.data_layout(index));
        if bytes == 0 {
            return Err("this mode carries no payload to send");
        }
        // not all-zero: the codec refuses that block (ADR-0009)
        let payload = vec![0x5a_u8; bytes];
        for _ in 0..bursts {
            self.pending
                .push_back(Outgoing::Drive(vec![aether_link::TxFrame {
                    container: aether_link::Container::Data,
                    payload: payload.clone(),
                    mode: index,
                    rv: 0,
                    // a data frame's family follows its mode; this flag is for control frames
                    floor: false,
                }]));
        }
        Ok(())
    }

    /// Cut a tune tone or a drive burst short, playing or still queued.
    ///
    /// An operator setting drive has a hand on the control and the other on this: the tone
    /// is bounded either way, but ten seconds of carrier after the ALC is where it should
    /// be is ten seconds too many. Only an operator's own test transmission is ever cut —
    /// a session's burst stopped halfway is a session broken. Returns whether there was
    /// something to stop.
    pub fn tune_stop(&mut self) -> bool {
        let queued = self.pending.len();
        self.pending.retain(|next| {
            !matches!(
                next,
                Outgoing::Audio { tone: true, .. } | Outgoing::Drive(_)
            )
        });
        let mut stopped = self.pending.len() != queued;
        if self.playing_test {
            // the keying tail goes with it; the transmitter unkeys as soon as the queue
            // is empty, which is the point
            self.playback.clear();
            self.playing_test = false;
            stopped = true;
        }
        stopped
    }

    /// The peak of the last transmission, in dBFS, and whether it reached full scale.
    ///
    /// `None` until this station has transmitted. 0 dBFS means the audio itself clipped
    /// before the radio ever saw it; well below is what a rig's ALC wants, because the
    /// waveform's peaks are what drive it and they sit about 6-7 dB above a tone of the
    /// same average power (measured; see [`set_drive`](Self::set_drive)).
    #[must_use]
    pub fn tx_peak_dbfs(&self) -> Option<f64> {
        self.tx_peak_last.map(|peak| {
            if peak <= 0.0 {
                f64::NEG_INFINITY
            } else {
                20.0 * f64::from(peak).log10()
            }
        })
    }

    /// The last few seconds of received audio, as levels an operator can set a sound card by.
    #[must_use]
    pub fn audio_level(&self) -> LevelReading {
        self.meter.reading()
    }

    /// Start recording the channel and what the modem makes of it.
    ///
    /// `name` overrides the time-and-callsigns name; `notes` is whatever the operator wants
    /// the sidecar to say about the band, the frequency, the other station.
    ///
    /// # Errors
    /// If recording is not configured, one is already running, or the file cannot be made.
    pub fn start_recording(
        &mut self,
        name: Option<&str>,
        notes: Option<&str>,
    ) -> Result<std::path::PathBuf, String> {
        let Some(dir) = self.config.record_dir.clone() else {
            return Err("no recording directory is configured ([record] dir)".into());
        };
        if self.recording.is_some() {
            return Err("a recording is already running".into());
        }
        let remote = self.engine.remote_call.clone();
        let remote = (!remote.is_empty()).then_some(remote);
        let name = name.map_or_else(
            || crate::record::session_name(&self.engine.my_call, remote.as_deref()),
            str::to_owned,
        );
        // the rig's frequency, when the keying backend can ask for it: the one fact about a
        // recording nobody can reconstruct afterwards
        let frequency_hz = self.ptt.inner_mut().frequency_hz();
        let meta = serde_json::json!({
            "callsign": self.engine.my_call,
            "remote": remote,
            "notes": notes,
            "frequency_hz": frequency_hz,
            "bandwidth_hz": self.config.params.bandwidth.hz(),
            "max_mode": self.config.link.max_mode,
            "compress": self.config.compress,
            "tx_level": self.config.tx_level,
            "wait_for_clear": self.config.wait_for_clear,
            "state": format!("{:?}", self.engine.state()),
            "operator": {
                "grid": self.config.operator.grid,
                "rig": self.config.operator.rig,
                "power_w": self.config.operator.power_w,
                "antenna": self.config.operator.antenna,
            },
        });
        let recording = crate::record::Recording::start(
            &dir,
            &name,
            u32::try_from(self.config.params.audio_rate).unwrap_or(48_000),
            self.now(),
            meta,
        )
        .map_err(|e| format!("cannot write to {}: {e}", dir.display()))?;
        let path = recording.path().to_path_buf();
        self.recording = Some(recording);
        self.note("recording", &format!("started {}", path.display()));
        Ok(path)
    }

    /// Stop the recording, if one is running, and say what it holds.
    pub fn stop_recording(&mut self) -> Option<crate::record::Summary> {
        let recording = self.recording.take()?;
        let counters = crate::control::methods::counters(self);
        match recording.finish(&counters) {
            Ok(summary) => {
                self.note(
                    "recording",
                    &format!(
                        "stopped {} ({:.0} s, {} frames, {} decoded)",
                        summary.wav.display(),
                        summary.seconds,
                        summary.frames,
                        summary.decoded
                    ),
                );
                Some(summary)
            }
            Err(error) => {
                self.note("error", &format!("could not finish the recording: {error}"));
                None
            }
        }
    }

    /// Where recordings are written, whether or not one is running now.
    #[must_use]
    pub fn record_dir(&self) -> Option<&std::path::Path> {
        self.config.record_dir.as_deref()
    }

    /// Zero the counters the panel shows, on both this station and its engine. Only the
    /// display tallies are cleared; the session, the link account and the configuration
    /// are untouched.
    pub fn reset_counters(&mut self) {
        self.engine.reset_stats();
        let StationStats {
            bytes_before_compression,
            bytes_after_compression,
            ..
        } = self.stats;
        // the two compression figures are recomputed from the live compressor each
        // metrics tick, so they are carried rather than zeroed to a value that would
        // only be overwritten a moment later
        self.stats = StationStats {
            bytes_before_compression,
            bytes_after_compression,
            ..StationStats::default()
        };
    }

    /// The recording in progress: its file and how long it is.
    #[must_use]
    pub fn recording(&self) -> Option<(std::path::PathBuf, f64)> {
        self.recording
            .as_ref()
            .map(|r| (r.path().to_path_buf(), r.seconds()))
    }

    /// Notes for the next recording that starts on its own, when `record_auto` is on.
    pub fn set_record_notes(&mut self, notes: Option<String>) {
        self.record_notes = notes;
    }

    /// Where recordings go, whether sessions record themselves, and what every such
    /// recording says about the station.
    pub fn set_recording(&mut self, dir: Option<std::path::PathBuf>, auto: bool, notes: &str) {
        self.config.record_dir = dir;
        self.config.record_auto = auto;
        notes.clone_into(&mut self.config.record_notes);
    }

    /// Report something: to whoever drains the events, and to the recording.
    fn note(&mut self, event: &str, detail: &str) {
        self.events.push(format!("{event}:{detail}"));
        if let Some(recording) = &mut self.recording {
            recording.event(
                self.audio_seen as f64 / self.config.params.audio_rate as f64,
                event,
                detail,
                &format!("{:?}", self.engine.state()),
            );
        }
    }

    // ── audio ─────────────────────────────────────────────────────────

    /// Hand over audio the sound card captured.
    ///
    /// # Errors
    /// If the key had to be released and the radio refused.
    pub fn capture(&mut self, audio: &[f32]) -> Result<(), PttError> {
        self.audio_seen += audio.len();
        self.meter.push(audio);
        self.spectrum.push(audio);
        if let Some(recording) = &mut self.recording
            && let Err(error) = recording.captured(audio)
        {
            // a full disk must not stop the modem; the recording is what is lost
            self.note("error", &format!("recording stopped: {error}"));
            self.recording = None;
        }
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
            // the receiver hears everything; the busy detector is spared the tail of this
            // station's own burst, which the card delivers a lead after the key is released
            if now < self.deaf_until {
                self.busy.skip(&baseband);
            } else {
                self.busy.push(&baseband, now);
            }
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
            self.note("watchdog", "key time exceeded");
        }
        self.pump();
        self.advance_test();
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
            self.playing_test = false;
            self.tx_peak_last = Some(self.tx_peak_running);
            let peak = self.tx_peak_running;
            self.tx_peak_running = 0.0;
            if let Some(recording) = &mut self.recording {
                let state = format!("{:?}", self.engine.state());
                // `detail` on a ptt event is read back by the replay to find the intervals
                // this station was deaf for (`replay.rs`), so it stays exactly "keyed" and
                // "released" and nothing else.
                recording.event(now, "ptt", "released", &state);
                // The peak goes in an event of its own: a burst nobody decoded is a
                // different story depending on whether the transmitter was being clipped
                // at the time, and by the time anyone asks, the audio is all that is left.
                if peak > 0.0 {
                    let dbfs = 20.0 * f64::from(peak).log10();
                    recording.event(now, "tx_peak", &format!("{dbfs:.1} dBFS"), &state);
                }
            }
            self.ptt.unkey(now)?;
            // the queue drained now, and what the sound card still holds is the tail's
            // silence: the burst itself has already left
            self.engine.on_tx_done(now);
            self.deaf_until = now + self.config.playback_lead_s + CAPTURE_LAG_ALLOWANCE_S;
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
            if let Some(recording) = &mut self.recording {
                recording.event(now, "ptt", "keyed", &format!("{:?}", self.engine.state()));
            }
        }
        // the level is applied here, on the way out, so a change reaches a tone that is
        // already playing — an operator adjusts drive by ear and by the ALC, live
        let level = self.config.tx_level as f32;
        let count = out.len().min(self.playback.len());
        for slot in out.iter_mut().take(count) {
            *slot = self.playback.pop_front().unwrap_or(0.0) * level;
            // the ALC sees peaks, so this is the number that decides whether the rig is
            // being over-driven — measured after the level, which is where it is applied
            self.tx_peak_running = self.tx_peak_running.max(slot.abs());
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
            self.report(&decoded, now);
            let detected = decoded.frame.sync.detect_confidence(&self.air());
            if let Some(recording) = &mut self.recording {
                recording.frame(crate::record::FrameRecord {
                    t_s: now,
                    kind: if decoded.frame.sync.frame_type == FrameType::Control {
                        "control".into()
                    } else {
                        "data".into()
                    },
                    mode: decoded.frame.mode,
                    rv: decoded.frame.rv,
                    snr_3k_db: decoded.frame.snr_3k_db,
                    cfo_hz: reported_cfo(
                        decoded.ok(),
                        decoded.frame.mode_confidence,
                        detected,
                        decoded.frame.cfo_hz,
                    ),
                    confidence: decoded.frame.mode_confidence,
                    detect_confidence: detected,
                    decoded: decoded.ok(),
                    bytes: decoded.payload.as_ref().map_or(0, Vec::len),
                    control: if decoded.frame.sync.frame_type == FrameType::Control {
                        decoded
                            .payload
                            .as_deref()
                            .and_then(crate::record::describe_control)
                    } else {
                        None
                    },
                });
            }
            // A beacon belongs to no session, so it is handled before anything the engine
            // would do with it. It is reported and never answered: a channel where every
            // beacon drew a reply would be unusable.
            if let Some(caller) = beacon_callsign(&decoded) {
                self.stats.beacons_heard += 1;
                self.busy.mark_frame(now);
                self.note(
                    "beacon",
                    &format!("{caller} at {:.1} dB", decoded.frame.snr_3k_db),
                );
                continue;
            }
            let container = if decoded.frame.sync.frame_type == FrameType::Control {
                Container::Control
            } else {
                Container::Data
            };
            let layout = self
                .transmitter
                .air()
                .layout_for_family(container == Container::Data, decoded.frame.sync.floor);
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
            self.rx_until = self
                .rx_until
                .max(now + self.transmitter.air().long.duration_s());
            self.engine.on_preamble(pending.sync.start as f64 / fs, now);
        }
        self.pump();
    }

    /// The air interface this station runs, for the numbers that only mean something
    /// against it — an acquisition threshold, a layout, a mode table.
    fn air(&self) -> aether_phy::modes::AirInterface {
        aether_phy::modes::air_interface(self.config.params)
    }

    /// Describe a frame for the displays, and keep its constellation.
    fn report(&mut self, decoded: &aether_phy::DecodedFrame, now: f64) {
        let frame = &decoded.frame;
        let control = frame.sync.frame_type == FrameType::Control;
        let payload = decoded.payload.as_deref();
        let mut report = FrameReport {
            t_s: now,
            kind: if control { "control" } else { "data" },
            mode: frame.mode,
            rv: frame.rv,
            snr_db: frame.snr_3k_db,
            cfo_hz: frame.cfo_hz,
            confidence: frame.mode_confidence,
            detect_confidence: frame.sync.detect_confidence(&self.air()),
            decoded: decoded.ok(),
            bytes: payload.map_or(0, <[u8]>::len),
            from: None,
            to: None,
            control: None,
        };
        // a frame that belongs to the session is the other station's; one with another
        // session id is somebody else's business and stays unattributed
        let ours = |session: u8| {
            (self.engine.connected() && session == self.engine.session())
                .then(|| self.engine.remote_call.clone())
        };
        if control {
            if let Some(payload) = payload {
                report.control = crate::record::describe_control(payload);
                if let Ok(control) = aether_link::frames::ControlFrame::decode(payload) {
                    report.from = ours(control.session);
                }
            }
        } else if let Some((header, body)) = payload.and_then(|p| decode_data(p).ok()) {
            match header.kind {
                DataKind::Beacon => {
                    report.kind = "beacon";
                    report.from = unpack_callsign(&body).ok();
                }
                DataKind::ConnectReq | DataKind::ConnectAck => {
                    report.kind = if header.kind == DataKind::ConnectReq {
                        "connect"
                    } else {
                        "answer"
                    };
                    if let Ok(connect) = ConnectBody::decode(&body) {
                        report.from = Some(connect.src);
                        report.to = Some(connect.dst);
                    }
                }
                DataKind::Probe | DataKind::ProbeAck => {
                    report.kind = if header.kind == DataKind::Probe {
                        "probe"
                    } else {
                        "probe-answer"
                    };
                    if let Ok(probe) = ProbeBody::decode(&body) {
                        report.from = Some(probe.src);
                        report.to = Some(probe.dst);
                    }
                }
                DataKind::Data => report.from = ours(header.session),
            }
        }
        // the constellation, thinned evenly so a long frame costs a display no more
        // than a short one
        let step = frame.symbols.len().div_ceil(CONSTELLATION_POINTS).max(1);
        self.last_symbols = frame.symbols.iter().step_by(step).copied().collect();
        self.last_frame = Some(report.clone());
        // bounded, for a station nobody drains: a test harness, or a client that never asks
        if self.reports.len() >= MAX_UNTAKEN_REPORTS {
            self.reports.remove(0);
        }
        self.reports.push(report);
        // the burst may go on: the next frame's preamble is a symbol or two away
        self.rx_until = self.rx_until.max(now + 0.5);
    }

    /// Note what crossed the air since last time, for the throughput reading: payload
    /// bytes the other station acknowledged, and payload bytes received from it — the
    /// engine's own counters, so a burst that was sent but never acknowledged counts for
    /// nothing, exactly as it should.
    fn account(&mut self) {
        let stats = self.engine.stats;
        let crossed = stats.bytes_acked + stats.bytes_delivered;
        let bytes = crossed.saturating_sub(self.crossed);
        self.crossed = crossed;
        if bytes == 0 || self.link.is_none() {
            return;
        }
        let now = self.now();
        self.moved.push_back((now, bytes));
        while self
            .moved
            .front()
            .is_some_and(|(t, _)| *t < now - THROUGHPUT_WINDOW_S)
        {
            self.moved.pop_front();
        }
    }

    /// Take what the engine has decided and act on it.
    fn pump(&mut self) {
        let mut connected = false;
        for action in self.engine.drain() {
            match action {
                Action::Transmit { frames, .. } => {
                    self.pending.push_back(Outgoing::Frames(frames));
                }
                Action::Deliver(bytes) => {
                    let plain = self.decompressor.push(&bytes);
                    if self.decompressor.failed() {
                        self.note(
                            "error",
                            "the peer sent a compressed stream this station cannot read",
                        );
                    }
                    if let Some(link) = &mut self.link {
                        link.bytes_received += plain.len();
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
                        self.link = Some(LinkAccount {
                            started_s: self.now(),
                            bytes_sent: 0,
                            bytes_received: 0,
                        });
                        self.moved.clear();
                        connected = true;
                    } else if name == "disconnected" {
                        self.link = None;
                        self.moved.clear();
                        self.compressor = Compressor::new(false);
                        self.decompressor = Decompressor::new(false);
                        if let Some(calls) = self.pending_callsigns.take() {
                            // validated when they were given; the engine is idle now
                            let _ = self.engine.set_callsigns(&calls);
                        }
                    }
                    self.note(name, &detail);
                    // a session is the unit of a field recording: one file per session,
                    // started when it comes up and closed when it ends — unless a Test
                    // session is recording itself, probe to disconnect
                    if self.config.record_auto && !self.test_running() {
                        if name == "connected" {
                            // a note given for the next recording wins; otherwise the
                            // standing one, which is what an unattended station has
                            let notes = self.record_notes.take().or_else(|| {
                                (!self.config.record_notes.is_empty())
                                    .then(|| self.config.record_notes.clone())
                            });
                            if let Err(error) = self.start_recording(None, notes.as_deref()) {
                                self.note("error", &format!("could not record: {error}"));
                            }
                        } else if name == "disconnected" {
                            self.stop_recording();
                        }
                    }
                }
            }
        }
        if connected {
            self.flush_outbound();
        }
        self.account();
    }

    /// Render the next queued burst into playable audio, if the channel allows.
    fn start_pending(&mut self, now: f64) {
        let Some(next) = self.pending.front() else {
            return;
        };
        // silence radiates nothing, so a keying test does not wait for the channel
        let radiates = !matches!(next, Outgoing::Audio { silent: true, .. });
        if radiates && self.config.wait_for_clear && !self.channel_clear(now) {
            self.stats.deferred_for_busy += 1;
            // the engine's timers move with the burst, or a retry fires against a burst
            // that has not left yet and the two go out back to back when the channel clears
            if let Some(since) = self.held_since {
                self.engine.on_tx_delayed(now - since);
            }
            self.held_since = Some(now);
            return;
        }
        self.held_since = None;
        let Some(outgoing) = self.pending.pop_front() else {
            return;
        };
        let cuttable = matches!(outgoing, Outgoing::Drive(_));
        let frames = match outgoing {
            Outgoing::Frames(frames) | Outgoing::Drive(frames) => frames,
            // raw audio goes out as it is, inside the same keying and lead and tail as a
            // burst, so a keying test exercises exactly the path a transmission uses
            Outgoing::Audio { samples, tone, .. } => {
                let audio_rate = self.config.params.audio_rate as f64;
                let lead = (self.config.key_lead_s * audio_rate) as usize;
                let tail = (self.config.key_tail_s * audio_rate) as usize;
                self.playback.extend(std::iter::repeat_n(0.0f32, lead));
                self.playback.extend(samples);
                self.playback.extend(std::iter::repeat_n(0.0f32, tail));
                self.playing_test = tone;
                return;
            }
        };

        self.record_sent(&frames, now);

        let mut baseband: Vec<Complex> = Vec::new();
        for frame in &frames {
            let burst = match frame.container {
                Container::Data => self.transmitter.data_burst(
                    &frame.payload,
                    self.transmitter.modes()[frame.mode],
                    frame.rv,
                ),
                Container::Control => {
                    self.transmitter
                        .control_burst_of(&frame.payload, frame.rv, frame.floor)
                }
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

        // at unit level: the transmit level is applied as the audio leaves
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.playback.extend(rendered);
        self.append_cw_id(now, audio_rate);
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.playing_test = cuttable;
    }

    /// Record what this station is putting on the air, beside what it hears.
    ///
    /// Without it a sidecar cannot answer the first question a failed burst raises — what
    /// mode was that? — and even with both stations' recordings in hand, half of the link
    /// stays invisible: a burst that decoded nowhere looks exactly like a burst that was
    /// never sent (`field/OTA-2-FINDINGS.md`).
    fn record_sent(&mut self, frames: &[aether_link::TxFrame], now: f64) {
        let Some(recording) = &mut self.recording else {
            return;
        };
        for frame in frames {
            recording.sent(crate::record::SentRecord {
                t_s: now,
                kind: match frame.container {
                    Container::Data => "data".to_owned(),
                    Container::Control => "control".to_owned(),
                },
                mode: frame.mode,
                rv: frame.rv,
                floor: frame.floor,
                bytes: frame.payload.len(),
            });
        }
    }

    /// Put a Morse identifier at the end of this transmission, if one is due.
    ///
    /// It goes inside the same keying, after the data and before the tail, which is what an
    /// identifier is: part of the transmission it identifies. That also means it counts
    /// against the key-time watchdog like everything else, which is correct — a station is
    /// transmitting either way.
    fn append_cw_id(&mut self, now: f64, audio_rate: f64) {
        let Some(cw) = self.config.cw_id else { return };
        let due = self
            .last_cw_id
            .is_none_or(|last| now - last >= self.config.cw_id_interval_s);
        if !due {
            return;
        }
        let cw = crate::cwid::CwId {
            level: CW_ID_RELATIVE_LEVEL,
            ..cw
        };
        let audio = cw.audio(&self.engine.my_call, audio_rate);
        if audio.is_empty() {
            return;
        }
        // a moment of silence so the identifier is not run into the data
        let gap = (0.1 * audio_rate) as usize;
        self.playback.extend(std::iter::repeat_n(0.0f32, gap));
        self.playback.extend(audio);
        self.last_cw_id = Some(now);
        self.stats.cw_ids += 1;
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

/// The callsign in a beacon frame, if that is what this is.
fn beacon_callsign(decoded: &aether_phy::DecodedFrame) -> Option<String> {
    if decoded.frame.sync.frame_type != FrameType::Data {
        return None;
    }
    let payload = decoded.payload.as_ref()?;
    let (header, body) = decode_data(payload).ok()?;
    if header.kind != DataKind::Beacon {
        return None;
    }
    unpack_callsign(&body).ok()
}

/// Link-layer timing derived from the waveform tables.
///
/// Every number here comes from [`WaveformParams`] or a measurement, never a guess: the frame
/// durations are what the layouts actually occupy, and the capacities are what the modes
/// actually carry.
#[must_use]
pub fn phy_timing(params: WaveformParams) -> PhyTiming {
    let air = aether_phy::modes::air_interface(params);
    // the mode table of the waveform in use: its capacities, and the thresholds its
    // benchmark measured, so the rate controller steps whichever table the air has
    let (capacity, thresholds): (Vec<usize>, Vec<f64>) =
        if params.bandwidth == aether_phy::waveform::Bandwidth::Narrow500 {
            (
                aether_link::rate::NARROW_PAYLOAD_BYTES.to_vec(),
                aether_link::rate::NARROW_AWGN_THRESHOLD_DB.to_vec(),
            )
        } else {
            (PAYLOAD_BYTES.to_vec(), Vec::new())
        };
    PhyTiming {
        data_frame_s: air.long.duration_s(),
        control_frame_s: air.short.duration_s(),
        // PTT, the radio's own transmit delay, and the audio buffers at both ends
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        // the station fills this in from its keying lead and the daemon's playback backlog
        tx_latency_s: 0.0,
        // acquisition reports a frame once its whole preamble is in, plus the search block:
        // four symbols on the wide air, ten where the floor family's eight-symbol preamble
        // is only complete that late (ADR-0009)
        preamble_detect_s: Some((air.longest_preamble() + 2) as f64 * params.symbol_period_s()),
        data_capacity: capacity,
        mode_threshold_db: thresholds,
        // the floor family (ADR-0009): its layouts' air times and how many modes use them
        floor_data_frame_s: air.floor_long.map(|l| l.duration_s()),
        floor_control_frame_s: air.floor_short.map(|l| l.duration_s()),
        floor_modes: air.floor_modes,
    }
}

/// Helpers other modules' tests use to get a station in a known state.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::ptt::NullPtt;

    /// A station that is in a session, for tests of what is refused during one.
    pub(crate) fn connected_station() -> Station<NullPtt> {
        let mut air = super::tests::Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(30.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected(), "the test fixture could not connect");
        air.a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ptt::NullPtt;

    /// The safety property behind starting anyway: a station whose keying interface would
    /// not open runs, but puts nothing on the air. `playback` keys before it copies a single
    /// sample, so a refusal there means the sound card is handed silence, not a burst.
    #[test]
    fn a_station_whose_keying_failed_transmits_nothing() {
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            crate::ptt::BrokenPtt::new("cannot open the serial port COM9"),
            1,
        );
        assert_eq!(
            station.ptt_fault().as_deref(),
            Some("cannot open the serial port COM9")
        );
        // ask it for the one thing that would key the radio
        station
            .beacon()
            .expect("a beacon is queued like any other burst");
        let mut out = vec![1.0f32; 4096];
        let refused = station
            .playback(&mut out)
            .expect_err("keying must fail rather than transmit");
        assert!(matches!(refused, crate::ptt::PttError::Backend(_)));
        assert!(!station.transmitting(), "it never entered transmit");
        assert!(
            out.iter().all(|&x| x == 0.0),
            "the sound card is handed silence, not a burst"
        );
    }

    #[test]
    fn a_recording_says_what_the_station_transmitted_not_only_what_it_heard() {
        // OTA-2 could not answer "what mode did that burst go out on?" from either
        // station's sidecar, because a recording held only what arrived. With both
        // recordings in hand, half the link was still invisible.
        let dir = std::env::temp_dir().join(format!("aether-sent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                record_dir: Some(dir.clone()),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station
            .start_recording(Some("sent"), None)
            .expect("start the recording");

        station.set_drive(1).expect("set drive");
        let rate = station.config.params.audio_rate;
        let _ = drain_peak(&mut station, rate * 20);
        let summary = station.stop_recording().expect("a recording");

        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&summary.sidecar).expect("sidecar"))
                .expect("json");
        let sent = sidecar["sent"].as_array().expect("a sent array");
        assert!(!sent.is_empty(), "the burst it transmitted is recorded");
        assert!(
            sent[0]["mode"].as_u64().is_some(),
            "and the mode it went out on: {}",
            sent[0]
        );
        // the keying tells you how hard the transmitter was driven for it
        let events = sidecar["events"].as_array().expect("events");
        // the ptt details stay exactly "keyed" and "released" — the replay parses them
        assert!(
            events
                .iter()
                .any(|e| e["event"] == "ptt" && e["detail"] == "released"),
            "the release event keeps the wording the replay reads back"
        );
        let peak = events
            .iter()
            .find(|e| e["event"] == "tx_peak")
            .expect("a tx_peak event");
        assert!(
            peak["detail"].as_str().expect("detail").contains("dBFS"),
            "and the burst's peak is recorded beside it: {peak}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drive_is_set_against_the_waveform_not_against_a_tone() {
        // The point of the whole thing: a tune tone is a sine, the data waveform is OFDM,
        // and the daemon scales the waveform to the tone's RMS — so the waveform's peaks
        // land well above anything the tone reaches, and an ALC set on the tone is that far
        // into limiting on traffic (OTA-2, finding 1).
        let mut station = idle_station();
        let rate = station.config.params.audio_rate;

        station.tune(2.0).expect("tune");
        let tone_peak = drain_peak(&mut station, rate * 6);

        station.set_drive(1).expect("set drive");
        let burst_peak = drain_peak(&mut station, rate * 20);

        assert!(tone_peak > 0.0 && burst_peak > 0.0, "both radiated something");
        let gap_db = 20.0 * (burst_peak / tone_peak).log10();
        assert!(
            gap_db > 2.0,
            "the waveform should peak well above the tone that sets the drive, not level              with it: {gap_db:.1} dB"
        );
    }

    #[test]
    fn the_station_reports_the_peak_it_actually_transmitted() {
        let mut station = idle_station();
        let rate = station.config.params.audio_rate;
        assert_eq!(station.tx_peak_dbfs(), None, "nothing transmitted yet");

        station.tune(2.0).expect("tune");
        let measured = drain_peak(&mut station, rate * 6);
        let reported = station.tx_peak_dbfs().expect("a transmission finished");
        assert!(
            (reported - 20.0 * f64::from(measured).log10()).abs() < 0.01,
            "reported {reported} dBFS against a measured peak of {measured}"
        );
        assert!(reported < 0.0, "a sane drive does not reach full scale");
    }

    #[test]
    fn an_operator_can_cut_a_drive_check_short_but_not_a_session() {
        let mut station = idle_station();
        station.set_drive(4).expect("set drive");
        assert!(station.tune_stop(), "the queued drive bursts are cut");
        let mut out = vec![0.0f32; 4_096];
        station.playback(&mut out).expect("playback");
        assert!(
            out.iter().all(|&x| x == 0.0),
            "nothing is left to transmit once the operator stops it"
        );
    }

    /// A station that will transmit on demand: no channel gate, no keying to fail.
    fn idle_station() -> Station<NullPtt> {
        Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        )
    }

    /// Run the station until it has nothing queued, returning the largest sample it played.
    fn drain_peak(station: &mut Station<NullPtt>, samples: usize) -> f32 {
        let mut out = vec![0.0f32; 2_048];
        let mut peak = 0.0f32;
        for _ in 0..samples.div_ceil(2_048) {
            station.playback(&mut out).expect("playback");
            for &x in &out {
                peak = peak.max(x.abs());
            }
        }
        peak
    }

    #[test]
    fn cfo_is_reported_only_when_the_acquisition_can_be_trusted() {
        // a decoded frame's offset is real, whatever its confidence
        assert_eq!(reported_cfo(true, 0.5, 1.0, 12.0), Some(12.0));
        // a confident non-decode is a real near-miss: keep its offset
        assert_eq!(reported_cfo(false, 2.0, 1.0, -3.0), Some(-3.0));
        // a low-confidence non-decode is a noise trigger: its +55 Hz is the correlator on
        // noise, not a rig error, and is not reported
        assert_eq!(reported_cfo(false, 0.8, 1.0, 55.0), None);
    }

    #[test]
    fn a_control_frame_is_judged_by_the_acquisition_it_cannot_judge_by_its_chips() {
        // Only a DATA frame carries mode chips, so every CONTROL frame arrives here with a
        // mode confidence of exactly 1.0 — a real poll and a noise trigger alike. Judging
        // on that number alone suppressed the offset of every control frame there is,
        // which is the whole floor family: connect, poll, acknowledge (OTA-2, finding 4).
        let control_mode_confidence = 1.0;

        // acquisition saw this one well clear of its threshold: it is a frame
        assert_eq!(
            reported_cfo(false, control_mode_confidence, 2.4, -1.8),
            Some(-1.8)
        );
        // and this one barely at the threshold, which is what noise does
        assert_eq!(reported_cfo(false, control_mode_confidence, 1.02, 30.5), None);
    }

    #[test]
    fn acquisition_confidence_is_measured_against_the_threshold_of_its_own_family() {
        use aether_phy::preamble::FrameType;
        use aether_phy::rx::FrameSync;

        let air = aether_phy::modes::air_interface(aether_phy::waveform::NARROW_500);
        let sync = |floor, peak| FrameSync {
            start: 0,
            cfo_hz: 0.0,
            frame_type: FrameType::Control,
            floor,
            timing_peak: peak,
            type_confidence: 1.0,
        };
        // the two families are detected by different statistics against different
        // thresholds, so the same raw peak means different things
        let ordinary = air.acceptance_threshold(false);
        let floor = air.acceptance_threshold(true);
        assert!(floor < ordinary, "the floor threshold is the lower one");
        assert!((sync(false, ordinary).detect_confidence(&air) - 1.0).abs() < 1e-9);
        assert!((sync(true, floor).detect_confidence(&air) - 1.0).abs() < 1e-9);
        // a peak that is ample for a floor candidate is a bare pass as an ordinary one
        assert!(sync(true, ordinary).detect_confidence(&air) > 1.5);
    }

    /// Step two stations against each other through a channel, in blocks of audio.
    ///
    /// Each station's playback becomes the other's capture, scaled and with noise added. This
    /// is the real thing end to end: the ARQ protocol over the real codec, over the real
    /// waveform, over real 48 kHz audio.
    pub(crate) struct Air {
        pub(crate) a: Station<NullPtt>,
        pub(crate) b: Station<NullPtt>,
        block: usize,
        gain: f32,
        noise_sigma: f32,
        state: u64,
    }

    impl Air {
        pub(crate) fn new(gain: f32, noise_sigma: f32) -> Self {
            Self::with(gain, noise_sigma, |config| config)
        }

        /// Two stations built from the default configuration, adjusted by `tweak` — the
        /// waveform, the answer-only rule, whatever a test wants both ends to share.
        pub(crate) fn with(
            gain: f32,
            noise_sigma: f32,
            tweak: impl Fn(StationConfig) -> StationConfig,
        ) -> Self {
            let config = |call: &str| {
                tweak(StationConfig {
                    callsign: call.to_owned(),
                    // the channel in these tests is a wire, so politeness would only slow them
                    wait_for_clear: false,
                    ..StationConfig::default()
                })
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
        pub(crate) fn run(
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
    fn a_message_sent_after_the_session_is_up_still_crosses() {
        let message = b"Sent once the session was already connected.";
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.run(3.0, |_, _| false);
        air.a.send(message);
        air.run(90.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received().as_slice(), message.as_slice());
    }

    #[test]
    fn a_message_sent_while_the_first_burst_is_on_the_air_still_crosses() {
        // what a host does: connect, see "connected", send at once — which lands while the
        // sending station is already keyed for its first burst of the session
        let message = b"Queued while the transmitter was already keyed.";
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, _| a.connected());
        air.run(10.0, |a, _| a.transmitting());
        assert!(air.a.transmitting(), "a never keyed after connecting");
        air.a.send(message);
        air.run(90.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received().as_slice(), message.as_slice());
    }

    #[test]
    fn the_burst_has_the_rms_the_tx_level_documents() {
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                tx_level: 0.25,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.connect("KK4XYZ").expect("idle");
        let mut out = vec![0.0f32; 4096];
        let mut audio: Vec<f32> = Vec::new();
        for _ in 0..60 {
            let count = station.playback(&mut out).expect("playback");
            audio.extend_from_slice(&out[..count]);
            station.capture(&vec![0.0f32; 4096]).expect("capture");
        }
        // the burst proper, without the keying lead and tail
        let loud: Vec<f32> = audio.iter().copied().filter(|x| x.abs() > 1e-4).collect();
        let rms = (loud.iter().map(|x| x * x).sum::<f32>() / loud.len() as f32).sqrt();
        let expected = 0.25 / std::f32::consts::SQRT_2;
        assert!(
            (rms / expected - 1.0).abs() < 0.1,
            "burst RMS {rms:.4}; tx_level / sqrt 2 is {expected:.4}"
        );
        let peak = loud.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            peak < 0.7,
            "peak {peak} leaves no headroom at tx_level 0.25"
        );
    }

    #[test]
    fn the_key_outlasts_what_the_sound_card_still_holds() {
        // the first on-air attempt: a spike on the rig's meters at the end of every burst,
        // because the key dropped while the card still held the last quarter second of it
        let lead = 0.25;
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                playback_lead_s: lead,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let rate = station.config.params.audio_rate as f64;
        station.tune(1.0).expect("idle");
        let mut out = vec![0.0f32; 960];
        let mut handed = 0usize;
        let mut last_signal = None;
        let mut released = None;
        for _ in 0..200 {
            let count = station.playback(&mut out).expect("playback");
            if let Some(offset) = out[..count].iter().rposition(|x| x.abs() > 1e-4) {
                last_signal = Some(handed + offset);
            }
            handed += count;
            if released.is_none() && last_signal.is_some() && !station.transmitting() {
                released = Some(handed);
            }
            station.capture(&vec![0.0f32; 960]).expect("capture");
        }
        let (last_signal, released) = (last_signal.expect("a tone"), released.expect("a release"));
        // silence handed to the card after the last of the tone and before the release:
        // at least the lead, or the release comes while the card still holds the tone
        let silence_s = (released - last_signal) as f64 / rate;
        assert!(
            silence_s >= lead,
            "the key was released with {:.3} s of the burst still in the sound card",
            lead - silence_s
        );
        assert!(
            (silence_s - station.config.key_tail_s).abs() < 0.03,
            "{silence_s:.3} s of silence for a tail of {:.3} s",
            station.config.key_tail_s
        );
    }

    #[test]
    fn the_transmit_level_reaches_a_tone_that_is_already_playing_and_the_tone_can_be_stopped() {
        // an operator sets drive with one hand on the rig's control and the other on the
        // level: the tone has to follow the level while it plays, and stop when asked
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                tx_level: 0.25,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.tune(4.0).expect("idle");
        let mut out = vec![0.0f32; 4800];
        let rms = |out: &[f32]| (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
        // past the keying lead and the tone's shaped edge
        for _ in 0..6 {
            station.playback(&mut out).expect("playback");
        }
        let before = rms(&out);
        let mut louder = crate::config::Config::parse(crate::config::EXAMPLE).expect("example");
        louder.audio.tx_level = 0.5;
        station.apply_live(&louder);
        station.playback(&mut out).expect("playback");
        let after = rms(&out);
        assert!(
            (after / before - 2.0).abs() < 0.05,
            "the level did not follow: {before} then {after}"
        );
        assert!(station.transmitting());
        assert!(station.tune_stop(), "there was a tone to stop");
        let count = station.playback(&mut out).expect("playback");
        assert_eq!(count, 0, "the tone kept playing");
        assert!(!station.transmitting(), "the transmitter stayed keyed");
        assert!(!station.tune_stop(), "nothing left to stop");
    }

    #[test]
    fn a_host_names_the_station_and_the_name_waits_for_the_session_to_end() {
        // Winlink Express at both ends: each end's MYCALL has to reach the modem, or a
        // call to the name the host chose is never answered. A name given mid-session is
        // for the next one — the current session is addressed by the old one on every
        // frame — and the station says so.
        let mut air = Air::new(1.0, 0.0005);
        assert_eq!(
            air.b
                .set_callsigns(&["KK4XYZ-2".to_owned(), "KK4XYZ".to_owned()]),
            Ok(true)
        );
        assert_eq!(air.b.callsigns(), ["KK4XYZ-2", "KK4XYZ"]);
        air.a.connect("KK4XYZ-2").expect("idle");
        air.a.send(b"to the name the host chose");
        air.run(90.0, |a, _| a.connected());
        assert_eq!(air.b.engine().my_call, "KK4XYZ-2");
        // mid-session: accepted, applied later
        assert_eq!(air.b.set_callsigns(&["KK4XYZ-3".to_owned()]), Ok(false));
        assert_eq!(air.b.callsigns(), ["KK4XYZ-2", "KK4XYZ"]);
        assert!(
            air.b.set_callsigns(&["TOOLONGCALL".to_owned()]).is_err(),
            "a name the air interface cannot carry is refused at once, not later"
        );
        air.a.disconnect();
        air.run(60.0, |a, b| !a.connected() && !b.connected());
        assert_eq!(air.b.callsigns(), ["KK4XYZ-3"]);
        assert_eq!(air.b.engine().my_call, "KK4XYZ-3");
    }

    /// A second session with no note of its own gets the standing note: what an unattended
    /// station has to say about itself.
    fn second_session_carries_the_standing_note(air: &mut Air, dir: &std::path::Path) {
        air.run(60.0, |a, b| {
            a.engine().state() == State::Idle && b.engine().state() == State::Idle
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.a.send(b"second session");
        air.run(60.0, |a, _| a.connected());
        air.a.disconnect();
        air.run(60.0, |a, b| !a.connected() && !b.connected());
        let standing: Vec<serde_json::Value> = std::fs::read_dir(dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
            .filter(|s: &serde_json::Value| {
                s["session"]["notes"] == "40 m dipole, the standing note"
            })
            .collect();
        assert_eq!(
            standing.len(),
            1,
            "the second recording did not carry the standing note"
        );
        assert!(
            standing[0]["session"]["frequency_hz"].is_null(),
            "a keying line cannot know the frequency, and must not pretend to"
        );
    }

    #[test]
    fn a_session_records_itself_when_asked_to() {
        // the receiving station records: what it heard is the channel, and the sidecar
        // is what its modem made of it
        let dir = std::env::temp_dir().join(format!("aether-auto-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let message = b"Recorded for posterity, and for the regression tier.";
        let mut air = Air::new(1.0, 0.0005);
        air.b
            .set_recording(Some(dir.clone()), true, "40 m dipole, the standing note");
        air.b.set_record_notes(Some("bench loopback".to_owned()));
        air.a.connect("KK4XYZ").expect("idle");
        air.a.send(message);
        air.run(90.0, |_, b| {
            b.engine().stats.bytes_delivered >= message.len()
        });
        assert!(
            air.b.recording().is_some(),
            "the session did not start a recording"
        );
        air.a.disconnect();
        air.run(60.0, |a, b| !a.connected() && !b.connected());
        assert!(
            air.b.recording().is_none(),
            "the recording did not stop with the session"
        );

        let sidecar_path = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
            .expect("a sidecar");
        let name = sidecar_path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.ends_with("_KK4XYZ_W4ODA"), "{name}");
        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&sidecar_path).unwrap()).unwrap();
        assert_eq!(sidecar["session"]["notes"], "bench loopback");
        assert_eq!(sidecar["session"]["remote"], "W4ODA");
        let frames = sidecar["frames"].as_array().expect("frames");
        assert!(
            frames
                .iter()
                .any(|f| f["decoded"] == true && f["kind"] == "data"),
            "no decoded data frame in the sidecar"
        );
        let events: Vec<&str> = sidecar["events"]
            .as_array()
            .expect("events")
            .iter()
            .filter_map(|e| e["event"].as_str())
            .collect();
        assert!(events.contains(&"ptt"), "{events:?}");
        assert!(events.contains(&"disconnected"), "{events:?}");
        assert!(sidecar["counters"]["frames_detected"].as_u64().unwrap() > 0);
        let wav = crate::record::read_wav(&sidecar_path.with_extension("wav")).expect("wav");
        assert_eq!(wav.sample_rate, 48_000);
        let seconds = wav.samples.len() as f64 / 48_000.0;
        assert!(
            (seconds - sidecar["audio"]["seconds"].as_f64().unwrap()).abs() < 1e-6,
            "the sidecar's length disagrees with the file"
        );
        assert!(
            seconds > 5.0,
            "a session this size lasts longer than {seconds} s"
        );

        // and the recording replays: the receiver, run over the file with the keyed spans
        // muted as they were live, decodes every frame the station decoded on the day
        let expectation = crate::replay::Expectation::from_sidecar(&sidecar_path).unwrap();
        assert!(!expectation.muted.is_empty(), "the station never keyed?");
        let found = crate::replay::replay(
            &sidecar_path.with_extension("wav"),
            &expectation.muted,
            expectation.bandwidth_hz,
        )
        .expect("replay");
        let verdict = crate::replay::compare(&expectation.frames, &found);
        assert!(
            verdict.holds(),
            "recorded {} decoded frames but the replay decoded {} (found {})",
            verdict.recorded,
            verdict.replayed,
            verdict.found
        );
        let _ = std::fs::remove_dir_all(&dir);
        second_session_carries_the_standing_note(&mut air, &dir);
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
    fn a_station_identifies_in_morse_when_asked_to_and_not_otherwise() {
        // Off by default: a station that identifies when it need not is spending air time,
        // and one that fails to when it must is breaking the rules. Only the operator knows
        // which applies, so the daemon does not guess.
        let quiet = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        assert!(
            quiet.config.cw_id.is_none(),
            "it identified without being asked"
        );

        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                cw_id: Some(CwId::default()),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.connect("KK4XYZ").expect("idle");
        let mut out = vec![0.0f32; 4096];
        for _ in 0..400 {
            station.playback(&mut out).expect("playback");
            station.capture(&vec![0.0f32; 4096]).expect("capture");
            if station.stats.cw_ids > 0 {
                break;
            }
        }
        assert_eq!(station.stats.cw_ids, 1, "the identifier never went out");
    }

    #[test]
    fn the_identifier_does_not_repeat_inside_its_interval() {
        // it is an identifier, not a beacon; sending it on every transmission would waste a
        // large fraction of a slow link
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                cw_id: Some(CwId::default()),
                cw_id_interval_s: 600.0,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.connect("KK4XYZ").expect("idle");
        let mut out = vec![0.0f32; 4096];
        for _ in 0..1200 {
            station.playback(&mut out).expect("playback");
            station.capture(&vec![0.0f32; 4096]).expect("capture");
        }
        assert!(
            station.now() > 60.0,
            "the test did not run long enough to matter"
        );
        assert!(
            station.stats.transmissions > 1,
            "only {} transmission(s), so there was nothing to repeat over",
            station.stats.transmissions
        );
        assert_eq!(
            station.stats.cw_ids,
            1,
            "it identified {} times in {:.0} s with a 600 s interval",
            station.stats.cw_ids,
            station.now()
        );
    }

    #[test]
    fn a_beacon_crosses_the_air_and_is_reported_but_never_answered() {
        // "can anybody hear me?" without arranging a contact first, which on HF is most of
        // what a new station needs to know
        let mut air = Air::new(1.0, 0.0005);
        air.a.beacon().expect("idle");
        air.run(30.0, |_, b| b.stats.beacons_heard > 0);

        assert_eq!(air.b.stats.beacons_heard, 1, "the beacon was not heard");
        let events = air.b.take_events();
        assert!(
            events.iter().any(|e| e.starts_with("beacon:W4ODA")),
            "the callsign was not reported: {events:?}"
        );
        assert!(
            events.iter().any(|e| e.contains("dB")),
            "the signal report is missing, which is the whole point: {events:?}"
        );
        // a channel where every beacon drew a reply would be unusable
        assert_eq!(air.b.stats.transmissions, 0, "it answered a beacon");
        assert_eq!(air.b.state(), State::Idle);
        assert_eq!(air.a.state(), State::Idle);
    }

    #[test]
    fn every_frame_is_reported_with_what_the_receiver_made_of_it() {
        // the readings a panel shows come from the receiver's own estimates, frame by
        // frame: a beacon names its sender outright, and once a session is up the frames
        // with its id are the other station's
        let mut air = Air::new(1.0, 0.0005);
        assert!(air.b.last_frame().is_none());
        assert!(air.b.constellation().is_none());
        air.a.beacon().expect("idle");
        air.run(30.0, |_, b| b.stats.beacons_heard > 0);
        let reports = air.b.take_frame_reports();
        let beacon = reports
            .iter()
            .find(|r| r.kind == "beacon")
            .expect("the beacon was reported");
        assert_eq!(beacon.from.as_deref(), Some("W4ODA"));
        assert_eq!(beacon.to, None);
        assert!(beacon.decoded);
        assert!(beacon.snr_db > 20.0, "a wire reads {} dB", beacon.snr_db);
        assert!(
            beacon.cfo_hz.abs() < 5.0,
            "a wire has no offset: {}",
            beacon.cfo_hz
        );
        assert!(beacon.confidence >= 1.0);
        let (last, points) = air.b.constellation().expect("a frame was heard");
        assert_eq!(last, beacon);
        assert!(!points.is_empty() && points.len() <= CONSTELLATION_POINTS);
        // decoded on a wire, the points sit on their constellation: none is near the origin
        assert!(points.iter().all(|(i, q)| i.hypot(*q) > 0.2));
        assert!(
            air.b.take_frame_reports().is_empty(),
            "reports are taken once"
        );

        // a session: its frames carry no callsign, and are attributed by their session id
        air.a.connect("KK4XYZ").expect("idle");
        air.run(30.0, |a, b| a.connected() && b.connected());
        air.a.take_frame_reports();
        air.b.take_frame_reports();
        air.a.send(&[0x5a; 300]);
        air.run(120.0, |_, b| b.received_len() >= 300);
        let on_b = air.b.take_frame_reports();
        let data: Vec<_> = on_b.iter().filter(|r| r.kind == "data").collect();
        assert!(!data.is_empty());
        assert!(
            data.iter().all(|r| r.from.as_deref() == Some("W4ODA")),
            "{data:?}"
        );
        let on_a = air.a.take_frame_reports();
        let acks: Vec<_> = on_a.iter().filter(|r| r.kind == "control").collect();
        assert!(!acks.is_empty());
        assert!(acks.iter().all(|r| r.from.as_deref() == Some("KK4XYZ")));
        assert!(
            acks.iter()
                .all(|r| r.control.as_deref().is_some_and(|c| c.starts_with("Ack")))
        );
    }

    #[test]
    fn a_session_keeps_its_account_and_its_throughput() {
        let mut air = Air::new(1.0, 0.0005);
        assert!(air.a.link().is_none());
        assert!(air.a.throughput_bps().abs() < f64::EPSILON);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(30.0, |a, b| a.connected() && b.connected());
        let link = air.a.link().expect("a session is up");
        assert_eq!(link.bytes_sent, 0);
        assert!(link.started_s > 0.0 && link.started_s <= air.a.now());
        // the connect frames themselves are reported, with both callsigns
        let reports = air.b.take_frame_reports();
        let request = reports
            .iter()
            .find(|r| r.kind == "connect")
            .expect("the request was heard by the called station");
        assert_eq!(request.from.as_deref(), Some("W4ODA"));
        assert_eq!(request.to.as_deref(), Some("KK4XYZ"));
        let answer = air
            .a
            .take_frame_reports()
            .into_iter()
            .find(|r| r.kind == "answer")
            .expect("the answer was heard by the caller");
        assert_eq!(answer.from.as_deref(), Some("KK4XYZ"));
        assert_eq!(answer.to.as_deref(), Some("W4ODA"));

        air.a.send(&[0x33; 500]);
        air.run(120.0, |_, b| b.received_len() >= 500);
        assert_eq!(air.a.link().expect("still up").bytes_sent, 500);
        assert_eq!(air.b.link().expect("still up").bytes_received, 500);
        assert_eq!(air.b.link().expect("still up").bytes_sent, 0);
        // the reading counts what crossed the air: 500 repeated bytes compress to a few
        // dozen, so it is small, and it is not zero
        let bps = air.b.throughput_bps();
        assert!(bps > 0.0 && bps < 20_000.0, "{bps} bit/s");
        assert!(
            air.b.link().expect("still up").started_s < air.b.now(),
            "the session has lasted a while"
        );

        air.a.disconnect();
        air.run(60.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle
        });
        assert!(air.a.link().is_none(), "the account ends with the session");
        assert!(air.a.throughput_bps().abs() < f64::EPSILON);
    }

    #[test]
    fn the_receiving_lamp_follows_a_burst() {
        let mut air = Air::new(1.0, 0.0005);
        assert!(!air.b.receiving());
        air.a.beacon().expect("idle");
        let mut lit = false;
        air.run(30.0, |_, b| {
            lit |= b.receiving();
            b.stats.beacons_heard > 0
        });
        assert!(lit, "the lamp never lit while the beacon arrived");
        air.run(3.0, |_, _| false);
        assert!(!air.b.receiving(), "the lamp stayed lit after the burst");
    }

    #[test]
    fn a_spectrum_is_available_once_a_window_of_audio_was_heard() {
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        assert!(station.spectrum().is_none());
        let fs = WIDE_2300.audio_rate as f64;
        let tone: Vec<f32> = (0..8192)
            .map(|n| 0.3 * (std::f64::consts::TAU * 1500.0 * f64::from(n) / fs).sin() as f32)
            .collect();
        station.capture(&tone).expect("capture");
        let spectrum = station.spectrum().expect("a window was heard");
        let peak = spectrum
            .bins_db
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i as f64 * spectrum.bin_hz)
            .expect("bins");
        assert!(
            (peak - 1500.0).abs() < 2.0 * spectrum.bin_hz,
            "peak at {peak} Hz"
        );
    }

    #[test]
    fn two_narrow_stations_carry_a_message_over_real_audio() {
        // the 500 Hz waveform end to end: connect, the narrow mode table, a transfer, close
        let narrow = |config: StationConfig| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        };
        let mut air = Air::with(1.0, 0.0005, narrow);
        assert_eq!(air.a.engine().timing().data_capacity.len(), 13);
        assert_eq!(air.a.engine().timing().mode_threshold_db.len(), 13);
        assert_eq!(air.a.engine().timing().floor_modes, 2);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(40.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.connected() && air.b.connected(),
            "{:?}",
            air.b.take_events()
        );
        let message = b"Aether HF at 500 Hz: the bandwidth peer-to-peer contacts are made in.";
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received(), message);
        // the frames the other end reported were narrow-table modes
        let reports = air.b.take_frame_reports();
        assert!(reports.iter().any(|r| r.kind == "data" && r.decoded));
        assert!(reports.iter().all(|r| r.mode < 13), "{reports:?}");
        air.a.disconnect();
        air.run(60.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle
        });
        assert_eq!(air.a.state(), State::Idle);
    }

    #[test]
    fn a_wide_station_does_not_hear_a_narrow_call() {
        // the two waveforms do not decode each other's preambles: a 500 Hz call at a
        // 2 300 Hz station is silence, which is the point of stating the bandwidth
        let mut air = Air::new(1.0, 0.0005);
        air.a = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                params: aether_phy::waveform::NARROW_500,
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        air.a.connect("KK4XYZ").expect("idle");
        air.run(20.0, |_, b| b.stats.frames_detected > 0);
        assert_eq!(air.b.stats.frames_detected, 0);
        assert!(!air.b.connected());
    }

    #[test]
    fn an_answer_only_station_takes_calls_and_makes_none() {
        // §97.221(c): an automatically controlled station outside the automatic sub-bands
        // may use 500 Hz only to respond. The rule is the station's, whatever the bandwidth.
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            answer_only: true,
            ..config
        });
        assert_eq!(
            air.a.connect("KK4XYZ"),
            Err("this station is answer-only: it takes calls and makes none")
        );
        assert!(air.a.beacon().is_err());
        assert_eq!(air.a.state(), State::Idle);
        // the other end (also answer-only here) is still called by a station that may call
        let caller = StationConfig {
            callsign: "N0CALL".to_owned(),
            wait_for_clear: false,
            ..StationConfig::default()
        };
        air.b = Station::new(caller, NullPtt::default(), 3);
        air.b
            .connect("W4ODA")
            .expect("a station that may call, calls");
        air.run(40.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected(), "the answer-only station answered");
        assert_eq!(air.a.role(), Role::Irs);
    }

    #[test]
    fn a_probe_over_real_audio_reports_both_directions() {
        // "can you hear me, and how well?": the answer carries the SNR the probe arrived
        // at, the prober measures the answer, and both frames are reported with their
        // callsigns — no session, and nothing left running afterwards
        let mut air = Air::new(1.0, 0.0005);
        air.a.probe("KK4XYZ", None).expect("idle");
        air.run(30.0, |a, _| a.engine().stats.probe_replies > 0);
        assert_eq!(air.a.engine().stats.probe_replies, 1, "no answer arrived");
        assert_eq!(air.b.engine().stats.probes_answered, 1);
        let events = air.a.take_events();
        let report = events
            .iter()
            .find(|e| e.starts_with("probe:KK4XYZ hears us at "))
            .unwrap_or_else(|| panic!("no probe report: {events:?}"));
        let words: Vec<&str> = report.split(' ').collect();
        let theirs: f64 = words[4].parse().expect("their reading");
        let ours: f64 = words[8].parse().expect("our reading");
        // a quiet loopback: both ends hear well, and within a few dB of each other
        assert!(theirs > 15.0 && ours > 15.0, "{report}");
        assert!((theirs - ours).abs() < 6.0, "{report}");
        assert!(
            air.b
                .take_events()
                .iter()
                .any(|e| e.starts_with("probed:W4ODA at ")),
            "the probed station did not report it"
        );
        let kinds: Vec<(&str, Option<String>, Option<String>)> = air
            .b
            .take_frame_reports()
            .into_iter()
            .map(|r| (r.kind, r.from, r.to))
            .collect();
        assert!(
            kinds.contains(&("probe", Some("W4ODA".into()), Some("KK4XYZ".into()))),
            "{kinds:?}"
        );
        assert!(
            air.a
                .take_frame_reports()
                .iter()
                .any(|r| r.kind == "probe-answer"
                    && r.from.as_deref() == Some("KK4XYZ")
                    && r.to.as_deref() == Some("W4ODA")),
            "the answer was not reported"
        );
        assert_eq!(air.a.state(), State::Idle);
        assert_eq!(air.b.state(), State::Idle);
        // an answer-only station answers probes and sends none
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            answer_only: true,
            ..config
        });
        assert_eq!(
            air.a.probe("KK4XYZ", None),
            Err("this station is answer-only: it answers probes and sends none")
        );
        air.b = Station::new(
            StationConfig {
                callsign: "N0CALL".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            3,
        );
        air.b
            .probe("W4ODA", None)
            .expect("a station that may call, probes");
        air.run(30.0, |_, b| b.engine().stats.probe_replies > 0);
        assert_eq!(
            air.a.engine().stats.probes_answered,
            1,
            "the answer-only station did not answer"
        );
    }

    #[test]
    fn a_beacon_is_refused_while_a_session_is_running() {
        // it would put an unproto frame into the middle of somebody's transfer
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(30.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected());
        assert!(air.a.beacon().is_err(), "it beaconed during a session");
    }

    #[test]
    fn a_keying_test_keys_for_the_time_asked_and_no_longer() {
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.key_test(1.0).expect("idle");
        let block = 4096;
        let mut out = vec![0.0f32; block];
        let mut keyed_samples = 0usize;
        let mut ever_keyed = false;
        for _ in 0..60 {
            let count = station.playback(&mut out).expect("playback");
            if station.transmitting() {
                ever_keyed = true;
                keyed_samples += count;
                assert!(
                    out[..count].iter().all(|&x| x == 0.0),
                    "a keying test put audio on the air"
                );
            }
            station.capture(&vec![0.0f32; block]).expect("capture");
        }
        assert!(ever_keyed, "the test never keyed the radio");
        assert!(!station.transmitting(), "it stayed keyed");
        let expected = ((1.0 + station.config.key_lead_s + station.config.key_tail_s)
            * station.config.params.audio_rate as f64) as usize;
        assert!(
            keyed_samples.abs_diff(expected) <= block,
            "keyed for {keyed_samples} samples, asked for about {expected}"
        );
        assert_eq!(station.stats.transmissions, 1);
    }

    #[test]
    fn a_keying_test_keys_now_and_a_tune_waits_for_the_channel() {
        // a fresh station has not learned the noise floor yet, so anything that radiates
        // is held; a keying test radiates nothing and must not be
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: true,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        assert!(!station.busy.settled());
        assert!(
            station.tune(1.0).is_err(),
            "a tone went out before the channel was known to be clear"
        );
        station.key_test(0.5).expect("idle");
        let mut out = vec![0.0f32; 4096];
        station.playback(&mut out).expect("playback");
        assert!(
            station.transmitting(),
            "the keying test waited for a busy detector that has nothing to say about silence"
        );
        assert_eq!(station.stats.deferred_for_busy, 0);
    }

    #[test]
    fn a_tune_tone_is_at_the_configured_level_and_frequency() {
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                tx_level: 0.3,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.tune(2.0).expect("idle");
        let mut out = vec![0.0f32; 4096];
        let mut audio: Vec<f32> = Vec::new();
        for _ in 0..40 {
            let count = station.playback(&mut out).expect("playback");
            audio.extend_from_slice(&out[..count]);
            station.capture(&vec![0.0f32; 4096]).expect("capture");
        }
        let peak = audio.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!(
            (peak - 0.3).abs() < 0.02,
            "the tone peaked at {peak}, the configured level is 0.3"
        );
        // and it is a tone, not noise: nearly all its energy at 1500 Hz
        let rate = 48_000.0;
        let energy_at = |f: f64| {
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &x) in audio.iter().enumerate() {
                let p = 2.0 * std::f64::consts::PI * f * i as f64 / rate;
                re += f64::from(x) * p.cos();
                im += f64::from(x) * p.sin();
            }
            re.hypot(im)
        };
        assert!(energy_at(1500.0) > 20.0 * energy_at(700.0));
    }

    #[test]
    fn the_level_meter_tells_an_operator_what_to_do() {
        let mut station = Station::new(StationConfig::default(), NullPtt::default(), 1);
        assert_eq!(station.audio_level().advice(), "Still listening.");

        let quiet: Vec<f32> = (0..48_000)
            .map(|i| 0.001 * (i as f32 * 0.3).sin())
            .collect();
        for block in quiet.chunks(4096).cycle().take(60) {
            station.capture(block).expect("capture");
        }
        let reading = station.audio_level();
        assert!(reading.settled);
        assert!(reading.rms_dbfs < -40.0, "{}", reading.rms_dbfs);
        assert!(
            reading.advice().starts_with("Very quiet"),
            "{}",
            reading.advice()
        );

        let hot: Vec<f32> = (0..48_000)
            .map(|i| (1.2 * (i as f32 * 0.3).sin()).clamp(-1.0, 1.0))
            .collect();
        for block in hot.chunks(4096).cycle().take(60) {
            station.capture(block).expect("capture");
        }
        let reading = station.audio_level();
        assert!(reading.clipping > 0.0005, "{}", reading.clipping);
        assert!(
            reading.advice().starts_with("Clipping"),
            "{}",
            reading.advice()
        );

        let good: Vec<f32> = (0..48_000).map(|i| 0.25 * (i as f32 * 0.3).sin()).collect();
        for block in good.chunks(4096).cycle().take(60) {
            station.capture(block).expect("capture");
        }
        assert_eq!(station.audio_level().advice(), "Good.");
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
