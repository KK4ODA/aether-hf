//! Who answers for which callsigns, and whose sessions are whose (ADR-0106).
//!
//! One station may serve several programs at once: RMS Trimode on the VARA-compatible host
//! port and a `RadioPost` gateway on the control API, on one radio and one sound card. A call
//! names a callsign, so the callsign decides whose call it is. A program **claims** the
//! callsigns it answers for; the station holds its own (the configuration's, and any the
//! panel adds) and answers them always.
//!
//! * One holder per callsign. A program's claim takes a callsign from the station's own list,
//!   as `MYCALL` always did; between two programs the first claim holds it, and a later claim
//!   of it is refused for that callsign. Callsigns are compared exactly.
//! * A claim starts with answering off, as VARA's `LISTEN` and the `listen` of `RadioPost`'s modem interface do, and
//!   ends with the connection that made it: a program that crashed stops being answered for.
//! * A call or probe to a callsign is answered only while its holder listens; the session it
//!   brings up belongs to that holder. A call a program places belongs to it. Everything else
//!   — the panel's calls, a Test — is the station's own.
//! * While a program holds a claim it speaks for the station, on either port (ADR-0108), as
//!   VARA's `MYCALL` replaced the modem's callsigns: the station's own callsigns no program
//!   holds are not answered, and come back when the last program goes.
//! * Under automatic control only a program answers calls (ADR-0107): a call to a callsign no
//!   program holds is not answered, because nobody is at the station to carry the session the
//!   call would bring up. Probes are still answered: they bring up nothing.

use serde_json::{Value, json};

use super::{State, Station};
use crate::control::protocol::{ConnectionId, ConnectionKind};
use crate::ptt::Ptt;

/// How long after one line about a claim's answering the next waits, unless nothing has
/// changed: a scanning RMS Trimode turns `LISTEN` off and on every three and a half seconds,
/// and a line at each turn was 9 448 lines of one night's log at KK4ODA-1 (2026-10-25).
const LISTEN_QUIET_S: f64 = 60.0;

/// One program's claim on callsigns.
#[derive(Debug, Clone, PartialEq)]
pub struct Claim {
    /// The connection that holds it, which is also its id.
    pub connection: ConnectionId,
    /// The program's name, for the panel and the log.
    pub app: String,
    /// The callsigns it answers to and calls as; the first is its default.
    pub callsigns: Vec<String>,
    /// Whether calls to them are answered.
    pub listening: bool,
    /// When it was made, milliseconds since the epoch.
    pub since_ms: u64,
    /// When the log last said whether its calls are answered, and the turns since.
    said_at: Option<f64>,
    turns: u32,
}

impl Claim {
    /// Why a call to one of its callsigns is not answered now.
    fn deaf_because(&self) -> String {
        match self.connection.kind {
            ConnectionKind::Host => "the host program has not said LISTEN ON".to_owned(),
            ConnectionKind::Control => format!("{} is not listening", self.app),
        }
    }

    /// What `status.claims` says of it.
    fn json(&self) -> Value {
        json!({
            "id": self.connection.id,
            "app": self.app,
            "kind": self.connection.kind.name(),
            "callsigns": self.callsigns,
            "listening": self.listening,
            "since_ms": self.since_ms,
        })
    }
}

/// What a claim came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    /// The callsigns the program holds now.
    pub callsigns: Vec<String>,
    /// Each callsign refused, and the program that holds it.
    pub refused: Vec<(String, String)>,
    /// Whether its calls are answered.
    pub listening: bool,
}

/// The station's callsigns and the programs' claims on them.
#[derive(Debug, Clone)]
pub(super) struct Claims {
    /// The station's own: the configuration's callsign, and those the panel set.
    own: Vec<String>,
    /// The programs' claims, in the order they were made.
    list: Vec<Claim>,
    /// Whose call or session is under way, or was the last: `None` is the station's own.
    owner: Option<ConnectionId>,
    /// Each callsign a program has held since the daemon started, and the program that held it
    /// last: a call to one nobody holds now is said to be that program's (ADR-0108).
    held_before: Vec<(String, String)>,
}

impl Claims {
    /// A station that answers to its configuration's callsign, and no program yet.
    pub(super) fn new(callsign: &str) -> Self {
        Self {
            own: vec![callsign.trim().to_ascii_uppercase()],
            list: Vec::new(),
            owner: None,
            held_before: Vec::new(),
        }
    }

    /// The claim that holds a callsign.
    fn holder(&self, call: &str) -> Option<&Claim> {
        self.list
            .iter()
            .find(|claim| claim.callsigns.iter().any(|held| held == call))
    }

