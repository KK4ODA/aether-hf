//! The bandwidth the station runs, and the moves between the two (ADR-0026).
//!
//! A station has a bandwidth of its own — `[radio] bandwidth`, what Setup saves — and runs it
//! unless one of two things moves it, both between sessions:
//!
//! * **A host program's `BW500`, `BW2300` or `BW2750`.** VARA's published commands set the
//!   modem's mode with them ("Set VARA HF to 500Hz Narrow mode"), and a program set up for
//!   VARA sends the one it wants — `VarAC` 500 Hz on its calling frequencies, Winlink Express
//!   its session's, Pat its configuration's — so an operator going from one program to the
//!   other no longer changes Setup and restarts the modem. A control-API program asks the
//!   same with `bandwidth.set`: `RadioPost` asks for 2 300 Hz while it listens for P2P calls,
//!   so that every call is answered whichever bandwidth it comes in. Each program's request
//!   holds while its connection lasts, the latest one in force; when the last goes, the
//!   station goes back to its own.
//! * **A call to this station in the narrower bandwidth.** A 2 300 Hz station hears a 500 Hz
//!   station's call — calls start on the tone floor (ADR-0016), whose frames are the same on
//!   both airs — and answers it at 500 Hz, as VARA's *Accept 500 Hz connections* does; once
//!   the session is over and the channel has been quiet for [`RETURN_QUIET_S`], it goes back.
//!   Never the other way: a 2 300 Hz call to a station running 500 Hz is not answered,
//!   because a 2 300 Hz signal where the operator or the program chose 500 Hz — a 500 Hz
//!   calling frequency, most of all — is nobody's choice.
//! * **A call from this station to one it knows runs 500 Hz** (ADR-0035). A 2 300 Hz station
//!   that has heard the other's bandwidth — from a beacon (ADR-0024), a call, a probe or a
//!   probe's answer, each of which states it — moves to 500 Hz before it calls, and back as
//!   after a call it answered. The station heard does the same the other way round on its own
//!   (the first point), but a station of an earlier version does not.
//!
//! The one crossing that cannot be made is a 2 300 Hz call to a station running 500 Hz. A
//! probe is answered across bandwidths (ADR-0035), so each side learns what the other runs,
//! and a call or probe to this station that it could not answer as a call is kept
//! ([`Mismatch`]) for the panel to say — with both bandwidths and the fix — instead of only a
//! log line nobody reads while waiting for an answer.
//!
//! A move rebuilds what depends on the waveform — the receiver, the modems, the band filters,
//! the busy detector, the occupancy the rules judge by, the link engine's air (its timing,
//! ladder and handshake bandwidth, [`GrantedEngine::set_air`](aether_link::GrantedEngine::set_air))
//! — and keeps the rest: the callsigns, the counters, the identifier's clock, the stations
//! heard, the KISS datagrams not yet framed. It is made only when nothing is under way that
//! runs on the air it would leave: no session, call or probe, no Test, nothing queued or on
//! the air.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::{Value, json};

use super::{State, Station};
use crate::control::protocol::ConnectionId;
use crate::ptt::Ptt;

/// How long the channel stays quiet, after a session in the bandwidth a call brought, before
/// the station goes back to its own: time for the other station's goodbye — its identifier, a
/// disconnect repeated on the tone floor, three tries of 3.2 s each and their turnarounds —
/// and for the call that often follows a ping.
pub const RETURN_QUIET_S: f64 = 20.0;

/// A callsign without what an operator adds to it: the SSID and anything after it (`-9`,
/// `VarAC`'s `-1-T`), and a portable prefix or suffix (`VE3/`, `/P`) — the longest part
/// between slashes, which is the licensed call.
#[must_use]
pub fn base_callsign(callsign: &str) -> &str {
    let call = callsign.trim();
    let call = call.split('-').next().unwrap_or(call);
    call.split('/')
        .max_by_key(|part| part.len())
        .unwrap_or(call)
}

/// How many other stations' bandwidths the station remembers — the stations-heard list's
/// bound: the daemon seeds these from it at start, and the frames heard since keep them.
pub const KNOWN_LIMIT: usize = crate::heard::LIMIT;

/// A call or probe to this station in a bandwidth whose calls it does not answer (ADR-0035):
/// what the panel warns of, until the station runs the other's bandwidth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mismatch {
    /// Who called or probed.
    pub callsign: String,
    /// `call` or `probe`.
    pub what: &'static str,
    /// The bandwidth the frame stated, hertz.
    pub theirs_hz: usize,
    /// The bandwidth this station ran when it heard it.
    pub ours_hz: usize,
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The sentence the log and the panel give: who, both bandwidths, and what to do.
    pub sentence: String,
    /// For `what` = `version`: the link protocol the other station's frame stated, and this
    /// station's — stations of different protocols cannot connect (ADR-0041). The bandwidths
    /// are then both this station's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theirs_protocol: Option<u8>,
    /// See [`Mismatch::theirs_protocol`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ours_protocol: Option<u8>,
    /// For `what` = `callsign`: the callsign the other station called — one of this station's
    /// with another SSID or none, which the panel offers to answer to as well. For `what` =
    /// `suggest`: the callsign heard lately under the base of one this station called and got
    /// no answer from, which the panel offers to call (ADR-0057). For `what` = `program`: the
    /// callsign called, which a program answered for earlier and no longer does (ADR-0108).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub called: Option<String>,
}

/// How recently a station must have been heard for a call to another of its operator's
/// callsigns, unanswered, to suggest it (ADR-0057).
pub const KIN_HEARD_S: f64 = 30.0 * 60.0;

