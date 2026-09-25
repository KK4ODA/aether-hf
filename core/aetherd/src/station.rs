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

mod datagrams;
mod fieldtest;
pub use datagrams::{
    DATAGRAM_QUEUE, DatagramQueued, DatagramRefusal, DatagramReport, DatagramRequest,
    ReceivedDatagram,
};
pub use fieldtest::{Rung, Step, TestPlan, TestRun, Transfer};

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use aether_link::{
    Action, Container, HarqBuffer, LinkConfig, LinkEngine, PhyTiming, Role, SoftFrame, State,
    frames::{
        ConnectBody, DataHeader, DataKind, ProbeBody, decode_data, encode_data, pack_callsign,
        unpack_callsign, with_bandwidth,
    },
};
use aether_phy::{
    AudioToBaseband, BasebandToAudio, Complex, Modem, Received, StreamingReceiver,
    waveform::{WIDE_2300, WaveformParams},
};

use crate::{
    busy::{BusyConfig, BusyDetector},
    compress::{Compressor, Decompressor, negotiated, offered_capabilities},
    cwid::CwId,
    ptt::{Ptt, PttError, PttWatchdog, WatchdogState},
    regulatory::{
        AirOccupancy, Authorization, Ceiling, Decision, DialSource, Direction, Edges, EmissionKind,
        Policy, Situation, Transmission,
    },
    spectrum::{PassbandMonitor, Spectrum, SpectrumAnalyser},
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
    /// Keep the exact audio of every transmission, as handed to the sound card, under
    /// `tx/` in the recordings directory, with a sidecar describing its envelope: what
    /// the modem *sent*, for holding the air against it (`[record] tx_audio`).
    pub record_tx_audio: bool,
    /// The regulatory policy's settings (ADR-0018): the profile, the station's control, the
    /// operator's class, the sideband, and a dial for a radio that cannot report its own.
    pub regulatory: crate::regulatory::Settings,
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
            record_tx_audio: false,
            // a harness checks nothing; the daemon always builds this from the file
            regulatory: crate::regulatory::Settings::unchecked(),
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
    /// The soft frame of either family: an OFDM frame, or the tone floor's (ADR-0013).
    frame: Received,
    /// The rung of the ladder it was sent at; 0 for a control frame.
    rung: usize,
    modem: Rc<RefCell<Modem>>,
    t_start: f64,
    t_end: f64,
    container: Container,
}

impl std::fmt::Debug for PhyFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhyFrame")
            .field("container", &self.container)
            .field("rung", &self.rung)
            .field("tone", &self.frame.is_tone())
            .field("rv", &self.frame.rv())
            .field("snr_3k_db", &self.frame.snr_3k_db())
            .finish_non_exhaustive()
    }
}

impl SoftFrame for PhyFrame {
    fn container(&self) -> Container {
        self.container
    }

    fn mode(&self) -> usize {
        self.rung
    }

    fn floor(&self) -> bool {
        self.frame.is_tone()
    }

    fn rv(&self) -> u8 {
        self.frame.rv()
    }

    fn snr_db(&self) -> f64 {
        self.frame.snr_3k_db()
    }

    fn t_start(&self) -> f64 {
        self.t_start
    }

    fn t_end(&self) -> f64 {
        self.t_end
    }

