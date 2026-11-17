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
//!    answered, on the next.
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
//!    time, tries again. At most the longest call in all; a frame decoded as another station's
//!    ends it. Under that confidence an announcement only holds the dial (rule 4): noise reached
//!    1.07, and a tone arrival on noise at 1.01 had once held a dial nobody used.
//! 6. After a session, a call or a probe it goes on from the same dial, with a linger.
//! 7. The list is judged once, at the start, as this station's answer would be.
//! 8. A dial the radio will not take is dropped; below two dials the scan ends.
//! 9. **The operator wins**: another tune, or the knob, ends the scan, and its owner is told.
//! 10. **A scan belongs to its connection**: when the connection closes, the scan ends and the
//!     radio goes to the home dial once nothing holds it where it is.
//!
//! [`GrantedEngine::call_gaps`]: aether_link::GrantedEngine::call_gaps
//! [`GrantedEngine::call_deferral_s`]: aether_link::GrantedEngine::call_deferral_s

use serde_json::{Value, json};

use super::{FrameReport, RegulatoryQuery, State, Station};
use crate::control::protocol::ConnectionId;
use crate::ptt::Ptt;
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
/// How far a dial read back may differ from the dial asked for before it is taken for the knob:
/// a radio that rounds its dial to its tuning step.
const KNOB_TOLERANCE_HZ: u64 = 50;

/// How long a caller's tries must keep starting to be read by a scan of `dials` dials (the
/// `scan_s` a program publishes): a round of them, and two lingers, so a try noticed partway
/// through is followed by two the scan stays for.
#[must_use]
pub fn must_last(dials: usize, dwell_s: f64, linger_s: f64) -> f64 {
    (dials as f64 * dwell_s + 2.0 * linger_s).ceil()
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
    /// The longest a visit lasts however much it lingers: the longest call (fifteen tries).
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
    /// What held it at the last pass was a contact: a session, a call, a probe or a Test.
    held_by_contact: bool,
    hops: u64,
    /// Dials the radio would not take, and why.
    dropped: Vec<(u64, String)>,
    since_ms: u64,
}

