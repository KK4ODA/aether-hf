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

mod bandwidth;
mod beacons;
mod datagrams;
mod fieldtest;
mod gaps;
mod host;
mod latency;
mod reserve;
pub use bandwidth::{
    KNOWN_LIMIT, Mismatch, RETURN_QUIET_S, Why as BandwidthWhy, base_callsign, params_for,
};
pub use beacons::{BEACON_EVERY_MAX_S, BEACON_EVERY_MIN_S};
pub use datagrams::{
    DATAGRAM_QUEUE, DatagramQueued, DatagramRefusal, DatagramReport, DatagramRequest,
    ReceivedDatagram,
};
pub use fieldtest::{Rung, Step, TestPlan, TestRun, Transfer};
pub use gaps::Learned as LearnedGap;
pub use host::HostPresence;

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use aether_link::{
    Action, Container, GrantConfig, GrantFrame, GrantedEngine, HarqBuffer, PhyTiming, SoftFrame,
    State,
    frames::{
        ConnectBody, DataHeader, DataKind, ProbeBody, decode_data, encode_data, pack_callsign,
        unpack_callsign, with_bandwidth,
    },
    granted::{HEADER_UNDECODABLE_SHARE, Role},
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
    /// Answer calls but never make one, and send no beacon, probe or datagram: a station
    /// left listening with nobody at it (`[radio] answer_only`).
    pub answer_only: bool,
    /// Link-layer tuning.
    pub link: GrantConfig,
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
    /// The least time between the end of another station's frames and this station
    /// transmitting, in seconds (`radio.answer_gap_ms`, ADR-0036): the engine's answer gap,
    /// with what this station learned the other needs (ADR-0054).
    pub answer_gap_s: f64,
    /// This station's radio: how long after it stops sending it hears again (VOX hold and
    /// receiver recovery), declared in its call or acceptance so the other station's slots
    /// wait for it (07-protocol-8.md §3).
    pub turnaround_s: f64,
    /// Offer stream compression in the connect handshake. Used only if the peer offers it
    /// too; a station that cannot decompress must never be sent a compressed stream.
    pub compress: bool,
    /// The operator's Morse identifier setting (`[radio] cw_id`), and how it is sent: `None` for
    /// off, the default. A station operating under a United States callsign identifies at the
    /// end of every session whatever this says (`cwid::required_for`, ADR-0075), at
    /// [`cw_id_wpm`](Self::cw_id_wpm); for any other callsign this is the operator's choice.
    pub cw_id: Option<CwId>,
    /// The speed the identifier is sent at when a callsign requires it and the setting is off
    /// (`[radio] cw_id_wpm`).
    pub cw_id_wpm: f64,
    /// The host program keys the radio (`[ptt] kind = "host"`, ADR-0025): with none attached,
    /// nothing this station sends reaches the air, which the log says of a closing identifier.
    pub keyed_by_host: bool,
    /// Where session recordings go. `None` means recording is refused.
    pub record_dir: Option<std::path::PathBuf>,
    /// Record every session on its own: start when one connects, stop when it ends. For a
    /// gateway, which nobody is watching, and for field validation, which wants all of them.
    pub record_auto: bool,
    /// Debug mode (ADR-0050): record every session a host program runs, whatever
    /// `record_auto` says — the daemon sends those recordings to the project.
    pub record_host_sessions: bool,
    /// What every recording that starts on its own should say about the station: the band,
    /// the antenna, whatever the operator would have written down had they been there.
    pub record_notes: String,
    /// Who and where, for the field log a recording becomes.
    pub operator: crate::config::OperatorSection,
    /// Keep the exact audio of every transmission, as handed to the sound card, under
    /// `tx/` in the recordings directory, with a sidecar describing its envelope: what
    /// the modem *sent*, for holding the air against it (`[record] tx_audio`).
    pub record_tx_audio: bool,
    /// The regulatory policy's settings (ADR-0018): the profile, the station's control, the
    /// operator's class, the sideband, and a dial for a radio that cannot report its own.
    pub regulatory: crate::regulatory::Settings,
    /// `[radio] max_mode` as the operator set it, which indexes the ladder of whichever
    /// bandwidth runs: `link.max_mode` is it held to the ladder running now, and a move to the
    /// other bandwidth (ADR-0026) holds it to that one's. `None` takes `link.max_mode`.
    pub max_mode_setting: Option<usize>,
}

