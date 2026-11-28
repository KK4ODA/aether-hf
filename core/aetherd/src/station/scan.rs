//! The station scans its channels for the program that asks, and never leaves one a caller may
//! be on (ADR-0129).
//!
//! A Winlink gateway scans with RMS Trimode: a program apart from the modem that tunes the radio
//! and turns the modem's `LISTEN` off and on every few seconds. It cannot see a frame arriving or
//! audio from the last channel still in the receiver; through the VARA-compatible port that cost
//! five fixes and still lost calls in its deaf half second. So the station moves the radio
//! itself, on the run loop that hears frames and keys the radio, and answering stays on:
//!
//! 1. **It moves only when the dial is free**: what [`Station::tune_to`] refuses for, a frame
//!    announced and not yet decoded or refused, an identifier owed for a setup on this dial
//!    (ADR-0055), a linger.
//! 2. **A move starts the receiver again**, at once and again once the radio's change has come
//!    through the sound card ([`move_deaf_s`]), so no frame begun on one dial is decoded, and
//!    answered, on the next. The second start has silence behind it, the longest tone frame's
//!    worth (§12): a try begun on the new dial before the station got there is read from its
//!    later sync blocks, as a receiver never started again would, with nothing of the old dial.
//! 3. **It leaves a quiet dial after a short listen, and stays on one where a call is under way**
//!    (§11, the author's design after S4): a dwell is the move's deafness and
//!    [`NOTICE_LISTEN_S`], long enough to hear one of the sync blocks of any try on the air; a
//!    try noticed partway through keeps the radio there a linger, the longest gap between two
//!    tries' starts before a call's pause ([`GrantedEngine::call_gaps`]) with the most a try can
//!    be moved past a frame heard arriving ([`GrantedEngine::call_deferral_s`]) and the move's
//!    deafness, so the caller's next try begins while it listens.
//! 4. **A frame heard holds the dial until it is reported**; an answer, a session and its
//!    identifiers hold it as they always have.
//! 5. **It lingers** after a frame announced or reported, not decoded, at [`NOTICE_CONFIDENCE`]
//!    or more, and after a probe it answered: a try under way, or a caller too weak to read this
//!    time, tries again. A frame decoded as another station's ends it. Under that confidence an
//!    announcement only holds the dial (rule 4): noise reached 1.07, and a tone arrival on noise
//!    at 1.01 had once held a dial nobody used. **What it does not read keeps it two lingers at
//!    most** (§17), counted from the first such frame of the visit: other modes' traffic looks
//!    like a try often enough to have held a busy dial the longest call each visit, and a caller
//!    on another dial went unheard. What it reads (a broadcast's piece, a probe to it, a
//!    contact) keeps it up to the longest call.
//! 6. After a session, a call or a probe it goes on from the same dial, with a linger.
//! 7. The list is judged once, at the start, as this station's answer would be.
//! 8. A dial the radio refuses is dropped, and one it has not answered for
//!    [`UNANSWERED_TO_DROP`] moves in a row; below two dials the scan ends. A move it did not
//!    answer for says nothing of the dial (§13): the receiver starts again as after a move, the
//!    scan listens a dwell where the radio is, the next reading of the dial says where that is,
//!    and then it asks for the dial again.
//! 9. **The operator wins**: another tune, or the knob, ends the scan, and its owner is told.
//! 10. **A scan belongs to its connection**: when the connection closes, the scan ends and the
//!     radio goes to the home dial once nothing holds it where it is.
//! 11. **A broadcast keeps the radio where it was heard** (§14): a piece of a datagram decoded
//!     starts a linger, so the copies a sender sends to outlast the scan are heard whole; and a
//!     datagram waiting to go out holds the dial, so a program's answer to one goes where it
//!     came from, for as long as a visit may last.
//! 12. **A datagram that names one of its dials goes out there** (§15): a program's later
//!     answer to a broadcast heard on that dial waits for the scan to get there, and the scan
//!     goes there next once its visit is over, not round the list. A dial the scan has not, or
//!     no scan, and it goes where the radio is.
//! 13. **A visit kept past its dwell says why** (§16), once, when the scan leaves the dial: how
//!     long it stayed, whether it left at the most a visit lasts, and what kept it — the frames
//!     it noticed and did not decode (announced on the floor or ordinary, reported by what the
//!     receiver read them as, their confidences, the first and the last), datagram pieces, a
//!     probe to this station, a contact, and anything else that held the dial. On a busy band a
//!     dial was kept 184–214 s at a time, and the log could not say on what (report 20 §3).
//!
//! [`GrantedEngine::call_gaps`]: aether_link::GrantedEngine::call_gaps
//! [`GrantedEngine::call_deferral_s`]: aether_link::GrantedEngine::call_deferral_s

use serde_json::{Value, json};

use super::{FrameReport, RegulatoryQuery, State, Station};
use crate::control::protocol::ConnectionId;
use crate::ptt::{Ptt, PttError};
use crate::regulatory::Direction;

/// The fewest dials a scan takes: one is a dial, not a scan.
pub const MIN_SCAN_DIALS: usize = 2;
/// The most dials a scan takes, the author's cap (ADR-0129 §11): up to four a scanning gateway
/// reads every call on a good path and, in the timing model fed with the detector's measured
/// notices, more than four in five at the weak path S4 is set at; at five the 500 Hz weak path
/// falls to 76 % (`docs/research/scanning/02-sense-and-stay.md`). Three-quarters of the Winlink
/// HF gateways listed on 2026-11-01 publish four dials or fewer.
pub const MAX_SCAN_DIALS: usize = 4;
/// How long the scan listens on a dial, after the move's deafness, before it leaves one where it
/// noticed nothing: the longest two of a try's markers stand apart (its tone frame's middle and
/// end sync blocks, 2.4 s), a block's length, and its announcement after it. Measured: listening
/// from any point of a try but its last second noticed it within this, on good paths at either
/// air (`docs/research/scanning/02-sense-and-stay.md` §1).
pub const NOTICE_LISTEN_S: f64 = 3.5;
/// The most each dwell is drawn longer than the move and [`NOTICE_LISTEN_S`]: a uniform part, so
/// a round of the dials never keeps in step with a caller whose tries come once a round. Three
/// dials at four seconds are a 12 s round against tries 11.4–12.4 s apart; the harness's grid
/// found a phase at which every visit fell in the quiet between tries, on both airs, and the
/// call was never heard. A second's jitter reads every call at every phase on good paths in the
/// timing model; two or three cost the weak path more than they buy
/// (`docs/research/scanning/02-sense-and-stay.md` §4).
pub const DWELL_JITTER_S: f64 = 1.0;
/// How far over its gate an arrival announced, or a frame reported that did not decode, must
/// stand for the scan to take it as a try under way and linger. Twenty minutes of noise reached
/// 1.07; the first notice of a try heard partway through stood at 1.1 or more in four of five at
/// −16 dB, its median 1.58 (`02-sense-and-stay.md` §1).
pub const NOTICE_CONFIDENCE: f64 = 1.1;
/// The radio's own settling after it has taken a new dial, before what it delivers is the new
/// channel's: its synthesiser and its AGC. The ADR's timing model assumed half a second for the
/// whole move with the card's input latency; measured at the radio in its step 2.
pub const MOVE_SETTLE_S: f64 = 0.25;
/// How long after a frame's end it is decoded or refused: the floor's frame is final 18 symbols
/// (0.72 s) after it ends (ADR-0009 §8), and a pass of the run loop on top.
pub const REPORT_AFTER_END_S: f64 = 1.0;
/// How many moves in a row to one dial the radio may leave unanswered before the scan leaves the
/// dial out (ADR-0129 §13). An FTDX10 just out of transmit missed the read-back of a move asked
/// 5 ms after its release, and the scan, which dropped a dial on any error, ended for the day
/// (report 18 §3). The moves are a dwell apart at least: three in a row is a radio that does not
/// go there, or does not answer at all.
pub const UNANSWERED_TO_DROP: u32 = 3;
/// How long past its dwell a visit must last for the log to say what kept the radio there
/// (rule 13): a frame arriving under [`NOTICE_CONFIDENCE`] holds the dial until it is reported,
/// a second or so, and that is not worth a line.
pub const STAY_SAID_S: f64 = 1.0;
/// How many of the things that held a dial a stay's line names, and how many of the receiver's
/// readings of the frames it did not decode.
const STAY_NAMES: usize = 4;
/// How far a dial read back may differ from the dial asked for before it is taken for the knob:
/// a radio that rounds its dial to its tuning step.
pub(super) const KNOB_TOLERANCE_HZ: u64 = 50;

/// How long a caller's tries must keep starting to be read by a scan of `dials` dials (the
/// `scan_s` a program publishes): a round of them, and two lingers, so a try noticed partway
/// through is followed by two the scan stays for.
#[must_use]
pub fn must_last(dials: usize, dwell_s: f64, linger_s: f64) -> f64 {
    (dials as f64 * dwell_s + 2.0 * linger_s).ceil()
}

/// The next draw in 0..1 from a `SplitMix64` state.
fn next_unit(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// How long a move leaves the station deaf: the card's input latency, and the radio's settling.
#[must_use]
pub fn move_deaf_s() -> f64 {
    crate::audio::DEVICE_LATENCY_S + MOVE_SETTLE_S
}

/// Why a scan ended: the `scan` event's `why`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanEnd {
    /// Its owner asked (`scan.stop`, or a new scan in place of it).
    Asked,
    /// Its owner's connection closed.
    Left,
    /// Another program, or the panel, tuned the radio.
    Tuned,
    /// The radio's own knob moved the dial.
    Knob,
    /// The operator stopped it at the panel.
    Operator,
    /// Fewer than two of its dials are left that the radio will take.
    Radio,
}

impl ScanEnd {
    /// The word the `scan` event carries.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Asked => "asked",
            Self::Left => "connection closed",
            Self::Tuned => "tuned by another program",
            Self::Knob => "knob",
            Self::Operator => "operator",
            Self::Radio => "radio",
        }
    }

    /// Whether the radio goes back to the home dial: not when somebody put it where it is.
    const fn goes_home(self) -> bool {
        matches!(self, Self::Asked | Self::Left | Self::Radio)
    }
}

/// Why `scan.start` was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanRefusal {
    /// The keying interface cannot set the dial: a serial line, a CM108 codec, none.
    CannotTune,
    /// The list itself: too few or too many dials, a zero, or one given twice.
    BadDials(String),
    /// Another program's scan runs: its name.
    Claimed(String),
    /// A dial where the rules would refuse this station's answer, and why.
    Rules {
        /// The dial, hertz.
        dial_hz: u64,
        /// The rules' one line.
        why: String,
    },
}