/// How lately a station must have been read for a capped call to it to go by how well it was
/// (ADR-0126 §2.3): longer ago, the call is made as to a station not heard.
pub const CALL_HEARD_S: f64 = 15.0 * 60.0;

/// Unanswered tries at 2 300 Hz before a call to a station whose bandwidth is not known goes on
/// at 500 Hz (ADR-0058), when the call makes more than this (the tries setting at 6 or more:
/// ADR-0126 §9).
pub const NARROW_AFTER_TRIES: usize = 4;

/// How long before its next try a call is carried on to the narrow air (ADR-0058): a few of the
/// run loop's passes.
const MOVE_BEFORE_TRY_S: f64 = 0.25;

/// The waveform of a bandwidth this version runs.
#[must_use]
pub fn params_for(hz: usize) -> Option<aether_phy::waveform::WaveformParams> {
    match hz {
        2300 => Some(aether_phy::waveform::WIDE_2300),
        500 => Some(aether_phy::waveform::NARROW_500),
        _ => None,
    }
}

/// Why the station runs the bandwidth it runs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Why {
    /// Its own: `[radio] bandwidth`.
    #[default]
    Configured,
    /// A program asked for it: a host program's `BW<n>`, or `RadioPost` for its call.
    Host,
    /// A program listening for calls asked for it: `RadioPost` listens in 2 300 Hz, which
    /// answers calls in either bandwidth.
    Listening,
    /// A call to this station came in it; the callsign that called.
    Call(String),
    /// This station called one it knows runs it (ADR-0035); the callsign called.
    Calling(String),
    /// This station's wider call went unanswered, and goes on in it (ADR-0058); the callsign
    /// called.
    Unanswered(String),
}

impl Why {
    fn name(&self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Host => "host",
            Self::Listening => "listening",
            Self::Call(_) => "call",
            Self::Calling(_) | Self::Unanswered(_) => "calling",
        }
    }
}

/// The bandwidth the station should run, and why it runs what it does.
#[derive(Debug, Clone)]
pub(super) struct Bandwidth {
    /// The station's own: `[radio] bandwidth`.
    pub(super) home_hz: usize,
    /// What each program asked for with `bandwidth.set`, the latest last. The latest holds — a
    /// listening request only when no other holds — and a request goes when its connection
    /// closes, so a program that crashes leaves the station as it found it.
    requests: Vec<Request>,
    /// Why the station runs what it runs now.
    why: Why,
    /// Station time the station was last anything but idle and quiet while it runs another
    /// bandwidth than it should: the move back waits [`RETURN_QUIET_S`] after it.
    stirred_s: f64,
    /// Moves made since the modem started.
    moves: usize,
    /// Other stations' bandwidths as their frames stated them, the most recently heard last,
    /// with the station time it was stated — minus infinity for what the stations-heard list
    /// kept from an earlier run.
    known: VecDeque<(String, usize, f64)>,
    /// The last call or probe to this station in a bandwidth whose calls it does not answer.
    mismatch: Option<Mismatch>,
    /// The callsigns frames have named as their sender, with the station time each was last
    /// heard and the SNR it was read at, the most recently heard last: what a call nobody
    /// answered is checked against (ADR-0057), and what chooses a capped call's tries (ADR-0126).
    heard_at: VecDeque<(String, f64, f64)>,
}

impl Bandwidth {
    pub(super) fn new(home_hz: usize) -> Self {
        Self {
            home_hz,
            requests: Vec::new(),
            why: Why::Configured,
            stirred_s: f64::NEG_INFINITY,
            moves: 0,
            known: VecDeque::new(),
            mismatch: None,
            heard_at: VecDeque::new(),
        }
    }

    /// The bandwidth the station should run when nothing holds it where it is.
    fn target_hz(&self) -> usize {
        self.host_hz().unwrap_or(self.home_hz)
    }

    /// The bandwidth of the request in force, if one is.
    fn host_hz(&self) -> Option<usize> {
        self.in_force().map(|request| request.hz)
    }

    /// The request in force: the latest a program still connected made for its calls or its
    /// mode, else the latest made for listening. A program's `BW500` on `VarAC`'s calling
    /// frequency is its choice, and outranks `RadioPost` listening in 2 300 Hz on the same radio.
    fn in_force(&self) -> Option<&Request> {
        self.requests
            .iter()
            .rev()
            .find(|request| !request.listening)
            .or_else(|| self.requests.last())
    }

    /// Why a station that runs its target runs it: the request in force, or its own.
    fn why_for_target(&self) -> Why {
        match self.in_force() {
            Some(request) if request.listening => Why::Listening,
            Some(_) => Why::Host,
            None => Why::Configured,
        }
    }

    /// Withdraw what `connection` asked for: whether it had asked anything.
    fn withdraw(&mut self, connection: Option<ConnectionId>) -> bool {
        let before = self.requests.len();
        self.requests.retain(|request| request.by != connection);
        self.requests.len() != before
    }
}

/// A program's `bandwidth.set`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    /// The connection it came on, `None` for one that came without one (a one-shot `POST`).
    by: Option<ConnectionId>,
    /// The bandwidth asked for.
    hz: usize,
    /// Whether it is for listening: held below the others, and the station moves to it once
    /// idle, never refused.
    listening: bool,
}

impl<P: Ptt> Station<P> {
    /// The bandwidth the station runs now, in hertz.
    #[must_use]
    pub fn bandwidth_hz(&self) -> usize {
        self.air.info().bandwidth_hz
    }