impl Default for StationConfig {
    fn default() -> Self {
        Self {
            callsign: String::new(),
            params: WIDE_2300,
            answer_only: false,
            link: GrantConfig::default(),
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
            answer_gap_s: 0.0,
            turnaround_s: 0.0,
            compress: true,
            cw_id: None,
            cw_id_wpm: CwId::default().wpm,
            keyed_by_host: false,
            record_dir: None,
            record_auto: false,
            record_host_sessions: false,
            record_notes: String::new(),
            operator: crate::config::OperatorSection::default(),
            record_tx_audio: false,
            // a harness checks nothing; the daemon always builds this from the file
            regulatory: crate::regulatory::Settings::unchecked(),
            max_mode_setting: None,
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
    /// Acquired confidently enough that its measurements mean something when it does not
    /// decode: the rule `reported_cfo` keeps a noise trigger's offset off the panel by
    /// (ADR-0020).
    trusted: bool,
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

    fn trusted(&self) -> bool {
        self.trusted
    }

    fn follows(&self) -> Option<u8> {
        self.frame.follows()
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
    /// A transmission the link engine decided (ADR-0069): its frames, when it may radiate and
    /// the identifier the engine put ahead of them. The engine follows each one it hands over
    /// to its end, so one the station does not send is reported dropped, never just forgotten.
    Link(LinkTx),
    /// Frames the station queues on its own — a beacon — to be modulated, and why.
    Frames(Vec<aether_link::TxFrame>, Origin),
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

/// A transmission of the link engine's, as it handed it over.
struct LinkTx {
    frames: Vec<aether_link::TxFrame>,
    origin: Origin,
    /// When its radiation may start: no sooner than `not_before`, no later than `not_after`.
    window: KeyWindow,
    /// The closing Morse identifier after the frames, or alone: the engine's placement
    /// (ADR-0075).
    identifier_s: f64,
    /// Made outside every slot — a closing identifier no slot could be had for: it waits for a
    /// clear channel as a call does.
    listen_first: bool,
}

/// When a transmission may radiate, on the station's clock: an answer in a slot from the
/// slot's start to its slack (ADR-0069 §2.3). The keying gate holds the rendered transmission
/// until the first and drops it past the second, whatever held it.
#[derive(Debug, Clone, Copy, Default)]
struct KeyWindow {
    not_before: Option<f64>,
    not_after: Option<f64>,
    /// It carries a closing identifier that was counted as sent: uncounted if the gate drops it,
    /// since nothing of it went out.
    identifier: bool,
    /// Samples of its audio, from the first, to the end of its last frame: the keying's lead and
    /// the frames.
    frames_samples: usize,
    /// Samples after the last frame before the tail: the closing identifier behind a `DISC_ACK`.
    trailing_samples: usize,
    /// Where its last frame ends on the air if it plays whole, set when it is keyed.
    whole_end: Option<f64>,
    /// Samples of its audio, from the first, where its header begins and ends. `None` with no
    /// header first.
    header_samples: Option<(usize, usize)>,
    /// Where on the air its header begins and ends, set when it is keyed.
    header_on_air: Option<(f64, f64)>,
    /// It starts an exchange — a call, a probe — or listens first: it has no slot to be late
    /// for, and may wait for a clear channel (13-wc4y-beta111.md §7).
    slotless: bool,
}

impl KeyWindow {
    /// Whether a transmission whose radiation stopped at `radiated_until` was cut before any
    /// frame of it could be read: its header not [`HEADER_UNDECODABLE_SHARE`] of the way out
    /// (10-step-3.md §3.6). Then it granted nothing. Otherwise a header the other station may
    /// have read anchors the slot it stated, wherever the cut fell (09-step-2.md §2.2).
    fn cut_unread(&self, radiated_until: f64) -> bool {
        self.header_on_air.is_some_and(|(start, end)| {
            radiated_until < start + HEADER_UNDECODABLE_SHARE * (end - start)
        })
    }
}

/// Why a burst of frames was queued and when: the account of the air a harness keeps
/// (`AudioIo::annotate`, ADR-0069 §5). Nothing in the protocol reads it.
#[derive(Debug, Clone, Copy)]
struct Origin {
    /// The engine's entry point or timer (`Action::Transmit::cause`), or what the station
    /// itself queued it for (`beacon`).
    cause: &'static str,
    /// The station's clock when it was queued.
    queued_s: f64,
}

impl Origin {
    /// Frames the station queues on its own: a beacon, or a test's.
    const fn station(cause: &'static str) -> Self {
        Self {
            cause,
            queued_s: f64::NAN,
        }
    }
}

/// Room kept between a burst's air time and the key watchdog: the sound card's clock and
/// the loop's are not the same clock, and a burst that ends a hair past the limit loses its
/// last frame as surely as one that ends seconds past it.
const KEY_TIME_MARGIN_S: f64 = 1.0;

/// The moment of silence between the Morse identifier and the frames it goes with.
const CW_ID_GAP_S: f64 = 0.1;

/// How long a reported capture skip is remembered: longer than any frame takes to arrive and be
/// decoded (ADR-0076).
const CAPTURE_SKIP_MEMORY_S: f64 = 30.0;

/// How long after the audio it lost a capture skip may be noted: a slow pass of the run loop,
/// which is when they come — up to half a second on the mini PC of 13-wc4y-beta111.md §5.
const CAPTURE_SKIP_NOTED_WITHIN_S: f64 = 0.5;

/// The longest a recording runs on past its session's end for the closing identifiers
/// (ADR-0076).
const RECORDING_TAIL_MAX_S: f64 = 30.0;

/// How long a recording runs on once nothing more of the session is to come: the last of
/// the other station's signal and this station's release in it.
const RECORDING_TAIL_AFTER_S: f64 = 1.5;

/// Past the other station's declared identifier, what a slave's recording waits for the
/// master's to have ended: the master keys a turnaround after the slave's identifier, with its
/// keying latency and the radios' paths (13-wc4y-beta111.md §12: 0.7 s apart).
const RECORDING_TAIL_PEER_ID_S: f64 = 2.0;

/// A recording running on past its session's end (ADR-0076).
#[derive(Debug, Clone, Copy)]
struct RecordingTail {
    /// When the session ended, on the station's clock.
    since: f64,
    /// The station was the session's slave: the master's identifier follows its own.
    slave: bool,
    /// The other station's closing identifier, as its call or acceptance declared it.
    peer_identifier_s: f64,
    /// When nothing more of the session was left for this station to send.
    done_at: Option<f64>,
}

/// The longest a burst of frames may be on the air, so that the whole transmission — the
/// keying's lead, the frames and the tail — ends inside the key watchdog's limit (ADR-0017).
/// Six tone frames are 32 s; the watchdog cut the last of every full tone burst on the air,
/// and the link fell to the floor for it. Never less than one OFDM frame's worth, so an
/// extreme setting still sends something. The closing identifier behind a `DISC_ACK` is the
/// engine's to fit (ADR-0075).
fn burst_limit_s(config: &StationConfig) -> f64 {
    (config.max_key_s - config.key_lead_s - config.key_tail_s - KEY_TIME_MARGIN_S).max(1.1)
}

/// How much of a frame that did not decode may lie under this station's own transmission
/// before it is taken for a fragment and kept from the engine: more than the receiver's
/// timing slack and the capture's lag, less than any frame.
const UNDER_OWN_TX_S: f64 = 0.05;

/// The spans of this station's own transmissions remembered for that judgement: a frame
/// reaches the engine within a second or two of its end.
const MUTED_SPANS_KEPT: usize = 8;

/// How much later than the playback lead a sound card's capture of this station's own
/// burst may still be arriving, over and above the lead itself: device buffering both ways,
/// and the radio's own switch back to receive. Measured on an FTDX10 over USB: the receive
/// audio stays at digital silence for 250-325 ms after the key is released, and eight
/// blocks of that in a row would have put the busy detector's floor at −78 dBFS. Half a
/// second covers it with room for a slower rig; the receiver itself still hears throughout.
const CAPTURE_LAG_ALLOWANCE_S: f64 = 0.5;

/// Room kept past the sound card's own reported delay before the key comes up: its
/// report is of the buffer, not the codec and the cable after it (ADR-0038).
const DEVICE_LATENCY_MARGIN_S: f64 = 0.03;

/// How far the waits learned for keying and rendering (`latency.rs`) must move the engine's
/// keying latency before the log says so again.
const LATENCY_NOTE_STEP_S: f64 = 0.01;

/// How late in its slot, by the station's own reckoning at the key, a transmission of the
/// engine's goes on the air before the log says so, with what took the time; or how near its
/// limit (`not_after`). ND1J's first morning Test of 2026-10-15: one answer of KK4ODA-1's
/// rendered in 122 ms where 60 was typical, reached the air 281 ms into a 350 ms slack, and
/// the log said only that the radio was keyed (12-two-paths-80m.md §4). The gate judges the
/// start before the key; what stalls after it is unjudged, and S2 holds there by margin —
/// how often the margin runs thin is what decides whether the gate must judge later.
const SLOW_KEYING_LATE_S: f64 = 0.05;
const SLOW_KEYING_MARGIN_S: f64 = 0.1;

/// The least the keyed tail keeps for the card, whatever it reports: a report of nothing is
/// a host that does not know, not a card with no buffer.
const DEVICE_LATENCY_MIN_S: f64 = 0.05;

/// How long after the key comes up the recording traces what the receiver hears
/// (`trace_after_release`), and how often.
const RELEASE_TRACE_S: f64 = 3.0;
/// See [`RELEASE_TRACE_S`].
const RELEASE_TRACE_STEP_S: f64 = 0.05;

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

/// The longest an identifier sent on its own waits for a clear channel, whatever
/// `wait_for_clear` says — the other station's own identifier, which no frame announces, takes
/// about thirteen seconds for ten characters at 10 wpm. Longer, and what holds the channel is
/// not an identifier: §97.119 asks for this one at the end of the communication, and a host
/// program is told the session is over only once it has gone out (ADR-0075).
const OTHER_ID_WAIT_MAX_S: f64 = 15.0;

/// Why a station under automatic control sends no beacon (ADR-0024).
pub const BEACON_UNDER_AUTOMATIC_CONTROL: &str = "a station under automatic control sends no \
    beacon: a beacon may be automatically controlled only on 28.20–28.30, 50.06–50.08, \
    144.275–144.300, 222.05–222.06 or 432.300–432.400 MHz, or on 33 cm and up (§97.203(d))";

/// Why a second beacon is not queued behind one still waiting: pressed again because nothing
/// seemed to happen, it would go out twice once the channel cleared (ND1J, 2026-09-29).
pub const BEACON_ALREADY_WAITING: &str = "a beacon is already waiting to go out";

/// Why a beacon named for another station is not sent: a beacon identifies this one.
pub const BEACON_NOT_OURS: &str =
    "a beacon carries this station's callsign, or a name with one of its callsigns as its base";

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

/// How long after the end of the last frame heard an arrival may begin and still be taken
/// for the next frame of the same burst, whatever its confidence (ADR-0040). The frames of a
/// burst are contiguous, so the next one starts where the last ended; an acquisition places
/// it up to half a second late when it reads the start weakly, and in ND1J's session of
/// 2026-10-06 the faint arrivals over which this station keyed its acknowledgement began
/// 0.0–0.55 s after the last frame's end.
pub const FOLLOW_ON_WINDOW_S: f64 = 0.6;

/// How far before that end such an arrival may begin: the last frame's own end is a
/// measurement too.
pub const FOLLOW_ON_EARLY_S: f64 = 0.15;

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
    (decoded || trusted_measurement(confidence, detect_confidence)).then_some(cfo_hz)
}

/// Whether a frame was acquired confidently enough that what the receiver measured on it —
/// its offset, its SNR — is a measurement of a real frame even when it does not decode: the
/// mode chips read with confidence, or acquisition well above its threshold. Below both it
/// is as likely the correlator on noise, and the link's rate controller takes no SNR from it
/// and no failure (ADR-0020): one such frame read -11 dB on ND1J's path between frames
/// decoding at +5 to +8, and took the recommendation from rung 4 to rung 1.
#[must_use]
pub fn trusted_measurement(confidence: f64, detect_confidence: f64) -> bool {
    confidence >= aether_phy::modem::MODE_RETRY_CONFIDENCE
        || detect_confidence >= DETECT_CONFIDENCE_TRUSTED
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
    /// The bandwidth the frame says its sender runs, hertz: a beacon's (ADR-0024), a call's,
    /// an answer's, a probe's or a probe answer's capability byte (ADR-0035); none for any
    /// other frame, or for a beacon from before ADR-0024.
    pub bandwidth_hz: Option<u32>,
    /// How many more frames of its burst an ordinary data frame said follow it (ADR-0041);
    /// none for control and tone frames, which cannot say.
    pub follows: Option<u8>,
    /// The sound card's capture reported skipping audio inside the frame: what became of it
    /// says nothing of the path (ADR-0076).
    pub capture_skip: bool,
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
    /// A host program was attached when it came up: its session, as far as the station can
    /// tell (debug mode sends those, ADR-0050).
    host: bool,
    /// The recording running is the session's own, started when it came up: it ends with
    /// the session, though the setting or the host program that started it has gone since.
    recorded: bool,
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

/// Seconds of what was heard before a recording that the recording starts with: the probe and
/// the call that brought a session, which a session's recording, started when it comes up,
/// otherwise misses.
pub const PREROLL_S: f64 = 30.0;

/// How far under its level on quiet the radio's gain must stand before the busy log line and
/// the metrics say so, in dB: the reading wanders a decibel or two on noise alone, and a cut that
/// matters to the floor is the threshold's size (ADR-0078).
pub const RADIO_GAIN_SHOWN_DB: f64 = 6.0;

/// The most deliveries kept for `status` — a panel that was closed or reloaded while one
/// resolved looks its messages up there — and held between two calls of `take_deliveries`.
const RECENT_DELIVERIES: usize = 32;

/// Where the Morse identifier stands (§97.119, ADR-0075). A session's closing identifier is the
/// engine's to place — the last thing each station sends for the session — and nothing inside a
/// session, a beacon or a datagram carries one. What the station keeps is the contact that went
/// no further than its setup, and whether a closing identifier is still to go out.
#[derive(Debug, Clone, Default)]
struct IdentifierState {
    /// When this station last sent something that sets up a contact outside a session — a
    /// call, a probe or its answer — which carries no identifier (ADR-0055), and the callsign
    /// it went as. A contact that goes no further — a probe answered and no call, a call never
    /// answered — is identified on its own once the station has been idle [`SETUP_ID_WAIT_S`]
    /// since; a session it opens is identified at its end.
    setup_at: Option<f64>,
    setup_call: String,
    /// A session's closing identifier was still to go out at the last pass: a host program
    /// hears `DISCONNECTED` once it has (ADR-0075).
    closing: bool,
    /// What became of the last closing identifier: what the `identified` event says.
    outcome: Option<String>,
    /// The callsign whose required identifier the log last said goes out though the setting is
    /// off: said once at start and once when the callsign changes.
    noted_required: Option<String>,
}

/// How long after a contact-setup transmission that led to no session the station waits,
/// idle, before identifying on its own (ADR-0055): long enough for the call a probe is the
/// prelude to — the Test calls at once, an operator within seconds — so the identifier goes at
/// the end of the session instead.
pub const SETUP_ID_WAIT_S: f64 = 30.0;

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
/// A spectrum is taken for the passband monitor no more often than this, in seconds of
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
    /// Frames that did not decode and lay partly under this station's own transmission, so
    /// were never heard whole: kept from the engine.
    pub frames_under_own_tx: usize,
    /// Times a transmission was held back because the channel was busy.
    pub deferred_for_busy: usize,
    /// Answers the keying gate did not send: they would have started radiating past the end of
    /// their slot (ADR-0069 §2.3).
    pub dropped_late: usize,
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
    /// Calls and probes to this station left unanswered because the host program attached
    /// had not said `LISTEN ON`.
    pub calls_unanswered: usize,
}

/// The sound card's playback clock, and where the transmission now running ends on it.
///
/// A whole burst is handed to the sound card as soon as it is rendered (ADR-0010), so the
/// station's own queue drains long before the audio has left; the key is released when the
/// card's clock reaches the reading at which the last sample handed over leaves it. Only the
/// run loop can say what that is, after it has queued the samples: the burst is rendered and
/// the radio keyed between the loop's reading of the clock and the first sample queued, and
/// the card plays silence meanwhile, which its clock counts — so the reading before the burst
/// plus the burst's length comes short of the burst's end by that time (ADR-0010 §7).
#[derive(Debug, Default, Clone, Copy)]
struct PlaybackClock {
    /// The sound card's own clock as last reported by the run loop — samples it has
    /// consumed since it opened. `None` for a harness that never reports one, whose key is
    /// released the moment the queue drains, as it always was.
    device_played: Option<u64>,
    /// Where that clock will stand once the last sample handed over so far has left the
    /// card, as the run loop reported just after handing it over. `None` from every handover
    /// until the report: until then the transmission's end is not known, so it has not come.
    drains_at: Option<u64>,
    /// The transmission now running was cut short: what the card still held was dropped,
    /// so the key is released as soon as the station's own queue is empty.
    cut_short: bool,
    /// Whether the run loop should drop what the sound card still holds.
    flush_device: bool,
}

impl PlaybackClock {
    /// How many samples the sound card has played past the last one handed to it for the
    /// transmission now running: `Some(0)` while that is still leaving, `None` for a
    /// harness that reports no clock, or before the run loop has said where the card drains.
    fn played_past_transmission(&self) -> Option<u64> {
        Some(self.device_played?.saturating_sub(self.drains_at?))
    }

    /// Whether the card's clock has reached the last sample handed for this transmission.
    /// A harness that reports no clock is taken at its word the moment the queue drains,
    /// as it always was.
    fn caught_up(&self) -> bool {
        match self.device_played {
            Some(played) => self.drains_at.is_some_and(|end| played >= end),
            None => true,
        }
    }
}

/// How far the audio this station has processed trails the air (10-step-3.md §3.1).
///
/// The station's clock is the audio it has processed. The sound card's playback clock keeps
/// real time, so the two stand a fixed distance apart while the station keeps up — and further
/// by whatever the card has captured that the station has not yet processed: the pile a slow
/// pass leaves, or the newest seconds a stalled computer kept. The keying gate judges a slot on
/// the air's clock, the station's plus this backlog; on the station's alone, a station catching
/// up after a stall keyed an answer 1.84 s past its slot, over the other station
/// (`80m-slow-machine-500`, six times in ten seeds).
#[derive(Debug, Default, Clone, Copy)]
struct CaptureLag {
    /// The playback clock less the station's clock at its least: the distance while the
    /// station keeps up, which a backlog only adds to. It may creep up by
    /// [`CLOCK_CREEP`] for the two clocks' drift, and drops at once to a smaller reading.
    baseline: Option<f64>,
    /// The station's clock at the last reading.
    at: f64,
    /// The backlog at the last reading, in seconds.
    lag_s: f64,
}

/// How fast the playback clock may gain on the capture clock without it counting as a backlog:
/// seconds a second. Two cards' crystals differ by 100 ppm at most; a backlog builds a
/// thousand times faster.
const CLOCK_CREEP: f64 = 1e-3;

impl CaptureLag {
    /// A reading of the playback clock, in seconds, against the station's.
    fn read(&mut self, played_s: f64, now: f64) {
        let distance = played_s - now;
        let baseline = match self.baseline {
            Some(least) if distance >= least => {
                least + (distance - least).min(CLOCK_CREEP * (now - self.at).max(0.0))
            }
            _ => distance,
        };
        self.baseline = Some(baseline);
        self.at = now;
        self.lag_s = (distance - baseline).max(0.0);
    }
}

/// One station: link engine, physical layer, radio.
// its flags are separate states of separate machinery (the transmitter, the keying test, a Test
// taking a step), not one state spelled in bools
#[allow(clippy::struct_excessive_bools)]
pub struct Station<P: Ptt> {
    config: StationConfig,
    engine: GrantedEngine,
    receiver: StreamingReceiver,
    /// Shared with every [`PhyFrame`] handed to the engine, for decoding and combining.
    decoder: Rc<RefCell<Modem>>,
    transmitter: Modem,
    to_audio: BasebandToAudio,
    from_audio: AudioToBaseband,
    ptt: PttWatchdog<P>,
    busy: BusyDetector,
    /// How far the audio processed trails the air ([`CaptureLag`]).
    capture_lag: CaptureLag,
    /// The channel held by another session this station heard, which its own calls wait out.
    reservation: reserve::Reservation,
    /// Audio waiting for the sound card, at unit level; the transmit level is applied as
    /// it leaves, which is what lets the level change under a tone that is playing.
    playback: VecDeque<f32>,
    baseband_seen: usize,
    /// The receiver's input this station muted while it transmitted, as spans of
    /// `baseband_seen`, the latest last.
    muted: VecDeque<(usize, usize)>,
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
    /// The account of the transmission rendered and not yet keyed (`Self::tag`), and of
    /// those keyed since the run loop last took them (`Self::take_tags`): what a harness puts
    /// on the air beside the audio (ADR-0069 §5).
    tx_tag: Option<String>,
    tx_tags: Vec<String>,
    /// How long the transmission now leaving the queue was held for a busy channel or the
    /// answer gap (`held_for_busy`).
    last_hold_s: f64,
    /// The same for the last transmission that finished, which is the one worth reporting:
    /// a peak is only meaningful over a whole burst.
    tx_peak_last: Option<f32>,
    /// The audio clock when a held-back burst was last reported to the engine, while one
    /// is held.
    held_since: Option<f64>,
    /// When the burst held back now was first held.
    held_from: Option<f64>,
    /// Until when the busy detector ignores what the sound card delivers: the card's
    /// capture runs a lead behind its playback, so the end of this station's own burst
    /// arrives after the key is released, and read as channel it would hold the next burst
    /// back for its own echo.
    deaf_until: f64,
    /// When the sound card's capture reported skipping audio (`audio::CaptureSkips`), on the
    /// station's clock, for the last [`CAPTURE_SKIP_MEMORY_S`]: a frame one falls inside is not
    /// trusted (ADR-0076).
    capture_skips_at: VecDeque<f64>,
    /// The session's recording, running on past its end until the closing identifiers have
    /// gone (ADR-0076).
    recording_tail: Option<RecordingTail>,
    /// When the key last came up, for the receive trace after it (`trace_after_release`).
    released_at: f64,
    /// When that trace last wrote a line.
    traced_at: f64,
    /// How long a sample takes to leave the sound card after its callback, as the card
    /// reports it (`AudioIo::output_latency_s`); `None` until it does, or for one that cannot.
    device_latency_s: Option<f64>,
    /// The waits between deciding a transmission and its first sample leaving the card that the
    /// card's report leaves out: the radio's keying command and the render (`latency.rs`).
    latency: latency::KeyingLatency,
    /// What the engine was last told of this station's keying latency beyond the lead and the
    /// card, as logged, so a change is said once.
    latency_said_s: f64,
    /// The keying command's wait for the transmission now running, and its render on the
    /// card's clock when the run loop measured one, for the recording's `tx_end`.
    key_waited_s: f64,
    rendered_s: Option<f64>,
    /// Transmissions of the engine's that went on the air late in their slots
    /// ([`Self::note_slow_keying`]), for the diagnostic bundle's `keying`.
    late_keyings: u32,
    /// The two playback clocks and how the transmission now running stands against them.
    clock: PlaybackClock,
    /// The exact audio of the transmission now running, when the operator asked for it
    /// to be kept (`[record] tx_audio`).
    tx_capture: Option<crate::record::TxCapture>,
    pending: VecDeque<Outgoing>,
    meter: LevelMeter,
    /// The session recording in progress, if one is.
    recording: Option<crate::record::Recording>,
    /// The last [`PREROLL_S`] of captured audio while nothing records, and the frames found
    /// in it: what a recording starts with (`Recording::prepend`).
    preroll: std::collections::VecDeque<f32>,
    preroll_frames: std::collections::VecDeque<crate::record::FrameRecord>,
    /// Notes the operator gave for the next automatic recording, if any.
    record_notes: Option<String>,
    /// The Test session running, or the last one run (P6-7).
    test: Option<TestRun>,
    /// A Test session is taking its next step: the run is out of `test` meanwhile, and what
    /// it does — ending its session on a crawling transfer — must still see it running, or the
    /// session's end closed the recording before the run wrote its report into the sidecar.
    test_stepping: bool,
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
    /// Station time the passband may next be looked at: a spectrum is taken about twice a
    /// second, not every block, and only when the channel is quiet.
    passband_next_s: f64,
    /// Station time the channel was last anything but quiet — occupied, a frame arriving,
    /// this station transmitting or deaf to its own tail: a spectrum is taken only once a
    /// whole window of audio has been heard since.
    passband_quiet_since: f64,
    /// Spectra taken and not yet folded in, oldest first, each with when it was taken: they
    /// wait out the busy detector's attack, and are dropped if the quiet breaks first.
    passband_pending: VecDeque<(f64, Spectrum)>,
    /// Until when a burst is known to be arriving: a preamble was found and its frame
    /// has not finished, or a frame just finished and the next may follow.
    rx_until: f64,
    /// The session's account, while one is up.
    link: Option<LinkAccount>,
    /// Bytes that crossed the air, with when, for the throughput reading.
    moved: VecDeque<(f64, usize)>,
    /// The rung of the session data this station last sent, and last decoded from the other
    /// station, each with when: what the speed chart draws its rate lines from. The rung this
    /// station would send at is not what moves the bytes while it only answers: KK4ODA-1's chart
    /// showed tone-36 under data arriving at tone4x100-75 (12-two-paths-80m.md §16).
    data_sent: Option<(f64, usize)>,
    data_heard: Option<(f64, usize)>,
    /// The engine's acknowledged-plus-delivered count when `moved` was last fed.
    crossed: usize,
    frequency: FrequencyCache,
    compressor: Compressor,
    decompressor: Decompressor,
    /// Application bytes waiting for a session to negotiate compression.
    outbound: Vec<u8>,
    /// The Morse identifier: when it last went out, and what is owed.
    identifier: IdentifierState,
    /// The engine's transmission rendered or on the air, and when it may radiate (ADR-0069
    /// §2.3): the gate holds it until the slot's start and drops it past the slot's slack. Only
    /// the engine's own transmissions are reported to it, done or dropped — a beacon, a
    /// datagram or a tune tone is not in its account — and this is how they are told apart.
    keying: Option<KeyWindow>,
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
    /// The repeating beacon's timer and what the beacons did.
    beacons: beacons::Beacons,
    /// The bandwidth the station should run, and why it runs what it does (ADR-0026).
    bandwidth: bandwidth::Bandwidth,
    /// The answer gap each station has shown it needs (ADR-0054).
    gaps: gaps::LearnedGaps,
    /// Where the receiver's sample count starts in the station's: the receiver is built again
    /// when the station moves to another bandwidth, and counts from there.
    rx_origin: usize,
    /// What the host program attached now said about this station: whether to answer calls.
    host: host::Host,
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
    // a field a line: the station's state is long, and its constructor says so
    #[allow(clippy::too_many_lines)]
    pub fn new(mut config: StationConfig, ptt: P, seed: u64) -> Self {
        // The key is released when this station's queue runs dry, and at that moment the
        // sound card still holds a playback lead's worth of what was queued last. Unless
        // the tail of silence is at least that long, what the card is still holding is
        // the end of the burst, and the radio drops out of transmit with the last fifth
        // of a second of every frame still to play — seen on a rig's meters as a spike as
        // the transmission ends, and on the air as a frame nobody can decode. The bench
        // never showed it: the simulated channel carries audio whether keyed or not.
        config.key_tail_s += config.playback_lead_s;
        config.max_mode_setting.get_or_insert(config.link.max_mode);
        let params = config.params;
        let mut timing = phy_timing(params);
        // how long after a transmission is keyed it radiates: what an answer is keyed ahead of
        // its slot by (07-protocol-8.md §10); the card's own report replaces the assumed lead
        timing.tx_latency_s = config.key_lead_s + config.playback_lead_s;
        let link = GrantConfig {
            // what this station offers, and the bandwidth it transmits in — stated, not
            // negotiated: a call claiming another bandwidth than it arrived in is ignored
            capabilities: with_bandwidth(
                offered_capabilities(config.compress),
                params.bandwidth.hz(),
            ),
            // the engine keeps the identifier's room itself, in the transmission it places it in
            max_burst_s: config
                .link
                .max_burst_s
                .or_else(|| Some(burst_limit_s(&config))),
            turnaround_s: config.turnaround_s,
            ..config.link.clone()
        };
        let engine = GrantedEngine::new(&config.callsign, timing, link, seed);
        let busy = BusyDetector::new(BusyConfig {
            fs: params.fs_baseband,
            passband_hz: params.occupied_bandwidth_hz(),
            centre_hz: params.centre_hz,
            ..config.busy
        });
        let mut station = Self {
            engine,
            receiver: StreamingReceiver::new(params, 6.0, true),
            decoder: Rc::new(RefCell::new(Modem::new(params, false))),
            transmitter: Modem::new(params, false),
            to_audio: BasebandToAudio::new(params),
            from_audio: AudioToBaseband::new(params),
            ptt: PttWatchdog::new(ptt, config.max_key_s),
            busy,
            capture_lag: CaptureLag::default(),
            reservation: reserve::Reservation::default(),
            playback: VecDeque::new(),
            baseband_seen: 0,
            muted: VecDeque::new(),
            audio_seen: 0,
            transmitting: false,
            playing_test: false,
            was_busy: false,
            tx_peak_running: 0.0,
            tx_tag: None,
            tx_tags: Vec::new(),
            last_hold_s: 0.0,
            tx_peak_last: None,
            held_since: None,
            held_from: None,
            deaf_until: f64::NEG_INFINITY,
            capture_skips_at: VecDeque::new(),
            recording_tail: None,
            released_at: f64::NEG_INFINITY,
            traced_at: f64::NEG_INFINITY,
            device_latency_s: None,
            latency: latency::KeyingLatency::default(),
            latency_said_s: 0.0,
            key_waited_s: 0.0,
            rendered_s: None,
            late_keyings: 0,
            clock: PlaybackClock::default(),
            tx_capture: None,
            pending: VecDeque::new(),
            meter: LevelMeter::new(params.audio_rate as f64, 3.0),
            recording: None,
            preroll: std::collections::VecDeque::new(),
            preroll_frames: std::collections::VecDeque::new(),
            record_notes: None,
            test: None,
            test_stepping: false,
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
            passband_quiet_since: f64::NEG_INFINITY,
            passband_pending: VecDeque::new(),
            rx_until: f64::NEG_INFINITY,
            link: None,
            moved: VecDeque::new(),
            data_sent: None,
            data_heard: None,
            crossed: 0,
            frequency: FrequencyCache::default(),
            // Nothing is compressed until a session negotiates it. Before that the two ends
            // have not agreed on anything, and a guess would produce a stream the peer
            // cannot read.
            compressor: Compressor::new(false),
            decompressor: Decompressor::new(false),
            outbound: Vec::new(),
            identifier: IdentifierState::default(),
            keying: None,
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
            beacons: beacons::Beacons::default(),
            bandwidth: bandwidth::Bandwidth::new(params.bandwidth.hz()),
            gaps: gaps::LearnedGaps::default(),
            rx_origin: 0,
            host: host::Host::default(),
            stats: StationStats::default(),
            config,
        };
        // a speed set above what the rules allow is sent at their limit, and said so at start,
        // as is an identifier the callsign requires with the setting off (ADR-0075)
        station.note_identifier_speed();
        station.note_required_identifier();
        station.refresh_identifier_length();
        station
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

    /// The sound card's playback clock, as the run loop reads it before each capture and
    /// each top-up: samples the card has consumed since it opened.
    ///
    /// A whole burst is handed to the card as soon as it is rendered, so the station's own
    /// queue drains long before the audio has left; the key is released against this clock
    /// instead, when it reaches where the card drains ([`Self::device_drains_at`]) and the
    /// last sample has really played. A harness that never reports one gets the old
    /// behaviour: release on drain.
    pub fn device_played(&mut self, played: u64) {
        self.clock.device_played = Some(played);
        let rate = self.config.params.audio_rate as f64;
        let now = self.now();
        self.capture_lag.read(played as f64 / rate, now);
    }

    /// How far the audio this station has processed trails the air, in seconds: what the card
    /// has captured and the station not yet processed ([`CaptureLag`]). 0 for a harness that
    /// reports no playback clock.
    #[must_use]
    pub const fn capture_lag_s(&self) -> f64 {
        self.capture_lag.lag_s
    }

    /// Where the sound card's playback clock will stand once everything handed to it so far
    /// has left it — its clock plus what it holds queued ([`crate::audio::AudioIo::drains_at`])
    /// — as the run loop reads it just after handing samples over.
    ///
    /// The key is released, and the deafness at the end of a transmission ends, when the
    /// clock gets there. Not at the loop's reading before the burst plus the burst's length:
    /// the burst is rendered and the radio keyed between that reading and the first sample
    /// queued, and the card plays silence meanwhile, which its clock counts (ADR-0010 §7).
    /// Only the first report after a handover counts; a later one reads the same while the
    /// card still holds the burst, and more once it has run dry. A harness that reports the
    /// clock must report this too, or the key stays down until the watchdog trips.
    pub fn device_drains_at(&mut self, at: u64) {
        self.clock.drains_at.get_or_insert(at);
    }

    /// Whether the transmission now running still has samples to hand the sound card. While
    /// it does, silence the card plays for want of them is a hole in the burst; once the last
    /// is handed over, a card that runs dry has reached the burst's end.
    #[must_use]
    pub fn handing_over(&self) -> bool {
        self.transmitting && !self.playback.is_empty()
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

    /// The rungs the session's data went at in the throughput's window: this station's last
    /// data sent and the other station's last decoded, each `None` when none moved that way —
    /// the speed chart's rate lines, one a direction, against a goodput counted both ways.
    #[must_use]
    pub fn data_rungs(&self) -> (Option<usize>, Option<usize>) {
        let Some(link) = self.link else {
            return (None, None);
        };
        let since = (self.now() - THROUGHPUT_WINDOW_S).max(link.started_s);
        let recent =
            |moved: Option<(f64, usize)>| moved.filter(|(t, _)| *t >= since).map(|(_, rung)| rung);
        (recent(self.data_sent), recent(self.data_heard))
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
    pub fn engine(&self) -> &GrantedEngine {
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
        // a station known to run 500 Hz is called at 500 Hz (ADR-0035) — not for a call the
        // engine is about to refuse
        let ours = as_call.is_none_or(|call| {
            self.engine
                .callsigns
                .contains(&call.trim().to_ascii_uppercase())
        });
        if ours && self.engine.state() == State::Idle {
            self.move_for_call(remote);
        }
        self.engine.connect(remote, as_call)?;
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
            // the identifier sends the callsign: its length, and whether there is one, follow it
            self.refresh_identifier_length();
            self.note_required_identifier();
            return Ok(true);
        }
        // validated now, so a host is not told OK for a list the engine will refuse later
        let mut probe = GrantedEngine::new(
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

    /// Close the session in order: a sending station once everything queued has been
    /// acknowledged, a receiving one between the other station's bursts (ADR-0023) — for no
    /// longer than the engine's drain limit (ADR-0072). With something still queued the log
    /// says how much, how long it may take at most, and that Abort closes at once: on a slow
    /// path a queue took minutes, and Disconnect looked broken.
    pub fn disconnect(&mut self) {
        let asked = self.engine.disconnect_requested();
        self.engine.disconnect();
        let queued = self.engine.tx_pending_bytes();
        if !asked && queued > 0 {
            if let Some(left) = self.leave_in_s() {
                // the master closes at its limit; a slave's request is read an answer later,
                // and the master counts from there
                let bound = if self.engine.role() == Role::Master {
                    "at most"
                } else {
                    "about"
                };
                self.note(
                    "closing",
                    &format!(
                        "{queued} bytes still queued go first, for {bound} {left:.0} s; \
                         Abort closes at once"
                    ),
                );
            }
        }
        self.pump();
    }

    /// Seconds until a disconnect asked for stops waiting for what is queued (ADR-0072), or
    /// `None` when none was asked. A slave's figure is a floor: the master counts from when it
    /// reads the request.
    #[must_use]
    pub fn leave_in_s(&self) -> Option<f64> {
        self.engine.leave_by().map(|by| (by - self.now()).max(0.0))
    }

    /// Close the session now: what is left of a burst goes with it, and one DISC goes out.
    ///
    /// The rest of the burst is the session's, and the session is over. Left to play, a
    /// long tone burst ran on after the operator's abort until the key watchdog cut it,
    /// ten seconds later (ND1J, 2026-09-25), with the DISC queued behind it. An answer
    /// rendered and waiting for its slot goes no further either.
    ///
    /// The engine times what follows: a master's DISC once the slot it granted is over, a
    /// slave's in the old session's next slot (ADR-0052, ADR-0069), so neither lands in the
    /// other station's transmission.
    pub fn abort(&mut self) {
        // The engine's account of what this station has handed over runs in order, the oldest
        // first: the transmission on the air or waiting for its slot, then those queued behind
        // it (`drop_link_pending`).
        if self.transmitting && !self.playing_test {
            self.playback.clear();
            self.authorized = None;
            self.clock.flush_device = true;
            self.clock.cut_short = true;
            // the engine's transmission cut here is over; the slot its header granted is
            // where the header said, which is where the other station looks for it — unless
            // the cut came before the header could be read
            if let Some(window) = self.keying.take() {
                let now = self.now();
                self.report_cut(&window, now);
            }
        } else if self.keying.is_some() && !self.transmitting && !self.playback.is_empty() {
            // rendered and waiting for its slot: it goes no further
            self.discard_waiting_link();
        }
        self.drop_link_pending();
        self.pending
            .retain(|next| !matches!(next, Outgoing::Frames(..)));
        // a master's cut header still granted the slot its count said, which the engine knows
        self.engine.abort();
        self.pump();
    }

    /// Drop every engine transmission still queued, telling the engine of each. The engine
    /// counts them off its oldest first, so the transmission on the air or waiting for its slot
    /// is reported before this.
    fn drop_link_pending(&mut self) {
        let now = self.now();
        let before = self.pending.len();
        self.pending
            .retain(|next| !matches!(next, Outgoing::Link(..)));
        for _ in self.pending.len()..before {
            self.engine.on_tx_dropped(now);
        }
    }

    /// The engine transmission rendered and waiting for its slot is dropped.
    fn discard_waiting_link(&mut self) {
        self.playback.clear();
        self.authorized = None;
        self.tx_capture = None;
        self.tx_tag = None;
        if self.keying.take().is_some() {
            self.engine.on_tx_dropped(self.now());
        }
    }

    /// Nothing on the air, nothing queued for it, no session, and no closing identifier still to
    /// go out (ADR-0075): a station that may stop.
    #[must_use]
    pub fn quiescent(&self) -> bool {
        !self.transmitting
            && self.playback.is_empty()
            && self.pending.is_empty()
            && self.engine.state() == State::Idle
            && !self.engine.identifier_owed()
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
        let now = self.now();
        // the oldest the engine handed over first: the one on the air, or the one waiting
        if !was_transmitting && self.keying.is_some() {
            self.discard_waiting_link();
        } else if let Some(window) = self.keying.take() {
            // cut short: the slot it granted stays where its header said, if it was read
            self.report_cut(&window, now);
        }
        self.drop_link_pending();
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
        if was_transmitting {
            self.forced_release(now, "abandoned");
        }
        let result = self.ptt.unkey(now);
        self.pump();
        result
    }

    /// A transmission that ended without `finish_transmission` — the key watchdog, or the
    /// keying interface failing — still ends in the recording, where the replay looks for
    /// the end of this station's deafness, and still leaves the busy detector deaf to the
    /// tail the card plays after it.
    fn forced_release(&mut self, now: f64, why: &str) {
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, "ptt", "released", &state);
            recording.event(now, "tx_end", why, &state);
        }
        self.released_at = now;
        self.deaf_until = now + self.config.playback_lead_s + CAPTURE_LAG_ALLOWANCE_S;
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
        self.config.answer_gap_s = f64::from(config.radio.answer_gap_ms) / 1000.0;
        // this radio's turnaround: declared in the next call or acceptance
        self.config.turnaround_s = f64::from(config.radio.turnaround_ms) / 1000.0;
        self.engine.set_turnaround_s(self.config.turnaround_s);
        // the operator's fastest rung, on the ladder running now — and in the engine, which
        // used to keep the one it started with until the modem restarted
        self.config.max_mode_setting = Some(config.radio.max_mode);
        let top = self.top_rung();
        self.config.link.max_mode = top;
        self.engine.set_max_mode(top);
        // the station's own bandwidth: it moves to it once idle, unless a host program or a
        // call holds it on the other (ADR-0026)
        self.bandwidth.home_hz = config.radio.bandwidth as usize;
        self.config.answer_only = config.radio.answer_only;
        self.ptt.max_key_s = config.radio.max_key_s;
        self.busy.set_threshold_db(config.radio.busy_threshold_db);
        self.config.record_auto = config.record.auto;
        self.config.record_host_sessions = config.record.send_to_project;
        self.config.record_notes.clone_from(&config.record.notes);
        self.config.operator.clone_from(&config.operator);
        self.set_regulatory(config.regulatory.settings());
        // the identifier: on, off and its speed, from the next session on
        let tone_hz = self
            .config
            .cw_id
            .map_or(CwId::default().tone_hz, |cw| cw.tone_hz);
        self.config.cw_id = config.radio.cw_id.then(|| CwId {
            wpm: config.radio.cw_id_wpm,
            tone_hz,
            ..CwId::default()
        });
        self.config.cw_id_wpm = config.radio.cw_id_wpm;
        self.config.keyed_by_host = matches!(config.ptt, crate::config::PttConfig::Host { .. });
        self.refresh_burst_cap(self.now());
        self.note_identifier_speed();
        self.refresh_identifier_length();
    }

    /// The Morse identifier this station sends when it identifies as `call` (ADR-0075): the
    /// operator's when the setting is on, and — for a callsign that requires one
    /// (`cwid::required_for`: a US callsign) — one at the operator's speed whatever the
    /// setting says. `None`: this callsign sends none.
    #[must_use]
    pub fn identifier_for(&self, call: &str) -> Option<CwId> {
        self.config.cw_id.or_else(|| {
            crate::cwid::required_for(call).map(|_| CwId {
                wpm: self.config.cw_id_wpm,
                ..CwId::default()
            })
        })
    }

    /// The callsign this station identifies with now: the session's, or the first of its
    /// callsigns when none is under way.
    fn identifying_call(&self) -> String {
        if self.engine.state() == State::Idle {
            self.engine
                .callsigns
                .first()
                .cloned()
                .unwrap_or_else(|| self.engine.my_call.clone())
        } else {
            self.engine.my_call.clone()
        }
    }

    /// The speed an identifier set to `cw` is sent at: the one set, or the rules' limit on an
    /// automatic identifier when that is lower (§97.119(b)(1): 20 wpm).
    fn sent_wpm(&self, cw: CwId) -> f64 {
        self.policy
            .cw_id_max_wpm()
            .map_or(cw.wpm, |max| cw.wpm.min(max))
    }

    /// The speed the Morse identifier is sent at for the callsign the station identifies with
    /// now; `None` when it sends none.
    #[must_use]
    pub fn identifier_wpm(&self) -> Option<f64> {
        self.identifier_for(&self.identifying_call())
            .map(|cw| self.sent_wpm(cw))
    }

    /// Say once, when it is so, that the identifier goes slower than set. ND1J set a faster
    /// speed and heard the same 20 wpm every time: the rules' limit was applied and never
    /// mentioned.
    pub fn note_identifier_speed(&mut self) {
        let Some(cw) = self.identifier_for(&self.identifying_call()) else {
            return;
        };
        let sent = self.sent_wpm(cw);
        if sent < cw.wpm {
            self.note(
                "identifier",
                &format!(
                    "set to {} wpm, sent at {sent} wpm: the rules limit an automatically keyed \
                     identifier to {sent} wpm (§97.119(b)(1))",
                    cw.wpm
                ),
            );
        }
    }

    /// Say once at start, and once each time the callsign changes, that the identifier goes out
    /// though the setting is off, and why (ADR-0075).
    fn note_required_identifier(&mut self) {
        let call = self.identifying_call();
        let required = crate::cwid::required_for(&call);
        if self.config.cw_id.is_some() || required.is_none() {
            self.identifier.noted_required = None;
            return;
        }
        if self.identifier.noted_required.as_deref() == Some(call.as_str()) {
            return;
        }
        let reason = required.map_or("", crate::cwid::Required::reason);
        let sent = crate::cwid::sent_as(&call);
        self.note(
            "identifier",
            &format!(
                "the Morse ID goes out as {sent} at the end of every session though its setting \
                 is off. {reason}"
            ),
        );
        self.identifier.noted_required = Some(call);
    }

    /// The identifier's part of `status`: whether it is sent for the callsign the station
    /// identifies with now, whether the callsign requires it and why, the operator's setting,
    /// the speed set and the speed sent, and whether a session's closing identifier is still
    /// to go out.
    #[must_use]
    pub fn identifier_status(&self) -> serde_json::Value {
        let mut status = self.identifier_check(&self.identifying_call());
        status["pending"] = serde_json::Value::Bool(self.closing_identifier_pending());
        status
    }

    /// What the identifier would be for `call` with the station's settings — `identifier.check`,
    /// which the panel asks of a callsign typed in Setup, so the one rule (`cwid::required_for`)
    /// decides there too: whether one is sent, whether the callsign requires it and why, the
    /// operator's setting, and the speeds.
    #[must_use]
    pub fn identifier_check(&self, call: &str) -> serde_json::Value {
        let call = call.trim().to_ascii_uppercase();
        let required = crate::cwid::required_for(&call);
        let cw = self.identifier_for(&call);
        serde_json::json!({
            "enabled": cw.is_some(),
            "setting": self.config.cw_id.is_some(),
            "forced": required.is_some(),
            "reason": required.map(crate::cwid::Required::reason),
            "sent_as": crate::cwid::sent_as(&call),
            "callsign": call,
            "wpm": cw.map_or(self.config.cw_id_wpm, |cw| cw.wpm),
            "sent_wpm": cw.map(|cw| self.sent_wpm(cw)),
            "max_wpm": self.policy.cw_id_max_wpm(),
        })
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
        if self.transmitting || !self.playback.is_empty() {
            return Err("a transmission is on the air".into());
        }
        if matches!(self.engine.state(), State::Connected | State::Disconnecting) {
            return Err("a session is running".into());
        }
        // nor with anything under way that expects this dial: a call or a probe whose answer
        // would come back where the radio no longer listens, a Test — the panel tunes on a
        // pick from the dial list, and a pick is easy to make by accident
        if self.test_running() {
            return Err("a Test session is running".into());
        }
        if self.engine.probing() {
            return Err("a probe is out".into());
        }
        if self.engine.state() == State::Connecting {
            return Err("a call is going out".into());
        }
        // A beacon or a KISS program's datagram that is only waiting for a clear channel goes
        // with the radio: moving off a busy frequency is what an operator does about one, and
        // it is judged by the rules at the new dial when it goes. ND1J's two beacons sat behind
        // a busy channel and the dial list refused him "the station is transmitting", which it
        // was not (2026-09-29). What does belong to this dial — an answer, the identifier after
        // a session, a tone or drive bursts the operator is watching — holds it.
        if let Some(what) = self.pending.iter().find_map(holds_the_dial) {
            return Err(format!("{what} is waiting to go out on this frequency"));
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

    /// Transmit one unproto beacon: this station's callsign, addressed to nobody — or `name`,
    /// the name a host program gave it (`VarAC`'s `KK4ODA-9`), when one of the station's
    /// callsigns is its base — with the bandwidth the station runs (ADR-0024).
    ///
    /// It is how an operator answers "can anybody hear me?" without arranging a contact
    /// first. Sent at the most robust mode, because the point is to be heard by somebody who
    /// cannot yet hear anything else — and never while a session is running, which would put
    /// a frame into the middle of somebody's transfer.
    ///
    /// # Errors
    /// On an answer-only station, under automatic control (§97.203(d)), if a session is up,
    /// for a name that is not this station's ([`BEACON_NOT_OURS`]), or if the callsign cannot
    /// be packed.
    pub fn beacon(&mut self, name: Option<&str>) -> Result<(), &'static str> {
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and sends no beacon");
        }
        // A beacon may be automatically controlled only in the segments of §97.203(d), and
        // of those only 28.20–28.30 MHz is anywhere Aether's data may go. The repeating
        // beacon was refused under automatic control already; a host program's beacon timer
        // on an unattended station is the same thing by another road (ADR-0024).
        if self.config.regulatory.control == Some(crate::regulatory::ControlMode::Automatic) {
            return Err(BEACON_UNDER_AUTOMATIC_CONTROL);
        }
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        if self.beacon_waiting() {
            return Err(BEACON_ALREADY_WAITING);
        }
        // the name a host program gave the beacon — VarAC's CQs and beacons are `KK4ODA-9`
        // and the like, meaning something to the program at the other end — is carried as
        // given when this station's callsign is its base (ADR-0024)
        let call = match name.map(|n| n.trim().to_ascii_uppercase()) {
            Some(name) if self.names_this_station(&name) => name,
            Some(_) => return Err(BEACON_NOT_OURS),
            None => self.engine.my_call.clone(),
        };
        let mut body = pack_callsign(&call)
            .map_err(|_| "the callsign will not pack")?
            .to_vec();
        // and the bandwidth this station runs, which a host program told of the beacon
        // reports beside it (`CQFRAME <call> <bandwidth>`); a receiver of an earlier version
        // reads the callsign and never looks further
        body.push(aether_link::frames::with_bandwidth(
            0,
            self.config.params.bandwidth.hz(),
        ));
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
        self.pending.push_back(Outgoing::Frames(
            vec![aether_link::TxFrame {
                container: Container::Data,
                payload,
                mode,
                rv: 0,
                floor: false,
                follows: 0,
            }],
            Origin::station("beacon"),
        ));
        self.stats.beacons_sent += 1;
        Ok(())
    }

    /// Whether a name is this station's: one of its callsigns, or one with a suffix a host
    /// program added (`KK4ODA-9` for `KK4ODA`) — the same base before any `-`.
    fn names_this_station(&self, name: &str) -> bool {
        let base = |call: &str| call.split('-').next().unwrap_or_default().to_owned();
        let wanted = base(name);
        !wanted.is_empty()
            && self
                .engine
                .callsigns
                .iter()
                .any(|call| base(call).eq_ignore_ascii_case(&wanted))
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
            follows: 0,
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
        // the last session's, still waiting for its identifiers: it ends here
        if self.recording_tail.take().is_some() {
            self.stop_recording();
        }
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
        let mut recording = recording;
        let preroll: Vec<f32> = std::mem::take(&mut self.preroll).into();
        let frames: Vec<_> = std::mem::take(&mut self.preroll_frames).into();
        if let Err(error) = recording.prepend(&preroll, frames) {
            self.note(
                "error",
                &format!("the recording's first seconds were lost: {error}"),
            );
        }
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

    /// End the session's recording once its closing identifiers have gone (ADR-0076): this
    /// station's, and — when it was the slave — the master's, which follows it. Until then the
    /// recording runs on, at most [`RECORDING_TAIL_MAX_S`]. Ended at the session's end, no
    /// recording held either identifier, and ADR-0075's order could not be read from the air
    /// (13-wc4y-beta111.md §12).
    pub(crate) fn stop_recording_after_identifiers(&mut self, slave: bool) {
        if self.recording.is_none() {
            return;
        }
        self.recording_tail = Some(RecordingTail {
            since: self.now(),
            slave,
            peer_identifier_s: self.engine.peer_identifier_s(),
            done_at: None,
        });
        // judged from the next pass on: the session's end is drained from the engine before
        // the DISC_ACK it sends with it is queued
    }

    /// Close a recording running on past its session once nothing more of the session is to
    /// come on the air: this station's own transmissions over, its identifier gone or none
    /// owed, and the other station's identifier given its time.
    fn advance_recording_tail(&mut self, now: f64) {
        let Some(mut tail) = self.recording_tail else {
            return;
        };
        if self.recording.is_none() {
            self.recording_tail = None;
            return;
        }
        let own_done = !self.transmitting
            && !self.closing_identifier_pending()
            && !self
                .pending
                .iter()
                .any(|next| matches!(next, Outgoing::Link(_)));
        if own_done && tail.done_at.is_none() {
            tail.done_at = Some(now);
        }
        // the master identifies after the slave's identifier, a turnaround and its keying later
        let hold = if tail.slave && tail.peer_identifier_s > 0.0 {
            tail.peer_identifier_s + RECORDING_TAIL_PEER_ID_S
        } else {
            RECORDING_TAIL_AFTER_S
        };
        let due = tail.done_at.is_some_and(|at| now >= at + hold)
            || now >= tail.since + RECORDING_TAIL_MAX_S;
        if due {
            self.recording_tail = None;
            self.stop_recording();
        } else {
            self.recording_tail = Some(tail);
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
        if self.recording.is_none() {
            let keep = (PREROLL_S * self.config.params.audio_rate as f64) as usize;
            self.preroll.extend(audio.iter().copied());
            let excess = self.preroll.len().saturating_sub(keep);
            self.preroll.drain(..excess);
        }
        if let Some(recording) = &mut self.recording
            && let Err(error) =
                crate::waits::timed(crate::waits::Wait::Recording, || recording.captured(audio))
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
                self.note_muted(muted.len());
                self.absorb(&muted, now);
            } else {
                let during = baseband.len() - after;
                let ended = now - after as f64 * self.config.params.fs_baseband.recip();
                let muted = vec![(0.0, 0.0); during];
                self.note_muted(during);
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
                // the whole passband first: the radio's gain, which the channel's own blocks
                // are judged at (ADR-0078)
                self.busy
                    .hear_passband(audio, self.config.params.audio_rate as f64, now);
                self.busy.push(&baseband, now);
            }
            self.absorb(&baseband, now);
        }
        self.note_busy_transition(now);
        self.reservation.channel(now, self.busy.busy(now));
        self.trace_after_release(&baseband, now);
        self.sample_passband(now);

        self.refresh_burst_cap(now);
        self.refresh_ceiling(false);
        self.engine.set_answer_gap(self.answer_gap_s());
        self.engine.tick(now);
        if self.ptt.poll(now)? == WatchdogState::Tripped {
            // the key was stuck: drop whatever was still queued rather than resume mid-burst
            self.stats.watchdog_trips += 1;
            self.playback.clear();
            self.authorized = None;
            // the engine's transmission on the air first, then those queued behind it; the
            // slot it granted stays where its header said
            if let Some(window) = self.keying.take() {
                self.report_cut(&window, now);
            }
            self.drop_link_pending();
            self.pending.clear();
            self.clock.flush_device = true;
            self.clock.cut_short = false;
            self.tx_capture = None;
            self.transmitting = false;
            self.forced_release(now, "watchdog");
            self.ptt.unkey(now)?;
            self.note("watchdog", "key time exceeded");
        }
        self.pump();
        self.advance_identifier(now);
        self.advance_recording_tail(now);
        self.advance_test();
        self.advance_beacons(now);
        self.advance_call_bandwidth(now);
        self.advance_bandwidth(now);
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
        // the card's own clock reaches where the run loop, having queued the last sample,
        // said the card would drain. A harness that reports no clock releases on drain, as
        // before.
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

        if !self.transmitting && !self.within_window(now) {
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
            // the radio's keying command holds the run loop until the radio has answered (a
            // CAT round trip), and the card plays on meanwhile: the wait is learned, and this
            // transmission's start judged on the wait it took (`latency.rs`)
            let asked = std::time::Instant::now();
            self.ptt.key(now)?;
            let waited = asked.elapsed().as_secs_f64();
            self.latency.key.push(waited);
            self.key_waited_s = waited;
            self.tell_engine_latency();
            let starts_at = now + self.capture_lag_s() + waited;
            let radiates_at = starts_at + self.radiation_latency_s();
            if let Some(latest) = self.keying.and_then(|window| window.not_after) {
                if radiates_at > latest + 1e-6 {
                    // S2 by construction: the key is up and nothing has been played, so letting
                    // it go again radiates nothing (ADR-0071)
                    self.ptt.unkey(now)?;
                    self.drop_late(now, radiates_at - latest);
                    return Ok(0);
                }
            }
            self.note_slow_keying(now, radiates_at, waited);
            self.transmitting = true;
            self.clock.cut_short = false;
            self.stats.transmissions += 1;
            // where its frames end if it plays whole: the slot it grants is anchored there, by
            // the other station on what its header stated, whatever cuts it short — from where
            // the air's clock stands once the key is up, then the card's output latency
            let audio_rate = self.config.params.audio_rate as f64;
            let output_latency =
                starts_at - now + self.radiation_latency_s() - self.config.key_lead_s;
            if let Some(window) = &mut self.keying {
                window.whole_end =
                    Some(now + output_latency + window.frames_samples as f64 / audio_rate);
                let at = |samples: usize| now + output_latency + samples as f64 / audio_rate;
                window.header_on_air = window.header_samples.map(|(a, b)| (at(a), at(b)));
            }
            if let Some(tag) = self.tx_tag.take() {
                self.tx_tags.push(tag);
            }
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
        if count > 0 {
            // these leave after everything already queued: where the card drains is not
            // known again until the run loop, having queued them, says
            self.clock.drains_at = None;
        }
        if let Some(capture) = &mut self.tx_capture {
            capture.push(&out[..count]);
        }
        Ok(count)
    }

    /// Tell the engine of its transmission cut short at `now`. The audio already past the
    /// station's queue still leaves the card, so radiation stops the card's output latency
    /// after the cut. Cut before it could be read, it granted nothing; otherwise it ended where
    /// its frames would have, which is where the other station places the slot it granted.
    fn report_cut(&mut self, window: &KeyWindow, now: f64) {
        let radiated_until = now + self.radiation_latency_s() - self.config.key_lead_s;
        if window.cut_unread(radiated_until) {
            self.engine.on_tx_cut_unread(now);
        } else {
            self.engine.on_tx_done(now, window.whole_end);
        }
    }

    /// How long after the key comes up a transmission starts radiating: the silence of the
    /// keying lead, then the sound card's output latency — its own report once it runs, the
    /// assumed playback lead until then.
    fn radiation_latency_s(&self) -> f64 {
        self.config.key_lead_s + self.device_latency_s.unwrap_or(self.config.playback_lead_s)
    }

    /// The keying gate (ADR-0069 §2.3): whether the rendered transmission waiting to be keyed
    /// may be keyed now. An answer decided ahead of its slot waits for it (`not_before`); one
    /// that would start radiating past its slack is not sent at all — whatever held it, a slow
    /// pass, the radio, the rules — and the engine is told, so its account of what this station
    /// has on the air stays true.
    fn within_window(&mut self, now: f64) -> bool {
        let Some(window) = self.keying else {
            return true;
        };
        if self.playback.is_empty() {
            return true;
        }
        // on the air's clock: the station's, plus what it has captured and not yet processed —
        // the render just done among it, read off the card's clock — then the radio's keying
        // command, the key lead and the card. Held while even the fastest recent command would
        // put it on the air early; refused if the typical one would put it past its slack (and
        // judged again with the wait measured, once the key is up: `playback`)
        let keyed_at = now + self.capture_lag_s() + self.radiation_latency_s();
        if window
            .not_before
            .is_some_and(|earliest| keyed_at + self.latency.key.least() < earliest - 1e-6)
        {
            return false;
        }
        let radiates_at = keyed_at + self.latency.key.typical();
        if window
            .not_after
            .is_some_and(|latest| radiates_at > latest + 1e-6)
        {
            let late = radiates_at - window.not_after.unwrap_or(radiates_at);
            self.drop_late(now, late);
            return false;
        }
        true
    }

    /// Say in the log, and in the recording, when a transmission of the engine's just keyed
    /// starts radiating [`SLOW_KEYING_LATE_S`] or more after its slot opened, or within
    /// [`SLOW_KEYING_MARGIN_S`] of its limit, by the station's own reckoning at the key: its
    /// render, the radio's keying command and how far the station is behind the air, each
    /// beside what it typically takes. What the sound card does after the key is not in it.
    fn note_slow_keying(&mut self, now: f64, radiates_at: f64, key_s: f64) {
        let Some(window) = self.keying else {
            return;
        };
        // a call or a probe has no slot: the engine places what it grants from where it goes,
        // and one held for a busy channel is held on purpose — WC4Y's call was reported "1820 ms
        // after its slot opened" for the busy hangover after a probe's answer
        if window.slotless {
            return;
        }
        let Some(opened) = window.not_before else {
            return;
        };
        let late = radiates_at - opened;
        let margin = window.not_after.map(|latest| latest - radiates_at);
        if late < SLOW_KEYING_LATE_S && margin.is_none_or(|m| m >= SLOW_KEYING_MARGIN_S) {
            return;
        }
        self.late_keyings += 1;
        let ms = |seconds: f64| seconds * 1000.0;
        let behind = self.capture_lag_s();
        let detail = format!(
            "a transmission on the air about {:.0} ms after its slot opened{}: rendered in {} \
             (typically {:.0} ms), the radio keyed in {:.0} ms (typically {:.0} ms), the station \
             {:.0} ms behind the air",
            ms(late),
            margin.map_or_else(String::new, |m| format!(
                ", {:.0} ms inside its limit",
                ms(m)
            )),
            self.rendered_s.map_or_else(
                || "a pass not timed".to_owned(),
                |r| format!("{:.0} ms", ms(r))
            ),
            ms(self.latency.render.typical()),
            ms(key_s),
            ms(self.latency.key.typical()),
            ms(behind),
        );
        self.note("keying", &detail);
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(
                now,
                "keying_late",
                &format!(
                    "late {late:.3} margin {} render {} key {key_s:.3} behind {behind:.3}",
                    margin.map_or_else(|| "-".to_owned(), |m| format!("{m:.3}")),
                    self.rendered_s
                        .map_or_else(|| "-".to_owned(), |r| format!("{r:.3}")),
                ),
                &state,
            );
        }
    }

    /// Drop the rendered transmission waiting to be keyed, which would start radiating `late`
    /// seconds past its slack: not sent at all, and the engine told.
    fn drop_late(&mut self, now: f64, late: f64) {
        let Some(window) = self.keying else {
            return;
        };
        if window.identifier {
            // nothing of it went out: the engine owes it again, behind the next DISC_ACK
            self.stats.cw_ids = self.stats.cw_ids.saturating_sub(1);
            self.identifier.outcome = None;
        }
        self.playback.clear();
        self.authorized = None;
        self.keying = None;
        self.tx_capture = None;
        self.tx_tag = None;
        self.stats.dropped_late += 1;
        self.engine.on_tx_dropped(now);
        let behind = self.capture_lag_s();
        let why = if behind >= 0.05 {
            format!(" — the station {:.0} ms behind the air", behind * 1000.0)
        } else {
            String::new()
        };
        self.note(
            "dropped",
            &format!(
                "an answer {:.0} ms past its slot: not sent{why}",
                late * 1000.0
            ),
        );
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, "dropped", &format!("late {late:.3}"), &state);
        }
        self.pump();
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
        let tail_s = self.tail_s();
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
            // Where the sound card stood when the key came up, against where it drains: how
            // long the radio stayed keyed past the burst's last sample, which the turnaround
            // plot puts at its t = 0 (ADR-0037).
            let rate = self.config.params.audio_rate as f64;
            let seconds = |samples: Option<u64>| samples.map(|s| format!("{:.3}", s as f64 / rate));
            recording.event(
                now,
                "tx_end",
                &format!(
                    "played {} drains_at {} tail {:.3} latency {} deaf_for {:.3} key {:.3} \
                     render {}{}",
                    seconds(self.clock.device_played).unwrap_or_else(|| "-".into()),
                    seconds(self.clock.drains_at).unwrap_or_else(|| "-".into()),
                    tail_s,
                    self.device_latency_s
                        .map_or_else(|| "-".to_owned(), |s| format!("{s:.3}")),
                    self.config.playback_lead_s + CAPTURE_LAG_ALLOWANCE_S,
                    self.key_waited_s,
                    self.rendered_s
                        .take()
                        .map_or_else(|| "-".to_owned(), |s| format!("{s:.3}")),
                    if cut { " cut" } else { "" },
                ),
                &state,
            );
        }
        self.released_at = now;
        self.ptt.unkey(now)?;
        self.datagram_transmitted(cut);
        // the engine is told of its own transmissions only, with where the last frame ended
        // on the air: the tail's silence before the release less, the card's output latency
        // more (07-protocol-8.md §3.4). The slot the transmission granted is measured from
        // there, as the other station measures it from the frames it hears end.
        if let Some(window) = self.keying.take() {
            // the frames end before the closing identifier behind them (ADR-0075)
            let trailing = window.trailing_samples as f64 / self.config.params.audio_rate as f64;
            let frames_end =
                now - tail_s - trailing + self.radiation_latency_s() - self.config.key_lead_s;
            self.engine.on_tx_done(now, Some(frames_end));
        }
        self.deaf_until = now + self.config.playback_lead_s + CAPTURE_LAG_ALLOWANCE_S;
        self.pump();
        Ok(())
    }

    /// The sound card's report of how long a sample takes to leave it after its callback
    /// (ADR-0038): the run loop passes it on each pass, and the keyed tail follows it.
    pub fn device_latency(&mut self, seconds: Option<f64>) {
        if let Some(s) = seconds.filter(|s| s.is_finite() && *s >= 0.0 && *s < 1.0) {
            self.device_latency_s = Some(s);
            self.tell_engine_latency();
        }
    }

    /// Render what is queued to go next, if nothing is on the air or waiting to be keyed, and
    /// say whether that was a transmission of the engine's. The run loop reads the sound card's
    /// clock either side ([`Self::render_took`]) and again before the keying gate judges it: a
    /// render that took the card's clock on took the air's with it (`latency.rs`).
    pub fn render_pending(&mut self) -> bool {
        if self.transmitting || !self.playback.is_empty() {
            return false;
        }
        let now = self.now();
        self.start_pending(now);
        !self.playback.is_empty() && self.keying.is_some()
    }

    /// How long, in samples of the sound card's playback clock, the render of a transmission
    /// of the engine's took ([`Self::render_pending`]).
    pub fn render_took(&mut self, samples: u64) {
        let seconds = samples as f64 / self.config.params.audio_rate as f64;
        self.latency.render.push(seconds);
        self.rendered_s = Some(seconds);
        self.tell_engine_latency();
    }

    /// The sound card's capture reported skipping `lost_s` of audio, just now: recorded, and
    /// remembered so that a frame it fell inside is not trusted (ADR-0076). The capture stream's
    /// timestamps see about half of such losses (12-two-paths-80m.md §15.2; 13-wc4y-beta111.md
    /// §5), so the station's clock is not moved by them.
    pub fn note_capture_skip(&mut self, lost_s: f64) {
        let now = self.now();
        self.record_event("capture_skip", &format!("lost {lost_s:.3}"));
        self.capture_skips_at.push_back(now);
        while self
            .capture_skips_at
            .front()
            .is_some_and(|&at| at < now - CAPTURE_SKIP_MEMORY_S)
        {
            self.capture_skips_at.pop_front();
        }
    }

    /// Whether a capture skip was reported inside `start..end` on the station's clock. A skip
    /// is noted after the audio it was in has been taken in, by up to a slow pass of the run
    /// loop — which is when they come — so the window reaches that far past the frame.
    fn capture_skipped_within(&self, start: f64, end: f64) -> bool {
        self.capture_skips_at
            .iter()
            .any(|&at| at >= start && at <= end + CAPTURE_SKIP_NOTED_WITHIN_S)
    }

    /// Put a line in the session recording, when one is running: what the run loop learns of
    /// the audio that the station cannot see itself.
    pub fn record_event(&mut self, event: &str, detail: &str) {
        let now = self.now();
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, event, detail, &state);
        }
    }

