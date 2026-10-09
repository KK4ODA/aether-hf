//! The answer gap each station has shown it needs, learned from the air (ADR-0054).
//!
//! `[radio] answer_gap_ms` (ADR-0036) holds this station's answers until a gap after the last
//! frame heard, for a station whose radio is keyed by VOX: its interface holds the transmitter
//! on for its DLY time after the audio stops, deaf, and an answer keyed into that hold is never
//! heard. One number for every station is either too short for the one with a long hold or a
//! cost on every exchange with all the others — KE4QCM's FT-100 lost KK4ODA-1's probe answers
//! with the gap at 500 ms (2026-10-09), and the other testers need none.
//!
//! So each station's gap is learned from the one sign that this station's answer to it was
//! lost: it asks again. A caller whose call was accepted calls again — the acceptance did
//! not reach it, and each try the station answers with its acceptance again and is not heard
//! says so again — or a station whose probe was answered probes again within a minute. Each
//! raises that station's gap a step, up to a ceiling; a session that went on after the
//! acceptance with no second call lowers it a little, so a gap learned on one bad evening does
//! not cost every exchange after it. The gap used is the larger of the operator's and the
//! learned one of the station answered; the learned ones are kept in the stations-heard list,
//! so a restarted daemon still knows them.
//!
//! A gap cannot help a station whose computer falls seconds behind its audio (KE4QCM's did,
//! the same morning): its stalls come at random, and no gap lines up with them. Nor is a
//! repeated call only ever a VOX hold — the acceptance may simply have faded — but a step
//! costs a quarter second on that station's exchanges and nothing on anyone else's, and the
//! clean sessions after take it back.

use std::collections::HashMap;

use super::bandwidth::base_callsign;

/// How much one sign of a lost first answer raises a station's gap, seconds.
pub const STEP_S: f64 = 0.25;
/// The most a station's gap is raised to, seconds: the longest DLY a VOX interface is set to
/// in practice, and the answer gap's own ceiling is 2 s.
pub const MOST_S: f64 = 2.0;
/// How much a session that went on after its acceptance lowers the gap, seconds: a tenth of a
/// step, so ten clean sessions undo one raise. At a third of a step the gap learned from a
/// 900 ms VOX hold fell under the hold within three sessions, and the fourth lost its message
/// (the scenario harness, `80m-vox-caller-learned-gap-500`, three seeds of four).
pub const EASE_S: f64 = 0.025;
/// How long after a probe was answered a second probe from the same station says the answer
/// was lost: a Test probes once, and an operator probing again is asking why there was none.
pub const PROBE_AGAIN_S: f64 = 60.0;
/// How long after an acceptance a second call from the same station says it was lost: a
/// caller tries again within a few of its frames, and later is a new call.
pub const CALL_AGAIN_S: f64 = 60.0;

/// What this station answered last, waiting to see whether it was heard.
#[derive(Debug, Clone, PartialEq)]
struct Answered {
    /// The other station's base callsign.
    callsign: String,
    /// When the answer was decided, in the station's seconds.
    at_s: f64,
    /// Whether this answer's loss has been counted already: a caller that never hears the
    /// acceptance calls three or four times, and that is one lost answer, not four.
    counted: bool,
    /// Whether an answer before this one, in the same exchange, was lost: a session that came
    /// up on a second acceptance says nothing in the gap's favour.
    after_loss: bool,
}

/// The gaps learned, and what is being watched.
#[derive(Debug, Default)]
pub struct LearnedGaps {
    /// Each station's learned gap, seconds, by base callsign.
    gaps: HashMap<String, f64>,
    /// The last call this station accepted.
    accepted: Option<Answered>,
    /// The last probe this station answered.
    probed: Option<Answered>,
    /// The station whose call or probe to this one was heard last: whom an answer goes to
    /// when no session is up.
    last_asker: Option<String>,
    /// The gaps that changed, for the stations-heard list to keep.
    changes: Vec<Learned>,
}

/// What a change to a station's gap was, for the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Learned {
    /// The station.
    pub callsign: String,
    /// Its gap now, seconds.
    pub gap_s: f64,
    /// Why it changed.
    pub why: &'static str,
}

impl LearnedGaps {
    /// The gap learned for a station, seconds; none when nothing has been learned.
    #[must_use]
    pub fn gap_s(&self, callsign: &str) -> Option<f64> {
        self.gaps
            .get(&base_callsign(callsign).to_ascii_uppercase())
            .copied()
    }

