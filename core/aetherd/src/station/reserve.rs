//! Another session on the channel: how long a station that heard it holds its own calls
//! (10-step-3.md §3.2).
//!
//! A station that starts something of its own — a call, a probe, a beacon, a datagram — waits
//! for a clear channel, and the busy detector is what says so. Between the transmissions of a
//! session under granted slots the air is quiet for up to `max(D + KEY, T) + GUARD + LEVEL`
//! (ADR-0069), and the detector's hangover does not bridge it. On the scenario harness a
//! station that had waited 108 s for a gateway's session to end called in a 0.6 s gap, and the
//! session's next three transmissions went over its call, in four seeds of ten. In two of them
//! it called three seconds into a transmission it could not hear at all: it heard the gateway's
//! answers and not the client it answered.
//!
//! So a frame this station decodes of a session not its own holds its calls:
//!
//! * at least [`RESERVE_GAP_S`] past it, the longest quiet inside a session;
//! * past a DISC, [`RESERVE_DISC_S`], the `DISC_ACK`'s slot;
//! * and as long as the longest gap this station has heard between that activity's frames,
//!   which covers a side of the session it does not hear;
//! * until a `DISC_ACK` ends it, or nothing of it has been heard for [`RESERVE_MAX_S`];
//! * and after the `DISC_ACK`, through the session's closing Morse identifiers (ADR-0075): the
//!   one behind the `DISC_ACK`, the quiet of the slot's end and the other station's after it.
//!   Until the channel has been quiet [`RESERVE_GAP_S`], for at most [`RESERVE_CLOSING_S`].
//!
//! Like the 802.11 network allocation vector, a hold starts only on a frame decoded: a frame
//! that did not decode starts nothing. Only the closing's is lengthened by what the busy
//! detector hears, as any call already waits for a clear channel: a Morse identifier is no
//! frame, and nothing says how long one is.

/// The longest quiet inside a protocol 8 session, seconds: a slot opens at most
/// `max(D + KEY, T) + GUARD + LEVEL` after the frames before it — the longest turnaround a
/// station declares (1.5 s), the guard and the top gap level (2.0 s) — and a master keys again
/// as soon after a slot. A margin over 3.55 s.
pub const RESERVE_GAP_S: f64 = 4.0;

/// How long a DISC heard holds: its `DISC_ACK`'s slot, at most [`RESERVE_GAP_S`] off and a floor
/// control frame long (3.2 s).
pub const RESERVE_DISC_S: f64 = 7.5;

/// Silence after which an activity heard is over, and the gaps learned from it forgotten: a
/// transmission lasts at most the key's 30 s, and a slot at most as long again.
pub const RESERVE_MAX_S: f64 = 45.0;

/// The longest a session's closing holds after its `DISC_ACK`: two callsigns in Morse at the
/// slowest an identifier is usually sent, and the quiet between them (ADR-0075). A channel
/// busy with something else for longer is no closing, and the plain clear-channel wait has it.
pub const RESERVE_CLOSING_S: f64 = 40.0;

/// What a decoded frame of another session says of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    /// A frame of a session under way, or of one starting (a call, an acceptance).
    Session,
    /// A DISC: the session is ending, and its `DISC_ACK` is still to come.
    Leaving,
    /// A `DISC_ACK`: the session is over.
    Over,
}

/// The channel held by another session, as this station has heard it.
#[derive(Debug, Default, Clone, Copy)]
pub struct Reservation {
    /// When the last frame of the activity was heard.
    last: Option<f64>,
    /// The longest gap heard between two of its frames.
    cycle_s: f64,
    /// Until when this station's own calls wait.
    until: f64,
    /// A closing heard: when its `DISC_ACK` was, and when the channel was last heard busy since.
    closing: Option<(f64, f64)>,
}

impl Reservation {
    /// A frame of another session, decoded at `now`.
    pub fn heard(&mut self, now: f64, what: Heard) {
        match self.last {
            Some(last) if now - last <= RESERVE_MAX_S => {
                self.cycle_s = self.cycle_s.max(now - last);
            }
            _ => self.cycle_s = 0.0,
        }
        self.last = Some(now);
        self.closing = None;
        self.until = match what {
            Heard::Session => now + RESERVE_GAP_S.max(self.cycle_s),
            Heard::Leaving => now + RESERVE_DISC_S,
            Heard::Over => {
                self.last = None;
                self.cycle_s = 0.0;
                self.closing = Some((now, now));
                now + RESERVE_GAP_S
            }
        };
    }