    /// What this station has learned of its keying, for the diagnostic bundle: the key lead,
    /// the card's reported output latency, the radio's keying command and the render (typical,
    /// fastest and slowest of the recent ones), and the latency the engine is told — in ms.
    #[must_use]
    pub fn keying_report(&self) -> serde_json::Value {
        let ms = |s: f64| (s * 10_000.0).round() / 10.0;
        let recent = |r: &latency::Recent| {
            serde_json::json!({
                "typical_ms": ms(r.typical()),
                "least_ms": ms(r.least()),
                "most_ms": ms(r.most()),
                "count": r.count(),
            })
        };
        serde_json::json!({
            "lead_ms": ms(self.config.key_lead_s),
            "card_ms": self.device_latency_s.map(ms),
            "key": recent(&self.latency.key),
            "render": recent(&self.latency.render),
            "engine_latency_ms": ms(self.engine_latency_s()),
            "late": self.late_keyings,
        })
    }

    /// The engine's own keying latency: from deciding a transmission to its radiation — the key
    /// lead and the card's output latency, and the radio's keying command and the render as
    /// they typically take (`latency.rs`). An answer is decided that much ahead of its slot, and
    /// the master keys that much ahead of where its transmission belongs.
    fn engine_latency_s(&self) -> f64 {
        self.radiation_latency_s() + self.latency.typical_s()
    }

    /// Tell the engine its keying latency, and say it in the log when what was learned moves it
    /// by [`LATENCY_NOTE_STEP_S`] or more.
    fn tell_engine_latency(&mut self) {
        self.engine.set_tx_latency_s(self.engine_latency_s());
        let waits = self.latency.typical_s();
        if (waits - self.latency_said_s).abs() < LATENCY_NOTE_STEP_S {
            return;
        }
        self.latency_said_s = waits;
        let detail = format!(
            "the radio takes {:.0} ms to key and a transmission {:.0} ms to render; \
             transmissions now start {:.0} ms after they are decided, and are decided that much \
             ahead",
            self.latency.key.typical() * 1000.0,
            self.latency.render.typical() * 1000.0,
            self.engine_latency_s() * 1000.0,
        );
        self.note("keying", &detail);
    }

    /// The silence the key stays down for after a burst's last sample (ADR-0038). It covers
    /// the sound card's buffering after the playback clock has passed the sample: the card's
    /// own report plus a margin, between [`DEVICE_LATENCY_MIN_S`] and the assumed playback
    /// lead, where it reports one; the whole assumed lead where it does not. ND1J's answers
    /// began 90–180 ms after the key came up and the radio's audio 60–120 ms after it: a
    /// quarter second of a tail kept for a card that needs a tenth is what that margin was.
    fn tail_s(&self) -> f64 {
        let lead = self.config.playback_lead_s;
        let base = self.config.key_tail_s - lead;
        match self.device_latency_s {
            Some(latency) if lead > 0.0 => {
                base + (latency + DEVICE_LATENCY_MARGIN_S)
                    .clamp(DEVICE_LATENCY_MIN_S.min(lead), lead)
            }
            _ => self.config.key_tail_s,
        }
    }

    /// For [`RELEASE_TRACE_S`] after the key comes up, a line every [`RELEASE_TRACE_STEP_S`]
    /// in the recording: the block's own power, the busy detector's level and floor, whether
    /// it is busy and whether it is still deaf to this station's tail. What the receiver
    /// hears as the radio comes back to receive, against what the station made of it — the
    /// raw audio is in the WAV, the station's reading of it only here (ADR-0037).
    fn trace_after_release(&mut self, baseband: &[Complex], now: f64) {
        if self.recording.is_none()
            || now - self.released_at > RELEASE_TRACE_S
            || now - self.traced_at < RELEASE_TRACE_STEP_S
            || baseband.is_empty()
        {
            return;
        }
        self.traced_at = now;
        let power =
            baseband.iter().map(|&(i, q)| i * i + q * q).sum::<f64>() / baseband.len() as f64;
        let detail = format!(
            "after {:.3} power {:.1} level {:.1} floor {:.1} busy {} deaf {}",
            now - self.released_at,
            10.0 * power.max(1e-12).log10(),
            self.busy.level_db,
            self.busy.floor_db,
            u8::from(self.busy.busy(now)),
            u8::from(now < self.deaf_until),
        );
        let state = format!("{:?}", self.engine.state());
        if let Some(recording) = &mut self.recording {
            recording.event(now, "rx_trace", &detail, &state);
        }
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
    /// The next `len` samples the receiver is given are silence for this station's own
    /// transmission.
    fn note_muted(&mut self, len: usize) {
        let from = self.baseband_seen;
        match self.muted.back_mut() {
            Some(span) if span.1 == from => span.1 += len,
            _ => {
                self.muted.push_back((from, from + len));
                if self.muted.len() > MUTED_SPANS_KEPT {
                    self.muted.pop_front();
                }
            }
        }
    }

    /// Whether a frame found at receiver samples `start..end` lay under this station's own
    /// transmission for more than [`UNDER_OWN_TX_S`].
    fn under_own_tx(&self, start: usize, end: usize) -> bool {
        let limit = UNDER_OWN_TX_S * self.config.params.fs_baseband;
        self.muted.iter().any(|&(m0, m1)| {
            let overlap = end.min(m1).saturating_sub(start.max(m0));
            overlap as f64 > limit
        })
    }

    /// A frame that did not decode and lay partly under this station's own transmission
    /// was never heard whole — the receiver was given silence for that part — and its timing
    /// is as doubtful as its content: one acquired three quarters of a second late, out of the
    /// last frame of a burst already acknowledged, ran on under the acknowledgement, took a
    /// burst to be still arriving, and the acknowledgement went again over the sender's next
    /// burst (the scenario harness, 80 m at 500 Hz). It says nothing of the other station's
    /// burst, and the engine does not see it.
    fn fragment_of_own_tx(
        &mut self,
        decoded: &aether_phy::modem::DecodedFrame,
        origin: usize,
        air: &aether_phy::modes::AirInterface,
    ) -> bool {
        let first = origin + decoded.frame.start();
        let fragment =
            !decoded.ok() && self.under_own_tx(first, first + decoded.frame.samples(air));
        if fragment {
            self.stats.frames_under_own_tx += 1;
        }
        fragment
    }

    fn absorb(&mut self, baseband: &[Complex], now: f64) {
        let fs = self.config.params.fs_baseband;
        // where this receiver's positions start: a move to the other bandwidth (ADR-0026)
        // builds another, but what this one found is placed by its own count
        let origin = self.rx_origin;
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
            let span_start = (origin + decoded.frame.start()) as f64 / fs;
            let span_end = span_start + decoded.frame.samples(&air) as f64 / fs;
            // its blocks are a frame's, not the noise's (ADR-0076)
            self.busy.explain(span_start, span_end);
            let record = crate::record::FrameRecord {
                t_s: now,
                start_s: Some(span_start),
                end_s: Some(span_end),
                follows: decoded.frame.follows(),
                kind: if control { "control" } else { "data" }.into(),
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
                capture_skip: self.capture_skipped_within(span_start, span_end),
            };
            self.keep_frame(record, now);
            // A beacon belongs to no session, so it is handled before anything the engine
            // would do with it. It is reported and never answered: a channel where every
            // beacon drew a reply would be unusable.
            if let Some(caller) = beacon_callsign(&decoded) {
                self.stats.beacons_heard += 1;
                self.busy.mark_frame(now, detected);
                self.note(
                    "beacon",
                    &format!("heard {caller} at {:.1} dB", decoded.frame.snr_3k_db()),
                );
                continue;
            }
            let container = [Container::Data, Container::Control][usize::from(control)];
            let start = (origin + decoded.frame.start()) as f64 / fs;
            let frame_s = decoded.frame.samples(&air) as f64 / fs;
            // counted for the record; the engine sees it as it sees any frame (a slot is the
            // only place the other station transmits, so a fragment is on no grid)
            self.fragment_of_own_tx(&decoded, origin, &air);
            let decoded_ok = decoded.ok();
            // a frame the sound card's capture lost audio inside of says nothing of the path:
            // two of WC4Y's rung-3 frames at -1 to -4 dB, where that rung decodes 9 dB lower,
            // were lost that way, and the failure stepped the recommendation down (ADR-0076)
            let trusted = trusted_measurement(decoded.frame.mode_confidence(), detected)
                && !self.capture_skipped_within(start, start + frame_s);
            // A frame that decoded is real whatever its acquisition looked like, and a weak
            // one at the floor may have acquired below the gate below — it counts here. One
            // that did not decode is a soft frame the engine may still combine, and it says
            // nothing about the channel: a phantom gets this far too.
            if decoded_ok {
                self.busy.mark_frame(now, detected);
            }
            // a call or a probe to this station while the host program attached says not to
            // answer (`LISTEN OFF`, VARA's default until it says otherwise)
            if self.unanswered(&decoded) {
                continue;
            }
            // A call to this station in the narrower bandwidth: the station moves to it before
            // the engine sees the call, which it then answers on the caller's air (ADR-0026).
            // The frame is the tone floor's, the same on both airs, and its rung is the same.
            let mut rung = rung;
            let narrower = self.narrower_call(&decoded);
            // a call or probe it will not answer as a call, for the panel to say (ADR-0035)
            if narrower.is_none() {
                self.note_mismatch(&decoded);
            }
            // a call or an answer in another link protocol, which cannot connect (ADR-0041)
            self.note_version(&decoded);
            if let Some(caller) = narrower {
                self.move_to(500, BandwidthWhy::Call(caller));
                rung = decoded.frame.rung(&self.air()).unwrap_or(rung);
            }
            let asked = asked_by(decoded.payload.as_deref(), &self.engine.callsigns);
            let frame = PhyFrame {
                container,
                t_start: start,
                t_end: start + frame_s,
                rung,
                frame: decoded.frame,
                modem: Rc::clone(&self.decoder),
                trusted,
            };
            self.hand_over(&frame, asked, decoded_ok, now);
        }

        self.heed_preambles(&preambles, origin, now);
        self.pump();
    }

    /// The answer gap now: the operator's, or the one learned for the station this one would
    /// answer — the other end of the session or the call, or the last to call or probe it —
    /// when that is longer (ADR-0054). The engine waits it out after that station's frames.
    #[must_use]
    pub fn answer_gap_s(&self) -> f64 {
        let whom = if self.engine.state() == State::Idle {
            self.gaps.last_asker()
        } else {
            Some(self.engine.remote_call.as_str())
        };
        let learned = whom.and_then(|call| self.gaps.gap_s(call)).unwrap_or(0.0);
        self.config.answer_gap_s.max(learned)
    }

    /// A gap the stations-heard list kept from an earlier run (ADR-0054).
    pub fn remember_gap(&mut self, callsign: &str, gap_s: f64) {
        self.gaps.remember(callsign, gap_s);
    }

    /// The learned gaps that changed since the last call, for the stations-heard list.
    pub fn take_gap_changes(&mut self) -> Vec<LearnedGap> {
        self.gaps.take_changes()
    }

    /// Give a frame to the engine, and learn from what it did with it (ADR-0054).
    fn hand_over(
        &mut self,
        frame: &PhyFrame,
        asked: Option<(Asked, String)>,
        decoded: bool,
        now: f64,
    ) {
        let was_connected = self.engine.connected();
        let probes_answered = self.engine.stats.probes_answered;
        self.engine.on_frame(frame, now);
        self.learn_gap(asked, was_connected, probes_answered, decoded, now);
    }