impl ScanRefusal {
    /// The control API's error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::CannotTune => "unsupported",
            Self::BadDials(_) => "bad_params",
            Self::Claimed(_) => "claimed",
            Self::Rules { .. } => "regulatory",
        }
    }

    /// For the operator, in a sentence.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::CannotTune => "Cannot scan: the keying interface has no way to set the radio's \
                                 dial (CAT, rigctld or FLRig can)."
                .to_owned(),
            Self::BadDials(why) => format!("Cannot scan: {why}."),
            Self::Claimed(app) => format!("Cannot scan: {app} is scanning this station."),
            Self::Rules { dial_hz, why } => {
                format!("Cannot scan {:.4} MHz: {why}", *dial_hz as f64 / 1e6)
            }
        }
    }
}

/// A scan under way.
#[derive(Debug, Clone)]
pub(super) struct Scan {
    owner: ConnectionId,
    app: String,
    /// The dials, the first the home dial.
    dials: Vec<u64>,
    /// The dial the radio is on, or going to, as an index into `dials`.
    at: usize,
    /// Whether the radio has been put on `dials[at]` yet.
    placed: bool,
    dwell_s: f64,
    linger_s: f64,
    /// The draws that lengthen each dwell by up to [`DWELL_JITTER_S`] (`SplitMix64`, seeded from
    /// the scan's owner and dials, so a replay of the same scan draws the same).
    jitter: u64,
    /// The longest a visit lasts however much what it reads keeps it: the longest call (fifteen
    /// tries).
    cap_s: f64,
    /// How long a call's tries must keep starting to be read: a round of the dials, and two
    /// lingers, so a try noticed is followed by two the scan stays for.
    must_last_s: f64,
    /// When the radio was put on this dial, and when its audio is the new channel's.
    moved_at: f64,
    hear_from: f64,
    /// The receiver starts again here: the radio's change has come through the card.
    restart_at: Option<f64>,
    /// When the visit's dwell ends.
    leave_at: f64,
    /// Until when it lingers.
    linger_until: Option<f64>,
    /// Until when a frame announced may still be decoded or refused.
    arriving_until: f64,
    /// The visit's first frame announced, or reported undecoded at [`NOTICE_CONFIDENCE`] or
    /// more, since the move or the last contact: what it does not read keeps the radio two
    /// lingers from here at most (§17).
    guessed_from: Option<f64>,
    /// What held it at the last pass was a contact: a session, a call, a probe or a Test.
    held_by_contact: bool,
    hops: u64,
    /// Dials the radio would not take, and why.
    dropped: Vec<(u64, String)>,
    /// Moves in a row to each dial, by its place in `dials`, that the radio did not answer.
    unanswered: Vec<u32>,
    /// The dial the last move asked for, by its place in `dials`, when the radio did not answer
    /// it: the radio may be there rather than on `dials[at]` until a reading says (§13).
    or_at: Option<usize>,
    /// What has kept the radio on this visit's dial, for the line the log gets when it leaves
    /// (rule 13).
    stay: Stay,
    since_ms: u64,
}

/// What cut a linger short when the scan left a dial: the most a visit lasts (`cap_s`), or two
/// lingers after the visit's first frame it did not read (§17), in seconds.
#[derive(Debug, Clone, Copy)]
enum Ended {
    Cap(f64),
    Unread(f64),
}

/// What kept the radio on one visit's dial (rule 13).
#[derive(Debug, Clone, Default)]
struct Stay {
    /// When the visit's dwell, drawn at the move, ended, on the station's clock.
    dwell_end: f64,
    /// Frames announced at [`NOTICE_CONFIDENCE`] or more: on the floor, and ordinary.
    announced_floor: u32,
    announced_ordinary: u32,
    /// Frames reported and not decoded at [`NOTICE_CONFIDENCE`] or more, by what the receiver
    /// read them as, in the order first met.
    undecoded: Vec<(String, u32)>,
    /// The least and the most confidence among the frames announced or reported.
    confidence: Option<(f64, f64)>,
    /// When the first and the last of them came, in seconds after the move.
    first_s: f64,
    last_s: f64,
    /// Pieces of a datagram decoded (rule 11).
    datagram_pieces: u32,
    /// Probes to this station decoded.
    probes: u32,
    /// A session, a call, a probe or a Test held the dial.
    contact: bool,
    /// Anything else that held the dial, each once.
    held: Vec<String>,
}

impl Stay {
    fn starting(dwell_end: f64) -> Self {
        Self {
            dwell_end,
            ..Self::default()
        }
    }

    /// A frame noticed `at` seconds into the visit, at `confidence`.
    fn noticed(&mut self, at: f64, confidence: f64) {
        self.confidence = Some(
            self.confidence
                .map_or((confidence, confidence), |(lo, hi)| {
                    (lo.min(confidence), hi.max(confidence))
                }),
        );
        if self.announced_floor + self.announced_ordinary == 0 && self.undecoded.is_empty() {
            self.first_s = at;
        }
        self.last_s = self.last_s.max(at);
    }

    fn undecoded(&mut self, read_as: String, at: f64, confidence: f64) {
        self.noticed(at, confidence);
        match self.undecoded.iter_mut().find(|(what, _)| *what == read_as) {
            Some((_, n)) => *n += 1,
            None => self.undecoded.push((read_as, 1)),
        }
    }

    fn held_by(&mut self, why: String) {
        if self.held.len() < STAY_NAMES && !self.held.contains(&why) {
            self.held.push(why);
        }
    }

    /// The log's line for a visit of `visit_s` on `dial` that lasted `past_s` past its dwell, or
    /// `None` when that is under [`STAY_SAID_S`]. `ended`: what cut a linger short when it left,
    /// the most a visit lasts or two lingers of frames it did not read (§17).
    fn said(&self, dial: u64, visit_s: f64, past_s: f64, ended: Option<Ended>) -> Option<String> {
        if past_s < STAY_SAID_S {
            return None;
        }
        let left = match ended {
            None => String::new(),
            Some(Ended::Cap(cap_s)) => {
                format!(", and left at the most a visit lasts ({cap_s:.0} s)")
            }
            Some(Ended::Unread(bound_s)) => format!(
                ", and left two lingers after the first frame it did not read ({bound_s:.0} s)"
            ),
        };
        let mut line = format!(
            "stayed {visit_s:.1} s on {}, {past_s:.1} s past its dwell{left}",
            mhz(dial)
        );
        let mut why = Vec::new();
        let announced = self.announced_floor + self.announced_ordinary;
        let reported: u32 = self.undecoded.iter().map(|(_, n)| n).sum();
        if announced + reported > 0 {
            let mut parts = Vec::new();
            if announced > 0 {
                parts.push(format!(
                    "{announced} announced ({} on the floor, {} ordinary)",
                    self.announced_floor, self.announced_ordinary
                ));
            }
            if reported > 0 {
                let mut read: Vec<_> = self.undecoded.iter().collect();
                read.sort_by_key(|entry| std::cmp::Reverse(entry.1));
                let read: Vec<String> = read
                    .iter()
                    .take(STAY_NAMES)
                    .map(|(what, n)| format!("{what} ×{n}"))
                    .collect();
                parts.push(format!(
                    "{reported} reported and not decoded ({})",
                    read.join(", ")
                ));
            }
            let (lo, hi) = self.confidence.unwrap_or_default();
            why.push(format!(
                "frames at confidence {NOTICE_CONFIDENCE} or more, {}, at {lo:.2}–{hi:.2}, \
                 the first {:.1} s in and the last {:.1} s in",
                parts.join(" and "),
                self.first_s,
                self.last_s
            ));
        }
        if self.datagram_pieces > 0 {
            why.push(format!("{} datagram pieces", self.datagram_pieces));
        }
        if self.probes > 0 {
            why.push(format!("{} probes to this station", self.probes));
        }
        if self.contact {
            why.push("a session, a call or a probe on this dial".to_owned());
        }
        if !self.held.is_empty() {
            why.push(format!("held: {}", self.held.join("; ")));
        }
        if why.is_empty() {
            why.push("nothing it noted".to_owned());
        }
        line.push_str(": ");
        line.push_str(&why.join("; "));
        Some(line)
    }
}

impl Scan {
    /// The most of `until` that a frame heard and not read may keep the radio (§17): two lingers
    /// from the visit's first such frame, which this one is when none came before.
    fn unread_until(&mut self, now: f64, until: f64) -> f64 {
        let first = *self.guessed_from.get_or_insert(now);
        until.min(first + 2.0 * self.linger_s)
    }
}

/// The station's scan, the radio's way home after one, and how the last one ended.
#[derive(Debug, Default)]
pub(super) struct Scanner {
    scan: Option<Scan>,
    /// A dial to go back to once nothing holds the radio where it is.
    home: Option<u64>,
    /// Why the last scan ended, until another starts.
    ended: Option<ScanEnd>,
    /// The receiver was started again after a move since the run loop last asked: that pass
    /// fed it the longest tone frame of silence (rule 2), about 0.4 s of work on a 4-core mini
    /// PC, and is not the modem falling behind (report 19 §8).
    restarted: bool,
}