    /// The station whose call or probe was heard last.
    #[must_use]
    pub fn last_asker(&self) -> Option<&str> {
        self.last_asker.as_deref()
    }

    /// A call or probe to this station from `callsign` was heard.
    pub fn asked_by(&mut self, callsign: String) {
        self.last_asker = Some(callsign);
    }

    /// A gap changed: kept for the stations-heard list.
    pub fn changed(&mut self, learned: Learned) {
        self.changes.push(learned);
    }

    /// The gaps that changed since the last call.
    pub fn take_changes(&mut self) -> Vec<Learned> {
        std::mem::take(&mut self.changes)
    }

    /// A gap the stations-heard list kept from an earlier run.
    pub fn remember(&mut self, callsign: &str, gap_s: f64) {
        if gap_s > 0.0 {
            self.gaps.insert(
                base_callsign(callsign).to_ascii_uppercase(),
                gap_s.min(MOST_S),
            );
        }
    }

    /// This station accepted a call from `callsign` at `at_s`.
    pub fn accepted(&mut self, callsign: &str, at_s: f64) {
        let callsign = base_callsign(callsign).to_ascii_uppercase();
        let after_loss = self
            .accepted
            .as_ref()
            .is_some_and(|a| a.callsign == callsign && (a.counted || a.after_loss));
        self.accepted = Some(Answered {
            callsign,
            at_s,
            counted: false,
            after_loss,
        });
    }

    /// This station answered a probe from `callsign` at `at_s`.
    pub fn probe_answered(&mut self, callsign: &str, at_s: f64) {
        self.probed = Some(Answered {
            callsign: base_callsign(callsign).to_ascii_uppercase(),
            at_s,
            counted: false,
            after_loss: false,
        });
    }

    /// A call to this station from `callsign` at `at_s`: when it follows an acceptance of the
    /// same station's call, the acceptance was lost.
    pub fn called(&mut self, callsign: &str, at_s: f64) -> Option<Learned> {
        let call = base_callsign(callsign).to_ascii_uppercase();
        let lost = self.accepted.as_mut().filter(|a| {
            a.callsign == call && !a.counted && at_s - a.at_s <= CALL_AGAIN_S && at_s > a.at_s
        })?;
        lost.counted = true;
        Some(self.raise(&call, "it called again after its call was accepted"))
    }

    /// A probe to this station from `callsign` at `at_s`: when it follows an answer to the same
    /// station's probe, the answer was lost.
    pub fn probed(&mut self, callsign: &str, at_s: f64) -> Option<Learned> {
        let call = base_callsign(callsign).to_ascii_uppercase();
        let lost = self.probed.as_mut().filter(|a| {
            a.callsign == call && !a.counted && at_s - a.at_s <= PROBE_AGAIN_S && at_s > a.at_s
        })?;
        lost.counted = true;
        Some(self.raise(&call, "it probed again after its probe was answered"))
    }

    /// A frame of the session with `callsign` that is not a call: the session went on after
    /// the acceptance. Once a session, and only when the acceptance was heard the first time.
    pub fn session_went_on(&mut self, callsign: &str) -> Option<Learned> {
        let call = base_callsign(callsign).to_ascii_uppercase();
        let answered = self.accepted.take_if(|a| a.callsign == call)?;
        if answered.counted || answered.after_loss {
            return None;
        }
        let gap = self.gaps.get_mut(&call)?;
        // under a millisecond is none: the eases do not sum to a raise exactly in floating point
        *gap = if *gap - EASE_S < 0.001 {
            0.0
        } else {
            *gap - EASE_S
        };
        let gap_s = *gap;
        if gap_s <= 0.0 {
            self.gaps.remove(&call);
        }
        Some(Learned {
            callsign: call,
            gap_s,
            why: "its call was accepted the first time",
        })
    }

