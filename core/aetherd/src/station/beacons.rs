//! Beacons at an interval, and what became of every beacon (ND1J, 2026-09-25: "where does one
//! see beacons received — and sent — and how does one set a beacon to repeat every so many
//! minutes?").
//!
//! A repeating beacon is the Beacon button pressed on a timer, with manners the button leaves
//! to the operator: it waits for a clear channel whatever `wait_for_clear` says, it waits for
//! a session, a probe or a test to finish, and a beacon it could not send within
//! [`BEACON_WINDOW_S`] of falling due is skipped, said so, and the next one kept to the
//! interval — a busy evening does not end in a burst of catching up. Each one goes through the
//! same queue and regulatory gate as a single beacon.
//!
//! It needs a control operator: a station under automatic control may not start one while a
//! rules profile is in force (with the rules at *None*, or while the host program keys the
//! radio, the control operator answers for it: ADR-0024 §4). A
//! beacon (§97.3(a)(9)) may be automatically controlled only in the segments of §97.203(d) —
//! 28.20–28.30 MHz, 50.06–50.08 MHz (inside 6 m's CW-only 50.0–50.1 MHz, so never for
//! Aether's data), 144.275–144.300 MHz and a few above — and an unattended station has no
//! business announcing itself anywhere else. It lasts until it is stopped or the modem restarts: nothing on the disk sets
//! a station beaconing on its own the next time it starts.

use aether_link::frames::{DataHeader, DataKind, encode_data};
use aether_link::{BeaconBody, BeaconPosition, Container, TxFrame};
use serde_json::{Value, json};

use super::claims::{CLAIMED, NOT_HELD};
use super::scan::KNOB_TOLERANCE_HZ;
use super::{
    BEACON_ALREADY_WAITING, BEACON_NOT_OURS, BEACON_UNDER_AUTOMATIC_CONTROL, Origin, Outgoing,
    State, Station, is_beacon,
};
use crate::control::protocol::ConnectionId;
use crate::ptt::Ptt;

/// The shortest interval a repeating beacon may have. A beacon is a transmission nobody asked
/// for; ten minutes is the §97.119 identification interval, and more often than that a
/// frequency starts to belong to the beacon.
pub const BEACON_EVERY_MIN_S: f64 = 600.0;

/// The longest: past four hours a timer is a forgotten one.
pub const BEACON_EVERY_MAX_S: f64 = 4.0 * 3600.0;

/// How long a due beacon waits for the station to be idle and the channel clear before it is
/// skipped. Long enough to ride out an over or a probe; short enough that a beacon still goes
/// out near when it was due.
pub const BEACON_WINDOW_S: f64 = 120.0;

/// The longest a program's beacon may wait for a clear channel (ADR-0139; `RadioPost`'s RP-4
/// §4.4 `within_s`): ten minutes, the shortest repeating interval, so one never waits into the
/// next.
pub const BEACON_WITHIN_MAX_S: f64 = 600.0;

/// A program's beacon asked for a dial the station is not on and will not visit.
pub const BEACON_ELSEWHERE: &str = "that dial is neither the radio's nor one the scan visits";

/// What became of the beacons, kept until the run loop takes it.
const REPORTS_KEPT: usize = 32;

/// Where a beacon says its station is (ADR-0142).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum BeaconPlace {
    /// The station's own `[operator] grid`, when one is set: what the panel's beacon, the
    /// repeating one and a host program's carry.
    #[default]
    Station,
    /// Nowhere: the program's operator chose to say nothing of where the station is.
    Nowhere,
    /// The one a program gave, at the precision it chose.
    Given(BeaconPosition),
}

/// A beacon asked for (ADR-0139).
#[derive(Debug, Clone, Default)]
pub struct BeaconAsk {
    /// The callsign it carries: one the program holds, or for a connection that holds none
    /// (the panel, a host program before `MYCALL`) a name with one of the station's callsigns
    /// as its base. Not given, the program's first, or the station's own.
    pub callsign: Option<String>,
    /// The program's reference, carried back by `beacon-sent`.
    pub reference: Option<String>,
    /// How long it may wait, seconds: for a clear channel whatever `wait_for_clear` says, for a
    /// call or a session to end, and for the scan to reach its dial; it is dropped after. Not
    /// given, it waits as a beacon pressed on the panel does.
    pub within_s: Option<f64>,
    /// The dial it goes on: the radio's, or one of the scan's.
    pub dial_hz: Option<u64>,
    /// Where it says the station is.
    pub place: BeaconPlace,
}

