//! The bandwidth the station runs, and the moves between the two (ADR-0026).
//!
//! A station has a bandwidth of its own — `[radio] bandwidth`, what Setup saves — and runs it
//! unless one of two things moves it, both between sessions:
//!
//! * **A host program's `BW500`, `BW2300` or `BW2750`.** VARA's published commands set the
//!   modem's mode with them ("Set VARA HF to 500Hz Narrow mode"), and a program set up for
//!   VARA sends the one it wants — `VarAC` 500 Hz on its calling frequencies, Winlink Express
//!   its session's, Pat its configuration's — so an operator going from one program to the
//!   other no longer changes Setup and restarts the modem. The request holds while that
//!   program is attached; when it goes, the station goes back to its own.
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
//! ladder and handshake bandwidth, [`LinkEngine::set_air`](aether_link::LinkEngine::set_air))
//! — and keeps the rest: the callsigns, the counters, the identifier's clock, the stations
//! heard, the KISS datagrams not yet framed. It is made only when nothing is under way that
//! runs on the air it would leave: no session, call or probe, no Test, nothing queued or on
//! the air.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::{Value, json};

use super::{State, Station};
use crate::ptt::Ptt;

/// How long the channel stays quiet, after a session in the bandwidth a call brought, before
/// the station goes back to its own: time for the other station's goodbye — its identifier, a
/// disconnect repeated on the tone floor, three tries of 3.2 s each and their turnarounds —
/// and for the call that often follows a ping.
pub const RETURN_QUIET_S: f64 = 20.0;

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
}

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
    /// A host program asked for it with `BW<n>`.
    Host,
    /// A call to this station came in it; the callsign that called.
    Call(String),
    /// This station called one it knows runs it (ADR-0035); the callsign called.
    Calling(String),
}

impl Why {
    fn name(&self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Host => "host",
            Self::Call(_) => "call",
            Self::Calling(_) => "calling",
        }
    }
}

/// The bandwidth the station should run, and why it runs what it does.
#[derive(Debug, Clone)]
pub(super) struct Bandwidth {
    /// The station's own: `[radio] bandwidth`.
    pub(super) home_hz: usize,
    /// What the host program attached now asked for.
    host_hz: Option<usize>,
    /// Why the station runs what it runs now.
    why: Why,
    /// Station time the station was last anything but idle and quiet while it runs another
    /// bandwidth than it should: the move back waits [`RETURN_QUIET_S`] after it.
    stirred_s: f64,
    /// Moves made since the modem started.
    moves: usize,
    /// Other stations' bandwidths as their frames stated them, the most recently heard last.
    known: VecDeque<(String, usize)>,
    /// The last call or probe to this station in a bandwidth whose calls it does not answer.
    mismatch: Option<Mismatch>,
}

impl Bandwidth {
    pub(super) fn new(home_hz: usize) -> Self {
        Self {
            home_hz,
            host_hz: None,
            why: Why::Configured,
            stirred_s: f64::NEG_INFINITY,
            moves: 0,
            known: VecDeque::new(),
            mismatch: None,
        }
    }

    /// The bandwidth the station should run when nothing holds it where it is.
    fn target_hz(&self) -> usize {
        self.host_hz.unwrap_or(self.home_hz)
    }
}

impl<P: Ptt> Station<P> {
    /// The bandwidth the station runs now, in hertz.
    #[must_use]
    pub fn bandwidth_hz(&self) -> usize {
        self.config.params.bandwidth.hz()
    }

