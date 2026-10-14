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
//! * until a `DISC_ACK` ends it, or nothing of it has been heard for [`RESERVE_MAX_S`].
//!
//! Like the 802.11 network allocation vector, it rests only on frames decoded: a frame that did
//! not decode moves nothing.

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
        self.until = match what {
            Heard::Session => now + RESERVE_GAP_S.max(self.cycle_s),
            Heard::Leaving => now + RESERVE_DISC_S,
            Heard::Over => {
                self.last = None;
                self.cycle_s = 0.0;
                now
            }
        };
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
        assert!(!reservation.holds(13.0));
        // what follows is a new activity: the old gaps are forgotten
        reservation.heard(14.0, Heard::Session);
        assert!(!reservation.holds(14.0 + RESERVE_GAP_S + 0.01));
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