/// What became of a beacon: the `beacon-sent` event (ADR-0139).
#[derive(Debug, Clone, PartialEq)]
pub struct BeaconReport {
    /// The program's reference, when it gave one.
    pub reference: Option<String>,
    /// The callsign the beacon carried, or would have.
    pub callsign: String,
    /// Whether it went on the air.
    pub sent: bool,
    /// The radio's dial it went on, when the station knows it; for one that did not go, the dial
    /// it was asked for.
    pub dial_hz: Option<u64>,
    /// Why not, when it did not.
    pub reason: Option<String>,
}

/// A beacon taken and not yet gone: at most one at a time.
#[derive(Debug, Clone)]
pub(super) struct QueuedBeacon {
    callsign: String,
    reference: Option<String>,
    /// Station time it is dropped at, when it has a window.
    until_s: Option<f64>,
    dial_hz: Option<u64>,
    position: Option<BeaconPosition>,
    /// Whether it is on the transmitter's queue yet.
    fed: bool,
}

/// The position a grid square names, when it is one (`[operator] grid`; an eight-character
/// locator is taken to six).
fn grid_position(grid: &str) -> Option<BeaconPosition> {
    let grid = grid.trim();
    let position = match grid.len() {
        4 => BeaconPosition::Grid4(grid.to_owned()),
        6 | 8 => BeaconPosition::Grid6(grid.get(..6)?.to_owned()),
        _ => return None,
    };
    // a square the frame cannot carry is said to nobody
    BeaconBody {
        callsign: "N0CALL".into(),
        bandwidth_hz: None,
        answers: None,
        position: Some(position.clone()),
    }
    .encode()
    .ok()
    .map(|_| position)
}

/// The repeating beacon's timer, and what the beacons did.
#[derive(Debug, Clone, Default)]
pub(super) struct Beacons {
    /// The interval, seconds, while a repeating beacon runs.
    every_s: Option<f64>,
    /// Station time the next one falls due.
    pub(super) next_s: f64,
    /// Wall-clock milliseconds when the last beacon went on the air, repeating or not.
    last_sent_ms: Option<u64>,
    /// Beacons that went on the air since the modem started.
    sent: usize,
    /// Due beacons skipped: the station was not idle or the channel not clear in time.
    pub(super) skipped: usize,
    /// The beacon taken and not gone yet.
    pub(super) queued: Option<QueuedBeacon>,
    /// What became of the beacons, for the run loop to publish.
    reports: Vec<BeaconReport>,
}

