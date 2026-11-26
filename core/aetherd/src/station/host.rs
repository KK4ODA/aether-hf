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

/// How long after a frame that may be a call has ended the station waits for its report before
/// it says that nothing came of it (ADR-0138): the receiver hands a tone frame over up to
/// 0.72 s after its end, once nothing still to come can claim its span.
const ARRIVAL_REPORT_S: f64 = 1.0;

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
    /// A frame that may be a call is on the air, and until when its report is waited for
    /// (ADR-0138).
    pub(super) arriving_until: Option<f64>,
    /// What the host program is to hear of it: `true` a frame that may be a call began to
    /// arrive while it listened, `false` the frames have come to nothing.
    pub(super) arrival_news: Vec<bool>,
    /// Such a frame is arriving and the program has not been told yet: it waits for the program
    /// to have listened long enough to take it.
    pub(super) arrival_wanted: bool,
    /// The program was told, and holds its scan; since when, on the station's clock. It holds
    /// until it hears that nothing came of the frames, or turns its answering off.
    pub(super) held_since: Option<f64>,
    /// When its last hold ended with `CANCELPENDING`: it restarts its scan then, and when it
    /// steps next is not known until it turns its answering off and on again.
    pub(super) hold_ended: Option<f64>,
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
            // what it asked for goes with its connection, as its callsigns go with its claim
            // (ADR-0106); a request made without one goes now. Withdrawing cannot fail.
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
        }
        .and_then(|()| self.heard_on_its_channel(&to, decoded.start_s));
        let Err(why) = answered else {
            return false;
        };
        self.stats.calls_unanswered += 1;
        // the external program that keys the radio is not attached: the banner says so, as the
        // log line alone did not on 2026-11-24 (report 22 §2)
        if what == "call" && self.config.keyed_by_host && !self.host_attached() {
            if let Some(port) = self.config.external_port {
                let hz = self.bandwidth_hz();
                self.note_no_program(&from, &to, false, port, (hz, hz));
            }
        }
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

    /// A frame announced while the station is idle that may be a call or a probe: a DATA
    /// frame's length, at the confidence a scan takes for a try under way (ADR-0129), begun
    /// while the host program listened. The program hears it as `PENDING` from the host adapter
    /// when it scans (ADR-0138): RMS Trimode holds its scanner then, and the radio stays on the
    /// dial the call is coming in on. Decoding takes the whole frame (a tone try is 5.4 s,
    /// longer than Trimode's 4.4 s on a channel), and the `PENDING` that went only once the
    /// call was taken came after Trimode had moved the radio (KK4ODA-1, 2026-11-25).
    pub(super) fn note_call_arriving(&mut self, arrival: &crate::air::Arrival) {
        if !self.config.program_scans
            || self.engine.state() != State::Idle
            || arrival.detect_confidence < super::scan::NOTICE_CONFIDENCE
        {
            return;
        }
        let Some(since) = self.host_listening_since() else {
            return;
        };
        if since > arrival.start_s + super::claims::LISTEN_SLACK_S {
            return;
        }
        // a call and a probe are DATA frames; a header, an answer or an acceptance is shorter
        let timing = self.air.timing();
        let data_s = if arrival.floor {
            timing.floor_data_frame_s.unwrap_or(f64::INFINITY)
        } else {
            timing.data_frame_s
        };
        if arrival.length_s + 0.05 < data_s {
            return;
        }
        self.host.arrival_wanted = true;
        let until = arrival.end_s + ARRIVAL_REPORT_S;
        self.host.arriving_until = Some(self.host.arriving_until.map_or(until, |u| u.max(until)));
    }

    /// Tell a scanning program of a frame that may be a call once it can take it, and that
    /// nothing came of the frames once they are over and reported (ADR-0138). A program that
    /// steps its scan on `LISTEN` is told once it has listened a moment, and not when about to
    /// step, since the `PENDING` would meet the `LISTEN OFF` its scan has decided; one that
    /// listens without a break while it scans (`VarAC`) is told at once (§8). Only with
    /// `[host] scans` on: a program on one channel moves no radio and is told nothing.
    pub(super) fn advance_call_arriving(&mut self, now: f64) {
        // turned off, or off and on again: whatever it held for is over
        if let Some(held) = self.host.held_since
            && self.host_listening_since().is_none_or(|since| since > held)
        {
            self.host.held_since = None;
        }
        if self.host.arrival_wanted && self.host.held_since.is_none() {
            match self.host_scan() {
                Some(scan) if !self.beat_known(scan) || scan.steps_soon(now) => {
                    self.host.arrival_wanted = false;
                }
                Some(scan) if scan.settled(now) => {
                    self.host.arrival_wanted = false;
                    self.host.held_since = Some(now);
                    self.host.arrival_news.push(true);
                }
                Some(_) => {}
                // a program that scans without a word to the modem (VarAC's Frequency Scanner):
                // told at once, since nothing says when it will move next, and a PENDING holds
                // it on the channel the call is arriving on (ADR-0138 §8)
                None if self.host_listens_steadily(now) => {
                    self.host.arrival_wanted = false;
                    self.host.held_since = Some(now);
                    self.host.arrival_news.push(true);
                }
                None => self.host.arrival_wanted = false,
            }
        }
        if self.host.arriving_until.is_some_and(|until| now >= until) {
            self.host.arriving_until = None;
            self.host.arrival_wanted = false;
            if self.host.held_since.take().is_some() {
                self.host.arrival_news.push(false);
                self.host.hold_ended = Some(now);
            }
        }
    }

    /// Whether the program's next step can be told from its beat: not between the end of a
    /// hold and its next `LISTEN ON`. RMS Trimode restarts its scan at `CANCELPENDING`, and the
    /// harness's four-channel gateway, 10 s into a window it had held for, was told of a call
    /// as if it had stopped stepping; its step crossed the `PENDING`, and the acceptance went out
    /// on 40 m (ADR-0138).
    fn beat_known(&self, scan: super::claims::ScanBeat) -> bool {
        self.host.hold_ended.is_none_or(|ended| scan.since >= ended)
    }

    /// Whether a call or probe to `call` can be answered before the program holding it steps
    /// its scan, or why not (ADR-0138). A program holding for a frame that may be a call does
    /// not step; one that is not holding steps at the end of its window, and a program
    /// scanning channels moves the radio then. A call taken in the last quarter of a window
    /// was told `PENDING` too late to hold it (the `LISTEN OFF` its scan had decided crossed
    /// it), and its acceptance went out on the next channel (the scenario harness, 2026-11-25:
    /// a both-ways try read at its OFDM part, answered after its tone part).
    pub(super) fn answered_before_its_step(&self, call: &str) -> Result<(), String> {
        if self.host.held_since.is_some() {
            return Ok(());
        }
        let now = self.now();
        match self.holder_scan(call) {
            Some((scan, app)) if !self.beat_known(scan) => Err(format!(
                "{app} has just restarted its scan, and may move the radio before the answer goes"
            )),
            Some((scan, app)) if scan.steps_soon(now) => Err(format!(
                "{app} is about to step its scan, and may move the radio before the answer goes"
            )),
            _ => Ok(()),
        }
    }

    /// Whether a call or probe to `call`, begun at `start_s`, was heard on the channel its
    /// answer goes out on, or why not (ADR-0138). Asked only of a program that scans channels
    /// (`[host] scans`, §6): one on a single channel answers on the channel it heard the call
    /// on, whatever its `LISTEN` steps, and refusing a try read late in one of its windows
    /// only cost it a try.
    fn heard_on_its_channel(&self, call: &str, start_s: f64) -> Result<(), String> {
        if !self.config.program_scans {
            return Ok(());
        }
        self.listened_throughout(call, start_s)
            .and_then(|()| self.answered_before_its_step(call))
    }

    /// What the host program is to hear of frames that may be calls (ADR-0138), oldest first.
    pub fn take_arrival_news(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.host.arrival_news)
    }
}
