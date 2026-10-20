//! The Test session (P6-7): a fixed sequence a volunteer runs against another station,
//! recorded, so that every such contact leaves a sidecar the bench can replay and a
//! ladder of frame error rates per mode on a real path.
//!
//! The sequence: a probe (both directions' SNR, ADR-0006); a session; a message of
//! incompressible bytes, timed; a file, sized from the message's rate; the **mode ladder** —
//! a measurement at each mode from a rung under the fastest the transfers were acknowledged at,
//! the other station's report of it kept as a rung, until three rungs in a row decode fewer
//! than half their frames; and an orderly disconnect. The other station needs nothing but to be
//! listening: it answers the probe and the call and acknowledges what it decodes, as it would
//! for any session. What the run learns goes into the recording's sidecar under
//! `session.test`, with the operator's grid, rig, power and antenna from `[operator]` beside
//! it, and `test.status` reports the same while it runs and after it ends.
//!
//! **The Test measures the link the way an operator uses it, then measures the rungs in a way
//! that cannot disturb that** (ADR-0074). The message and the file go first, as any transfer
//! would, and the rates the run reports are the link's own. The ladder comes after them, and a
//! rung is a **measurement** (ADR-0077, link protocol 9): frames of their own kind, filler the
//! rung's full size, sent once in a transmission of their own, never delivered and never sent
//! again; the other station's acknowledgement reports which arrived. The ladder had pinned the
//! session's own data at each rung, so a frame that failed there was data still to deliver: on
//! ND1J's Tests of 2026-10-15 those frames were sent again at the rung that failed, and on WC4Y's
//! Test of 2026-10-17, re-encoded at the rung asked for, delivering them took 93 s of 435. Now a
//! rung follows the last one's report at once, and the run moves to the disconnect at the last
//! rung's. The ladder climbs only the rungs the link uses: one another rung beats on both data
//! rate and threshold is never recommended (ADR-0073), and measuring it measured nothing the
//! link would do (the 500 Hz rungs 4 and 8 on that Test). The file leaves the ladder its time
//! ([`LADDER_RESERVE_S`]): with the file last, three Tests with ND1J on a slow path (2026-09-25)
//! spent the budget on the transfers and never reached a rung.

use aether_link::State;
use serde_json::{Value, json};

use super::Station;
use crate::ptt::Ptt;

/// What a Test session is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub struct TestPlan {
    /// The station to test with.
    pub remote: String,
    /// Which of this station's callsigns to use; the first, when not said.
    pub as_call: Option<String>,
    /// The other station's Maidenhead locator, when the operator knows it: the path
    /// length comes from the two grids.
    pub remote_grid: Option<String>,
    /// The most the message may be, bytes; zero skips it. The probe's SNR sizes the
    /// message under this (see [`message_size_for`]).
    pub message_bytes: usize,
    /// The most the file may be, bytes; zero skips it. The message's measured rate sizes
    /// the file under this, to about two minutes' worth (see [`file_size_for`]).
    pub file_bytes: usize,
    /// Whether to climb the mode ladder after the transfers.
    pub ladder: bool,
    /// Frames per rung.
    pub rung_frames: usize,
    /// The whole run's time budget, seconds: a step that would not fit in what is left is
    /// shortened or skipped, and the disconnect is still orderly.
    pub budget_s: f64,
}

impl Default for TestPlan {
    fn default() -> Self {
        Self {
            remote: String::new(),
            as_call: None,
            remote_grid: None,
            message_bytes: 2048,
            file_bytes: 16_384,
            ladder: true,
            rung_frames: 4,
            budget_s: 600.0,
        }
    }
}

/// The most bytes a transfer step may be asked for.
const MAX_TRANSFER_BYTES: usize = 262_144;

impl TestPlan {
    /// From a control-API request: `remote` is required, the rest has defaults.
    ///
    /// # Errors
    /// When the callsign is missing, a grid is not a locator, or a size is not a number.
    pub fn from_params(params: &Value) -> Result<Self, String> {
        let remote = params
            .get("remote")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("a callsign to test with is required: {\"remote\": \"KK4XYZ\"}")?;
        let mut plan = Self {
            remote: remote.to_ascii_uppercase(),
            ..Self::default()
        };
        plan.as_call = params
            .get("callsign")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_ascii_uppercase())
            .filter(|s| !s.is_empty());
        plan.remote_grid = params
            .get("remote_grid")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        if let Some(grid) = &plan.remote_grid
            && crate::grid::locator(grid).is_none()
        {
            return Err(format!(
                "{grid:?} is not a Maidenhead locator (EM73 or EM73tv)"
            ));
        }
        let size = |key: &str, default: usize| -> Result<usize, String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(default),
                Some(value) => value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|&n| n <= MAX_TRANSFER_BYTES)
                    .ok_or_else(|| {
                        format!("{key} must be a number of bytes up to {MAX_TRANSFER_BYTES}")
                    }),
            }
        };
        plan.message_bytes = size("message_bytes", plan.message_bytes)?;
        plan.file_bytes = size("file_bytes", plan.file_bytes)?;
        plan.ladder = params
            .get("ladder")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        plan.rung_frames = size("rung_frames", plan.rung_frames)?.clamp(1, 16);
        plan.budget_s = match params.get("budget_s") {
            None | Some(Value::Null) => plan.budget_s,
            Some(value) => value
                .as_f64()
                .filter(|b| (60.0..=3600.0).contains(b))
                .ok_or("budget_s must be seconds, 60 to 3600")?,
        };
        Ok(plan)
    }
}

/// Where a run is in its sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// One probe frame and its answer.
    Probe,
    /// The call, until the session is up.
    Connect,
    /// The message, until acknowledged.
    Message,
    /// The file, until acknowledged.
    File,
    /// The mode ladder, a rung at a time.
    Ladder,
    /// The orderly close.
    Disconnect,
    /// Over, with an outcome.
    Done,
}

impl Step {
    fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe",
            Self::Connect => "connect",
            Self::Message => "message",
            Self::File => "file",
            Self::Ladder => "ladder",
            Self::Disconnect => "disconnect",
            Self::Done => "done",
        }
    }
}

/// A timed transfer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transfer {
    /// Application bytes sent.
    pub bytes: usize,
    /// From the send to the last acknowledgement.
    pub seconds: f64,
}

impl Transfer {
    /// Goodput, bits per second.
    #[must_use]
    pub fn bps(&self) -> f64 {
        if self.seconds > 0.0 {
            self.bytes as f64 * 8.0 / self.seconds
        } else {
            0.0
        }
    }

    fn json(self) -> Value {
        json!({ "bytes": self.bytes, "seconds": round1(self.seconds), "bps": round1(self.bps()) })
    }
}

/// One rung of the ladder: a measurement at a mode and what the other station reported of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rung {
    /// The mode measured.
    pub mode: usize,
    /// The measurement's frames.
    pub frames: usize,
    /// Of those, the ones the other station reported arrived.
    pub decoded: usize,
    /// The SNR it measured on them; `None` when the report came after a poll.
    pub snr_db: Option<f64>,
    /// From the measurement's asking to its report.
    pub seconds: f64,
}

impl Rung {
    fn failed(&self) -> bool {
        self.decoded * 2 < self.frames
    }