impl<P: Ptt> Station<P> {
    /// Start a scan of `dials` for `owner`, or replace its own (ADR-0129). The answer is
    /// [`Self::scan_status`].
    ///
    /// # Errors
    /// [`ScanRefusal`]: a radio that cannot be tuned, a list out of bounds, another program's
    /// scan, or a dial where the rules would refuse this station's answer.
    pub fn scan_start(&mut self, owner: ConnectionId, dials: &[u64]) -> Result<Value, ScanRefusal> {
        if !self.ptt.can_tune() {
            return Err(ScanRefusal::CannotTune);
        }
        if dials.len() < MIN_SCAN_DIALS || dials.len() > MAX_SCAN_DIALS {
            return Err(ScanRefusal::BadDials(format!(
                "a scan takes {MIN_SCAN_DIALS} to {MAX_SCAN_DIALS} dials, not {}",
                dials.len()
            )));
        }
        if dials.contains(&0) {
            return Err(ScanRefusal::BadDials("a dial of 0 Hz".to_owned()));
        }
        for (i, dial) in dials.iter().enumerate() {
            if dials[..i].contains(dial) {
                return Err(ScanRefusal::BadDials(format!("{dial} Hz is given twice")));
            }
        }
        if let Some(scan) = &self.scanner.scan
            && scan.owner != owner
        {
            return Err(ScanRefusal::Claimed(scan.app.clone()));
        }
        for &dial_hz in dials {
            let check = self.regulatory_check(&RegulatoryQuery {
                dial_hz: Some(dial_hz as f64),
                direction: Some(Direction::Respond),
                ..RegulatoryQuery::default()
            });
            if check["decision"]["verdict"] == "blocked" {
                let why = check["decision"]["summary"]
                    .as_str()
                    .unwrap_or("the rules refuse an answer there")
                    .to_owned();
                return Err(ScanRefusal::Rules { dial_hz, why });
            }
        }
        // a quiet dial is left after a listen; a try under way keeps the radio until the
        // caller's next one has begun: the gap its engine leaves before it, a try moved past a
        // frame heard arriving (a tone arrival on noise is heeded), and the move
        let (before, _) = self.engine.call_gaps();
        let dwell_s = move_deaf_s() + NOTICE_LISTEN_S;
        let linger_s = before + self.engine.call_deferral_s() + move_deaf_s();
        let longest_call = self
            .engine
            .call_offsets(&[b'H'; aether_link::MAX_CALL_TRIES])
            .last()
            .copied()
            .unwrap_or(0.0);
        let now = self.now();
        let app = self
            .program_name(owner)
            .unwrap_or_else(|| "a program".to_owned());
        // the radio stays where it is when that is one of the dials: the scan starts there
        let on = self.frequency.value.and_then(|hz| {
            dials
                .iter()
                .position(|&d| d.abs_diff(hz) <= KNOB_TOLERANCE_HZ)
        });
        self.scanner.scan = Some(Scan {
            owner,
            app: app.clone(),
            dials: dials.to_vec(),
            at: on.unwrap_or(0),
            placed: on.is_some(),
            dwell_s,
            linger_s,
            jitter: dials
                .iter()
                .fold(0x5CA9_u64 ^ owner.id, |h, &d| h.rotate_left(13) ^ d),
            cap_s: longest_call + dwell_s,
            must_last_s: must_last(dials.len(), dwell_s + DWELL_JITTER_S, linger_s),
            moved_at: now,
            hear_from: now,
            restart_at: None,
            // on none of the dials, it moves at once
            leave_at: now + if on.is_some() { dwell_s } else { 0.0 },
            linger_until: None,
            arriving_until: f64::NEG_INFINITY,
            guessed_from: None,
            held_by_contact: false,
            hops: 0,
            dropped: Vec::new(),
            unanswered: vec![0; dials.len()],
            or_at: None,
            stay: Stay::starting(now + if on.is_some() { dwell_s } else { 0.0 }),
            since_ms: super::beacons::unix_ms(),
        });
        self.scanner.home = None;
        self.scanner.ended = None;
        let list: Vec<String> = dials.iter().map(|&d| mhz(d)).collect();
        self.note(
            "scan",
            &format!(
                "scanning {} for {app}: {:.1}–{:.1} s on each, {:.1} s more where a call is under \
                 way, a call must keep trying {:.0} s",
                list.join(", "),
                dwell_s,
                dwell_s + DWELL_JITTER_S,
                linger_s,
                must_last(dials.len(), dwell_s + DWELL_JITTER_S, linger_s)
            ),
        );
        Ok(self.scan_status())
    }

    /// Stop a scan for `connection`: whether one stopped. Its owner's stop sends the radio to
    /// the home dial once nothing holds it where it is; a connection that holds no claim — the
    /// panel, which acts on any program's call or session (ADR-0106) — stops any scan, as the
    /// operator, and the radio stays where it is (rule 9).
    pub fn scan_stop(&mut self, connection: ConnectionId) -> bool {
        let Some(scan) = &self.scanner.scan else {
            return false;
        };
        if scan.owner == connection {
            self.end_scan(ScanEnd::Asked);
            true
        } else if self.claim_of(connection).is_none() {
            self.end_scan_staying(ScanEnd::Operator);
            true
        } else {
            false
        }
    }

    /// The scan under way, for `status` and the `scan` event; null when none runs.
    #[must_use]
    pub fn scan_status(&self) -> Value {
        let Some(scan) = &self.scanner.scan else {
            return Value::Null;
        };
        let now = self.now();
        let at_ms = |t: f64| {
            let ahead = ((t - now) * 1000.0).round() as i64;
            super::beacons::unix_ms().saturating_add_signed(ahead)
        };
        json!({
            "owner": scan.owner.id,
            "app": scan.app,
            "dials": scan.dials,
            "at_hz": scan.dials.get(scan.at),
            "dwell_s": round3(scan.dwell_s),
            "dwell_jitter_s": DWELL_JITTER_S,
            "linger_s": round3(scan.linger_s),
            "scan_s": scan.must_last_s,
            "held": self.scan_held(now),
            "lingering_until_ms": scan.linger_until.filter(|&t| t > now).map(at_ms),
            "hops": scan.hops,
            "dropped": scan.dropped.iter().map(|(hz, why)| json!({ "hz": hz, "why": why }))
                .collect::<Vec<_>>(),
            "since_ms": scan.since_ms,
        })
    }

    /// The `scan` event: the scan as it stands, or null and why it ended.
    #[must_use]
    pub fn scan_event(&self) -> Value {
        json!({
            "scan": self.scan_status(),
            "why": self.scanner.ended.map(ScanEnd::word),
        })
    }

    /// Whether a scan runs.
    #[must_use]
    pub fn scanning(&self) -> bool {
        self.scanner.scan.is_some()
    }

    /// Tune the radio for `connection` (`frequency.set`): the operator wins over a scan
    /// (ADR-0129 rule 9), so a tune that goes ends any scan, and its owner is told.
    ///
    /// # Errors
    /// As [`Self::tune_to`].
    pub fn tune_for(&mut self, connection: Option<ConnectionId>, hz: u64) -> Result<(), String> {
        self.tune_to(hz)?;
        if let Some(scan) = &self.scanner.scan {
            let why = if Some(scan.owner) == connection {
                ScanEnd::Asked
            } else {
                ScanEnd::Tuned
            };
            self.end_scan_staying(why);
        }
        self.scanner.home = None;
        Ok(())
    }

    /// A connection has closed: its scan ends (rule 10).
    pub(super) fn scan_connection_closed(&mut self, connection: ConnectionId) {
        if self
            .scanner
            .scan
            .as_ref()
            .is_some_and(|s| s.owner == connection)
        {
            self.end_scan(ScanEnd::Left);
        }
    }

    /// A frame announced past the gate the station heeds (rule 4): the dial is held until it
    /// can have been decoded or refused. At [`NOTICE_CONFIDENCE`] or more it is a try under way,
    /// heard from its start or partway through it, and the scan lingers for the caller's next
    /// (rule 5). Under it the announcement only holds: a tone arrival is heeded whatever its
    /// confidence, and the harness's first grid found one on noise, at 1.01. Either way it is a
    /// frame not read yet, which keeps the radio two lingers from the visit's first at most
    /// (§17): a busy band's other modes announce one after another. `floor`: announced in the
    /// air's robust family, as the log's line for a long visit says (rule 13).
    pub(super) fn scan_arriving(&mut self, end_s: f64, confidence: f64, floor: bool, now: f64) {
        if let Some(scan) = &mut self.scanner.scan {
            let mut held = end_s + REPORT_AFTER_END_S;
            if now >= scan.hear_from {
                held = scan.unread_until(now, held);
            }
            scan.arriving_until = scan.arriving_until.max(held);
            if confidence >= NOTICE_CONFIDENCE && now >= scan.hear_from {
                let until = scan.unread_until(now, now + scan.linger_s);
                scan.linger_until = Some(scan.linger_until.map_or(until, |t| t.max(until)));
                scan.stay.noticed(now - scan.moved_at, confidence);
                if floor {
                    scan.stay.announced_floor += 1;
                } else {
                    scan.stay.announced_ordinary += 1;
                }
            }
        }
    }

    /// A frame reported (rule 5): one that came to nothing at [`NOTICE_CONFIDENCE`] or more, a
    /// probe this station answers, or a piece of a datagram (rule 11) keeps it a linger longer
    /// (the first within two lingers of the visit's first frame not read, §17); one decoded as
    /// another station's ends the linger. The noise candidates a quiet band brings fall under
    /// the gate.
    pub(super) fn scan_heard(&mut self, report: &FrameReport, now: f64) {
        let ours = report
            .to
            .as_ref()
            .is_some_and(|to| self.engine.callsigns.contains(to));
        let Some(scan) = &mut self.scanner.scan else {
            return;
        };
        if now < scan.hear_from {
            return;
        }
        let idle = self.engine.state() == State::Idle;
        // a piece of a broadcast: the rest of it, and the copies its sender sends to outlast a
        // scan, come on this dial, and the program's answer goes out here (rule 11)
        let datagram = report.decoded && report.kind == "datagram";
        let anothers = report.decoded
            && match report.kind {
                "beacon" => true,
                "connect" | "probe" => !ours,
                "datagram" => false,
                // another session's frames, heard by a station in none
                _ => idle,
            };
        let weak_caller = !report.decoded && report.detect_confidence >= NOTICE_CONFIDENCE;
        let probe = report.decoded && report.kind == "probe" && ours;
        if anothers {
            scan.linger_until = None;
        } else if weak_caller || datagram || probe {
            let mut until = now + scan.linger_s;
            if weak_caller {
                // heard and not read: two lingers from the visit's first such frame at most (§17)
                until = scan.unread_until(now, until);
            }
            scan.linger_until = Some(scan.linger_until.map_or(until, |t| t.max(until)));
        }
        // what the log's line for a long visit names (rule 13)
        let at = now - scan.moved_at;
        if weak_caller {
            let read_as = if report.kind == "data" {
                format!("data at rung {}", report.mode)
            } else {
                report.kind.to_owned()
            };
            scan.stay.undecoded(read_as, at, report.detect_confidence);
        } else if datagram {
            scan.stay.datagram_pieces += 1;
        } else if probe {
            scan.stay.probes += 1;
        }
    }

    /// Called every pass: move the radio when the visit is over and nothing holds the dial.
    pub(super) fn advance_scan(&mut self, now: f64) {
        if self.scanner.scan.is_none() {
            self.go_home();
            return;
        }
        self.restart_after_move(now);
        self.follow_unanswered();
        if self.knob_turned(now) {
            self.end_scan_staying(ScanEnd::Knob);
            return;
        }
        let held = self.scan_held(now);
        let contact = self.held_by_contact();
        let wanted = self
            .datagram_dial_index()
            .or_else(|| self.beacon_dial_index());
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        if let Some(why) = held {
            scan.held_by_contact |= contact;
            scan.leave_at = scan.leave_at.max(now);
            if contact {
                scan.stay.contact = true;
            } else if now > scan.stay.dwell_end {
                scan.stay.held_by(why);
            }
            return;
        }
        if scan.held_by_contact {
            // a session, a call or a probe has ended here: its station may call again
            scan.held_by_contact = false;
            scan.stay.contact = true;
            scan.linger_until = Some(now + scan.linger_s);
            scan.guessed_from = None;
            scan.leave_at = scan.leave_at.max(now);
            return;
        }
        if !scan.placed {
            // after a move the radio did not answer for, a dwell first
            if now < scan.leave_at {
                return;
            }
            let at = scan.at;
            self.move_scan(at, now);
            return;
        }
        let lingering = scan
            .linger_until
            .is_some_and(|until| now < until && now - scan.moved_at < scan.cap_s);
        if lingering || now < scan.leave_at {
            return;
        }
        // a datagram that belongs on another dial takes the scan there next (rule 12)
        let next = wanted.unwrap_or((scan.at + 1) % scan.dials.len());
        self.move_scan(next, now);
    }

