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

/// What the host program attached now said. With none attached the station is its own: it
/// answers every call to its callsigns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostPresence {
    /// A program holds the host command port.
    pub attached: bool,
    /// It said `LISTEN ON` (or `LISTEN CQ`, or `CHAT ON`, which includes it): answer calls.
    pub listening: bool,
}

impl HostPresence {
    /// Whether calls to this station are answered.
    #[must_use]
    pub fn answering(self) -> bool {
        !self.attached || self.listening
    }
}

/// The host program's say, and what was last said of a call it kept unanswered.
#[derive(Debug, Clone, Default)]
pub(super) struct Host {
    presence: HostPresence,
    /// The last caller left unanswered, and when that was said.
    said: Option<(String, f64)>,
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
        if !presence.attached && was.attached {
            // what it asked for goes with it; withdrawing a request cannot fail
            let _ = self.host_bandwidth(None);
        }
        if presence.answering() != was.answering() {
            let detail = if presence.answering() {
                "answering calls"
            } else {
                "not answering calls: the host program has not said LISTEN ON"
            };
            self.note("listen", detail);
        }
    }

    /// Whether calls to this station are answered now.
    #[must_use]
    pub fn answering(&self) -> bool {
        self.host.presence.answering()
    }

    /// A decoded call or probe to this station that is not to be answered, because the host
    /// program attached has not said `LISTEN ON`: counted, said once a minute for a caller,
    /// and kept from the engine. Only while idle — the answer to this station's own call, or
    /// anything in a session, is not an incoming call.
    pub(super) fn unanswered(&mut self, decoded: &aether_phy::DecodedFrame) -> bool {
        use aether_link::frames::{ConnectBody, DataKind, ProbeBody, decode_data};
        if self.answering() || self.engine.state() != State::Idle || decoded.frame.is_control() {
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
                &format!(
                    "a {what} from {from} to {to} not answered: the host program has not said \
                     LISTEN ON"
                ),
            );
            self.host.said = Some((from, now));
        }
        true
    }
}