    /// A request made without a connection ([`Self::host_bandwidth_for`] with `None`): a
    /// one-shot `POST`, and the tests.
    ///
    /// # Errors
    /// As [`Self::host_bandwidth_for`].
    pub fn host_bandwidth(&mut self, hz: Option<usize>) -> Result<usize, String> {
        self.host_bandwidth_for(None, hz, false)
    }

    /// A program's `bandwidth.set` — a host program's `BW<n>` through the VARA-compatible
    /// adapter, or `RadioPost` listening: run `hz` from now on while the program's connection
    /// lasts — or, with `None`, withdraw what it asked for, and run the request before it or
    /// the station's own. 2 750 Hz, which Winlink Express asks for unless told otherwise, is
    /// 2 300 Hz: a narrower signal is always inside what was asked.
    ///
    /// The move is made now or not at all: a program that asked for 500 Hz and was told `OK`
    /// calls at once, and a move that waited for a session to end would have it call in the
    /// wrong bandwidth. Withdrawing a request always succeeds; the move back waits for the
    /// station to be idle.
    ///
    /// A request for `listening` is the bandwidth a program wants calls answered in while it
    /// listens: never refused, held below the other requests, and the station moves to it now
    /// if it can, or once idle (`RadioPost`, which listens in 2 300 Hz so that a call in either
    /// bandwidth is answered: a profile had left its station at 500 Hz, and WD5EMA's 2 300 Hz
    /// call went unanswered, 2026-11-23).
    ///
    /// # Errors
    /// With the reason in a sentence: a bandwidth this version does not run, or something under
    /// way that runs on the air the station would leave.
    pub fn host_bandwidth_for(
        &mut self,
        connection: Option<ConnectionId>,
        hz: Option<usize>,
        listening: bool,
    ) -> Result<usize, String> {
        let Some(hz) = hz else {
            self.bandwidth.withdraw(connection);
            return Ok(self.bandwidth_hz());
        };
        let hz = if hz == 2750 { 2300 } else { hz };
        if params_for(hz).is_none() {
            return Err(format!("this modem runs 2300 or 500 Hz, not {hz}"));
        }
        let request = Request {
            by: connection,
            hz,
            listening,
        };
        if listening {
            self.bandwidth.withdraw(connection);
            self.bandwidth.requests.push(request);
            // what a program's own request holds stays held; this one waits under it
            if self.bandwidth.target_hz() == hz {
                if hz == self.bandwidth_hz() {
                    self.bandwidth.why = Why::Listening;
                } else if self.movable().is_ok() {
                    self.move_to(hz, Why::Listening);
                }
            }
            return Ok(self.bandwidth_hz());
        }
        if hz != self.bandwidth_hz() {
            self.movable()?;
            self.move_to(hz, Why::Host);
        } else if self.bandwidth.why != Why::Host {
            self.bandwidth.why = Why::Host;
        }
        self.bandwidth.withdraw(connection);
        self.bandwidth.requests.push(request);
        Ok(hz)
    }

    /// A connection has closed: what it asked for goes, and the station goes back to the
    /// request before it, or its own, once idle.
    pub(super) fn bandwidth_connection_closed(&mut self, connection: ConnectionId) {
        if self.bandwidth.withdraw(Some(connection)) {
            let target = self.bandwidth.target_hz();
            if target != self.bandwidth_hz() {
                self.note(
                    "bandwidth",
                    &format!(
                        "the program that asked for {} Hz has gone: back to {target} Hz once idle",
                        self.bandwidth_hz()
                    ),
                );
            }
        }
    }

    /// The bandwidth, as `status.bandwidth` reports it: what runs, the station's own, what a
    /// program asked for and which, whether for listening, and why.
    #[must_use]
    pub fn bandwidth_status(&self) -> Value {
        let request = self.bandwidth.in_force();
        json!({
            "bandwidth_hz": self.bandwidth_hz(),
            "home_hz": self.bandwidth.home_hz,
            "host_hz": request.map(|request| request.hz),
            "listening": request.is_some_and(|request| request.listening),
            "by": request
                .and_then(|request| request.by)
                .and_then(|by| self.program_name(by)),
            "why": self.bandwidth.why.name(),
            "caller": match &self.bandwidth.why {
                Why::Call(call) => Some(call.as_str()),
                _ => None,
            },
            "callee": match &self.bandwidth.why {
                Why::Calling(call) | Why::Unanswered(call) => Some(call.as_str()),
                _ => None,
            },
            "moves": self.bandwidth.moves,
            "mismatch": self.mismatch(),
        })
    }

    /// The last call or probe to this station in a bandwidth whose calls it does not answer,
    /// while it still runs the bandwidth it heard it in: one the operator has since moved to
    /// the other's bandwidth for is answered.
    #[must_use]
    pub fn mismatch(&self) -> Option<&Mismatch> {
        self.bandwidth
            .mismatch
            .as_ref()
            .filter(|m| m.ours_hz == self.bandwidth_hz())
    }

    /// Remember the bandwidth a station runs, as the stations-heard list kept it from an
    /// earlier run — which is how a restarted daemon still knows — with no time it was heard.
    pub fn learn_bandwidth(&mut self, callsign: &str, hz: usize) {
        self.heard_bandwidth(callsign, hz, f64::NEG_INFINITY);
    }