    /// What keeps the radio on its dial now, in a sentence, or `None` (rules 1, 4 and 11).
    fn scan_held(&self, now: f64) -> Option<String> {
        if let Err(why) = self.dial_movable() {
            return Some(why);
        }
        let scan = self.scanner.scan.as_ref()?;
        if now < scan.arriving_until {
            return Some("a frame is arriving".to_owned());
        }
        if self.identifier.setup_at.is_some() || self.closing_identifier_pending() {
            return Some("the identifier is owed on this dial".to_owned());
        }
        // a program's datagram may answer one heard here: it goes out here, for as long as a
        // visit may last; past that it goes with the radio, as a tune would take it (rule 11)
        if self.datagram_waiting() && now - scan.moved_at < scan.cap_s {
            return Some("a datagram is waiting to go out on this dial".to_owned());
        }
        // and a program's beacon asked for this dial (ADR-0139)
        if self.beacon_waiting_here() && now - scan.moved_at < scan.cap_s {
            return Some("a beacon is waiting to go out on this dial".to_owned());
        }
        None
    }

    /// Whether a datagram is waiting to go out on the dial the radio is on: handed to the
    /// transmitter's queue, or first in its own and not waiting for another of the scan's dials
    /// (the queue goes in order, so nothing behind that one can go first).
    fn datagram_waiting(&self) -> bool {
        self.pending
            .iter()
            .any(|out| matches!(out, super::Outgoing::Datagram { .. }))
            || self
                .datagrams
                .queue
                .front()
                .is_some_and(|front| !self.scan_goes_elsewhere_for(front.dial_hz))
    }

    /// Whether a datagram that belongs on `dial` must wait for the scan to get there: a scan
    /// runs, the dial is one of its own, and the radio is not on it (rule 12).
    pub(super) fn scan_goes_elsewhere_for(&self, dial: Option<u64>) -> bool {
        let (Some(scan), Some(dial)) = (self.scanner.scan.as_ref(), dial) else {
            return false;
        };
        let Some(index) = scan
            .dials
            .iter()
            .position(|&d| d.abs_diff(dial) <= KNOB_TOLERANCE_HZ)
        else {
            return false;
        };
        !(scan.placed && scan.at == index)
    }

    /// Which of a running scan's dials `dial` is, if it is one.
    pub(super) fn scan_index_of(&self, dial: u64) -> Option<usize> {
        self.scanner
            .scan
            .as_ref()?
            .dials
            .iter()
            .position(|&d| d.abs_diff(dial) <= KNOB_TOLERANCE_HZ)
    }

    /// The scan's dial the first datagram waiting belongs on, when it is another than the one
    /// the radio is on: where the scan goes next (rule 12).
    fn datagram_dial_index(&self) -> Option<usize> {
        let front = self.datagrams.queue.front()?;
        if !self.scan_goes_elsewhere_for(front.dial_hz) {
            return None;
        }
        let dial = front.dial_hz?;
        self.scanner
            .scan
            .as_ref()?
            .dials
            .iter()
            .position(|&d| d.abs_diff(dial) <= KNOB_TOLERANCE_HZ)
    }

    /// Whether what holds the dial is a contact this station may hear from again.
    fn held_by_contact(&self) -> bool {
        self.engine.state() != State::Idle || self.probing() || self.test_running()
    }

    /// The scan leaves the dial it is on: when the visit lasted past its dwell, the log says what
    /// kept it (rule 13). Once a visit: the record starts again at the next move.
    fn say_stay(&mut self, now: f64) {
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        let stay = std::mem::replace(&mut scan.stay, Stay::starting(f64::INFINITY));
        if !scan.placed {
            return;
        }
        let Some(&dial) = scan.dials.get(scan.at) else {
            return;
        };
        // a linger still running when the visit ends is one the cap cut short; one that ran to
        // two lingers after the first frame not read, because a later one asked for more, is one
        // §17's bound cut short
        let bound = scan.guessed_from.map(|first| first + 2.0 * scan.linger_s);
        let ended = if scan.linger_until.is_some_and(|until| now < until) {
            Some(Ended::Cap(scan.cap_s))
        } else {
            bound
                .filter(|&b| {
                    scan.linger_until
                        .is_some_and(|until| (until - b).abs() < 1e-6)
                })
                .map(|_| Ended::Unread(2.0 * scan.linger_s))
        };
        if let Some(line) = stay.said(dial, now - scan.moved_at, now - stay.dwell_end, ended) {
            self.note("scan", &line);
        }
    }

    /// Put the radio on the scan's dial `index` (rules 2 and 8).
    fn move_scan(&mut self, index: usize, now: f64) {
        self.say_stay(now);
        let Some(scan) = self.scanner.scan.as_ref() else {
            return;
        };
        let Some(&dial) = scan.dials.get(index) else {
            return;
        };
        if let Err(error) = self.ptt.inner_mut().set_frequency_hz(dial) {
            self.move_failed(index, dial, &error, now);
            return;
        }
        let deaf = move_deaf_s();
        // the receiver keeps nothing heard on the dial the radio has left: a call whose end it
        // was still settling would be answered on this one
        self.restart_receiver();
        self.read_dial(dial, now);
        self.frequency.next_ask_s = now + 2.0;
        self.refresh_ceiling(true);
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        scan.at = index;
        scan.placed = true;
        scan.or_at = None;
        if let Some(misses) = scan.unanswered.get_mut(index) {
            *misses = 0;
        }
        scan.moved_at = now;
        scan.hear_from = now + deaf;
        scan.restart_at = Some(now + deaf);
        scan.leave_at = now + scan.dwell_s + DWELL_JITTER_S * next_unit(&mut scan.jitter);
        scan.stay = Stay::starting(scan.leave_at);
        scan.linger_until = None;
        scan.arriving_until = f64::NEG_INFINITY;
        scan.guessed_from = None;
        scan.hops += 1;
        self.record_event("hop", &format!("dial {dial} deaf {deaf:.3}"));
    }

    /// A move to the scan's dial `index` failed (rule 8, §13). A refusal drops the dial at once;
    /// no answer keeps it, until the radio has left [`UNANSWERED_TO_DROP`] moves to it in a row
    /// unanswered.
    fn move_failed(&mut self, index: usize, dial: u64, error: &PttError, now: f64) {
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        let misses = match error {
            PttError::Refused(_) => None,
            PttError::Backend(_) | PttError::WatchdogTripped => {
                scan.unanswered.get_mut(index).map(|misses| {
                    *misses += 1;
                    *misses
                })
            }
        };
        match misses {
            Some(misses) if misses < UNANSWERED_TO_DROP => {
                self.move_unanswered(index, dial, misses, error, now);
            }
            _ => self.drop_dial(index, dial, misses, error),
        }
    }

    /// Leave the scan's dial `index` out: the radio refused it, or has not answered `misses`
    /// moves to it in a row. Below [`MIN_SCAN_DIALS`] the scan ends.
    fn drop_dial(&mut self, index: usize, dial: u64, misses: Option<u32>, error: &PttError) {
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        let (why, said) = match misses {
            Some(misses) => (
                format!("no answer {misses} times in a row ({error})"),
                format!(
                    "the radio did not answer at {} {misses} times in a row ({error})",
                    mhz(dial)
                ),
            ),
            None => (
                error.to_string(),
                format!("the radio would not go to {}: {error}", mhz(dial)),
            ),
        };
        scan.dials.remove(index);
        scan.unanswered.remove(index);
        scan.or_at = None;
        scan.dropped.push((dial, why));
        let left = scan.dials.len();
        if left < MIN_SCAN_DIALS {
            self.note("scan", &said);
            self.end_scan(ScanEnd::Radio);
            return;
        }
        scan.at = index % left;
        scan.placed = false;
        scan.must_last_s = must_last(left, scan.dwell_s + DWELL_JITTER_S, scan.linger_s);
        self.note("scan", &format!("{said}; the scan leaves it out"));
    }

    /// A move the radio did not answer for, the dial kept (§13). The radio may have gone there
    /// or not: the receiver starts again as after a move (rule 2), the scan stays where it was a
    /// dwell, and the next reading of the dial says where the radio is
    /// ([`Self::follow_unanswered`]). Then the scan asks for the dial again.
    fn move_unanswered(
        &mut self,
        index: usize,
        dial: u64,
        misses: u32,
        error: &PttError,
        now: f64,
    ) {
        let deaf = move_deaf_s();
        self.restart_receiver();
        self.note(
            "scan",
            &format!(
                "the radio did not answer at {}; the scan tries it again after a dwell (miss \
                 {misses} of {UNANSWERED_TO_DROP}: {error})",
                mhz(dial)
            ),
        );
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        scan.or_at = (scan.placed && scan.at != index).then_some(index);
        scan.moved_at = now;
        scan.hear_from = now + deaf;
        scan.restart_at = Some(now + deaf);
        scan.leave_at = now + scan.dwell_s + DWELL_JITTER_S * next_unit(&mut scan.jitter);
        scan.stay = Stay::starting(scan.leave_at);
        scan.linger_until = None;
        scan.arriving_until = f64::NEG_INFINITY;
        scan.guessed_from = None;
        self.record_event("hop", &format!("dial {dial} deaf {deaf:.3} unanswered"));
    }

    /// A reading of the dial taken after a move the radio did not answer for says where the
    /// scan is (§13): on the dial asked for, the move went through after all; on the one it was
    /// on, it did not, and the scan asks again after its dwell. Neither is the knob; any other
    /// dial still is (rule 9).
    fn follow_unanswered(&mut self) {
        let (Some(hz), Some(read_at)) = (self.frequency.value, self.frequency.read_at_s) else {
            return;
        };
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        if read_at <= scan.moved_at + 1e-6 {
            return;
        }
        let on = |i: usize| {
            scan.dials
                .get(i)
                .is_some_and(|&d| d.abs_diff(hz) <= KNOB_TOLERANCE_HZ)
        };
        let went = if scan.placed {
            scan.or_at.filter(|&asked| on(asked))
        } else {
            Some(scan.at).filter(|&at| on(at))
        };
        let Some(went) = went else {
            return;
        };
        scan.at = went;
        scan.placed = true;
        scan.or_at = None;
        if let Some(misses) = scan.unanswered.get_mut(went) {
            *misses = 0;
        }
        let dial = scan.dials[went];
        self.refresh_ceiling(true);
        self.note(
            "scan",
            &format!(
                "the radio reads {}: the move went through after all",
                mhz(dial)
            ),
        );
    }