    /// What the busy detector says of the channel at `now`: during a closing, a busy channel
    /// is its identifiers, and the hold lasts until the channel has been quiet a gap.
    pub fn channel(&mut self, now: f64, busy: bool) {
        let Some((at, mut last_busy)) = self.closing else {
            return;
        };
        if busy {
            last_busy = now;
        }
        self.until = (last_busy + RESERVE_GAP_S).min(at + RESERVE_CLOSING_S);
        self.closing = (now < self.until).then_some((at, last_busy));
    }

    /// Whether this station's own calls wait at `now`.
    #[must_use]
    pub fn holds(&self, now: f64) -> bool {
        now < self.until
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_heard_holds_for_the_longest_quiet_inside_a_session() {
        let mut reservation = Reservation::default();
        assert!(!reservation.holds(0.0));
        reservation.heard(10.0, Heard::Session);
        assert!(reservation.holds(10.0 + RESERVE_GAP_S - 0.01));
        assert!(!reservation.holds(10.0 + RESERVE_GAP_S + 0.01));
    }

    #[test]
    fn the_gaps_between_its_frames_cover_a_side_this_station_does_not_hear() {
        // a gateway's answers every ten seconds, the client's transmissions between them unheard
        let mut reservation = Reservation::default();
        for t in [0.0, 10.0, 20.0] {
            reservation.heard(t, Heard::Session);
        }
        assert!(
            reservation.holds(28.0),
            "eight seconds after an answer the client is still sending"
        );
        assert!(!reservation.holds(30.5));
    }

    #[test]
    fn a_disc_holds_for_its_answer_and_a_disc_ack_ends_it() {
        let mut reservation = Reservation::default();
        reservation.heard(0.0, Heard::Session);
        reservation.heard(9.0, Heard::Leaving);
        assert!(reservation.holds(9.0 + RESERVE_DISC_S - 0.01));
        reservation.heard(13.0, Heard::Over);
        // a closing with no identifier: the quiet of a gap, and it is over
        for tenth in 130..=170 {
            reservation.channel(f64::from(tenth) / 10.0, false);
        }
        assert!(!reservation.holds(13.0 + RESERVE_GAP_S + 0.01));
        // what follows is a new activity: the old gaps are forgotten
        reservation.heard(18.0, Heard::Session);
        assert!(!reservation.holds(18.0 + RESERVE_GAP_S + 0.01));
    }

    #[test]
    fn a_closing_holds_through_both_identifiers_and_the_quiet_between_them() {
        // the DISC_ACK at 10 s, its sender's identifier to 16 s, the quiet of the slot's end, the
        // other station's identifier from 18.5 s to 24.5 s (ADR-0075)
        let mut reservation = Reservation::default();
        reservation.heard(10.0, Heard::Over);
        let busy = |t: f64| t < 16.0 || (18.5..24.5).contains(&t);
        let mut released = None;
        for tenth in 100..400 {
            let t = f64::from(tenth) / 10.0;
            reservation.channel(t, busy(t));
            if released.is_none() && !reservation.holds(t) {
                released = Some(t);
            }
        }
        let released = released.expect("the closing ends");
        assert!(
            (24.5 + RESERVE_GAP_S - 0.2..=24.5 + RESERVE_GAP_S + 0.2).contains(&released),
            "calls wait out the second identifier and a gap, not {released}"
        );
    }

    #[test]
    fn a_channel_that_stays_busy_is_no_closing() {
        let mut reservation = Reservation::default();
        reservation.heard(0.0, Heard::Over);
        for second in 0..=60 {
            reservation.channel(f64::from(second), true);
        }
        assert!(!reservation.holds(RESERVE_CLOSING_S + 0.01));
    }

    #[test]
    fn an_activity_long_silent_is_over() {
        let mut reservation = Reservation::default();
        reservation.heard(0.0, Heard::Session);
        reservation.heard(20.0, Heard::Session);
        reservation.heard(20.0 + RESERVE_MAX_S + 1.0, Heard::Session);
        assert!(!reservation.holds(20.0 + RESERVE_MAX_S + 1.0 + RESERVE_GAP_S + 0.01));
    }
}