    fn raise(&mut self, call: &str, why: &'static str) -> Learned {
        let gap = self.gaps.entry(call.to_owned()).or_insert(0.0);
        *gap = (*gap + STEP_S).min(MOST_S);
        Learned {
            callsign: call.to_owned(),
            gap_s: *gap,
            why,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_lost_acceptance_raises_that_stations_gap_once() {
        let mut gaps = LearnedGaps::default();
        gaps.accepted("KE4QCM", 10.0);
        // the caller never heard it, and tries again: one lost answer, however many tries
        let learned = gaps.called("KE4QCM", 17.0).expect("a lost acceptance");
        assert_eq!(learned.gap_s, STEP_S);
        assert!(gaps.called("KE4QCM", 24.0).is_none(), "one lost answer");
        // the station answers the try with its acceptance again, and that one is lost too
        gaps.accepted("KE4QCM", 24.0);
        let learned = gaps
            .called("KE4QCM", 31.0)
            .expect("a second lost acceptance");
        assert!((learned.gap_s - 2.0 * STEP_S).abs() < 1e-9);
        // the third acceptance is heard and the session goes on: a session that needed three
        // does not ease the gap
        gaps.accepted("KE4QCM", 31.0);
        assert!(gaps.session_went_on("KE4QCM").is_none());
        assert!((gaps.gap_s("KE4QCM").unwrap_or(0.0) - 2.0 * STEP_S).abs() < 1e-9);
        // nobody else's
        assert_eq!(gaps.gap_s("ND1J"), None);
    }

    #[test]
    fn a_call_from_another_station_or_much_later_says_nothing() {
        let mut gaps = LearnedGaps::default();
        gaps.accepted("KE4QCM", 10.0);
        assert!(gaps.called("ND1J", 15.0).is_none());
        assert!(gaps.called("KE4QCM", 10.0 + CALL_AGAIN_S + 1.0).is_none());
        // and a call before the acceptance is the one it accepted
        gaps.accepted("KE4QCM", 100.0);
        assert!(gaps.called("KE4QCM", 99.0).is_none());
    }

    #[test]
    fn a_second_probe_after_an_answer_raises_it_too() {
        let mut gaps = LearnedGaps::default();
        gaps.probe_answered("KE4QCM", 0.0);
        let learned = gaps.probed("KE4QCM", 20.0).expect("a lost answer");
        assert_eq!(learned.gap_s, STEP_S);
        assert!(gaps.probed("KE4QCM", 30.0).is_none());
        // a probe a long while later is a new question
        gaps.probe_answered("KE4QCM", 100.0);
        assert!(gaps.probed("KE4QCM", 100.0 + PROBE_AGAIN_S + 1.0).is_none());
    }

    #[test]
    fn the_gap_rises_to_its_ceiling_and_eases_back_with_clean_sessions() {
        let mut gaps = LearnedGaps::default();
        for i in 0..20 {
            let t = f64::from(i) * 100.0;
            gaps.accepted("KE4QCM", t);
            gaps.called("KE4QCM", t + 5.0);
        }
        assert_eq!(gaps.gap_s("KE4QCM"), Some(MOST_S));
        // a session whose acceptance was lost does not ease it
        gaps.accepted("KE4QCM", 5000.0);
        gaps.called("KE4QCM", 5005.0);
        assert!(gaps.session_went_on("KE4QCM").is_none());
        // one heard the first time does, a step smaller than a raise
        gaps.accepted("KE4QCM", 6000.0);
        let eased = gaps.session_went_on("KE4QCM").expect("eased");
        assert!((eased.gap_s - (MOST_S - EASE_S)).abs() < 1e-9);
        // and only once a session
        assert!(gaps.session_went_on("KE4QCM").is_none());
    }

    #[test]
    fn an_eased_gap_is_forgotten_at_zero_and_ssids_are_one_station() {
        let mut gaps = LearnedGaps::default();
        gaps.accepted("KE4QCM-1", 0.0);
        gaps.called("KE4QCM", 5.0);
        assert_eq!(gaps.gap_s("ke4qcm-7"), Some(STEP_S));
        // ten clean sessions undo one raise (the first, after the raise, eases nothing)
        for i in 0..9 {
            gaps.accepted("KE4QCM", 100.0 * f64::from(i + 1));
            gaps.session_went_on("KE4QCM");
        }
        assert!(gaps.gap_s("KE4QCM").is_some_and(|gap| gap > 0.0));
        for i in 9..11 {
            gaps.accepted("KE4QCM", 100.0 * f64::from(i + 1));
            gaps.session_went_on("KE4QCM");
        }
        assert_eq!(gaps.gap_s("KE4QCM"), None);
        assert!(gaps.gaps.is_empty());
    }

    #[test]
    fn a_remembered_gap_is_kept_under_the_ceiling() {
        let mut gaps = LearnedGaps::default();
        gaps.remember("KE4QCM", 9.0);
        gaps.remember("ND1J", 0.0);
        assert_eq!(gaps.gap_s("KE4QCM"), Some(MOST_S));
        assert_eq!(gaps.gap_s("ND1J"), None);
    }
}