    /// Start the receiver again once the radio's change has come through the card.
    fn restart_after_move(&mut self, now: f64) {
        let due = self
            .scanner
            .scan
            .as_ref()
            .and_then(|scan| scan.restart_at)
            .is_some_and(|at| now >= at);
        if due {
            self.air.restart_front_end();
            self.air.listen_after_silence(self.readable_lead_s());
            self.scanner.restarted = true;
            if let Some(scan) = self.scanner.scan.as_mut() {
                scan.restart_at = None;
            }
        }
    }

    /// Whether the receiver was started again after a scan's move since the last call: the run
    /// loop counts that pass apart from the slow ones, since the restart's work is the scan's
    /// own (rule 2) and comes at every move.
    pub fn take_receiver_restart(&mut self) -> bool {
        std::mem::take(&mut self.scanner.restarted)
    }

    /// How much silence the receiver is given behind it on a new dial (rule 2): the longest
    /// tone frame, so a try begun before the station arrived is read from its later sync blocks
    /// rather than only noticed (§12).
    fn readable_lead_s(&self) -> f64 {
        let timing = self.air.timing();
        timing
            .floor_data_frame_s
            .unwrap_or(0.0)
            .max(timing.floor_control_frame_s.unwrap_or(0.0))
    }

    /// A new receiver from here on: what the old one held, and anything it was settling, goes.
    fn restart_receiver(&mut self) {
        self.air.restart_front_end();
        self.air.lose_captured(0);
    }

    /// The radio's dial read back, after the move was confirmed, is not the scan's (rule 9).
    fn knob_turned(&self, now: f64) -> bool {
        let Some(scan) = &self.scanner.scan else {
            return false;
        };
        if !scan.placed || now < scan.hear_from {
            return false;
        }
        let (Some(hz), Some(read_at)) = (self.frequency.value, self.frequency.read_at_s) else {
            return false;
        };
        let Some(&dial) = scan.dials.get(scan.at) else {
            return false;
        };
        read_at > scan.moved_at + 1e-6 && hz.abs_diff(dial) > KNOB_TOLERANCE_HZ
    }

    /// End the scan and go home once the radio is free, when the end calls for it.
    fn end_scan(&mut self, why: ScanEnd) {
        let home = self
            .scanner
            .scan
            .as_ref()
            .and_then(|s| s.dials.first().copied());
        self.finish_scan(why);
        if why.goes_home()
            && let Some(home) = home
            && self
                .frequency
                .value
                .is_none_or(|hz| hz.abs_diff(home) > KNOB_TOLERANCE_HZ)
        {
            self.scanner.home = Some(home);
        }
    }

    /// End the scan with the radio left where it is.
    fn end_scan_staying(&mut self, why: ScanEnd) {
        self.finish_scan(why);
        self.scanner.home = None;
    }

    fn finish_scan(&mut self, why: ScanEnd) {
        let Some(scan) = self.scanner.scan.take() else {
            return;
        };
        self.scanner.ended = Some(why);
        // the receiver starts again if a move's restart was still to come
        if scan.restart_at.is_some() {
            self.restart_receiver();
        }
        let said = match why {
            ScanEnd::Asked => "as asked",
            ScanEnd::Left => "its program has gone",
            ScanEnd::Tuned => "the radio was tuned elsewhere",
            ScanEnd::Knob => "the radio's dial was moved at the radio",
            ScanEnd::Operator => "the operator stopped it",
            ScanEnd::Radio => "fewer than two of its dials are left that the radio will take",
        };
        self.note("scan", &format!("{} stopped scanning: {said}", scan.app));
    }

    /// Back to the home dial after a scan, once nothing holds the radio where it is.
    fn go_home(&mut self) {
        let Some(home) = self.scanner.home else {
            return;
        };
        if self.dial_movable().is_err()
            || self.identifier.setup_at.is_some()
            || self.closing_identifier_pending()
        {
            return;
        }
        self.scanner.home = None;
        match self.tune_to(home) {
            Ok(()) => self.restart_receiver(),
            Err(why) => self.note(
                "scan",
                &format!(
                    "the radio stays where it is, not back to {}: {why}",
                    mhz(home)
                ),
            ),
        }
    }
}