    /// A host program's `BW<n>`: run `hz` from now on while the program is attached — or, with
    /// `None`, the station's own again. 2 750 Hz, which Winlink Express asks for unless told
    /// otherwise, is 2 300 Hz: a narrower signal is always inside what was asked.
    ///
    /// The move is made now or not at all: a program that asked for 500 Hz and was told `OK`
    /// calls at once, and a move that waited for a session to end would have it call in the
    /// wrong bandwidth. Withdrawing a request always succeeds; the move back waits for the
    /// station to be idle.
    ///
    /// # Errors
    /// With the reason in a sentence: a bandwidth this version does not run, or something under
    /// way that runs on the air the station would leave.
    pub fn host_bandwidth(&mut self, hz: Option<usize>) -> Result<usize, String> {
        let Some(hz) = hz else {
            self.bandwidth.host_hz = None;
            return Ok(self.bandwidth_hz());
        };
        let hz = if hz == 2750 { 2300 } else { hz };
        if params_for(hz).is_none() {
            return Err(format!("this modem runs 2300 or 500 Hz, not {hz}"));
        }
        if hz != self.bandwidth_hz() {
            self.movable()?;
            self.move_to(hz, Why::Host);
        } else if self.bandwidth.why != Why::Host {
            self.bandwidth.why = Why::Host;
        }
        self.bandwidth.host_hz = Some(hz);
        Ok(hz)
    }