    /// The program that last held a callsign nobody holds now and the station does not hold
    /// either: a call to it is that program's, gone (ADR-0108).
    fn gone_holder(&self, call: &str) -> Option<&str> {
        if self.holder(call).is_some() || self.own.iter().any(|own| own == call) {
            return None;
        }
        self.held_before
            .iter()
            .find(|(held, _)| held == call)
            .map(|(_, app)| app.as_str())
    }

    /// Every callsign the station answers to: its own first — the first of them is what it
    /// calls as when nobody says — then each claim's, once each.
    fn all(&self) -> Vec<String> {
        let mut all: Vec<String> = Vec::new();
        for call in self
            .own
            .iter()
            .chain(self.list.iter().flat_map(|claim| claim.callsigns.iter()))
        {
            if !all.contains(call) {
                all.push(call.clone());
            }
        }
        all
    }
}

/// Callsigns as the air carries them: trimmed, upper case, each one that fits, once each.
///
/// # Errors
/// If a callsign is one the air interface cannot carry.
fn cleaned(calls: &[String]) -> Result<Vec<String>, &'static str> {
    let mut out: Vec<String> = Vec::new();
    for call in calls {
        let call = call.trim().to_ascii_uppercase();
        if call.is_empty() {
            continue;
        }
        if aether_link::frames::pack_callsign(&call).is_err() {
            return Err("a callsign cannot be packed");
        }
        if !out.contains(&call) {
            out.push(call);
        }
    }
    Ok(out)
}

impl<P: Ptt> Station<P> {
    /// The station's own callsigns: the configuration's, and those the panel set.
    #[must_use]
    pub fn own_callsigns(&self) -> &[String] {
        &self.claims.own
    }

    /// The programs' claims.
    #[must_use]
    pub fn claims_json(&self) -> Value {
        Value::Array(self.claims.list.iter().map(Claim::json).collect())
    }

    /// The claim a connection holds.
    #[must_use]
    pub fn claim_of(&self, connection: ConnectionId) -> Option<&Claim> {
        self.claims
            .list
            .iter()
            .find(|claim| claim.connection == connection)
    }

    /// Whose call or session is under way: a program's connection, or `None` for the
    /// station's own — and, once it is over, whose it was, until the next.
    #[must_use]
    pub fn owner(&self) -> Option<ConnectionId> {
        self.claims.owner
    }

    /// Whether the call or session under way is a program's on the host interface: what debug
    /// mode records and sends (ADR-0050), and whose `CWID ON` it honours (ADR-0075 §2.7).
    pub(super) fn owned_by_host(&self) -> bool {
        self.claims
            .owner
            .is_some_and(|owner| owner.kind == ConnectionKind::Host)
    }

    /// Whether a callsign is held by the program on the host interface.
    pub(super) fn held_by_host(&self, call: &str) -> bool {
        self.claims
            .holder(&call.trim().to_ascii_uppercase())
            .is_some_and(|claim| claim.connection.kind == ConnectionKind::Host)
    }

