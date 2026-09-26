//! Beacons at an interval, and what became of every beacon (ND1J, 2026-09-25: "where does one
//! see beacons received — and sent — and how does one set a beacon to repeat every so many
//! minutes?").
//!
//! A repeating beacon is the Beacon button pressed on a timer, with manners the button leaves
//! to the operator: it waits for a clear channel whatever `wait_for_clear` says, it waits for
//! a session, a probe or a test to finish, and a beacon it could not send within
//! [`BEACON_WINDOW_S`] of falling due is skipped, said so, and the next one kept to the
//! interval — a busy evening does not end in a burst of catching up. Each one goes through the
//! same queue and regulatory gate as a single beacon.
//!
//! It needs a control operator: a station under automatic control may not start one. A
//! beacon (§97.3(a)(9)) may be automatically controlled only in the segments of §97.203(d) —
//! 28.20–28.30 MHz, 50.06–50.08 MHz (inside 6 m's CW-only 50.0–50.1 MHz, so never for
//! Aether's data), 144.275–144.300 MHz and a few above — and an unattended station has no
//! business announcing itself anywhere else. It lasts until it is stopped or the modem restarts: nothing on the disk sets
//! a station beaconing on its own the next time it starts.

use serde_json::{Value, json};

use super::{State, Station};
use crate::ptt::Ptt;
use crate::regulatory::ControlMode;

/// The shortest interval a repeating beacon may have. A beacon is a transmission nobody asked
/// for; ten minutes is the §97.119 identification interval, and more often than that a
/// frequency starts to belong to the beacon.
pub const BEACON_EVERY_MIN_S: f64 = 600.0;

/// The longest: past four hours a timer is a forgotten one.
pub const BEACON_EVERY_MAX_S: f64 = 4.0 * 3600.0;

/// How long a due beacon waits for the station to be idle and the channel clear before it is
/// skipped. Long enough to ride out an over or a probe; short enough that a beacon still goes
/// out near when it was due.
pub const BEACON_WINDOW_S: f64 = 120.0;

/// The repeating beacon's timer, and what the beacons did.
#[derive(Debug, Clone, Default)]
pub(super) struct Beacons {
    /// The interval, seconds, while a repeating beacon runs.
    every_s: Option<f64>,
    /// Station time the next one falls due.
    pub(super) next_s: f64,
    /// Wall-clock milliseconds when the last beacon went on the air, repeating or not.
    last_sent_ms: Option<u64>,
    /// Beacons that went on the air since the modem started.
    sent: usize,
    /// Due beacons skipped: the station was not idle or the channel not clear in time.
    pub(super) skipped: usize,
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl<P: Ptt> Station<P> {
    /// Beacon every `seconds`, the first one now; `None` stops it.
    ///
    /// # Errors
    /// With the reason in a sentence for the operator: the interval out of range, an
    /// answer-only station, or one under automatic control.
    pub fn beacon_every(&mut self, seconds: Option<f64>) -> Result<(), String> {
        let Some(seconds) = seconds else {
            if self.beacons.every_s.take().is_some() {
                self.note("beacon", "repeating stopped");
            }
            return Ok(());
        };
        if !(BEACON_EVERY_MIN_S..=BEACON_EVERY_MAX_S).contains(&seconds) {
            return Err(format!(
                "a beacon repeats every {} to {} minutes",
                BEACON_EVERY_MIN_S / 60.0,
                BEACON_EVERY_MAX_S / 60.0
            ));
        }
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and sends no beacon".into());
        }
        if self.config.regulatory.control == Some(ControlMode::Automatic) {
            return Err(
                "a station under automatic control may not beacon on a timer: a beacon \
                        may be automatically controlled only on 28.20–28.30, 50.06–50.08, \
                        144.275–144.300, 222.05–222.06 or 432.300–432.400 MHz, or on 33 cm \
                        and up (§97.203(d))"
                    .into(),
            );
        }
        self.beacons.every_s = Some(seconds);
        self.beacons.next_s = self.now();
        self.note(
            "beacon",
            &format!("repeating every {} min", (seconds / 60.0).round()),
        );
        Ok(())
    }

    /// Send the repeating beacon when it is due, the station idle and the channel clear.
    pub(super) fn advance_beacons(&mut self, now: f64) {
        let Some(every) = self.beacons.every_s else {
            return;
        };
        if now < self.beacons.next_s {
            return;
        }
        // the rules may have changed under it: an answer-only or automatic station stops
        if self.config.answer_only || self.config.regulatory.control == Some(ControlMode::Automatic)
        {
            self.beacons.every_s = None;
            self.note(
                "beacon",
                "repeating stopped: the station is answer-only or under automatic control now",
            );
            return;
        }
        let idle = self.engine.state() == State::Idle
            && !self.test_running()
            && !self.engine.probing()
            && self.pending.is_empty()
            && !self.transmitting;
        let clear = self.busy.settled() && !self.busy.busy(now);
        if idle && clear {
            match self.beacon() {
                Ok(()) => self.beacons.next_s = now + every,
                Err(reason) => {
                    self.beacons.every_s = None;
                    self.note("beacon", &format!("repeating stopped: {reason}"));
                }
            }
            return;
        }
        if now - self.beacons.next_s >= BEACON_WINDOW_S {
            self.beacons.skipped += 1;
            // the next one keeps to the interval rather than catching up
            while self.beacons.next_s <= now {
                self.beacons.next_s += every;
            }
            let why = if idle {
                "the channel stayed busy"
            } else {
                "the station was busy"
            };
            self.note(
                "beacon",
                &format!(
                    "skipped: {why}; the next in {} min",
                    ((self.beacons.next_s - now) / 60.0).ceil()
                ),
            );
        }
    }

    /// A beacon went on the air: counted, timed and said.
    pub(super) fn beacon_sent(&mut self) {
        self.beacons.sent += 1;
        self.beacons.last_sent_ms = Some(unix_ms());
        // the timer's own next, which a beacon sent by hand in between does not move
        let detail = match self.beacons.every_s {
            Some(_) => format!(
                "sent; the next in {} min",
                ((self.beacons.next_s - self.now()) / 60.0).ceil().max(1.0)
            ),
            None => "sent".to_owned(),
        };
        self.note("beacon", &detail);
    }

    /// The beacon's part of `status`.
    #[must_use]
    pub fn beacon_status(&self) -> Value {
        let now = self.now();
        json!({
            "every_s": self.beacons.every_s,
            "next_in_s": self.beacons.every_s.map(|_| (self.beacons.next_s - now).max(0.0)),
            "last_sent_ms": self.beacons.last_sent_ms,
            "sent": self.beacons.sent,
            "skipped": self.beacons.skipped,
        })
    }
}