/// A dial in megahertz, as the log says one.
fn mhz(hz: u64) -> String {
    format!("{:.4} MHz", hz as f64 / 1e6)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::control::protocol::ConnectionKind;
    use crate::ptt::{NullPtt, PttError};
    use crate::station::StationConfig;

    const DIALS: [u64; 3] = [3_590_000, 7_101_000, 10_145_000];

    /// A radio whose dial a test can turn at the knob, which can refuse a dial, and which can
    /// leave a move unanswered.
    #[derive(Debug, Default)]
    struct Rig {
        dial: u64,
        /// Dials it reads another frequency after: refused.
        refuses: Vec<u64>,
        /// Moves it does not answer: one taken off at each move to the dial it names.
        silent: Vec<u64>,
        /// Whether a move it did not answer went through all the same.
        moves_unanswered: bool,
        /// Moves it took and answered.
        set: Vec<u64>,
        /// Every move asked of it, answered or not.
        tries: Vec<u64>,
    }

    #[derive(Debug, Clone, Default)]
    struct ScanRadio {
        rig: Arc<Mutex<Rig>>,
        asked: Arc<Mutex<Vec<crate::ptt::Asked>>>,
        /// Answers a test hands the station as if a question had been served late.
        late: Arc<Mutex<Vec<crate::ptt::Readings>>>,
    }

    impl Ptt for ScanRadio {
        fn key(&mut self) -> Result<(), PttError> {
            Ok(())
        }

        fn unkey(&mut self) -> Result<(), PttError> {
            Ok(())
        }

        fn describe(&self) -> String {
            "a radio that tunes".to_owned()
        }

        fn frequency_hz(&mut self) -> Option<u64> {
            Some(self.rig.lock().expect("rig").dial)
        }

        fn can_tune(&self) -> bool {
            true
        }

        fn set_frequency_hz(&mut self, hz: u64) -> Result<(), PttError> {
            let mut rig = self.rig.lock().expect("rig");
            rig.tries.push(hz);
            if rig.refuses.contains(&hz) {
                return Err(PttError::Refused(format!(
                    "the radio reads {} Hz after being asked for {hz}",
                    rig.dial
                )));
            }
            if let Some(at) = rig.silent.iter().position(|&dial| dial == hz) {
                rig.silent.remove(at);
                if rig.moves_unanswered {
                    rig.dial = hz;
                }
                return Err(PttError::Backend(
                    "the radio did not answer after the frequency was set".into(),
                ));
            }
            rig.dial = hz;
            rig.set.push(hz);
            Ok(())
        }

        fn request_readings(&mut self, asked: crate::ptt::Asked) {
            self.asked.lock().expect("rig").push(asked);
        }

        /// Answered when the station takes them, as a radio's thread serves a question at
        /// once: a test's clock runs ahead of the wall clock, so an answer kept from the
        /// question would carry a dial the scan has since left, stamped as new.
        fn take_readings(&mut self) -> Vec<crate::ptt::Readings> {
            let dial = self.rig.lock().expect("rig").dial;
            let mut out = std::mem::take(&mut *self.late.lock().expect("rig"));
            out.extend(
                std::mem::take(&mut *self.asked.lock().expect("rig"))
                    .into_iter()
                    .map(|asked| crate::ptt::Readings {
                        at: std::time::Instant::now(),
                        asked,
                        meter: None,
                        dial_hz: asked.dial.then_some(dial),
                        settings: Vec::new(),
                        transmitting: None,
                    }),
            );
            out
        }
    }

    const fn gateway() -> ConnectionId {
        ConnectionId {
            id: 7,
            kind: ConnectionKind::Control,
        }
    }

    fn scanning_station() -> (Station<ScanRadio>, Arc<Mutex<Rig>>) {
        let radio = ScanRadio::default();
        radio.rig.lock().expect("rig").dial = 14_105_000;
        let rig = Arc::clone(&radio.rig);
        let station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            radio,
            1,
        );
        (station, rig)
    }

    /// Feed `seconds` of silence, a tenth of a second at a time.
    fn listen<P: Ptt>(station: &mut Station<P>, seconds: f64) {
        let tenth = vec![0.0f32; station.config.params.audio_rate / 10];
        for _ in 0..(seconds * 10.0).round() as usize {
            station.capture(&tenth).expect("capture");
        }
    }

    /// A data frame heard: decoded or not, acquired at `detect_confidence`.
    fn report(decoded: bool, detect_confidence: f64) -> FrameReport {
        FrameReport {
            t_s: 0.0,
            kind: "data",
            mode: 5,
            rv: 0,
            snr_db: -3.0,
            cfo_hz: 0.0,
            confidence: 1.0,
            detect_confidence,
            decoded,
            bytes: 0,
            from: None,
            to: None,
            control: None,
            bandwidth_hz: None,
            follows: None,
            capture_skip: false,
            echo: None,
            answering: None,
            position: None,
        }
    }

    fn dwell(station: &Station<ScanRadio>) -> f64 {
        station.scan_status()["dwell_s"].as_f64().expect("dwell")
    }

    /// Listen a tenth of a second at a time until `done`, at most `seconds`.
    fn until(
        station: &mut Station<ScanRadio>,
        seconds: f64,
        mut done: impl FnMut(&Station<ScanRadio>) -> bool,
    ) {
        let limit = station.now() + seconds;
        while !done(station) {
            assert!(station.now() < limit, "not within {seconds} s");
            listen(station, 0.1);
        }
    }

    /// The gateway of 2026-11-17: its home dial, and the one it scanned with it.
    const GATEWAY: [u64; 2] = [7_101_000, 3_590_000];

    /// A scan of `dials` started with the radio on the first, as the gateway's was.
    fn scanning_from_home(dials: &[u64]) -> (Station<ScanRadio>, Arc<Mutex<Rig>>) {
        let (mut station, rig) = scanning_station();
        rig.lock().expect("rig").dial = dials[0];
        // the station reads the dial, so the scan starts where the radio is
        listen(&mut station, 1.5);
        station.scan_start(gateway(), dials).expect("a scan");
        assert_eq!(station.scan_status()["at_hz"], dials[0]);
        (station, rig)
    }

    #[test]
    fn a_scan_needs_a_radio_that_tunes_and_two_to_four_distinct_dials() {
        let mut deaf = Station::new(StationConfig::default(), NullPtt::default(), 1);
        assert_eq!(
            deaf.scan_start(gateway(), &DIALS),
            Err(ScanRefusal::CannotTune)
        );
        let (mut station, _) = scanning_station();
        for bad in [
            &[7_101_000][..],
            &[7_101_000, 7_101_000],
            &[7_101_000, 0],
            &[1, 2, 3, 4, 5],
        ] {
            let refused = station.scan_start(gateway(), bad).expect_err("bad dials");
            assert_eq!(
                refused.code(),
                "bad_params",
                "{bad:?}: {}",
                refused.message()
            );
        }
        assert!(!station.scanning());
        let other = ConnectionId {
            id: 8,
            kind: ConnectionKind::Control,
        };
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let refused = station
            .scan_start(other, &DIALS)
            .expect_err("another's scan runs");
        assert_eq!(refused.code(), "claimed");
        // its own owner replaces it
        station
            .scan_start(gateway(), &DIALS[..2])
            .expect("the owner's new list");
        assert_eq!(station.scan_status()["dials"], json!(DIALS[..2]));
    }

    #[test]
    fn a_dwell_is_a_listen_and_a_linger_the_callers_gap() {
        let (mut station, _) = scanning_station();
        let scan = station.scan_start(gateway(), &DIALS).expect("a scan");
        let (before, _) = station.engine().call_gaps();
        let dwell = move_deaf_s() + NOTICE_LISTEN_S;
        let linger = before + station.engine().call_deferral_s() + move_deaf_s();
        assert!((scan["dwell_s"].as_f64().expect("dwell") - dwell).abs() < 1e-3);
        assert!((scan["linger_s"].as_f64().expect("linger") - linger).abs() < 1e-3);
        // a round of the dials at their longest and two lingers: a try noticed, then two the
        // scan stays for
        let rounded = (DIALS.len() as f64 * (dwell + DWELL_JITTER_S) + 2.0 * linger).ceil();
        assert_eq!(scan["scan_s"], json!(rounded));
    }

    #[test]
    fn the_radio_goes_round_the_dials_a_dwell_on_each() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        // not on any of them: it goes to the home dial at once
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        // each visit a dwell and a part of DWELL_JITTER_S, never more
        let dwell = dwell(&station);
        let mut moved = Vec::new();
        let mut last = station.now();
        for _ in 0..3 {
            let from = rig.lock().expect("rig").set.len();
            while rig.lock().expect("rig").set.len() == from {
                // the helper listens in tenths of a second
                listen(&mut station, 0.1);
                assert!(
                    station.now() - last < dwell + DWELL_JITTER_S + 0.2,
                    "stayed too long"
                );
            }
            assert!(station.now() - last > dwell - 0.3, "left too soon");
            moved.push(station.now() - last);
            last = station.now();
        }
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        // the draws differ: a round of the dials does not keep in step with a caller
        assert!(
            (moved[1] - moved[2]).abs() > 0.05 || (moved[0] - moved[1]).abs() > 0.05,
            "{moved:?}"
        );
        assert_eq!(
            rig.lock().expect("rig").set,
            [DIALS[0], DIALS[1], DIALS[2], DIALS[0]]
        );
        assert_eq!(station.scan_status()["hops"], 4);
    }

    #[test]
    fn each_move_restarts_the_receiver_once_and_the_run_loop_is_told() {
        // the run loop counts the pass that started the receiver again apart from the slow
        // ones (report 19 §8): told once a move, and never without one
        let (mut station, rig) = scanning_station();
        listen(&mut station, 1.0);
        assert!(!station.take_receiver_restart(), "no scan, no restart");
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let mut restarts = 0;
        let end = station.now() + 3.0 * (dwell(&station) + DWELL_JITTER_S) + 1.0;
        while station.now() < end {
            listen(&mut station, 0.1);
            restarts += usize::from(station.take_receiver_restart());
        }
        let moves = rig.lock().expect("rig").set.len();
        assert!(moves >= 3, "{moves} moves");
        assert!(
            (moves - 1..=moves).contains(&restarts),
            "{moves} moves, {restarts} restarts"
        );
    }

    #[test]
    fn a_call_or_a_frame_arriving_holds_the_dial_and_a_linger_follows_a_contact() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        let dwell = dwell(&station);
        // a frame announced holds the radio until it can have been decoded or refused; under the
        // confidence a try under way reaches it starts no linger
        let now = station.now();
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        station.scan_arriving(now + dwell + 2.0, NOTICE_CONFIDENCE - 0.05, false, now);
        listen(&mut station, dwell + 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "left a frame arriving"
        );
        assert_eq!(station.scan_status()["held"], "a frame is arriving");
        listen(&mut station, REPORT_AFTER_END_S + 0.3);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[1],
            "lingered on an announcement"
        );
        // a frame that came to nothing at the confidence a try reaches is a caller too weak to
        // read this time: it lingers for the next try; a noise candidate does not
        listen(&mut station, move_deaf_s() + 0.1);
        let now = station.now();
        station.scan_heard(&report(false, NOTICE_CONFIDENCE - 0.05), now);
        assert!(
            station.scan_status()["lingering_until_ms"].is_null(),
            "lingered on noise"
        );
        station.scan_heard(&report(false, NOTICE_CONFIDENCE + 0.2), now);
        assert!(!station.scan_status()["lingering_until_ms"].is_null());
        listen(&mut station, linger - 2.0);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1], "did not linger");
        listen(&mut station, 3.0);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[2]);
        // a call out holds it, and when the call is over it lingers there
        station.connect("KK4XYZ").expect("idle");
        listen(&mut station, dwell + 1.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[2],
            "left a call going out"
        );
        station.abort();
        listen(&mut station, 1.0);
        assert!(!station.scan_status()["lingering_until_ms"].is_null());
        listen(&mut station, linger - 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[2],
            "did not linger after the call"
        );
        listen(&mut station, 3.0);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
    }

    #[test]
    fn the_operator_wins_and_the_program_is_told() {
        // another program's tune, or the panel's: the radio stays where it was put
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.5);
        station.take_events();
        station.tune_for(None, 7_074_000).expect("tuned");
        assert!(!station.scanning());
        assert_eq!(station.scan_event()["why"], "tuned by another program");
        assert!(station.take_events().iter().any(|e| e.starts_with("scan:")));
        listen(&mut station, 5.0);
        assert_eq!(rig.lock().expect("rig").dial, 7_074_000);
        // the knob: a dial read back, after the move, that is not the scan's
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 2.0);
        rig.lock().expect("rig").dial = 3_587_000;
        listen(&mut station, 2.0);
        assert!(!station.scanning());
        assert_eq!(station.scan_event()["why"], "knob");
        listen(&mut station, 5.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            3_587_000,
            "taken back from the operator"
        );
    }

    #[test]
    fn a_scan_ends_with_its_connection_and_the_radio_goes_home() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let dwell = dwell(&station);
        listen(&mut station, dwell + DWELL_JITTER_S + 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
        station.connection_closed(gateway());
        assert!(!station.scanning());
        assert_eq!(station.scan_event()["why"], "connection closed");
        listen(&mut station, 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        // and its owner's stop is the same, but for the word
        station.scan_start(gateway(), &DIALS[1..]).expect("a scan");
        listen(&mut station, 0.3);
        assert!(station.scan_stop(gateway()));
        assert_eq!(station.scan_event()["why"], "asked");
        listen(&mut station, 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
        // the panel, which holds no claim, stops any program's scan, and the radio stays
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.3);
        let panel = ConnectionId {
            id: 99,
            kind: ConnectionKind::Control,
        };
        assert!(station.scan_stop(panel));
        assert_eq!(station.scan_event()["why"], "operator");
        listen(&mut station, 0.3);
        // the scan began on 7.101 MHz, one of its dials, and the radio stays there
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
    }

    /// A dial the radio refuses — it reads another frequency after the move — is left out at
    /// the first move to it.
    #[test]
    fn a_dial_the_radio_refuses_is_dropped() {
        let (mut station, rig) = scanning_station();
        rig.lock().expect("rig").refuses.push(DIALS[1]);
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let dwell = dwell(&station);
        listen(&mut station, dwell + DWELL_JITTER_S + 0.3);
        assert_eq!(station.scan_status()["dials"], json!([DIALS[0], DIALS[2]]));
        assert_eq!(station.scan_status()["dropped"][0]["hz"], DIALS[1]);
        assert_eq!(
            rig.lock()
                .expect("rig")
                .tries
                .iter()
                .filter(|&&d| d == DIALS[1])
                .count(),
            1,
            "asked once"
        );
        listen(&mut station, 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[2]);
        // below two, the scan is over
        rig.lock().expect("rig").refuses.push(DIALS[0]);
        listen(&mut station, dwell + 0.5);
        assert!(!station.scanning());
        assert_eq!(station.scan_event()["why"], "radio");
    }

    /// The gateway on 2026-11-17 (report 18 §3): the FTDX10, just out of transmit, did not
    /// answer the read-back of the scan's move, and the scan dropped the dial and ended. Now the
    /// dial is kept, the scan listens a dwell where the radio is, and asks for it again.
    #[test]
    fn a_move_the_radio_did_not_answer_keeps_the_dial_and_the_scan_goes_on() {
        let (mut station, rig) = scanning_from_home(&GATEWAY);
        let dwell = dwell(&station);
        rig.lock().expect("rig").silent.push(GATEWAY[1]);
        station.take_events();
        until(&mut station, dwell + 1.0, |_| {
            !rig.lock().expect("rig").tries.is_empty()
        });
        let missed_at = station.now();
        listen(&mut station, 1.5);
        assert!(station.scanning(), "{}", station.scan_event());
        assert_eq!(station.scan_status()["dials"], json!(GATEWAY));
        assert_eq!(station.scan_status()["dropped"], json!([]));
        assert_eq!(rig.lock().expect("rig").dial, GATEWAY[0]);
        assert_eq!(
            station.scan_status()["at_hz"],
            GATEWAY[0],
            "where the radio is"
        );
        let events = station.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.contains("did not answer at 3.5900 MHz; the scan tries it again")),
            "{events:?}"
        );
        // a dwell, and it asks again; the radio answers this time
        until(&mut station, dwell + DWELL_JITTER_S, |_| {
            rig.lock().expect("rig").tries.len() == 2
        });
        let waited = station.now() - missed_at;
        assert!(
            waited > dwell - 0.2,
            "asked again {waited:.1} s after the miss"
        );
        assert_eq!(rig.lock().expect("rig").dial, GATEWAY[1]);
        assert_eq!(station.scan_status()["at_hz"], GATEWAY[1]);
        // and round again
        until(&mut station, dwell + DWELL_JITTER_S + 0.2, |_| {
            rig.lock().expect("rig").dial == GATEWAY[0]
        });
        assert_eq!(
            rig.lock().expect("rig").tries,
            [GATEWAY[1], GATEWAY[1], GATEWAY[0]]
        );
        assert!(station.scanning());
    }

    /// A move that went through though its read-back was missed: the next reading of the dial
    /// says where the radio is, and the scan goes on from there. Without it that reading is a
    /// dial that is not the scan's, taken for the knob, and the scan ends.
    #[test]
    fn a_move_that_went_through_unanswered_is_found_by_the_next_reading() {
        let (mut station, rig) = scanning_from_home(&GATEWAY);
        let dwell = dwell(&station);
        {
            let mut rig = rig.lock().expect("rig");
            rig.silent.push(GATEWAY[1]);
            rig.moves_unanswered = true;
        }
        until(&mut station, dwell + 1.0, |_| {
            !rig.lock().expect("rig").tries.is_empty()
        });
        assert_eq!(rig.lock().expect("rig").dial, GATEWAY[1], "it went");
        listen(&mut station, 1.5);
        assert!(station.scanning(), "{}", station.scan_event());
        assert_eq!(station.scan_status()["at_hz"], GATEWAY[1]);
        until(&mut station, dwell + DWELL_JITTER_S, |_| {
            rig.lock().expect("rig").tries.len() == 2
        });
        assert_eq!(rig.lock().expect("rig").tries, [GATEWAY[1], GATEWAY[0]]);
        assert_eq!(rig.lock().expect("rig").dial, GATEWAY[0]);
    }

    /// The first move, from a dial that is none of the scan's, unanswered: the scan does not
    /// ask again at the next pass — three misses in a third of a second would drop the dial —
    /// but after a dwell.
    #[test]
    fn a_first_move_unanswered_is_asked_again_after_a_dwell() {
        let (mut station, rig) = scanning_station();
        rig.lock().expect("rig").silent.push(DIALS[0]);
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let dwell = dwell(&station);
        listen(&mut station, dwell - 0.3);
        assert_eq!(rig.lock().expect("rig").tries, [DIALS[0]]);
        assert_eq!(rig.lock().expect("rig").dial, 14_105_000);
        until(&mut station, DWELL_JITTER_S + 0.6, |_| {
            rig.lock().expect("rig").tries.len() == 2
        });
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        assert_eq!(station.scan_status()["dials"], json!(DIALS));
    }

    /// A dial the radio leaves unanswered three moves in a row is left out; misses with an
    /// answer between them are not in a row.
    #[test]
    fn a_dial_unanswered_three_moves_in_a_row_is_dropped() {
        let dials = [GATEWAY[0], GATEWAY[1], 10_145_000];
        let (mut station, rig) = scanning_from_home(&dials);
        let dwell = dwell(&station);
        let round = 3.0 * (dwell + DWELL_JITTER_S) + 1.0;
        // two misses, then an answer
        rig.lock().expect("rig").silent = vec![dials[1]; 2];
        until(&mut station, 3.0 * round, |_| {
            rig.lock().expect("rig").set.contains(&dials[1])
        });
        assert_eq!(rig.lock().expect("rig").tries, [dials[1]; 3]);
        assert_eq!(station.scan_status()["dials"], json!(dials));
        // three in a row
        rig.lock().expect("rig").silent = vec![dials[1]; 3];
        until(&mut station, 4.0 * round, |station| {
            station.scan_status()["dropped"]
                .as_array()
                .is_some_and(|dropped| !dropped.is_empty())
        });
        let status = station.scan_status();
        assert_eq!(status["dials"], json!([dials[0], dials[2]]));
        assert_eq!(status["dropped"][0]["hz"], dials[1]);
        assert!(
            status["dropped"][0]["why"]
                .as_str()
                .is_some_and(|why| why.starts_with("no answer 3 times in a row")),
            "{status}"
        );
        let asked = rig.lock().expect("rig").tries.clone();
        assert_eq!(
            asked.iter().filter(|&&d| d == dials[1]).count(),
            6,
            "{asked:?}"
        );
        // the scan goes on, at once, with the dial after it
        assert!(station.scanning());
        listen(&mut station, 0.3);
        assert_eq!(rig.lock().expect("rig").dial, dials[2]);
    }

    #[test]
    fn a_dial_the_rules_refuse_refuses_the_whole_list() {
        // an automatic station keeps to the §97.221(b) segments: 7.101 MHz is one, 7.050 is not
        let (mut station, _) = scanning_station();
        station.set_regulatory(crate::regulatory::Settings {
            profile: "us-fcc-part97".to_owned(),
            control: Some(crate::regulatory::ControlMode::Automatic),
            license: Some(crate::regulatory::LicenseClass::Extra),
            ..crate::regulatory::Settings::unchecked()
        });
        let refused = station
            .scan_start(gateway(), &[7_101_000, 7_050_000])
            .expect_err("outside the segments");
        assert_eq!(refused.code(), "regulatory");
        assert!(
            refused.message().contains("7.0500"),
            "{}",
            refused.message()
        );
        station
            .scan_start(gateway(), &[7_101_000, 14_105_000])
            .expect("both automatic segments");
    }

    #[test]
    fn a_call_asked_to_last_makes_the_tries_it_needs() {
        let (mut station, _) = scanning_station();
        station
            .connect_keeping("KK4XYZ", None, false, Some(54.0))
            .expect("idle");
        let want = station.engine().tries_lasting(Some(54.0), None);
        assert!(want > station.config.link.call_tries);
        assert_eq!(station.engine().call_plan().len(), want);
    }

    /// A try noticed partway through (its middle or end sync block announced) keeps the radio a
    /// linger from the notice, so the caller's next try begins while it listens (§11).
    #[test]
    fn a_try_noticed_partway_through_keeps_the_radio_for_the_next() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        let dwell = dwell(&station);
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        listen(&mut station, dwell - 1.5);
        let now = station.now();
        // the stream takes the block it heard for a frame's start: it ends a tone frame later
        station.scan_arriving(now + 5.4, 1.6, true, now);
        listen(&mut station, linger - 1.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "left a dial a call is under way on"
        );
        assert!(!station.scan_status()["lingering_until_ms"].is_null());
        listen(&mut station, 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[1],
            "stayed past the linger"
        );
    }

    /// The log's lines for visits that stayed past their dwell.
    fn stays(station: &mut Station<ScanRadio>) -> Vec<String> {
        station
            .take_events()
            .into_iter()
            .filter(|e| e.starts_with("scan:stayed"))
            .collect()
    }

    #[test]
    fn a_visit_kept_past_its_dwell_says_why_once_the_scan_leaves() {
        // report 20 §3: a busy dial kept the scan 184–214 s at a time, and the log could not say
        // on what
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        let dwell = dwell(&station);
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        let on = |dial: u64| {
            let rig = Arc::clone(&rig);
            move |_: &Station<ScanRadio>| rig.lock().expect("rig").dial == dial
        };
        // a quiet visit says nothing when the scan leaves it
        stays(&mut station);
        until(&mut station, dwell + DWELL_JITTER_S + 1.0, on(DIALS[1]));
        assert!(
            stays(&mut station).is_empty(),
            "a quiet visit said why it stayed"
        );
        // a try noticed on the floor, two frames that came to nothing, and a noise candidate
        // under the gate, which keeps nothing and is not named
        listen(&mut station, move_deaf_s() + 0.1);
        let now = station.now();
        station.scan_arriving(now + 1.0, 1.4, true, now);
        listen(&mut station, 3.0);
        let now = station.now();
        station.scan_heard(&report(false, 1.3), now);
        station.scan_heard(&report(false, 1.25), now);
        station.scan_heard(&report(false, NOTICE_CONFIDENCE - 0.05), now);
        until(
            &mut station,
            linger + dwell + DWELL_JITTER_S + 2.0,
            on(DIALS[2]),
        );
        let said = stays(&mut station);
        assert_eq!(said.len(), 1, "{said:?}");
        let line = &said[0];
        for part in [
            "on 7.1010 MHz",
            "1 announced (1 on the floor, 0 ordinary)",
            "2 reported and not decoded (data at rung 5 ×2)",
            "at 1.25–1.40",
        ] {
            assert!(line.contains(part), "{part:?} not in {line:?}");
        }
        assert!(!line.contains("the most a visit lasts"), "{line}");
        // frames that keep coming keep the radio until the most a visit lasts, and the line
        // says so
        let cap = dwell + 4.0;
        if let Some(scan) = station.scanner.scan.as_mut() {
            scan.cap_s = cap;
        }
        listen(&mut station, move_deaf_s() + 0.1);
        for _ in 0..4 {
            let now = station.now();
            station.scan_heard(&report(false, 1.2), now);
            listen(&mut station, 1.5);
        }
        until(&mut station, cap + 2.0, on(DIALS[0]));
        let said = stays(&mut station);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("on 10.1450 MHz") && said[0].contains("the most a visit lasts"),
            "{said:?}"
        );
    }

    /// A caller's first try to `to` as a station sends it to one it has not heard: the audio
    /// from its first sample to its last, rendered by a station of its own.
    fn a_callers_try(to: &str) -> Vec<f32> {
        let mut caller = Station::new(
            StationConfig {
                callsign: "KK4XYZ".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            2,
        );
        caller.connect(to).expect("idle");
        let block = 960;
        let (mut out, silence) = (vec![0.0f32; block], vec![0.0f32; block]);
        let (mut played, mut audio) = (0u64, Vec::new());
        for _ in 0..1000 {
            caller.device_played(played);
            let handed = caller.playback(&mut out).expect("playback");
            caller.device_drains_at(played + handed as u64);
            played += block as u64;
            caller.capture(&silence).expect("capture");
            if handed > 0 {
                audio.extend_from_slice(&out[..handed]);
            } else if !audio.is_empty() {
                break;
            }
        }
        let first = audio.iter().position(|&x| x != 0.0).expect("a try");
        let last = audio.iter().rposition(|&x| x != 0.0).expect("a try");
        audio[first..=last].to_vec()
    }

    /// A try begun on a dial before the scan got there is read, not only noticed: the receiver
    /// started again after the move has the longest tone frame of silence behind it (ADR-0129
    /// §12), and finds the try's tone frame by its later sync blocks. Started with nothing
    /// behind it, the station noticed the try and waited for the next.
    #[test]
    fn a_try_begun_before_the_scan_arrived_is_read() {
        let (mut station, rig) = scanning_station();
        let try_audio = a_callers_try("W4ODA");
        let rate = station.config.params.audio_rate;
        let block = rate / 50;
        let silence = vec![0.0f32; block];
        station.scan_start(gateway(), &DIALS).expect("a scan");
        // on the first dial until the scan moves to the caller's
        let mut waited = 0;
        while rig.lock().expect("rig").dial != DIALS[1] {
            station.capture(&silence).expect("capture");
            waited += 1;
            assert!(waited < 50 * 30, "the scan never reached the caller's dial");
        }
        // the caller's try began two seconds before: its tone frame a second before
        let mut heard = try_audio[2 * rate..].to_vec();
        heard.extend(vec![0.0f32; 2 * rate]);
        for chunk in heard.chunks(block) {
            let mut piece = chunk.to_vec();
            piece.resize(block, 0.0);
            station.capture(&piece).expect("capture");
            if station.engine().state() != State::Idle {
                break;
            }
        }
        assert_ne!(
            station.engine().state(),
            State::Idle,
            "a try begun before the scan arrived was not read"
        );
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[1],
            "answered on another dial"
        );
    }

    /// A dial reading taken before the one the station holds is stale, and is not taken: on a
    /// radio thread a question asked just before a scan's move can come back with the dial the
    /// radio has left, and the station took it for its dial (the scan then thought the radio
    /// home, and left it on another).
    #[test]
    fn a_dial_reading_taken_before_the_last_is_not_taken() {
        let radio = ScanRadio::default();
        radio.rig.lock().expect("rig").dial = DIALS[1];
        let late = Arc::clone(&radio.late);
        let mut station = Station::new(StationConfig::default(), radio, 1);
        listen(&mut station, 1.5);
        assert_eq!(station.frequency.value, Some(DIALS[1]));
        let stale = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(5))
            .expect("an instant five seconds ago");
        late.lock().expect("rig").push(crate::ptt::Readings {
            at: stale,
            asked: crate::ptt::Asked {
                dial: true,
                ..crate::ptt::Asked::default()
            },
            meter: None,
            dial_hz: Some(DIALS[0]),
            settings: Vec::new(),
            transmitting: None,
        });
        listen(&mut station, 0.1);
        assert_eq!(
            station.frequency.value,
            Some(DIALS[1]),
            "took a stale reading"
        );
    }

    /// A piece of a datagram decoded, as a broadcast's.
    fn a_datagram_piece() -> FrameReport {
        FrameReport {
            kind: "datagram",
            mode: 1,
            from: Some("KK4XYZ".to_owned()),
            ..report(true, 2.0)
        }
    }

    /// A broadcast heard keeps the radio where it came from for a linger (rule 11): a sender
    /// outlasting the scan sends copies back to back, and the gap between two of them, or a
    /// copy heard partway through, must not move it on. Decoded as another station's frame, it
    /// had ended the linger.
    #[test]
    fn a_piece_of_a_datagram_keeps_the_radio_for_a_linger() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, move_deaf_s() + 0.1);
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        let now = station.now();
        station.scan_heard(&a_datagram_piece(), now);
        assert!(
            !station.scan_status()["lingering_until_ms"].is_null(),
            "a datagram's piece started no linger"
        );
        listen(&mut station, linger - 1.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "left a dial a broadcast was heard on"
        );
        // the next copy's piece keeps it again
        let now = station.now();
        station.scan_heard(&a_datagram_piece(), now);
        listen(&mut station, linger - 1.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "left between copies"
        );
        listen(&mut station, 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[1],
            "stayed past the linger"
        );
    }

    /// A dial busy with signals the receiver notices and never reads keeps the radio at most two
    /// lingers from the first (§17). On 2026-11-21 the gateway's 40 m and 20 m dials held it up
    /// to 205 s a visit, the longest call, each new look-alike starting the linger again, and a
    /// caller on another dial went unheard. What it reads keeps it as before.
    #[test]
    fn a_dial_busy_with_signals_never_read_keeps_the_radio_two_lingers() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, move_deaf_s() + 0.1);
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        stays(&mut station);
        // another mode's traffic: frames announced and reported undecoded, over the notice's
        // confidence, every few seconds, each held until it can have been decoded
        let first = station.now();
        let mut next = first;
        while station.scanner.scan.as_ref().expect("a scan").at == 0 {
            let now = station.now();
            assert!(
                now - first < 3.0 * linger,
                "still on the busy dial after {:.1} s",
                now - first
            );
            if now >= next {
                station.scan_arriving(now + 3.0, NOTICE_CONFIDENCE + 0.2, false, now);
                station.scan_heard(&report(false, NOTICE_CONFIDENCE + 0.2), now);
                next = now + 6.0;
            }
            listen(&mut station, 0.1);
        }
        let stayed = station.now() - first;
        assert!(
            (2.0 * linger - 0.5..=2.0 * linger + 0.5).contains(&stayed),
            "left after {stayed:.1} s, not two lingers ({:.1} s)",
            2.0 * linger
        );
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
        // and the log's line for the visit says what ended it (rule 13)
        let said = stays(&mut station);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("left two lingers after the first frame it did not read"),
            "{said:?}"
        );
        // the next dial's guesses count from its own first
        listen(&mut station, move_deaf_s() + 0.1);
        let first = station.now();
        station.scan_heard(&report(false, NOTICE_CONFIDENCE + 0.2), first);
        listen(&mut station, linger - 1.0);
        let now = station.now();
        station.scan_heard(&report(false, NOTICE_CONFIDENCE + 0.2), now);
        listen(&mut station, linger - 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[1],
            "the last dial's guesses counted here"
        );
        until(&mut station, 3.0, |s| s.scan_status()["at_hz"] == DIALS[2]);
        // what was read keeps the radio past two lingers: the copies of a broadcast
        listen(&mut station, move_deaf_s() + 0.1);
        let first = station.now();
        for _ in 0..3 {
            let now = station.now();
            station.scan_heard(&report(false, NOTICE_CONFIDENCE + 0.2), now);
            station.scan_heard(&a_datagram_piece(), now);
            listen(&mut station, linger - 1.0);
        }
        assert!(station.now() - first > 2.0 * linger);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[2],
            "left a dial a broadcast was being read on"
        );
    }

    /// A datagram waiting to go out holds the dial (rule 11): a program's answer to a broadcast
    /// heard here goes out here, not on the dial the scan moves to next — for as long as a visit
    /// may last, then it goes with the radio. A tune still takes it, as before.
    #[test]
    fn a_datagram_waiting_holds_the_dial_as_long_as_a_visit_may_last() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        let dwell = dwell(&station);
        let linger = station.scan_status()["linger_s"].as_f64().expect("linger");
        let cap = station.scanner.scan.as_ref().expect("a scan").cap_s;
        assert!(cap > 2.0 * linger, "a visit's most is {cap} s");
        // a channel never clear enough for it, as p-persistence 0 has it: it waits
        let moved_at = station.scanner.scan.as_ref().expect("a scan").moved_at;
        station
            .send_datagram(crate::station::DatagramRequest {
                frame_type: 2,
                frame: vec![0x42; 60],
                reference: Some("heard".to_owned()),
                rung: Some(1),
                wait_for_clear: true,
                persistence: 0.0,
                slot_s: 0.1,
                dial_hz: None,
            })
            .expect("queued");
        listen(&mut station, dwell + linger + 2.0);
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "left a datagram behind"
        );
        assert_eq!(
            station.scan_status()["held"],
            "a datagram is waiting to go out on this dial"
        );
        assert!(
            station.dial_movable().is_ok(),
            "a datagram waiting held a tune too"
        );
        until(&mut station, cap, |s| s.scan_status()["at_hz"] != DIALS[0]);
        let left = station.now() - moved_at;
        assert!(
            left >= cap - 0.2,
            "left after {left:.1} s, before a visit's most, {cap:.1} s"
        );
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
    }

    /// A scanning station whose transmissions the rules do not judge: a datagram it queues goes
    /// out when the station lets it.
    fn scanning_station_unchecked() -> (Station<ScanRadio>, Arc<Mutex<Rig>>) {
        let radio = ScanRadio::default();
        radio.rig.lock().expect("rig").dial = 14_105_000;
        let rig = Arc::clone(&radio.rig);
        let station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                regulatory: crate::regulatory::Settings::unchecked(),
                ..StationConfig::default()
            },
            radio,
            1,
        );
        (station, rig)
    }

    /// A datagram to go at once, belonging on `dial`.
    fn an_answer_for(dial: Option<u64>) -> crate::station::DatagramRequest {
        crate::station::DatagramRequest {
            frame_type: 2,
            frame: vec![0x42; 30],
            reference: Some("seen".to_owned()),
            rung: Some(1),
            wait_for_clear: false,
            persistence: 1.0,
            slot_s: 0.1,
            dial_hz: dial,
        }
    }

    /// Play and listen a tenth of a second at a time, as the run loop does, until `done`, at
    /// most `seconds`: what is queued is fed and keyed only from the playback side.
    fn running_until(
        station: &mut Station<ScanRadio>,
        seconds: f64,
        mut done: impl FnMut(&Station<ScanRadio>) -> bool,
    ) {
        let tenth = station.config.params.audio_rate / 10;
        let mut out = vec![0.0f32; tenth];
        let heard = vec![0.0f32; tenth];
        let limit = station.now() + seconds;
        while !done(station) {
            assert!(station.now() < limit, "not within {seconds} s");
            station.playback(&mut out).expect("playback");
            station.capture(&heard).expect("capture");
        }
    }

    fn queued(station: &Station<ScanRadio>) -> u64 {
        station.datagram_status()["queued"]
            .as_u64()
            .expect("queued")
    }

    /// A datagram heard whole says the dial it was heard on (§15): where its answer belongs.
    #[test]
    fn a_datagram_heard_whole_says_the_dial_it_was_heard_on() {
        let (mut station, _rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, move_deaf_s() + 0.1);
        let carried = aether_link::datagram::body("KK4XYZ", 2, &[0x5A; 50]).expect("body");
        for piece in aether_link::datagram::fragments(&carried, 7, 36).expect("fits") {
            let now = station.now();
            station.heard_datagram_piece(&piece, 3.5, 1, now);
        }
        let heard = station.take_received_datagrams();
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(heard[0].dial_hz, Some(DIALS[0]));
    }

    /// A datagram that names another of the scan's dials waits for the scan to get there, and
    /// the scan goes there next, not round its list (rule 12): an alert's *seen*, pressed a
    /// minute after the alert was heard. One that names a dial the scan has not goes where the
    /// radio is.
    #[test]
    fn a_datagram_for_another_of_the_dials_goes_out_there_and_the_scan_goes_there_next() {
        let (mut station, rig) = scanning_station_unchecked();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        let dwell = dwell(&station);
        until(&mut station, dwell + 2.0, |s| {
            s.scan_status()["at_hz"] == DIALS[1]
        });
        station
            .send_datagram(an_answer_for(Some(DIALS[0])))
            .expect("queued");
        let a_second_on = station.now() + 1.0;
        running_until(&mut station, 2.0, |s| s.now() >= a_second_on);
        assert_eq!(queued(&station), 1, "went out on the wrong dial");
        assert!(
            station.scan_status()["held"].is_null(),
            "held the wrong dial for it"
        );
        // the visit ends, and the scan goes to the datagram's dial, not to the next in order
        running_until(&mut station, dwell + 2.0, |s| {
            s.scan_status()["at_hz"] != DIALS[1]
        });
        assert_eq!(
            rig.lock().expect("rig").dial,
            DIALS[0],
            "went round instead"
        );
        running_until(&mut station, 2.0, |s| queued(s) == 0);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        // a dial the scan has not: it goes where the radio is
        running_until(&mut station, 60.0, |s| {
            !s.transmitting && s.pending.is_empty() && s.scan_status()["held"].is_null()
        });
        let here = rig.lock().expect("rig").dial;
        station
            .send_datagram(an_answer_for(Some(14_105_000)))
            .expect("queued");
        running_until(&mut station, 1.0, |s| queued(s) == 0);
        assert_eq!(rig.lock().expect("rig").dial, here);
    }
}