    /// Set the station's own callsigns; the programs' claims stand.
    ///
    /// # Errors
    /// If the list is empty or a callsign is one the air interface cannot carry.
    pub(super) fn set_own_callsigns(&mut self, calls: &[String]) -> Result<bool, &'static str> {
        let own = cleaned(calls)?;
        if own.is_empty() {
            return Err("at least one callsign is needed");
        }
        let was = std::mem::replace(&mut self.claims.own, own);
        let answering = self.answering();
        match self.apply_callsigns() {
            Ok(applied) => {
                self.say_answering_change(answering);
                Ok(applied)
            }
            Err(reason) => {
                self.claims.own = was;
                Err(reason)
            }
        }
    }

    /// Claim callsigns for a connection, replacing what it claimed before; an empty list lets
    /// them all go. A callsign another program holds is refused and the rest are taken. A new
    /// claim starts with answering off.
    ///
    /// # Errors
    /// If a callsign is one the air interface cannot carry.
    pub fn claim(
        &mut self,
        connection: ConnectionId,
        app: Option<&str>,
        calls: &[String],
    ) -> Result<Claimed, &'static str> {
        let wanted = cleaned(calls)?;
        let answering = self.answering();
        let mut taken = Vec::new();
        let mut refused = Vec::new();
        for call in wanted {
            match self.claims.holder(&call) {
                Some(other) if other.connection != connection => {
                    refused.push((call, other.app.clone()));
                }
                _ => taken.push(call),
            }
        }
        let app = app
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let index = self
            .claims
            .list
            .iter()
            .position(|claim| claim.connection == connection);
        let listening = if let Some(index) = index {
            let claim = &mut self.claims.list[index];
            claim.callsigns.clone_from(&taken);
            if let Some(app) = app {
                claim.app = app;
            }
            claim.listening
        } else {
            let app = app.unwrap_or_else(|| {
                match connection.kind {
                    ConnectionKind::Host => "the host program",
                    ConnectionKind::Control => "a program",
                }
                .to_owned()
            });
            self.claims.list.push(Claim {
                connection,
                app,
                callsigns: taken.clone(),
                listening: false,
                since_ms: super::beacons::unix_ms(),
                said_at: None,
                turns: 0,
            });
            false
        };
        let app = self
            .claim_of(connection)
            .map_or_else(String::new, |claim| claim.app.clone());
        for call in &taken {
            match self
                .claims
                .held_before
                .iter_mut()
                .find(|(held, _)| held == call)
            {
                Some(entry) => entry.1.clone_from(&app),
                None => self.claims.held_before.push((call.clone(), app.clone())),
            }
        }
        // a claim of nothing is a program letting go: it has no say over any call now
        if calls.is_empty() {
            self.claims
                .list
                .retain(|claim| claim.connection != connection);
        }
        let _ = self.apply_callsigns();
        if calls.is_empty() {
            self.note("claims", &format!("{app} lets go of its callsigns"));
        } else {
            let mut said = if taken.is_empty() {
                format!("{app} holds no callsign")
            } else {
                format!("{app} answers for {}", taken.join(", "))
            };
            for (call, holder) in &refused {
                use std::fmt::Write as _;
                let _ = write!(said, "; {call} refused: {holder} holds it");
            }
            self.note("claims", &said);
        }
        self.say_answering_change(answering);
        Ok(Claimed {
            callsigns: taken,
            refused,
            listening,
        })
    }

    /// Turn answering on or off for a connection's claim. Whether it changed.
    ///
    /// # Errors
    /// If the connection holds no claim.
    pub fn listen_for(&mut self, connection: ConnectionId, on: bool) -> Result<bool, &'static str> {
        let now = self.now();
        let Some(claim) = self
            .claims
            .list
            .iter_mut()
            .find(|claim| claim.connection == connection)
        else {
            return Err("this connection holds no callsigns");
        };
        let changed = claim.listening != on;
        claim.listening = on;
        if changed {
            claim.turns += 1;
        }
        // what answering comes to, at once when it changes, then at most once a minute
        if claim.turns > 0 && claim.said_at.is_none_or(|at| now - at >= LISTEN_QUIET_S) {
            let turns = std::mem::take(&mut claim.turns);
            claim.said_at = Some(now);
            let calls = claim.callsigns.join(", ");
            let state = if on {
                format!("answering calls to {calls}")
            } else {
                format!("not answering calls to {calls}: {}", claim.deaf_because())
            };
            let detail = if turns > 1 {
                format!(
                    "{state} (LISTEN turned on and off {turns} times since the last line: the \
                     program scans)"
                )
            } else {
                state
            };
            self.note("listen", &detail);
        }
        Ok(changed)
    }

    /// A connection has closed: its claim goes, and the callsigns it held go back to the
    /// station when they are its own, or stop being answered. The call or session it owned
    /// goes with it (ADR-0107): a call stops, as the panel's *Stop calling* does, and a session
    /// closes as `disconnect` does — what is queued may still go, then the closing exchange and
    /// the identifiers. Left up, nobody would hear what the other station sends or send it
    /// anything: KK4ODA's session with KK4ODA-1 ran on five minutes after its `RadioPost` client
    /// had gone (2026-11-12), until the operator pressed Abort.
    pub fn connection_closed(&mut self, connection: ConnectionId) {
        self.connection_closed_because(connection, None);
    }

    /// [`Self::connection_closed`], with why the connection ended when it was not the
    /// program's own close (a read or write that failed, the modem stopping), said in the
    /// claim's line: two of KK4ODA's `RadioPost` sessions lost their program with nothing in
    /// either log to say how (2026-11-12).
    pub fn connection_closed_because(&mut self, connection: ConnectionId, why: Option<&str>) {
        let index = self
            .claims
            .list
            .iter()
            .position(|claim| claim.connection == connection);
        // a program that let its callsigns go still owns the session it brought up
        let owned = self.claims.owner == Some(connection);
        if index.is_none() && !owned {
            return;
        }
        let answering = self.answering();
        let (app, held) = match index.map(|index| self.claims.list.remove(index)) {
            Some(claim) => {
                let _ = self.apply_callsigns();
                (claim.app, claim.callsigns)
            }
            None => ("the program".to_owned(), Vec::new()),
        };
        let remote = self.engine.remote_call.clone();
        let (ended, end) = match self.engine.state() {
            State::Connecting if owned => (format!("; its call to {remote} stops"), Some(true)),
            State::Connected if owned && !self.engine.disconnect_requested() => (
                format!("; its session with {remote} closes, as Disconnect does"),
                Some(false),
            ),
            _ => (String::new(), None),
        };
        self.note(
            "claims",
            &format!(
                "{app} has gone{}: no longer answering for {}{ended}",
                why.map(|why| format!(" ({why})")).unwrap_or_default(),
                if held.is_empty() {
                    "anything".to_owned()
                } else {
                    held.join(", ")
                }
            ),
        );
        // said first: the end it sets in motion is said after it
        match end {
            Some(true) => self.abort(),
            Some(false) => self.disconnect(),
            None => {}
        }
        self.say_answering_change(answering);
    }

    /// Whether a call to this callsign is answered now, or why not.
    ///
    /// A callsign no program holds is the station's own. While a program holds a claim it
    /// speaks for the station, whichever port it is on (ADR-0108), as VARA's `MYCALL`
    /// replaced the modem's callsigns: the station's own callsigns no program holds are not
    /// answered. A gateway whose Setup says KK4ODA and whose Trimode says `MYCALL KK4ODA-1`
    /// answered nothing to KK4ODA before claims, and must not start answering it unattended
    /// now; a `RadioPost` gateway is a program the same way, so what Setup shows — the
    /// program's callsign — is what is answered.
    pub(super) fn answers_for(&self, call: &str) -> Result<(), String> {
        if self.host.stopping {
            return Err("the modem is stopping".to_owned());
        }
        match self.claims.holder(call) {
            Some(claim) if !claim.listening => Err(claim.deaf_because()),
            Some(_) => Ok(()),
            None => match self.claims.list.first() {
                None => Ok(()),
                Some(claim) if claim.connection.kind == ConnectionKind::Host => Err(
                    "the host program attached answers only for the callsigns its MYCALL names"
                        .to_owned(),
                ),
                Some(claim) => Err(format!(
                    "{} answers for this station while it runs, as {}",
                    claim.app,
                    if claim.callsigns.is_empty() {
                        "no callsign".to_owned()
                    } else {
                        claim.callsigns.join(", ")
                    }
                )),
            },
        }
    }

    /// The callsign the station is using now: the one its call or session runs under; when
    /// none is up, the first a program holds — a program speaks for the station (ADR-0108) —
    /// or else its own first. What the panel's header shows; never the callsign of a session
    /// that has ended.
    #[must_use]
    pub fn callsign_in_use(&self) -> String {
        if self.engine.state() != State::Idle {
            return self.engine.my_call.clone();
        }
        self.claims
            .list
            .iter()
            .find_map(|claim| claim.callsigns.first())
            .or_else(|| self.claims.own.first())
            .cloned()
            .unwrap_or_else(|| self.engine.my_call.clone())
    }

    /// The station's own callsign: the first of its own list, Setup's. What the station's own
    /// calls, probes and Tests go as when none is named, and so its beacons, KISS frames and
    /// the files it writes too — never the callsign of the last session (ADR-0108).
    #[must_use]
    pub fn station_callsign(&self) -> String {
        self.claims
            .own
            .first()
            .cloned()
            .unwrap_or_else(|| self.engine.my_call.clone())
    }

    /// The program that answered for `call` earlier in this run and no longer does, when the
    /// station does not answer to it either: a caller to it is that program's (ADR-0108).
    #[must_use]
    pub fn gone_holder(&self, call: &str) -> Option<String> {
        self.claims
            .gone_holder(&call.trim().to_ascii_uppercase())
            .map(str::to_owned)
    }

    /// Whether a call to this callsign is answered now, or why not: [`Self::answers_for`], and
    /// under automatic control only for a program (ADR-0107).
    ///
    /// The station's own sessions are the operator's (ADR-0106 §2.1), and under automatic
    /// control no operator is there to carry one. A gateway whose `RadioPost` program had lost
    /// Aether answered KK4ODA's call to KK4ODA-1 itself (2026-11-12): the caller's opening
    /// crossed in five minutes and reached nobody, and the session ran on until the caller's
    /// operator aborted it. A probe is not a call: it is answered as [`Self::answers_for`] says.
    pub(super) fn answers_call_for(&self, call: &str) -> Result<(), String> {
        self.answers_for(call)?;
        if self.claims.holder(call).is_none()
            && self.config.regulatory.control == Some(crate::regulatory::ControlMode::Automatic)
        {
            return Err(format!(
                "under automatic control only a program answers calls, and none holds {call}"
            ));
        }
        Ok(())
    }

    /// Whether calls to the station's first callsign are answered now: what the panel shows.
    #[must_use]
    pub fn answering(&self) -> bool {
        let first = self.claims.own.first().cloned().unwrap_or_default();
        self.answers_call_for(&first).is_ok()
    }

    /// Why calls to the station's first callsign are not answered now, for the panel; `None`
    /// while they are.
    #[must_use]
    pub fn not_answering_because(&self) -> Option<String> {
        let first = self.claims.own.first().cloned().unwrap_or_default();
        self.answers_call_for(&first).err()
    }

    /// Hand the engine every callsign the station answers to, now or at the session's end.
    fn apply_callsigns(&mut self) -> Result<bool, &'static str> {
        let all = self.claims.all();
        if self.engine.state() == State::Idle {
            self.engine.set_callsigns(&all)?;
            self.pending_callsigns = None;
            // the identifier sends the callsign: its length, and whether there is one, follow it
            self.refresh_identifier_length();
            self.note_required_identifier();
            return Ok(true);
        }
        self.pending_callsigns = Some(all);
        Ok(false)
    }

    /// Say it when the station's first callsign stops or starts being answered by a change
    /// that is not a program's `LISTEN` — a claim taking it, or letting it go, or the control
    /// the rules are set for.
    pub(super) fn say_answering_change(&mut self, was: bool) {
        let now = self.answering();
        if now == was {
            return;
        }
        let first = self.claims.own.first().cloned().unwrap_or_default();
        let detail = match self.answers_call_for(&first) {
            Ok(()) => format!("answering calls to {first}"),
            Err(why) => format!("not answering calls to {first}: {why}"),
        };
        self.note("listen", &detail);
    }

    /// Call `remote` for a connection: as one of its claim's callsigns (its first by default)
    /// when it holds one, and the call and the session it brings up are its own. A connection
    /// with no claim — the panel — calls for the station, as any callsign the station answers
    /// to.
    ///
    /// # Errors
    /// [`CLAIMED`] for a callsign another program holds, [`NOT_HELD`] for a callsign the
    /// claim does not hold, or what [`Station::connect_keeping`] refuses.
    pub fn connect_for(
        &mut self,
        connection: Option<ConnectionId>,
        remote: &str,
        as_call: Option<&str>,
        keep_bandwidth: bool,
    ) -> Result<(), &'static str> {
        let claim = connection.and_then(|connection| self.claim_of(connection).cloned());
        let Some(claim) = claim else {
            return self.connect_keeping(remote, as_call, keep_bandwidth);
        };
        let call = match as_call.map(|call| call.trim().to_ascii_uppercase()) {
            Some(call) => call,
            None => claim.callsigns.first().cloned().ok_or(NOT_HELD)?,
        };
        if !claim.callsigns.contains(&call) {
            return Err(
                if self
                    .claims
                    .holder(&call)
                    .is_some_and(|other| other.connection != claim.connection)
                {
                    CLAIMED
                } else {
                    NOT_HELD
                },
            );
        }
        self.connect_keeping(remote, Some(&call), keep_bandwidth)?;
        self.claims.owner = Some(claim.connection);
        Ok(())
    }

    /// The call just placed is the station's own; a program's sets itself after.
    pub(super) fn call_is_the_stations(&mut self) {
        self.claims.owner = None;
    }

    /// A session came up: one this station was called into is its called callsign's holder's.
    pub(super) fn note_owner(&mut self, called: bool) {
        if called {
            let call = self.engine.my_call.clone();
            self.claims.owner = self.claims.holder(&call).map(|claim| claim.connection);
        }
    }

    /// Whether a connection may act on what is under way — `send`, `disconnect`, `abort`: a
    /// program only on its own call or session; a connection with no claim (the panel, the
    /// control operator) on anything.
    #[must_use]
    pub fn may_act_on_session(&self, connection: Option<ConnectionId>) -> bool {
        let Some(connection) = connection else {
            return true;
        };
        if self.claim_of(connection).is_none() {
            return true;
        }
        self.engine.state() == State::Idle || self.claims.owner == Some(connection)
    }
}

/// A program called as a callsign another program holds (ADR-0106).
pub const CLAIMED: &str = "another program holds that callsign";

/// A program called as a callsign it does not hold.
pub const NOT_HELD: &str = "that callsign is not one this program holds";