    /// Remember the bandwidth a station runs, as one of its frames stated it at station time
    /// `now`.
    pub(super) fn heard_bandwidth(&mut self, callsign: &str, hz: usize, now: f64) {
        let callsign = callsign.trim().to_ascii_uppercase();
        let known = &mut self.bandwidth.known;
        known.retain(|(call, _, _)| *call != callsign);
        known.push_back((callsign, hz, now));
        while known.len() > KNOWN_LIMIT {
            known.pop_front();
        }
    }

    /// Note that a frame named `callsign` as its sender at station time `now`, read at
    /// `snr_db`.
    pub(super) fn note_sender(&mut self, callsign: &str, now: f64, snr_db: f64) {
        let callsign = callsign.trim().to_ascii_uppercase();
        let heard = &mut self.bandwidth.heard_at;
        heard.retain(|(call, _, _)| *call != callsign);
        heard.push_back((callsign, now, snr_db));
        while heard.len() > KNOWN_LIMIT {
            heard.pop_front();
        }
    }

    /// The SNR the last frame read from `callsign` had, if it was read within
    /// [`CALL_HEARD_S`] of station time `now`: what a capped call's tries are chosen by
    /// (ADR-0126 §2.3).
    #[must_use]
    pub fn heard_lately(&self, callsign: &str, now: f64) -> Option<f64> {
        let callsign = callsign.trim().to_ascii_uppercase();
        self.bandwidth
            .heard_at
            .iter()
            .rev()
            .find(|(call, _, _)| *call == callsign)
            .filter(|(_, at, _)| now - at <= CALL_HEARD_S)
            .map(|(_, _, snr)| *snr)
    }

    /// Whether `callsign` is another name of one of this station's: the same base callsign
    /// ([`base_callsign`]) with another SSID or none — KO4WX called KK4ODA, and the station
    /// answered as KK4ODA-1 only (2026-10-10).
    fn kin_of_ours(&self, callsign: &str) -> bool {
        let base = base_callsign(callsign).to_ascii_uppercase();
        !self
            .engine
            .callsigns
            .iter()
            .any(|ours| ours.eq_ignore_ascii_case(callsign))
            && self
                .engine
                .callsigns
                .iter()
                .any(|ours| base_callsign(ours).eq_ignore_ascii_case(&base))
    }