    fn json(self) -> Value {
        json!({
            "mode": self.mode,
            "frames": self.frames,
            "decoded": self.decoded,
            "snr_db": self.snr_db,
            "seconds": round1(self.seconds),
        })
    }
}

/// How many of the ladder's `modes` it skips when the message ran at `rung`: those under the
/// rung below it on the ladder. The ladder starts a rung under the message's so its first rung
/// is one the path has just carried: started at the message's own rung, a burst of RTTY in the
/// harness failed it and the two above, and the ladder stopped having passed nothing.
pub fn skipped_rungs(modes: &[usize], rung: usize) -> usize {
    let under = modes
        .iter()
        .copied()
        .filter(|&m| m < rung)
        .max()
        .unwrap_or(rung);
    modes
        .iter()
        .take_while(|&&m| m < under)
        .count()
        .min(modes.len().saturating_sub(1))
}

/// The rungs the link uses, which the ladder climbs: those no other rung beats on both the
/// data it carries a second and its threshold — the rate controller's own front
/// (`usable_modes_by_rate`, ADR-0073). WC4Y's Test of 2026-10-17 pinned the 500 Hz rungs 4 and
/// 8, which the link never recommends; rung 8's slot went silent and its four frames were
/// re-encoded and delivered as filler, about 35 s of the Test (13-wc4y-beta111.md §10).
pub fn ladder_rungs(timing: &aether_link::PhyTiming) -> Vec<usize> {
    let rungs = timing.data_capacity.len();
    if timing.mode_threshold_db.is_empty() {
        // the wide air's table, which a timing leaves empty (`PhyTiming::mode_threshold_db`)
        let wide = aether_link::rate::usable_modes();
        return (0..rungs).filter(|m| wide.contains(m)).collect();
    }
    if timing.mode_threshold_db.len() != rungs {
        return (0..rungs).collect();
    }
    let frame_s: Vec<f64> = (0..rungs).map(|m| timing.data_frame_s_for(m)).collect();
    aether_link::rate::usable_modes_by_rate(
        &timing.mode_threshold_db,
        &timing.data_capacity,
        &frame_s,
    )
}

/// Rungs in a row that may fail before the ladder stops.
const LADDER_FAILS: usize = 3;

/// How long the file should take at the rate the message measured.
const FILE_TARGET_S: f64 = 120.0;

/// The time the file leaves for the ladder that follows it. ND1J's weak-path Tests of
/// 2026-10-15 took 140–170 s to measure their rungs (the four-tone rungs a 26 s exchange each,
/// a failed rung half a minute more, most of it its leftovers); a rung is now its measurement's
/// one exchange (ADR-0077).
const LADDER_RESERVE_S: f64 = 150.0;

/// The least time left in the budget for the ladder to start another rung: a tone rung's
/// measurement, its report after a poll if its answer is lost, and the disconnect.
const LADDER_RUNG_LEFT_S: f64 = 60.0;

/// The message the probe's SNR earns, under the plan's ceiling: half a kilobyte on a
/// path heard below 0 dB, a kilobyte below 6 dB or when the probe went unanswered, the
/// ceiling above that. The message is the first transfer and the one that measures the
/// rate the file is sized from, so it is kept short where the rate will be low.
///
/// The SNR is the one the message travels at: how the other station hears this one, the
/// probe's `heard_there_db` — or how this one hears it when the answer did not say. It
/// was sized from `heard_here_db`, the other direction: ND1J at 100 W heard KK4ODA at
/// −1 dB and was heard at +5, and a kilobyte took five minutes of a ten-minute budget.
#[must_use]
pub fn message_size_for(snr_db: Option<f64>, ceiling: usize) -> usize {
    let earned = match snr_db {
        Some(snr) if snr < 0.0 => 512,
        Some(snr) if snr < 6.0 => 1024,
        Some(_) => usize::MAX,
        None => 1024,
    };
    ceiling.min(earned)
}

/// The SNR a message sent to the other station travels at, from the probe: how it hears
/// this station, or — when its answer did not say — how this one hears it.
#[must_use]
pub fn message_snr(probe: Option<(Option<f64>, f64)>) -> Option<f64> {
    probe.map(|(there, here)| there.unwrap_or(here))
}

/// The SNR a message travels at when the probe went unanswered, from the call: how the other
/// station said it hears this one, or — when nothing of the session has said — how this one
/// heard the acceptance.
#[must_use]
pub fn message_snr_after_call(heard_there: Option<f64>, heard_here: Option<f64>) -> Option<f64> {
    heard_there.or(heard_here)
}

/// The least file worth sending, bytes.
const FILE_MIN: usize = 256;

/// The file the measured rate earns: about two minutes' worth, never past the ceiling, and
/// nothing at all when the time left in the budget is under a minute or would carry less than
/// [`FILE_MIN`]. Rounded down to 256 bytes so the number reads as what it is. It had a floor of
/// a kilobyte, which on the 80 m evening scenario (19–36 bit/s) was four to seven minutes the
/// budget did not have: the Test ended "the file timed out" every time (the scenario harness,
/// 2026-10-07).
#[must_use]
pub fn file_size_for(message_bps: f64, ceiling: usize, remaining_s: f64) -> usize {
    if ceiling == 0 || remaining_s < 60.0 || message_bps <= 0.0 {
        return 0;
    }
    let seconds = FILE_TARGET_S.min(remaining_s - 60.0);
    let earned = (message_bps * seconds / 8.0) as usize / 256 * 256;
    if earned < FILE_MIN.min(ceiling) {
        return 0;
    }
    earned.min(ceiling)
}

/// A Test session in progress, or the last one run.
#[derive(Debug, Clone)]
pub struct TestRun {
    /// What was asked for.
    pub plan: TestPlan,
    /// Where it is.
    pub step: Step,
    /// The probe's answer: the SNR the other station measured on it, and the SNR this one
    /// measured on the answer. `None` when it went unanswered.
    pub probe: Option<(Option<f64>, f64)>,
    /// The message transfer, once acknowledged.
    pub message: Option<Transfer>,
    /// The file transfer, once acknowledged.
    pub file: Option<Transfer>,
    /// The ladder's rungs, from the floor up.
    pub rungs: Vec<Rung>,
    /// `complete`, or `aborted: <why>`, once the run is over.
    pub outcome: Option<String>,
    started_s: f64,
    started: String,
    step_started_s: f64,
    /// The station's time held for a clear channel when this step began: a step's clock does
    /// not run while the station waits to transmit ([`TestRun::step_elapsed_s`]).
    held_at_step: f64,
    entered: bool,
    rung_reported: bool,
    ladder_next: usize,
    ladder_fails: usize,
    modes: Vec<usize>,
    recorded: bool,
    seed: u64,
    failure: Option<String>,
    /// When the run ended, so the elapsed time stops with it.
    ended_s: Option<f64>,
    message_size: usize,
    file_size: usize,
    /// The engine's acknowledged-bytes count when the transfer now running began.
    acked_at_step: usize,
    /// The message, queued with the call when the probe had sized it: what the outgoing
    /// buffer held before it, to take it back out if the call is not answered.
    message_queued: Option<usize>,
    /// The ladder's rungs left out under the rung the message ran at, once it has run.
    ladder_skipped: Option<usize>,
    /// The fastest rung that passed, once one has.
    highest_passed: Option<usize>,
    /// What was shortened or skipped, and why, for the report.
    pub adjustments: Vec<String>,
}