    fn decode(&self, buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
        let mut modem = self.modem.borrow_mut();
        match modem.decode_received(&self.frame, buffer.map(Vec::as_slice)) {
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
    /// A pause between drive bursts, with the transmitter unkeyed: time for the operator to
    /// read the ALC and move the level before the next one lands. The clock starts when it
    /// reaches the front of the queue. Cut with the bursts.
    Pause { seconds: f64, until_s: Option<f64> },
    /// The Morse identifier on its own: the end of a session whose last transmission did
    /// not carry one — the link timed out, or the peer closed it without a word.
    Identifier,
    /// A burst of a KISS client's datagram (ADR-0019): frames outside any session, which
    /// reached the queue through their own channel access (`feed_datagram`).
    Datagram {
        frames: Vec<aether_link::TxFrame>,
        /// The client's reference, for the report of what became of it.
        reference: Option<String>,
        /// Whether it is the datagram's last burst.
        last: bool,
    },
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

/// Room kept between a burst's air time and the key watchdog: the sound card's clock and
/// the loop's are not the same clock, and a burst that ends a hair past the limit loses its
/// last frame as surely as one that ends seconds past it.
const KEY_TIME_MARGIN_S: f64 = 1.0;

/// How long before a Morse identifier falls due the bursts are already sized to carry it:
/// a burst is shaped when the engine decides it and rendered a moment later. In a session
/// nothing holds a burst back, so a minute is room to spare.
const ID_LOOKAHEAD_S: f64 = 60.0;

/// The longest a burst of frames may be on the air, so that the whole transmission — the
/// keying's lead, the frames, the Morse identifier when `with_id` says one rides on it, and
/// the tail — ends inside the key watchdog's limit (ADR-0017). Six tone frames are 32 s; the
/// watchdog cut the last of every full tone burst on the air, and the link fell to the floor
/// for it. Never less than one OFDM frame's worth, so an extreme setting still sends
/// something.
fn burst_limit_s(config: &StationConfig, with_id: bool) -> f64 {
    let identifier = config.cw_id.filter(|_| with_id).map_or(0.0, |cw| {
        let rate = config.params.audio_rate as f64;
        // the identifier and the moment of silence before it (`append_cw_id`)
        cw.audio(&config.callsign, rate).len() as f64 / rate + 0.1
    });
    (config.max_key_s - config.key_lead_s - config.key_tail_s - identifier - KEY_TIME_MARGIN_S)
        .max(1.1)
}

/// How much later than the playback lead a sound card's capture of this station's own
/// burst may still be arriving, over and above the lead itself: device buffering both ways,
/// and the radio's own switch back to receive. Measured on an FTDX10 over USB: the receive
/// audio stays at digital silence for 250-325 ms after the key is released, and eight
/// blocks of that in a row would have put the busy detector's floor at −78 dBFS. Half a
/// second covers it with room for a slower rig; the receiver itself still hears throughout.
const CAPTURE_LAG_ALLOWANCE_S: f64 = 0.5;

/// How long each drive-setting burst runs: enough to read the ALC and move the level while
/// it is still transmitting, since the level is live and the meter answers at once.
const DRIVE_BURST_S: f64 = 6.0;
/// The silence between drive bursts, unkeyed: time to see where the meter settled, decide,
/// and be ready for the next one.
const DRIVE_GAP_S: f64 = 5.0;

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
/// threshold set on the statistic's maximum over *noise* implies.
///
/// It gates what an *unconfirmed* acquisition is allowed to do — report an offset, mark
/// the channel busy, extend the receive window, tell the engine a burst is arriving — and
/// nothing about what the receiver decodes: every candidate is still pursued, and a frame
/// that decodes is real whatever its acquisition looked like. Raising the detector's own
/// threshold is ADR business, because it would cost weak-signal frames that do decode.
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

/// What the session history will say of the session now up, gathered as it runs: the
/// engine forgets its readings the moment a session ends, before the station hears of it.
#[derive(Debug, Clone, Default)]
struct SessionNotes {
    remote: String,
    caller: bool,
    frequency_hz: Option<u64>,
    /// The engine's acknowledged-bytes count when the session came up.
    acked_at_start: usize,
    snr_db: Option<f64>,
    best_snr_db: Option<f64>,
    heard_there_db: Option<f64>,
    top_rung_sent: Option<usize>,
    top_rung_heard: Option<usize>,
}

/// The most finished sessions held for the daemon between two calls of
/// `take_finished_sessions`.
const MAX_UNTAKEN_SESSIONS: usize = 64;

/// A message sent with a reference, waiting for the other station to have all of it: `end`
/// is where it ends in the session's stream of link bytes.
#[derive(Debug, Clone)]
struct SentMark {
    reference: String,
    bytes: usize,
    end: usize,
}

/// What became of a message sent with a reference ([`Station::send_tracked`]): the other
/// station has all of it, or the session ended first. The panel's check mark on a sent line
/// comes from this.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Delivery {
    /// The sender's reference for the message.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Its size, application bytes.
    pub bytes: usize,
    /// Whether the other station has all of it, in order.
    pub delivered: bool,
    /// Why not, when not: how the session ended, or that there was none.
    pub reason: Option<String>,
}

/// The most deliveries kept for `status` — a panel that was closed or reloaded while one
/// resolved looks its messages up there — and held between two calls of `take_deliveries`.
const RECENT_DELIVERIES: usize = 32;

/// Where the Morse identifier stands (§97.119).
#[derive(Debug, Clone, Copy, Default)]
struct IdentifierState {
    /// When the last one went out, in station time.
    last: Option<f64>,
    /// Whether this station has transmitted since it last identified.
    transmitted_since: bool,
    /// A session ended after transmitting since the last identifier: the next transmission
    /// carries one whatever the interval says, or one goes out on its own.
    final_due: bool,
}

/// A "what if" for the regulatory policy: the station's facts, any of them replaced.
#[derive(Debug, Clone, Default)]
pub struct RegulatoryQuery {
    /// A dial, hertz.
    pub dial_hz: Option<f64>,
    /// The station's control.
    pub control: Option<crate::regulatory::ControlMode>,
    /// The operator's class.
    pub license: Option<crate::regulatory::LicenseClass>,
    /// The sideband.
    pub sideband: Option<crate::regulatory::Sideband>,
    /// Who began the exchange.
    pub direction: Option<Direction>,
    /// The fastest rung, in place of the station's `max_mode`.
    pub rung: Option<usize>,
}

/// The dial frequency, asked of the radio now and then rather than on every frame.
#[derive(Debug, Clone, Copy, Default)]
struct FrequencyCache {
    value: Option<u64>,
    next_ask_s: f64,
    /// Station time of the last reading the radio gave (or of a tune it accepted).
    read_at_s: Option<f64>,
}

/// How old the radio's reading of its dial may be when a transmission is judged: older, and
/// it is asked again first — the dial may have moved since.
const DIAL_FRESH_S: f64 = 2.0;

/// How old a reading may be and still stand for the dial at all, when the radio does not
/// answer the fresh question.
const DIAL_STALE_S: f64 = 15.0;

/// How often the regulatory ceiling on the link's rungs is worked out again, station time.
const CEILING_EVERY_S: f64 = 0.5;

/// Decisions held for the daemon between two calls of `take_regulatory_reports`.
const MAX_UNTAKEN_DECISIONS: usize = 64;

/// How long a refusal of the same kind goes unreported after the last: the gate refuses every
/// time, the log says so once.
const REPORT_QUIET_S: f64 = 10.0;

/// How far back the throughput reading looks, seconds.
const THROUGHPUT_WINDOW_S: f64 = 30.0;

/// The audio the modem occupies is centred here; the radio's receive filter must pass a band
/// of the occupied width around it, and the passband monitor measures the width at this
/// centre.
const AUDIO_CENTER_HZ: f64 = 1500.0;
/// The receiver's passband is folded into the monitor no more often than this, in seconds of
/// station time: a couple of times a second, not every block.
const PASSBAND_SAMPLE_S: f64 = 0.5;
/// Below this block level there is no real noise to read a passband from — a muted card, a
/// disconnected antenna — so the passband is not sampled then. −70 dBFS, the silence floor.
const PASSBAND_MIN_LEVEL_DBFS: f64 = -70.0;

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

/// The two playback clocks, and how the transmission now running stands against them.
///
/// A whole burst is handed to the sound card as soon as it is rendered (ADR-0010), so the
/// station's own queue drains long before the audio has left; the key is released when
/// the card has consumed as many samples since keying as the station handed it since
/// keying. The card's clock runs through the idle time too and the station's does not, so
/// only the spans since keying compare.
#[derive(Debug, Default, Clone, Copy)]
struct PlaybackClock {
    /// Samples handed to the sound card so far, silence included.
    handed: u64,
    /// The sound card's own clock as last reported by the run loop — samples it has
    /// consumed since it opened. `None` for a harness that never reports one, whose key is
    /// released the moment the queue drains, as it always was.
    device_played: Option<u64>,
    /// Where each clock stood when the transmission now running was keyed.
    handed_at_key: u64,
    played_at_key: Option<u64>,
    /// The transmission now running was cut short: what the card still held was dropped,
    /// so the key is released as soon as the station's own queue is empty.
    cut_short: bool,
    /// Whether the run loop should drop what the sound card still holds.
    flush_device: bool,
}

impl PlaybackClock {
    /// How many samples the sound card has played past the last one handed to it for the
    /// transmission now running: `Some(0)` while that is still leaving, `None` for a
    /// harness that reports no clock.
    fn played_past_transmission(&self) -> Option<u64> {
        let (played, at_key) = (self.device_played?, self.played_at_key?);
        Some(
            played
                .saturating_sub(at_key)
                .saturating_sub(self.handed - self.handed_at_key),
        )
    }

    /// Whether the card's clock has reached the last sample handed for this transmission.
    /// A harness that reports no clock is taken at its word the moment the queue drains,
    /// as it always was.
    fn caught_up(&self) -> bool {
        match (self.device_played, self.played_at_key) {
            (Some(played), Some(at_key)) => {
                played.saturating_sub(at_key) >= self.handed - self.handed_at_key
            }
            _ => true,
        }
    }
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
    /// The busy state as last observed, so a change of it is logged exactly once.
    was_busy: bool,
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
    /// The two playback clocks and how the transmission now running stands against them.
    clock: PlaybackClock,
    /// The exact audio of the transmission now running, when the operator asked for it
    /// to be kept (`[record] tx_audio`).
    tx_capture: Option<crate::record::TxCapture>,
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
    /// The session now up, as the history will record it.
    session_notes: Option<SessionNotes>,
    /// Link bytes handed to the engine this session: where the stream has got to.
    link_sent: usize,
    /// Messages sent with a reference, not yet all acknowledged, oldest first.
    sent_marks: VecDeque<SentMark>,
    /// Deliveries resolved since the daemon last took them.
    deliveries: Vec<Delivery>,
    /// The last deliveries resolved, for `status`.
    recent_deliveries: VecDeque<Delivery>,
    /// Sessions that ended since the daemon last took them.
    finished: Vec<crate::sessions::Session>,
    /// The last frame, and its equalised constellation, for the diagnostics display.
    last_frame: Option<FrameReport>,
    last_symbols: Vec<Complex>,
    /// What the sound card is delivering, for the spectrum display.
    spectrum: SpectrumAnalyser,
    /// The receiver's passband, learned from the noise it delivers between signals, so a
    /// radio filter set narrower than the modem's bandwidth is caught without asking the rig.
    passband: PassbandMonitor,
    /// Station time the passband was last sampled: it is folded in about twice a second, not
    /// every block, and only when the channel is quiet.
    passband_next_s: f64,
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
    /// The Morse identifier: when it last went out, and what is owed.
    identifier: IdentifierState,
    /// The frames of the burst on the air: how long each is, and whether they are the
    /// tone floor's — what an abort that cuts it has to wait out before its DISC.
    on_air_frames: Option<(f64, bool)>,
    /// The regulatory policy in force (ADR-0018).
    policy: Policy,
    /// What this air's transmissions occupy, measured; `None` when nothing is measured for
    /// it — every data transmission is then refused.
    occupancy: Option<&'static AirOccupancy>,
    /// Leave to key for the transmission rendered and not yet keyed. Only the policy makes
    /// one, and the keying path will not key without it.
    authorized: Option<Authorization>,
    /// Who began the session now up, or the last one: its frames are judged by it
    /// (§97.221(c)(1)).
    session_direction: Option<Direction>,
    /// The fastest rung the rules allow now, and why not the next.
    ceiling: Option<Ceiling>,
    /// When the ceiling was last worked out.
    ceiling_at_s: f64,
    /// Decisions for the log and the clients: every refusal, and permitted automatic-control
    /// transmissions when asked for.
    regulatory_reports: Vec<Decision>,
    /// The last decision reported, and when: a refusal repeated within
    /// [`REPORT_QUIET_S`] is not reported again.
    last_report: Option<(&'static str, String, f64)>,
    /// The last decision the gate made, and when.
    last_gate: Option<(f64, Decision)>,
    /// KISS clients' datagrams: waiting, on the air, and heard (ADR-0019).
    datagrams: datagrams::Datagrams,
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
            // the first transmission identifies, so the first bursts keep room for it
            max_burst_s: config
                .link
                .max_burst_s
                .or_else(|| Some(burst_limit_s(&config, true))),
            ..config.link.clone()
        };
        let engine = LinkEngine::new(&config.callsign, timing, link, seed);
        let busy = BusyDetector::new(BusyConfig {
            fs: params.fs_baseband,
            passband_hz: params.occupied_bandwidth_hz(),
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
            was_busy: false,
            tx_peak_running: 0.0,
            tx_peak_last: None,
            held_since: None,
            deaf_until: f64::NEG_INFINITY,
            clock: PlaybackClock::default(),
            tx_capture: None,
            pending: VecDeque::new(),
            meter: LevelMeter::new(params.audio_rate as f64, 3.0),
            recording: None,
            record_notes: None,
            test: None,
            pending_callsigns: None,
            delivered: Vec::new(),
            events: Vec::new(),
            reports: Vec::new(),
            session_notes: None,
            link_sent: 0,
            sent_marks: VecDeque::new(),
            deliveries: Vec::new(),
            recent_deliveries: VecDeque::new(),
            finished: Vec::new(),
            last_frame: None,
            last_symbols: Vec::new(),
            spectrum: SpectrumAnalyser::new(params.audio_rate as f64),
            passband: PassbandMonitor::new(),
            passband_next_s: 0.0,
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
            identifier: IdentifierState::default(),
            on_air_frames: None,
            policy: Policy::from_setting(&config.regulatory.profile),
            occupancy: crate::regulatory::occupancy::air(params.bandwidth.hz()).ok(),
            authorized: None,
            session_direction: None,
            ceiling: None,
            ceiling_at_s: f64::NEG_INFINITY,
            regulatory_reports: Vec::new(),
            last_report: None,
            last_gate: None,
            datagrams: datagrams::Datagrams::new(seed),
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

    /// The sound card's playback clock, as the run loop reads it before each top-up:
    /// samples the card has consumed since it opened.
    ///
    /// A whole burst is handed to the card as soon as it is rendered, so the station's own
    /// queue drains long before the audio has left; the key is released against this clock
    /// instead, when the last sample has really played. A harness that never reports one
    /// gets the old behaviour: release on drain.
    pub fn device_played(&mut self, played: u64) {
        self.clock.device_played = Some(played);
    }

    /// Whether the run loop should drop what the sound card still holds — set when a
    /// transmission was cut short — and clear the request.
    pub fn take_device_flush(&mut self) -> bool {
        std::mem::take(&mut self.clock.flush_device)
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

    /// What the station is running with, as last applied.
    #[must_use]
    pub fn config(&self) -> &StationConfig {
        &self.config
    }

    /// Frames reported since the last call.
    pub fn take_frame_reports(&mut self) -> Vec<FrameReport> {
        std::mem::take(&mut self.reports)
    }

    /// Sessions that ended since the last call, for the history. Their wall-clock times
    /// are the caller's to give ([`Session::ended_at`](crate::sessions::Session::ended_at)).
    pub fn take_finished_sessions(&mut self) -> Vec<crate::sessions::Session> {
        std::mem::take(&mut self.finished)
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

    /// The receiver's passband width in hertz, learned from the noise between signals, once
    /// enough quiet audio has been heard. Materially below [`occupied_bandwidth_hz`] means
    /// the radio's filter is set narrower than the modem's signal — it carries only the
    /// middle of every burst, which looks like a dead band. `None` until it has heard enough.
    ///
    /// [`occupied_bandwidth_hz`]: Self::occupied_bandwidth_hz
    #[must_use]
    pub fn rx_passband_hz(&self) -> Option<f64> {
        self.passband.width_hz(AUDIO_CENTER_HZ)
    }

    /// The audio bandwidth the modem's signal occupies, in hertz: what the radio's receive
    /// filter has to pass for a burst to arrive whole.
    #[must_use]
    pub fn occupied_bandwidth_hz(&self) -> f64 {
        self.config.params.occupied_bandwidth_hz()
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
            self.frequency.read_at_s = Some(now);
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

    /// Start a new interval for the detector's peak reading, once the last one has gone out.
    pub fn reset_busy_peak(&mut self) {
        self.busy.reset_excess_peak();
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

    /// Queue a message for the session and follow it: a [`Delivery`] under `reference` says
    /// when the other station has all of it, in order — from the link's acknowledgements,
    /// so after every byte before it too — or that the session ended first. A message given
    /// with no session up is not queued, and is reported undelivered at once.
    pub fn send_tracked(&mut self, data: &[u8], reference: &str) {
        if !self.connected() {
            self.resolve(Delivery {
                reference: reference.to_owned(),
                bytes: data.len(),
                delivered: false,
                reason: Some("no session".to_owned()),
            });
            return;
        }
        self.outbound.extend_from_slice(data);
        self.flush_outbound();
        self.sent_marks.push_back(SentMark {
            reference: reference.to_owned(),
            bytes: data.len(),
            end: self.link_sent,
        });
        self.pump();
    }

    /// Deliveries resolved since the last call.
    pub fn take_deliveries(&mut self) -> Vec<Delivery> {
        std::mem::take(&mut self.deliveries)
    }

    /// The references still waiting, oldest first, and the last deliveries resolved.
    #[must_use]
    pub fn delivery_status(&self) -> (Vec<String>, Vec<Delivery>) {
        (
            self.sent_marks
                .iter()
                .map(|m| m.reference.clone())
                .collect(),
            self.recent_deliveries.iter().cloned().collect(),
        )
    }

    fn resolve(&mut self, delivery: Delivery) {
        if self.recent_deliveries.len() >= RECENT_DELIVERIES {
            self.recent_deliveries.pop_front();
        }
        self.recent_deliveries.push_back(delivery.clone());
        if self.deliveries.len() >= RECENT_DELIVERIES {
            self.deliveries.remove(0);
        }
        self.deliveries.push(delivery);
    }

    /// Resolve the messages the other station now has all of: the stream handed to the
    /// engine this session, less what it has not yet delivered in order.
    fn note_arrivals(&mut self) {
        if self.link.is_none() || self.sent_marks.is_empty() {
            return;
        }
        let arrived = self
            .link_sent
            .saturating_sub(self.engine.tx_undelivered_bytes());
        while self.sent_marks.front().is_some_and(|m| m.end <= arrived) {
            let Some(mark) = self.sent_marks.pop_front() else {
                break;
            };
            self.resolve(Delivery {
                reference: mark.reference,
                bytes: mark.bytes,
                delivered: true,
                reason: None,
            });
        }
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
        self.link_sent += wire.len();
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

    /// Close the session now: what is left of a burst goes with it, and one DISC goes out.
    ///
    /// The rest of the burst is the session's, and the session is over. Left to play, a
    /// long tone burst ran on after the operator's abort until the key watchdog cut it,
    /// ten seconds later (ND1J, 2026-09-25), with the DISC queued behind it.
    ///
    /// The DISC waits, key up, for what the cut would otherwise have run into: the rest of
    /// the frame that was on the air, which the other station's receiver still holds the
    /// span of, and the acknowledgement it then sends for the burst. Sent at once, the DISC
    /// arrived inside the one and under the other, and the peer never heard it.
    pub fn abort(&mut self) {
        self.pending
            .retain(|next| !matches!(next, Outgoing::Frames(_)));
        if self.transmitting && !self.playing_test {
            self.playback.clear();
            self.authorized = None;
            self.clock.flush_device = true;
            self.clock.cut_short = true;
            if self.engine.state() != State::Idle
                && let Some((frame_s, floor)) = self.on_air_frames
            {
                let timing = self.engine.timing();
                let wait = frame_s
                    + self.engine.config().burst_gap_s
                    + timing.control_frame_s_for(floor)
                    + 2.0 * timing.turnaround_s
                    + 0.5;
                self.pending.push_back(Outgoing::Pause {
                    seconds: wait,
                    until_s: None,
                });
            }
        }
        self.engine.abort();
        self.pump();
    }

    /// Nothing on the air, nothing queued for it, and no session: a station that may stop.
    #[must_use]
    pub fn quiescent(&self) -> bool {
        !self.transmitting
            && self.playback.is_empty()
            && self.pending.is_empty()
            && self.engine.state() == State::Idle
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

    /// Give up on the transmission in progress: drop what is queued and what the sound card
    /// still holds, release the key, and tell the engine the transmission is over so its
    /// timers run again. For the run loop when the radio or the sound card has failed under
    /// it — a keying error, or a card that has stopped delivering audio while keyed. Unlike
    /// [`shut_down`](Self::shut_down) the session is left intact: the engine's own timeouts
    /// then retransmit or end it, rather than the modem going silent mid-contact.
    ///
    /// # Errors
    /// If the radio refuses to release. The local state is cleared either way, so the next
    /// pass can try again.
    pub fn abandon_tx(&mut self) -> Result<(), PttError> {
        let was_transmitting = self.transmitting;
        self.playback.clear();
        self.authorized = None;
        // a datagram on the air, or its burst waiting, is lost with the transmission
        self.datagram_transmitted(true);
        for dropped in std::mem::take(&mut self.pending) {
            if let Outgoing::Datagram {
                reference, last, ..
            } = dropped
            {
                self.refuse_datagram(reference, last, "the transmitter failed");
            }
        }
        self.clock.flush_device = true;
        self.clock.cut_short = false;
        self.tx_capture = None;
        self.transmitting = false;
        self.playing_test = false;
        self.tx_peak_running = 0.0;
        let now = self.now();
        let result = self.ptt.unkey(now);
        if was_transmitting {
            self.engine.on_tx_done(now);
            self.pump();
        }
        result
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
        self.refresh_burst_cap(self.now());
        self.config.wait_for_clear = config.radio.wait_for_clear;
        self.config.link.max_mode = config.radio.fastest_mode();
        self.config.answer_only = config.radio.answer_only;
        self.ptt.max_key_s = config.radio.max_key_s;
        self.busy.set_threshold_db(config.radio.busy_threshold_db);
        self.config.record_auto = config.record.auto;
        self.config.record_notes.clone_from(&config.record.notes);
        self.config.operator.clone_from(&config.operator);
        self.set_regulatory(config.regulatory.settings());
    }

    /// New regulatory settings, from the next transmission on: the policy is loaded again
    /// when the profile changed, and the ceiling on the link's rungs is worked out again.
    pub fn set_regulatory(&mut self, settings: crate::regulatory::Settings) {
        if settings.profile != self.config.regulatory.profile {
            self.policy = Policy::from_setting(&settings.profile);
        }
        self.config.regulatory = settings;
        self.refresh_ceiling(true);
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
        self.frequency.read_at_s = Some(self.now());
        self.frequency.next_ask_s = self.now() + 2.0;
        self.refresh_ceiling(true);
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
        // beacons go out on the tone floor, where calls and probes start (ADR-0016): the
        // slowest kind that carries a connect frame carries a callsign, and a beacon exists to
        // be heard by somebody who can hear nothing else — a station of either bandwidth, as
        // the floor's frames are the same on both airs
        let mode = self.engine.robust_mode(true);
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
        let index = self.config.link.max_mode.min(air.n_rungs() - 1);
        let rung = air.rung(index);
        let bytes = rung.payload_bytes();
        if bytes == 0 {
            return Err("this mode carries no payload to send");
        }
        // not all-zero: the codec refuses that block (ADR-0009)
        let payload = vec![0x5a_u8; bytes];
        // A burst is several frames back to back, long enough to read the ALC and turn the
        // knob against, and the bursts are spaced by as much silence: a one-frame burst is
        // a second long, which is no time at all for a hand on a drive control.
        let frame_s = rung.duration_s().max(0.1);
        let per_burst = (DRIVE_BURST_S / frame_s).ceil().max(1.0) as usize;
        let frame = aether_link::TxFrame {
            container: aether_link::Container::Data,
            payload,
            mode: index,
            rv: 0,
            // a data frame's family follows its mode; this flag is for control frames
            floor: false,
        };
        for n in 0..bursts {
            if n > 0 {
                self.pending.push_back(Outgoing::Pause {
                    seconds: DRIVE_GAP_S,
                    until_s: None,
                });
            }
            self.pending
                .push_back(Outgoing::Drive(vec![frame.clone(); per_burst]));
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
                Outgoing::Audio { tone: true, .. } | Outgoing::Drive(_) | Outgoing::Pause { .. }
            )
        });
        let mut stopped = self.pending.len() != queued;
        if self.playing_test {
            // the keying tail goes with it; the transmitter unkeys as soon as the queue
            // is empty, which is the point — and the sound card holds the rest of the
            // burst, so the run loop is asked to drop that too, and the release does not
            // wait for audio that will never play
            self.playback.clear();
            self.clock.flush_device = true;
            self.clock.cut_short = true;
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
            //
            // The deafness ends where the transmission did, though, not where the loop
            // noticed: the card's clock says how much of this block came in after the last
            // sample had left, and that much is the channel again and is heard. A pass slow
            // enough to bring in the end of our own tail and the start of the peer's reply
            // together used to mute the reply's start with the tail — the first frame of
            // the burst after every slow pass, on a loaded machine.
            self.busy.skip(&baseband);
            let after = self.captured_after_transmission(audio.len(), baseband.len());
            if after == 0 || !self.playback.is_empty() {
                let muted = vec![(0.0, 0.0); baseband.len()];
                self.absorb(&muted, now);
            } else {
                let during = baseband.len() - after;
                let ended = now - after as f64 * self.config.params.fs_baseband.recip();
                let muted = vec![(0.0, 0.0); during];
                self.absorb(&muted, ended);
                self.finish_transmission(ended)?;
                self.absorb(&baseband[during..], now);
            }
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
        self.note_busy_transition(now);
        self.sample_passband(now);

        self.refresh_burst_cap(now);
        self.refresh_ceiling(false);
        self.engine.tick(now);
        if self.ptt.poll(now)? == WatchdogState::Tripped {
            // the key was stuck: drop whatever was still queued rather than resume mid-burst
            self.stats.watchdog_trips += 1;
            self.playback.clear();
            self.authorized = None;
            self.pending.clear();
            self.clock.flush_device = true;
            self.clock.cut_short = false;
            self.tx_capture = None;
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
        //
        // The whole burst is handed over as soon as it is rendered, so the queue here
        // drains while the sound card still holds most of it; the key is released when
        // the card's own clock has passed the last sample handed over. A harness that
        // reports no clock releases on drain, as before.
        if self.playback.is_empty() && self.transmitting {
            if !self.clock.cut_short && !self.clock.caught_up() {
                return Ok(0); // still leaving the sound card
            }
            self.finish_transmission(now)?;
            return Ok(0);
        }
        if self.playback.is_empty() {
            self.start_pending(now);
        }
        if self.playback.is_empty() {
            return Ok(0);
        }

        if !self.transmitting {
            // Nothing the regulatory gate has not judged is keyed: leave is made only by the
            // policy (`Policy::authorize`), in `start_pending`, and spent here.
            if self.authorized.take().is_none() {
                self.playback.clear();
                self.events.push(
                    "error:regulatory: audio reached the transmitter without the gate's leave; \
                     nothing was keyed"
                        .to_owned(),
                );
                return Ok(0);
            }
            self.ptt.key(now)?;
            self.transmitting = true;
            self.clock.handed_at_key = self.clock.handed;
            self.clock.played_at_key = self.clock.device_played;
            self.clock.cut_short = false;
            self.stats.transmissions += 1;
            if let Some(recording) = &mut self.recording {
                recording.event(now, "ptt", "keyed", &format!("{:?}", self.engine.state()));
            }
            if let Some(capture) = &mut self.tx_capture {
                capture.keyed(now);
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
        self.clock.handed += count as u64;
        if let Some(capture) = &mut self.tx_capture {
            capture.push(&out[..count]);
        }
        Ok(count)
    }

    /// The transmission is over — its queue drained and, by the card's clock, its last
    /// sample gone: release the key, tell the engine, close the capture of what was sent.
    fn finish_transmission(&mut self, now: f64) -> Result<(), PttError> {
        let cut = self.clock.cut_short;
        self.clock.cut_short = false;
        self.transmitting = false;
        self.playing_test = false;
        self.tx_peak_last = Some(self.tx_peak_running);
        let peak = self.tx_peak_running;
        self.tx_peak_running = 0.0;
        if let Some(capture) = self.tx_capture.take() {
            match capture.finish(self.config.record_dir.as_deref()) {
                Ok(summary) => self.events.push(format!("tx:{summary}")),
                Err(error) => self.events.push(format!("error:tx capture: {error}")),
            }
        }
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
        self.datagram_transmitted(cut);
        // the queue drained now, and what the sound card still holds is the tail's
        // silence: the burst itself has already left
        self.engine.on_tx_done(now);
        self.deaf_until = now + self.config.playback_lead_s + CAPTURE_LAG_ALLOWANCE_S;
        self.pump();
        Ok(())
    }

    /// How many of the baseband samples of the block just captured came in after the
    /// transmission now running had wholly left the sound card. The card's playback clock
    /// and the capture run at one rate, so what the card has played past the transmission's
    /// last sample is what the capture holds past it — near enough: a card captures its own
    /// tail a little later still, and that is silence either way. `captured` audio samples
    /// became `baseband` baseband ones.
    fn captured_after_transmission(&self, captured: usize, baseband: usize) -> usize {
        let past = self
            .clock
            .played_past_transmission()
            .map_or(0, |past| usize::try_from(past).unwrap_or(usize::MAX));
        if past == 0 || captured == 0 {
            return 0;
        }
        let after = past.min(captured) as f64 * baseband as f64 / captured as f64;
        (after.round() as usize).min(baseband)
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
            let air = self.air();
            let control = decoded.frame.is_control();
            // the rung a DATA frame was sent at; chips naming an OFDM mode on no rung of the
            // ladder are noise, and go no further (the model's harness drops them too)
            let Some(rung) = (if control {
                Some(0)
            } else {
                decoded.frame.rung(&air)
            }) else {
                continue;
            };
            self.report(&decoded, rung, now);
            let detected = decoded.frame.detect_confidence(&air);
            if let Some(recording) = &mut self.recording {
                recording.frame(crate::record::FrameRecord {
                    t_s: now,
                    kind: if control {
                        "control".into()
                    } else {
                        "data".into()
                    },
                    mode: rung,
                    rv: decoded.frame.rv(),
                    snr_3k_db: decoded.frame.snr_3k_db(),
                    cfo_hz: reported_cfo(
                        decoded.ok(),
                        decoded.frame.mode_confidence(),
                        detected,
                        decoded.frame.cfo_hz(),
                    ),
                    confidence: decoded.frame.mode_confidence(),
                    detect_confidence: detected,
                    decoded: decoded.ok(),
                    bytes: decoded.payload.as_ref().map_or(0, Vec::len),
                    control: if control {
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
                self.busy.mark_frame(now, detected);
                self.note(
                    "beacon",
                    &format!("{caller} at {:.1} dB", decoded.frame.snr_3k_db()),
                );
                continue;
            }
            let container = if control {
                Container::Control
            } else {
                Container::Data
            };
            let start = decoded.frame.start() as f64 / fs;
            let frame_s = decoded.frame.samples(&air) as f64 / fs;
            let decoded_ok = decoded.ok();
            let frame = PhyFrame {
                container,
                t_start: start,
                t_end: start + frame_s,
                rung,
                frame: decoded.frame,
                modem: Rc::clone(&self.decoder),
            };
            // A frame that decoded is real whatever its acquisition looked like, and a weak
            // one at the floor may have acquired below the gate below — it counts here. One
            // that did not decode is a soft frame the engine may still combine, and it says
            // nothing about the channel: a phantom gets this far too.
            if decoded_ok {
                self.busy.mark_frame(now, detected);
            }
            self.engine.on_frame(&frame, now);
        }

        self.heed_preambles(&preambles, now);
        self.pump();
    }

    /// The air interface this station runs, for the numbers that only mean something
    /// against it — an acquisition threshold, a layout, a mode table.
    fn air(&self) -> aether_phy::modes::AirInterface {
        aether_phy::modes::air_interface(self.config.params)
    }

    /// Describe a frame for the displays, and keep its constellation. `rung` is the rung of
    /// the ladder a DATA frame was sent at.
    fn report(&mut self, decoded: &aether_phy::DecodedFrame, rung: usize, now: f64) {
        let frame = &decoded.frame;
        let control = frame.is_control();
        let payload = decoded.payload.as_deref();
        let mut report = FrameReport {
            t_s: now,
            kind: if control { "control" } else { "data" },
            mode: rung,
            rv: frame.rv(),
            snr_db: frame.snr_3k_db(),
            cfo_hz: frame.cfo_hz(),
            confidence: frame.mode_confidence(),
            detect_confidence: frame.detect_confidence(&self.air()),
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
                DataKind::Datagram => report.kind = "datagram",
            }
        }
        // a piece of somebody's datagram: joined with the others, and named by the first
        if report.kind == "datagram"
            && let Some(piece) = payload
        {
            report.from = self.heard_datagram_piece(piece, report.snr_db, rung, now);
        }
        // the constellation, thinned evenly so a long frame costs a display no more
        // than a short one; a tone frame has none — one tone at a time, detected by energy
        self.last_symbols = frame.ofdm().map_or_else(Vec::new, |ofdm| {
            let step = ofdm.symbols.len().div_ceil(CONSTELLATION_POINTS).max(1);
            ofdm.symbols.iter().step_by(step).copied().collect()
        });
        self.last_frame = Some(report.clone());
        // bounded, for a station nobody drains: a test harness, or a client that never asks
        if self.reports.len() >= MAX_UNTAKEN_REPORTS {
            self.reports.remove(0);
        }
        // the burst may go on: the next frame's preamble is a symbol or two away. But only
        // a frame that decoded, or acquired past the gate the preamble path takes, says a
        // burst is there at all — on a quiet band the detector tries a noise candidate
        // every few seconds and every one of them ends here undecoded, and this half
        // second lit the receive lamp for each of them (26 blinks in a 71 s recording of
        // an empty frequency, none acquired above 1.16)
        if report.decoded || report.detect_confidence >= DETECT_CONFIDENCE_TRUSTED {
            self.rx_until = self.rx_until.max(now + 0.5);
        }
        if let Some(notes) = &mut self.session_notes
            && report.decoded
            && report.from.as_deref() == Some(notes.remote.as_str())
        {
            notes.snr_db = Some(report.snr_db);
            notes.best_snr_db = Some(
                notes
                    .best_snr_db
                    .map_or(report.snr_db, |b| b.max(report.snr_db)),
            );
            if report.kind == "data" {
                notes.top_rung_heard = Some(notes.top_rung_heard.map_or(rung, |t| t.max(rung)));
            }
        }
        self.reports.push(report);
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

    /// The session that just ended, for the history: called at the `disconnected` event,
    /// while the account and the recording are still the session's.
    fn finish_session(&mut self, end: &str) {
        let (Some(notes), Some(link)) = (self.session_notes.take(), self.link) else {
            return;
        };
        let session = crate::sessions::Session {
            remote: notes.remote,
            started_ms: 0,
            ended_ms: 0,
            duration_s: ((self.now() - link.started_s).max(0.0) * 10.0).round() / 10.0,
            role: if notes.caller {
                crate::sessions::Role::Caller
            } else {
                crate::sessions::Role::Called
            },
            bandwidth_hz: u32::try_from(self.config.params.bandwidth.hz()).unwrap_or(u32::MAX),
            frequency_hz: self.frequency.value.or(notes.frequency_hz),
            bytes_sent: link.bytes_sent,
            bytes_acked: self
                .engine
                .stats
                .bytes_acked
                .saturating_sub(notes.acked_at_start),
            bytes_received: link.bytes_received,
            end: end.to_owned(),
            snr_db: notes.snr_db,
            best_snr_db: notes.best_snr_db,
            heard_there_db: notes.heard_there_db,
            top_rung_sent: notes.top_rung_sent,
            top_rung_heard: notes.top_rung_heard,
            test: self.test_running(),
            recording: self.recording().and_then(|(path, _)| {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            }),
        };
        if self.finished.len() >= MAX_UNTAKEN_SESSIONS {
            self.finished.remove(0);
        }
        self.finished.push(session);
    }

    /// A session came up: its coders, its account, its notes for the history, and the
    /// engine's stream counted from nothing again.
    fn session_began(&mut self, detail: &str) {
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
        self.link_sent = 0;
        // the frames of a session are the call's or the answer's (§97.221(c)(1))
        self.session_direction = Some(if detail.ends_with("(iss)") {
            Direction::Originate
        } else {
            Direction::Respond
        });
        self.refresh_ceiling(true);
        self.session_notes = Some(SessionNotes {
            remote: detail
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned(),
            caller: detail.ends_with("(iss)"),
            frequency_hz: self.frequency.value,
            acked_at_start: self.engine.stats.bytes_acked,
            ..SessionNotes::default()
        });
        self.moved.clear();
    }

    /// A session ended, however it ended: its identifier falls due, it joins the history,
    /// what it did not deliver is said to be undelivered, and its coders go.
    fn session_ended(&mut self, detail: &str) {
        // the end of a communication is identified, whatever ended it
        if self.config.cw_id.is_some() && self.identifier.transmitted_since {
            self.identifier.final_due = true;
        }
        self.finish_session(detail);
        // what the other station did not have all of when the session ended never will: the
        // engine has already let the stream go
        for mark in std::mem::take(&mut self.sent_marks) {
            self.resolve(Delivery {
                reference: mark.reference,
                bytes: mark.bytes,
                delivered: false,
                reason: Some(detail.to_owned()),
            });
        }
        self.link = None;
        self.moved.clear();
        self.compressor = Compressor::new(false);
        self.decompressor = Decompressor::new(false);
        if let Some(calls) = self.pending_callsigns.take() {
            // validated when they were given; the engine is idle now
            let _ = self.engine.set_callsigns(&calls);
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
                        self.session_began(&detail);
                        connected = true;
                    } else if name == "disconnected" {
                        self.session_ended(&detail);
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
        if let Some(notes) = &mut self.session_notes
            && let Some(heard) = self.engine.peer_snr_db()
        {
            notes.heard_there_db = Some(heard);
        }
        self.note_arrivals();
        // a DISC or DISC_ACK of ours still to go carries the identifier; a session that
        // ended in silence — a link timeout, a peer that closed without a word — gets one
        // on its own, after whatever is on the air now
        if self.identifier.final_due
            && !self
                .pending
                .iter()
                .any(|next| matches!(next, Outgoing::Frames(_)))
        {
            self.identifier.final_due = false;
            self.pending.push_back(Outgoing::Identifier);
        }
        self.account();
    }

    /// Render the next queued burst into playable audio, if the channel allows.
    fn start_pending(&mut self, now: f64) {
        if let Some(Outgoing::Pause { seconds, until_s }) = self.pending.front_mut() {
            let deadline = *until_s.get_or_insert(now + *seconds);
            if now < deadline {
                return; // the operator's hands are on the drive control; leave them to it
            }
            self.pending.pop_front();
            self.start_pending(now);
            return;
        }
        // a KISS client's datagram goes when nothing else is queued and no session is up
        if self.pending.is_empty() {
            self.feed_datagram(now);
        }
        if self.pending.is_empty() || self.held_for_busy(now) {
            return;
        }
        // the regulatory gate: every transmission, whatever it is, is judged before it is
        // rendered, and keyed only with the leave the policy gives (ADR-0018)
        if !self.gate(now) {
            return;
        }
        self.render_next(now);
    }

    /// Whether the transmission at the head of the queue waits for a busy channel: a
    /// transmission this station starts, when the operator asked to wait for a clear one.
    fn held_for_busy(&mut self, now: f64) -> bool {
        let Some(next) = self.pending.front() else {
            return true;
        };
        // silence radiates nothing, so a keying test does not wait for the channel
        let radiates = !matches!(next, Outgoing::Audio { silent: true, .. });
        // A probe answer is a response to a frame addressed to this station (§97.221(c)),
        // not a transmission this station is starting — and the very frame it answers has
        // just marked the channel busy for two seconds (`mark_frame`), so holding it for
        // that meant every probe to a station with `wait_for_clear` on reported "no answer".
        // A connect answer never reaches here held: the station is Connected by then and
        // `channel_clear` lets a session's frames through.
        let responding = matches!(next, Outgoing::Frames(frames) if is_probe_answer(frames));
        // a datagram has had its own channel access already: the client's DCD and persistence
        let own_access = matches!(next, Outgoing::Datagram { .. });
        if radiates
            && !responding
            && !own_access
            && self.config.wait_for_clear
            && !self.channel_clear(now)
        {
            self.stats.deferred_for_busy += 1;
            // the engine's timers move with the burst, or a retry fires against a burst
            // that has not left yet and the two go out back to back when the channel clears
            if let Some(since) = self.held_since {
                self.engine.on_tx_delayed(now - since);
            }
            self.held_since = Some(now);
            return true;
        }
        self.held_since = None;
        false
    }

    /// Take the transmission at the head of the queue — judged and allowed — and render it.
    fn render_next(&mut self, now: f64) {
        let Some(outgoing) = self.pending.pop_front() else {
            return;
        };
        let cuttable = matches!(outgoing, Outgoing::Drive(_));
        let frames = match outgoing {
            Outgoing::Frames(frames) | Outgoing::Drive(frames) => frames,
            Outgoing::Datagram {
                frames,
                reference,
                last,
            } => {
                if last {
                    self.datagrams.on_air = Some(datagrams::OnAir { reference });
                }
                frames
            }
            // handled above, before anything is popped
            Outgoing::Pause { .. } => return,
            Outgoing::Identifier => return self.render_identifier(now),
            Outgoing::Audio { samples, tone, .. } => return self.render_audio(samples, tone),
        };

        self.record_sent(&frames, now);
        self.on_air_frames = frames.first().map(|frame| {
            let timing = self.engine.timing();
            let floor = match frame.container {
                aether_link::Container::Data => timing.is_floor(frame.mode),
                aether_link::Container::Control => frame.floor,
            };
            (timing.frame_s(frame), floor)
        });
        if self.config.record_tx_audio {
            self.tx_capture = Some(crate::record::TxCapture::begin(
                self.config.params.audio_rate as u32,
                self.config.tx_level,
                self.config.key_lead_s,
                self.config.key_tail_s,
                &frames,
            ));
        }

        let mut baseband: Vec<Complex> = Vec::new();
        for frame in &frames {
            let burst = match frame.container {
                // a rung of the ladder: the tone floor's, at its peak, or an OFDM mode
                Container::Data => {
                    self.transmitter
                        .rung_burst(&frame.payload, frame.mode, frame.rv)
                }
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

    /// The Morse identifier on its own, inside the keying's lead and tail: the end of a
    /// session no DISC of this station's carried it for.
    fn render_identifier(&mut self, now: f64) {
        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.config.key_tail_s * audio_rate) as usize;
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.identifier.final_due = true;
        self.append_cw_id(now, audio_rate);
        if self.playback.len() == lead {
            // nothing to say after all — no identifier set any more, or none that fits the
            // key: key nothing, and owe nothing, or the next pass would queue it again
            self.playback.clear();
            self.identifier.final_due = false;
            return;
        }
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
    }

    /// Raw audio goes out as it is, inside the same keying and lead and tail as a burst, so
    /// a keying test exercises exactly the path a transmission uses.
    fn render_audio(&mut self, samples: Vec<f32>, tone: bool) {
        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.config.key_tail_s * audio_rate) as usize;
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.playback.extend(samples);
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.playing_test = tone;
        if self.config.record_tx_audio {
            self.tx_capture = Some(crate::record::TxCapture::begin(
                self.config.params.audio_rate as u32,
                self.config.tx_level,
                self.config.key_lead_s,
                self.config.key_tail_s,
                &[],
            ));
        }
    }

    /// Act on the preambles acquisition found in this block: the start-of-frame signal.
    ///
    /// It tells the engine a burst is still running long before the frame itself arrives,
    /// and lights the receive indicator. It does **not** mark the channel busy. It used
    /// to — on the reasoning that acquiring a frame is better evidence than any power
    /// measurement — and on a crowded band that lit the busy indicator for seconds with
    /// nothing above the threshold, because the detector false-alarms many times a minute
    /// (OTA-2: 13.6) and every phantom held the channel for two seconds. A confidence gate
    /// was tried first and measured on the same recordings: phantoms reach 1.54 over their
    /// threshold and a real frame at the base acquired at 1.38, so no cut on acquisition
    /// confidence separates them. The one piece of evidence a phantom can never produce is
    /// a decode, so busy is asserted by the level criterion and by decoded frames, and a
    /// preamble alone asserts nothing (`field/OTA-2-FINDINGS.md`).
    ///
    /// The engine's timing signal and the receive indicator still take the gate: a phantom
    /// telling the engine a burst is arriving held this station's own turn. An OFDM
    /// preamble's threshold sits at the statistic's noise maximum, so its acquisitions take
    /// [`DETECT_CONFIDENCE_TRUSTED`]; a tone frame (ADR-0013) is announced only once its
    /// first sync block has cleared a threshold set over the noise maximum of that block and
    /// beaten its neighbours for three symbols, which is the gate already.
    fn heed_preambles(&mut self, preambles: &[aether_phy::PendingFrame], now: f64) {
        let air = self.air();
        let fs = self.config.params.fs_baseband;
        for pending in preambles {
            if !pending.tone && pending.detect_confidence < DETECT_CONFIDENCE_TRUSTED {
                continue;
            }
            // the frame named itself — an OFDM preamble its layout, a tone frame its kind:
            // its own air time, not a guess from the peer's last mode — a floor frame after
            // ordinary connect frames is five times as long, and an answer timed for an
            // ordinary one tramples it (ADR-0012)
            let frame_s = pending.end.saturating_sub(pending.start) as f64 / fs;
            self.rx_until = self.rx_until.max(now + frame_s.max(air.long.duration_s()));
            self.engine
                .on_preamble(pending.start as f64 / fs, now, Some(frame_s));
        }
    }

    /// Log a change of the busy state with the numbers that decided it.
    ///
    /// `busy_until` is one number extended by several paths, and when the indicator lights
    /// with nothing above the threshold the only useful question is which path did it. So
    /// every transition says: the level and floor at that moment, the threshold, and the
    /// reason the hold was last extended.
    /// Fold the receiver's noise spectrum into the passband monitor, at most a couple of
    /// times a second and only when the channel is quiet: the detector has learned the floor,
    /// nothing is on the channel, we are not hearing our own tail, and there is real noise to
    /// measure. So what it learns is the receiver's own passband, not a signal that is on.
    fn sample_passband(&mut self, now: f64) {
        if now < self.passband_next_s {
            return;
        }
        let quiet = self.busy.settled()
            && !self.busy.busy(now)
            && !self.transmitting
            && now >= self.deaf_until
            && self.busy.level_db > PASSBAND_MIN_LEVEL_DBFS;
        if !quiet {
            return;
        }
        if let Some(spectrum) = self.spectrum.compute() {
            self.passband.observe(&spectrum);
            self.passband_next_s = now + PASSBAND_SAMPLE_S;
        }
    }

    fn note_busy_transition(&mut self, now: f64) {
        let busy = self.busy.busy(now);
        if busy == self.was_busy {
            return;
        }
        self.was_busy = busy;
        let margin = self.busy.config().threshold_db;
        let (level, floor) = (self.busy.level_db, self.busy.floor_db);
        // one line per transition, every quantity the decision used, in a fixed order:
        //   signal | floor | delta | margin | threshold | before -> after | reason
        let why = if busy {
            match self.busy.reason() {
                Some(crate::busy::BusyReason::Level { .. }) => {
                    "energy over the threshold for the attack".to_owned()
                }
                Some(crate::busy::BusyReason::Frame { detect_confidence }) => {
                    format!("a frame decoded (acquired at {detect_confidence:.2})")
                }
                Some(crate::busy::BusyReason::Shape { peak_db }) => {
                    format!("the passband is peaked ({peak_db:.1} dB over its median)")
                }
                None => "no reason recorded".to_owned(),
            }
        } else {
            "hangover ran out with the energy under the threshold".to_owned()
        };
        let detail = format!(
            "{level:.1} dBFS | floor {floor:.1} | delta {:+.1} dB | margin {margin:.1} |              threshold {:.1} dBFS | {} | {why}",
            level - floor,
            floor + margin,
            if busy { "OFF -> ON" } else { "ON -> OFF" },
        );
        self.note("busy", &detail);
    }

    /// Record what this station is putting on the air, beside what it hears.
    ///
    /// Without it a sidecar cannot answer the first question a failed burst raises — what
    /// mode was that? — and even with both stations' recordings in hand, half of the link
    /// stays invisible: a burst that decoded nowhere looks exactly like a burst that was
    /// never sent (`field/OTA-2-FINDINGS.md`).
    fn record_sent(&mut self, frames: &[aether_link::TxFrame], now: f64) {
        if let Some(notes) = &mut self.session_notes {
            let data = frames.iter().filter(|frame| {
                frame.container == Container::Data
                    && decode_data(&frame.payload).is_ok_and(|(h, _)| h.kind == DataKind::Data)
            });
            for frame in data {
                notes.top_rung_sent = Some(
                    notes
                        .top_rung_sent
                        .map_or(frame.mode, |t| t.max(frame.mode)),
                );
            }
        }
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
        let due = self.identifier.final_due
            || self
                .identifier
                .last
                .is_none_or(|last| now - last >= self.config.cw_id_interval_s);
        if !due {
            self.identifier.transmitted_since = true;
            return;
        }
        let cw = crate::cwid::CwId {
            level: CW_ID_RELATIVE_LEVEL,
            // an identifier keyed by an automatic device is sent no faster than the rules
            // allow (§97.119(b)(1): 20 wpm)
            wpm: self
                .policy
                .cw_id_max_wpm()
                .map_or(cw.wpm, |max| cw.wpm.min(max)),
            ..cw
        };
        // The identifier rides inside the burst's keying, so the gate's leave for the burst
        // covers its data, not this: it is judged here as the CW emission it is (ADR-0018),
        // and a refused one waits for a transmission of its own, which is judged again.
        let identifier = Transmission {
            what: "the Morse identifier".to_owned(),
            kind: EmissionKind::Cw,
            audio: Edges::tone(cw.tone_hz, crate::regulatory::occupancy::CW_HALF_WIDTH_HZ),
            direction: self.session_direction.unwrap_or(Direction::Originate),
        };
        let situation = self.situation(false);
        let decision = self.policy.evaluate(&situation, &identifier);
        if !decision.allowed() {
            self.identifier.transmitted_since = true;
            self.report_decision(&decision, now);
            return;
        }
        let audio = cw.audio(&self.engine.my_call, audio_rate);
        if audio.is_empty() {
            return;
        }
        // a moment of silence so the identifier is not run into the data
        let gap = (0.1 * audio_rate) as usize;
        // Bursts are sized to leave it room from a minute before it falls due; one that
        // does not — an operator's own `max_burst_s`, a watchdog setting just lowered —
        // passes it to the next transmission rather than taking the burst over the key's
        // limit, where the watchdog would cut both.
        let on_air =
            (self.playback.len() + gap + audio.len()) as f64 / audio_rate + self.config.key_tail_s;
        if on_air > self.config.max_key_s - KEY_TIME_MARGIN_S / 2.0 {
            self.identifier.transmitted_since = true;
            return;
        }
        self.playback.extend(std::iter::repeat_n(0.0f32, gap));
        self.playback.extend(audio);
        self.identifier.last = Some(now);
        self.stats.cw_ids += 1;
        self.identifier.transmitted_since = false;
        self.identifier.final_due = false;
    }

    /// Whether a Morse identifier will be due by station time `t`.
    fn id_due_by(&self, t: f64) -> bool {
        self.config.cw_id.is_some()
            && (self.identifier.final_due
                || self
                    .identifier
                    .last
                    .is_none_or(|last| t - last >= self.config.cw_id_interval_s))
    }

    /// Size the engine's bursts to the key: with room for the identifier when it falls due
    /// within [`ID_LOOKAHEAD_S`], without it otherwise — an identifying station gives up a
    /// tone frame only on the burst the identifier rides on. An explicit `max_burst_s` in
    /// the link settings stands.
    fn refresh_burst_cap(&mut self, now: f64) {
        if self.config.link.max_burst_s.is_some() {
            return;
        }
        let with_id = self.id_due_by(now + ID_LOOKAHEAD_S);
        self.engine
            .set_max_burst_s(Some(burst_limit_s(&self.config, with_id)));
    }

    // ── the regulatory gate (ADR-0018) ───────────────────────────────────

    /// The situation the rules judge. The dial is the radio's when it can report it — asked
    /// again first when `fresh` and the last reading is older than [`DIAL_FRESH_S`], and
    /// only while not transmitting — or the dial the operator entered; unknown otherwise.
    fn situation(&mut self, fresh: bool) -> Situation {
        let now = self.now();
        let settings = &self.config.regulatory;
        let (dial_hz, source) = if self.ptt.can_tune() {
            let stale = self
                .frequency
                .read_at_s
                .is_none_or(|t| now - t > DIAL_FRESH_S);
            if fresh && stale && !self.transmitting {
                if let Some(hz) = self.ptt.inner_mut().frequency_hz() {
                    self.frequency.value = Some(hz);
                    self.frequency.read_at_s = Some(now);
                    self.frequency.next_ask_s = now + 10.0;
                }
            } else if !fresh {
                // the cadence the dial display keeps anyway: at most every ten seconds
                let _ = self.frequency_hz();
            }
            let recent = self
                .frequency
                .read_at_s
                .is_some_and(|t| now - t <= DIAL_STALE_S);
            (
                self.frequency.value.filter(|_| recent).map(|hz| hz as f64),
                Some(DialSource::Radio),
            )
        } else {
            (
                settings.dial_hz.map(|hz| hz as f64),
                Some(DialSource::Declared),
            )
        };
        let settings = &self.config.regulatory;
        Situation {
            dial_hz,
            dial_source: dial_hz.and(source),
            sideband: settings.sideband,
            control: settings.control,
            license: settings.license,
            itu_region: settings.itu_region,
            margin_hz: settings.margin_hz,
            band_plan: settings.band_plan,
            power_w: self.config.operator.power_w,
        }
    }

    /// Who began the exchange the next transmission belongs to, outside a frame that says:
    /// the session's direction, or — for a station with none yet — an answer when it is
    /// automatically controlled (answering is what it does) and a call otherwise.
    fn standing_direction(&self) -> Direction {
        if self.engine.state() != State::Idle
            && let Some(direction) = self.session_direction
        {
            return direction;
        }
        match self.config.regulatory.control {
            Some(crate::regulatory::ControlMode::Automatic) => Direction::Respond,
            _ => Direction::Originate,
        }
    }

    /// What a queued transmission is, as the rules class it: how wide, and who began it.
    fn transmission_of(&self, outgoing: &Outgoing) -> Result<Transmission, String> {
        let air = self.occupancy.ok_or_else(|| {
            format!(
                "nothing is measured for the {} Hz air",
                self.config.params.bandwidth.hz()
            )
        })?;
        match outgoing {
            Outgoing::Frames(frames)
            | Outgoing::Drive(frames)
            | Outgoing::Datagram { frames, .. } => {
                let mut edges: Option<Edges> = None;
                for frame in frames {
                    let e = match frame.container {
                        Container::Data => air
                            .rung(frame.mode)
                            .ok_or_else(|| format!("rung {} is not measured", frame.mode))?,
                        Container::Control => air.control(frame.floor),
                    };
                    edges = Some(edges.map_or(e, |x| x.union(e)));
                }
                let audio = edges.ok_or("an empty burst")?;
                let (what, direction) = if matches!(outgoing, Outgoing::Drive(_)) {
                    ("a drive burst".to_owned(), Direction::Operator)
                } else {
                    self.describe_frames(frames, air)
                };
                Ok(Transmission {
                    what,
                    kind: EmissionKind::Data,
                    audio,
                    direction,
                })
            }
            Outgoing::Identifier => Ok(Transmission {
                what: "the Morse identifier".to_owned(),
                kind: EmissionKind::Cw,
                audio: Edges::tone(
                    self.config.cw_id.map_or(AUDIO_CENTER_HZ, |c| c.tone_hz),
                    crate::regulatory::occupancy::CW_HALF_WIDTH_HZ,
                ),
                direction: self.session_direction.unwrap_or(Direction::Originate),
            }),
            Outgoing::Audio { silent: true, .. } => Ok(Transmission {
                what: "a keying test".to_owned(),
                kind: EmissionKind::Nothing,
                audio: Edges::tone(AUDIO_CENTER_HZ, 0.0),
                direction: Direction::Operator,
            }),
            Outgoing::Audio { .. } => Ok(Transmission {
                what: "the tune tone".to_owned(),
                kind: EmissionKind::Test,
                audio: Edges::tone(
                    AUDIO_CENTER_HZ,
                    crate::regulatory::occupancy::TONE_HALF_WIDTH_HZ,
                ),
                direction: Direction::Operator,
            }),
            Outgoing::Pause { .. } => Err("a pause is not a transmission".to_owned()),
        }
    }

    /// What a burst of frames is, in words, and who began its exchange: a call, a probe and
    /// a beacon start one; an acceptance and a probe's answer answer one; everything else
    /// belongs to the session.
    fn describe_frames(
        &self,
        frames: &[aether_link::TxFrame],
        air: &AirOccupancy,
    ) -> (String, Direction) {
        let session = self.session_direction.unwrap_or(Direction::Originate);
        let Some(first) = frames.first() else {
            return ("nothing".to_owned(), session);
        };
        match first.container {
            Container::Data => match decode_data(&first.payload).map(|(h, _)| h.kind) {
                Ok(DataKind::ConnectReq) => ("a call".to_owned(), Direction::Originate),
                Ok(DataKind::ConnectAck) => ("the answer to a call".to_owned(), Direction::Respond),
                Ok(DataKind::Beacon) => ("a beacon".to_owned(), Direction::Originate),
                Ok(DataKind::Probe) => ("a probe".to_owned(), Direction::Originate),
                Ok(DataKind::ProbeAck) => ("the answer to a probe".to_owned(), Direction::Respond),
                Ok(DataKind::Datagram) => {
                    ("a KISS client's datagram".to_owned(), Direction::Originate)
                }
                _ => {
                    let name = air.rungs.get(first.mode).map_or("", |r| r.name.as_str());
                    (
                        format!("a data burst at rung {} ({name})", first.mode),
                        session,
                    )
                }
            },
            Container::Control => {
                let kind = aether_link::frames::ControlFrame::decode(&first.payload)
                    .map_or_else(|_| "control".to_owned(), |c| format!("{:?}", c.kind));
                (format!("a {} frame", kind.to_lowercase()), session)
            }
        }
    }

    /// Judge the transmission at the head of the queue. With leave, it is kept for the
    /// keying path and the transmission may be rendered; without, the transmission is
    /// dropped and the refusal reported.
    fn gate(&mut self, now: f64) -> bool {
        let Some(next) = self.pending.front() else {
            return false;
        };
        let judged = self.transmission_of(next);
        let s = self.situation(true);
        let result = match judged {
            Ok(tx) => self.policy.authorize(&s, &tx),
            Err(why) => Err(Box::new(self.policy.unmeasured(
                &s,
                "this transmission",
                &why,
            ))),
        };
        match result {
            Ok(leave) => {
                let decision = leave.decision().clone();
                if self.config.regulatory.log_permitted
                    && decision.control == Some(crate::regulatory::ControlMode::Automatic)
                {
                    self.report_decision(&decision, now);
                }
                self.last_gate = Some((now, decision));
                self.authorized = Some(leave);
                true
            }
            Err(decision) => {
                self.refuse(*decision, now);
                false
            }
        }
    }

    /// A transmission the rules refuse: it is dropped, reported, and — when it was a
    /// session's — the session is over. Its DISC is judged like anything else and goes only
    /// if it may; an abrupt disconnect frame that would itself be unlawful is never sent.
    fn refuse(&mut self, decision: Decision, now: f64) {
        let outgoing = self.pending.pop_front();
        let why = decision.summary.clone();
        self.authorized = None;
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, "regulatory", &decision.summary, &state);
        }
        self.report_decision(&decision, now);
        self.last_gate = Some((now, decision));
        match outgoing {
            Some(Outgoing::Frames(_)) if self.engine.state() != State::Idle => {
                self.note(
                    "regulatory",
                    "session halted: its next transmission is not lawful",
                );
                self.engine.abort();
                self.pump();
            }
            Some(Outgoing::Identifier) => self.identifier.final_due = false,
            Some(Outgoing::Datagram {
                reference, last, ..
            }) => self.refuse_datagram(reference, last, &why),
            _ => {}
        }
    }

    /// Put a decision in the log's and the clients' queue, unless the same one went a moment
    /// ago.
    fn report_decision(&mut self, decision: &Decision, now: f64) {
        let repeat = self.last_report.as_ref().is_some_and(|(code, what, at)| {
            *code == decision.code && *what == decision.what && now - at < REPORT_QUIET_S
        });
        if repeat {
            return;
        }
        self.last_report = Some((decision.code, decision.what.clone(), now));
        if self.regulatory_reports.len() >= MAX_UNTAKEN_DECISIONS {
            self.regulatory_reports.remove(0);
        }
        self.regulatory_reports.push(decision.clone());
    }

    /// Decisions to log and publish since the last call.
    pub fn take_regulatory_reports(&mut self) -> Vec<Decision> {
        std::mem::take(&mut self.regulatory_reports)
    }

    /// Work out the regulatory ceiling on the link's rungs again, and hand it to the engine:
    /// what the rules allow limits what link adaptation may choose (§97.221(c)(2) for an
    /// automatic station answering outside the §97.221(b) segments, a segment's edge for
    /// anybody). Now when `force`, otherwise at most every [`CEILING_EVERY_S`].
    fn refresh_ceiling(&mut self, force: bool) {
        let now = self.now();
        if !force && now - self.ceiling_at_s < CEILING_EVERY_S {
            return;
        }
        self.ceiling_at_s = now;
        let Some(air) = self.occupancy else {
            self.engine.set_ceiling(Some(0));
            self.ceiling = None;
            return;
        };
        let s = self.situation(false);
        let direction = self.standing_direction();
        let timing = self.engine.timing().clone();
        let ceiling = self
            .policy
            .ceiling(&s, air, direction, &|r| timing.is_floor(r));
        let rung = match self.policy {
            Policy::NoProfile => None,
            // nothing is allowed: the gate refuses whatever the engine offers, and the
            // engine offers the least
            _ => Some(ceiling.rung.unwrap_or(0)),
        };
        self.engine.set_ceiling(rung);
        self.ceiling = Some(ceiling);
    }

    /// Whether the rules let this station start an exchange now — a call, a probe or a
    /// beacon, whose first frame goes on the tone floor — or the refusal.
    ///
    /// # Errors
    /// The refusal, with its reasoning.
    pub fn check_originate(&mut self) -> Result<(), Box<Decision>> {
        let rung = self.engine.robust_mode(true);
        self.check(
            Direction::Originate,
            EmissionKind::Data,
            Some(rung),
            "a call, a probe or a beacon",
        )
    }

    /// Whether the rules let an operator test the station now: a tune tone, a keying test or
    /// drive bursts at the fastest rung.
    ///
    /// # Errors
    /// The refusal, with its reasoning.
    pub fn check_operator(&mut self, kind: EmissionKind) -> Result<(), Box<Decision>> {
        let top = self.config.link.max_mode;
        let (rung, what) = match kind {
            EmissionKind::Data => (Some(top), "drive bursts"),
            EmissionKind::Nothing => (None, "a keying test"),
            _ => (None, "the tune tone"),
        };
        self.check(Direction::Operator, kind, rung, what)
    }

    fn check(
        &mut self,
        direction: Direction,
        kind: EmissionKind,
        rung: Option<usize>,
        what: &str,
    ) -> Result<(), Box<Decision>> {
        let s = self.situation(true);
        let Some(air) = self.occupancy else {
            return Err(Box::new(self.policy.unmeasured(
                &s,
                what,
                "nothing is measured for this air",
            )));
        };
        let audio = match (kind, rung) {
            (EmissionKind::Data, Some(r)) => air
                .rung(r.min(air.rungs.len() - 1))
                .unwrap_or(air.control_floor),
            (EmissionKind::Nothing, _) => Edges::tone(AUDIO_CENTER_HZ, 0.0),
            _ => Edges::tone(
                AUDIO_CENTER_HZ,
                crate::regulatory::occupancy::TONE_HALF_WIDTH_HZ,
            ),
        };
        let tx = Transmission {
            what: what.to_owned(),
            kind,
            audio,
            direction,
        };
        let decision = self.policy.evaluate(&s, &tx);
        if decision.allowed() {
            Ok(())
        } else {
            Err(Box::new(decision))
        }
    }

    /// Where the station stands with the rules: the situation, what its widest transmission
    /// would be, the ceiling on the link's rungs, the gate's last decision and the dials
    /// where its waveforms fit — for the panel's indicator and diagnostics.
    pub fn regulatory_status(&mut self) -> serde_json::Value {
        let s = self.situation(false);
        let direction = self.standing_direction();
        let policy_state = match &self.policy {
            Policy::Unset => "unset",
            Policy::NoProfile => "none",
            Policy::Rules(_) => "rules",
            Policy::Broken(_) => "broken",
        };
        let profile = self.policy.profile().map(|p| {
            serde_json::json!({
                "id": p.id, "name": p.name, "authority": p.authority,
                "rules_as_of": p.rules_as_of, "source": p.source,
                "bandwidth_reading": p.bandwidth.reading,
            })
        });
        let error = match &self.policy {
            Policy::Broken(e) => Some(e.clone()),
            _ => None,
        };
        let Some(air) = self.occupancy else {
            return serde_json::json!({ "policy": policy_state, "profile": profile, "error": "nothing is measured for this air" });
        };
        let top = self.config.link.max_mode.min(air.rungs.len() - 1);
        let timing = self.engine.timing().clone();
        let envelope = |upto: usize| {
            (0..=upto)
                .map(|r| air.rungs[r].edges.union(air.control(timing.is_floor(r))))
                .reduce(Edges::union)
                .unwrap_or(air.control_floor)
        };
        let widest = Transmission {
            what: format!(
                "the {} Hz waveform up to rung {top} ({})",
                air.bandwidth_hz, air.rungs[top].name
            ),
            kind: EmissionKind::Data,
            audio: envelope(top),
            direction,
        };
        let floor = Transmission {
            what: format!("the tone floor ({})", air.rungs[0].name),
            kind: EmissionKind::Data,
            audio: envelope(0),
            direction,
        };
        let full = self.policy.evaluate(&s, &widest);
        let ceiling = self.ceiling.clone();
        let indicator = match ceiling.as_ref().and_then(|c| c.rung) {
            Some(k) if !full.allowed() && k < top => {
                let mut limited = self.policy.evaluate(
                    &s,
                    &Transmission {
                        what: format!("rungs 0–{k}"),
                        audio: envelope(k),
                        ..widest.clone()
                    },
                );
                limited.verdict = crate::regulatory::Verdict::Warning;
                limited.summary = format!(
                    "FCC: rungs 0–{k} only here — {}",
                    full.summary.trim_start_matches("TX BLOCKED: ")
                );
                limited.detail = format!(
                    "{} The link stays on rungs 0–{k} ({}), which fit.",
                    full.detail, air.rungs[k].name
                );
                limited
            }
            _ => full,
        };
        let last = self.last_gate.as_ref().map(|(t, d)| {
            serde_json::json!({ "t_s": t, "age_s": (self.now() - t).max(0.0), "decision": d })
        });
        serde_json::json!({
            "policy": policy_state,
            "profile": profile,
            "error": error,
            "situation": s,
            "direction": direction,
            "indicator": indicator,
            "ceiling": ceiling.map(|c| serde_json::json!({
                "rung": c.rung,
                "name": c.rung.and_then(|r| air.rungs.get(r)).map(|r| r.name.clone()),
                "of": air.rungs.len(),
                "limit": c.limit,
            })),
            "last": last,
            "occupied": {
                "widest": { "audio": widest.audio, "rung": top },
                "floor": { "audio": floor.audio, "rung": 0 },
            },
            "safe_dials": {
                "widest": self.policy.safe_dials(&s, widest.audio, direction),
                "floor": self.policy.safe_dials(&s, floor.audio, direction),
            },
        })
    }

    /// The profile in force, as data.
    #[must_use]
    pub fn regulatory_profile(&self) -> Option<serde_json::Value> {
        self.policy
            .profile()
            .and_then(|p| serde_json::to_value(p).ok())
    }

    /// What the rules would say about a transmission with any of the station's facts
    /// replaced: the decision, the dials where it would fit, and the ceiling on the link.
    pub fn regulatory_check(&mut self, q: &RegulatoryQuery) -> serde_json::Value {
        let mut s = self.situation(false);
        if let Some(dial) = q.dial_hz {
            s.dial_hz = Some(dial);
            s.dial_source = Some(DialSource::Declared);
        }
        if q.control.is_some() {
            s.control = q.control;
        }
        if q.license.is_some() {
            s.license = q.license;
        }
        if q.sideband.is_some() {
            s.sideband = q.sideband;
        }
        let direction = q.direction.unwrap_or_else(|| self.standing_direction());
        let Some(air) = self.occupancy else {
            return serde_json::json!({ "error": "nothing is measured for this air" });
        };
        let top = q
            .rung
            .unwrap_or(self.config.link.max_mode)
            .min(air.rungs.len() - 1);
        let timing = self.engine.timing().clone();
        let audio = (0..=top)
            .map(|r| air.rungs[r].edges.union(air.control(timing.is_floor(r))))
            .reduce(Edges::union)
            .unwrap_or(air.control_floor);
        let tx = Transmission {
            what: format!(
                "the {} Hz waveform up to rung {top} ({})",
                air.bandwidth_hz, air.rungs[top].name
            ),
            kind: EmissionKind::Data,
            audio,
            direction,
        };
        let ceiling = self
            .policy
            .ceiling(&s, air, direction, &|r| timing.is_floor(r));
        serde_json::json!({
            "decision": self.policy.evaluate(&s, &tx),
            "safe_dials": self.policy.safe_dials(&s, audio, direction),
            "ceiling": ceiling,
            "situation": s,
        })
    }

    /// Replace the policy: a test's own profile.
    #[cfg(test)]
    pub(crate) fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
        self.refresh_ceiling(true);
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

/// Whether a queued burst is a probe answer: one data frame carrying a `ProbeAck`. Such a
/// burst is a response to a frame addressed to this station and is not held for a busy
/// channel (see [`Station::start_pending`]).
fn is_probe_answer(frames: &[aether_link::TxFrame]) -> bool {
    frames.len() == 1
        && frames[0].container == Container::Data
        && decode_data(&frames[0].payload)
            .is_ok_and(|(header, _)| header.kind == DataKind::ProbeAck)
}

/// The callsign in a beacon frame, if that is what this is.
fn beacon_callsign(decoded: &aether_phy::DecodedFrame) -> Option<String> {
    if decoded.frame.is_control() {
        return None;
    }
    let payload = decoded.payload.as_ref()?;
    let (header, body) = decode_data(payload).ok()?;
    if header.kind != DataKind::Beacon {
        return None;
    }
    unpack_callsign(&body).ok()
}

/// Link-layer timing derived from the waveform tables — what the model's harness
/// (`phy_timing`) hands its engines.
///
/// Every number here comes from [`WaveformParams`] or a measurement, never a guess: the frame
/// durations are what the frames actually occupy, and the capacities are what the rungs of
/// the air's ladder — the tone floor's two, then its OFDM modes (ADR-0013) — actually carry.
///
/// # Panics
/// If the air's tone floor has data kinds of different lengths, which the link layer cannot
/// time: the fixed tables never have.
#[must_use]
pub fn phy_timing(params: WaveformParams) -> PhyTiming {
    let air = aether_phy::modes::air_interface(params);
    let narrow = params.bandwidth == aether_phy::waveform::Bandwidth::Narrow500;
    // the thresholds the air's benchmarks measured, so the rate controller steps whichever
    // ladder the air has (empty: the wide one), and its control frames' for the family
    let (thresholds, controls) = if narrow {
        (
            aether_link::rate::NARROW_AWGN_THRESHOLD_DB.to_vec(),
            aether_link::rate::NARROW_CONTROL_THRESHOLD_DB,
        )
    } else {
        (Vec::new(), aether_link::rate::CONTROL_THRESHOLD_DB)
    };
    let floor_s = air.tone_data()[0].duration_s();
    assert!(
        air.tone_data()
            .iter()
            .all(|k| (k.duration_s() - floor_s).abs() < 1e-12),
        "the link layer takes one floor data-frame length"
    );
    PhyTiming {
        data_frame_s: air.long.duration_s(),
        control_frame_s: air.short.duration_s(),
        // PTT, the radio's own transmit delay, and the audio buffers at both ends
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        // the station fills this in from its keying lead and the daemon's playback backlog
        tx_latency_s: 0.0,
        // acquisition reports an OFDM frame once its preamble is in, plus the sidelobe guard
        // and the search block: four symbols
        preamble_detect_s: Some(
            (aether_phy::PREAMBLE_SYMBOLS + 2) as f64 * params.symbol_period_s(),
        ),
        data_capacity: air
            .ladder()
            .iter()
            .map(aether_phy::Rung::payload_bytes)
            .collect(),
        mode_threshold_db: thresholds,
        // the tone floor (ADR-0013): its frames' air times and how many rungs are its
        floor_data_frame_s: Some(floor_s),
        floor_control_frame_s: Some(air.tone_control().duration_s()),
        floor_modes: air.floor_modes(),
        control_threshold_db: Some(controls),
        // the wide air's first OFDM rung stays productive on a fading path a decibel above
        // its 10 % point; the narrow air's does not (ADR-0013 §4)
        floor_margin_db: (!narrow).then_some(aether_link::rate::WIDE_FLOOR_MARGIN_DB),
        // a tone frame is announced once its first sync block is in and has beaten its
        // neighbours, plus this receiver's own lateness (blanker, filter and a block)
        floor_preamble_detect_s: Some(aether_phy::tone::announce_delay_s(0.1)),
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

        assert!(
            tone_peak > 0.0 && burst_peak > 0.0,
            "both radiated something"
        );
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
    fn drive_bursts_are_long_and_leave_the_operator_time_between_them() {
        // A one-frame burst is a second long, which is no time at all for a hand on a
        // drive control: the bursts run several seconds and are spaced by silence with the
        // transmitter unkeyed, so the meter can be read, the level moved, and the next one
        // seen to land.
        let mut station = idle_station();
        let rate = station.config.params.audio_rate;
        station.set_drive(2).expect("set drive");

        let mut out = vec![0.0f32; 2_048];
        let silence = vec![0.0f32; 2_048];
        let mut keyed_runs: Vec<(usize, usize)> = Vec::new(); // (start, end) in samples
        let mut was = false;
        let mut at = 0usize;
        for _ in 0..(rate * 30 / 2_048) {
            // the station's clock is what it has captured, so the sound card's other
            // direction is fed too, as it is live
            station.capture(&silence).expect("capture");
            station.playback(&mut out).expect("playback");
            let now = station.transmitting();
            if now && !was {
                keyed_runs.push((at, at));
            }
            if now {
                keyed_runs.last_mut().expect("a run").1 = at + out.len();
            }
            was = now;
            at += out.len();
        }
        assert_eq!(
            keyed_runs.len(),
            2,
            "two bursts, two keyings: {keyed_runs:?}"
        );
        let secs = |(a, b): (usize, usize)| (b - a) as f64 / rate as f64;
        assert!(
            secs(keyed_runs[0]) >= DRIVE_BURST_S - 0.5,
            "the first burst runs about {DRIVE_BURST_S} s, not {:.1}",
            secs(keyed_runs[0])
        );
        let gap = (keyed_runs[1].0 - keyed_runs[0].1) as f64 / rate as f64;
        assert!(
            gap >= DRIVE_GAP_S - 0.5,
            "the gap between them is about {DRIVE_GAP_S} s of silence, not {gap:.1}"
        );
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

    /// A block of quiet: low-level noise, as a receiver delivers it. A constant value is
    /// not quiet — the band-pass turns it into digital silence, which the busy detector
    /// rightly refuses to learn as a noise floor.
    fn quiet_block(seed: u32) -> Vec<f32> {
        let mut state = seed.wrapping_mul(2_654_435_761) | 1;
        (0..4096)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (f64::from(state) / f64::from(u32::MAX) - 0.5) as f32 * 0.004
            })
            .collect()
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
        assert_eq!(
            reported_cfo(false, control_mode_confidence, 1.02, 30.5),
            None
        );
    }

    #[test]
    fn acquisition_confidence_is_measured_against_the_threshold_of_its_own_family() {
        use aether_phy::preamble::FrameType;
        use aether_phy::rx::FrameSync;

        // the two families are detected by different statistics against different
        // thresholds, so a raw number means something only next to its own: an OFDM frame's
        // matched-filter peak against the air's acquisition threshold, a tone frame's sync
        // statistic against the tone detector's (ADR-0013)
        let air = aether_phy::modes::air_interface(aether_phy::waveform::NARROW_500);
        let sync = FrameSync {
            start: 0,
            cfo_hz: 0.0,
            frame_type: FrameType::Control,
            timing_peak: air.acquisition_threshold,
            type_confidence: 1.0,
        };
        assert!((sync.detect_confidence(&air) - 1.0).abs() < 1e-9);
        let detector = aether_phy::tone::ToneDetector::new();
        let tone = aether_phy::ToneSync {
            start: 0,
            cfo_hz: 0.0,
            kind: aether_phy::tone::control_kind(),
            rv: 0,
            statistic: detector.threshold() * 2.0,
        };
        assert!((tone.detect_confidence() - 2.0).abs() < 1e-9);
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
        /// The harness's playback clock: blocks taken from the stations so far, across
        /// every `run` — a sound card's clock does not restart between calls either.
        played: u64,
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
                played: 0,
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

        /// Run station `a` alone, hearing silence — `b` has gone off the air — for `seconds`
        /// or until `done`, on the same sound-card clock.
        pub(crate) fn run_a_alone(
            &mut self,
            seconds: f64,
            mut done: impl FnMut(&Station<NullPtt>) -> bool,
        ) {
            let rate = WIDE_2300.audio_rate as f64;
            let blocks = (seconds * rate / self.block as f64) as usize;
            let mut out = vec![0.0f32; self.block];
            let silence = vec![0.0f32; self.block];
            for _ in 0..blocks {
                self.a.device_played(self.played);
                self.played += self.block as u64;
                self.a.playback(&mut out).expect("playback");
                self.a.capture(&silence).expect("capture");
                if done(&self.a) {
                    return;
                }
            }
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
                // the harness is the sound card: every block it takes has played by the
                // time it takes the next, which is the clock the key is released against
                self.a.device_played(self.played);
                self.b.device_played(self.played);
                self.played += self.block as u64;
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
    fn a_probe_is_answered_even_with_wait_for_clear_on() {
        // The probe a station decodes marks its own channel busy for two seconds; before the
        // fix that held the station's own answer for that long, longer than the prober waits,
        // so every probe to a station with `wait_for_clear` on reported "no answer" (the
        // OTA-2 test sessions: "probe: no answer; calling anyway"). A probe answer is a
        // response to a frame addressed to us and now goes out despite the busy channel.
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            wait_for_clear: true,
            ..config
        });
        // let both busy detectors learn the noise floor and settle
        air.run(4.0, |_, _| false);
        air.a.probe("KK4XYZ", None).expect("idle");
        air.run(30.0, |a, _| a.engine().last_probe().is_some());
        assert!(
            air.a.engine().last_probe().is_some(),
            "the probe went unanswered with wait_for_clear on; b answered {} probe(s)",
            air.b.engine().stats.probes_answered
        );
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
    fn the_key_is_released_when_the_sound_card_has_played_the_burst_not_when_the_queue_drains() {
        // The whole burst is handed over at once, so the station's queue drains long before
        // the audio has left the card; the release waits for the card's own clock to pass
        // the last sample handed over. ADR-0010.
        let mut station = idle_station();
        station.key_test(0.5).expect("idle");
        let mut out = vec![0.0f32; 4800];
        station.device_played(1_000_000); // the card has been running a while
        let mut handed = 0usize;
        loop {
            let count = station.playback(&mut out).expect("playback");
            if count == 0 {
                break;
            }
            handed += count;
        }
        assert!(
            station.transmitting(),
            "the queue drained but nothing has played yet"
        );
        assert!(handed > 0);
        // the card has played half of it: still keyed
        station.device_played(1_000_000 + handed as u64 / 2);
        assert_eq!(station.playback(&mut out).expect("playback"), 0);
        assert!(
            station.transmitting(),
            "released with half the burst still in the card"
        );
        // the card has played all of it: released now
        station.device_played(1_000_000 + handed as u64);
        assert_eq!(station.playback(&mut out).expect("playback"), 0);
        assert!(!station.transmitting(), "the burst has left the card");
        assert!(!station.take_device_flush(), "nothing was cut short");
    }

    /// Everything a station hands to the sound card for its next transmission.
    fn handed_audio(station: &mut Station<NullPtt>) -> Vec<f32> {
        let mut out = vec![0.0f32; 4800];
        let mut audio = Vec::new();
        loop {
            let count = station.playback(&mut out).expect("playback");
            if count == 0 {
                break;
            }
            audio.extend_from_slice(&out[..count]);
        }
        audio
    }

    #[test]
    fn a_block_that_outlasts_the_transmission_is_heard_past_its_end() {
        // A slow pass on a loaded machine brings in the end of this station's own tail and
        // the start of the peer's reply together. The reply is heard: the deafness ends
        // where the card's clock says the transmission did, not where the loop noticed.
        let rate = WIDE_2300.audio_rate;
        let mut caller = Station::new(
            StationConfig {
                callsign: "KK4XYZ".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            2,
        );
        caller.connect("W4ODA").expect("idle");
        let request = handed_audio(&mut caller);
        assert!(request.len() > rate / 2, "a connect request was rendered");

        // half a second past the tail, then the peer's request, then the room the receiver
        // needs after a frame — a quarter second — all in one block
        let quiet = rate / 2;
        let mut arrives = vec![0.0f32; quiet];
        arrives.extend_from_slice(&request);
        arrives.extend(std::iter::repeat_n(0.0f32, rate / 4));

        for (past_end, heard) in [(0usize, false), (arrives.len(), true)] {
            let mut station = idle_station();
            station.key_test(0.2).expect("idle");
            station.device_played(0);
            let handed = handed_audio(&mut station).len();
            assert!(station.transmitting(), "the burst is still in the card");
            // the block: the transmission's last `arrives.len() - past_end` samples, and then
            // what came in after it
            let mut block = vec![0.0f32; arrives.len() - past_end];
            block.extend_from_slice(&arrives[arrives.len() - past_end..]);
            station.device_played(handed as u64 + past_end as u64);
            station.capture(&block).expect("capture");
            assert_eq!(
                station.stats.frames_detected > 0,
                heard,
                "{past_end} samples of the block came in after the transmission"
            );
            assert!(
                !station.transmitting() || past_end == 0,
                "the transmission ended inside the block"
            );
        }
    }

    #[test]
    fn a_cut_tone_flushes_the_card_and_releases_at_once() {
        let mut station = idle_station();
        station.tune(5.0).expect("idle");
        let mut out = vec![0.0f32; 4800];
        station.device_played(0);
        // hand the card the whole tone, as the run loop does
        while station.playback(&mut out).expect("playback") > 0 {}
        assert!(station.transmitting());
        assert!(station.tune_stop(), "there was a tone to stop");
        assert!(
            station.take_device_flush(),
            "the card still held the tone: the loop must drop it"
        );
        // the card's clock has barely moved, and that no longer matters: the rest was dropped
        station.device_played(4800);
        assert_eq!(station.playback(&mut out).expect("playback"), 0);
        assert!(!station.transmitting(), "a cut tone releases at once");
    }

    #[test]
    fn the_burst_has_the_rms_the_tx_level_documents() {
        // a call's first try is a tone frame (ADR-0016), which goes out at the OFDM frames'
        // peak — `tone::gain_db()` above their average — and its second an OFDM frame, whose
        // RMS is what `tx_level` documents
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
        for _ in 0..1200 {
            let count = station.playback(&mut out).expect("playback");
            audio.extend_from_slice(&out[..count]);
            station.capture(&vec![0.0f32; 4096]).expect("capture");
            if station.stats.transmissions >= 2 && !station.transmitting() {
                break;
            }
        }
        // each burst proper, without the keying lead and tail: runs of sound apart by more
        // than a tenth of a second of silence
        let mut bursts: Vec<Vec<f32>> = Vec::new();
        let mut quiet = usize::MAX;
        for &x in &audio {
            if x.abs() > 1e-4 {
                if quiet > 4800 {
                    bursts.push(Vec::new());
                }
                bursts.last_mut().expect("started").push(x);
                quiet = 0;
            } else {
                quiet = quiet.saturating_add(1);
            }
        }
        assert_eq!(bursts.len(), 2, "two tries of the call");
        let rms = |b: &[f32]| (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
        let expected = 0.25 / std::f32::consts::SQRT_2;
        let tone = expected * 10f32.powf(aether_phy::tone::gain_db() as f32 / 20.0);
        let (floor, ordinary) = (rms(&bursts[0]), rms(&bursts[1]));
        assert!(
            (floor / tone - 1.0).abs() < 0.1,
            "tone burst RMS {floor:.4}; the OFDM frames' peak is {tone:.4}"
        );
        assert!(
            (ordinary / expected - 1.0).abs() < 0.1,
            "OFDM burst RMS {ordinary:.4}; tx_level / sqrt 2 is {expected:.4}"
        );
        for burst in &bursts {
            let peak = burst.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(
                peak < 0.7,
                "peak {peak} leaves no headroom at tx_level 0.25"
            );
        }
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
    fn a_burst_ends_inside_the_key_watchdog() {
        // ADR-0017: six tone frames are 32 s and the watchdog's limit 30 s; on the air it
        // cut the last frame of every full tone burst, and the link fell to the floor
        let station = |max_key_s: f64, cw_id: Option<CwId>| {
            Station::new(
                StationConfig {
                    callsign: "KK4ODA-1".to_owned(),
                    max_key_s,
                    cw_id,
                    playback_lead_s: 0.25,
                    ..StationConfig::default()
                },
                NullPtt::default(),
                1,
            )
        };
        let plain = station(30.0, None);
        let tone = plain.engine.timing().data_frame_s_for(0);
        let ofdm = plain
            .engine
            .timing()
            .data_frame_s_for(plain.engine.timing().floor_modes);
        assert_eq!(plain.engine.burst_capacity(tone), 5);
        assert!(
            5.0 * tone + plain.config.key_lead_s + plain.config.key_tail_s + KEY_TIME_MARGIN_S
                <= 30.0
        );
        assert_eq!(plain.engine.burst_capacity(ofdm), 6);
        // an identifier needs room on the bursts it rides on: the first transmission's, and
        // those shaped within a minute of the next falling due — and only those
        let mut identifying = station(30.0, Some(CwId::default()));
        assert_eq!(identifying.engine.burst_capacity(tone), 4);
        identifying.identifier.last = Some(0.0);
        identifying.refresh_burst_cap(1.0);
        assert_eq!(identifying.engine.burst_capacity(tone), 5);
        let interval = identifying.config.cw_id_interval_s;
        identifying.refresh_burst_cap(interval - ID_LOOKAHEAD_S - 1.0);
        assert_eq!(identifying.engine.burst_capacity(tone), 5);
        identifying.refresh_burst_cap(interval - ID_LOOKAHEAD_S + 1.0);
        assert_eq!(identifying.engine.burst_capacity(tone), 4);
        // the end of a session is identified at once, and its last bursts keep room for it
        identifying.identifier.final_due = true;
        identifying.refresh_burst_cap(2.0);
        assert_eq!(identifying.engine.burst_capacity(tone), 4);
        // and the limit follows the setting, live
        let mut longer = station(30.0, None);
        let mut config =
            crate::config::Config::parse("callsign = \"KK4ODA-1\"\n[radio]\nmax_key_s = 60.0\n")
                .expect("parse");
        config.radio.max_key_s = 60.0;
        longer.apply_live(&config);
        assert_eq!(longer.engine.burst_capacity(tone), 6);
    }

    /// Run one station alone, hearing silence, for `seconds` or until `done`.
    fn run_alone(
        station: &mut Station<NullPtt>,
        seconds: f64,
        mut done: impl FnMut(&Station<NullPtt>) -> bool,
    ) {
        let mut out = vec![0.0f32; 4096];
        let silence = vec![0.0f32; 4096];
        let blocks = (seconds * WIDE_2300.audio_rate as f64 / 4096.0) as usize;
        let mut played = 0u64;
        for _ in 0..blocks {
            station.device_played(played);
            played += 4096;
            station.playback(&mut out).expect("playback");
            station.capture(&silence).expect("capture");
            if done(station) {
                return;
            }
        }
    }

    #[test]
    fn an_abort_drops_the_rest_of_the_burst() {
        // a long tone burst ran on after the operator's abort until the key watchdog cut it
        // (ND1J, 2026-09-25): the session is over, and so is its burst
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.connect("KK4XYZ").expect("idle");
        run_alone(&mut station, 1.0, |_| false);
        assert!(station.transmitting(), "the call is on the air");
        let aborted_at = station.now();
        station.abort();
        run_alone(&mut station, 5.0, |s| !s.transmitting());
        assert!(!station.transmitting());
        assert!(
            station.now() - aborted_at < 0.3,
            "the key stayed down {:.1} s after the abort",
            station.now() - aborted_at
        );
        assert_eq!(station.stats.watchdog_trips, 0);
    }

    #[test]
    fn a_sent_message_is_reported_once_the_other_station_has_it() {
        // the panel's check mark: a message sent with a reference is reported delivered when
        // the other station has all of it in order, and undelivered when the session ends
        // first — or at once, with no session to send on
        let delivery =
            |reference: &str, bytes: usize, delivered: bool, reason: Option<&str>| Delivery {
                reference: reference.to_owned(),
                bytes,
                delivered,
                reason: reason.map(str::to_owned),
            };
        let mut air = Air::new(1.0, 0.0005);
        air.a.send_tracked(b"too early\n", "m0");
        assert_eq!(
            air.a.take_deliveries(),
            [delivery("m0", 10, false, Some("no session"))]
        );
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert_eq!(
            air.a.take_received(),
            b"",
            "nothing sent before the session is queued"
        );

        air.a.send_tracked(b"first\n", "m1");
        air.a.send_tracked(&[0x42; 600], "m2");
        assert_eq!(air.a.delivery_status().0, ["m1", "m2"]);
        assert!(
            air.a.take_deliveries().is_empty(),
            "nothing has arrived yet"
        );
        air.run(120.0, |a, _| a.delivery_status().0.is_empty());
        assert_eq!(
            air.a.take_deliveries(),
            [
                delivery("m1", 6, true, None),
                delivery("m2", 600, true, None)
            ]
        );
        let got = air.b.take_received();
        assert!(got.starts_with(b"first\n") && got.len() == 606);

        // a long message the operator aborts: it never all arrives. Incompressible — the
        // session compresses, and 20 kB of one byte would cross in a frame
        let mut state = 0x2545_f491_u32;
        let noise: Vec<u8> = (0..20_000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect();
        air.a.send_tracked(&noise, "m3");
        air.run(5.0, |_, _| false);
        air.a.abort();
        assert_eq!(
            air.a.take_deliveries(),
            [delivery("m3", 20_000, false, Some("aborted"))]
        );
        let (pending, recent) = air.a.delivery_status();
        assert!(pending.is_empty());
        let names: Vec<&str> = recent.iter().map(|d| d.reference.as_str()).collect();
        assert_eq!(names, ["m0", "m1", "m2", "m3"]);
    }

    // ── the regulatory gate (ADR-0018) ───────────────────────────────────

    /// United States rules for a station on a declared dial (a `NullPtt` cannot report one).
    fn under_us_rules(
        dial_hz: u64,
        control: crate::regulatory::ControlMode,
    ) -> crate::regulatory::Settings {
        crate::regulatory::Settings {
            profile: "us-fcc-part97".to_owned(),
            control: Some(control),
            license: Some(crate::regulatory::LicenseClass::General),
            sideband: Some(crate::regulatory::Sideband::Usb),
            itu_region: 2,
            margin_hz: 50.0,
            band_plan: false,
            dial_hz: Some(dial_hz),
            log_permitted: true,
        }
    }

    fn lone_station(regulatory: crate::regulatory::Settings) -> Station<NullPtt> {
        Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                regulatory,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        )
    }

    #[test]
    fn a_transmission_the_rules_refuse_is_never_keyed() {
        use crate::regulatory::ControlMode;
        // a call from the 20 m data segment goes out
        let mut lawful = lone_station(under_us_rules(14_078_000, ControlMode::Local));
        lawful.connect("KK4XYZ").expect("idle");
        run_alone(&mut lawful, 2.0, Station::transmitting);
        assert!(lawful.transmitting(), "a lawful call is keyed");

        // the same call from the 20 m phone segment does not, and the attempt ends
        let mut unlawful = lone_station(under_us_rules(14_200_000, ControlMode::Local));
        assert_eq!(
            unlawful.check_originate().expect_err("refused").code,
            "no_data_here"
        );
        unlawful.connect("KK4XYZ").expect("the engine takes it");
        run_alone(&mut unlawful, 10.0, |_| false);
        assert_eq!(unlawful.stats.transmissions, 0, "never keyed");
        assert_eq!(unlawful.state(), State::Idle, "the attempt is over");
        let reports = unlawful.take_regulatory_reports();
        assert!(
            reports
                .iter()
                .any(|d| d.code == "no_data_here" && d.what == "a call"),
            "{reports:?}"
        );

        // nothing is guessed: no profile chosen, nothing keyed
        let mut unset = lone_station(crate::regulatory::Settings {
            profile: String::new(),
            ..under_us_rules(14_078_000, ControlMode::Local)
        });
        unset.connect("KK4XYZ").expect("the engine takes it");
        run_alone(&mut unset, 5.0, |_| false);
        unset.beacon().expect("queued");
        run_alone(&mut unset, 5.0, |_| false);
        assert_eq!(unset.stats.transmissions, 0);
        assert!(
            unset
                .take_regulatory_reports()
                .iter()
                .all(|d| d.code == "no_profile")
        );
    }

    #[test]
    fn the_keying_path_refuses_audio_the_gate_did_not_see() {
        // audio that reached the playback queue by any path but the gate is dropped, not
        // keyed: leave to key is made only by the policy
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        station.playback.extend(std::iter::repeat_n(0.5f32, 48_000));
        let mut out = vec![0.0f32; 4096];
        station.playback(&mut out).expect("playback");
        assert!(!station.transmitting());
        assert_eq!(station.stats.transmissions, 0);
        assert!(station.playback.is_empty());
        assert!(
            station
                .take_events()
                .iter()
                .any(|e| e.starts_with("error:regulatory"))
        );
    }

    #[test]
    fn every_way_to_the_transmitter_passes_the_gate() {
        // a call, a beacon, a probe, a tune tone, a keying test, drive bursts, a Morse
        // identifier and a KISS client's datagram: with no profile chosen, not one of them
        // keys the radio
        let mut station = lone_station(crate::regulatory::Settings {
            profile: String::new(),
            ..crate::regulatory::Settings::unchecked()
        });
        station.connect("KK4XYZ").expect("queued");
        run_alone(&mut station, 6.0, |_| false);
        station.beacon().expect("queued");
        run_alone(&mut station, 3.0, |_| false);
        station.probe("KK4XYZ", None).expect("queued");
        run_alone(&mut station, 3.0, |_| false);
        station.tune(1.0).expect("queued");
        run_alone(&mut station, 3.0, |_| false);
        station.key_test(1.0).expect("queued");
        run_alone(&mut station, 3.0, |_| false);
        station.set_drive(1).expect("queued");
        run_alone(&mut station, 6.0, |_| false);
        station.pending.push_back(Outgoing::Identifier);
        run_alone(&mut station, 3.0, |_| false);
        station
            .send_datagram(datagram(vec![0x82; 20], None))
            .expect("queued");
        run_alone(&mut station, 6.0, |_| false);
        assert_eq!(station.stats.transmissions, 0, "nothing was keyed");
        let whats: Vec<String> = station
            .take_regulatory_reports()
            .into_iter()
            .map(|d| d.what)
            .collect();
        for what in [
            "a call",
            "a beacon",
            "a probe",
            "the tune tone",
            "a keying test",
            "a drive burst",
            "the Morse identifier",
            "a KISS client's datagram",
        ] {
            assert!(
                whats.iter().any(|w| w == what),
                "{what} not judged: {whats:?}"
            );
        }
    }

    #[test]
    fn an_identifier_inside_a_burst_is_judged_as_the_cw_it_is() {
        // the Morse identifier rides inside a data burst's keying, after the data the gate
        // judged: it is judged again as CW before it is appended, so a profile that allowed
        // the data but not CW there keeps it out of the burst
        use crate::regulatory::{ControlMode, Policy};
        let identifying = |regulatory| StationConfig {
            callsign: "W4ODA".to_owned(),
            wait_for_clear: false,
            cw_id: Some(CwId::default()),
            regulatory,
            ..StationConfig::default()
        };
        let mut lawful = Station::new(
            identifying(under_us_rules(14_078_000, ControlMode::Local)),
            NullPtt::default(),
            1,
        );
        lawful.connect("KK4XYZ").expect("idle");
        run_alone(&mut lawful, 3.0, |s| s.stats.cw_ids > 0);
        assert_eq!(lawful.stats.cw_ids, 1, "the call carries the identifier");

        let mut profile =
            crate::regulatory::profile::load("us-fcc-part97").expect("the profile reads");
        for privilege in &mut profile.privileges.general {
            if privilege.range.holds(14_078_000.0, 14_078_000.0) {
                privilege.emissions = Some(vec!["data".to_owned()]);
            }
        }
        let mut station = Station::new(
            identifying(under_us_rules(14_078_000, ControlMode::Local)),
            NullPtt::default(),
            1,
        );
        station.set_policy(Policy::Rules(std::sync::Arc::new(profile)));
        station.connect("KK4XYZ").expect("idle");
        run_alone(&mut station, 3.0, Station::transmitting);
        assert!(
            station.transmitting(),
            "the call itself is lawful and keyed"
        );
        assert_eq!(station.stats.cw_ids, 0, "but it carries no identifier");
        assert!(
            station
                .take_regulatory_reports()
                .iter()
                .any(|d| d.what == "the Morse identifier" && !d.allowed()),
            "and the refusal is reported"
        );
    }

    #[test]
    fn a_session_whose_dial_moves_out_of_the_data_segment_stops_transmitting() {
        use crate::regulatory::ControlMode;
        let rules = |dial| under_us_rules(dial, ControlMode::Local);
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            regulatory: rules(14_078_000),
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected());
        let mut state = 0x9e37_79b9_u32;
        let noise: Vec<u8> = (0..6000)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect();
        air.a.send(&noise);
        air.run(8.0, |a, _| a.transmitting());
        // the operator turns the dial to the phone segment in the middle of it
        air.a.set_regulatory(rules(14_200_000));
        air.run(20.0, |a, _| !a.transmitting());
        let keyed_before = air.a.stats.transmissions;
        air.run(90.0, |a, _| a.state() == State::Idle && a.quiescent());
        assert_eq!(air.a.state(), State::Idle, "the session is halted");
        assert_eq!(
            air.a.stats.transmissions, keyed_before,
            "and nothing more is keyed"
        );
        let reports = air.a.take_regulatory_reports();
        assert!(
            reports.iter().any(|d| d.code == "no_data_here"),
            "{reports:?}"
        );
        // its disconnect would itself be unlawful there, so it was not sent either
        assert!(
            !air.b
                .take_events()
                .iter()
                .any(|e| e == "disconnected:peer disconnected")
        );
    }

    #[test]
    fn an_automatic_station_answers_only_where_the_rules_let_it() {
        use crate::regulatory::ControlMode;
        let setup = |b_dial: u64| {
            Air::with(1.0, 0.0005, move |config| StationConfig {
                regulatory: if config.callsign == "KK4XYZ" {
                    under_us_rules(b_dial, ControlMode::Automatic)
                } else {
                    under_us_rules(b_dial, ControlMode::Local)
                },
                ..config
            })
        };
        // inside §97.221(b) (14.1005–14.112 MHz) it answers, and may call too
        let mut inside = setup(14_103_000);
        assert!(inside.b.check_originate().is_ok());
        inside.a.connect("KK4XYZ").expect("idle");
        inside.run(60.0, |a, b| a.connected() && b.connected());
        assert!(inside.b.connected());

        // outside them it may not call; and no Aether emission is 500 Hz or less under the
        // conservative reading of §97.3(a)(8), so it does not answer either
        let mut outside = setup(14_080_000);
        let refusal = outside.b.check_originate().expect_err("refused");
        assert!(refusal.code.starts_with("automatic_"), "{}", refusal.code);
        outside.a.connect("KK4XYZ").expect("idle");
        outside.run(30.0, |_, _| false);
        assert!(!outside.a.connected());
        assert_eq!(
            outside.b.stats.transmissions, 0,
            "the automatic station never keyed"
        );
        assert!(
            outside
                .b
                .take_regulatory_reports()
                .iter()
                .any(|d| d.code == "automatic_bandwidth")
        );
    }

    #[test]
    fn an_automatic_answer_never_climbs_past_500_hz_however_good_the_path() {
        use crate::regulatory::ControlMode;
        // the rules read as a ratio of powers, under which the tone floor's rungs are 500 Hz
        // or less: the automatic station answers outside §97.221(b) and stays on them
        let mut profile = crate::regulatory::profile::load("us-fcc-part97").expect("loads");
        profile.bandwidth.reading = crate::regulatory::Reading::Power;
        let policy = crate::regulatory::Policy::Rules(std::sync::Arc::new(profile));
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            regulatory: if config.callsign == "KK4XYZ" {
                under_us_rules(14_080_000, ControlMode::Automatic)
            } else {
                under_us_rules(14_080_000, ControlMode::Local)
            },
            ..config
        });
        air.b.set_policy(policy);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(90.0, |a, b| a.connected() && b.connected());
        assert!(air.b.connected(), "answered under §97.221(c)");
        air.b.send(&[0x55; 400]);
        air.a.send(&[0x33; 400]);
        air.run(400.0, |_, b| {
            b.engine.all_acknowledged() && b.engine.stats.bytes_acked > 0
        });
        assert!(
            air.b.engine.stats.bytes_acked > 0,
            "the automatic station's data crossed"
        );
        assert_eq!(
            air.b.engine.ceiling(),
            Some(1),
            "the ceiling: the tone floor's two rungs"
        );
        let permitted = air.b.take_regulatory_reports();
        assert!(!permitted.is_empty());
        for d in &permitted {
            assert!(d.allowed(), "{d:?}");
            assert_eq!(d.code, "automatic_response", "{}", d.what);
            assert!(d.bandwidth_hz <= 500.0, "{}: {} Hz", d.what, d.bandwidth_hz);
        }
    }

    #[test]
    fn a_session_that_ends_is_kept_for_the_history() {
        // who called whom, what crossed, the fastest rung each way and how it ended: what
        // three test sessions with ND1J left to be read out of recordings (2026-09-25)
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 300]);
        air.run(120.0, |a, _| {
            a.engine.all_acknowledged() && a.engine.stats.bytes_acked > 0
        });
        assert_eq!(air.b.take_received().len(), 300);
        assert!(
            air.a.take_finished_sessions().is_empty(),
            "nothing has ended yet"
        );
        air.a.disconnect();
        air.run(120.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle
        });

        let caller = air.a.take_finished_sessions();
        let called = air.b.take_finished_sessions();
        assert_eq!((caller.len(), called.len()), (1, 1));
        let (a, b) = (&caller[0], &called[0]);
        assert_eq!(
            (a.remote.as_str(), a.role),
            ("KK4XYZ", crate::sessions::Role::Caller)
        );
        assert_eq!(
            (b.remote.as_str(), b.role),
            ("W4ODA", crate::sessions::Role::Called)
        );
        assert_eq!(
            (a.end.as_str(), b.end.as_str()),
            ("closed", "peer disconnected")
        );
        assert_eq!((a.bytes_sent, b.bytes_received), (300, 300));
        assert!(a.bytes_acked > 0 && b.bytes_acked == 0);
        assert!(a.duration_s > 0.0 && (a.duration_s - b.duration_s).abs() < 10.0);
        assert_eq!(a.bandwidth_hz, 2300);
        // the data went at some rung and was heard at that rung; the SNRs were read while
        // the session was up — the engine forgets them the moment it ends
        let sent = a.top_rung_sent.expect("data was sent");
        assert_eq!(b.top_rung_heard, Some(sent));
        assert_eq!(
            a.top_rung_heard, None,
            "nothing but acknowledgements came back"
        );
        assert!(a.heard_there_db.is_some() && b.snr_db.is_some() && a.snr_db.is_some());
        assert!(!a.test && a.recording.is_none());
        assert!(air.a.take_finished_sessions().is_empty(), "taken once");
    }

    #[test]
    fn a_session_ends_with_an_identifier_however_it_ends() {
        // §97.119: the end of each communication is identified. An identifier rode only on a
        // transmission that fell due by the interval, so a session that ended in silence —
        // the peer's application closed, the link timed out — ended without one
        let identifying = |config: StationConfig| StationConfig {
            cw_id: (config.callsign == "W4ODA").then(CwId::default),
            ..config
        };

        // an orderly close: the first transmission identifies, the end once more — twice in
        // all, however long the session ran inside the interval
        let mut air = Air::with(1.0, 0.0005, identifying);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 300]);
        air.a.disconnect();
        air.run(120.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle && a.quiescent()
        });
        air.run(10.0, |_, _| false);
        assert_eq!(air.a.stats.cw_ids, 2, "one to start and one to end");
        assert_eq!(air.b.stats.cw_ids, 0, "the other station does not identify");

        // the peer goes silent mid-session: the link times out, and the identifier goes
        // out on its own
        let mut air = Air::with(1.0, 0.0005, identifying);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 3000]);
        air.run(3.0, |_, _| false);
        let before = air.a.stats.cw_ids;
        air.run_a_alone(400.0, |a| a.state() == State::Idle && a.quiescent());
        assert_eq!(air.a.state(), State::Idle, "the link timed out");
        assert_eq!(air.a.stats.cw_ids, before + 1, "the end was identified");

        // an abort mid-burst: the burst is cut, the DISC carries the identifier, and the
        // peer hears the DISC rather than timing out
        let mut air = Air::with(1.0, 0.0005, identifying);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 3000]);
        air.run(20.0, |a, _| a.transmitting());
        let before = air.a.stats.cw_ids;
        air.a.abort();
        air.run(30.0, |a, b| b.state() == State::Idle && a.quiescent());
        assert_eq!(air.b.state(), State::Idle);
        assert!(
            air.b
                .take_events()
                .iter()
                .any(|e| e == "disconnected:peer disconnected"),
            "the peer heard the DISC"
        );
        assert_eq!(air.a.stats.cw_ids, before + 1);
        assert_eq!(air.a.stats.watchdog_trips, 0);
    }

    #[test]
    fn an_identifier_owed_but_impossible_is_dropped_not_queued_for_ever() {
        // the operator turns identification off after a session that owed one: nothing is
        // keyed, and the station does not queue the identifier again on every pass
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
        station.identifier.final_due = true;
        station.config.cw_id = None;
        run_alone(&mut station, 2.0, |_| false);
        assert!(station.quiescent());
        assert!(!station.identifier.final_due);
        assert_eq!(station.stats.cw_ids, 0);
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
        // a beacon is a tone frame (ADR-0016), whose SNR reading tops out near +17 dB however
        // clean the path — an OFDM frame's reads past +20 on a wire, checked below — and which
        // has no constellation to show
        assert!(beacon.snr_db > 15.0, "a wire reads {} dB", beacon.snr_db);
        assert!(
            beacon.cfo_hz.abs() < 5.0,
            "a wire has no offset: {}",
            beacon.cfo_hz
        );
        assert!(beacon.confidence >= 1.0);
        let (last, points) = air.b.constellation().expect("a frame was heard");
        assert_eq!(last, beacon);
        assert!(points.is_empty(), "a tone frame has no constellation");
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
        // the acknowledgements of OFDM bursts are OFDM frames at the control mode, as beacons
        // were before ADR-0016: a wire reads past +20 dB, and the last one's symbols sit on
        // their constellation — none near the origin
        assert!(acks.iter().all(|r| r.snr_db > 20.0), "{acks:?}");
        let (last, points) = air.a.constellation().expect("a frame was heard");
        assert_eq!(last.kind, "control");
        assert!(!points.is_empty() && points.len() <= CONSTELLATION_POINTS);
        assert!(points.iter().all(|(i, q)| i.hypot(*q) > 0.2));
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
        // fifteen rungs: the tone floor's two, its four-tone middle kinds (ADR-0015), and the
        // OFDM modes from QPSK 1/3
        assert_eq!(air.a.engine().timing().data_capacity.len(), 15);
        assert_eq!(air.a.engine().timing().mode_threshold_db.len(), 15);
        assert_eq!(air.a.engine().timing().floor_modes, 4);
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
    fn two_narrow_stations_connect_and_carry_a_message_on_the_floor() {
        // −9 dB, 3 kHz reference: below every ordinary narrow mode (the connect mode stops at
        // −5.2), inside the floor family's range (ADR-0009). A floor frame is 2.2–4.2 s and
        // the receiver searches a fraction of that per block, so this is the live receiver's
        // test as much as the protocol's: the family decoded offline and never through the
        // streaming receiver until it learned to take a floor frame as final while the frame
        // was still arriving (ADR-0009 §8), and nothing connected on the air below −5 dB.
        let narrow = |config: StationConfig| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        };
        // the waveform's RMS is tx_level/√2 and the noise is white across 24 kHz, so the
        // noise in 3 kHz is an eighth of its variance
        let gain = 0.1_f32;
        let signal_power = (0.25 * f64::from(gain)).powi(2) / 2.0;
        let snr_db = -9.0_f64;
        let sigma = (8.0 * signal_power / 10f64.powf(snr_db / 10.0)).sqrt() as f32;
        let mut air = Air::with(gain, sigma, narrow);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(150.0, |a, b| a.connected() && b.connected());
        let reports = air.b.take_frame_reports();
        assert!(
            air.a.connected() && air.b.connected(),
            "no session at {snr_db} dB: {reports:?}"
        );
        let message = b"On the floor at -9 dB.";
        air.a.send(message);
        air.run(150.0, |_, b| b.received_len() >= message.len());
        let mut reports = reports;
        reports.extend(air.b.take_frame_reports());
        assert_eq!(air.b.take_received(), message, "{reports:?}");
        // what carried it was the floor: its modes are the table's first two
        let floor_modes = air.a.engine().timing().floor_modes;
        assert!(
            reports
                .iter()
                .any(|r| r.kind == "data" && r.decoded && r.mode < floor_modes),
            "{reports:?}"
        );
        // and the channel was as stated: every decoded frame measured within a few decibels
        let measured: Vec<f64> = reports
            .iter()
            .filter(|r| r.decoded)
            .map(|r| r.snr_db)
            .collect();
        assert!(
            measured.iter().all(|&snr| (snr - snr_db).abs() < 3.0),
            "{measured:?}"
        );
    }

    #[test]
    fn a_wide_station_does_not_answer_a_narrow_call() {
        // the two OFDM waveforms do not decode each other's preambles: a 500 Hz call's
        // ordinary tries are silence at a 2 300 Hz station. The tone floor (ADR-0013) is the
        // same frames on both airs, so its tries on the floor reach the wide station — which
        // leaves them alone, because the call states its bandwidth
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
        air.run(90.0, |_, b| {
            b.events
                .iter()
                .any(|e| e.contains("calls in another bandwidth"))
        });
        assert!(!air.b.connected() && !air.a.connected());
        let events = air.b.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("ignored:W4ODA calls in another bandwidth")),
            "{events:?}"
        );
        // and whatever the wide station heard was the floor's, never the narrow OFDM
        let floor = air.b.engine().timing().floor_modes;
        let reports = air.b.take_frame_reports();
        assert!(!reports.is_empty());
        assert!(
            reports.iter().all(|r| r.mode < floor && r.decoded),
            "{reports:?}"
        );
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
        for i in 0..80 {
            station.capture(&quiet_block(i)).expect("capture");
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
    fn every_busy_transition_is_logged_with_the_numbers_that_decided_it() {
        // The instrumentation: when the indicator lights, the log says which path did it
        // and what the level, floor and threshold were at that moment — so the next report
        // of "busy with nothing above the threshold" names its cause instead of guessing.
        let mut station = idle_station();
        for i in 0..120 {
            station.capture(&quiet_block(i)).expect("capture");
        }
        let _ = station.take_events();
        assert!(!station.channel_busy(), "quiet to begin with");

        // +32 dB of the same noise: well over the threshold, for well over the attack
        for i in 0..6 {
            let loud: Vec<f32> = quiet_block(500 + i).iter().map(|x| x * 40.0).collect();
            station.capture(&loud).expect("capture");
        }
        assert!(
            station.channel_busy(),
            "a level far over the floor lights it"
        );
        let events = station.take_events();
        let on = events
            .iter()
            .find(|e| e.starts_with("busy:") && e.contains("OFF -> ON"))
            .expect("the transition to busy is logged");
        assert!(
            on.contains("OFF -> ON") && on.contains("margin 6.0") && on.contains("delta +"),
            "the log carries the level, floor, delta, margin and direction: {on}"
        );
        assert!(
            matches!(
                station.busy_detector().reason(),
                Some(crate::busy::BusyReason::Level { .. })
            ),
            "and the reason is the level"
        );

        // and back to quiet: the clearing is logged too, once the hang has run out
        for i in 0..40 {
            station.capture(&quiet_block(900 + i)).expect("capture");
        }
        let events = station.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("busy:") && e.contains("ON -> OFF")),
            "the clearing is logged: {events:?}"
        );
        assert!(!station.channel_busy());
    }

    #[test]
    fn no_acquisition_lights_the_busy_indicator_on_its_own_only_a_decode_does() {
        // The operator watched the level never reach 6 dB over the floor while the busy
        // indicator stayed on for seconds at a time. It was the acquisition path: every
        // preamble marked the channel busy for two seconds, and on a crowded band the
        // detector finds a phantom every few seconds. Gating on acquisition confidence
        // was tried and measured: phantoms reach 1.54 and a real frame acquired at 1.38,
        // so there is no cut. Only a decode is evidence, so a preamble marks nothing.
        let mut station = idle_station();
        let quiet = vec![0.0001f32; 4096];
        for _ in 0..80 {
            station.capture(&quiet).expect("capture");
        }
        let now = station.now();
        let preamble = |confidence: f64| aether_phy::PendingFrame {
            start: 0,
            end: 1000,
            detect_confidence: confidence,
            tone: false,
            control: true,
        };
        assert!(!station.channel_busy(), "quiet to begin with");

        // just over the threshold: what noise produces
        let phantom = preamble(1.05);
        station.heed_preambles(&[phantom], now);
        assert!(
            !station.channel_busy(),
            "a phantom acquisition must not silence the station"
        );

        // well clear of it, as a frame would be — still nothing: acquisition is not evidence
        let confident = preamble(2.5);
        station.heed_preambles(&[confident], now);
        assert!(
            !station.channel_busy(),
            "an acquisition alone must not mark the channel, however confident"
        );
        assert!(
            station.receiving(),
            "but the receive indicator does follow a confident one"
        );

        // a decoded frame is the evidence, and it does
        station.busy.mark_frame(now, 1.38);
        assert!(
            station.channel_busy(),
            "a decoded frame marks the channel busy"
        );
        assert!(matches!(
            station.busy.reason(),
            Some(crate::busy::BusyReason::Frame { .. })
        ));
    }

    #[test]
    fn a_noise_candidate_that_fails_to_decode_does_not_light_the_receive_lamp() {
        // On an empty frequency the detector tries a candidate every few seconds and every
        // one of them ends undecoded; the half second the lamp is kept lit after a frame,
        // so a burst's next frame keeps it on, lit it for each of them — 26 blinks in a
        // 71 s recording of a quiet band, none acquired above 1.16. The lamp takes the
        // preamble path's gate: a decode, or a confident acquisition.
        use aether_phy::preamble::FrameType;
        use aether_phy::rx::FrameSync;

        let mut station = idle_station();
        let quiet = vec![0.0001f32; 4096];
        for _ in 0..80 {
            station.capture(&quiet).expect("capture");
        }
        let now = station.now();
        let air = station.air();
        let frame = |peak: f64, payload: Option<Vec<u8>>| aether_phy::DecodedFrame {
            payload,
            frame: aether_phy::Received::Ofdm(aether_phy::rx::ReceivedFrame {
                sync: FrameSync {
                    start: 0,
                    cfo_hz: 0.0,
                    frame_type: FrameType::Control,
                    timing_peak: peak,
                    type_confidence: 1.0,
                },
                symbols: Vec::new(),
                noise_var: Vec::new(),
                snr_carrier_db: -5.0,
                snr_3k_db: -12.0,
                cfo_hz: 0.0,
                mode: 0,
                rv: 0,
                chip_runner_up: 1,
                mode_confidence: 1.0,
            }),
        };
        assert!(!station.receiving(), "dark to begin with");

        // what noise produces: just over the threshold, and no payload
        station.report(&frame(air.acquisition_threshold * 1.05, None), 0, now);
        assert!(
            !station.receiving(),
            "a candidate that neither decoded nor acquired confidently lit the lamp"
        );

        // a frame that acquired well clear of the threshold is a burst even undecoded
        station.report(&frame(air.acquisition_threshold * 1.5, None), 0, now);
        assert!(
            station.receiving(),
            "a confident acquisition keeps the lamp lit"
        );

        // and a decode is a burst whatever its acquisition looked like
        for _ in 0..12 {
            station.capture(&quiet).expect("capture");
        }
        assert!(
            !station.receiving(),
            "the half second has run out a second later"
        );
        let later = station.now();
        station.report(
            &frame(air.acquisition_threshold * 1.05, Some(vec![0u8; 7])),
            0,
            later,
        );
        assert!(station.receiving(), "a decoded frame lights the lamp");
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
        station.busy.mark_frame(now, 3.0);
        assert!(station.channel_busy());
        assert!(
            !station.channel_clear(now),
            "an idle station must wait for a busy channel"
        );

        station.connect("KK4XYZ").expect("idle");
        // pretend the handshake completed: the engine is what decides this in a real session
        assert!(!station.channel_clear(station.now()), "still only calling");
    }

    // ── datagrams for KISS clients (ADR-0019) ────────────────────────────────

    fn datagram(frame: Vec<u8>, reference: Option<&str>) -> DatagramRequest {
        DatagramRequest {
            frame_type: 0,
            frame,
            reference: reference.map(str::to_owned),
            rung: Some(1),
            wait_for_clear: false,
            persistence: 1.0,
            slot_s: 0.1,
        }
    }

    #[test]
    fn a_kiss_frame_goes_out_as_bursts_that_fit_the_key() {
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        let queued = station
            .send_datagram(datagram(vec![0x42; 300], Some("r1")))
            .expect("queued");
        // 300 bytes, the sender's callsign and the type, at tone-36's 33 a frame: ten
        // fragments, and more than one keying — six tone frames would outrun the key
        assert_eq!((queued.fragments, queued.rung), (10, 1));
        assert!(queued.bursts >= 2);
        run_alone(&mut station, 200.0, |s| {
            s.datagram_status()["sent"] == 1 && !s.transmitting()
        });
        assert_eq!(station.stats.transmissions, queued.bursts);
        assert_eq!(
            station.take_datagram_reports(),
            [DatagramReport {
                reference: "r1".to_owned(),
                sent: true,
                reason: None
            }]
        );
    }

    #[test]
    fn a_kiss_frame_waits_while_a_session_is_up() {
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        station.connect("KK4XYZ").expect("idle");
        station
            .send_datagram(datagram(vec![1; 20], None))
            .expect("queued");
        run_alone(&mut station, 12.0, |_| false);
        assert_eq!(station.datagram_status()["queued"], 1, "held while calling");
        station.abort();
        run_alone(&mut station, 120.0, |s| {
            s.datagram_status()["sent"] == 1 && !s.transmitting()
        });
        assert_eq!(
            station.datagram_status()["sent"],
            1,
            "sent once the station is idle"
        );
    }

    #[test]
    fn a_kiss_frame_that_cannot_go_is_refused_at_once() {
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        assert!(matches!(
            station.send_datagram(datagram(vec![0; 2000], None)),
            Err(DatagramRefusal::Invalid(_))
        ));
        for _ in 0..DATAGRAM_QUEUE {
            station
                .send_datagram(datagram(vec![0; 10], None))
                .expect("room");
        }
        assert_eq!(
            station.send_datagram(datagram(vec![0; 10], None)),
            Err(DatagramRefusal::QueueFull)
        );
        let mut answering = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                answer_only: true,
                regulatory: crate::regulatory::Settings::unchecked(),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        assert!(matches!(
            answering.send_datagram(datagram(vec![0; 10], None)),
            Err(DatagramRefusal::NotAllowed(_))
        ));
    }

    #[test]
    fn a_kiss_frame_the_rules_refuse_is_reported_and_never_keyed() {
        let mut station = lone_station(crate::regulatory::Settings {
            profile: String::new(),
            ..crate::regulatory::Settings::unchecked()
        });
        station
            .send_datagram(datagram(vec![7; 100], Some("r2")))
            .expect("queued");
        run_alone(&mut station, 20.0, |_| false);
        assert_eq!(station.stats.transmissions, 0);
        let reports = station.take_datagram_reports();
        assert_eq!(reports.len(), 1);
        assert!(!reports[0].sent && reports[0].reference == "r2");
        assert!(
            station
                .take_regulatory_reports()
                .iter()
                .any(|d| d.what == "a KISS client's datagram")
        );
    }

    #[test]
    fn a_datagram_heard_in_pieces_is_joined_and_names_its_sender() {
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        let frame = vec![0x5A; 70];
        let carried = aether_link::datagram::body("KK4XYZ", 1, &frame).expect("body");
        let pieces = aether_link::datagram::fragments(&carried, 7, 36).expect("fits");
        assert!(pieces.len() > 1);
        let named: Vec<Option<String>> = pieces
            .iter()
            .rev()
            .map(|piece| station.heard_datagram_piece(piece, 3.5, 1, 10.0))
            .collect();
        // the first piece carries the callsign; whichever order they come in
        assert_eq!(named.last().cloned().flatten().as_deref(), Some("KK4XYZ"));
        assert_eq!(
            station.take_received_datagrams(),
            [ReceivedDatagram {
                source: "KK4XYZ".to_owned(),
                frame_type: 1,
                frame,
                snr_db: 3.5,
                rung: 1,
            }]
        );
    }
}
