//! How long this station takes to put a transmission on the air, learned from its own keyings
//! (ADR-0071; 11-step-4-first-air.md §4.3 and §12, change 1).
//!
//! The engine schedules on this station's own keying latency: it decides an answer that much
//! ahead of its slot, forfeits one decided too late for it, asks for a wider gap when it is
//! slower than the protocol allows, and keys its own transmissions that much ahead of where
//! they belong. It had been told the key lead and the sound card's reported output latency —
//! 0.122 s at the mini PC by the radio — and the keying gate judged a transmission's start on
//! the same figure. Two waits nobody counted lay between the decision and the air:
//!
//! * rendering the transmission, done in the pass that keys it unless the engine decided it
//!   ahead: 30–80 ms for one tone control frame on the mini PC;
//! * the radio's keying command, which the run loop waits for (`ThreadedPtt::key`): 19 ms for
//!   the FTDX10's `TX1;`, about 45–65 ms for an IC-7300's CI-V at 9600 Bd.
//!
//! Protocol 8's first session on the air had every answer on the air 76–197 ms after its
//! slot's start, and the gate's `not_after` judged an estimate short by the same: S2 held by
//! margin, not by construction. Each wait is learned here as the run of its recent values. The
//! typical one (the median) is what the engine is told, so it decides that much earlier; the
//! gate holds a transmission until even the fastest recent key would not put it on the air
//! early, and once the key is up, judges the start against `not_after` with the wait measured.
//! Render time is measured on the sound card's own clock, so a harness whose clock stands still
//! while the daemon works (the lockstep channel, ADR-0042) learns none, and a keying wait under
//! half a millisecond — a serial line's, a harness's — counts as none.

use std::collections::VecDeque;

/// How many recent waits each estimate is taken over: enough that one slow command does not
/// move the typical, few enough that a radio swapped mid-run is learned within minutes.
const KEPT: usize = 9;

/// The recent run of one kind of wait, in seconds.
#[derive(Debug, Default, Clone)]
pub struct Recent {
    waits: VecDeque<f64>,
}

impl Recent {
    /// One more wait, to the millisecond: under half of one is none, so a station keyed by a
    /// serial line or a harness keeps exactly the latency it had.
    pub fn push(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds < 0.0 {
            return;
        }
        self.waits.push_back((seconds * 1000.0).round() / 1000.0);
        if self.waits.len() > KEPT {
            self.waits.pop_front();
        }
    }

    fn sorted(&self) -> Vec<f64> {
        let mut waits: Vec<f64> = self.waits.iter().copied().collect();
        waits.sort_by(f64::total_cmp);
        waits
    }

    /// The middle of the recent waits (the upper of the two middles of an even run); none
    /// before the first.
    #[must_use]
    pub fn typical(&self) -> f64 {
        let waits = self.sorted();
        waits.get(waits.len() / 2).copied().unwrap_or(0.0)
    }

    /// The shortest recent wait; none before the first.
    #[must_use]
    pub fn least(&self) -> f64 {
        self.sorted().first().copied().unwrap_or(0.0)
    }

    /// The longest recent wait; none before the first.
    #[must_use]
    pub fn most(&self) -> f64 {
        self.sorted().last().copied().unwrap_or(0.0)
    }

    /// How many waits the estimates are taken over.
    #[must_use]
    pub fn count(&self) -> usize {
        self.waits.len()
    }
}

/// The two waits between deciding a transmission and its first sample leaving the sound card.
#[derive(Debug, Default, Clone)]
pub struct KeyingLatency {
    /// The radio's keying command: from asking for the key to the run loop going on.
    pub key: Recent,
    /// Rendering a transmission of the engine's, on the sound card's clock.
    pub render: Recent,
}

impl KeyingLatency {
    /// What a transmission typically waits beyond the key lead and the card's output latency:
    /// what the engine is told on top of them.
    #[must_use]
    pub fn typical_s(&self) -> f64 {
        self.key.typical() + self.render.typical()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wait in whole milliseconds, which is what is kept.
    fn ms(seconds: f64) -> i64 {
        (seconds * 1000.0).round() as i64
    }

    #[test]
    fn nothing_is_learned_before_the_first_wait() {
        let latency = KeyingLatency::default();
        assert_eq!(ms(latency.typical_s()), 0);
        assert_eq!(ms(latency.key.least()), 0);
        assert_eq!(ms(latency.key.most()), 0);
    }

    #[test]
    fn the_typical_wait_is_the_middle_of_the_recent_ones() {
        // the FTDX10 of 2026-10-14: 19.4 ms five times in seven, 28 and 48 ms once each
        let mut key = Recent::default();
        for ms in [19.4, 19.4, 28.0, 19.4, 48.0, 19.4, 19.4] {
            key.push(ms / 1000.0);
        }
        assert_eq!(ms(key.typical()), 19);
        assert_eq!(ms(key.least()), 19);
        assert_eq!(ms(key.most()), 48);
    }

    #[test]
    fn only_the_recent_waits_count() {
        let mut render = Recent::default();
        for _ in 0..KEPT {
            render.push(0.080);
        }
        for _ in 0..KEPT {
            render.push(0.030);
        }
        assert_eq!(render.count(), KEPT);
        assert_eq!(ms(render.typical()), 30);
    }

    #[test]
    fn a_wait_under_a_millisecond_is_none_and_nonsense_is_ignored() {
        let mut key = Recent::default();
        key.push(0.000_4);
        key.push(f64::NAN);
        key.push(-0.01);
        assert_eq!(key.count(), 1);
        assert_eq!(ms(key.typical()), 0);
    }

    #[test]
    fn the_engine_is_told_both_typical_waits() {
        let mut latency = KeyingLatency::default();
        latency.key.push(0.019);
        latency.render.push(0.045);
        assert!((latency.typical_s() - 0.064).abs() < 1e-12);
    }
}