impl TestRun {
    fn new(plan: TestPlan, now: f64, modes: Vec<usize>, seed: u64) -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
        Self {
            plan,
            step: Step::Probe,
            probe: None,
            message: None,
            file: None,
            rungs: Vec::new(),
            outcome: None,
            started_s: now,
            started: crate::log::rfc3339(ms),
            step_started_s: now,
            held_at_step: 0.0,
            entered: false,
            rung_reported: false,
            ladder_next: 0,
            ladder_fails: 0,
            modes,
            recorded: false,
            seed,
            failure: None,
            ended_s: None,
            message_size: 0,
            file_size: 0,
            acked_at_step: 0,
            message_queued: None,
            ladder_skipped: None,
            highest_passed: None,
            adjustments: Vec::new(),
        }
    }

    fn remaining_s(&self, now: f64) -> f64 {
        self.plan.budget_s - (now - self.started_s)
    }

    /// Whether the run is still going.
    #[must_use]
    pub fn running(&self) -> bool {
        self.step != Step::Done
    }

    fn enter(&mut self, step: Step, now: f64, held_s: f64) {
        self.step = step;
        self.step_started_s = now;
        self.held_at_step = held_s;
        self.entered = false;
        self.rung_reported = false;
    }

    /// How long this step has run, less the time the station held a transmission for a clear
    /// channel meanwhile (`held_s`, the station's running total): a step's deadline counts from
    /// the transmission. The probe and the call are what such a hold can delay; in a session
    /// nothing waits for the channel. On a contest frequency a Test's probe was held 29.7 s,
    /// the step's 30 s ran out as the answer arrived, and it called on "no answer"
    /// (10-step-3.md §3.7).
    fn step_elapsed_s(&self, now: f64, held_s: f64) -> f64 {
        now - self.step_started_s - (held_s - self.held_at_step).max(0.0)
    }

    /// Incompressible bytes, different for every step, the same for every run with this
    /// seed: what a transfer is measured with, and never anything an operator wrote.
    fn test_bytes(&self, count: usize, salt: u64) -> Vec<u8> {
        let mut state = self.seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 24) as u8
            })
            .collect()
    }

    /// The run as `test.status` and the sidecar report it.
    #[must_use]
    pub fn json(&self, now: f64, my_grid: &str, bandwidth_hz: usize) -> Value {
        let their_grid = self.plan.remote_grid.as_deref();
        let km = their_grid.and_then(|theirs| crate::grid::path_km(my_grid, theirs));
        json!({
            "remote": self.plan.remote,
            "started": self.started,
            "elapsed_s": round1(self.ended_s.unwrap_or(now) - self.started_s),
            "step": self.step.name(),
            "outcome": self.outcome,
            "bandwidth_hz": bandwidth_hz,
            "probe": self.probe.map(|(there, here)| json!({
                "heard_there_db": there,
                "heard_here_db": round1(here),
            })),
            "message": self.message.map(Transfer::json),
            "file": self.file.map(Transfer::json),
            "ladder": self.rungs.iter().copied().map(Rung::json).collect::<Vec<_>>(),
            "ladder_rungs": self.modes.len(),
            "highest_passed": self.highest_passed,
            "budget_s": self.plan.budget_s,
            "adjustments": self.adjustments,
            "path": {
                "my_grid": (!my_grid.is_empty()).then_some(my_grid),
                "their_grid": their_grid,
                "km": km.map(f64::round),
            },
        })
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// How long a transfer of `bytes` may take before the run gives up on it: a backstop
/// behind the link's own timeout, generous enough for the floor modes.
fn transfer_timeout_s(bytes: usize) -> f64 {
    300.0 + bytes as f64 * 8.0 / 25.0
}

/// How long a rung may take: a burst, its acknowledgement, and the retransmissions of a
/// rung the path could not carry until they are re-encoded and through.
const RUNG_TIMEOUT_S: f64 = 300.0;
/// How long the probe and the call may take, not counting the time the station held them for a
/// clear channel ([`TestRun::step_elapsed_s`]): backstops behind the engine's own timers, which
/// move with every hold too. A channel that never clears ends the run with its budget.
const PROBE_TIMEOUT_S: f64 = 30.0;
const CONNECT_TIMEOUT_S: f64 = 120.0;
const DISCONNECT_TIMEOUT_S: f64 = 90.0;

impl<P: Ptt> Station<P> {
    /// Begin a Test session with another station. Refused on an answer-only station,
    /// during a session or a probe, and while a Test session is running.
    ///
    /// Whether the rules let this station start an exchange here is asked before, as it is for
    /// a call: the control API refuses `test.start` with the decision (`regulatory`), and the
    /// gate judges every burst the run makes.
    ///
    /// # Errors
    /// With the reason, in a sentence for the operator.
    pub fn start_test(&mut self, plan: TestPlan) -> Result<(), String> {
        if self.config.answer_only {
            return Err("this station is answer-only: it takes calls and makes none".into());
        }
        if self.test_running() {
            return Err("a test session is already running".into());
        }
        if self.engine.state() != State::Idle {
            return Err("a session is running".into());
        }
        if self.engine.probing() {
            return Err("a probe is already out".into());
        }
        if let Some(call) = &plan.as_call
            && !self.engine.callsigns.contains(call)
        {
            return Err(format!("{call} is not one of this station's callsigns"));
        }
        // the ladder climbs only as far as the rules allow here (ADR-0018), and only the rungs
        // the link uses (ADR-0076)
        let ceiling = self.engine.ceiling().map_or(usize::MAX, |c| c + 1);
        let timing = self.engine.timing();
        let usable = ladder_rungs(timing);
        let modes: Vec<usize> = (0..timing
            .data_capacity
            .len()
            .min(self.config.link.max_mode + 1)
            .min(ceiling))
            .filter(|m| usable.contains(m))
            .collect();
        let now = self.now();
        let seed = self
            .engine
            .my_call
            .bytes()
            .fold(0x5DEE_CE66_D1CE_4E5Du64, |h, b| {
                h.rotate_left(5) ^ u64::from(b)
            })
            ^ (now * 1000.0) as u64;
        let mut run = TestRun::new(plan, now, modes, seed);
        run.held_at_step = self.held_total_s;
        let name = format!(
            "{}_test",
            crate::record::session_name(&self.engine.my_call, Some(&run.plan.remote))
        );
        let notes = self.record_notes.take().or_else(|| {
            (!self.config.record_notes.is_empty()).then(|| self.config.record_notes.clone())
        });
        match self.start_recording(Some(&name), notes.as_deref()) {
            Ok(_) => run.recorded = true,
            Err(reason) => self.note("test", &format!("not recorded: {reason}")),
        }
        self.note("test", &format!("started with {}", run.plan.remote));
        self.test = Some(run);
        self.advance_test();
        Ok(())
    }

    /// Stop a running Test session: the session is aborted and the outcome recorded.
    /// Returns whether one was running.
    pub fn abort_test(&mut self) -> bool {
        let Some(mut run) = self.test.take() else {
            return false;
        };
        if !run.running() {
            self.test = Some(run);
            return false;
        }
        if self.engine.state() == State::Idle {
            self.finish_test(&mut run, "stopped by the operator");
        } else {
            run.failure = Some("stopped by the operator".into());
            self.abort();
            run.enter(Step::Disconnect, self.now(), self.held_total_s);
            run.entered = true;
        }
        self.test = Some(run);
        true
    }

    /// Whether a Test session is running.
    #[must_use]
    pub fn test_running(&self) -> bool {
        self.test_stepping || self.test.as_ref().is_some_and(TestRun::running)
    }

    /// The Test session, running or last run, as `test.status` reports it.
    #[must_use]
    pub fn test_status(&self) -> Value {
        match &self.test {
            Some(run) => json!({
                "running": run.running(),
                "results": run.json(
                    self.now(),
                    &self.config.operator.grid,
                    self.config.params.bandwidth.hz(),
                ),
            }),
            None => json!({ "running": false, "results": Value::Null }),
        }
    }

    /// A running Test session's progress for `status`, or nothing: the step; for a
    /// transfer the bytes acknowledged of the total; for the ladder the rung under test out
    /// of all of them, the fastest that has passed and the failures in a row that stop it;
    /// the rung the link is using now and how the other station hears it; and the time,
    /// elapsed and at most left. No countdown: how long a step takes is the path's to say.
    #[must_use]
    pub fn test_brief(&self) -> Value {
        let Some(run) = self.test.as_ref().filter(|run| run.running()) else {
            return Value::Null;
        };
        let now = self.now();
        let air = self.air();
        let ladder = air.ladder();
        let name = |mode: usize| ladder.get(mode).map(aether_phy::modes::Rung::name);
        let transfer = match run.step {
            Step::Message | Step::File if run.entered => {
                let total = if run.step == Step::Message {
                    run.message_size
                } else {
                    run.file_size
                };
                let acked = self
                    .engine
                    .stats
                    .bytes_acked
                    .saturating_sub(run.acked_at_step)
                    .min(total);
                json!({ "bytes": total, "acked": acked })
            }
            _ => Value::Null,
        };
        let testing = (run.step == Step::Ladder && run.ladder_next < run.modes.len())
            .then(|| run.modes[run.ladder_next]);
        let last = run.rungs.last().copied().map(Rung::json);
        json!({
            "remote": run.plan.remote,
            "step": run.step.name(),
            "elapsed_s": round1(now - run.started_s),
            "budget_s": run.plan.budget_s,
            "remaining_s": round1(run.remaining_s(now).max(0.0)),
            "rungs": run.rungs.len(),
            "transfer": transfer,
            "ladder": {
                "total": run.modes.len(),
                "done": run.ladder_skipped.unwrap_or(0) + run.rungs.len(),
                "testing": testing,
                "testing_name": testing.and_then(name),
                "frames": run.plan.rung_frames,
                "highest_passed": run.highest_passed,
                "highest_passed_name": run.highest_passed.and_then(name),
                "failures_in_row": run.ladder_fails,
                "failures_allowed": LADDER_FAILS,
                "last": last,
            },
            "link": (self.engine.state() == State::Connected).then(|| {
                let mode = self.engine.current_mode();
                json!({
                    "rung": mode,
                    "rung_name": name(mode),
                    "heard_there_db": self.engine.peer_snr_db().map(round1),
                })
            }),
        })
    }

    /// Move a running Test session along: called once per audio block.
    pub(super) fn advance_test(&mut self) {
        let Some(mut run) = self.test.take() else {
            return;
        };
        if run.running() {
            self.test_stepping = true;
            self.step_test(&mut run);
            self.test_stepping = false;
        }
        self.test = Some(run);
    }

    #[allow(clippy::too_many_lines)]
    fn step_test(&mut self, run: &mut TestRun) {
        let now = self.now();
        let held = self.held_total_s;
        let since = run.step_elapsed_s(now, held);
        match run.step {
            Step::Probe => {
                if !run.entered {
                    match self
                        .engine
                        .probe(&run.plan.remote, run.plan.as_call.as_deref())
                    {
                        Ok(()) => {
                            self.pump();
                            run.entered = true;
                        }
                        Err(reason) => self.finish_test(run, &format!("aborted: {reason}")),
                    }
                } else if run.remaining_s(now) <= 0.0 {
                    // the step's clock stops while the channel is busy, the run's does not
                    self.abort();
                    let busy = held - run.held_at_step;
                    self.finish_test(
                        run,
                        &format!("aborted: out of time, the channel busy {busy:.0} s"),
                    );
                } else if !self.engine.probing() || since > PROBE_TIMEOUT_S {
                    run.probe = self
                        .engine
                        .last_probe()
                        .map(|p| (p.heard_there_db, p.heard_here_db));
                    match run.probe {
                        Some((there, here)) => self.note(
                            "test",
                            &format!(
                                "probe: {} hears us at {} dB, heard at {here:.1} dB",
                                run.plan.remote,
                                there.map_or("?".to_owned(), |t| format!("{t:.0}"))
                            ),
                        ),
                        None => self.note("test", "probe: no answer; calling anyway"),
                    }
                    run.message_size =
                        message_size_for(message_snr(run.probe), run.plan.message_bytes);
                    if run.message_size < run.plan.message_bytes {
                        let why = match run.probe {
                            Some((Some(there), _)) => {
                                format!("{} hears us at {there:.0} dB", run.plan.remote)
                            }
                            Some((None, here)) => format!("heard at {here:.0} dB"),
                            None => "the probe went unanswered".to_owned(),
                        };
                        run.adjustments
                            .push(format!("message {} bytes: {why}", run.message_size));
                    }
                    run.enter(Step::Connect, now, held);
                }
            }
            Step::Connect => {
                if !run.entered {
                    match self.connect_as(&run.plan.remote, run.plan.as_call.as_deref()) {
                        Ok(()) => {
                            run.entered = true;
                            // The message goes with the call when the probe has sized it: the
                            // station hands it to the link as the session comes up, and the
                            // first burst carries it. Queued once the session was up, it
                            // followed the caller's poll and its answer — two floor frames,
                            // 7.4 s of WC4Y's 71 s message on 2026-10-11 (ADR-0066). Unsized,
                            // it waits for the call's measurement as before.
                            if run.probe.is_some() && run.message_size > 0 {
                                let before = self.outbound.len();
                                let data = run.test_bytes(run.message_size, 1);
                                self.send(&data);
                                run.message_queued = Some(before);
                            }
                        }
                        Err(reason) => self.finish_test(run, &format!("aborted: {reason}")),
                    }
                } else if self.engine.state() == State::Connected {
                    self.note("test", "connected");
                    // A probe that went unanswered left the message at a kilobyte, sized for
                    // nothing; the call has measured the path since. KE4QCM's Test of
                    // 2026-10-09: no probe answer, the acceptance read at −7 dB, and a
                    // kilobyte at the tone floor's 20 bit/s ran the message out of time.
                    if run.probe.is_none() {
                        let call = message_snr_after_call(
                            self.engine.peer_snr_db(),
                            self.engine.rate_readings().0,
                        );
                        let size = message_size_for(call, run.plan.message_bytes);
                        if let Some(snr) = call
                            && size != run.message_size
                        {
                            run.adjustments.retain(|a| !a.starts_with("message "));
                            run.adjustments.push(format!(
                                "message {size} bytes: the probe went unanswered, \
                                 the call measured {snr:.0} dB"
                            ));
                            run.message_size = size;
                        }
                    }
                    run.enter(Step::Message, now, held);
                } else if self.engine.state() == State::Idle && since > 2.0 {
                    self.finish_test(run, "aborted: the call was not answered");
                } else if run.remaining_s(now) <= 0.0 {
                    self.abort();
                    let busy = held - run.held_at_step;
                    self.finish_test(
                        run,
                        &format!("aborted: out of time, the channel busy {busy:.0} s"),
                    );
                } else if since > CONNECT_TIMEOUT_S {
                    self.abort();
                    self.finish_test(run, "aborted: the call timed out");
                }
            }
            Step::Message | Step::File => {
                let (bytes, salt, next) = if run.step == Step::Message {
                    (run.message_size, 1, Step::File)
                } else {
                    (run.file_size, 2, Step::Ladder)
                };
                if bytes == 0 {
                    if run.step == Step::Message {
                        self.size_test_file(run, now);
                    } else {
                        self.start_ladder(run);
                    }
                    run.enter(next, now, held);
                } else if !run.entered {
                    run.acked_at_step = self.engine.stats.bytes_acked;
                    // the message may have gone with the call
                    if !(run.step == Step::Message && run.message_queued.take().is_some()) {
                        let data = run.test_bytes(bytes, salt);
                        self.send(&data);
                    }
                    run.entered = true;
                } else if self.engine.state() != State::Connected {
                    self.finish_test(run, "aborted: the session dropped during the transfer");
                } else if self.engine.all_acknowledged() && self.outbound.is_empty() {
                    let transfer = Transfer {
                        bytes,
                        seconds: since,
                    };
                    self.note(
                        "test",
                        &format!(
                            "{}: {bytes} bytes in {:.1} s ({:.0} bit/s)",
                            run.step.name(),
                            transfer.seconds,
                            transfer.bps()
                        ),
                    );
                    if run.step == Step::Message {
                        run.message = Some(transfer);
                        self.size_test_file(run, now);
                    } else {
                        run.file = Some(transfer);
                        self.start_ladder(run);
                    }
                    run.enter(next, now, held);
                } else if since > transfer_timeout_s(bytes).min(run.remaining_s(now) + 60.0) {
                    // a crawl: end it now, keeping what was learned, rather than wait for
                    // an orderly close that would first deliver the rest of the transfer
                    let acked = self
                        .engine
                        .stats
                        .bytes_acked
                        .saturating_sub(run.acked_at_step)
                        .min(bytes);
                    self.abort();
                    if run.step == Step::File && acked > 0 {
                        // the file is sized to what the budget has left; a path that slows
                        // under it (a burst of RTTY in the harness's contest scenario) still
                        // gave its rate, and the run is not a failure for it — though the
                        // ladder that would have followed has no time left
                        let transfer = Transfer {
                            bytes: acked,
                            seconds: since,
                        };
                        let what = format!(
                            "file: {acked} of {bytes} bytes before the budget ran out ({:.0} bit/s)",
                            transfer.bps()
                        );
                        self.note("test", &what);
                        run.adjustments.push(what);
                        if run.plan.ladder {
                            run.adjustments
                                .push("ladder skipped: the file took the budget".to_owned());
                        }
                        run.file = Some(transfer);
                        self.finish_test(run, "complete");
                        return;
                    }
                    let why = format!("aborted: the {} timed out", run.step.name());
                    self.finish_test(run, &why);
                    return;
                }
                self.sync_test_report(run);
            }
            Step::Ladder => {
                // a rung is started only with the time for it and the disconnect still in
                // hand; one under way is seen through
                let out_of_time = !run.entered && run.remaining_s(now) < LADDER_RUNG_LEFT_S;
                if !run.plan.ladder
                    || run.ladder_next >= run.modes.len()
                    || run.ladder_fails >= LADDER_FAILS
                    || out_of_time
                {
                    if run.plan.ladder && out_of_time && run.ladder_next < run.modes.len() {
                        run.adjustments.push(format!(
                            "ladder stopped after {} rungs: the budget ran out",
                            run.rungs.len()
                        ));
                    }
                    if run.plan.ladder {
                        self.note("test", &format!("ladder: {} rungs, done", run.rungs.len()));
                    }
                    run.enter(Step::Disconnect, now, held);
                } else if !run.entered {
                    let mode = run.modes[run.ladder_next];
                    let _ = self.engine.measure(mode, run.plan.rung_frames);
                    run.entered = true;
                } else {
                    let mode = run.modes[run.ladder_next];
                    for rung in self.engine.take_ladder() {
                        let rung = Rung {
                            mode: rung.mode,
                            frames: rung.frames,
                            decoded: rung.decoded,
                            snr_db: rung.snr_db,
                            seconds: since,
                        };
                        self.note(
                            "test",
                            &format!(
                                "rung {}: {}/{} decoded at {}",
                                rung.mode,
                                rung.decoded,
                                rung.frames,
                                rung.snr_db
                                    .map_or("? dB".to_owned(), |s| format!("{s:.0} dB"))
                            ),
                        );
                        run.ladder_fails = if rung.failed() {
                            run.ladder_fails + 1
                        } else {
                            run.highest_passed =
                                Some(run.highest_passed.map_or(rung.mode, |h| h.max(rung.mode)));
                            0
                        };
                        run.rungs.push(rung);
                        run.rung_reported = true;
                    }
                    if self.engine.state() != State::Connected {
                        self.finish_test(run, "aborted: the session dropped during the ladder");
                    } else if run.rung_reported {
                        // a measurement leaves nothing to deliver: the next rung follows its
                        // report at once (ADR-0077)
                        run.ladder_next += 1;
                        run.enter(Step::Ladder, now, held);
                    } else if since > RUNG_TIMEOUT_S {
                        self.note("test", &format!("rung {mode}: timed out"));
                        let why = format!("aborted: rung {mode} timed out");
                        self.abort();
                        self.finish_test(run, &why);
                        return;
                    }
                }
                self.sync_test_report(run);
            }
            Step::Disconnect => {
                if !run.entered {
                    self.close_test_session();
                    run.entered = true;
                } else if self.engine.state() == State::Idle {
                    let outcome = run.failure.clone().unwrap_or_else(|| "complete".to_owned());
                    self.finish_test(run, &outcome);
                } else if since > DISCONNECT_TIMEOUT_S {
                    self.abort();
                    let outcome = run
                        .failure
                        .clone()
                        .unwrap_or_else(|| "aborted: the disconnect timed out".to_owned());
                    self.finish_test(run, &outcome);
                }
            }
            Step::Done => {}
        }
    }

    /// The file the message's rate earns: about two minutes' worth, in the time the budget has
    /// left once the ladder that follows has its own ([`LADDER_RESERVE_S`]).
    fn size_test_file(&mut self, run: &mut TestRun, now: f64) {
        let bps = run.message.map_or(0.0, |m| m.bps());
        let reserve = if run.plan.ladder {
            LADDER_RESERVE_S
        } else {
            0.0
        };
        run.file_size = file_size_for(bps, run.plan.file_bytes, run.remaining_s(now) - reserve);
        if run.file_size == 0 && run.plan.file_bytes > 0 {
            run.adjustments
                .push("file skipped: no time left".to_owned());
            self.note("test", "file skipped: no time left in the budget");
        } else if run.file_size < run.plan.file_bytes {
            run.adjustments.push(format!(
                "file {} bytes: {bps:.0} bit/s measured",
                run.file_size
            ));
        }
    }

    /// Where the ladder starts: a rung under the fastest the transfers were acknowledged at. The
    /// rungs below have just carried the message and the file, and climbing them again cost
    /// WC4Y's Test of 2026-10-10 four tone rungs of 26 s each, a sixth of its budget, before the
    /// ladder learned anything new. The fastest acknowledged, not the recommendation at the end:
    /// two lost retries at the end of WC4Y's message of 2026-10-11 had stepped that down from 3
    /// to 1, and the ladder climbed rungs 0-3 again for 105 s.
    fn start_ladder(&mut self, run: &mut TestRun) {
        let rung = self
            .engine
            .top_acked_mode()
            .unwrap_or_else(|| self.engine.current_mode());
        let skip = skipped_rungs(&run.modes, rung);
        run.ladder_skipped = Some(skip);
        run.ladder_next = skip;
        if run.plan.ladder && skip > 0 {
            let from = run.modes[skip];
            let why = format!("ladder from rung {from}: the transfers reached rung {rung}");
            self.note("test", &why);
            run.adjustments.push(why);
        }
    }

    /// The Test's own disconnect: at once, with what is still queued left undelivered — a file
    /// whose time ran out; the ladder's measurements leave nothing queued (ADR-0077). WC4Y's
    /// Test of 2026-10-17 waited 26 s of its 435 to deliver 40 bytes of the old ladder's filler
    /// (ADR-0076). The other station may say the last frames did not arrive, which for a Test
    /// is so.
    fn close_test_session(&mut self) {
        let queued = self.engine.tx_pending_bytes();
        self.engine.disconnect_within(Some(0.0));
        if queued > 0 {
            self.note(
                "test",
                &format!("closing without the {queued} test bytes still queued: nobody reads them"),
            );
        }
        self.pump();
    }

    /// Put the run's report into the recording's sidecar, as it stands.
    fn sync_test_report(&mut self, run: &TestRun) {
        if !run.recorded {
            return;
        }
        let report = run.json(
            self.now(),
            &self.config.operator.grid,
            self.config.params.bandwidth.hz(),
        );
        if let Some(recording) = &mut self.recording {
            recording.set_session_field("test", report);
        }
    }

    fn finish_test(&mut self, run: &mut TestRun, outcome: &str) {
        // a message queued with a call that never came up goes with the Test, not into the
        // next session (ADR-0052)
        if let Some(before) = run.message_queued.take() {
            self.outbound.truncate(before);
        }
        run.step = Step::Done;
        run.outcome = Some(outcome.to_owned());
        run.ended_s = Some(self.now());
        self.sync_test_report(run);
        if run.recorded {
            // the Test calls, so its station is the master: its identifier is the last of all
            self.stop_recording_after_identifiers(false);
        }
        self.note("test", outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Air;
    use super::*;

    #[test]
    fn a_plan_comes_from_the_request_with_its_defaults() {
        let plan = TestPlan::from_params(&json!({ "remote": " kk4xyz " })).expect("a callsign");
        assert_eq!(plan.remote, "KK4XYZ");
        assert_eq!((plan.message_bytes, plan.file_bytes), (2048, 16_384));
        assert!(plan.ladder && plan.rung_frames == 4);
        assert!((plan.budget_s - 600.0).abs() < f64::EPSILON);
        let plan = TestPlan::from_params(&json!({
            "remote": "KK4XYZ", "remote_grid": "FN31", "message_bytes": 300,
            "file_bytes": 0, "ladder": false, "rung_frames": 40, "budget_s": 300
        }))
        .expect("a full plan");
        assert!((plan.budget_s - 300.0).abs() < f64::EPSILON);
        assert!(TestPlan::from_params(&json!({ "remote": "KK4XYZ", "budget_s": 5 })).is_err());
        assert_eq!(plan.remote_grid.as_deref(), Some("FN31"));
        assert_eq!(
            (plan.message_bytes, plan.file_bytes, plan.rung_frames),
            (300, 0, 16)
        );
        assert!(!plan.ladder);
        assert!(TestPlan::from_params(&json!({})).is_err());
        assert!(
            TestPlan::from_params(&json!({ "remote": "KK4XYZ", "remote_grid": "nowhere" }))
                .is_err()
        );
        assert!(TestPlan::from_params(&json!({ "remote": "KK4XYZ", "file_bytes": -1 })).is_err());
    }

    #[test]
    fn the_ladder_starts_where_the_message_ran() {
        let modes: Vec<usize> = (0..15).collect();
        // WC4Y's Test: the message ran at rung 3, so the ladder starts at 2 and skips 0-1
        assert_eq!(skipped_rungs(&modes, 3), 2);
        // a message on the floor or a rung above it skips nothing; one above every rung still
        // tries the top
        assert_eq!(skipped_rungs(&modes, 0), 0);
        assert_eq!(skipped_rungs(&modes, 1), 0);
        assert_eq!(skipped_rungs(&modes, 40), 14);
        // a ladder the rules cut short (ADR-0018) skips only the rungs it has
        assert_eq!(skipped_rungs(&[0, 1, 2], 9), 2);
        assert_eq!(skipped_rungs(&[], 3), 0);
    }

    #[test]
    fn the_sizes_follow_the_path() {
        assert_eq!(message_size_for(Some(12.0), 2048), 2048);
        assert_eq!(message_size_for(Some(3.0), 2048), 1024);
        assert_eq!(message_size_for(Some(-4.0), 2048), 512);
        assert_eq!(message_size_for(None, 2048), 1024);
        // the probe unanswered, the call measures the path: KE4QCM's acceptance read at -7 dB
        assert_eq!(
            message_size_for(message_snr_after_call(None, Some(-6.7)), 1024),
            512
        );
        assert_eq!(message_snr_after_call(Some(-2.0), Some(-6.7)), Some(-2.0));
        assert_eq!(message_snr_after_call(None, None), None);
        assert_eq!(message_size_for(Some(-4.0), 300), 300);
        // the message travels to the other station: its reading of this one sizes it — ND1J
        // heard KK4ODA at -1 dB and was heard at +5, and a kilobyte took five minutes
        assert_eq!(message_snr(Some((Some(-1.0), 5.0))), Some(-1.0));
        assert_eq!(
            message_size_for(message_snr(Some((Some(-1.0), 5.0))), 2048),
            512
        );
        assert_eq!(message_snr(Some((None, 5.0))), Some(5.0));
        assert_eq!(message_snr(None), None);
        // two minutes' worth at the measured rate, within a kilobyte and the ceiling
        assert_eq!(file_size_for(1000.0, 16_384, 600.0), 14_848);
        assert_eq!(file_size_for(2000.0, 16_384, 600.0), 16_384);
        // a slow path gets the file it can carry in two minutes, down to 256 bytes — the
        // floor of a kilobyte timed out at 19 bit/s every time
        assert_eq!(file_size_for(50.0, 16_384, 600.0), 512);
        assert_eq!(file_size_for(19.0, 16_384, 600.0), 256);
        assert_eq!(file_size_for(10.0, 16_384, 600.0), 0);
        assert_eq!(file_size_for(1000.0, 600, 600.0), 600);
        // and what the budget leaves: 90 s left is 30 s of file
        assert_eq!(file_size_for(1000.0, 16_384, 90.0), 3584);
        assert_eq!(file_size_for(1000.0, 16_384, 45.0), 0);
        assert_eq!(file_size_for(1000.0, 0, 600.0), 0);
        // the file leaves the ladder after it its time (ADR-0074): with five minutes left, the
        // ladder's 150 s and the minute in hand leave the file 90 s — at 70 bit/s, 768 bytes
        let sized = file_size_for(70.0, 16_384, 300.0 - LADDER_RESERVE_S);
        assert_eq!(sized, 768);
        assert!(sized as f64 * 8.0 / 70.0 + LADDER_RESERVE_S + 60.0 <= 300.0);
        // with eight minutes left the file has its two minutes and the ladder its time
        assert_eq!(file_size_for(70.0, 16_384, 480.0 - LADDER_RESERVE_S), 1024);
    }

    #[test]
    fn the_test_bytes_are_incompressible_and_repeatable() {
        let run = TestRun::new(TestPlan::default(), 0.0, vec![0], 7);
        let a = run.test_bytes(4096, 1);
        let b = run.test_bytes(4096, 1);
        let c = run.test_bytes(4096, 2);
        assert_eq!(a, b);
        assert_ne!(a, c);
        // every byte value turns up: nothing a compressor can lean on
        let mut seen = [false; 256];
        for &byte in &a {
            seen[usize::from(byte)] = true;
        }
        assert!(seen.iter().filter(|&&s| s).count() > 240);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn a_test_session_runs_its_sequence_and_records_it() {
        let dir = std::env::temp_dir().join(format!("aether-test-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut air = Air::new(1.0, 0.0005);
        air.a.set_recording(Some(dir.clone()), false, "");
        air.a.config.operator.grid = "EM73".to_owned();
        let plan = TestPlan {
            remote: "KK4XYZ".to_owned(),
            remote_grid: Some("FN31".to_owned()),
            message_bytes: 300,
            file_bytes: 600,
            rung_frames: 2,
            ..TestPlan::default()
        };
        air.a.start_test(plan).expect("idle");
        assert!(air.a.test_running());
        assert_eq!(
            air.a.start_test(TestPlan::default()),
            Err("a test session is already running".to_owned())
        );
        // what the panel is shown, step by step: the transfers first, measured as the link
        // runs, then the ladder (ADR-0074), and while it climbs, the rung under test out of all
        // of them
        let mut steps: Vec<String> = Vec::new();
        let mut during_ladder = Value::Null;
        let mut during_message = Value::Null;
        air.run(400.0, |a, _| {
            let brief = a.test_brief();
            if let Some(step) = brief["step"].as_str()
                && steps.last().is_none_or(|last| last != step)
            {
                steps.push(step.to_owned());
            }
            if brief["step"] == "ladder"
                && during_ladder.is_null()
                && brief["ladder"]["testing"].is_u64()
            {
                during_ladder = brief.clone();
            }
            if brief["step"] == "message" && !brief["transfer"].is_null() {
                during_message = brief.clone();
            }
            !a.test_running()
        });
        assert_eq!(
            steps,
            [
                "probe",
                "connect",
                "message",
                "file",
                "ladder",
                "disconnect"
            ],
            "{steps:?}"
        );
        // the ladder's first rung is the one the transfers ran at — on this clean wire well up
        // the ladder — and the rungs under it count as done on the panel; it climbs only the
        // rungs the link uses, the 2 300 Hz air's twenty less rungs 6 and 13 (ADR-0076)
        assert_eq!(during_ladder["ladder"]["total"], 18, "{during_ladder}");
        let first = during_ladder["ladder"]["testing"]
            .as_u64()
            .expect("a rung under test");
        assert!(
            first > 0,
            "the message ran above the floor: {during_ladder}"
        );
        let under = ladder_rungs(air.a.engine.timing())
            .iter()
            .filter(|&&m| (m as u64) < first)
            .count();
        assert_eq!(during_ladder["ladder"]["done"], under, "{during_ladder}");
        assert!(during_ladder["ladder"]["testing_name"].is_string());
        assert_eq!(during_ladder["ladder"]["failures_allowed"], 3);
        assert!(
            during_ladder["link"]["rung_name"].is_string(),
            "{during_ladder}"
        );
        assert!(
            during_ladder["remaining_s"]
                .as_f64()
                .is_some_and(|r| r > 0.0 && r < 600.0)
        );
        assert_eq!(during_message["transfer"]["bytes"], 300, "{during_message}");
        assert!(
            during_message["transfer"]["acked"]
                .as_u64()
                .is_some_and(|a| a <= 300)
        );
        let status = air.a.test_status();
        let results = &status["results"];
        assert_eq!(results["outcome"], "complete", "{results}");
        // a probe and its answer are tone frames (ADR-0016), whose SNR reading tops out near
        // +17 dB however clean the path; the ladder's OFDM rungs read past it
        assert!(
            results["probe"]["heard_here_db"]
                .as_f64()
                .is_some_and(|s| s > 15.0),
            "{results}"
        );
        assert!(
            results["message"]["bps"].as_f64().is_some_and(|b| b > 0.0),
            "{results}"
        );
        assert!(
            results["file"]["bps"].as_f64().is_some_and(|b| b > 0.0),
            "{results}"
        );
        // every rung of the ladder — the tone floor's six (ADR-0013, ADR-0014), then the OFDM
        // modes — and every rung whole, except the two top 64-QAM modes (OFDM modes 12 and
        // 13, rungs 18 and 19), which decode short of their frames on this wire at a
        // reported 24–26 dB: the open question of ADR-0008 (a 64-QAM burst on the clean
        // loopback), which the ladder is built to show and P9-1 is to settle. Not hidden
        // here: it is the reason the top two rungs are excused, and the excuse goes when the
        // modem does.
        let ladder = results["ladder"].as_array().expect("rungs");
        let modes: Vec<u64> = ladder.iter().filter_map(|r| r["mode"].as_u64()).collect();
        assert_eq!(modes, (first..20).collect::<Vec<u64>>(), "{results}");
        assert!(
            results["adjustments"]
                .as_array()
                .is_some_and(|a| a.iter().any(|x| x
                    .as_str()
                    .is_some_and(|x| x.starts_with(&format!("ladder from rung {first}: "))))),
            "{results}"
        );
        let short: Vec<&Value> = ladder
            .iter()
            .filter(|r| r["decoded"] != r["frames"] && r["mode"].as_u64().is_some_and(|m| m < 18))
            .collect();
        assert!(
            short.is_empty(),
            "rungs short of frames: {short:?}\nall: {ladder:?}"
        );
        // the log names each by its rung, as the ladder and the panel do: "rung mode 3" read
        // as a mode numbered on some other table
        let events = air.a.take_events();
        assert!(
            events
                .iter()
                .any(|e| e.starts_with(&format!("test:rung {first}: "))),
            "{events:?}"
        );
        assert!(
            !events.iter().any(|e| e.contains("rung mode")),
            "{events:?}"
        );
        assert_eq!(results["path"]["my_grid"], "EM73");
        assert!(
            results["path"]["km"].as_f64().is_some_and(|km| km > 1000.0),
            "{results}"
        );
        assert!(!air.a.connected() && !air.b.connected());
        // its recording runs on until the closing identifiers have gone (ADR-0076)
        air.run(60.0, |a, _| a.recording.is_none());
        // the sidecar carries the same report
        let sidecar = std::fs::read_dir(&dir)
            .expect("recordings")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|x| x == "json") && p.to_string_lossy().contains("_test")
            })
            .expect("a sidecar named for the test");
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("read")).expect("json");
        assert_eq!(document["session"]["test"]["outcome"], "complete");
        assert_eq!(document["session"]["operator"]["grid"], "EM73");
        assert_eq!(
            document["session"]["test"]["ladder"]
                .as_array()
                .map(Vec::len),
            Some(ladder.len())
        );
        // the message went with the call: what followed the acceptance was the message's
        // first transmission, not a header granting an answer and nothing else (ADR-0066) —
        // under granted slots a header and the message's data behind it (ADR-0069)
        let sent = document["sent"].as_array().expect("the frames sent");
        let mut transmissions: Vec<Vec<&str>> = Vec::new();
        let mut at: Option<f64> = None;
        for frame in sent {
            let t = frame["t_s"].as_f64().expect("a time");
            if at.is_none_or(|a| (t - a).abs() > 1e-9) {
                transmissions.push(Vec::new());
                at = Some(t);
            }
            if let (Some(last), Some(kind)) = (transmissions.last_mut(), frame["kind"].as_str()) {
                last.push(kind);
            }
        }
        assert_eq!(
            transmissions.first().map(Vec::as_slice),
            Some(&["data"][..]),
            "the probe"
        );
        assert_eq!(
            transmissions.get(1).map(Vec::as_slice),
            Some(&["data"][..]),
            "the call"
        );
        let message = transmissions.get(2).expect("the message");
        assert!(
            message.first() == Some(&"control") && message[1..].contains(&"data"),
            "a header and the message's data: {sent:?}"
        );
        // and the run can be asked for again
        assert!(!air.a.abort_test());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_crawls_ends_the_run_with_its_report_in_the_sidecar() {
        // ADR-0074's harness A/B, the RTTY contest's first draw: the file crawled under the
        // RTTY, the run ended the session on its deadline, and the session's end closed the
        // recording before the run wrote its report — the sidecar had no outcome and no file
        let dir = std::env::temp_dir().join(format!("aether-test-crawl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut air = Air::new(1.0, 0.0005);
        // every session recorded, as the harness's stations and many operators' are: the
        // session's end is what closes such a recording
        air.a.set_recording(Some(dir.clone()), true, "");
        let plan = TestPlan {
            remote: "KK4XYZ".to_owned(),
            message_bytes: 300,
            file_bytes: 16_384,
            rung_frames: 2,
            ..TestPlan::default()
        };
        air.a.start_test(plan).expect("idle");
        air.run(400.0, |a, _| {
            let brief = a.test_brief();
            brief["step"] == "file" && brief["transfer"]["acked"].as_u64().is_some_and(|b| b > 0)
        });
        // the budget runs out under the file
        let now = air.a.now();
        let run = air.a.test.as_mut().expect("a run");
        run.plan.budget_s = now - run.started_s - 70.0;
        air.run(120.0, |a, _| !a.test_running());
        // its recording runs on until the closing identifiers have gone (ADR-0076)
        air.run(60.0, |a, _| a.recording.is_none());
        let results = air.a.test_status()["results"].clone();
        assert_eq!(results["outcome"], "complete", "{results}");
        let carried = results["file"]["bytes"].as_u64().expect("the file's bytes");
        assert!(carried > 0 && carried < 16_384, "{results}");
        assert!(
            results["adjustments"].as_array().is_some_and(|a| a
                .iter()
                .any(|x| x == "ladder skipped: the file took the budget")),
            "{results}"
        );
        let sidecar = std::fs::read_dir(&dir)
            .expect("recordings")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.extension().is_some_and(|x| x == "json") && p.to_string_lossy().contains("_test")
            })
            .expect("a sidecar named for the test");
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("read")).expect("json");
        assert_eq!(document["session"]["test"]["outcome"], "complete");
        assert_eq!(document["session"]["test"]["file"]["bytes"], carried);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_test_session_is_refused_where_a_call_would_be() {
        let mut air = Air::with(1.0, 0.0005, |mut config| {
            config.answer_only = true;
            config
        });
        assert!(
            air.a
                .start_test(TestPlan {
                    remote: "KK4XYZ".into(),
                    ..TestPlan::default()
                })
                .is_err()
        );
        let mut air = Air::new(1.0, 0.0005);
        air.a.connect("KK4XYZ").expect("idle");
        assert_eq!(
            air.a.start_test(TestPlan {
                remote: "KK4XYZ".into(),
                ..TestPlan::default()
            }),
            Err("a session is running".to_owned())
        );
    }

    /// Step 3 §3.7: on a contest frequency a Test's probe was held 29.7 s for a clear channel,
    /// its 30 s ran out as the answer arrived, and it called on "no answer". The step's clock
    /// now stops while the station waits to transmit: here the other station's tune tones keep
    /// the channel busy for some 40 s, and the probe still goes and is answered.
    #[test]
    fn a_probe_held_for_a_busy_channel_keeps_its_own_deadline() {
        let mut air = Air::new(1.0, 0.0005);
        air.a.config.wait_for_clear = true;
        // the busy detectors learn the noise floor first
        air.run(4.0, |_, _| false);
        air.b.tune(10.0).expect("idle");
        air.run(5.0, |a, _| a.channel_busy());
        assert!(air.a.channel_busy(), "the tone was not heard");
        let plan = TestPlan {
            remote: "KK4XYZ".to_owned(),
            message_bytes: 100,
            file_bytes: 0,
            ladder: false,
            ..TestPlan::default()
        };
        air.a.start_test(plan).expect("idle");
        for _ in 0..3 {
            // the next tone is asked for as the last one ends: no gap the probe could go in
            air.run(12.0, |_, b| !b.transmitting());
            air.b.tune(10.0).expect("idle");
            air.run(2.0, |_, b| b.transmitting());
        }
        air.run(12.0, |_, b| !b.transmitting());
        let held = air.a.held_total_s;
        assert!(
            held > PROBE_TIMEOUT_S,
            "the probe was held only {held:.1} s"
        );
        air.run(60.0, |a, _| {
            a.test.as_ref().is_none_or(|run| run.step != Step::Probe)
        });
        let run = air.a.test.as_ref().expect("the run");
        assert!(
            run.probe.is_some(),
            "the probe went unanswered after {held:.1} s held: {:?} {:?}",
            run.outcome,
            run.step
        );
        assert!(run.outcome.is_none(), "{:?}", run.outcome);
    }
}
