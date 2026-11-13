//! What the host program attached now says about this station (`docs/spec/host-interfaces.md`).
//!
//! VARA's published commands give the program on its command port a say over the modem that
//! goes beyond its own sessions: `LISTEN ON`/`OFF` decides whether incoming calls are answered
//! at all — off, VARA's default, until the program says otherwise — and `BW<n>` which bandwidth
//! the station runs ([`super::bandwidth`]). The host adapter is a client of the control API on
//! its own thread; what it heard reaches the station here, from the run loop, every pass.

use super::{State, Station};
use crate::ptt::Ptt;

/// How long a call left unanswered is not said again: a caller tries up to eight times, and
/// one line is enough to tell the operator why nothing answered.
const UNANSWERED_QUIET_S: f64 = 60.0;

/// What the host program attached now said that is not about calls: whether it is there,
/// its `CHAT ON` and its `CWID ON`. Which calls are answered is its claim's business
/// (ADR-0106, `claims.rs`): `MYCALL` and `LISTEN` reach the station as the control API's
/// `claim` and `listen`, on the program's own connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostPresence {
    /// A program holds the host command port.
    pub attached: bool,
    /// It said `CHAT ON`. It once switched on the turn request (ADR-0027), which granted slots
    /// have no use for (ADR-0069); what it still decides is the KISS programs' priority
    /// (ADR-0019).
    pub chat: bool,
    /// It said `CWID ON`: the program's sessions end with the Morse identifier whatever the
    /// station's own setting, as when either asks (ADR-0075 §2.7).
    pub cw_id: bool,
}

/// The host program's say, and what was last said of a call kept unanswered.
#[derive(Debug, Clone, Default)]
pub(super) struct Host {
    presence: HostPresence,
    /// The last caller left unanswered, and when that was said.
    said: Option<(String, f64)>,
    /// The daemon is stopping (`Station::stop_answering`): no call or probe is answered while
    /// its last session's end leaves the air.
    pub(super) stopping: bool,
}

impl<P: Ptt> Station<P> {
    /// What the host program attached now said — or that none is. A program that goes takes
    /// its bandwidth with it: the station goes back to its own once idle (ADR-0026).
    pub fn set_host(&mut self, presence: HostPresence) {
        let was = self.host.presence;
        if presence == was {
            return;
        }
        self.host.presence = presence;
        if presence.attached && !was.attached {
            // the keying the program and the station were each set up for is first tried here:
            // the first transmission keyed is judged on its own, not after three that changed
            // nothing (`keyed_audio.rs`)
            if let Some(watch) = self.keyed_audio.as_mut() {
                watch.judge_next_alone();
            }
        }
        if !presence.attached && was.attached {
            // what it asked for goes with it; withdrawing a request cannot fail. Its callsigns
            // go with its connection's claim (ADR-0106).
            let _ = self.host_bandwidth(None);
        }
        if presence.cw_id != was.cw_id {
            self.refresh_identifier_length();
        }
    }

    /// Whether a program attached asked for the Morse identifier (`CWID ON`).
    pub(super) fn host_asks_identifier(&self) -> bool {
        self.host.presence.attached && self.host.presence.cw_id
    }

    /// Whether a host program holds the command port now.
    #[must_use]
    pub fn host_attached(&self) -> bool {
        self.host.presence.attached
    }

    /// The daemon is stopping: answer no call or probe from now on. A stop first ends the
    /// session on the air (the wind-down), and the station listens meanwhile; at KK4ODA
    /// (2026-11-02 20:30Z) it answered a call that arrived in those seconds and was stopped
    /// in the middle of the new session, leaving the caller to time out.
    pub fn stop_answering(&mut self) {
        self.host.stopping = true;
    }

    /// A decoded call or probe to this station that is not to be answered, because the
    /// program holding the callsign called is not listening (ADR-0106), no program holds it
    /// under automatic control (a call, ADR-0107) or the daemon is stopping: counted, said once
    /// a minute for a caller, and kept from the engine. Only
    /// while idle — the answer to this station's own call, or anything in a session, is not an
    /// incoming call.
    pub(super) fn unanswered(&mut self, decoded: &crate::air::HeardFrame) -> bool {
        use aether_link::frames::{ConnectBody, DataKind, ProbeBody, decode_data};
        if self.engine.state() != State::Idle || decoded.control {
            return false;
        }
        let Some((header, body)) = decoded
            .payload
            .as_ref()
            .and_then(|payload| decode_data(payload).ok())
        else {
            return false;
        };
        let (what, from, to) = match header.kind {
            DataKind::ConnectReq => match ConnectBody::decode(&body) {
                Ok(call) => ("call", call.src, call.dst),
                Err(_) => return false,
            },
            DataKind::Probe => match ProbeBody::decode(&body) {
                Ok(probe) => ("probe", probe.src, probe.dst),
                Err(_) => return false,
            },
            _ => return false,
        };
        if !self.engine.callsigns.contains(&to) {
            return false;
        }
        // a probe brings up no session: only a call needs a program behind it (ADR-0107)
        let answered = if what == "call" {
            self.answers_call_for(&to)
        } else {
            self.answers_for(&to)
        };
        let Err(why) = answered else {
            return false;
        };
        self.stats.calls_unanswered += 1;
        let now = self.now();
        let repeated = self
            .host
            .said
            .as_ref()
            .is_some_and(|(caller, at)| *caller == from && now - at < UNANSWERED_QUIET_S);
        if !repeated {
            self.note(
                "listen",
                &format!("a {what} from {from} to {to} not answered: {why}"),
            );
            self.host.said = Some((from, now));
        }
        true
    }
}