    /// What a frame just given to the engine says about the answers this station gives
    /// (ADR-0054): a call or probe from a station whose last one was answered says the answer
    /// was lost; one the engine has just answered is watched; a frame of a session the station
    /// accepted says the acceptance was heard.
    fn learn_gap(
        &mut self,
        asked: Option<(Asked, String)>,
        was_connected: bool,
        probes_answered: usize,
        decoded: bool,
        now: f64,
    ) {
        let mut changed = Vec::new();
        match asked {
            Some((Asked::Call, from)) => {
                changed.extend(self.gaps.called(&from, now));
                // a first call accepted, or a call heard again in the session it opened, which
                // the engine answers with its acceptance again: each is watched, and each one
                // the caller does not hear is a sign of its own
                if self.engine.connected() {
                    let ours = self.engine.remote_call.eq_ignore_ascii_case(&from);
                    if !was_connected || ours {
                        self.gaps.accepted(&from, now);
                    }
                }
                self.gaps.asked_by(from);
            }
            Some((Asked::Probe, from)) => {
                changed.extend(self.gaps.probed(&from, now));
                if self.engine.stats.probes_answered > probes_answered {
                    self.gaps.probe_answered(&from, now);
                }
                self.gaps.asked_by(from);
            }
            None if decoded && self.engine.connected() => {
                let remote = self.engine.remote_call.clone();
                changed.extend(self.gaps.session_went_on(&remote));
            }
            None => {}
        }
        for learned in changed {
            self.note(
                "gap",
                &format!(
                    "answer gap for {} now {:.0} ms: {}",
                    learned.callsign,
                    learned.gap_s * 1000.0,
                    learned.why
                ),
            );
            self.gaps.changed(learned);
        }
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
            bandwidth_hz: None,
            follows: frame.follows(),
            capture_skip: false,
        };
        let span_start = (self.rx_origin + frame.start()) as f64 / self.config.params.fs_baseband;
        let span_end =
            span_start + frame.samples(&self.air()) as f64 / self.config.params.fs_baseband;
        report.capture_skip = self.capture_skipped_within(span_start, span_end);
        // a frame that belongs to the session is the other station's; one with another
        // session id is somebody else's business and stays unattributed
        let ours = |session: u8| {
            (self.engine.connected() && session == self.engine.session())
                .then(|| self.engine.remote_call.clone())
        };
        if control {
            if let Some(payload) = payload {
                report.control = crate::record::describe_control(payload);
                // a control frame of protocol 8 carries the session's low seven bits
                if let Ok(control) = GrantFrame::decode(payload) {
                    report.from = ours(self.engine.session())
                        .filter(|_| control.session == self.engine.session() & 0x7F);
                }
            }
        } else if let Some((header, body)) = payload.and_then(|p| decode_data(p).ok()) {
            match header.kind {
                DataKind::Beacon => {
                    report.kind = "beacon";
                    report.from = unpack_callsign(&body).ok();
                    report.bandwidth_hz = beacon_bandwidth_hz(&body);
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
                        report.bandwidth_hz = stated_bandwidth_hz(connect.caps);
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
                        report.bandwidth_hz = stated_bandwidth_hz(probe.caps);
                    }
                }
                DataKind::Data => report.from = ours(header.session),
                DataKind::Measure => {
                    report.kind = "measure";
                    report.from = ours(header.session);
                }
                DataKind::Datagram => report.kind = "datagram",
            }
        }
        self.learn_from(&report, now);
        self.note_other_session(control, payload, now);
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
        self.note_session_frame(&report, rung, now);
        self.reports.push(report);
    }

    /// A frame the session's other station sent, decoded: the session's notes for the history,
    /// and the rung its data came in at for the speed chart.
    fn note_session_frame(&mut self, report: &FrameReport, rung: usize, now: f64) {
        let Some(notes) = &mut self.session_notes else {
            return;
        };
        if !report.decoded || report.from.as_deref() != Some(notes.remote.as_str()) {
            return;
        }
        notes.snr_db = Some(report.snr_db);
        notes.best_snr_db = Some(
            notes
                .best_snr_db
                .map_or(report.snr_db, |b| b.max(report.snr_db)),
        );
        if report.kind == "data" {
            notes.top_rung_heard = Some(notes.top_rung_heard.map_or(rung, |t| t.max(rung)));
            self.data_heard = Some((now, rung));
        }
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
            // the other station's disconnect is the last word on how it heard this one, and
            // the only one a station that only received gets (ADR-0021)
            heard_there_db: self.engine.ended_peer_snr_db().or(notes.heard_there_db),
            top_rung_sent: notes.top_rung_sent,
            top_rung_heard: notes.top_rung_heard,
            test: self.test_running(),
            host: notes.host,
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
        // the contact its setup began is this session now, identified at its end (ADR-0075)
        self.identifier.setup_at = None;
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
        self.session_direction = Some(if detail.ends_with("(master)") {
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
            caller: detail.ends_with("(master)"),
            frequency_hz: self.frequency.value,
            acked_at_start: self.engine.stats.bytes_acked,
            host: self.host_attached(),
            ..SessionNotes::default()
        });
        self.moved.clear();
    }

    /// A session ended, however it ended: its identifier falls due, it joins the history,
    /// what it did not deliver is said to be undelivered, and its coders go.
    fn session_ended(&mut self, detail: &str) {
        // A session that came up is identified at its end by the engine, the last thing each
        // station sends for it (ADR-0075) — except one the same station ended by calling again
        // (ADR-0056), which goes on in the session its call opens. A call that went nowhere is
        // a contact that went no further than its setup: identified on its own after
        // `SETUP_ID_WAIT_S` idle (ADR-0055 §4).
        if self.link.is_some() {
            self.identifier.setup_at = None;
        }
        if detail == "no answer" {
            let called = self.engine.remote_call.clone();
            let now = self.now();
            self.suggest_kin(&called, now);
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
            self.refresh_identifier_length();
            self.note_required_identifier();
        }
    }

    /// Whether a session's closing identifier is still to go out (ADR-0075): owed by the
    /// engine, handed over and queued, or on the air now.
    #[must_use]
    pub fn closing_identifier_pending(&self) -> bool {
        self.engine.identifier_owed()
            || self
                .pending
                .iter()
                .any(|next| matches!(next, Outgoing::Link(tx) if tx.identifier_s > 0.0))
            || self.keying.is_some_and(|window| window.identifier)
    }

    /// Say when a session's closing identifier has gone out — or will not — so that a host
    /// program hears `DISCONNECTED` once nothing more of the session is to go on the air
    /// (ADR-0075).
    fn note_closing_identifier(&mut self) {
        let pending = self.closing_identifier_pending();
        if self.identifier.closing && !pending {
            let outcome = self
                .identifier
                .outcome
                .take()
                .unwrap_or_else(|| "the closing identifier is not sent".to_owned());
            self.note("identified", &outcome);
        }
        self.identifier.closing = pending;
    }

    /// Take what the engine has decided and act on it.
    fn pump(&mut self) {
        let mut connected = false;
        for action in self.engine.drain() {
            match action {
                Action::Transmit {
                    frames,
                    cause,
                    not_after,
                    not_before,
                    identifier_s,
                    listen_first,
                    ..
                } => {
                    let origin = Origin {
                        cause,
                        queued_s: self.now(),
                    };
                    // A transmission the engine keys at once — the master's, a call, a probe —
                    // it reckons on the air its keying latency from now, and places what it
                    // grants from there. That latency counts the typical render and keying
                    // command, so one that goes faster than typical waits for it: on the air no
                    // sooner than the engine reckoned, never early (`latency.rs`).
                    let not_before =
                        not_before.or_else(|| Some(self.now() + self.engine_latency_s()));
                    let slotless = listen_first || starts_an_exchange(&frames);
                    self.pending.push_back(Outgoing::Link(LinkTx {
                        frames,
                        origin,
                        window: KeyWindow {
                            not_before,
                            not_after,
                            slotless,
                            ..KeyWindow::default()
                        },
                        identifier_s,
                        listen_first,
                    }));
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
                    let session_recorded = self.session_notes.as_ref().is_some_and(|n| n.recorded);
                    let was_slave = self.session_notes.as_ref().is_some_and(|n| !n.caller);
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
                    let record_this = self.config.record_auto
                        || (self.config.record_host_sessions && self.host_attached());
                    if (record_this || session_recorded) && !self.test_running() {
                        if name == "connected" && record_this {
                            // a note given for the next recording wins; otherwise the
                            // standing one, which is what an unattended station has
                            let notes = self.record_notes.take().or_else(|| {
                                (!self.config.record_notes.is_empty())
                                    .then(|| self.config.record_notes.clone())
                            });
                            match self.start_recording(None, notes.as_deref()) {
                                Ok(_) => {
                                    if let Some(notes) = &mut self.session_notes {
                                        notes.recorded = true;
                                    }
                                }
                                Err(error) => {
                                    self.note("error", &format!("could not record: {error}"));
                                }
                            }
                        } else if name == "disconnected" {
                            self.stop_recording_after_identifiers(was_slave);
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
        self.note_closing_identifier();
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
    /// transmission this station starts on its own, when the operator asked to wait for a
    /// clear one.
    ///
    /// Under granted slots (ADR-0069) nothing of a session's waits for the channel, and
    /// nothing answered in a slot does: the master keys only once the slot it granted is over,
    /// the slave only in its slot, and a held answer would only miss it — nor does the master's
    /// closing identifier, in time the closing exchange left it (ADR-0075). What does wait is
    /// what this station starts outside a session — a call, a probe, a beacon, a datagram, the
    /// Morse identifier on its own — and that only while `wait_for_clear` says so; an
    /// identifier on its own waits for busy too, for at most [`OTHER_ID_WAIT_MAX_S`].
    fn held_for_busy(&mut self, now: f64) -> bool {
        self.drop_beacons_in_session();
        let Some(next) = self.pending.front() else {
            return true;
        };
        let in_turn = match next {
            // a transmission of the engine's waits for the channel only when it starts an
            // exchange: a call or a probe, a retry included. Everything else belongs to an
            // exchange already under way — an answer in its slot, the master's next
            // transmission, a DISC, an abort's among them, which the engine sends once it is
            // Idle — and a slot is used or forfeited. Judged on the engine's state, an abort's
            // DISC waited 29 s for a contest to clear and the other station never heard it
            // (10-step-3.md §3.3)
            Outgoing::Link(tx) => !tx.listen_first && !starts_an_exchange(&tx.frames),
            // silence radiates nothing, so a keying test does not wait for the channel; a
            // datagram has had its own channel access already: the client's DCD and persistence
            Outgoing::Audio { silent: true, .. } | Outgoing::Datagram { .. } => true,
            _ => false,
        };
        let alone = matches!(next, Outgoing::Identifier)
            || matches!(next, Outgoing::Link(tx) if tx.listen_first);
        let waited_out = self
            .held_from
            .is_some_and(|from| now - from >= OTHER_ID_WAIT_MAX_S);
        let busy_for_id = alone && self.busy.busy(now) && !waited_out;
        // an identifier waits for a clear channel no longer than the wait above: §97.119 asks
        // for it at the end of the communication, and a host program waits for it
        let id_waited_out = alone && waited_out;
        let polite =
            !in_turn && self.config.wait_for_clear && !self.channel_clear(now) && !id_waited_out;
        if !in_turn && (polite || busy_for_id) {
            self.stats.deferred_for_busy += 1;
            // a call's next try, a probe's wait move with it, or a try fires against a call
            // that has not left yet and the two go out back to back when the channel clears
            if let Some(since) = self.held_since {
                self.engine.on_tx_delayed(now - since);
            }
            self.held_since = Some(now);
            self.held_from.get_or_insert(now);
            return true;
        }
        self.last_hold_s = self.held_from.map_or(0.0, |from| now - from);
        self.held_since = None;
        self.held_from = None;
        false
    }

    /// A beacon still waiting when a session starts — held for a busy channel, and the busy
    /// channel was the call — is dropped: a beacon is for a station nobody is talking to, and
    /// sent now it would go into the middle of the session (ADR-0038). A session refuses a new
    /// one already ([`Self::beacon`]); this is the one queued before it began.
    fn drop_beacons_in_session(&mut self) {
        if self.engine.state() == State::Idle || !self.beacon_waiting() {
            return;
        }
        self.pending
            .retain(|next| !matches!(next, Outgoing::Frames(frames, _) if is_beacon(frames)));
        self.held_since = None;
        self.held_from = None;
        self.note("beacon", "dropped: a session started before it went out");
    }

    /// A call, a probe or its answer outside a session sets up a contact: one that goes no
    /// further is identified on its own (ADR-0055). An acceptance opens a session, which the
    /// engine identifies at its end.
    fn note_setup(&mut self, now: f64, frames: &[aether_link::TxFrame]) {
        if sets_up_contact(frames) && matches!(self.engine.state(), State::Idle | State::Connecting)
        {
            self.identifier.setup_at = Some(now);
            self.identifier.setup_call.clone_from(&self.engine.my_call);
        }
    }

    /// Take the transmission at the head of the queue — judged and allowed — and render it.
    fn render_next(&mut self, now: f64) {
        let Some(outgoing) = self.pending.pop_front() else {
            return;
        };
        let cuttable = matches!(outgoing, Outgoing::Drive(_));
        let mut link: Option<(KeyWindow, f64)> = None;
        let (frames, origin) = match outgoing {
            // the engine's closing identifier on its own (ADR-0075)
            Outgoing::Link(tx) if tx.frames.is_empty() => {
                return self.render_link_identifier(now, &tx);
            }
            Outgoing::Link(tx) => {
                link = Some((tx.window, tx.identifier_s));
                (tx.frames, tx.origin)
            }
            Outgoing::Frames(frames, origin) => (frames, origin),
            Outgoing::Drive(frames) => (frames, Origin::station("drive")),
            Outgoing::Datagram {
                frames,
                reference,
                last,
            } => {
                if last {
                    self.datagrams.on_air = Some(datagrams::OnAir { reference });
                }
                (frames, Origin::station("datagram"))
            }
            // handled above, before anything is popped
            Outgoing::Pause { .. } => return,
            Outgoing::Identifier => return self.render_identifier_tagged(now),
            Outgoing::Audio { samples, tone, .. } => {
                return self.render_audio_tagged(now, samples, tone);
            }
        };
        let ids_before = self.stats.cw_ids;

        if link.is_some()
            && let Some(rung) = frames
                .iter()
                .filter(|f| {
                    // session data only: a call's acceptance is a DATA frame too
                    f.container == Container::Data
                        && decode_data(&f.payload)
                            .is_ok_and(|(header, _)| header.kind == DataKind::Data)
                })
                .map(|f| f.mode)
                .max()
        {
            self.data_sent = Some((now, rung));
        }
        self.record_sent(&frames, now);
        if is_beacon(&frames) {
            self.beacon_sent();
        }
        if self.config.record_tx_audio {
            self.tx_capture = Some(crate::record::TxCapture::begin(
                self.config.params.audio_rate as u32,
                self.config.tx_level,
                self.config.key_lead_s,
                self.tail_s(),
                &frames,
            ));
        }

        let Some((baseband, first_frame)) = self.modulate(&frames) else {
            // nothing of an engine transmission goes out: the engine is told, as for a drop
            if link.is_some() {
                self.engine.on_tx_dropped(now);
            }
            return;
        };

        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.tail_s() * audio_rate) as usize;
        // flush the chain after the burst, or its filters keep the tail of the last frame
        let mut rendered = self.to_audio.process(&baseband);
        rendered.extend(self.to_audio.flush());

        // at unit level: the transmit level is applied as the audio leaves
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        if let Some((window, identifier_s)) = link {
            self.note_setup(now, &frames);
            // where in the transmission its header lies, for a cut that may come before it can
            // be read
            let header =
                (frames.first().map(|f| f.container) == Some(Container::Control)).then(|| {
                    let start = self.playback.len();
                    let len = rendered.len() * first_frame / baseband.len();
                    (start, start + len)
                });
            self.playback.extend(rendered);
            let frames_samples = self.playback.len();
            // Nothing else follows a link transmission's frames — the other station's slot
            // starts after them, or this station's own after its answer's (ADR-0069) — but the
            // closing identifier behind a DISC_ACK, in the time every DISC grants it: the last
            // thing this station sends for the session (ADR-0075)
            let carried = self.link_identifier(now, audio_rate, identifier_s);
            self.keying = Some(KeyWindow {
                identifier: carried,
                frames_samples,
                trailing_samples: self.playback.len() - frames_samples,
                header_samples: header,
                ..window
            });
        } else {
            // a beacon, a datagram, a drive burst: no identifier (ADR-0075)
            self.playback.extend(rendered);
            self.keying = None;
        }
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.playing_test = cuttable;
        let with_id = self.stats.cw_ids > ids_before;
        self.tx_tag = Some(self.tag(now, origin, &frames, with_id));
    }

    /// The frames' baseband, back to back, and the first frame's share of it; `None` when there
    /// is nothing to send.
    fn modulate(&mut self, frames: &[aether_link::TxFrame]) -> Option<(Vec<Complex>, usize)> {
        let mut baseband: Vec<Complex> = Vec::new();
        let mut first = 0;
        for frame in frames {
            let burst = match frame.container {
                // a rung of the ladder: the tone floor's, at its peak, or an OFDM mode
                Container::Data => {
                    // an OFDM frame says how many of the burst follow it (ADR-0041)
                    self.transmitter.rung_burst_following(
                        &frame.payload,
                        frame.mode,
                        frame.rv,
                        frame.follows,
                    )
                }
                Container::Control => {
                    self.transmitter
                        .control_burst_of(&frame.payload, frame.rv, frame.floor)
                }
            };
            match burst {
                Ok(samples) => {
                    baseband.extend(samples);
                    if first == 0 {
                        first = baseband.len();
                    }
                }
                // The engine only ever asks for payloads it sized from this same table, so
                // this cannot happen without a bug; dropping the frame keeps a bug off the
                // air rather than transmitting something malformed.
                Err(error) => {
                    self.events
                        .push(format!("error:cannot modulate frame: {error}"));
                    return None;
                }
            }
        }
        (!baseband.is_empty()).then_some((baseband, first))
    }

    /// [`Self::render_identifier`], with its account of the air.
    fn render_identifier_tagged(&mut self, now: f64) {
        self.render_identifier(now);
        self.tx_tag = Some(self.tag(now, Origin::station("identifier"), &[], true));
    }

    /// [`Self::render_audio`], with its account of the air.
    fn render_audio_tagged(&mut self, now: f64, samples: Vec<f32>, tone: bool) {
        let what = if tone { "tune" } else { "keying test" };
        self.render_audio(samples, tone);
        self.tx_tag = Some(self.tag(now, Origin::station(what), &[], false));
    }

    /// One line of JSON for the account of the air (ADR-0069 §5): why this transmission was
    /// queued, how long it waited, what the engine believed, and what it carries — control
    /// frames by kind and flags, data frames by kind, rung, RV and count. A harness puts it on
    /// the air beside the audio (`AudioIo::annotate`), so a collision can be put down to the
    /// two transmissions in it without either recording; nothing on the air reads it.
    fn tag(
        &self,
        now: f64,
        origin: Origin,
        frames: &[aether_link::TxFrame],
        with_id: bool,
    ) -> String {
        let described: Vec<serde_json::Value> = frames
            .iter()
            .map(|frame| match frame.container {
                Container::Control => match GrantFrame::decode(&frame.payload) {
                    Ok(control) => serde_json::json!({
                        "c": "control",
                        "kind": format!("{:?}", control.kind),
                        "count": control.count,
                        "level": control.level,
                        "floor": frame.floor,
                    }),
                    Err(_) => serde_json::json!({"c": "control", "kind": "?"}),
                },
                Container::Data => {
                    let kind = decode_data(&frame.payload)
                        .map_or_else(|_| "?".to_owned(), |(h, _)| format!("{:?}", h.kind));
                    serde_json::json!({
                        "c": "data",
                        "kind": kind,
                        "mode": frame.mode,
                        "rv": frame.rv,
                        "follows": frame.follows,
                    })
                }
            })
            .collect();
        serde_json::json!({
            "cause": origin.cause,
            "waited_s": if origin.queued_s.is_finite() {
                Some((now - origin.queued_s).max(0.0))
            } else {
                None
            },
            "held_s": self.last_hold_s,
            "state": format!("{:?}", self.engine.state()),
            "role": format!("{:?}", self.engine.role()),
            "session": self.engine.session(),
            "cw_id": with_id,
            "frames": described,
        })
        .to_string()
    }

    /// The accounts of the transmissions keyed since the last call, oldest first: the run
    /// loop hands each to the audio device ahead of the transmission's first samples.
    pub fn take_tags(&mut self) -> Vec<String> {
        std::mem::take(&mut self.tx_tags)
    }

    /// A frame found: into the recording, or kept for the next one's first seconds.
    fn keep_frame(&mut self, record: crate::record::FrameRecord, now: f64) {
        if let Some(recording) = &mut self.recording {
            recording.frame(record);
            return;
        }
        self.preroll_frames.push_back(record);
        while self
            .preroll_frames
            .front()
            .is_some_and(|f| f.t_s < now - PREROLL_S)
        {
            self.preroll_frames.pop_front();
        }
    }

    /// What a frame says of its sender: the bandwidth it runs, which is what a call to it
    /// should be made in, and — when it decoded — that it was heard now (ADR-0057).
    fn learn_from(&mut self, report: &FrameReport, now: f64) {
        let Some(from) = report.from.clone() else {
            return;
        };
        if let Some(hz) = report.bandwidth_hz {
            self.learn_bandwidth(&from, hz as usize);
        }
        if report.decoded {
            self.note_sender(&from, now);
        }
        // a call or probe to this station is said, heard or not answered: WC4Y's probe of
        // 2026-10-10 went unanswered, and nothing in KK4ODA-1's files said whether it had been
        // heard at all (the engine's own event says when one is answered)
        if report.decoded
            && matches!(report.kind, "connect" | "probe")
            && report
                .to
                .as_ref()
                .is_some_and(|to| self.engine.callsigns.contains(to))
        {
            let what = if report.kind == "probe" {
                "probe"
            } else {
                "call"
            };
            let detail = format!(
                "{what} from {} to {} at {:.1} dB",
                report.from.as_deref().unwrap_or("?"),
                report.to.as_deref().unwrap_or("?"),
                report.snr_db
            );
            self.note("heard", &detail);
        }
    }

    /// The Morse identifier on its own, inside the keying's lead and tail, for a contact that
    /// went no further than its setup (ADR-0055 §4): as the callsign the setup went as.
    fn render_identifier(&mut self, now: f64) {
        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.tail_s() * audio_rate) as usize;
        let call = self.identifier.setup_call.clone();
        // nothing to say after all — no identifier for that callsign, or the rules refuse it
        // now: key nothing
        let Some(audio) = self.identifier_audio_for(&call, now, audio_rate) else {
            return;
        };
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.playback.extend(audio);
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.identified(now);
    }

    /// The engine's closing identifier on its own (ADR-0075): a master's once the last slot it
    /// granted is over, or a station's no slot could be had for. An engine transmission like any
    /// other, followed to its end; one with nothing to send — the rules refuse it now, or the
    /// host program that keys the radio has gone — is reported done at once, and the log says
    /// why.
    fn render_link_identifier(&mut self, now: f64, tx: &LinkTx) {
        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.tail_s() * audio_rate) as usize;
        let call = self.engine.my_call.clone();
        let audio = if self.unkeyable_identifier() {
            None
        } else {
            self.identifier_audio_for(&call, now, audio_rate)
        };
        let Some(audio) = audio else {
            self.identifier
                .outcome
                .get_or_insert_with(|| "the closing identifier is not sent".to_owned());
            self.engine.on_tx_done(now, None);
            self.pump();
            return;
        };
        let room = (tx.identifier_s * audio_rate).round() as usize;
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        let frames_samples = self.playback.len();
        self.playback.extend(audio.into_iter().take(room.max(1)));
        self.keying = Some(KeyWindow {
            identifier: true,
            frames_samples,
            trailing_samples: self.playback.len() - frames_samples,
            ..tx.window
        });
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.identified(now);
        let sent = crate::cwid::sent_as(&call);
        self.identifier.outcome = Some(format!("the closing identifier went out as {sent}"));
        self.tx_tag = Some(self.tag(now, tx.origin, &[], true));
    }

    /// Whether nothing would key the radio for an identifier now: the host program keys it and
    /// none is attached. Said in the log, once a session ends with an identifier owed.
    fn unkeyable_identifier(&mut self) -> bool {
        if !self.config.keyed_by_host || self.host_attached() {
            return false;
        }
        let why = "the closing identifier did not go out: the host program keys the radio, and \
                   none is attached";
        self.identifier.outcome = Some(why.to_owned());
        self.note("identifier", why);
        true
    }

    /// Raw audio goes out as it is, inside the same keying and lead and tail as a burst, so
    /// a keying test exercises exactly the path a transmission uses.
    fn render_audio(&mut self, samples: Vec<f32>, tone: bool) {
        let audio_rate = self.config.params.audio_rate as f64;
        let lead = (self.config.key_lead_s * audio_rate) as usize;
        let tail = (self.tail_s() * audio_rate) as usize;
        self.playback.extend(std::iter::repeat_n(0.0f32, lead));
        self.playback.extend(samples);
        self.playback.extend(std::iter::repeat_n(0.0f32, tail));
        self.playing_test = tone;
        if self.config.record_tx_audio {
            self.tx_capture = Some(crate::record::TxCapture::begin(
                self.config.params.audio_rate as u32,
                self.config.tx_level,
                self.config.key_lead_s,
                self.tail_s(),
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
    /// The engine's signal and the receive indicator still take the gate. Under granted slots
    /// an announcement changes nothing in a session (ADR-0069 §2.5); a caller or a prober takes
    /// a trusted one as carrier, its next try waiting past it. An OFDM preamble's threshold
    /// sits at the statistic's noise maximum, so its acquisitions take
    /// [`DETECT_CONFIDENCE_TRUSTED`]; a tone frame (ADR-0013) is announced only once its
    /// first sync block has cleared a threshold set over the noise maximum of that block and
    /// beaten its neighbours for three symbols, which is the gate already.
    fn heed_preambles(&mut self, preambles: &[aether_phy::PendingFrame], origin: usize, now: f64) {
        let air = self.air();
        let fs = self.config.params.fs_baseband;
        for pending in preambles {
            let trusted = pending.tone || pending.detect_confidence >= DETECT_CONFIDENCE_TRUSTED;
            let start_s = (origin + pending.start) as f64 / fs;
            // a frame arriving: none of it is the noise floor, heeded or not (ADR-0076)
            self.busy
                .explain(start_s, (origin + pending.end) as f64 / fs);
            // every announcement, heeded or not, so a frame can be placed against the key's
            // release and a phantom just after it seen (ADR-0037)
            if let Some(recording) = &mut self.recording {
                let detail = format!(
                    "ago {:.3} frame_s {:.3} tone {} confidence {:.2} heeded {}",
                    now - start_s,
                    pending.end.saturating_sub(pending.start) as f64 / fs,
                    u8::from(pending.tone),
                    pending.detect_confidence,
                    u8::from(trusted),
                );
                recording.event(
                    now,
                    "preamble",
                    &detail,
                    &format!("{:?}", self.engine.state()),
                );
            }
            if !trusted {
                continue;
            }
            // the frame named itself — an OFDM preamble its layout, a tone frame its kind:
            // its own air time, not a guess from the peer's last mode — a floor frame after
            // ordinary connect frames is five times as long (ADR-0012)
            let frame_s = pending.end.saturating_sub(pending.start) as f64 / fs;
            self.rx_until = self.rx_until.max(now + frame_s.max(air.long.duration_s()));
            self.engine.on_preamble(start_s, now, Some(frame_s));
        }
    }

    /// Captured audio lost before the next block: the capture queue dropped its oldest while
    /// the run loop was behind (`AudioIo::capture_gap`).
    ///
    /// The station's clock is the audio it has heard, and the slots it answers in are on the
    /// air's: a clock that did not move on would run that far behind the air from then on, and
    /// every answer would radiate that late — past the keying gate, which reads the same clock
    /// (the scenario harness's slow machine, Step 2: 0.85 s dropped after a stall, the answer
    /// keyed over the caller's next transmission). It moves on by what was lost. The receiver
    /// starts again after the gap, as it does on a move of bandwidth — a frame across it is
    /// lost with the audio — and a recording gets silence for it, so a replay keeps time.
    pub fn skip_captured(&mut self, samples: usize) {
        if samples == 0 {
            return;
        }
        let params = self.config.params;
        let rate = params.audio_rate as f64;
        if let Some(recording) = &mut self.recording
            && let Err(error) = recording.captured(&vec![0.0; samples])
        {
            self.note("error", &format!("recording stopped: {error}"));
            self.recording = None;
        }
        self.audio_seen += samples;
        self.from_audio = AudioToBaseband::new(params);
        self.baseband_seen += (samples as f64 * params.fs_baseband / rate).round() as usize;
        self.receiver = StreamingReceiver::new(params, 6.0, true);
        self.rx_origin = self.baseband_seen;
        self.note(
            "audio",
            &format!(
                "{:.0} ms of captured audio lost: the clock moves on past it",
                samples as f64 * 1000.0 / rate
            ),
        );
    }

    /// Fold the receiver's noise spectrum into the passband monitor, a couple of times a
    /// second and only from audio heard while the channel was quiet: the detector has learned
    /// the floor, nothing is on the channel or arriving, this station is neither transmitting
    /// nor hearing its own tail, and there is real noise to measure. So what it learns is the
    /// receiver's own passband, not a signal that is on.
    ///
    /// Quiet at the moment of a look is not enough, on either side of the window. A spectrum
    /// is of the last [`SpectrumAnalyser::window_s`] of audio, so the channel must have been
    /// quiet that long already; and the busy detector calls a signal only up to
    /// [`BusyDetector::attack_s`] after it starts, so a spectrum waits that much longer and
    /// is folded in only if the quiet held. Without the wait the bench pair read a 650 Hz
    /// "filter" over flat noise (2026-09-26): a station stops being deaf to its own tail
    /// with the other's answer already arriving, and the one look a session left it between
    /// two bursts was the start of that answer — the tone floor's 400 Hz, turnaround after
    /// turnaround.
    fn sample_passband(&mut self, now: f64) {
        let quiet = self.busy.settled()
            && !self.busy.busy(now)
            && !self.transmitting
            && !self.receiving()
            && now >= self.deaf_until
            && self.busy.level_db > PASSBAND_MIN_LEVEL_DBFS;
        if !quiet {
            // the recent audio, and every spectrum still waiting, may hold whatever has just
            // been noticed
            self.passband_quiet_since = now;
            self.passband_pending.clear();
            return;
        }
        let attack = self.busy.attack_s();
        while self
            .passband_pending
            .front()
            .is_some_and(|&(taken, _)| now - taken >= attack)
        {
            if let Some((_, spectrum)) = self.passband_pending.pop_front() {
                self.passband.observe(&spectrum);
            }
        }
        // the window has to lie wholly in the quiet, and a block can straddle the end of the
        // deafness
        let quiet_s = now - self.passband_quiet_since.max(self.deaf_until);
        if now >= self.passband_next_s
            && quiet_s >= self.spectrum.window_s()
            && let Some(spectrum) = self.spectrum.compute()
        {
            self.passband_pending.push_back((now, spectrum));
            self.passband_next_s = now + PASSBAND_SAMPLE_S;
        }
    }

    /// Log a change of the busy state with the numbers that decided it.
    ///
    /// `busy_until` is one number extended by several paths, and when the indicator lights
    /// with nothing above the threshold the only useful question is which path did it. So
    /// every transition says: the level and floor at that moment, the threshold, and the
    /// reason the hold was last extended.
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
        // and the radio's gain, when its AGC holds it down: the floor follows it (ADR-0078),
        // and a reader of the log should know the noise moved for that reason
        let gain = self
            .busy
            .gain_db()
            .filter(|&g| g <= -RADIO_GAIN_SHOWN_DB)
            .map_or_else(String::new, |g| format!(" | radio gain {g:.0} dB"));
        let detail = format!(
            "{level:.1} dBFS | floor {floor:.1} | delta {:+.1} dB | margin {margin:.1} | \
             threshold {:.1} dBFS | {} | {why}{gain}",
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

    /// The Morse identifier went out now.
    fn identified(&mut self, _now: f64) {
        self.stats.cw_ids += 1;
        self.identifier.setup_at = None;
    }

    /// The Morse identifier's audio for `call` at the speed it goes out, judged as the CW
    /// emission it is (ADR-0018): behind a `DISC_ACK` it rides inside a transmission's keying, so
    /// the gate's leave for the transmission covers its data, not this. `None` when that
    /// callsign sends none, or the rules refuse it now — the refusal reported.
    fn identifier_audio_for(&mut self, call: &str, now: f64, audio_rate: f64) -> Option<Vec<f32>> {
        let cw = self.identifier_for(call)?;
        let cw = crate::cwid::CwId {
            level: CW_ID_RELATIVE_LEVEL,
            // an identifier keyed by an automatic device is sent no faster than the rules
            // allow (§97.119(b)(1): 20 wpm)
            wpm: self.sent_wpm(cw),
            ..cw
        };
        let identifier = Transmission {
            what: "the Morse identifier".to_owned(),
            kind: EmissionKind::Cw,
            audio: Edges::tone(cw.tone_hz, crate::regulatory::occupancy::CW_HALF_WIDTH_HZ),
            direction: self.session_direction.unwrap_or(Direction::Originate),
        };
        let situation = self.situation(false);
        let decision = self.policy.evaluate(&situation, &identifier);
        if !decision.allowed() {
            self.identifier.outcome = Some(format!(
                "the closing identifier is not sent: {}",
                decision.summary
            ));
            self.report_decision(&decision, now);
            return None;
        }
        let audio = cw.audio(&crate::cwid::sent_as(call), audio_rate);
        (!audio.is_empty()).then_some(audio)
    }

    /// How long the Morse identifier for `call` keeps the radio keyed, with the moment of
    /// silence that parts it from the frames; 0 when that callsign sends none. What a call or an
    /// acceptance under that callsign declares, so the master grants the slave's in the
    /// `DISC_ACK`'s slot and the slave waits out the master's (ADR-0075).
    fn identifier_length_for(&self, call: &str) -> f64 {
        let Some(cw) = self.identifier_for(call) else {
            return 0.0;
        };
        let rate = self.config.params.audio_rate as f64;
        let cw = crate::cwid::CwId {
            wpm: self.sent_wpm(cw),
            ..cw
        };
        cw.audio(&crate::cwid::sent_as(call), rate).len() as f64 / rate + CW_ID_GAP_S
    }

    /// Tell the engine how long the identifier is for each of the station's callsigns, after
    /// anything that may change it: on or off, its speed, the rules' limit on it, the callsigns.
    fn refresh_identifier_length(&mut self) {
        let lengths: Vec<(String, f64)> = self
            .engine
            .callsigns
            .iter()
            .map(|call| (call.clone(), self.identifier_length_for(call)))
            .collect();
        self.engine.set_identifiers(&lengths);
        self.engine
            .set_identifier_s(lengths.first().map_or(0.0, |(_, seconds)| *seconds));
    }

    /// The closing identifier the engine placed after a link transmission's frames — behind a
    /// `DISC_ACK`, in the time every DISC grants it (ADR-0075): the moment of silence that parts
    /// it from the frames and the identifier, inside `identifier_s`, which the engine counted
    /// in the transmission's air time. One the rules refuse, or that nothing would key, is left
    /// out: silence radiates nothing, and nothing of the session comes after it. Whether one
    /// went out.
    fn link_identifier(&mut self, now: f64, audio_rate: f64, identifier_s: f64) -> bool {
        if identifier_s <= 0.0 || self.unkeyable_identifier() {
            return false;
        }
        let call = self.engine.my_call.clone();
        let Some(audio) = self.identifier_audio_for(&call, now, audio_rate) else {
            return false;
        };
        let room = (identifier_s * audio_rate).round() as usize;
        let gap = ((CW_ID_GAP_S * audio_rate) as usize).min(room);
        self.playback.extend(std::iter::repeat_n(0.0f32, gap));
        self.playback.extend(audio.into_iter().take(room - gap));
        self.identified(now);
        let sent = crate::cwid::sent_as(&call);
        self.identifier.outcome = Some(format!("the closing identifier went out as {sent}"));
        true
    }

    /// A contact that went no further than its setup — a probe answered and no call after it,
    /// a call never answered, a probe this station answered and no call came — is identified
    /// on its own once the station has been idle [`SETUP_ID_WAIT_S`] (ADR-0055), as the callsign
    /// the setup went as: whenever that callsign identifies, a US one always (ADR-0075).
    fn advance_identifier(&mut self, now: f64) {
        let Some(at) = self.identifier.setup_at else {
            return;
        };
        if self.identifier_for(&self.identifier.setup_call).is_none() {
            self.identifier.setup_at = None;
            return;
        }
        if self.engine.state() != State::Idle
            || self.engine.probing()
            || self.transmitting
            || !self.pending.is_empty()
            || self.closing_identifier_pending()
            || now - at < SETUP_ID_WAIT_S
        {
            return;
        }
        self.identifier.setup_at = None;
        self.pending.push_back(Outgoing::Identifier);
    }

    /// Size the engine's transmissions to the key. The engine keeps the identifier's room
    /// itself, in the transmission it places one in (07-protocol-8.md §5.4), so the limit is
    /// the key's less its lead and tail. An explicit `max_burst_s` in the link settings stands.
    fn refresh_burst_cap(&mut self, _now: f64) {
        if self.config.link.max_burst_s.is_some() {
            return;
        }
        self.engine
            .set_max_burst_s(Some(burst_limit_s(&self.config)));
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

    /// The Morse identifier as the rules class it: a CW emission at its tone.
    fn identifier_transmission(&self, call: &str) -> Transmission {
        Transmission {
            what: "the Morse identifier".to_owned(),
            kind: EmissionKind::Cw,
            audio: Edges::tone(
                self.identifier_for(call)
                    .map_or(AUDIO_CENTER_HZ, |c| c.tone_hz),
                crate::regulatory::occupancy::CW_HALF_WIDTH_HZ,
            ),
            direction: self.session_direction.unwrap_or(Direction::Originate),
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
            // the engine's closing identifier on its own (ADR-0075)
            Outgoing::Link(LinkTx { frames, .. }) if frames.is_empty() => {
                Ok(self.identifier_transmission(&self.engine.my_call))
            }
            Outgoing::Link(LinkTx { frames, .. })
            | Outgoing::Frames(frames, _)
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
            Outgoing::Identifier => Ok(self.identifier_transmission(&self.identifier.setup_call)),
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
                Ok(DataKind::Measure) => {
                    let name = air.rungs.get(first.mode).map_or("", |r| r.name.as_str());
                    (
                        format!("a measurement at rung {} ({name})", first.mode),
                        session,
                    )
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
                let kind = GrantFrame::decode(&first.payload)
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
            Some(Outgoing::Link(..)) => {
                // the engine follows each transmission it hands over: this one never went
                self.engine.on_tx_dropped(now);
                if self.engine.state() != State::Idle {
                    self.note(
                        "regulatory",
                        "session halted: its next transmission is not lawful",
                    );
                    // nothing more of the session goes either
                    self.drop_link_pending();
                    self.engine.abort();
                }
                self.pump();
            }
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
        // a session this station heard holds the channel through the quiet between its
        // transmissions, which the busy detector does not bridge (10-step-3.md §3.2)
        self.busy.settled() && !self.busy.busy(now) && !self.reservation.holds(now)
    }

    /// A decoded frame of a session not this station's holds the channel ([`Self::other_session`]).
    fn note_other_session(&mut self, control: bool, payload: Option<&[u8]>, now: f64) {
        if let Some(heard) = payload.and_then(|p| self.other_session(control, p)) {
            self.reservation.heard(now, heard);
        }
    }

    /// What a decoded frame says of a session not this station's: a frame of one under way, or
    /// of one starting between two other stations, a DISC, a `DISC_ACK`. `None` for this
    /// station's own session — the last one too, whose stray frames are the engine's to answer
    /// — and for what belongs to no session: a beacon, a probe, a datagram.
    fn other_session(&self, control: bool, payload: &[u8]) -> Option<reserve::Heard> {
        let ours = self.engine.session();
        if control {
            let frame = GrantFrame::decode(payload).ok()?;
            if frame.session == ours & 0x7F {
                return None;
            }
            return Some(match frame.kind {
                aether_link::GrantKind::Disc => reserve::Heard::Leaving,
                aether_link::GrantKind::DiscAck => reserve::Heard::Over,
                _ => reserve::Heard::Session,
            });
        }
        let (header, body) = decode_data(payload).ok()?;
        match header.kind {
            DataKind::Data | DataKind::Measure if header.session != ours => {
                Some(reserve::Heard::Session)
            }
            DataKind::ConnectReq | DataKind::ConnectAck => {
                let connect = ConnectBody::decode(&body).ok()?;
                let mine = |call: &str| self.engine.callsigns.iter().any(|c| c == call);
                (!mine(&connect.src) && !mine(&connect.dst)).then_some(reserve::Heard::Session)
            }
            _ => None,
        }
    }
}

/// Whether a transmission sets up a contact — a call, an acceptance, a probe or its answer —
/// and so carries no identifier (ADR-0055).
fn sets_up_contact(frames: &[aether_link::TxFrame]) -> bool {
    frames.iter().any(|frame| {
        frame.container == Container::Data
            && decode_data(&frame.payload).is_ok_and(|(header, _)| {
                matches!(
                    header.kind,
                    DataKind::ConnectReq
                        | DataKind::ConnectAck
                        | DataKind::Probe
                        | DataKind::ProbeAck
                )
            })
    })
}

/// Whether a transmission starts an exchange on the station's own initiative — a call or a
/// probe, a retry included — and so waits for a clear channel. The rest of the engine's
/// transmissions belong to an exchange under way (10-step-3.md §3.3).
fn starts_an_exchange(frames: &[aether_link::TxFrame]) -> bool {
    frames.iter().any(|frame| {
        frame.container == Container::Data
            && decode_data(&frame.payload).is_ok_and(|(header, _)| {
                matches!(header.kind, DataKind::ConnectReq | DataKind::Probe)
            })
    })
}

/// Whether a queued burst is a beacon: one data frame of kind `Beacon`.
fn is_beacon(frames: &[aether_link::TxFrame]) -> bool {
    frames.len() == 1
        && frames[0].container == Container::Data
        && decode_data(&frames[0].payload).is_ok_and(|(header, _)| header.kind == DataKind::Beacon)
}

/// What in the queue ties the station to the dial it is on, named for the operator: all but a
/// beacon or a datagram, which only wait for a clear channel and may go on any.
fn holds_the_dial(next: &Outgoing) -> Option<&'static str> {
    match next {
        Outgoing::Frames(frames, _) if is_beacon(frames) => None,
        Outgoing::Datagram { .. } => None,
        Outgoing::Frames(..) | Outgoing::Link(..) => Some("an answer"),
        Outgoing::Identifier => Some("the Morse identifier"),
        Outgoing::Drive(_) | Outgoing::Pause { .. } => Some("a drive check"),
        Outgoing::Audio { tone: true, .. } => Some("a tuner tone"),
        Outgoing::Audio { .. } => Some("a keying test"),
    }
}

/// The bandwidth a beacon says its sender runs: the capability byte after its callsign
/// (ADR-0024). A beacon from before that carries none.
fn beacon_bandwidth_hz(body: &[u8]) -> Option<u32> {
    stated_bandwidth_hz(*body.get(aether_link::frames::CALL_BYTES)?)
}

/// The bandwidth a capability byte states, hertz.
fn stated_bandwidth_hz(caps: u8) -> Option<u32> {
    aether_link::frames::bandwidth_hz_of(caps).and_then(|hz| u32::try_from(hz).ok())
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

/// What a frame asks of this station, for the gaps it learns (ADR-0054).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Call,
    Probe,
}

/// Whether a decoded frame is a call or probe to one of `ours`, and from whom.
fn asked_by(payload: Option<&[u8]>, ours: &[String]) -> Option<(Asked, String)> {
    let (header, body) = decode_data(payload?).ok()?;
    let (asked, src, dst) = match header.kind {
        DataKind::ConnectReq => {
            let call = ConnectBody::decode(&body).ok()?;
            (Asked::Call, call.src, call.dst)
        }
        DataKind::Probe => {
            let probe = ProbeBody::decode(&body).ok()?;
            (Asked::Probe, probe.src, probe.dst)
        }
        _ => return None,
    };
    ours.contains(&dst).then_some((asked, src))
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
            .beacon(None)
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
    fn the_channel_reads_clear_when_the_radios_agc_lets_go() {
        // ADR-0078 through the station's own capture path: the radio's audio is band noise at
        // about −29 dBFS in a 500 Hz channel and, from 20 s to 40 s, a carrier at 700 Hz —
        // outside the channel — 30 dB over the noise in 3 kHz, which an AGC on AUTO holds 21 dB
        // down (beta.17's model: a step down, 100 ms of hold, 50 dB/s back). The station must
        // read the channel clear once the gain is back, not for the 71 s the floor took when it
        // had learned the noise the AGC held down.
        let mut station = Station::new(
            StationConfig {
                callsign: "KK4ODA".to_owned(),
                params: aether_phy::waveform::NARROW_500,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let rate = station.config.params.audio_rate as f64;
        // uniform white noise: 0.61 peak is 0.18 RMS, −29 dBFS in the channel's 480 Hz
        let amplitude = 0.61;
        let noise_3k = amplitude / 12f64.sqrt() * (3000.0 / (rate / 2.0)).sqrt();
        let carrier = noise_3k * 10f64.powf(30.0 / 20.0) * std::f64::consts::SQRT_2;
        let threshold = noise_3k * 10f64.powf(12.0 / 20.0);
        let (mut gain_db, mut held) = (0.0f64, 0.0f64);
        let mut state = 11u32;
        let block = (rate / 50.0) as usize;
        let mut n = 0usize;
        let mut busy_after = Vec::new();
        while (n as f64) < 80.0 * rate {
            let audio: Vec<f32> = (0..block)
                .map(|_| {
                    let t = n as f64 / rate;
                    n += 1;
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let noise = (f64::from(state >> 8) / f64::from(1u32 << 24) - 0.5) * amplitude;
                    let on = (20.0..40.0).contains(&t);
                    let tone = if on {
                        carrier * (2.0 * std::f64::consts::PI * 700.0 * t).sin()
                    } else {
                        0.0
                    };
                    let limit = if on {
                        20.0 * (threshold / carrier).log10()
                    } else {
                        0.0
                    };
                    if limit < gain_db {
                        (gain_db, held) = (limit, 0.1);
                    } else if held > 0.0 {
                        held -= 1.0 / rate;
                    } else {
                        gain_db = (gain_db + 50.0 / rate).min(0.0).min(limit);
                    }
                    ((noise + tone) * 10f64.powf(gain_db / 20.0)) as f32
                })
                .collect();
            station.capture(&audio).expect("capture");
            let now = n as f64 / rate;
            if now > 43.0 && station.channel_busy() {
                busy_after.push(now);
            }
        }
        assert!(
            busy_after.is_empty(),
            "the channel read busy {:.1} s after the carrier had gone, the last at {:.1} s",
            busy_after.len() as f64 * 0.02,
            busy_after.last().copied().unwrap_or(0.0)
        );
    }

    #[test]
    fn a_recording_traces_what_the_receiver_heard_after_the_key_came_up() {
        // ADR-0037: the WAV holds what the radio delivered after a transmission, and only
        // these lines say what the station made of it — where the key came up against the
        // sound card's clock, and the busy detector's level, floor and deafness after it.
        let dir = std::env::temp_dir().join(format!("aether-trace-{}", std::process::id()));
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
            .start_recording(Some("trace"), None)
            .expect("start the recording");
        station.set_drive(1).expect("set drive");
        let rate = station.config.params.audio_rate;
        let _ = drain_peak(&mut station, rate * 20);
        assert!(!station.transmitting(), "the drive burst never ended");
        // two seconds of the radio back on receive, in 20 ms blocks
        let block = rate / 50;
        let mut state = 7u32;
        for _ in 0..100 {
            let noise: Vec<f32> = (0..block)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (f64::from(state >> 8) / f64::from(1u32 << 24) - 0.5) as f32 * 0.02
                })
                .collect();
            station.capture(&noise).expect("capture");
        }
        let summary = station.stop_recording().expect("a recording");
        let sidecar: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&summary.sidecar).expect("sidecar"))
                .expect("json");
        let events = sidecar["events"].as_array().expect("events");
        let end = events
            .iter()
            .find(|e| e["event"] == "tx_end")
            .expect("a tx_end event beside the release");
        let detail = end["detail"].as_str().expect("detail");
        assert!(
            detail.contains("drains_at") && detail.contains("deaf_for"),
            "the release says where the card stood: {detail}"
        );
        let trace: Vec<&str> = events
            .iter()
            .filter(|e| e["event"] == "rx_trace")
            .map(|e| e["detail"].as_str().expect("detail"))
            .collect();
        assert!(
            trace.len() >= 30,
            "{} trace lines in two seconds",
            trace.len()
        );
        assert!(
            trace
                .iter()
                .all(|line| line.contains("power") && line.contains("floor")),
            "a trace line: {}",
            trace[0]
        );
        assert!(
            trace[0].ends_with("deaf 1"),
            "the first line is inside the deafness: {}",
            trace[0]
        );
        assert!(
            trace.last().is_some_and(|line| line.ends_with("deaf 0")),
            "the last is past it: {:?}",
            trace.last()
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
            "the waveform should peak well above the tone that sets the drive, not level \
             with it: {gap_db:.1} dB"
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

    /// A frame that did not decode is kept from the engine when more than
    /// [`UNDER_OWN_TX_S`] of it lay under this station's own transmission — the receiver was
    /// given silence there — and not when it only touches it, or lies clear of it.
    #[test]
    fn a_frame_under_the_stations_own_transmission_is_a_fragment() {
        let mut station = idle_station();
        let fs = station.config.params.fs_baseband;
        let s = |seconds: f64| (seconds * fs) as usize;
        station.baseband_seen = s(10.0);
        station.note_muted(s(0.5));
        station.baseband_seen += s(0.5);
        // muted in two blocks: one span
        station.note_muted(s(0.4));
        station.baseband_seen += s(0.4);
        assert_eq!(station.muted.len(), 1);
        assert_eq!(station.muted[0], (s(10.0), s(10.9)));
        // a frame acquired late, running on under the transmission
        assert!(station.under_own_tx(s(9.2), s(10.25)));
        // one that ended as the station keyed, within the receiver's slack
        assert!(!station.under_own_tx(s(8.9), s(10.03)));
        // one after it
        assert!(!station.under_own_tx(s(11.0), s(12.0)));
        // the spans kept are the latest
        for k in 0..MUTED_SPANS_KEPT + 2 {
            station.baseband_seen += s(1.0);
            station.note_muted(s(0.1) + k);
            station.baseband_seen += s(0.1) + k;
        }
        assert_eq!(station.muted.len(), MUTED_SPANS_KEPT);
        assert!(!station.under_own_tx(s(9.2), s(10.25)));
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
    fn the_link_trusts_what_the_receiver_acquired_with_confidence() {
        // ADR-0020, from ND1J's 40 m path (2026-09-25): the frame that read -11 dB between
        // frames decoding at +5 to +8 had chips at 1.06 and acquisition at 1.13 — a noise
        // trigger, whose SNR the rate controller must not take, nor its failure
        assert!(!trusted_measurement(1.06, 1.13));
        // the frames that failed at +6 to +9 dB had both well clear: real frames, and their
        // SNR is the path's
        assert!(trusted_measurement(2.66, 1.67));
        // a control frame's chips always read 1.0: its acquisition decides
        assert!(trusted_measurement(1.0, 1.7));
        assert!(!trusted_measurement(1.0, 1.2));
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
            middle: 0,
            follows: Some(0),
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
        /// Each station's sound card's playback clock: blocks taken from it so far, across
        /// every `run` — a sound card's clock does not restart between calls either. One a
        /// station each: one not run (`run_a_alone`) does not move, as a station's card does
        /// not play on while its daemon stands still — shared, `b` came back to a card 45 s
        /// ahead of its own audio, and its gate dropped every answer as late.
        played: [u64; 2],
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
                played: [0, 0],
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
                let start = self.played[0];
                self.a.device_played(start);
                self.played[0] += self.block as u64;
                let handed = self.a.playback(&mut out).expect("playback");
                self.a.device_drains_at(start + handed as u64);
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
                // time it takes the next, which is the clock the key is released against,
                // and it plays what a station handed it from the start of the block
                let [start_a, start_b] = self.played;
                self.a.device_played(start_a);
                self.b.device_played(start_b);
                self.played[0] += self.block as u64;
                self.played[1] += self.block as u64;
                let handed_a = self.a.playback(&mut from_a).expect("playback");
                let handed_b = self.b.playback(&mut from_b).expect("playback");
                self.a.device_drains_at(start_a + handed_a as u64);
                self.b.device_drains_at(start_b + handed_b as u64);

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
        assert_eq!(air.a.role(), Role::Master);
        assert_eq!(air.b.role(), Role::Slave);
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

    /// How long after the caller's key came up the called station keyed its acceptance, with
    /// `turnaround` declared by the caller's radio; and whether the session came up.
    fn acceptance_keyed_after(turnaround: f64) -> (f64, bool) {
        let mut air = Air::new(1.0, 0.0005);
        air.a = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                turnaround_s: turnaround,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        air.a.connect("KK4XYZ").expect("idle");
        let (mut released, mut keyed) = (None, None);
        air.run(60.0, |a, b| {
            if released.is_none() && a.stats.transmissions > 0 && !a.transmitting() {
                released = Some(a.now());
            }
            if keyed.is_none() && b.transmitting() {
                keyed = Some(b.now());
            }
            a.connected() && b.connected()
        });
        let up = air.a.connected() && air.b.connected();
        let released = released.expect("the call went out");
        (keyed.expect("the called station keyed") - released, up)
    }

    #[test]
    fn an_answer_waits_out_the_callers_turnaround() {
        // A station keyed by VOX — a SignaLink, whose DLY knob holds the key after the audio
        // stops — is still transmitting for a moment after its frame ends, and an acceptance
        // that started inside that moment never reached KE4QCM (2026-10-05). Under granted
        // slots the caller's radio declares how long it takes to hear again, and the slot it
        // grants starts no sooner (07-protocol-8.md §3): with 1.2 s declared, past the decode
        // budget and the keying's 0.65 s, the acceptance keys 0.55 s later than with none.
        let (prompt, up) = acceptance_keyed_after(0.0);
        assert!(up, "the session did not come up with no turnaround");
        let (late, up) = acceptance_keyed_after(1.2);
        assert!(up, "the session did not come up with a 1.2 s turnaround");
        // the harness moves in 85 ms blocks
        assert!(
            (late - prompt - 0.55).abs() < 0.1,
            "the acceptance keyed {prompt:.2} s after the call's release with no turnaround, \
             {late:.2} s with 1.2 s"
        );
    }

    #[test]
    fn a_faint_arrival_lights_nothing() {
        // an OFDM detection under the trusted confidence is the detector's noise as often as
        // not (OTA-2): it neither lights the receive indicator nor reaches the engine
        let mut station = idle_station();
        let now = station.now();
        let fs = station.config.params.fs_baseband;
        let start = ((now + 0.2) * fs) as usize;
        station.heed_preambles(
            &[aether_phy::PendingFrame {
                start,
                end: start + 1000,
                detect_confidence: 1.05,
                tone: false,
                control: false,
            }],
            0,
            now,
        );
        assert!(!station.receiving(), "a phantom lit the receive indicator");
    }

    #[test]
    fn a_receiving_station_that_disconnects_sends_its_disc_at_once() {
        // Found by the scenario harness on its first run (ADR-0042): the receiving station
        // pressed Disconnect as a message arrived, its DISC — sent in place of the burst's
        // acknowledgement (ADR-0023) — waited two seconds behind the busy detector's hold on the
        // burst it answered, the sender heard no answer and sent the burst again, and the two
        // were keyed at once. ND1J's "the disconnect button does not work", from the other side.
        // Under granted slots the caller is the master: its DISC goes in its own time, once the
        // slot that carried the message is over, and nothing holds it (ADR-0069).
        let message: Vec<u8> = (0..900u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        // noise a few tens of dB under the signal, and a quiet half minute for the busy
        // detectors to learn it: then a burst's energy, not only its decode, holds the channel
        let mut air = Air::new(1.0, 0.01);
        air.run(30.0, |_, _| false);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        // the called station sends; the caller disconnects the moment it has it all
        air.b.send(&message);
        air.run(120.0, |a, _| a.received_len() >= message.len());
        assert_eq!(air.a.received_len(), message.len());
        let had_it = air.a.now();
        air.a.disconnect();
        let mut keyed_at = None;
        air.run(60.0, |a, b| {
            if keyed_at.is_none() && a.transmitting {
                keyed_at = Some(a.now());
            }
            a.state() == State::Idle && b.state() == State::Idle
        });
        let keyed_at = keyed_at.expect("the DISC went out");
        assert!(
            keyed_at - had_it < 1.2,
            "the DISC keyed {:.2} s after the message was in",
            keyed_at - had_it
        );
        assert_eq!(
            air.b.engine().stats.frames_resent,
            0,
            "the sender heard no answer and sent the burst again"
        );
    }

    #[test]
    fn a_session_crosses_with_both_radios_slow_to_turn_around() {
        // both radios declare 0.8 s to hear again: every slot waits for the radio that granted
        // it, and no answer is late for its slot or dropped by the keying gate
        let message = b"Both stations waited out the other's radio before every answer.";
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            turnaround_s: 0.8,
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received().as_slice(), message.as_slice());
        for station in [&air.a, &air.b] {
            assert_eq!(station.stats.dropped_late, 0, "{}", station.config.callsign);
            assert_eq!(
                station.engine().stats.forfeits,
                0,
                "{}",
                station.config.callsign
            );
        }
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
        // the card, which held nothing, drains once it has played what it was handed
        station.device_drains_at(1_000_000 + handed as u64);
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

    #[test]
    fn the_key_waits_for_the_last_sample_though_the_card_played_on_while_the_burst_was_rendered() {
        // The run loop reads the card's clock and then, in the fill's first `playback`,
        // renders the burst and keys the radio before it queues the first block. A real card
        // plays silence meanwhile and its clock counts it, so every sample of the burst
        // leaves that much later than the reading plus its place in the burst — the last one
        // too. Released, and deaf, by the reading, the station came up that much early:
        // passes of 255–366 ms while transmitting on a loaded machine (a `[sim]` bench
        // daemon, 2026-09-25/26) would drop the key before the end of the last frame had
        // even left the queue. ADR-0010 §7.
        struct Card {
            /// The playback clock: every sample played, silence included.
            played: u64,
            /// Samples queued and not played yet.
            queued: u64,
        }
        impl Card {
            fn play(&mut self, samples: u64) {
                self.queued -= samples.min(self.queued);
                self.played += samples;
            }
        }
        let rate = WIDE_2300.audio_rate as u64;
        let pass = rate / 50; // the run loop's 20 ms blocks
        let render = rate * 3 / 10; // rendering the burst and keying the radio: 0.3 s
        let ms = |samples: u64| samples as f64 * 1000.0 / rate as f64;
        let mut station = idle_station();
        station.connect("KK4XYZ").expect("idle");
        let mut card = Card {
            played: 10 * rate, // it has been running a while
            queued: 0,
        };

        // the pass that keys: the clock goes in, then the fill renders and keys ...
        station.device_played(card.played);
        let mut out = vec![0.0f32; pass as usize];
        let mut count = station.playback(&mut out).expect("playback");
        assert!(
            count > 0 && station.transmitting(),
            "the call was rendered and keyed"
        );
        // ... while the card plays silence, and only then is the first block queued
        card.play(render);
        while count > 0 {
            card.queued += count as u64;
            count = station.playback(&mut out).expect("playback");
        }
        assert!(
            station.transmitting() && !station.handing_over(),
            "the whole burst is with the card, and the key is down"
        );
        // the clock's reading once the last sample handed over has left the card — what the
        // run loop, having queued it, now tells the station
        let leaves = card.played + card.queued;
        station.device_drains_at(leaves);

        // the passes that follow: the card plays on, and the loop reads its clock and
        // captures, then reads it again and asks for more
        let silence = vec![0.0f32; pass as usize];
        let (mut released, mut heard_early) = (None, 0u64);
        while released.is_none() && card.played < leaves + rate {
            card.play(pass);
            station.device_played(card.played);
            // what the station hears of the block it is about to take: only what came in
            // after the last sample had left
            let heard = station.captured_after_transmission(pass as usize, pass as usize);
            heard_early =
                heard_early.max((heard as u64).saturating_sub(card.played.saturating_sub(leaves)));
            station.capture(&silence).expect("capture");
            station.device_played(card.played);
            station.playback(&mut out).expect("playback");
            if !station.transmitting() {
                released = Some(card.played);
            }
        }
        let released = released.expect("the key was never released");
        assert!(
            released >= leaves,
            "the key came up {:.0} ms before the last sample handed over had left the sound card",
            ms(leaves - released)
        );
        assert!(
            released < leaves + pass,
            "the key stayed down {:.0} ms after the burst had left",
            ms(released - leaves)
        );
        assert_eq!(
            heard_early,
            0,
            "the station heard {:.0} ms of its own transmission",
            ms(heard_early)
        );
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
            station.device_drains_at(handed as u64);
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
    fn the_keyed_tail_follows_what_the_sound_card_reports_it_holds() {
        // ADR-0038: the whole quarter-second lead was kept after every burst whatever the
        // card, and ND1J's answers began only 0-100 ms after the radio was back on receive.
        // The tail now covers the card's own reported delay and a margin, within bounds; a
        // card that reports nothing keeps the whole lead.
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
        let base = station.config.key_tail_s - lead;
        assert!(
            (station.tail_s() - (base + lead)).abs() < 1e-12,
            "no report: the whole lead"
        );
        station.device_latency(Some(0.04));
        let expected = base + 0.04 + DEVICE_LATENCY_MARGIN_S;
        assert!(
            (station.tail_s() - expected).abs() < 1e-12,
            "{}",
            station.tail_s()
        );
        // never under the least, never over the lead, and nonsense is ignored
        station.device_latency(Some(0.0));
        assert!((station.tail_s() - (base + DEVICE_LATENCY_MIN_S)).abs() < 1e-12);
        station.device_latency(Some(0.9));
        assert!((station.tail_s() - (base + lead)).abs() < 1e-12);
        station.device_latency(Some(f64::NAN));
        assert!((station.tail_s() - (base + lead)).abs() < 1e-12);
        // and the burst's silence after its last sample is that tail
        station.device_latency(Some(0.04));
        station.tune(1.0).expect("idle");
        let mut rendered = Vec::new();
        let mut out = vec![0.0f32; 960];
        for _ in 0..200 {
            let count = station.playback(&mut out).expect("playback");
            rendered.extend_from_slice(&out[..count]);
        }
        let rate = station.config.params.audio_rate as f64;
        let last = rendered
            .iter()
            .rposition(|x| x.abs() > 1e-4)
            .expect("a tone");
        let silence_s = (rendered.len() - last - 1) as f64 / rate;
        assert!(
            silence_s >= expected - 0.03,
            "{silence_s:.3} s of tail, {expected:.3} expected"
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
    fn a_called_stations_recording_starts_with_the_call_that_brought_it() {
        // WC4Y's probe of 2026-10-10 went unanswered, and KK4ODA-1's recording began when the
        // session came up: nothing said whether the probe had been heard. A recording now starts
        // with the last PREROLL_S heard, and the frames found in it; the call is in the log too
        let dir = std::env::temp_dir().join(format!("aether-preroll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut air = Air::new(1.0, 0.0005);
        air.b.set_recording(Some(dir.clone()), true, "");
        air.run(5.0, |_, _| false);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |_, b| b.recording().is_some());
        let events = air.b.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("heard:call from W4ODA to KK4XYZ at")),
            "{events:?}"
        );
        air.a.disconnect();
        air.run(60.0, |a, b| !a.connected() && !b.connected());
        let sidecar: serde_json::Value = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
            .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap())
            .expect("a sidecar");
        let preroll = sidecar["session"]["preroll_s"].as_f64().expect("preroll_s");
        assert!(preroll > 5.0 && preroll <= PREROLL_S + 0.1, "{preroll}");
        let first = &sidecar["frames"][0];
        assert!(
            first["start_s"].as_f64().expect("start_s") < preroll && first["decoded"] == true,
            "the call is the recording's first frame, before the session: {first}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debug_mode_records_the_host_programs_sessions_and_only_those() {
        // ADR-0050: with a host program attached the session is recorded and marked as the
        // program's, which is what the daemon sends; without one, nothing is recorded
        let dir = std::env::temp_dir().join(format!("aether-debug-rec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut air = Air::new(1.0, 0.0005);
        air.b.set_recording(Some(dir.clone()), false, "");
        air.b.config.record_host_sessions = true;
        let program = HostPresence {
            attached: true,
            listening: true,
            chat: false,
        };
        let message = b"Winlink by way of Aether.";
        for attached in [true, false] {
            air.b.set_host(if attached {
                program
            } else {
                HostPresence::default()
            });
            air.a.connect("KK4XYZ").expect("idle");
            air.a.send(message);
            air.run(90.0, |a, _| a.engine().stats.bytes_acked >= message.len());
            assert_eq!(
                air.b.recording().is_some(),
                attached,
                "recording with a host program attached: {attached}"
            );
            air.a.disconnect();
            air.run(60.0, |a, b| {
                a.state() == State::Idle && b.state() == State::Idle
            });
            // a recording runs on until the closing identifiers have gone (ADR-0076)
            air.run(60.0, |_, b| b.recording().is_none());
            assert!(air.b.recording().is_none());
            let finished = air.b.take_finished_sessions();
            assert_eq!(finished.len(), 1);
            assert_eq!(finished[0].host, attached);
            assert_eq!(finished[0].recording.is_some(), attached);
            air.a.reset_counters();
        }
        let _ = std::fs::remove_dir_all(&dir);
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
            air.b.recording().is_some(),
            "the recording runs on past the session for the closing identifiers (ADR-0076)"
        );
        air.run(60.0, |_, b| b.recording().is_none());
        assert!(
            air.b.recording().is_none(),
            "the recording did not stop after the session's identifiers"
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
        // the engine keeps the closing identifier's room itself, behind a DISC_ACK in the time
        // the DISC granted (ADR-0075): the limit is the key's, identifying or not, and the
        // identifier's length is what the station declares — a US callsign one whatever the
        // setting says
        let mut identifying = station(30.0, Some(CwId::default()));
        assert_eq!(identifying.engine.burst_capacity(tone), 5);
        identifying.refresh_burst_cap(2.0);
        assert_eq!(identifying.engine.burst_capacity(tone), 5);
        let declared = identifying.engine.config().identifier_s;
        assert!(
            (declared - identifying.identifier_length_for("KK4ODA-1")).abs() < 1e-9
                && declared > 1.0,
            "declared {declared:.2} s"
        );
        assert!(
            (plain.engine.config().identifier_s - declared).abs() < 1e-9,
            "a US callsign declares its identifier with the setting off too"
        );
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
            let handed = station.playback(&mut out).expect("playback");
            station.device_drains_at(played + handed as u64);
            played += 4096;
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

    /// Run `air` with `b` stalled for `stall_s` once `a` has keyed a transmission of the
    /// session's, as the scenario harness's slow machine does: `b` neither hears nor answers,
    /// what `a` sends piles up, the newest two seconds of it are kept and the rest lost — told
    /// to `b` when `skip` says so — and `b` takes the pile before the air moves on. How far
    /// `b`'s clock stood behind the air's after the pile, whether the two were ever keyed at
    /// once afterwards, and how many answers `b` dropped late.
    fn stalled_session(skip: bool, stall_s: f64) -> (f64, bool, usize) {
        let message: Vec<u8> = (0..4000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        air.a.send(&message);
        // a transmission of the caller's well under way: its header read, its frames going
        air.run(60.0, |a, b| {
            a.transmitting() && !b.transmitting() && b.received_len() > 0
        });
        let mut on = 0usize;
        air.run(20.0, |a, _| {
            on = if a.transmitting() { on + 1 } else { 0 };
            on >= 12
        });
        let rate = WIDE_2300.audio_rate as f64;
        let blocks = (stall_s * rate / air.block as f64) as usize;
        let kept = (2.0 * rate / air.block as f64) as usize;
        let mut pile = Vec::new();
        let mut from_a = vec![0.0f32; air.block];
        for _ in 0..blocks {
            // the stalled station's card plays on with the caller's
            let start = air.played[0];
            air.a.device_played(start);
            air.played[0] += air.block as u64;
            air.played[1] += air.block as u64;
            let handed = air.a.playback(&mut from_a).expect("playback");
            air.a.device_drains_at(start + handed as u64);
            let mut to_b = vec![0.0f32; air.block];
            let mut to_a = vec![0.0f32; air.block];
            for index in 0..air.block {
                to_b[index] = from_a[index].mul_add(air.gain, air.noise() * air.noise_sigma);
                to_a[index] = air.noise() * air.noise_sigma;
            }
            air.a.capture(&to_a).expect("capture");
            pile.push(to_b);
        }
        let lost = pile.len().saturating_sub(kept);
        if skip {
            air.b.skip_captured(lost * air.block);
        }
        for block in &pile[lost..] {
            air.b.capture(block).expect("capture");
        }
        let behind = air.a.now() - air.b.now();
        let mut keyed_over = false;
        air.run(120.0, |a, b| {
            keyed_over |= a.transmitting() && b.transmitting();
            b.received_len() >= message.len()
        });
        assert_eq!(air.b.received_len(), message.len(), "the message crossed");
        (behind, keyed_over, air.b.stats.dropped_late)
    }

    #[test]
    fn a_station_catching_up_after_a_stall_judges_its_slot_on_the_airs_clock() {
        // the harness (`80m-slow-machine-500`, Step 3 §3.1): after a stall the capture kept its
        // newest seconds, the station processed them a block a pass, and between two of them
        // it keyed the answer to frames it had read before the stall — on its own clock, 1.84 s
        // behind the air — over the caller's next transmission. The gate judges a slot on the
        // air's clock: the station's, plus what it has captured and not yet processed
        let message: Vec<u8> = (0..4000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        air.a.send(&message);
        // the caller's transmission read, and over: the called station's slot is about to open
        air.run(60.0, |a, b| a.transmitting() && b.received_len() > 0);
        air.run(30.0, |a, _| !a.transmitting());
        assert!(!air.a.transmitting() && !air.b.transmitting());
        // the called station's computer stops for 1.5 s; the caller's card plays on
        let rate = WIDE_2300.audio_rate as f64;
        let blocks = (1.5 * rate / air.block as f64) as usize;
        let mut pile = Vec::new();
        let mut from_a = vec![0.0f32; air.block];
        for _ in 0..blocks {
            // the stalled station's card plays on with the caller's
            let start = air.played[0];
            air.a.device_played(start);
            air.played[0] += air.block as u64;
            air.played[1] += air.block as u64;
            let handed = air.a.playback(&mut from_a).expect("playback");
            air.a.device_drains_at(start + handed as u64);
            let mut to_a = vec![0.0f32; air.block];
            let mut to_b = vec![0.0f32; air.block];
            for index in 0..air.block {
                to_b[index] = from_a[index].mul_add(air.gain, air.noise() * air.noise_sigma);
                to_a[index] = air.noise() * air.noise_sigma;
            }
            air.a.capture(&to_a).expect("capture");
            pile.push(to_b);
        }
        // it catches up a block a pass, the air held where the stall ended, as the channel
        // server holds it; nothing it keys in the meantime may radiate past its slot
        let dropped = air.b.stats.dropped_late;
        let mut keyed = false;
        let mut out = vec![0.0f32; air.block];
        for block in &pile {
            air.b.device_played(air.played[1]);
            air.b.capture(block).expect("capture");
            air.b.device_played(air.played[1]);
            let handed = air.b.playback(&mut out).expect("playback");
            air.b.device_drains_at(air.played[1] + handed as u64);
            keyed |= air.b.transmitting();
        }
        assert!(
            !keyed,
            "the called station keyed an answer whose slot had passed on the air"
        );
        assert!(
            air.b.stats.dropped_late > dropped,
            "the late answer was not dropped"
        );
        // and the session goes on
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.received_len(), message.len(), "the message crossed");
    }

    #[test]
    fn a_station_that_fell_behind_moves_its_clock_past_the_audio_it_lost() {
        // the scenario harness's slow machine (Step 2): after a stall the capture queue kept
        // the newest two seconds and dropped the rest, the station's clock — the audio it had
        // heard — fell that far behind the air, and its next answer went out that late, over
        // the caller's next transmission; the keying gate read the same clock and let it go.
        // Here the stall begins after the header was read: its slot passes during the stall,
        // and the answer, decided on the clock moved on, is the gate's to drop (1.5 s late
        // without the move, and radiated)
        let (behind, keyed_over, dropped) = stalled_session(true, 3.5);
        assert!(
            behind.abs() < 1e-9,
            "the station's clock {behind:.3} s behind the air's"
        );
        assert!(
            dropped >= 1,
            "the answer whose slot passed in the stall was not dropped"
        );
        assert!(
            !keyed_over,
            "an answer went out over the caller after the stall"
        );
        // without the move the station's clock stays behind the air; the keying gate, reading
        // the card's clock against it, still drops the late answer (10-step-3.md §3.1)
        let (behind, keyed_over, dropped) = stalled_session(false, 3.5);
        assert!(behind > 1.4, "the harness's stall: {behind:.2} s");
        assert!(
            dropped >= 1 && !keyed_over,
            "behind the air and not moved, the answer still went: {dropped} dropped"
        );
    }

    #[test]
    fn an_abort_that_cuts_a_header_waits_out_the_slot_the_header_stated() {
        // the scenario harness (`40m-abort-and-recall-2300`, Step 2): the caller aborted
        // halfway through a header and six data frames, the transmission was reported over at
        // the cut, the slot it granted was measured from there — and the leaving DISC keyed at
        // the start of the called station's answer, which the called station placed after the
        // six frames the header had stated. Rung 10 at most, as there: its frames are long.
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            max_mode_setting: Some(10),
            link: GrantConfig {
                max_mode: 10,
                ..config.link.clone()
            },
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        let message: Vec<u8> = (0..6000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        air.a.send(&message);
        // a transmission of the message's that runs over two seconds: its header out, a second
        // in, and its data frames still going
        let rate = WIDE_2300.audio_rate as f64;
        let long = |a: &Station<NullPtt>| {
            a.transmitting()
                && a.keying
                    .is_some_and(|w| w.frames_samples as f64 / rate > 2.0)
        };
        air.run(60.0, |a, _| long(a));
        assert!(long(&air.a), "a long transmission of the message's");
        air.run(1.0, |_, _| false);
        assert!(air.a.transmitting(), "the message is on the air");
        air.a.abort();
        let mut keyed_over = false;
        air.run(30.0, |a, b| {
            keyed_over |= a.transmitting() && b.transmitting();
            false
        });
        assert!(
            air.b.engine().stats.transmissions > 0,
            "the called station answered nothing"
        );
        assert!(
            !keyed_over,
            "the leaving DISC went over the called station's answer"
        );
    }

    #[test]
    fn an_abort_that_cuts_a_header_before_it_can_be_read_grants_nothing() {
        // the harness (`80m-floor-top-start-500`, Step 3 §3.6): a Test's abort cut a floor
        // header 1 s into its 3.2 s, the master waited out the slot that header would have
        // granted — 29 s of floor frames — and its DISC went as the run ended, unheard. A header
        // cut before it could be read granted nothing; its slot is measured from the cut
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        let message: Vec<u8> = (0..6000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8)
            .collect();
        air.a.send(&message);
        // a transmission with a header first and frames after it, long enough that its stated
        // end is seconds away
        let rate = WIDE_2300.audio_rate as f64;
        let headed = |a: &Station<NullPtt>| {
            a.transmitting()
                && a.keying.is_some_and(|w| {
                    w.header_samples.is_some() && w.frames_samples as f64 / rate > 3.0
                })
        };
        air.run(60.0, |a, _| headed(a));
        assert!(headed(&air.a), "a long transmission with a header first");
        // into the header, a block at a time, until a tenth of it has radiated: past its start,
        // well short of the point it could be read
        let radiated =
            |a: &Station<NullPtt>| a.now() + a.radiation_latency_s() - a.config.key_lead_s;
        let (start, end) = air
            .a
            .keying
            .and_then(|w| w.header_on_air)
            .expect("keyed with a header first");
        while radiated(&air.a) < start + 0.1 * (end - start) {
            air.run(0.09, |_, _| false);
        }
        assert!(
            radiated(&air.a) < start + 0.35 * (end - start),
            "a block past a third of the header"
        );
        let cut_at = air.a.now();
        air.a.abort();
        let mut disc_at = None;
        let mut keyed_over = false;
        air.run(30.0, |a, b| {
            keyed_over |= a.transmitting() && b.transmitting();
            if disc_at.is_none() && a.transmitting() {
                disc_at = Some(a.now());
            }
            disc_at.is_some() && !a.transmitting()
        });
        let after = disc_at.expect("the DISC never went") - cut_at;
        assert!(
            after < 3.0,
            "the DISC waited {after:.1} s, for the slot of a header nobody could read"
        );
        assert!(!keyed_over, "the DISC went over the other station");
    }

    #[test]
    fn an_abort_waits_out_the_slot_it_granted() {
        // the scenario harness (ADR-0065): a Test whose budget ran out keyed its DISC 1.4 s
        // into the other station's acknowledgement, which it had heard arriving; both were lost.
        // Under granted slots the master's DISC waits for the slot it granted to end (ADR-0069).
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.b.send(&[0x5A; 400]);
        // the other station's burst arriving, its first frame announced
        air.run(120.0, |a, b| b.transmitting() && a.receiving());
        assert!(air.b.transmitting() && air.a.receiving());
        air.a.abort();
        let mut keyed_over = false;
        air.run(30.0, |a, b| {
            keyed_over |= a.transmitting() && b.transmitting();
            a.transmitting()
        });
        assert!(air.a.transmitting(), "the DISC never went");
        assert!(
            !keyed_over,
            "the DISC went over the frame it heard arriving"
        );
    }

    #[test]
    fn an_aborts_disc_does_not_wait_for_a_clear_channel() {
        // the harness (`40m-rtty-contest-2300`, Step 3 §3.3): a Test's budget ran out, the
        // master aborted, and its leaving DISC waited 29 s for the contest to clear — the engine
        // was Idle by then, and the hold took the DISC for a station starting something of its
        // own. The other station never heard it and was still connected at the end. A DISC ends
        // an exchange already under way: only calls and probes wait for the channel
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        // the operator's politeness, which the harness's wire turns off
        air.a.config.wait_for_clear = true;
        air.a.send(&[0x5A; 400]);
        air.run(5.0, |_, _| false);
        air.a.abort();
        let mut sent_disc = false;
        // the channel busy throughout, as the contest kept it
        for _ in 0..200 {
            let now = air.a.now();
            air.a.busy.mark_frame(now, 3.0);
            air.run(0.1, |_, _| false);
            sent_disc |= air.a.transmitting() && air.a.state() == State::Idle;
            if !air.b.connected() {
                break;
            }
        }
        assert!(sent_disc, "the leaving DISC was held for the busy channel");
        assert!(!air.b.connected(), "the other station never heard it leave");
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
        assert!(pending.is_empty(), "{pending:?}");
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
        unset.beacon(None).expect("queued");
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
        station.beacon(None).expect("queued");
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
    fn a_closing_identifier_is_judged_as_the_cw_it_is() {
        // the closing identifier rides behind the slave's DISC_ACK and alone after the master's
        // last slot (ADR-0075): it is judged as CW before it goes, so a profile that allows the
        // data but not CW there keeps it off the air, says so, and the session still closes
        use crate::regulatory::{ControlMode, Policy};
        let rules = || under_us_rules(14_078_000, ControlMode::Local);
        let mut lawful = Air::with(1.0, 0.0005, |config| StationConfig {
            regulatory: rules(),
            ..config
        });
        lawful.a.connect("KK4XYZ").expect("idle");
        lawful.run(60.0, |a, b| a.connected() && b.connected());
        lawful.a.disconnect();
        lawful.run(120.0, |a, b| a.quiescent() && b.quiescent());
        assert_eq!(
            (lawful.a.stats.cw_ids, lawful.b.stats.cw_ids),
            (1, 1),
            "both closing identifiers go out where CW is allowed"
        );

        let mut profile =
            crate::regulatory::profile::load("us-fcc-part97").expect("the profile reads");
        for privilege in &mut profile.privileges.general {
            if privilege.range.holds(14_078_000.0, 14_078_000.0) {
                privilege.emissions = Some(vec!["data".to_owned()]);
            }
        }
        let profile = std::sync::Arc::new(profile);
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            regulatory: rules(),
            ..config
        });
        air.a.set_policy(Policy::Rules(profile.clone()));
        air.b.set_policy(Policy::Rules(profile));
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected(), "the session itself is lawful");
        air.a.disconnect();
        air.run(120.0, |a, b| a.quiescent() && b.quiescent());
        assert_eq!(air.a.state(), State::Idle);
        assert_eq!((air.a.stats.cw_ids, air.b.stats.cw_ids), (0, 0));
        for station in [&mut air.a, &mut air.b] {
            assert!(
                station
                    .take_regulatory_reports()
                    .iter()
                    .any(|d| d.what == "the Morse identifier" && !d.allowed()),
                "the refusal is reported"
            );
            assert!(
                station
                    .take_events()
                    .iter()
                    .any(|e| e.starts_with("identified:the closing identifier is not sent")),
                "and a host program is told the session is over all the same"
            );
            assert!(!station.closing_identifier_pending());
        }
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
        assert!(!permitted.is_empty(), "permitted is empty");
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
        // the called station only received and was never acknowledged: the caller's
        // disconnect said how it heard it (ADR-0021)
        assert!(b.heard_there_db.is_some(), "{b:?}");
        assert!(!a.test && a.recording.is_none());
        assert!(air.a.take_finished_sessions().is_empty(), "taken once");
    }

    /// What a station keyed, by its accounts of the air (`take_tags`): each transmission's
    /// first frame's kind — `id` for an identifier alone — and whether it carried the Morse
    /// identifier.
    fn keyed(station: &mut Station<NullPtt>) -> Vec<(String, bool)> {
        station
            .take_tags()
            .iter()
            .map(|tag| {
                let tag: serde_json::Value = serde_json::from_str(tag).expect("a tag");
                let kind = tag["frames"]
                    .as_array()
                    .and_then(|frames| frames.first())
                    .map_or_else(
                        || "id".to_owned(),
                        |frame| frame["kind"].as_str().unwrap_or("?").to_owned(),
                    );
                (kind, tag["cw_id"].as_bool() == Some(true))
            })
            .collect()
    }

    /// Each station's last transmission ends with its closing identifier, and none before it
    /// carried one (ADR-0075).
    fn ends_with_its_identifier(station: &mut Station<NullPtt>, last: &str) {
        let keyed = keyed(station);
        let call = station.engine.my_call.clone();
        assert_eq!(
            keyed.last(),
            Some(&(last.to_owned(), true)),
            "{call} did not end with its identifier: {keyed:?}"
        );
        assert!(
            keyed[..keyed.len() - 1].iter().all(|(_, id)| !id),
            "{call} identified before the end: {keyed:?}"
        );
    }

    #[test]
    fn a_session_ends_with_both_identifiers_last_however_it_ends() {
        // ADR-0075, the licensee's decision: at the end of a session the Morse identifier is
        // the very last thing each station sends, however the session ends; both callsigns
        // here are US ones, so both identify with the setting off (VARA HF's rule)
        let both_idle = |a: &Station<NullPtt>, b: &Station<NullPtt>| a.quiescent() && b.quiescent();

        // an orderly close: the DISC, the slave's DISC_ACK with its identifier behind it, the
        // master's identifier on its own once that slot is over — nothing on the call
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 300]);
        air.a.disconnect();
        air.run(150.0, both_idle);
        assert_eq!((air.a.stats.cw_ids, air.b.stats.cw_ids), (1, 1));
        ends_with_its_identifier(&mut air.a, "id");
        ends_with_its_identifier(&mut air.b, "DiscAck");

        // the slave asks to leave: the same order
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.b.disconnect();
        air.run(150.0, both_idle);
        ends_with_its_identifier(&mut air.a, "id");
        ends_with_its_identifier(&mut air.b, "DiscAck");

        // the peer goes silent mid-session: the master ends after its last slot and identifies
        // on its own
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 3000]);
        air.run(3.0, |_, _| false);
        let before = air.a.stats.cw_ids;
        air.run_a_alone(400.0, |a| a.state() == State::Idle && a.quiescent());
        assert_eq!(air.a.state(), State::Idle, "the session ended");
        assert_eq!(air.a.stats.cw_ids, before + 1, "the end was identified");
        ends_with_its_identifier(&mut air.a, "id");

        // an abort mid-burst: the burst is cut, the DISC goes on the floor and is answered with
        // the slave's DISC_ACK and identifier, and the master's identifier comes last
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 3000]);
        air.run(20.0, |a, _| a.transmitting());
        air.a.abort();
        air.run(90.0, both_idle);
        assert_eq!(air.b.state(), State::Idle);
        assert!(
            air.b
                .take_events()
                .iter()
                .any(|e| e == "disconnected:peer disconnected"),
            "the peer heard the DISC"
        );
        ends_with_its_identifier(&mut air.a, "id");
        ends_with_its_identifier(&mut air.b, "DiscAck");
        assert_eq!(air.a.stats.watchdog_trips, 0);

        // the slave aborts: it answers the next header with a DISC, the master closes with a
        // DISC of its own so that the slave's identifier has its slot, and identifies last
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(&[0x42; 3000]);
        air.run(5.0, |_, _| false);
        air.b.abort();
        air.run(150.0, both_idle);
        assert!(
            air.a
                .take_events()
                .iter()
                .any(|e| e == "disconnected:peer disconnected"),
            "the master closed the session the slave left"
        );
        ends_with_its_identifier(&mut air.a, "id");
        ends_with_its_identifier(&mut air.b, "DiscAck");
    }

    #[test]
    fn a_station_whose_callsign_needs_no_identifier_ends_as_before() {
        // a non-US callsign with the setting off sends none: its DISC_ACK is its last word,
        // and the US station's identifier still comes last of all
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            callsign: if config.callsign == "KK4XYZ" {
                "VE3XYZ".to_owned()
            } else {
                config.callsign
            },
            ..config
        });
        air.a.connect("VE3XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.disconnect();
        air.run(150.0, |a, b| a.quiescent() && b.quiescent());
        assert_eq!((air.a.stats.cw_ids, air.b.stats.cw_ids), (1, 0));
        ends_with_its_identifier(&mut air.a, "id");
        let keyed = keyed(&mut air.b);
        assert_eq!(
            keyed.last(),
            Some(&("DiscAck".to_owned(), false)),
            "{keyed:?}"
        );
    }

    #[test]
    fn a_station_that_calls_again_is_not_identified_over() {
        // KQ4SAX lost his session and called again (2026-10-12): the call ended the session
        // (ADR-0056) and KK4ODA-1 put the ended session's Morse identifier on the air 10 ms after
        // the new call's acceptance — 6.6 s of CW where a caller that read the acceptance makes
        // its first move. The same station calling again continues the communication.
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            cw_id: (config.callsign == "KK4XYZ").then(CwId::default),
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.b.connected(), "the first session came up");
        // the caller gives the session up where the other station cannot hear it go
        air.a.abort();
        air.run_a_alone(60.0, |a| a.state() == State::Idle && a.quiescent());
        assert_eq!(air.a.state(), State::Idle);
        assert!(air.b.connected(), "the called station never heard it go");
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(
            air.b
                .take_events()
                .iter()
                .any(|e| e.ends_with("called again")),
            "the new call ended the old session"
        );
        assert!(
            air.a.connected() && air.b.connected(),
            "the second session came up"
        );
        air.run(15.0, |_, _| false);
        assert_eq!(air.b.stats.cw_ids, 0, "identified behind the acceptance");
        air.a.send(&[0x42; 300]);
        air.a.disconnect();
        air.run(120.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle && b.quiescent()
        });
        air.run(10.0, |_, _| false);
        assert_eq!(air.b.stats.cw_ids, 1, "the communication's end, once");
    }

    /// Two identifying stations, the called one waiting for a clear channel, and `a` closes
    /// the session. How many times `a` keyed from its DISC on, in how many blocks both were
    /// keyed at once, and how many identifiers each sent meanwhile.
    fn close_between_identifying_stations() -> (Vec<String>, usize, usize, usize, usize) {
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            cw_id: Some(CwId::default()),
            wait_for_clear: config.callsign == "KK4XYZ",
            ..config
        });
        air.run(4.0, |_, _| false);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.send(b"73");
        air.run(60.0, |_, b| b.received_len() >= 2);
        air.run(3.0, |a, _| !a.transmitting() && a.pending.is_empty());
        air.a.take_events();
        let ids = (air.a.stats.cw_ids, air.b.stats.cw_ids);
        air.a.disconnect();
        let (mut overlap, mut a_keyings, mut was_keyed) = (0usize, 0usize, false);
        air.run(60.0, |a, b| {
            if a.transmitting() && b.transmitting() {
                overlap += 1;
            }
            if a.transmitting() && !was_keyed {
                a_keyings += 1;
            }
            was_keyed = a.transmitting();
            a.quiescent() && b.quiescent()
        });
        (
            air.a.take_events(),
            a_keyings,
            overlap,
            air.a.stats.cw_ids - ids.0,
            air.b.stats.cw_ids - ids.1,
        )
    }

    #[test]
    fn a_disconnect_with_data_queued_says_how_long_it_may_take_and_that_abort_is_at_once() {
        // The soak of 2026-10-15: on slow paths a disconnect waited 30–40 minutes for its
        // queue. It now waits a minute at most (ADR-0072), and says so when it is asked.
        let mut air = Air::new(1.0, 0.0005);
        air.run(4.0, |_, _| false);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected(), "no session");
        assert_eq!(
            air.a.leave_in_s(),
            None,
            "a limit before a disconnect was asked"
        );
        // bytes deflate cannot shrink: a repeated byte would cross in a frame
        let mut x = 0x9e37_79b9_u32;
        let noise: Vec<u8> = (0..20_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x.to_le_bytes()[0]
            })
            .collect();
        air.a.send(&noise);
        air.a.take_events();
        air.a.disconnect();
        let events = air.a.take_events();
        let said: Vec<_> = events
            .iter()
            .filter(|e| e.starts_with("closing:"))
            .collect();
        assert_eq!(said.len(), 1, "{events:?}");
        assert!(
            said[0].contains("bytes still queued go first, for at most 60 s")
                && said[0].contains("Abort closes at once"),
            "{said:?}"
        );
        let left = air.a.leave_in_s().expect("the limit");
        assert!((59.0..=60.001).contains(&left), "{left}");
        // asked again: the limit is not moved, and nothing more is said
        air.run(5.0, |_, _| false);
        air.a.take_events();
        air.a.disconnect();
        assert!(
            !air.a
                .take_events()
                .iter()
                .any(|e| e.starts_with("closing:")),
            "said twice"
        );
        let later = air.a.leave_in_s().expect("the limit");
        assert!(later < left - 4.0, "the limit moved: {left} then {later}");
    }

    #[test]
    fn a_disconnect_is_answered_at_once_and_nobody_keys_over_the_answer_or_an_identifier() {
        // ND1J, 2026-09-25 (`20260925-225955_KK4ODA-1_ND1J`): KK4ODA-1 disconnected, and
        // its retries went out over ND1J's DISC_ACK and then over his Morse identifier. His
        // station waits for a clear channel, and by the time its DISC_ACK was queued its
        // session was over — so the DISC it answered, which marks the channel busy for two
        // seconds, held the answer past the caller's retry. Under granted slots the slave's
        // closing identifier goes behind its DISC_ACK in the slot the DISC granted with its
        // time, and the master's after that slot (ADR-0075): the DISC once, the master's
        // identifier once, nothing over anything.
        let (events, keyings, overlap, a_ids, b_ids) = close_between_identifying_stations();
        assert!(
            events.iter().any(|e| e == "disconnected:closed"),
            "the DISC_ACK was not heard: {events:?}"
        );
        assert_eq!(
            keyings, 2,
            "the DISC, then the identifier, and nothing was repeated"
        );
        assert_eq!(overlap, 0, "the two stations were keyed at once");
        assert_eq!((a_ids, b_ids), (1, 1), "each closing identifier once");
    }

    #[test]
    fn an_identifier_no_longer_wanted_is_dropped_not_queued_for_ever() {
        // a probe that led nowhere owes an identifier after SETUP_ID_WAIT_S; the operator of a
        // non-US station turns identification off meanwhile: nothing is keyed, and the station
        // does not queue the identifier again on every pass
        let mut station = Station::new(
            StationConfig {
                callsign: "VE3XYZ".to_owned(),
                wait_for_clear: false,
                cw_id: Some(CwId::default()),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        station.probe("KK4XYZ", None).expect("idle");
        run_alone(&mut station, 10.0, Station::transmitting);
        run_alone(&mut station, 10.0, |s| !s.transmitting());
        assert!(
            station.identifier.setup_at.is_some(),
            "the probe set up a contact"
        );
        station.config.cw_id = None;
        run_alone(&mut station, SETUP_ID_WAIT_S + 10.0, |_| false);
        assert!(station.quiescent());
        assert!(station.identifier.setup_at.is_none());
        assert_eq!(station.stats.cw_ids, 0);
    }

    #[test]
    fn a_us_callsign_identifies_whatever_its_setting_and_any_other_only_when_asked() {
        // ADR-0075, as VARA HF does: a station operating under a US callsign always sends its
        // Morse identifier; for any other callsign it is the operator's choice, off by default.
        // The setting itself is never rewritten: the forcing is computed
        let station = |callsign: &str, cw_id: Option<CwId>| {
            Station::new(
                StationConfig {
                    callsign: callsign.to_owned(),
                    wait_for_clear: false,
                    cw_id,
                    ..StationConfig::default()
                },
                NullPtt::default(),
                1,
            )
        };
        let mut us = station("W4ODA", None);
        assert!(us.config.cw_id.is_none(), "the setting is left as it is");
        let status = us.identifier_status();
        assert_eq!(status["enabled"], true);
        assert_eq!(status["setting"], false);
        assert_eq!(status["forced"], true);
        assert_eq!(status["reason"], "Required for US callsigns (§97.119)");
        assert_eq!(status["callsign"], "W4ODA");
        assert!(
            us.engine.config().identifier_s > 1.0,
            "the call declares it"
        );
        let events = us.take_events();
        assert!(
            events.iter().any(
                |e| e.starts_with("identifier:the Morse ID goes out as W4ODA")
                    && e.ends_with("Required for US callsigns (§97.119)")
            ),
            "said at start: {events:?}"
        );

        let elsewhere = station("VE3XYZ", None);
        let status = elsewhere.identifier_status();
        assert_eq!(status["enabled"], false);
        assert_eq!(status["forced"], false);
        assert!(status["reason"].is_null());
        assert!(elsewhere.engine.config().identifier_s.abs() < 1e-12);

        let asked = station("VE3XYZ", Some(CwId::default()));
        let status = asked.identifier_status();
        assert_eq!(
            (status["enabled"].as_bool(), status["forced"].as_bool()),
            (Some(true), Some(false))
        );

        // a US station's probe that leads nowhere is identified on its own, setting off
        us.probe("KK4XYZ", None).expect("idle");
        run_alone(&mut us, SETUP_ID_WAIT_S + 30.0, |s| s.stats.cw_ids > 0);
        assert_eq!(us.stats.cw_ids, 1, "the identifier never went out");
    }

    #[test]
    fn the_log_says_once_that_a_us_callsign_identifies_though_the_setting_is_off() {
        let mut station = idle_station();
        let said = |events: Vec<String>| {
            events
                .iter()
                .filter(|e| e.starts_with("identifier:the Morse ID goes out as"))
                .count()
        };
        assert_eq!(said(station.take_events()), 1, "said at start");
        station.note_required_identifier();
        assert_eq!(said(station.take_events()), 0, "said once");
        station
            .set_callsigns(&["KK4ODA".to_owned()])
            .expect("callsigns");
        assert_eq!(
            said(station.take_events()),
            1,
            "said again for the new callsign"
        );
        station
            .set_callsigns(&["VE3XYZ".to_owned()])
            .expect("callsigns");
        assert_eq!(
            said(station.take_events()),
            0,
            "nothing to say for a non-US callsign"
        );
        assert_eq!(station.identifier_status()["enabled"], false);
    }

    #[test]
    fn the_closing_identifier_goes_behind_the_frames() {
        // ADR-0075: the slave's closing identifier follows its DISC_ACK, in the time the DISC
        // granted; the daemon renders the frames, a moment of silence, then the identifier, and
        // the frames' end it reports to the engine is before it
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let ack = GrantFrame::new(aether_link::GrantKind::DiscAck, 7);
        let frame = aether_link::TxFrame {
            container: Container::Control,
            payload: ack.encode().to_vec(),
            mode: 0,
            rv: 0,
            floor: false,
            follows: 0,
        };
        let identifier_s = station.identifier_length_for("W4ODA");
        let rate = station.config.params.audio_rate as f64;
        let (alone, _) = station
            .modulate(std::slice::from_ref(&frame))
            .expect("a frame");
        let frames = {
            let mut chain = station.to_audio.clone();
            let mut out = chain.process(&alone);
            out.extend(chain.flush());
            out.len()
        };
        station.pending.push_back(Outgoing::Link(LinkTx {
            frames: vec![frame],
            origin: Origin::station("test"),
            window: KeyWindow::default(),
            identifier_s,
            listen_first: false,
        }));
        station.render_next(1.0);
        assert_eq!(station.stats.cw_ids, 1);
        let id = CwId {
            level: CW_ID_RELATIVE_LEVEL,
            ..CwId::default()
        }
        .audio("W4ODA", rate);
        let queued: Vec<f32> = station.playback.iter().copied().collect();
        let lead = (station.config.key_lead_s * rate) as usize;
        let gap = (CW_ID_GAP_S * rate) as usize;
        let window = station.keying.expect("an engine transmission");
        assert_eq!(window.frames_samples, lead + frames, "the frames first");
        assert!(window.identifier);
        assert!(
            queued[lead + frames..lead + frames + gap]
                .iter()
                .all(|&x| x == 0.0),
            "a moment of silence"
        );
        assert_eq!(
            &queued[lead + frames + gap..lead + frames + gap + id.len()],
            id.as_slice(),
            "then the identifier"
        );
        assert_eq!(window.trailing_samples, gap + id.len());
    }

    #[test]
    fn the_identifier_sends_the_callsign_without_its_ssid() {
        // ADR-0076: the assigned call sign is what §97.119 asks for; "KK4ODA-1" took 6.3 s at
        // 20 wpm, 2.3 s of it the SSID (13-wc4y-beta111.md §12). What a call declares is the
        // length of what goes out
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA-1".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let with_ssid = station.identifier_length_for("W4ODA-1");
        assert!(with_ssid > 0.0, "a US callsign identifies");
        assert!((with_ssid - station.identifier_length_for("W4ODA")).abs() < 1e-9);
        assert_eq!(station.identifier_check("W4ODA-1")["sent_as"], "W4ODA");
        assert_eq!(station.identifier_check("W4ODA-1")["callsign"], "W4ODA-1");
        assert!(
            station
                .take_events()
                .iter()
                .any(|e| e.starts_with("identifier:the Morse ID goes out as W4ODA at")),
            "the log names what is sent"
        );
    }

    #[test]
    fn a_frame_a_capture_skip_fell_inside_is_not_trusted() {
        // ADR-0076: two of WC4Y's rung-3 frames were lost to the mini PC's capture skipping
        // audio inside them; a skip is noted after the audio it was in, by up to a slow pass
        let mut station = idle_station();
        let rate = station.config.params.audio_rate;
        station.capture(&vec![0.0f32; rate * 3]).expect("capture");
        let now = station.now();
        assert!(!station.capture_skipped_within(now - 2.0, now));
        station.note_capture_skip(0.048);
        assert!(station.capture_skipped_within(now - 2.0, now), "inside");
        assert!(
            station.capture_skipped_within(now - 2.0, now - 0.3),
            "noted a slow pass after the frame's end"
        );
        assert!(
            !station.capture_skipped_within(now - 2.0, now - 0.8),
            "before it"
        );
        assert!(
            !station.capture_skipped_within(now + 0.1, now + 2.0),
            "after it"
        );
    }

    #[test]
    fn a_call_or_a_probe_is_never_reported_late_for_a_slot() {
        // a call held for a busy channel was reported "1820 ms after its slot opened": a call
        // has no slot (13-wc4y-beta111.md §7)
        let mut station = idle_station();
        for (slotless, reported) in [(true, 0), (false, 1)] {
            station.late_keyings = 0;
            station.keying = Some(KeyWindow {
                not_before: Some(1.0),
                slotless,
                ..KeyWindow::default()
            });
            station.note_slow_keying(3.0, 3.0, 0.02);
            assert_eq!(station.late_keyings, reported, "slotless {slotless}");
        }
        station.keying = None;
    }

    #[test]
    fn a_recording_runs_on_until_both_closing_identifiers_have_gone() {
        // ADR-0076: a recording that stopped at the session's end held neither station's
        // closing identifier (13-wc4y-beta111.md §12). The slave's runs on past its own
        // identifier until the master's has gone; the master's stops after its own
        let dir = std::env::temp_dir().join(format!("aether-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            record_dir: Some(dir.clone()),
            record_auto: true,
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.recording.is_some() && air.b.recording.is_some());
        air.a.send(&[0x42; 300]);
        air.a.disconnect();
        air.run(150.0, |_, b| b.state() == State::Idle);
        assert_eq!(air.b.state(), State::Idle);
        assert!(air.b.recording.is_some(), "the slave's recording runs on");
        let mut master_identified = None;
        let mut slave_stopped = None;
        air.run(60.0, |a, b| {
            if master_identified.is_none() && a.stats.cw_ids == 1 && a.quiescent() {
                master_identified = Some(a.now());
            }
            if slave_stopped.is_none() && b.recording.is_none() {
                slave_stopped = Some(b.now());
            }
            a.recording.is_none() && b.recording.is_none()
        });
        let (Some(identified), Some(stopped)) = (master_identified, slave_stopped) else {
            panic!(
                "identified {master_identified:?}, the slave's recording stopped {slave_stopped:?}"
            );
        };
        assert!(
            stopped >= identified,
            "the slave's recording stopped at {stopped:.1} s, before the master's identifier \
             ended at {identified:.1} s"
        );
        assert!(
            air.a.recording.is_none(),
            "the master's recording stopped too"
        );
        let sidecars: Vec<_> = std::fs::read_dir(&dir)
            .expect("the recordings")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        assert_eq!(sidecars.len(), 2);
        for sidecar in sidecars {
            let doc: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("sidecar"))
                    .expect("json");
            let events = doc["events"].as_array().expect("events");
            assert!(
                events.iter().any(|e| e["event"] == "identified"),
                "{}: its own identifier is in it",
                sidecar.display()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_inside_a_session_carries_an_identifier() {
        // ADR-0075: no Morse during a session — the ten-minute identifier is gone, and the
        // session ends with both stations' closing identifiers
        let message = vec![0x3Cu8; 600];
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            cw_id: Some(CwId::default()),
            ..config
        });
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        air.a.send(&message);
        air.b.send(&message);
        air.run(180.0, |a, b| {
            a.received_len() >= message.len() && b.received_len() >= message.len()
        });
        assert_eq!(air.a.received_len(), message.len());
        assert_eq!(air.b.received_len(), message.len());
        assert_eq!((air.a.stats.cw_ids, air.b.stats.cw_ids), (0, 0));
        assert!(
            keyed(&mut air.a)
                .iter()
                .chain(keyed(&mut air.b).iter())
                .all(|(_, id)| !id)
        );
        air.a.disconnect();
        air.run(150.0, |a, b| a.quiescent() && b.quiescent());
        assert_eq!((air.a.stats.cw_ids, air.b.stats.cw_ids), (1, 1));
        for station in [&air.a, &air.b] {
            assert_eq!(station.stats.watchdog_trips, 0);
            assert_eq!(station.stats.dropped_late, 0);
            assert_eq!(station.engine().stats.forfeits, 0);
        }
    }

    #[test]
    fn a_host_hears_the_session_is_over_once_the_identifier_has_gone_out() {
        // ADR-0075: told DISCONNECTED, VarAC returns to its calling frequency and a scanning
        // Trimode changes channel — over the Morse. The station says `disconnected` with the
        // identifier still to come, and `identified` once it has gone out
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        air.a.take_events();
        air.b.take_events();
        air.a.disconnect();
        let mut ended = [None, None];
        let mut pending_at_end = [false, false];
        let mut identified = [None, None];
        let mut t = 0.0;
        air.run(150.0, |a, b| {
            t += 4096.0 / WIDE_2300.audio_rate as f64;
            for (i, station) in [a, b].into_iter().enumerate() {
                if ended[i].is_none() && station.state() == State::Idle {
                    ended[i] = Some(t);
                    pending_at_end[i] = station.closing_identifier_pending();
                }
                if identified[i].is_none()
                    && ended[i].is_some()
                    && !station.closing_identifier_pending()
                {
                    identified[i] = Some(t);
                }
            }
            a.quiescent() && b.quiescent()
        });
        assert_eq!(
            pending_at_end,
            [true, true],
            "an identifier was still to come at the end"
        );
        for (station, call) in [(&mut air.a, "W4ODA"), (&mut air.b, "KK4XYZ")] {
            let events = station.take_events();
            let end = events.iter().position(|e| e.starts_with("disconnected:"));
            let out = events.iter().position(|e| {
                *e == format!("identified:the closing identifier went out as {call}")
            });
            assert!(
                matches!((end, out), (Some(end), Some(out)) if out > end),
                "{call}: {events:?}"
            );
        }
        assert!(
            identified[0] > identified[1],
            "the master's identifier is the last of all"
        );
    }

    #[test]
    fn a_probe_carries_no_identifier_and_one_follows_when_nothing_else_does() {
        // ADR-0055: a probe sets up a contact and carries none — the first transmission after a
        // start had, an answer to WC4Y's probe (2026-10-10) — and a probe that leads to no
        // call is identified on its own once the station has been idle SETUP_ID_WAIT_S
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
        station.probe("KK4XYZ", None).expect("idle");
        run_alone(&mut station, 10.0, Station::transmitting);
        assert!(station.transmitting(), "the probe went out");
        let probed_at = station.now();
        run_alone(&mut station, 10.0, |s| !s.transmitting());
        assert_eq!(station.stats.cw_ids, 0, "the probe carried the identifier");
        run_alone(&mut station, SETUP_ID_WAIT_S + 20.0, |s| s.stats.cw_ids > 0);
        assert_eq!(station.stats.cw_ids, 1, "no identifier after the probe");
        assert!(
            station.now() - probed_at >= SETUP_ID_WAIT_S,
            "the identifier came {:.1} s after the probe",
            station.now() - probed_at
        );
    }

    #[test]
    fn a_beacon_carries_no_identifier() {
        // ADR-0075: beacons and KISS frames carry no Morse — a beacon names its station in its
        // own frame — whatever the setting and the callsign say
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
        let mut out = vec![0.0f32; 4096];
        for _ in 0..400 {
            let _ = station.beacon(None);
            station.playback(&mut out).expect("playback");
            station.capture(&vec![0.0f32; 4096]).expect("capture");
        }
        assert!(
            station.stats.transmissions > 1,
            "only {} transmission(s)",
            station.stats.transmissions
        );
        assert_eq!(station.stats.cw_ids, 0, "a beacon carried the identifier");
        assert!(!station.closing_identifier_pending());
    }

    #[test]
    fn a_beacon_carries_the_name_a_host_program_gave_it_and_its_bandwidth() {
        // VarAC's CQs and beacons are `CQFRAME KK4ODA-9 500`: the suffix means something to
        // the program at the other end, and the bandwidth tells it how to call (ADR-0024)
        let mut air = Air::new(1.0, 0.0005);
        air.a.beacon(Some("w4oda-9")).expect("its own name");
        air.run(30.0, |_, b| b.stats.beacons_heard > 0);
        let events = air.b.take_events();
        assert!(
            events.iter().any(|e| e.starts_with("beacon:heard W4ODA-9")),
            "the name was not carried: {events:?}"
        );
        let report = air
            .b
            .take_frame_reports()
            .into_iter()
            .find(|frame| frame.kind == "beacon")
            .expect("the beacon was reported");
        assert_eq!(report.from.as_deref(), Some("W4ODA-9"));
        assert_eq!(report.bandwidth_hz, Some(2300));
        // a name whose base is another station's is not this one's to send
        assert_eq!(air.a.beacon(Some("K1ABC-9")), Err(BEACON_NOT_OURS));
    }

    #[test]
    fn a_station_under_automatic_control_sends_no_beacon() {
        use crate::regulatory::ControlMode;
        // §97.203(d): a beacon may be automatically controlled only on 28.20–28.30 MHz and
        // above. The repeating beacon was refused already; a host program's beacon timer
        // on an unattended station is the same thing (ADR-0024)
        let mut station = lone_station(under_us_rules(14_105_000, ControlMode::Automatic));
        assert_eq!(station.beacon(None), Err(BEACON_UNDER_AUTOMATIC_CONTROL));
        assert_eq!(
            station.beacon(Some("W4ODA-9")),
            Err(BEACON_UNDER_AUTOMATIC_CONTROL)
        );
        assert!(BEACON_UNDER_AUTOMATIC_CONTROL.contains("§97.203(d)"));
        let mut station = lone_station(under_us_rules(14_105_000, ControlMode::Local));
        station.beacon(None).expect("local control");
    }

    #[test]
    fn a_beacon_crosses_the_air_and_is_reported_but_never_answered() {
        // "can anybody hear me?" without arranging a contact first, which on HF is most of
        // what a new station needs to know
        let mut air = Air::new(1.0, 0.0005);
        air.a.beacon(None).expect("idle");
        air.run(30.0, |_, b| b.stats.beacons_heard > 0);

        assert_eq!(air.b.stats.beacons_heard, 1, "the beacon was not heard");
        let events = air.b.take_events();
        assert!(
            events.iter().any(|e| e.starts_with("beacon:heard W4ODA")),
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
    fn a_beacon_repeats_on_its_timer_and_says_what_it_did() {
        // ND1J, 2026-09-25: where are beacons sent shown, and how does one make a beacon
        // repeat every so many minutes?
        let mut station = idle_station();
        // a busy detector that has not listened yet holds a beacon back: let it settle
        run_alone(&mut station, 4.0, |_| false);
        station.take_events();
        assert!(
            station.beacon_every(Some(60.0)).is_err(),
            "a minute is too often"
        );
        station.beacon_every(Some(600.0)).expect("ten minutes");
        run_alone(&mut station, 10.0, |s| {
            s.stats.beacons_sent > 0 && s.quiescent()
        });
        assert_eq!(station.stats.beacons_sent, 1, "the first goes at once");
        let events = station.take_events();
        assert!(
            events
                .iter()
                .any(|e| e == "beacon:sent; the next in 10 min"),
            "the beacon that went out was not said: {events:?}"
        );
        assert_eq!(station.beacon_status()["sent"], 1);
        assert!(station.beacon_status()["last_sent_ms"].is_u64());
        // not again until it falls due
        run_alone(&mut station, 5.0, |_| false);
        assert_eq!(station.stats.beacons_sent, 1);
        station.beacons.next_s = station.now();
        run_alone(&mut station, 10.0, |s| {
            s.stats.beacons_sent > 1 && s.quiescent()
        });
        assert_eq!(station.stats.beacons_sent, 2);

        // one the station could not send within the window is skipped, said so, and the next
        // keeps to the interval rather than catching up
        station.beacons.next_s = station.now() - beacons::BEACON_WINDOW_S - 1.0;
        station.pending.push_back(Outgoing::Pause {
            seconds: 30.0,
            until_s: None,
        });
        station.take_events();
        run_alone(&mut station, 1.0, |_| false);
        assert_eq!(station.beacons.skipped, 1);
        assert!(station.beacons.next_s > station.now());
        assert!(
            station
                .take_events()
                .iter()
                .any(|e| e.starts_with("beacon:skipped: the station was busy")),
        );
        assert_eq!(station.stats.beacons_sent, 2);

        // stopped, it stops
        station.pending.clear();
        station.beacon_every(None).expect("stops");
        station.beacons.next_s = station.now();
        run_alone(&mut station, 5.0, |_| false);
        assert_eq!(station.stats.beacons_sent, 2);
        assert!(station.beacon_status()["every_s"].is_null());
    }

    #[test]
    fn a_repeating_beacon_needs_a_control_operator() {
        use crate::regulatory::ControlMode;
        // a beacon may be automatically controlled only on 28.20-28.30 MHz and above
        // (§97.203(d)); an answer-only station sends none at all
        let mut station = lone_station(under_us_rules(14_105_000, ControlMode::Automatic));
        let refusal = station.beacon_every(Some(900.0)).expect_err("automatic");
        assert!(refusal.contains("§97.203(d)"), "{refusal}");
        let mut station = lone_station(under_us_rules(14_105_000, ControlMode::Local));
        station.beacon_every(Some(900.0)).expect("local control");
        // the rules changing under it stop it at the next one due
        station.config.regulatory.control = Some(ControlMode::Automatic);
        run_alone(&mut station, 1.0, |_| false);
        assert!(station.beacon_status()["every_s"].is_null());
        assert_eq!(station.stats.beacons_sent, 0);
        let mut station = idle_station();
        station.config.answer_only = true;
        assert!(station.beacon_every(Some(900.0)).is_err());
    }

    #[test]
    fn the_identifier_says_when_the_rules_slow_it() {
        use crate::regulatory::ControlMode;
        // ND1J set a faster Morse speed and heard the same 20 wpm whatever he set: the US
        // rules' limit on an automatically keyed identifier (§97.119(b)(1)), never mentioned
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                cw_id: Some(CwId {
                    wpm: 30.0,
                    ..CwId::default()
                }),
                regulatory: under_us_rules(14_105_000, ControlMode::Local),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        assert_eq!(station.identifier_wpm(), Some(20.0));
        let events = station.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("identifier:set to 30 wpm, sent at 20 wpm")),
            "{events:?}"
        );
        let status = station.identifier_status();
        assert_eq!(status["wpm"], 30.0);
        assert_eq!(status["sent_wpm"], 20.0);
        // slower than the limit is sent as set, and said nothing of
        station.config.cw_id = Some(CwId {
            wpm: 15.0,
            ..CwId::default()
        });
        assert_eq!(station.identifier_wpm(), Some(15.0));
        station.note_identifier_speed();
        let unexpected = station.take_events();
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    #[test]
    fn the_identifier_follows_its_settings_without_a_restart() {
        let mut station = idle_station();
        assert!(station.config.cw_id.is_none());
        let mut config = crate::config::Config::parse(
            "callsign = \"W4ODA\"
",
        )
        .expect("parses");
        config.radio.cw_id = true;
        config.radio.cw_id_wpm = 18.0;
        assert!(crate::config::LIVE_KEYS.contains(&"radio.cw_id_wpm"));
        station.apply_live(&config);
        assert_eq!(station.config.cw_id.map(|cw| cw.wpm), Some(18.0));
        assert!((station.config.cw_id_wpm - 18.0).abs() < 1e-9);
        config.radio.cw_id = false;
        station.apply_live(&config);
        assert!(station.config.cw_id.is_none());
    }

    #[test]
    fn each_frame_of_a_burst_says_over_the_air_how_many_follow_it() {
        // the countdown rides in the chips (ADR-0041): through the real modem and the audio,
        // the receiver reads every data frame of a burst counting down to its last
        // incompressible, so it takes whole bursts
        let message: Vec<u8> = (0..1500u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        air.a.send(&message);
        let mut reports = Vec::new();
        for _ in 0..40 {
            air.run(5.0, |_, _| false);
            reports.extend(air.b.take_frame_reports());
            if air.b.received_len() >= message.len() {
                break;
            }
        }
        assert_eq!(air.b.take_received().as_slice(), message.as_slice());
        let counts: Vec<Option<u8>> = reports
            .iter()
            .filter(|r| r.kind == "data" && r.decoded)
            .map(|r| r.follows)
            .collect();
        assert!(
            counts.iter().any(|&c| c.is_some_and(|n| n > 0)),
            "no frame said more of its burst followed: {counts:?}"
        );
        assert!(
            counts.windows(2).all(|w| match (w[0], w[1]) {
                // within a burst the count of pairs still to come (ADR-0046) holds or drops by
                // one from frame to frame — 3, 2, 2, 1, 1, 0 for six — and a burst's last says
                // none
                (Some(a), Some(b)) => b == a || b + 1 == a || a == 0,
                _ => true,
            }),
            "the countdown did not count down: {counts:?}"
        );
        assert!(
            reports
                .iter()
                .filter(|r| r.kind == "control")
                .all(|r| r.follows.is_none()),
            "a control frame cannot say"
        );
    }

    #[test]
    fn every_frame_is_reported_with_what_the_receiver_made_of_it() {
        // the readings a panel shows come from the receiver's own estimates, frame by
        // frame: a beacon names its sender outright, and once a session is up the frames
        // with its id are the other station's
        let mut air = Air::new(1.0, 0.0005);
        assert!(air.b.last_frame().is_none());
        assert!(air.b.constellation().is_none());
        air.a.beacon(None).expect("idle");
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
        // the answer to the data goes in the slot after it
        air.run(120.0, |a, b| {
            b.received_len() >= 300 && a.engine().all_acknowledged()
        });
        let on_b = air.b.take_frame_reports();
        let data: Vec<_> = on_b.iter().filter(|r| r.kind == "data").collect();
        assert!(!data.is_empty(), "data is empty");
        assert!(
            data.iter().all(|r| r.from.as_deref() == Some("W4ODA")),
            "{data:?}"
        );
        let on_a = air.a.take_frame_reports();
        let acks: Vec<_> = on_a.iter().filter(|r| r.kind == "control").collect();
        assert!(!acks.is_empty(), "acks is empty");
        assert!(acks.iter().all(|r| r.from.as_deref() == Some("KK4XYZ")));
        assert!(acks.iter().all(|r| {
            r.control
                .as_deref()
                .is_some_and(|c| c.starts_with("Answer"))
        }));
        // the acknowledgements of OFDM bursts are OFDM frames at the control mode, as beacons
        // were before ADR-0016: a wire reads past the tone floor's ceiling (+17 dB), and the
        // last one's symbols sit on their constellation — none near the origin. How far past
        // depends on the frame's own bits: on a wire the reading is the waveform's own floor,
        // which the model puts at +20.4 to +24.9 dB for an acknowledgement as its count and
        // rung vary (ADR-0068 changed the bits this one carries, and it read +19.2)
        assert!(acks.iter().all(|r| r.snr_db > 18.0), "{acks:?}");
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
        // the chart's rate lines: the data went one way, at the caller's rung, and the called
        // station only answered — its own rung moved no bytes (12-two-paths-80m.md §16)
        let (sent, _) = air.a.data_rungs();
        let (answered, heard) = air.b.data_rungs();
        assert!(
            sent.is_some() && heard == sent,
            "{sent:?} sent, {heard:?} heard"
        );
        assert_eq!(answered, None);

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
        air.a.beacon(None).expect("idle");
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

    /// Another station's tone-floor beacon as it goes on the air, from keying to release, at
    /// the level it was sent: a real frame of the tone floor, for a station to hear.
    fn tone_beacon_as_sent(callsign: &str, block: usize) -> Vec<f32> {
        let mut other = Station::new(
            StationConfig {
                callsign: callsign.to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            2,
        );
        other.beacon(None).expect("an idle station beacons");
        let mut audio = Vec::new();
        let mut out = vec![0.0f32; block];
        let silence = vec![0.0f32; block];
        let mut keyed = false;
        while !keyed || other.transmitting() {
            assert!(audio.len() < 20 * 48_000, "the beacon never ended");
            other.playback(&mut out).expect("playback");
            other.capture(&silence).expect("capture");
            keyed |= other.transmitting();
            audio.extend_from_slice(&out);
        }
        audio
    }

    #[test]
    fn a_frame_heard_as_the_station_listens_again_is_not_taken_for_its_passband() {
        // The bench pair at 12 dB (2026-09-26): over a channel whose noise is flat to 4 kHz,
        // the panel said the radio's receive filter looked about 650 Hz wide while tone-floor
        // frames went back and forth. The passband monitor had been sampling the other
        // station's frames. A station's deafness to its own tail ends with the answer
        // already arriving, the busy detector takes 200 ms to call even a strong signal, and
        // the one look the monitor got between two bursts of a session was the answer's
        // 400 Hz of tones — turnaround after turnaround, with no quiet look in between to
        // dilute them. A frame that starts just before a look while the station listens got
        // in the same way. So a real tone frame answers every burst here, and one arrives
        // while the station listens, over flat noise: the reading has to stay the whole
        // band's, and the monitor has to go on learning when the channel is quiet.
        let mut station = idle_station();
        let rate = station.config.params.audio_rate as f64;
        let occupied = station.occupied_bandwidth_hz();
        let block = 960; // a sound card's period, 20 ms
        let answer = tone_beacon_as_sent("KK4XYZ", block);
        // the silence it opens with, the keying's lead
        let lead_s = answer.iter().position(|&x| x != 0.0).expect("a frame") as f64 / rate;
        // noise flat to 24 kHz, 12 dB under the OFDM average in 3 kHz, as `[sim]` adds it
        let sigma = (station.config.tx_level / 2f64.sqrt() * (8.0 / 10f64.powf(1.2)).sqrt()) as f32;
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut hiss = move || {
            let mut uniform = || {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
            };
            let (u1, u2) = (uniform().max(1e-12), uniform());
            ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32 * sigma
        };
        // one period: the station plays what it has, and hears the noise and `signal`
        let mut out = vec![0.0f32; block];
        let mut step = |station: &mut Station<NullPtt>, signal: &[f32]| {
            station.playback(&mut out).expect("playback");
            let heard: Vec<f32> = (0..block)
                .map(|i| signal.get(i).copied().unwrap_or(0.0) + hiss())
                .collect();
            station.capture(&heard).expect("capture");
        };
        let blocks = |seconds: f64| (seconds * rate) as usize / block;

        for _ in 0..blocks(10.0) {
            step(&mut station, &[]);
        }
        let learned = station
            .rx_passband_hz()
            .expect("ten seconds of noise is enough");
        assert!(learned > occupied, "flat noise read as {learned:.0} Hz");
        let looks_before = station.passband.observations();
        let mut narrowest = f64::INFINITY;
        let mut hear = |station: &mut Station<NullPtt>, signal: &[f32]| {
            step(station, signal);
            if let Some(width) = station.rx_passband_hz() {
                narrowest = narrowest.min(width);
            }
        };
        // a session's turnarounds: this station transmits — a tune tone will do, the deafness
        // after it is the same whatever was sent — and the answer comes at once: it starts
        // while the station is still deaf to its own tail, and is on the air when that ends.
        // The next transmission follows it straight away, as a sender's follows an
        // acknowledgement, inside the two seconds a decoded frame holds the channel busy:
        // the monitor has no quiet moment between turnarounds to dilute what it takes in
        for _ in 0..3 {
            station.tune(0.5).expect("an idle station tunes");
            let mut keyed = false;
            while !keyed || station.transmitting() {
                hear(&mut station, &[]);
                keyed |= station.transmitting();
            }
            for chunk in answer.chunks(block) {
                hear(&mut station, chunk);
            }
        }
        // then frames that start while the station listens, each on the air for 150 ms —
        // under the 200 ms the busy detector takes — when the monitor next looks, with one
        // quiet look before each
        for _ in 0..3 {
            let (last, since) = (station.passband_next_s, station.now());
            while station.passband_next_s <= last {
                assert!(station.now() - since < 20.0, "the monitor stopped looking");
                hear(&mut station, &[]);
            }
            while station.passband_next_s - station.now() > lead_s + 0.15 {
                hear(&mut station, &[]);
            }
            for chunk in answer.chunks(block) {
                hear(&mut station, chunk);
            }
        }
        // and the quiet after them, which the monitor must still learn from
        for _ in 0..blocks(8.0) {
            hear(&mut station, &[]);
        }
        assert!(
            narrowest > occupied,
            "the monitor learned a {narrowest:.0} Hz passband from frames over flat noise, \
             where the modem needs {occupied:.0} Hz"
        );
        let looks = station.passband.observations() - looks_before;
        assert!(
            looks >= 6,
            "only {looks} looks at the noise between the frames: the monitor stopped listening"
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

    /// A station of `params` calling as W4ODA, for the tests of the two airs meeting.
    fn caller_on(params: aether_phy::waveform::WaveformParams) -> Station<NullPtt> {
        Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                params,
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        )
    }

    #[test]
    fn a_wide_station_answers_a_narrow_call_at_500_hz_and_goes_back_after() {
        // ADR-0026, as VARA's "Accept 500 Hz connections": the two OFDM waveforms do not
        // decode each other's preambles, but the tone floor (ADR-0013) is the same frames on
        // both airs, so a 500 Hz call's first try — on the floor since ADR-0016 — reaches a
        // 2 300 Hz station, which moves to 500 Hz and answers on the caller's air
        let mut air = Air::new(1.0, 0.0005);
        air.a = caller_on(aether_phy::waveform::NARROW_500);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(90.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.connected() && air.b.connected(),
            "{:?}",
            air.b.take_events()
        );
        assert_eq!(air.b.bandwidth_hz(), 500);
        let status = air.b.bandwidth_status();
        assert_eq!(status["why"], "call");
        assert_eq!(status["caller"], "W4ODA");
        assert_eq!(status["home_hz"], 2300);
        assert_eq!(
            aether_link::frames::bandwidth_code(air.b.engine().peer_capabilities()),
            1
        );
        // whatever the wide station heard before it moved was the floor's, never the narrow
        // OFDM: its first report is a floor rung
        let reports = air.b.take_frame_reports();
        assert!(reports[0].mode < 2 && reports[0].decoded, "{reports:?}");
        let message = b"answered in the bandwidth it was called in";
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received(), message);
        // the session ends; the station stays for a quiet spell — the other station's
        // goodbye, a call that follows a ping — and then goes back to its own
        air.a.disconnect();
        air.run(60.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle
        });
        assert_eq!(air.b.state(), State::Idle);
        assert_eq!(air.b.bandwidth_hz(), 500);
        air.run(RETURN_QUIET_S + 10.0, |_, b| b.bandwidth_hz() == 2300);
        assert_eq!(air.b.bandwidth_hz(), 2300);
        assert_eq!(air.b.bandwidth_status()["why"], "configured");
        assert_eq!(
            aether_link::frames::bandwidth_code(air.b.engine().config().capabilities),
            0
        );
        let events = air.b.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("bandwidth:2300 Hz → 500 Hz: W4ODA called in it")),
            "{events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("bandwidth:500 Hz → 2300 Hz")),
            "{events:?}"
        );
        // and it answers at 2 300 Hz as before
        air.a = caller_on(WIDE_2300);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(60.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        assert_eq!(air.b.bandwidth_hz(), 2300);
    }

    #[test]
    fn a_narrow_station_does_not_answer_a_wide_call() {
        // never the other way: a 2 300 Hz signal where the operator or a host program chose
        // 500 Hz — a 500 Hz calling frequency, most of all — is nobody's choice. The wide
        // call's floor tries reach the narrow station, which leaves them alone, because the
        // call states its bandwidth
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        });
        air.a = caller_on(WIDE_2300);
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
        assert_eq!(air.b.bandwidth_hz(), 500);
        // and it says so where the operator looks, with both bandwidths and the fix
        // (ADR-0035), in the log once a minute, not once a try
        let mismatch = air
            .b
            .mismatch()
            .expect("a wide call to a narrow station")
            .clone();
        assert_eq!(
            (mismatch.callsign.as_str(), mismatch.what),
            ("W4ODA", "call")
        );
        assert_eq!((mismatch.theirs_hz, mismatch.ours_hz), (2300, 500));
        assert!(
            mismatch.sentence.contains("Setup step 4"),
            "{}",
            mismatch.sentence
        );
        assert_eq!(air.b.bandwidth_status()["mismatch"]["what"], "call");
        let warned = events.iter().filter(|e| e.starts_with("mismatch:")).count();
        assert_eq!(warned, 1, "{events:?}");
        // and whatever the narrow station heard was the floor's, never the wide OFDM
        let floor = 2;
        let reports = air.b.take_frame_reports();
        assert!(!reports.is_empty(), "reports is empty");
        assert!(
            reports.iter().all(|r| r.mode < floor && r.decoded),
            "{reports:?}"
        );
    }

    #[test]
    fn a_call_in_another_link_protocol_says_who_needs_to_update() {
        // a station of an older version calls: the engine ignores it, as it always has, and the
        // panel now says why and which side needs updating — not only a log line (ADR-0041)
        use aether_link::frames::{ConnectBody, DataHeader, PROTOCOL_VERSION, encode_data};
        let mut air = Air::new(1.0, 0.0005);
        let body = ConnectBody {
            src: "W4ODA".into(),
            dst: "KK4XYZ".into(),
            caps: aether_link::frames::with_bandwidth(0, 2300),
            version: PROTOCOL_VERSION - 1,
            snr_db: None,
            turnaround_s: 0.0,
            identifier_s: 0.0,
        }
        .encode()
        .expect("body");
        let header = DataHeader {
            kind: DataKind::ConnectReq,
            seq: 0,
            session: 9,
        };
        let mode = air.a.engine.robust_mode(true);
        let capacity = air.a.engine.timing().capacity(mode);
        let payload = encode_data(&header, &body, capacity).expect("fits");
        air.a.pending.push_back(Outgoing::Frames(
            vec![aether_link::TxFrame {
                container: Container::Data,
                payload,
                mode,
                rv: 0,
                floor: false,
                follows: 0,
            }],
            Origin::station("test"),
        ));
        air.run(30.0, |_, b| b.mismatch().is_some());
        assert!(!air.b.connected());
        let mismatch = air.b.mismatch().expect("the call was noticed").clone();
        assert_eq!(
            (mismatch.callsign.as_str(), mismatch.what),
            ("W4ODA", "version")
        );
        assert_eq!(
            (mismatch.theirs_protocol, mismatch.ours_protocol),
            (Some(PROTOCOL_VERSION - 1), Some(PROTOCOL_VERSION))
        );
        assert!(
            mismatch.sentence.contains("W4ODA runs an older version")
                && mismatch.sentence.contains("needs to update"),
            "{}",
            mismatch.sentence
        );
        assert_eq!(
            air.b.bandwidth_status()["mismatch"]["theirs_protocol"],
            PROTOCOL_VERSION - 1
        );
    }

    #[test]
    fn a_call_to_another_ssid_of_ours_is_shown_and_can_be_answered() {
        // ADR-0057, from the air: KO4WX called KK4ODA, and the station, answering as KK4ODA-1,
        // ignored every try without a word. The call is shown with the callsign it named,
        // and a station told to answer to that name too answers the next try
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ-1").expect("idle");
        air.run(60.0, |_, b| b.mismatch().is_some());
        assert!(!air.b.connected());
        let mismatch = air.b.mismatch().expect("the call was noticed").clone();
        assert_eq!(
            (
                mismatch.callsign.as_str(),
                mismatch.what,
                mismatch.called.as_deref()
            ),
            ("W4ODA", "callsign", Some("KK4XYZ-1"))
        );
        assert!(
            mismatch.sentence.contains("W4ODA is calling KK4XYZ-1")
                && mismatch.sentence.contains("answers as KK4XYZ"),
            "{}",
            mismatch.sentence
        );
        air.b
            .set_callsigns(&["KK4XYZ".to_owned(), "KK4XYZ-1".to_owned()])
            .expect("idle");
        air.run(120.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        assert_eq!(air.b.engine().my_call, "KK4XYZ-1");
    }

    #[test]
    fn an_unanswered_call_suggests_another_ssid_heard_lately() {
        // ADR-0057: a call to KK4XYZ-5 that nobody answers, from a station that heard KK4XYZ
        // beacon minutes ago, suggests calling KK4XYZ
        let mut air = Air::new(1.0, 0.0005);
        air.b.beacon(None).expect("idle");
        air.run(30.0, |a, _| a.stats.beacons_heard > 0);
        assert_eq!(air.a.stats.beacons_heard, 1);
        air.a.connect("KK4XYZ-5").expect("idle");
        air.run(600.0, |a, _| {
            a.mismatch().is_some_and(|m| m.what == "suggest")
        });
        let mismatch = air.a.mismatch().expect("a suggestion").clone();
        assert_eq!(
            (mismatch.callsign.as_str(), mismatch.called.as_deref()),
            ("KK4XYZ-5", Some("KK4XYZ"))
        );
        assert!(
            mismatch
                .sentence
                .starts_with("No answer from KK4XYZ-5. KK4XYZ was heard"),
            "{}",
            mismatch.sentence
        );
    }

    #[test]
    fn a_wide_call_to_another_ssid_of_a_narrow_station_is_shown() {
        // ADR-0057: KO4WX called KK4ODA at 2 300 Hz of a station answering as KK4ODA-1 at
        // 500 Hz; the bandwidth warning now covers a call to another name of the station's
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        });
        air.a = caller_on(WIDE_2300);
        air.a.connect("KK4XYZ-1").expect("idle");
        air.run(60.0, |_, b| b.mismatch().is_some());
        let mismatch = air.b.mismatch().expect("the call was noticed").clone();
        assert_eq!(
            (
                mismatch.what,
                mismatch.theirs_hz,
                mismatch.called.as_deref()
            ),
            ("call", 2300, Some("KK4XYZ-1"))
        );
        assert!(
            mismatch
                .sentence
                .starts_with("W4ODA called KK4XYZ-1 at 2300 Hz"),
            "{}",
            mismatch.sentence
        );
    }

    #[test]
    fn a_wide_call_nobody_answers_goes_on_at_500_hz_and_a_narrow_station_answers() {
        // ADR-0058, from the air: KO4WX called this station at 2 300 Hz for minutes while it
        // ran 500 Hz. A 2 300 Hz call to a station of unknown bandwidth that four tries have
        // not had answered goes on at 500 Hz, where the narrow station answers it
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        });
        air.a = caller_on(WIDE_2300);
        air.a.connect("KK4XYZ").expect("idle");
        air.run(400.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.connected() && air.b.connected(),
            "{:?}",
            air.a.take_events()
        );
        assert_eq!(air.a.bandwidth_hz(), 500);
        assert_eq!(air.a.bandwidth_status()["why"], "calling");
        assert!(air.a.engine().connect_tries() > super::bandwidth::NARROW_AFTER_TRIES);
        let message = b"carried on at 500 Hz";
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received(), message);
    }

    #[test]
    fn a_wide_station_probes_a_narrow_one_and_calls_it_at_500_hz() {
        // ADR-0035, from the air: KK4ODA-1 at 500 Hz and WC4Y at 2 300 Hz each ignored the
        // other's probes and calls "in another bandwidth" for five hours. A probe is answered
        // across bandwidths now, its answer says the other runs 500 Hz, and a 2 300 Hz station
        // calling a station it knows runs 500 Hz moves to 500 Hz first — and back after
        let mut air = Air::with(1.0, 0.0005, |config| StationConfig {
            params: aether_phy::waveform::NARROW_500,
            ..config
        });
        air.a = caller_on(WIDE_2300);
        assert_eq!(air.a.known_bandwidth("KK4XYZ"), None);
        air.a.probe("KK4XYZ", None).expect("idle");
        air.run(60.0, |a, _| a.engine().last_probe().is_some());
        let probe = air.a.engine().last_probe().expect("answered").clone();
        assert_eq!(probe.bandwidth_hz, Some(500));
        assert_eq!(air.a.known_bandwidth("KK4XYZ"), Some(500));
        let events = air.a.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("probe:KK4XYZ hears us at")
                    && e.ends_with("— runs 500 Hz, this station 2300 Hz")),
            "{events:?}"
        );
        // the narrow station answered, and warns that a call from the wide one would not be
        assert_eq!(air.b.engine().stats.probes_answered, 1);
        let mismatch = air.b.mismatch().expect("a wide probe").clone();
        assert_eq!((mismatch.what, mismatch.theirs_hz), ("probe", 2300));
        // the call goes out at 500 Hz, and is answered
        air.a.connect("KK4XYZ").expect("idle");
        assert_eq!(air.a.bandwidth_hz(), 500);
        assert_eq!(air.a.bandwidth_status()["why"], "calling");
        assert_eq!(air.a.bandwidth_status()["callee"], "KK4XYZ");
        air.run(90.0, |a, b| a.connected() && b.connected());
        assert!(
            air.a.connected() && air.b.connected(),
            "{:?}",
            air.a.take_events()
        );
        let message = b"called in the bandwidth it runs";
        air.a.send(message);
        air.run(120.0, |_, b| b.received_len() >= message.len());
        assert_eq!(air.b.take_received(), message);
        air.a.disconnect();
        air.run(60.0, |a, b| {
            a.state() == State::Idle && b.state() == State::Idle
        });
        assert_eq!(air.a.state(), State::Idle);
        assert_eq!(air.a.bandwidth_hz(), 500);
        air.run(RETURN_QUIET_S + 10.0, |a, _| a.bandwidth_hz() == 2300);
        assert_eq!(air.a.bandwidth_hz(), 2300);
        assert_eq!(air.a.bandwidth_status()["why"], "configured");
        // a station known to run 2 300 Hz, or not known at all, is called as before
        air.a.learn_bandwidth("N0CALL", 2300);
        air.a.connect("N0CALL").expect("idle");
        assert_eq!(air.a.bandwidth_hz(), 2300);
        air.a.abort();
    }

    #[test]
    fn the_dial_is_not_moved_under_a_probe_or_a_call() {
        // the panel tunes the radio on a pick from the dial list, so the daemon refuses a
        // move that would leave an answer arriving where the radio no longer listens
        let mut station = idle_station();
        station.probe("KK4XYZ", None).expect("idle");
        let refused = station.tune_to(7_101_000).expect_err("probing");
        assert!(refused.contains("probe"), "{refused}");
        station.abort();
        let mut station = idle_station();
        station.connect("KK4XYZ").expect("idle");
        let refused = station.tune_to(7_101_000).expect_err("calling");
        assert!(refused.contains("call"), "{refused}");
    }

    #[test]
    fn a_beacon_still_waiting_when_a_session_starts_does_not_go_out_in_it() {
        // a beacon held for a busy channel — and the busy channel was a call — must not go
        // out in the middle of the session it started (ADR-0038); a session refuses a new
        // beacon already
        let mut station = idle_station();
        station.beacon(None).expect("idle");
        assert!(station.beacon_waiting());
        station.connect("KK4XYZ").expect("idle");
        assert_ne!(station.engine().state(), State::Idle);
        assert_eq!(station.beacon(None), Err("a session is running"));
        let _ = station.held_for_busy(station.now());
        assert!(
            !station.beacon_waiting(),
            "the beacon stayed queued under the session"
        );
        assert!(
            station
                .take_events()
                .iter()
                .any(|e| e == "beacon:dropped: a session started before it went out"),
            "and nobody was told"
        );
    }

    #[test]
    fn a_beacon_waiting_for_the_channel_goes_with_the_radio() {
        // ND1J, 2026-09-29: two beacons pressed on a busy channel sat in the queue, and the
        // dial list refused to move him off it with "the station is transmitting". A beacon
        // waits once, is said to be waiting, and does not hold the dial
        let mut station = idle_station();
        assert_eq!(station.beacon_status()["waiting"], false);
        station.beacon(None).expect("idle");
        assert!(station.beacon_waiting());
        assert_eq!(station.beacon_status()["waiting"], true);
        assert_eq!(station.beacon(None), Err(BEACON_ALREADY_WAITING));
        // the tune gets past every check of the station's own: what refuses it here is the
        // test harness's keying, which cannot tune any radio
        let refused = station.tune_to(7_082_000).expect_err("no CAT here");
        assert!(refused.contains("keying interface has no way"), "{refused}");
        // what belongs to this dial holds it, and says what it is
        let mut station = idle_station();
        station.key_test(1.0).expect("idle");
        let refused = station
            .tune_to(7_082_000)
            .expect_err("a keying test queued");
        assert_eq!(
            refused,
            "a keying test is waiting to go out on this frequency"
        );
    }

    #[test]
    fn a_station_is_known_under_its_other_names_when_they_agree() {
        // ADR-0035: VarAC beacons as KK4ODA-9 and pings KK4ODA-1-T; the operator calls
        // KK4ODA-1. The base callsign carries what was heard, unless its names disagree
        use super::bandwidth::base_callsign;
        assert_eq!(base_callsign("KK4ODA-1-T"), "KK4ODA");
        assert_eq!(base_callsign("VE3/KK4ODA/P"), "KK4ODA");
        assert_eq!(base_callsign(" w4tga "), "w4tga");
        let mut station = idle_station();
        assert_eq!(station.known_bandwidth("KK4ODA-1"), None);
        station.learn_bandwidth("kk4oda-9", 500);
        assert_eq!(station.known_bandwidth("KK4ODA-1"), Some(500));
        assert_eq!(station.known_bandwidth("KK4ODA-1-T"), Some(500));
        assert_eq!(station.known_bandwidth("KK4ODB"), None);
        // the operator's two stations in two bandwidths: no guess for a third name …
        station.learn_bandwidth("KK4ODA-2", 2300);
        assert_eq!(station.known_bandwidth("KK4ODA-1"), None);
        // … while a name heard itself is known as it was heard
        assert_eq!(station.known_bandwidth("KK4ODA-9"), Some(500));
        assert_eq!(station.known_bandwidth("KK4ODA-2"), Some(2300));
        // and a call to it goes out in the station's own
        station.connect("KK4ODA-1").expect("idle");
        assert_eq!(station.bandwidth_hz(), 2300);
        station.abort();
    }

    #[test]
    fn a_host_program_moves_the_bandwidth_between_sessions_and_takes_it_back() {
        // VARA's BW commands set the modem's mode (ADR-0026): the station moves while idle,
        // holds the host's bandwidth while the host is attached, and goes back when it goes
        let mut station = idle_station();
        assert_eq!(station.bandwidth_hz(), 2300);
        station.set_host(HostPresence {
            attached: true,
            listening: true,
            chat: true,
        });
        assert_eq!(station.host_bandwidth(Some(500)), Ok(500));
        assert_eq!(station.bandwidth_hz(), 500);
        assert_eq!(station.bandwidth_status()["why"], "host");
        assert_eq!(station.bandwidth_status()["host_hz"], 500);
        // the engine is on the narrow air: its ladder, its handshake bandwidth, its top rung
        let narrow = aether_phy::modes::air_interface(aether_phy::waveform::NARROW_500);
        assert_eq!(
            station.engine().timing().data_capacity.len(),
            narrow.n_rungs()
        );
        assert_eq!(station.engine().config().max_mode, narrow.n_rungs() - 1);
        assert_eq!(
            aether_link::frames::bandwidth_code(station.engine().config().capabilities),
            1
        );
        // 2750, Winlink Express's widest, is 2300: a narrower signal is inside what was asked
        assert_eq!(station.host_bandwidth(Some(2750)), Ok(2300));
        assert_eq!(station.bandwidth_hz(), 2300);
        assert!(station.host_bandwidth(Some(1800)).is_err());
        // nothing moves while a call is going out: it runs on the air it started on
        station.host_bandwidth(Some(500)).expect("idle");
        station.connect("KK4ABC").expect("idle");
        let refused = station.host_bandwidth(Some(2300)).expect_err("calling");
        assert!(refused.contains("a call is going out"), "{refused}");
        assert_eq!(station.bandwidth_hz(), 500);
        station.abort();
        // the host goes, and takes its bandwidth with it once the station is quiet
        station.set_host(HostPresence::default());
        let rate = WIDE_2300.audio_rate as f64;
        let mut out = vec![0.0f32; 4096];
        for seed in 0..(30.0 * rate / 4096.0) as u32 {
            station.playback(&mut out).expect("playback");
            station.capture(&quiet_block(seed)).expect("capture");
            if station.bandwidth_hz() == 2300 {
                break;
            }
        }
        assert_eq!(station.bandwidth_hz(), 2300);
        assert_eq!(station.bandwidth_status()["why"], "configured");
        assert_eq!(
            station.bandwidth_status()["host_hz"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn a_station_whose_host_has_not_said_listen_on_answers_no_call() {
        // VARA's LISTEN OFF, its default: incoming connections disabled until the program
        // attached says LISTEN ON (or CHAT ON, which includes it)
        let mut air = Air::new(1.0, 0.0005);
        air.b.set_host(HostPresence {
            attached: true,
            listening: false,
            chat: false,
        });
        assert!(!air.b.answering());
        air.a.connect("KK4XYZ").expect("idle");
        air.run(40.0, |_, b| b.stats.calls_unanswered >= 2);
        assert!(!air.b.connected());
        assert!(air.b.stats.calls_unanswered >= 2);
        let events = air.b.take_events();
        let said: Vec<&String> = events
            .iter()
            .filter(|e| e.starts_with("listen:a call from W4ODA to KK4XYZ not answered"))
            .collect();
        // said once for the caller's tries, not once a try
        assert_eq!(said.len(), 1, "{events:?}");
        // the program says LISTEN ON: the next try is answered
        air.b.set_host(HostPresence {
            attached: true,
            listening: true,
            chat: false,
        });
        air.run(90.0, |a, b| a.connected() && b.connected());
        assert!(air.a.connected() && air.b.connected());
        // and a station with no program attached answers, as it always did
        assert!(HostPresence::default().answering());
    }

    #[test]
    fn the_radios_turnaround_and_the_answer_gap_reach_the_engine() {
        // the turnaround is this radio's, declared in the next call or acceptance; the answer
        // gap is the least wait after the other station's frames, the larger of the
        // operator's and the one learned for the station answered (ADR-0054, ADR-0069)
        let mut station = idle_station();
        let config = crate::config::Config::parse(&format!(
            "schema_version = {}\ncallsign = \"KK4XYZ\"\n[radio]\nturnaround_ms = 800\n\
             answer_gap_ms = 600\n",
            crate::config::SCHEMA_VERSION
        ))
        .expect("a configuration");
        station.apply_live(&config);
        run_alone(&mut station, 0.2, |_| false);
        assert!((station.engine().config().turnaround_s - 0.8).abs() < 1e-9);
        assert!((station.engine().config().answer_gap_s - 0.6).abs() < 1e-9);
        station.remember_gap("W4ODA", 1.0);
        station.connect("W4ODA").expect("idle");
        run_alone(&mut station, 0.2, |_| false);
        assert!(
            (station.engine().config().answer_gap_s - 1.0).abs() < 1e-9,
            "the gap learned for the station called"
        );
        station.abort();
    }

    #[test]
    fn a_new_fastest_rung_reaches_the_link_and_follows_the_ladder_running() {
        // `radio.max_mode` is live: it reaches the engine, not only the gate and the Test's
        // ladder; and it indexes the ladder of the bandwidth running (ADR-0026)
        let mut station = idle_station();
        let mut config = crate::config::Config::parse(&format!(
            "schema_version = {}\ncallsign = \"KK4XYZ\"\n[radio]\nmax_mode = 8\n",
            crate::config::SCHEMA_VERSION
        ))
        .expect("a configuration");
        station.apply_live(&config);
        assert_eq!(station.engine().config().max_mode, 8);
        config.radio.max_mode = 19;
        station.apply_live(&config);
        assert_eq!(station.engine().config().max_mode, 19);
        station.host_bandwidth(Some(500)).expect("idle");
        assert_eq!(station.engine().config().max_mode, 14);
        station.host_bandwidth(Some(2300)).expect("idle");
        assert_eq!(station.engine().config().max_mode, 19);
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
        assert!(air.a.beacon(None).is_err());
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
        assert_eq!(air.a.role(), Role::Slave);
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
        assert!(air.a.beacon(None).is_err(), "it beaconed during a session");
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
        station.heed_preambles(&[phantom], 0, now);
        assert!(
            !station.channel_busy(),
            "a phantom acquisition must not silence the station"
        );

        // well clear of it, as a frame would be — still nothing: acquisition is not evidence
        let confident = preamble(2.5);
        station.heed_preambles(&[confident], 0, now);
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
                follows: 0,
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
    fn a_session_heard_between_other_stations_holds_this_stations_calls() {
        // the harness (`40m-gateway-busy-when-called-2300`, Step 3 §3.2): a client that had
        // waited 108 s for a gateway's session to end called in the 0.6 s between two of its
        // transmissions, and the session's next three went over the call. A frame decoded of a
        // session not this station's holds its own calls through the quiet inside a session
        use super::reserve::{Heard, RESERVE_GAP_S};
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let ours = station.engine.session() & 0x7F;
        let other = ours ^ 0x15;
        let control = |kind| GrantFrame::new(kind, other).encode();
        assert_eq!(
            station.other_session(true, &control(aether_link::GrantKind::Header)),
            Some(Heard::Session)
        );
        assert_eq!(
            station.other_session(true, &control(aether_link::GrantKind::Answer)),
            Some(Heard::Session)
        );
        assert_eq!(
            station.other_session(true, &control(aether_link::GrantKind::Disc)),
            Some(Heard::Leaving)
        );
        assert_eq!(
            station.other_session(true, &control(aether_link::GrantKind::DiscAck)),
            Some(Heard::Over)
        );
        let own = GrantFrame::new(aether_link::GrantKind::Header, ours).encode();
        assert_eq!(
            station.other_session(true, &own),
            None,
            "this station's own session is the engine's"
        );
        let call = |src: &str, dst: &str| {
            let body = ConnectBody {
                src: src.to_owned(),
                dst: dst.to_owned(),
                caps: 0,
                version: aether_link::frames::PROTOCOL_VERSION,
                snr_db: None,
                turnaround_s: 0.0,
                identifier_s: 0.0,
            }
            .encode()
            .expect("a body");
            let header = DataHeader {
                kind: DataKind::ConnectReq,
                seq: 0,
                session: 9,
            };
            encode_data(&header, &body, 64).expect("a frame")
        };
        assert_eq!(
            station.other_session(false, &call("N4CCC", "KK4XYZ")),
            Some(Heard::Session),
            "a call between two other stations starts a session"
        );
        assert_eq!(station.other_session(false, &call("N4CCC", "W4ODA")), None);

        // and this station's own call waits it out
        for i in 0..80 {
            station.capture(&quiet_block(i)).expect("capture");
        }
        assert!(station.busy_detector().settled());
        let heard_at = station.now();
        station.reservation.heard(heard_at, Heard::Session);
        assert!(!station.channel_clear(heard_at));
        station.connect("KK4XYZ").expect("idle");
        let mut out = vec![0.0f32; 4096];
        let mut went = None;
        for i in 0..400 {
            if station.playback(&mut out).expect("playback") > 0 {
                went = Some(station.now());
                break;
            }
            station.capture(&quiet_block(1000 + i)).expect("capture");
        }
        let went = went.expect("the call never went");
        assert!(
            went - heard_at >= RESERVE_GAP_S - 0.1,
            "the call went {:.2} s after the other session's frame",
            went - heard_at
        );
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

    #[test]
    fn a_datagram_whose_pieces_stop_coming_is_given_up_on_without_waiting_for_another() {
        // Two broadcasts lost to a collision on the VarAC bench (2026-10-07) were still not
        // counted dropped three minutes later: the reassembler expired them only when the
        // next piece of anything arrived. The pass itself gives up on them now.
        let mut station = lone_station(crate::regulatory::Settings::unchecked());
        let carried = aether_link::datagram::body("KK4XYZ", 1, &[0x5A; 70]).expect("body");
        let pieces = aether_link::datagram::fragments(&carried, 7, 36).expect("fits");
        assert!(pieces.len() > 1);
        let heard_at = 10.0;
        station.heard_datagram_piece(&pieces[0], 3.5, 1, heard_at);
        station.feed_datagram(heard_at + 1.0);
        assert_eq!(station.datagram_status()["incomplete_dropped"], 0);
        station.feed_datagram(heard_at + datagrams::REASSEMBLY_TIMEOUT_S + 1.0);
        assert_eq!(station.datagram_status()["incomplete_dropped"], 1);
        let received = station.take_received_datagrams();
        assert!(received.is_empty(), "{received:?}");
    }

    /// A radio whose keying command takes `key_s` to answer, as a CAT round trip does.
    #[derive(Debug, Default)]
    struct SlowPtt {
        key_s: f64,
        keys: usize,
        releases: usize,
        /// How long the last key really took: a loaded machine sleeps longer than it is asked
        /// to (a CI mac runner slept 80 ms for 30).
        took_s: f64,
    }

    impl Ptt for SlowPtt {
        fn key(&mut self) -> Result<(), PttError> {
            let began = std::time::Instant::now();
            std::thread::sleep(std::time::Duration::from_secs_f64(self.key_s));
            self.took_s = began.elapsed().as_secs_f64();
            self.keys += 1;
            Ok(())
        }

        fn unkey(&mut self) -> Result<(), PttError> {
            self.releases += 1;
            Ok(())
        }

        fn describe(&self) -> String {
            "a slow CAT port".to_owned()
        }
    }

    fn slow_station(key_s: f64) -> Station<SlowPtt> {
        Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            SlowPtt {
                key_s,
                ..SlowPtt::default()
            },
            1,
        )
    }

    /// Queue a transmission of the engine's kind — one ordinary control frame — with its window.
    fn queue_link<P: Ptt>(
        station: &mut Station<P>,
        not_before: Option<f64>,
        not_after: Option<f64>,
    ) {
        station.pending.push_back(Outgoing::Link(LinkTx {
            frames: vec![aether_link::TxFrame {
                container: Container::Control,
                payload: vec![0x11; 7],
                mode: 0,
                rv: 0,
                floor: false,
                follows: 0,
            }],
            origin: Origin::station("test"),
            window: KeyWindow {
                not_before,
                not_after,
                ..KeyWindow::default()
            },
            identifier_s: 0.0,
            listen_first: false,
        }));
    }

    #[test]
    fn the_radios_keying_wait_is_learned_and_the_engine_told() {
        // protocol 8's first session on the air (11-step-4-first-air.md §4.1): the FTDX10's
        // `TX1;` held the run loop 19 ms at every key, and nobody counted it — the engine
        // decided its answers that much too late, and the gate judged their start that much
        // early
        let mut station = slow_station(0.03);
        let before = station.engine_latency_s();
        queue_link(&mut station, None, None);
        station.device_played(0);
        let mut out = vec![0.0f32; 480];
        assert!(station.playback(&mut out).expect("playback") > 0);
        assert!(station.transmitting());
        // what the station learned is what the radio took, to the millisecond it keeps and the
        // little the station's own timing adds around the call
        let learned = station.latency.key.typical();
        let took = station.ptt.inner().took_s;
        assert!(
            took >= 0.03 && learned >= took - 0.001 && learned <= took + 0.05,
            "learned {learned}, the radio took {took}"
        );
        assert!((station.engine_latency_s() - before - learned).abs() < 1e-9);
        assert!(
            station.events.iter().any(|e| e.starts_with("keying:")),
            "{:?}",
            station.events
        );
        assert_eq!(
            station.keying_report()["key"]["count"],
            serde_json::json!(1)
        );
    }

    #[test]
    fn a_transmission_late_in_its_slot_is_said_with_what_took_the_time() {
        // ND1J's first morning Test of 2026-10-15 (12-two-paths-80m.md §4): one answer rendered
        // in 122 ms where 60 was typical and reached the air 281 ms into its 350 ms slack, and
        // the log said only "ptt: keyed"
        let mut station = slow_station(0.03);
        let now = station.now();
        queue_link(&mut station, Some(now - 0.2), Some(now + 2.0));
        station.device_played(0);
        let mut out = vec![0.0f32; 480];
        assert!(station.playback(&mut out).expect("playback") > 0);
        let said: Vec<&String> = station
            .events
            .iter()
            .filter(|e| e.contains("after its slot opened"))
            .collect();
        assert_eq!(said.len(), 1, "{:?}", station.events);
        assert!(
            said[0].starts_with("keying:") && said[0].contains("the radio keyed in"),
            "{said:?}"
        );
        assert_eq!(station.keying_report()["late"], serde_json::json!(1));

        // keyed where its slot opens: nothing to say. The radio keys at once here — a slow one's
        // sleep is the machine's to stretch (a CI mac runner took 52 and 179 ms for 30), and
        // anything past the 50 ms line is rightly said
        let mut station = slow_station(0.0);
        let opens = station.now() + station.capture_lag_s() + station.radiation_latency_s();
        queue_link(&mut station, Some(opens), Some(opens + 0.35));
        station.device_played(0);
        assert!(station.playback(&mut out).expect("playback") > 0);
        assert!(
            !station
                .events
                .iter()
                .any(|e| e.contains("after its slot opened")),
            "{:?}",
            station.events
        );
        assert_eq!(station.keying_report()["late"], serde_json::json!(0));
    }

    #[test]
    fn a_key_that_comes_up_past_the_slack_sends_nothing() {
        // S2 by construction (11-step-4-first-air.md §12): nothing learned yet, the gate expects
        // the key at once; the radio takes 60 ms, which puts the start 30 ms past the answer's
        // slack. The key goes down again with nothing played and the engine is told it was
        // dropped — and the next such answer is refused before the key, the wait now known
        let mut station = slow_station(0.06);
        let latest = station.now() + station.radiation_latency_s() + 0.03;
        queue_link(&mut station, None, Some(latest));
        station.device_played(0);
        let mut out = vec![0.0f32; 480];
        assert_eq!(station.playback(&mut out).expect("playback"), 0);
        assert!(!station.transmitting());
        assert!(out.iter().all(|&x| x == 0.0), "something was played");
        assert_eq!(station.stats.dropped_late, 1);
        assert_eq!(
            (station.ptt.inner().keys, station.ptt.inner().releases),
            (1, 1)
        );
        let latest = station.now() + station.radiation_latency_s() + 0.03;
        queue_link(&mut station, None, Some(latest));
        assert_eq!(station.playback(&mut out).expect("playback"), 0);
        assert_eq!(station.stats.dropped_late, 2);
        assert_eq!(
            station.ptt.inner().keys,
            1,
            "keyed when the wait said it would be late"
        );
    }

    #[test]
    fn a_transmission_the_engine_keys_at_once_goes_no_sooner_than_it_reckoned() {
        // the engine is told the typical render and keying wait, and places what it grants from
        // where it reckons its transmission goes on the air; one rendered faster than typical
        // waits for the rest, so a wait learned long never puts a transmission on the air early
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        let rate = station.config.params.audio_rate as f64;
        station.render_took((0.08 * rate) as u64);
        let queued = station.now();
        station.connect("KK4XYZ").expect("idle");
        let block = 480;
        let mut played = 0u64;
        let silence = vec![0.0f32; block];
        let mut out = vec![0.0f32; block];
        let mut keyed_at = None;
        for _ in 0..100 {
            station.device_played(played);
            let handed = station.playback(&mut out).expect("playback");
            station.device_drains_at(played + handed as u64);
            if station.transmitting() {
                keyed_at = Some(station.now());
                break;
            }
            played += block as u64;
            station.capture(&silence).expect("capture");
        }
        let waited = keyed_at.expect("the call went out") - queued;
        assert!(
            (0.08 - 0.011..0.08 + 0.011).contains(&waited),
            "keyed {waited:.3} s after the engine decided"
        );
    }
}