/// The station's scan, the radio's way home after one, and how the last one ended.
#[derive(Debug, Default)]
pub(super) struct Scanner {
    scan: Option<Scan>,
    /// A dial to go back to once nothing holds the radio where it is.
    home: Option<u64>,
    /// Why the last scan ended, until another starts.
    ended: Option<ScanEnd>,
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
            cap_s: longest_call + dwell_s,
            must_last_s: must_last(dials.len(), dwell_s, linger_s),
            moved_at: now,
            hear_from: now,
            restart_at: None,
            leave_at: now + dwell_s,
            linger_until: None,
            arriving_until: f64::NEG_INFINITY,
            held_by_contact: false,
            hops: 0,
            dropped: Vec::new(),
            since_ms: super::beacons::unix_ms(),
        });
        self.scanner.home = None;
        self.scanner.ended = None;
        let list: Vec<String> = dials.iter().map(|&d| mhz(d)).collect();
        self.note(
            "scan",
            &format!(
                "scanning {} for {app}: {:.1} s on each, {:.1} s more where a call is under way, \
                 a call must keep trying {:.0} s",
                list.join(", "),
                dwell_s,
                linger_s,
                must_last(dials.len(), dwell_s, linger_s)
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
    /// confidence, and the harness's first grid found one on noise, at 1.01.
    pub(super) fn scan_arriving(&mut self, end_s: f64, confidence: f64, now: f64) {
        if let Some(scan) = &mut self.scanner.scan {
            scan.arriving_until = scan.arriving_until.max(end_s + REPORT_AFTER_END_S);
            if confidence >= NOTICE_CONFIDENCE && now >= scan.hear_from {
                let until = now + scan.linger_s;
                scan.linger_until = Some(scan.linger_until.map_or(until, |t| t.max(until)));
            }
        }
    }

    /// A frame reported (rule 5): one that came to nothing at [`NOTICE_CONFIDENCE`] or more, or
    /// a probe this station answers, keeps it a linger longer; one decoded as another station's
    /// ends the linger. The noise candidates a quiet band brings fall under the gate.
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
        let anothers = report.decoded
            && match report.kind {
                "beacon" => true,
                "connect" | "probe" => !ours,
                // another session's frames, heard by a station in none
                _ => idle,
            };
        let weak_caller = !report.decoded && report.detect_confidence >= NOTICE_CONFIDENCE;
        if anothers {
            scan.linger_until = None;
        } else if weak_caller || (report.decoded && report.kind == "probe" && ours) {
            let until = now + scan.linger_s;
            scan.linger_until = Some(scan.linger_until.map_or(until, |t| t.max(until)));
        }
    }

    /// Called every pass: move the radio when the visit is over and nothing holds the dial.
    pub(super) fn advance_scan(&mut self, now: f64) {
        if self.scanner.scan.is_none() {
            self.go_home(now);
            return;
        }
        self.restart_after_move(now);
        if self.knob_turned(now) {
            self.end_scan_staying(ScanEnd::Knob);
            return;
        }
        let held = self.scan_held(now);
        let contact = self.held_by_contact();
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        if held.is_some() {
            scan.held_by_contact |= contact;
            scan.leave_at = scan.leave_at.max(now);
            return;
        }
        if scan.held_by_contact {
            // a session, a call or a probe has ended here: its station may call again
            scan.held_by_contact = false;
            scan.linger_until = Some(now + scan.linger_s);
            scan.leave_at = scan.leave_at.max(now);
            return;
        }
        if !scan.placed {
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
        let next = (scan.at + 1) % scan.dials.len();
        self.move_scan(next, now);
    }

    /// What keeps the radio on its dial now, in a sentence, or `None` (rules 1 and 4).
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
        None
    }

    /// Whether what holds the dial is a contact this station may hear from again.
    fn held_by_contact(&self) -> bool {
        self.engine.state() != State::Idle || self.probing() || self.test_running()
    }

    /// Put the radio on the scan's dial `index` (rules 2 and 8).
    fn move_scan(&mut self, index: usize, now: f64) {
        let Some(scan) = self.scanner.scan.as_ref() else {
            return;
        };
        let Some(&dial) = scan.dials.get(index) else {
            return;
        };
        if let Err(error) = self.ptt.inner_mut().set_frequency_hz(dial) {
            let why = error.to_string();
            let Some(scan) = self.scanner.scan.as_mut() else {
                return;
            };
            scan.dials.remove(index);
            scan.dropped.push((dial, why.clone()));
            let left = scan.dials.len();
            if left < MIN_SCAN_DIALS {
                self.note(
                    "scan",
                    &format!("the radio would not go to {}: {why}", mhz(dial)),
                );
                self.end_scan(ScanEnd::Radio);
                return;
            }
            scan.at = index % left;
            scan.placed = false;
            scan.must_last_s = must_last(left, scan.dwell_s, scan.linger_s);
            self.note(
                "scan",
                &format!(
                    "the radio would not go to {}, which the scan leaves out: {why}",
                    mhz(dial)
                ),
            );
            return;
        }
        let deaf = move_deaf_s();
        // the receiver keeps nothing heard on the dial the radio has left: a call whose end it
        // was still settling would be answered on this one
        self.restart_receiver();
        self.passband.frame(now, now + deaf);
        self.read_dial(dial, now);
        self.frequency.next_ask_s = now + 2.0;
        self.refresh_ceiling(true);
        let Some(scan) = self.scanner.scan.as_mut() else {
            return;
        };
        scan.at = index;
        scan.placed = true;
        scan.moved_at = now;
        scan.hear_from = now + deaf;
        scan.restart_at = Some(now + deaf);
        scan.leave_at = now + scan.dwell_s;
        scan.linger_until = None;
        scan.arriving_until = f64::NEG_INFINITY;
        scan.hops += 1;
        self.record_event("hop", &format!("dial {dial} deaf {deaf:.3}"));
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
            self.restart_receiver();
            if let Some(scan) = self.scanner.scan.as_mut() {
                scan.restart_at = None;
            }
        }
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
    fn go_home(&mut self, now: f64) {
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
            Ok(()) => {
                self.restart_receiver();
                self.passband.frame(now, now + move_deaf_s());
            }
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

    /// A radio whose dial a test can turn at the knob, and which can refuse a dial.
    #[derive(Debug, Default)]
    struct Rig {
        dial: u64,
        refuses: Vec<u64>,
        set: Vec<u64>,
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
            if rig.refuses.contains(&hz) {
                return Err(PttError::Backend("the radio did not take it".into()));
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
        }
    }

    fn dwell(station: &Station<ScanRadio>) -> f64 {
        station.scan_status()["dwell_s"].as_f64().expect("dwell")
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
        // a round of the dials and two lingers: a try noticed, then two the scan stays for
        let rounded = (DIALS.len() as f64 * dwell + 2.0 * linger).ceil();
        assert_eq!(scan["scan_s"], json!(rounded));
    }

    #[test]
    fn the_radio_goes_round_the_dials_a_dwell_on_each() {
        let (mut station, rig) = scanning_station();
        station.scan_start(gateway(), &DIALS).expect("a scan");
        listen(&mut station, 0.2);
        // not on any of them: it goes to the home dial at once
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        let dwell = dwell(&station);
        listen(&mut station, dwell + 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[1]);
        listen(&mut station, dwell);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[2]);
        listen(&mut station, dwell);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[0]);
        assert_eq!(
            rig.lock().expect("rig").set,
            [DIALS[0], DIALS[1], DIALS[2], DIALS[0]]
        );
        assert_eq!(station.scan_status()["hops"], 4);
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
        station.scan_arriving(now + dwell + 2.0, NOTICE_CONFIDENCE - 0.05, now);
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
        listen(&mut station, dwell + 0.5);
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

    #[test]
    fn a_dial_the_radio_will_not_take_is_dropped() {
        let (mut station, rig) = scanning_station();
        rig.lock().expect("rig").refuses.push(DIALS[1]);
        station.scan_start(gateway(), &DIALS).expect("a scan");
        let dwell = dwell(&station);
        listen(&mut station, dwell + 0.5);
        assert_eq!(station.scan_status()["dials"], json!([DIALS[0], DIALS[2]]));
        assert_eq!(station.scan_status()["dropped"][0]["hz"], DIALS[1]);
        listen(&mut station, 0.3);
        assert_eq!(rig.lock().expect("rig").dial, DIALS[2]);
        // below two, the scan is over
        rig.lock().expect("rig").refuses.push(DIALS[0]);
        listen(&mut station, dwell + 0.5);
        assert!(!station.scanning());
        assert_eq!(station.scan_event()["why"], "radio");
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
        station.scan_arriving(now + 5.4, 1.6, now);
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
                meter: false,
                dial: true,
                settings: false,
            },
            meter: None,
            dial_hz: Some(DIALS[0]),
            settings: Vec::new(),
        });
        listen(&mut station, 0.1);
        assert_eq!(
            station.frequency.value,
            Some(DIALS[1]),
            "took a stale reading"
        );
    }
}
