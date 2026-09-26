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
//!
//! A move rebuilds what depends on the waveform — the receiver, the modems, the band filters,
//! the busy detector, the occupancy the rules judge by, the link engine's air (its timing,
//! ladder and handshake bandwidth, [`LinkEngine::set_air`](aether_link::LinkEngine::set_air))
//! — and keeps the rest: the callsigns, the counters, the identifier's clock, the stations
//! heard, the KISS datagrams not yet framed. It is made only when nothing is under way that
//! runs on the air it would leave: no session, call or probe, no Test, nothing queued or on
//! the air.

use serde_json::{Value, json};

use super::{State, Station};
use crate::ptt::Ptt;

/// How long the channel stays quiet, after a session in the bandwidth a call brought, before
/// the station goes back to its own: time for the other station's goodbye — its identifier, a
/// disconnect repeated on the tone floor, three tries of 3.2 s each and their turnarounds —
/// and for the call that often follows a ping.
pub const RETURN_QUIET_S: f64 = 20.0;

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
}

impl Why {
    fn name(&self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Host => "host",
            Self::Call(_) => "call",
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
}

impl Bandwidth {
    pub(super) fn new(home_hz: usize) -> Self {
        Self {
            home_hz,
            host_hz: None,
            why: Why::Configured,
            stirred_s: f64::NEG_INFINITY,
            moves: 0,
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
            "moves": self.bandwidth.moves,
        })
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
        if matches!(self.bandwidth.why, Why::Call(_))
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