pub(super) fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl<P: Ptt> Station<P> {
    /// One beacon (ADR-0139): the callsign a program holds or the station's own, with the
    /// bandwidth the station runs, whether calls to that callsign are answered when it goes
    /// (ADR-0140) and where the station is (ADR-0142). A connection's beacon carries its own
    /// callsigns, as its calls do (ADR-0106); one with a window waits for a clear channel
    /// whatever `wait_for_clear` says and is dropped after; one with a dial goes there, taking a
    /// scan there as a datagram does (ADR-0129 §15). [`Self::take_beacon_reports`] says what
    /// became of it.
    ///
    /// # Errors
    /// On an answer-only station, under automatic control (§97.203(d)), if a session is up or a
    /// beacon already waits, for a callsign that is not the program's ([`CLAIMED`],
    /// [`NOT_HELD`]) or not this station's ([`BEACON_NOT_OURS`]), for a dial the station is not
    /// on and will not visit ([`BEACON_ELSEWHERE`]), or if the callsign cannot be packed.
    pub fn beacon_for(
        &mut self,
        connection: Option<ConnectionId>,
        ask: BeaconAsk,
    ) -> Result<(), &'static str> {
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and sends no beacon");
        }
        // A beacon may be automatically controlled only in the segments of §97.203(d), and
        // of those only 28.20–28.30 MHz is anywhere Aether's data may go. The repeating
        // beacon was refused under automatic control already; a host program's beacon timer
        // on an unattended station is the same thing by another road (ADR-0024). Under the
        // rules only: with none, the control operator answers for it (ADR-0024 §4).
        if self.beacons_barred() {
            return Err(BEACON_UNDER_AUTOMATIC_CONTROL);
        }
        if self.engine.state() != State::Idle {
            return Err("a session is running");
        }
        if self.beacon_waiting() {
            return Err(BEACON_ALREADY_WAITING);
        }
        let callsign = self.beacon_callsign(connection, ask.callsign.as_deref())?;
        if let Some(dial) = ask.dial_hz
            && !self.beacon_dial_reachable(dial)
        {
            return Err(BEACON_ELSEWHERE);
        }
        let position = match ask.place {
            BeaconPlace::Station => grid_position(&self.config.operator.grid),
            BeaconPlace::Nowhere => None,
            BeaconPlace::Given(position) => Some(position),
        };
        let queued = QueuedBeacon {
            callsign,
            reference: ask.reference,
            until_s: ask.within_s.map(|within| self.now() + within),
            dial_hz: ask.dial_hz,
            position,
            fed: false,
        };
        // built now, so a callsign that will not pack is refused at once; built again as it goes,
        // so that what it says of answering is said of that moment
        let frame = self.beacon_frame(&queued)?;
        // a beacon pressed on the panel goes on the transmitter's queue at once, as before, and
        // waits there as the operator's `wait_for_clear` says
        let at_once = queued.until_s.is_none() && !self.scan_goes_elsewhere_for(queued.dial_hz);
        self.beacons.queued = Some(queued);
        if at_once {
            self.push_beacon(frame);
        }
        self.stats.beacons_sent += 1;
        Ok(())
    }

    /// The callsign a beacon goes as: one the connection's program holds — or a name with one
    /// of them as its base, as `VarAC` names its beacons `KK4ODA-9` (ADR-0024) — or, for a
    /// connection that holds none, a name of this station's, or its own callsign.
    fn beacon_callsign(
        &self,
        connection: Option<ConnectionId>,
        name: Option<&str>,
    ) -> Result<String, &'static str> {
        let name = name
            .map(|n| n.trim().to_ascii_uppercase())
            .filter(|n| !n.is_empty());
        let base = |call: &str| call.split('-').next().unwrap_or_default().to_owned();
        if let Some(claim) = connection.and_then(|connection| self.claim_of(connection)) {
            let Some(call) = name else {
                return claim.callsigns.first().cloned().ok_or(NOT_HELD);
            };
            if self
                .claims
                .holder(&call)
                .is_some_and(|other| other.connection != claim.connection)
            {
                return Err(CLAIMED);
            }
            let wanted = base(&call);
            if claim.callsigns.contains(&call)
                || claim
                    .callsigns
                    .iter()
                    .any(|held| base(held).eq_ignore_ascii_case(&wanted))
            {
                return Ok(call);
            }
            // the name of another station: a host program's falls back to the plain beacon
            return Err(BEACON_NOT_OURS);
        }
        // the name a host program gave the beacon — VarAC's CQs and beacons are `KK4ODA-9`
        // and the like, meaning something to the program at the other end — is carried as
        // given when this station's callsign is its base (ADR-0024)
        match name {
            Some(name) if self.names_this_station(&name) => Ok(name),
            Some(_) => Err(BEACON_NOT_OURS),
            // the station's own, as its calls go: never the callsign of the last session,
            // which a program may have answered under (ADR-0108)
            None => Ok(self.station_callsign()),
        }
    }

    /// Whether the station can send on `dial`: the radio is there, or a scan visits it.
    fn beacon_dial_reachable(&mut self, dial: u64) -> bool {
        self.scan_index_of(dial).is_some()
            || self
                .frequency_hz()
                .is_some_and(|hz| hz.abs_diff(dial) <= KNOB_TOLERANCE_HZ)
    }

    /// The beacon frame for `queued`: its callsign, the bandwidth the station runs, whether
    /// calls to it are answered now, and its position, on the tone floor.
    fn beacon_frame(&self, queued: &QueuedBeacon) -> Result<TxFrame, &'static str> {
        let body = BeaconBody {
            callsign: queued.callsign.clone(),
            // which a host program told of the beacon reports beside it (`CQFRAME <call>
            // <bandwidth>`); a receiver of an earlier version reads the callsign and never looks
            // further
            bandwidth_hz: Some(self.air.info().bandwidth_hz),
            // a beacon that says it is answered tells a listener a call will be taken now; one
            // whose callsign nobody answers (VarAC's `-9`) says no (ADR-0140)
            answers: Some(self.answers_call_for(&queued.callsign).is_ok()),
            position: queued.position.clone(),
        }
        .encode()
        .map_err(|_| "the callsign will not pack")?;
        let header = DataHeader {
            kind: DataKind::Beacon,
            seq: 0,
            session: 0,
            piece: None,
        };
        // beacons go out on the tone floor, where calls and probes start (ADR-0016): the
        // slowest kind that carries a connect frame carries a callsign, and a beacon exists to
        // be heard by somebody who can hear nothing else — a station of either bandwidth, as
        // the floor's frames are the same on both airs
        let mode = self.engine.robust_mode(true);
        let capacity = self.engine.timing().capacity(mode);
        let payload =
            encode_data(&header, &body, capacity).map_err(|_| "the beacon will not fit")?;
        Ok(TxFrame {
            container: Container::Data,
            payload,
            mode,
            rv: 0,
            floor: false,
            follows: 0,
        })
    }

    /// Put the beacon on the transmitter's queue.
    fn push_beacon(&mut self, frame: TxFrame) {
        self.pending
            .push_back(Outgoing::Frames(vec![frame], Origin::station("beacon")));
        if let Some(queued) = self.beacons.queued.as_mut() {
            queued.fed = true;
        }
    }

    /// Hand a waiting beacon to the transmitter when it may go: the station idle and past the
    /// engine's closing hold, the radio on its dial, and, for one with a window, the channel
    /// clear whatever `wait_for_clear` says. Called from `start_pending` when nothing else is
    /// queued.
    pub(super) fn feed_beacon(&mut self, now: f64) {
        let Some(queued) = self.beacons.queued.as_ref().filter(|queued| !queued.fed) else {
            return;
        };
        let ready = self.pending.is_empty()
            && !self.transmitting
            && self.engine.state() == State::Idle
            && !self.test_running()
            && !self.probing()
            && now >= self.engine.hold_until()
            && !self.scan_goes_elsewhere_for(queued.dial_hz);
        let clear =
            queued.until_s.is_none() || (self.air.sense().settled() && !self.air.sense().busy(now));
        if !(ready && clear) {
            return;
        }
        match self.beacon_frame(queued) {
            Ok(frame) => self.push_beacon(frame),
            Err(why) => self.beacon_over(false, Some(why.to_owned())),
        }
    }

    /// The beacon as it goes now, said again for this moment: the frame on the queue was built
    /// when it was asked for, and whether its callsign is answered may have changed since.
    pub(super) fn beacon_now(&self) -> Option<TxFrame> {
        self.beacons
            .queued
            .as_ref()
            .and_then(|queued| self.beacon_frame(queued).ok())
    }

    /// Whether a beacon with a window must have the channel clear: it waits for one whatever
    /// `wait_for_clear` says (ADR-0139).
    pub(super) fn beacon_needs_clear(&self) -> bool {
        self.beacons
            .queued
            .as_ref()
            .is_some_and(|queued| queued.until_s.is_some())
    }

    /// The scan's dial a waiting beacon was asked for, when it is another than the one the radio
    /// is on: where the scan goes next.
    pub(super) fn beacon_dial_index(&self) -> Option<usize> {
        let dial = self.beacons.queued.as_ref()?.dial_hz;
        if !self.scan_goes_elsewhere_for(dial) {
            return None;
        }
        self.scan_index_of(dial?)
    }

    /// Whether a beacon asked for this very dial waits here: it holds the scan, as a datagram
    /// does. A beacon asked for no dial goes with the radio.
    pub(super) fn beacon_waiting_here(&self) -> bool {
        self.beacons.queued.as_ref().is_some_and(|queued| {
            queued.dial_hz.is_some() && !self.scan_goes_elsewhere_for(queued.dial_hz)
        })
    }

    /// The beacon's end, gone or not: reported for the run loop's `beacon-sent`.
    pub(super) fn beacon_over(&mut self, sent: bool, reason: Option<String>) {
        let Some(queued) = self.beacons.queued.take() else {
            return;
        };
        let dial_hz = if sent {
            self.frequency.value.or(queued.dial_hz)
        } else {
            queued.dial_hz
        };
        if self.beacons.reports.len() >= REPORTS_KEPT {
            self.beacons.reports.remove(0);
        }
        self.beacons.reports.push(BeaconReport {
            reference: queued.reference,
            callsign: queued.callsign,
            sent,
            dial_hz,
            reason,
        });
    }

    /// What became of the beacons since the last time this was asked.
    pub fn take_beacon_reports(&mut self) -> Vec<BeaconReport> {
        std::mem::take(&mut self.beacons.reports)
    }

    /// Every pass: a beacon whose window ran out is dropped and said; one taken off the
    /// transmitter's queue by anything but its own going is reported as not sent; one waiting
    /// when a session starts is dropped (ADR-0038).
    fn advance_queued_beacon(&mut self, now: f64) {
        let Some(queued) = self.beacons.queued.as_ref() else {
            return;
        };
        let (fed, until_s) = (queued.fed, queued.until_s);
        let on_queue = self
            .pending
            .iter()
            .any(|next| matches!(next, Outgoing::Frames(frames, _) if is_beacon(frames)));
        if fed && !on_queue {
            self.beacon_over(
                false,
                Some("taken off the queue before it went out".to_owned()),
            );
            return;
        }
        if self.engine.state() != State::Idle {
            self.drop_beacons_in_session();
            return;
        }
        if !until_s.is_some_and(|until| now > until) {
            return;
        }
        let dial = self
            .beacons
            .queued
            .as_ref()
            .and_then(|queued| queued.dial_hz);
        let why = if self.scan_goes_elsewhere_for(dial) {
            "the scan did not reach its dial in time"
        } else if self.air.sense().busy(now) || !self.air.sense().settled() {
            "the channel stayed busy"
        } else {
            "the station was busy"
        };
        if on_queue {
            self.pending
                .retain(|next| !matches!(next, Outgoing::Frames(frames, _) if is_beacon(frames)));
            self.held_since = None;
            self.held_from = None;
        }
        self.beacon_over(false, Some(why.to_owned()));
        self.note("beacon", &format!("not sent: {why}"));
    }

    /// Beacon every `seconds`, the first one now; `None` stops it.
    ///
    /// # Errors
    /// With the reason in a sentence for the operator: the interval out of range, an
    /// answer-only station, or one under automatic control.
    pub fn beacon_every(&mut self, seconds: Option<f64>) -> Result<(), String> {
        let Some(seconds) = seconds else {
            if self.beacons.every_s.take().is_some() {
                self.note("beacon", "repeating stopped");
            }
            return Ok(());
        };
        if !(BEACON_EVERY_MIN_S..=BEACON_EVERY_MAX_S).contains(&seconds) {
            return Err(format!(
                "a beacon repeats every {} to {} minutes",
                BEACON_EVERY_MIN_S / 60.0,
                BEACON_EVERY_MAX_S / 60.0
            ));
        }
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and sends no beacon".into());
        }
        if self.beacons_barred() {
            return Err(
                "a station under automatic control may not beacon on a timer: a beacon \
                        may be automatically controlled only on 28.20–28.30, 50.06–50.08, \
                        144.275–144.300, 222.05–222.06 or 432.300–432.400 MHz, or on 33 cm \
                        and up (§97.203(d))"
                    .into(),
            );
        }
        self.beacons.every_s = Some(seconds);
        self.beacons.next_s = self.now();
        self.note(
            "beacon",
            &format!("repeating every {} min", (seconds / 60.0).round()),
        );
        Ok(())
    }

    /// Send the repeating beacon when it is due, the station idle and the channel clear.
    pub(super) fn advance_beacons(&mut self, now: f64) {
        self.advance_queued_beacon(now);
        let Some(every) = self.beacons.every_s else {
            return;
        };
        if now < self.beacons.next_s {
            return;
        }
        // the rules may have changed under it: an answer-only or automatic station stops
        if self.config.answer_only || self.beacons_barred() {
            self.beacons.every_s = None;
            self.note(
                "beacon",
                "repeating stopped: the station is answer-only or under automatic control now",
            );
            return;
        }
        // and past the engine's closing hold, so the channel is judged once the other station's
        // closing identifier is over, not in the deafness after this station's own
        let idle = self.engine.state() == State::Idle
            && !self.test_running()
            && !self.probing()
            && self.pending.is_empty()
            && !self.transmitting
            && now >= self.engine.hold_until();
        let clear = self.air.sense().settled() && !self.air.sense().busy(now);
        if idle && clear {
            match self.beacon(None) {
                Ok(()) => self.beacons.next_s = now + every,
                Err(reason) => {
                    self.beacons.every_s = None;
                    self.note("beacon", &format!("repeating stopped: {reason}"));
                }
            }
            return;
        }
        if now - self.beacons.next_s >= BEACON_WINDOW_S {
            self.beacons.skipped += 1;
            // the next one keeps to the interval rather than catching up
            while self.beacons.next_s <= now {
                self.beacons.next_s += every;
            }
            let why = if idle {
                "the channel stayed busy"
            } else {
                "the station was busy"
            };
            self.note(
                "beacon",
                &format!(
                    "skipped: {why}; the next in {} min",
                    ((self.beacons.next_s - now) / 60.0).ceil()
                ),
            );
        }
    }

    /// A beacon went on the air: counted, timed and said.
    pub(super) fn beacon_sent(&mut self) {
        self.beacon_over(true, None);
        self.beacons.sent += 1;
        self.beacons.last_sent_ms = Some(unix_ms());
        // the timer's own next, which a beacon sent by hand in between does not move
        let detail = match self.beacons.every_s {
            Some(_) => format!(
                "sent; the next in {} min",
                ((self.beacons.next_s - self.now()) / 60.0).ceil().max(1.0)
            ),
            None => "sent".to_owned(),
        };
        self.note("beacon", &detail);
    }

    /// Whether a beacon is queued and has not gone yet — most often held for a busy channel.
    #[must_use]
    pub fn beacon_waiting(&self) -> bool {
        self.beacons.queued.is_some()
            || self
                .pending
                .iter()
                .any(|next| matches!(next, Outgoing::Frames(frames, _) if is_beacon(frames)))
    }

    /// The beacon's part of `status`.
    #[must_use]
    pub fn beacon_status(&self) -> Value {
        let now = self.now();
        let waiting = self.beacon_waiting();
        json!({
            // a beacon pressed for and not yet sent, and whether the channel is what holds it:
            // ND1J pressed twice on a busy channel and saw nothing either time (2026-09-29)
            "waiting": waiting,
            "waiting_for_clear": waiting && self.config.wait_for_clear && self.air.sense().busy(now),
            // the beacon taken and not gone: as what, for whom, where, and how long it may wait
            "queued": self.beacons.queued.as_ref().map(|queued| json!({
                "callsign": queued.callsign,
                "ref": queued.reference,
                "dial_hz": queued.dial_hz,
                "within_s": queued.until_s.map(|until| (until - now).max(0.0)),
            })),
            "every_s": self.beacons.every_s,
            "next_in_s": self.beacons.every_s.map(|_| (self.beacons.next_s - now).max(0.0)),
            "last_sent_ms": self.beacons.last_sent_ms,
            "sent": self.beacons.sent,
            "skipped": self.beacons.skipped,
        })
    }
}
