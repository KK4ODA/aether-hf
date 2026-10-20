//! The receiver follows a session's station whose rig is off frequency (ADR-0079, ADR-0064's
//! open step).
//!
//! The detectors search a carrier offset around where the receiver is centred: the tone floor
//! ±150 Hz, OFDM ±300 Hz. An older rig without a TCXO is tens of hertz off and drifts as it
//! warms. On the harness's 15 m path, 70 Hz and 0.3 Hz/s passed the floor's old ±100 Hz three
//! minutes into a session (`15m-old-rig-offset-500`), and ±150 Hz only puts the edge further
//! out. So the receiver centres on the station it is in session with: the receive mixer moves to
//! the offset that station's last decoded frame measured, and the band filter and both searches
//! move with it.
//!
//! * It moves only at this station's own key. The station is deaf while it transmits, so no
//!   frame is arriving to be cut in two by the mixer's step, and every exchange of a session
//!   has a transmission of each station's.
//! * It follows only the station of the session under way, and moves by [`FOLLOW_STEP_HZ`] or
//!   more. A call goes from the dial: what was heard of the station called may be a session old.
//! * Once no session is under way it goes back to the dial, at the first moment no frame is
//!   arriving: a station that answers calls must hear them where they are sent.
//! * A frame's offset is reported true: what it measured plus the mixer's offset where it began.
//! * Each move is a recording event, `rx_offset` with the offset in hertz, and the replay moves
//!   with it.
//!
//! Each station follows what it hears, and neither transmitter moves, so two stations following
//! each other cannot chase.

use std::collections::VecDeque;

use aether_link::State;

use super::{FrameReport, Station};
use crate::ptt::Ptt;

/// The least change of the followed station's offset that moves the receiver, hertz: the tone
/// floor's detector measures to a few hertz, and a smaller move buys nothing inside a ±150 Hz
/// search.
pub const FOLLOW_STEP_HZ: f64 = 10.0;

/// How far off a followed station has to be before the log says so, hertz: a fifth of the tone
/// floor's ±150 Hz, where a drifting rig starts to matter.
pub const FOLLOW_SAID_HZ: f64 = 30.0;

/// Moves kept for reading a frame's offset back: a frame is reported at most a floor frame's
/// settling after it began, and the mixer moves at most once a transmission.
const KEPT: usize = 16;

/// Where the receive mixer is centred, and what it follows.
#[derive(Debug, Default)]
pub(super) struct Centre {
    /// The mixer's offset from the dial, each with the station time it took effect from, newest
    /// last.
    moves: VecDeque<(f64, f64)>,
    /// The offset before the oldest move kept.
    before: f64,
    /// The last decoded frame that named its sender: whose, and its true offset.
    heard: Option<(String, f64)>,
}

impl Centre {
    /// The mixer's offset in force at station time `t`.
    fn at(&self, t: f64) -> f64 {
        self.moves
            .iter()
            .rev()
            .find(|(from, _)| *from <= t)
            .map_or(self.before, |&(_, hz)| hz)
    }

    fn push(&mut self, now: f64, hz: f64) {
        self.moves.push_back((now, hz));
        while self.moves.len() > KEPT {
            if let Some((_, hz)) = self.moves.pop_front() {
                self.before = hz;
            }
        }
    }
}

impl<P: Ptt> Station<P> {
    /// Where the receiver is centred, hertz from the dial: the offset of the station it follows,
    /// or 0.
    #[must_use]
    pub const fn rx_offset_hz(&self) -> f64 {
        self.from_audio.offset_hz()
    }

    /// A frame's true carrier offset: what it measured, plus the mixer's offset where it began
    /// (`start_s`, station time).
    pub(super) fn true_cfo(&self, measured: f64, start_s: f64) -> f64 {
        measured + self.centre.at(start_s)
    }

    /// A frame reported: a decoded one that names its sender is what the receiver follows, if
    /// that is the session's station.
    pub(super) fn heard_offset(&mut self, report: &FrameReport) {
        if let Some(from) = report.from.as_ref().filter(|_| report.decoded)
            && report.cfo_hz.is_finite()
        {
            self.centre.heard = Some((from.clone(), report.cfo_hz));
        }
    }

    /// Where the receiver should be centred now: on the session's station, as its last decoded
    /// frame measured it, or on the dial. A call goes from the dial: what was heard of the
    /// station called may be a session old, and its answer is looked for where it was sent.
    fn centre_target(&self) -> f64 {
        if !matches!(self.engine.state(), State::Connected | State::Disconnecting) {
            return 0.0;
        }
        match &self.centre.heard {
            Some((call, hz)) if *call == self.engine.remote_call => *hz,
            _ => self.from_audio.offset_hz(),
        }
    }

    /// At this station's key, deaf until it releases: centre on the session's station, if it
    /// has moved [`FOLLOW_STEP_HZ`] or more, or back on the dial.
    pub(super) fn follow_at_key(&mut self, now: f64) {
        let target = self.centre_target();
        let current = self.from_audio.offset_hz();
        if (target - current).abs() >= FOLLOW_STEP_HZ || (target == 0.0 && current != 0.0) {
            self.move_centre(target, now);
        }
    }

    /// Each pass: back on the dial once no session is under way, at the first moment no frame
    /// is arriving.
    pub(super) fn advance_centre(&mut self, now: f64) {
        if self.from_audio.offset_hz() == 0.0
            || self.engine.state() != State::Idle
            || self.receiving()
        {
            return;
        }
        self.move_centre(0.0, now);
    }

    /// The front end was built again from nothing — captured audio lost, or the station moved
    /// to the other bandwidth. A loss inside a session keeps the centre; a move between
    /// sessions starts on the dial.
    pub(super) fn front_end_rebuilt(&mut self, now: f64, keep: bool) {
        let was = self.centre.at(now);
        let hz = if keep { was } else { 0.0 };
        self.from_audio.set_offset_hz(hz);
        if (hz - was).abs() > f64::EPSILON {
            self.centre.push(now, hz);
            self.record_centre(now, hz);
        }
    }

    fn move_centre(&mut self, hz: f64, now: f64) {
        let was = self.from_audio.offset_hz();
        self.from_audio.set_offset_hz(hz);
        self.centre.push(now, hz);
        self.record_centre(now, hz);
        if hz.abs() >= FOLLOW_SAID_HZ && was.abs() < FOLLOW_SAID_HZ {
            let call = self.engine.remote_call.clone();
            self.note(
                "receiver",
                &format!("{call} is {hz:+.0} Hz off frequency: the receiver follows it"),
            );
        } else if hz == 0.0 && was.abs() >= FOLLOW_SAID_HZ {
            self.note("receiver", "back on the dial");
        }
    }

    fn record_centre(&mut self, now: f64, hz: f64) {
        if let Some(recording) = &mut self.recording {
            let state = format!("{:?}", self.engine.state());
            recording.event(now, "rx_offset", &format!("{hz:.3}"), &state);
        }
    }
}