    /// The bandwidth, as `status.bandwidth` reports it: what runs, the station's own, what a
    /// host program asked for, and why.
    #[must_use]
    pub fn bandwidth_status(&self) -> Value {
        json!({
            "bandwidth_hz": self.bandwidth_hz(),
            "home_hz": self.bandwidth.home_hz,
            "host_hz": self.bandwidth.host_hz,
            "why": self.bandwidth.why.name(),
            "caller": match &self.bandwidth.why {
                Why::Call(call) => Some(call.as_str()),
                _ => None,
            },
            "callee": match &self.bandwidth.why {
                Why::Calling(call) => Some(call.as_str()),
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

    /// Remember the bandwidth a station runs, as one of its frames stated it — or as the
    /// stations-heard list kept it, which is how a restarted daemon still knows.
    pub fn learn_bandwidth(&mut self, callsign: &str, hz: usize) {
        let callsign = callsign.trim().to_ascii_uppercase();
        let known = &mut self.bandwidth.known;
        known.retain(|(call, _)| *call != callsign);
        known.push_back((callsign, hz));
        while known.len() > KNOWN_LIMIT {
            known.pop_front();
        }
    }

    /// The bandwidth a station was last heard to run, if it has been.
    #[must_use]
    pub fn known_bandwidth(&self, callsign: &str) -> Option<usize> {
        let callsign = callsign.trim().to_ascii_uppercase();
        self.bandwidth
            .known
            .iter()
            .rev()
            .find(|(call, _)| *call == callsign)
            .map(|&(_, hz)| hz)
    }

    /// Before a call: a 2 300 Hz station calling one it knows runs 500 Hz moves to 500 Hz, so
    /// the call is one the other answers whatever its version (ADR-0035). The move back is a
    /// call's: after the session, or the call's end, and a quiet spell.
    pub(super) fn move_for_call(&mut self, remote: &str) {
        if self.bandwidth_hz() == 2300
            && self.known_bandwidth(remote) == Some(500)
            && self.movable().is_ok()
        {
            self.move_to(500, Why::Calling(remote.trim().to_ascii_uppercase()));
        }
    }

    /// A call or a probe to this station, while idle, in a bandwidth whose calls it does not
    /// answer: kept for the panel and said in the log, with both bandwidths and the fix. A
    /// 2 300 Hz station answers 500 Hz calls by moving (the narrower call), so what is left is
    /// a wider call — or probe, which is answered, but warns that a call would not be.
    pub(super) fn note_mismatch(&mut self, decoded: &aether_phy::DecodedFrame) {
        use aether_link::frames::{ConnectBody, DataKind, ProbeBody, bandwidth_hz_of, decode_data};
        if self.engine.state() != State::Idle || decoded.frame.is_control() {
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
        if !self.engine.callsigns.contains(&dst)
            || theirs == ours
            || (ours == 2300 && theirs == 500)
        {
            return;
        }
        let now_ms = super::beacons::unix_ms();
        // a caller tries again and again: one warning a minute per station and kind is plenty
        if self.bandwidth.mismatch.as_ref().is_some_and(|m| {
            m.callsign == src && m.what == what && now_ms.saturating_sub(m.at_ms) < 60_000
        }) {
            return;
        }
        let happened = if what == "call" {
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
                "The host program asked for {ours} Hz: have it ask for {theirs} Hz, or ask {src} \
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
        });
    }

    /// Whether the station could move to another bandwidth now.
    ///
    /// # Errors
    /// What is under way, in a sentence.
    pub(super) fn movable(&self) -> Result<(), String> {
        if self.test_running() {
            return Err("a Test session is running".into());
        }
        if self.engine.probing() {
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
            return;
        }
        let quiet = self.movable().is_ok() && !self.receiving() && now >= self.deaf_until;
        if !quiet {
            self.bandwidth.stirred_s = now;
            return;
        }
        if matches!(self.bandwidth.why, Why::Call(_) | Why::Calling(_))
            && now - self.bandwidth.stirred_s < RETURN_QUIET_S
        {
            return;
        }
        let why = if self.bandwidth.host_hz.is_some() {
            Why::Host
        } else {
            Why::Configured
        };
        self.move_to(target, why);
    }

    /// A decoded frame that is a call to this station in the narrower bandwidth, while it runs
    /// the wider and could move: the callsign calling. The engine would ignore it — the
    /// handshake states the bandwidth, and a call claiming another is not a session to start
    /// on this air — so the station moves first and the engine, on the caller's air, answers.
    pub(super) fn narrower_call(&self, decoded: &aether_phy::DecodedFrame) -> Option<String> {
        use aether_link::frames::{
            ConnectBody, DataKind, PROTOCOL_VERSION, bandwidth_code, bandwidth_code_of, decode_data,
        };
        if self.bandwidth_hz() != 2300 || decoded.frame.is_control() {
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
        self.receiver = aether_phy::StreamingReceiver::new(params, 6.0, true);
        // the new receiver counts its samples from here, the station's from its start
        self.rx_origin = self.baseband_seen;
        self.decoder = std::rc::Rc::new(std::cell::RefCell::new(aether_phy::Modem::new(
            params, false,
        )));
        self.transmitter = aether_phy::Modem::new(params, false);
        self.to_audio = aether_phy::BasebandToAudio::new(params);
        self.from_audio = aether_phy::AudioToBaseband::new(params);
        // the detector's floor was learned through the other band filter: it learns again
        self.busy = crate::busy::BusyDetector::new(crate::busy::BusyConfig {
            fs: params.fs_baseband,
            passband_hz: params.occupied_bandwidth_hz(),
            threshold_db: self.busy.config().threshold_db,
            ..self.config.busy
        });
        self.was_busy = false;
        self.occupancy = crate::regulatory::occupancy::air(hz).ok();
        let top = self.top_rung();
        self.config.link.max_mode = top;
        self.engine
            .set_air(self.air_timing(), self.offered_capabilities(), Some(top))
            .expect("moved only while idle");
        self.refresh_ceiling(true);
        self.refresh_burst_cap(now);
        self.bandwidth.moves += 1;
        self.bandwidth.stirred_s = now;
        let reason = match &why {
            Why::Configured => "the station's own".to_owned(),
            Why::Host => "the host program asked for it".to_owned(),
            Why::Call(call) => format!("{call} called in it; back to {from} Hz after the session"),
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
            .min(self.air().n_rungs().saturating_sub(1))
    }

    /// The link timing of the air the station runs, with this station's keying in it.
    pub(super) fn air_timing(&self) -> aether_link::PhyTiming {
        let mut timing = super::phy_timing(self.config.params);
        timing.tx_latency_s =
            self.config.key_lead_s + self.config.playback_lead_s + self.config.key_tail_s;
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