    /// A call this station made went unanswered: if a station under the same base callsign as
    /// the one called, but another SSID, has been heard within [`KIN_HEARD_S`], and the one
    /// called has not been heard since, the panel suggests calling that one (ADR-0057). Calling
    /// KK4ODA when only KK4ODA-1 listens is the mistake KO4WX made the other way round.
    pub(super) fn suggest_kin(&mut self, called: &str, now: f64) {
        let called = called.trim().to_ascii_uppercase();
        let base = base_callsign(&called).to_owned();
        let Some((kin, at)) = self
            .bandwidth
            .heard_at
            .iter()
            .rev()
            .find(|(call, at, _)| {
                *call != called && base_callsign(call) == base && now - at <= KIN_HEARD_S
            })
            .map(|(call, at, _)| (call.clone(), *at))
        else {
            return;
        };
        // the station called was heard after its kin: it is there, and the advice would send
        // the operator elsewhere — WD5EMA's call to KK4ODA, a minute after a session with it,
        // was told to call the gateway KK4ODA-1 (2026-11-23)
        if self
            .bandwidth
            .heard_at
            .iter()
            .any(|(call, heard, _)| *call == called && *heard >= at)
        {
            return;
        }
        let minutes = ((now - at) / 60.0).round();
        let when = if minutes < 1.0 {
            "just now".to_owned()
        } else if minutes < 1.5 {
            "a minute ago".to_owned()
        } else {
            format!("{minutes:.0} minutes ago")
        };
        let sentence = format!(
            "No answer from {called}. {kin} was heard {when}: if that is the station you meant, \
             call {kin}."
        );
        self.note("mismatch", &sentence);
        let hz = self.bandwidth_hz();
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: called,
            what: "suggest",
            theirs_hz: hz,
            ours_hz: hz,
            at_ms: super::beacons::unix_ms(),
            sentence,
            theirs_protocol: None,
            ours_protocol: None,
            called: Some(kin),
        });
    }

    /// The bandwidth a station was last heard to run, if it has been.
    ///
    /// The callsign it was heard under first. Failing that, the other names of the same
    /// operator: `VarAC` beacons as `KK4ODA-9` and pings `KK4ODA-1-T`, and a station may be
    /// heard beaconing under one SSID and called under another — so every station heard under
    /// the same base callsign ([`base_callsign`]) counts, but only when they all agree. One
    /// operator running two stations in different bandwidths is not a guess to make: the
    /// call then goes out in the station's own, as it would unknown.
    #[must_use]
    pub fn known_bandwidth(&self, callsign: &str) -> Option<usize> {
        self.bandwidth_stated_since(callsign, f64::NEG_INFINITY)
    }

    /// [`Self::known_bandwidth`], from what frames stated at station time `since` or later.
    fn bandwidth_stated_since(&self, callsign: &str, since: f64) -> Option<usize> {
        let callsign = callsign.trim().to_ascii_uppercase();
        let known = || {
            self.bandwidth
                .known
                .iter()
                .filter(move |(_, _, at)| *at >= since)
        };
        if let Some(&(_, hz, _)) = known().rev().find(|(call, _, _)| *call == callsign) {
            return Some(hz);
        }
        let base = base_callsign(&callsign);
        let mut kin = known()
            .filter(|(call, _, _)| base_callsign(call) == base)
            .map(|&(_, hz, _)| hz);
        let first = kin.next()?;
        kin.all(|hz| hz == first).then_some(first)
    }

    /// Whether a 500 Hz call to `remote` keeps its quick tries: only when the station was heard
    /// running 500 Hz within [`CALL_HEARD_S`] of station time `now`. The quick tries are 500 Hz
    /// OFDM, which a 2 300 Hz station does not read at all — it reads a 500 Hz call's tone part
    /// only (ADR-0126 §8) — and a listener runs 2 300 Hz to answer both (`RadioPost`): WD5EMA's
    /// 500 Hz call to KK4ODA, a few minutes after a 2 300 Hz session with it, went as three
    /// quick tries the station could not read and one it could (2026-11-23).
    pub(super) fn quick_tries_heard(&self, remote: &str, now: f64) -> bool {
        self.bandwidth_hz() != 500
            || self.bandwidth_stated_since(remote, now - CALL_HEARD_S) == Some(500)
    }

    /// A call that keeps the bandwidth its host program chose (ADR-0102), to a station
    /// remembered at another: said in the log, so the operator knows why the call is not where
    /// the memory would have put it, and that four unanswered tries move it.
    pub(super) fn note_kept_bandwidth(&mut self, remote: &str) {
        let ours = self.bandwidth_hz();
        if let Some(theirs) = self.known_bandwidth(remote)
            && theirs != ours
        {
            let remote = remote.trim().to_ascii_uppercase();
            let after = match self.narrow_after_tries() {
                Some(tries) if ours == 2300 => {
                    format!("; at 500 Hz after {tries} unanswered tries")
                }
                _ => String::new(),
            };
            self.note(
                "bandwidth",
                &format!(
                    "calling {remote} at {ours} Hz, as the program placing the call asked, though \
                     it was last heard running {theirs} Hz{after}"
                ),
            );
        }
    }

    /// Before a call: a 2 300 Hz station calling one it knows runs 500 Hz moves to 500 Hz, so
    /// the call is one the other answers whatever its version (ADR-0035). The move back is a
    /// call's: after the session, or the call's end, and a quiet spell.
    ///
    /// A Test's own call moves too. Its probe has just learned the other's bandwidth, and
    /// between the probe and the call nothing is on the air; refusing because a Test runs,
    /// as every other move does, sent WC4Y's Test of 2026-10-24 calling at 2 300 Hz a station
    /// its probe had found at 500 Hz, three tries ignored (report 17 §5).
    pub(super) fn move_for_call(&mut self, remote: &str) {
        let may = if self.test_stepping {
            self.movable_but_for_a_test()
        } else {
            self.movable()
        };
        if self.bandwidth_hz() == 2300 && self.known_bandwidth(remote) == Some(500) && may.is_ok() {
            self.move_to(500, Why::Calling(remote.trim().to_ascii_uppercase()));
        }
    }

    /// A call or a probe to this station, while idle, in a bandwidth whose calls it does not
    /// answer: kept for the panel and said in the log, with both bandwidths and the fix. A
    /// 2 300 Hz station answers 500 Hz calls by moving (the narrower call), so what is left is
    /// a wider call — or probe, which is answered, but warns that a call would not be.
    pub(super) fn note_mismatch(&mut self, decoded: &crate::air::HeardFrame) {
        use aether_link::frames::{ConnectBody, DataKind, ProbeBody, bandwidth_hz_of, decode_data};
        if self.engine.state() != State::Idle || decoded.control {
            return;
        }
        let Some((header, body)) = decoded
            .payload
            .as_ref()
            .and_then(|payload| decode_data(payload).ok())
        else {
            return;
        };
        let (what, src, dst, caps) = match header.kind {
            DataKind::ConnectReq => match ConnectBody::decode(&body) {
                Ok(call) => ("call", call.src, call.dst, call.caps),
                Err(_) => return,
            },
            DataKind::Probe => match ProbeBody::decode(&body) {
                Ok(probe) => ("probe", probe.src, probe.dst, probe.caps),
                Err(_) => return,
            },
            _ => return,
        };
        let ours = self.bandwidth_hz();
        let Some(theirs) = bandwidth_hz_of(caps) else {
            return;
        };
        let theirs = if theirs == 2750 { 2300 } else { theirs };
        // a call or probe to another name of this station's — its base callsign, another SSID
        // — is warned of too (ADR-0057): KO4WX called KK4ODA, at 2 300 Hz, of a station that
        // answers as KK4ODA-1 at 500 Hz, and nothing said either
        // a call to a callsign a program answered for and no longer does: the program is the
        // fix, not answering as that callsign from here (ADR-0108)
        if let Some(app) = self.gone_holder(&dst) {
            self.note_program_gone(&src, &dst, what, &app, (theirs, ours));
            return;
        }
        let kin = self.kin_of_ours(&dst);
        if !kin && !self.engine.callsigns.contains(&dst) {
            return;
        }
        // a call the external program would answer, with none attached: the program is the fix,
        // not another callsign (report 22 §2: the advice named KK4ODA-1 while home's Winlink
        // Express, which answers for KK4ODA, was not attached)
        if what == "call" && !self.host_attached() {
            if let Some(port) = self.config.external_port {
                if kin || self.config.keyed_by_host {
                    self.note_no_program(&src, &dst, kin, port, (theirs, ours));
                    return;
                }
            }
        }
        let bandwidths_agree = theirs == ours || (ours == 2300 && theirs == 500);
        if bandwidths_agree && !kin {
            return;
        }
        let what = if bandwidths_agree { "callsign" } else { what };
        let now_ms = super::beacons::unix_ms();
        // a caller tries again and again: one warning a minute per station and kind is plenty
        if self.bandwidth.mismatch.as_ref().is_some_and(|m| {
            m.callsign == src && m.what == what && now_ms.saturating_sub(m.at_ms) < 60_000
        }) {
            return;
        }
        if bandwidths_agree {
            self.note_kin_call(src, dst, (theirs, ours), now_ms);
            return;
        }
        let answering = self.engine.callsigns.join(", ");
        let happened = if kin {
            format!(
                "{src} {} {dst} at {theirs} Hz — this station answers as {answering}, and runs \
                 {ours} Hz.",
                if what == "call" { "called" } else { "probed" }
            )
        } else if what == "call" {
            format!(
                "{src} called this station at {theirs} Hz and was not answered: it runs {ours} Hz."
            )
        } else {
            format!(
                "{src} probed this station at {theirs} Hz and was answered, but a call from {src} \
                 would not be: this station runs {ours} Hz."
            )
        };
        let fix = if self.bandwidth.why == Why::Host {
            format!(
                "An external program asked for {ours} Hz: have it ask for {theirs} Hz, or ask {src} \
                 to call at {ours} Hz."
            )
        } else {
            format!(
                "To work {src}, set the bandwidth to {theirs} Hz in Setup step 4 (a {theirs} Hz \
                 station still answers {ours} Hz calls), or ask {src} to call at {ours} Hz."
            )
        };
        let sentence = format!("{happened} {fix}");
        self.note("mismatch", &sentence);
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: src,
            what,
            theirs_hz: theirs,
            ours_hz: ours,
            at_ms: now_ms,
            sentence,
            theirs_protocol: None,
            ours_protocol: None,
            called: kin.then_some(dst),
        });
    }

    /// A call or probe from `src` to `dst`, a callsign `app` answered for earlier in this run
    /// and no longer does (ADR-0108): it is the program's caller, and nothing here answers it
    /// until the program runs again. Said once a minute per caller.
    fn note_program_gone(
        &mut self,
        src: &str,
        dst: &str,
        what: &'static str,
        app: &str,
        (theirs, ours): (usize, usize),
    ) {
        let now_ms = super::beacons::unix_ms();
        if self.bandwidth.mismatch.as_ref().is_some_and(|m| {
            m.callsign == src && m.what == "program" && now_ms.saturating_sub(m.at_ms) < 60_000
        }) {
            return;
        }
        let verb = if what == "call" { "called" } else { "probed" };
        let sentence = format!(
            "{src} {verb} {dst}, which {app} answered for: {app} is not running now, so \
             nothing answers {dst}. Start {app} again."
        );
        self.note("mismatch", &sentence);
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: src.to_owned(),
            what: "program",
            theirs_hz: theirs,
            ours_hz: ours,
            at_ms: now_ms,
            sentence,
            theirs_protocol: None,
            ours_protocol: None,
            called: Some(dst.to_owned()),
        });
    }

    /// A call from `src` to `dst` that the station's external program would answer, heard with
    /// none attached: to one of its kin callsigns (`kin`), or to its own while the program keys
    /// the radio. Said once a minute per caller, with the fix: start the program, or check its
    /// port (report 22 §2).
    pub(super) fn note_no_program(
        &mut self,
        src: &str,
        dst: &str,
        kin: bool,
        port: u16,
        (theirs, ours): (usize, usize),
    ) {
        let now_ms = super::beacons::unix_ms();
        if self.bandwidth.mismatch.as_ref().is_some_and(|m| {
            m.callsign == src && m.what == "program" && now_ms.saturating_sub(m.at_ms) < 60_000
        }) {
            return;
        }
        let sentence = if kin {
            format!(
                "{src} is calling {dst}: no external program is attached to answer it. Start \
                 it, or check it uses port {port}. Aether itself answers as {}.",
                self.engine.callsigns.join(", ")
            )
        } else {
            format!(
                "{src} is calling {dst}: the external program that keys this radio is not \
                 attached. Start it, or check it uses port {port}."
            )
        };
        self.note("mismatch", &sentence);
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: src.to_owned(),
            what: "program",
            theirs_hz: theirs,
            ours_hz: ours,
            at_ms: now_ms,
            sentence,
            theirs_protocol: None,
            ours_protocol: None,
            called: Some(dst.to_owned()),
        });
    }

    /// A call or probe from `src`, in a bandwidth this station answers, to `dst` — another name
    /// of one of its callsigns, which it does not answer to (ADR-0057).
    fn note_kin_call(
        &mut self,
        src: String,
        dst: String,
        (theirs, ours): (usize, usize),
        at_ms: u64,
    ) {
        let sentence = format!(
            "{src} is calling {dst}, which this station does not answer to: it answers as \
             {}. Answer as {dst} too, or ask {src} to call {}.",
            self.engine.callsigns.join(", "),
            self.engine.callsigns.first().map_or("", String::as_str)
        );
        self.note("mismatch", &sentence);
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: src,
            what: "callsign",
            theirs_hz: theirs,
            ours_hz: ours,
            at_ms,
            sentence,
            theirs_protocol: None,
            ours_protocol: None,
            called: Some(dst),
        });
    }

    /// A call to this station, or an answer to its call, from a station speaking another link
    /// protocol: the engine ignores it — the two cannot connect — and the panel says which of
    /// the two needs to update, where before only a log line said "ignored" (ADR-0041).
    pub(super) fn note_version(&mut self, decoded: &crate::air::HeardFrame) {
        use aether_link::frames::{ConnectBody, DataKind, PROTOCOL_VERSION, decode_data};
        if decoded.control || !matches!(self.engine.state(), State::Idle | State::Connecting) {
            return;
        }
        let Some((header, body)) = decoded
            .payload
            .as_ref()
            .and_then(|payload| decode_data(payload).ok())
        else {
            return;
        };
        if !matches!(header.kind, DataKind::ConnectReq | DataKind::ConnectAck) {
            return;
        }
        let Ok(connect) = ConnectBody::decode(&body) else {
            return;
        };
        if connect.version == PROTOCOL_VERSION || !self.engine.callsigns.contains(&connect.dst) {
            return;
        }
        let (src, theirs, ours) = (connect.src, connect.version, PROTOCOL_VERSION);
        let now_ms = super::beacons::unix_ms();
        if self.bandwidth.mismatch.as_ref().is_some_and(|m| {
            m.callsign == src && m.what == "version" && now_ms.saturating_sub(m.at_ms) < 60_000
        }) {
            return;
        }
        let happened = if header.kind == DataKind::ConnectReq {
            format!("{src} called this station and was not answered")
        } else {
            format!("{src} answered this station's call and the answer was ignored")
        };
        let fix = if theirs < ours {
            format!("{src} runs an older version of Aether HF and needs to update to connect.")
        } else {
            "This station runs an older version of Aether HF: update it (Help › Check for \
             Updates) to connect."
                .to_owned()
        };
        let sentence = format!(
            "{happened}: it speaks link protocol {theirs}, this station {ours}, and stations of \
             different protocols cannot connect. {fix}"
        );
        self.note("mismatch", &sentence);
        let hz = self.bandwidth_hz();
        self.bandwidth.mismatch = Some(Mismatch {
            callsign: src,
            what: "version",
            theirs_hz: hz,
            ours_hz: hz,
            at_ms: now_ms,
            sentence,
            theirs_protocol: Some(theirs),
            ours_protocol: Some(ours),
            called: None,
        });
    }

    /// A 2 300 Hz call that [`NARROW_AFTER_TRIES`] tries have not had answered, to a station
    /// not known to run 2 300 Hz, goes on at 500 Hz (ADR-0058): a 500 Hz station ignores a wider
    /// call, and a 2 300 Hz one answers a narrower call by moving to it. KO4WX called this
    /// station at 2 300 Hz for minutes while it ran 500 Hz (2026-10-10). Only between tries. A
    /// host program's chosen bandwidth is the first one tried, not the only one (ADR-0102): it
    /// had been left alone, and Winlink Express, which always says a `BW`, never had a call
    /// to a 500 Hz station go on.
    pub(super) fn advance_call_bandwidth(&mut self, now: f64) {
        // just before the next try, when an answer to the last has had its time — a 2 300 Hz
        // acceptance arriving after the move would be read at 500 Hz — and with nothing
        // arriving now
        let due = self
            .engine
            .connect_due()
            .is_some_and(|at| at - now <= MOVE_BEFORE_TRY_S);
        if self.engine.state() != State::Connecting
            || self.bandwidth_hz() != 2300
            || self
                .narrow_after_tries()
                .is_none_or(|after| self.engine.connect_tries() < after)
            || !due
            || self.receiving()
            || self.transmitting
            || !self.pending.is_empty()
            || !self.playback.is_empty()
        {
            return;
        }
        let remote = self.engine.remote_call.trim().to_ascii_uppercase();
        if self.known_bandwidth(&remote) == Some(2300) {
            return;
        }
        self.move_to(500, Why::Unanswered(remote));
    }

    /// The unanswered tries after which a 2 300 Hz call goes on at 500 Hz: [`NARROW_AFTER_TRIES`],
    /// and only for a call that makes more (ADR-0126 §9): a 2 300 Hz station reads only the tone
    /// floor of a 500 Hz call, so a short call that moved lost its last try's OFDM half to every
    /// station not known to run 500 Hz.
    /// Not for a call asked to last (ADR-0129): its station scans its channels, and an unanswered
    /// try says it was elsewhere, not that it runs 500 Hz. A scanning gateway's caller fell back
    /// after four tries and lost the rest of its call to the gateway's visits.
    fn narrow_after_tries(&self) -> Option<usize> {
        (!self.call_lasting && self.engine.call_tries_planned() > NARROW_AFTER_TRIES)
            .then_some(NARROW_AFTER_TRIES)
    }

    /// Whether the station could move to another bandwidth now.
    ///
    /// # Errors
    /// What is under way, in a sentence.
    pub(super) fn movable(&self) -> Result<(), String> {
        if self.test_running() {
            return Err("a Test session is running".into());
        }
        self.movable_but_for_a_test()
    }

    /// [`Self::movable`] without the Test: what the Test itself asks before the move its own
    /// call makes ([`Self::move_for_call`]).
    fn movable_but_for_a_test(&self) -> Result<(), String> {
        if self.probing() {
            return Err("a probe is out".into());
        }
        match self.engine.state() {
            State::Idle => {}
            State::Connecting => return Err("a call is going out".into()),
            _ => return Err("a session is up".into()),
        }
        if self.transmitting || !self.pending.is_empty() || !self.playback.is_empty() {
            return Err("the station is transmitting".into());
        }
        Ok(())
    }

    /// Go back to the bandwidth the station should run — its own, or a host program's — once
    /// nothing holds it where it is: called every pass. After a session a call brought, the
    /// channel has to have been quiet for [`RETURN_QUIET_S`] first.
    pub(super) fn advance_bandwidth(&mut self, now: f64) {
        let target = self.bandwidth.target_hz();
        if target == self.bandwidth_hz() {
            // a request withdrawn may leave another in force at the same bandwidth: say whose
            if matches!(
                self.bandwidth.why,
                Why::Configured | Why::Host | Why::Listening
            ) {
                self.bandwidth.why = self.bandwidth.why_for_target();
            }
            return;
        }
        let quiet = self.movable().is_ok() && !self.receiving() && now >= self.deaf_until;
        if !quiet {
            self.bandwidth.stirred_s = now;
            return;
        }
        if matches!(
            self.bandwidth.why,
            Why::Call(_) | Why::Calling(_) | Why::Unanswered(_)
        ) && now - self.bandwidth.stirred_s < RETURN_QUIET_S
        {
            return;
        }
        let why = self.bandwidth.why_for_target();
        self.move_to(target, why);
    }

    /// A decoded frame that is a call to this station in the narrower bandwidth, while it runs
    /// the wider and could move: the callsign calling. The engine would ignore it — the
    /// handshake states the bandwidth, and a call claiming another is not a session to start
    /// on this air — so the station moves first and the engine, on the caller's air, answers.
    pub(super) fn narrower_call(&self, decoded: &crate::air::HeardFrame) -> Option<String> {
        use aether_link::frames::{
            ConnectBody, DataKind, PROTOCOL_VERSION, bandwidth_code, bandwidth_code_of, decode_data,
        };
        if self.bandwidth_hz() != 2300 || decoded.control {
            return None;
        }
        let (header, body) = decode_data(decoded.payload.as_ref()?).ok()?;
        if header.kind != DataKind::ConnectReq {
            return None;
        }
        let request = ConnectBody::decode(&body).ok()?;
        let narrow = Some(bandwidth_code(request.caps)) == bandwidth_code_of(500);
        let ours = self.engine.callsigns.contains(&request.dst);
        // a call of another link protocol would be ignored on either air (and said so)
        (narrow && ours && request.version == PROTOCOL_VERSION && self.movable().is_ok())
            .then_some(request.src)
    }

    /// Rebuild what depends on the waveform for `hz`, and say so.
    ///
    /// # Panics
    /// If `hz` is not a bandwidth this version runs, or the engine is not idle — both are
    /// checked by every caller.
    pub(super) fn move_to(&mut self, hz: usize, why: Why) {
        let params = params_for(hz).expect("a bandwidth this version runs");
        let now = self.now();
        let from = self.bandwidth_hz();
        self.config.params = params;
        // every part new; the new receiver counts its samples from here, the station's from its
        // start. The detector's floor was learned through the other band filter: it learns
        // again; the radio's levels on quiet are the radio's, and stay (ADR-0078 §5)
        if let Some(air) = self.air.moved_to(hz, self.config.busy) {
            self.air = air;
        }
        // a move is between sessions: the new receiver starts on the dial (ADR-0079)
        self.front_end_rebuilt(now, false);
        self.was_busy = false;
        self.occupancy = crate::regulatory::occupancy::air(hz).ok();
        let top = self.top_rung();
        self.config.link.max_mode = top;
        if self.engine.state() == State::Connecting {
            // a call carried on to the other air (ADR-0058): its tries and timers stand
            self.engine
                .move_call(self.air_timing(), self.offered_capabilities(), Some(top))
                .expect("calling");
        } else {
            self.engine
                .set_air(self.air_timing(), self.offered_capabilities(), Some(top))
                .expect("moved only while idle");
        }
        self.refresh_ceiling(true);
        self.refresh_burst_cap(now);
        self.bandwidth.moves += 1;
        self.bandwidth.stirred_s = now;
        let reason = match &why {
            Why::Configured => "the station's own".to_owned(),
            Why::Host => "a program asked for it".to_owned(),
            Why::Listening => {
                "a program listening for calls asked for it: it answers calls in either bandwidth"
                    .to_owned()
            }
            Why::Call(call) => format!("{call} called in it; back to {from} Hz after the session"),
            Why::Unanswered(call) => format!(
                "{call} has not answered at {from} Hz, and a {hz} Hz station ignores a wider call; \
                 back to {from} Hz after the session"
            ),
            Why::Calling(call) => {
                format!("calling {call}, which runs it; back to {from} Hz after the session")
            }
        };
        self.bandwidth.why = why;
        self.note("bandwidth", &format!("{from} Hz → {hz} Hz: {reason}"));
        // for the replay, which builds its receiver again here as the station did
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, "air", &hz.to_string(), &state);
        }
    }

    /// The fastest rung on the ladder the station runs: `[radio] max_mode`, which indexes the
    /// ladder of the bandwidth in use, held to its last rung.
    pub(super) fn top_rung(&self) -> usize {
        self.config
            .max_mode_setting
            .unwrap_or(self.config.link.max_mode)
            .min(self.air.info().rungs.saturating_sub(1))
    }

    /// The link timing of the air the station runs, with this station's keying in it.
    pub(super) fn air_timing(&self) -> aether_link::PhyTiming {
        let mut timing = self.air.timing();
        timing.tx_latency_s = self.engine_latency_s();
        timing
    }

    /// What the connect handshake offers: compression if the station offers it, and the
    /// bandwidth it runs.
    pub(super) fn offered_capabilities(&self) -> u8 {
        aether_link::frames::with_bandwidth(
            crate::compress::offered_capabilities(self.config.compress),
            self.bandwidth_hz(),
        )
    }
}
