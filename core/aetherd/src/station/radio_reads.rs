//! What a recording keeps of the radio itself: its S-meter, its dial and its settings, read in
//! the background while a recording runs and the station is idle, and the operator's marks
//! (the busy detector's investigation, `docs/research/busy-detection/00-recording-plan.md`).
//!
//! The audio says what the radio delivered. The S-meter follows the radio's AGC itself, which
//! the audio can only suggest, and a change of dial or of a setting at the radio explains a
//! change of level that nothing in the audio does: the first batch of the investigation's
//! recordings opened on a 26 dB rise of the whole passband that only the operator could have
//! said was a knob. A mark is the operator's press of a button at a moment that matters — each
//! move part C of the plan asks for — in place of a stopwatch.
//!
//! The radio is asked only while the station is idle (no session, call, probe or Test, nothing
//! queued, not transmitting), so a reading is never on the keying port when a slot needs the
//! key; the keying thread takes the run loop's own requests before any reading in any case
//! (`ThreadedPtt`). The S-meter four times a second, the dial once a second, the settings every
//! five. Only a keying interface on a thread of its own reads in the background: CAT, rigctld
//! and `FLRig`.

use super::Station;
use crate::ptt::{Asked, Ptt, Readings};

/// How often the S-meter is asked for, seconds.
pub const METER_EVERY_S: f64 = 0.25;
/// How often the dial is asked for, seconds.
pub const DIAL_EVERY_S: f64 = 1.0;
/// How often the settings are asked for, seconds.
pub const SETTINGS_EVERY_S: f64 = 5.0;
/// After this many S-meter questions in a row have gone unanswered, the meter is not asked
/// again for the rest of the recording: a radio or server without one (rigctld answers
/// `RPRT` for a rig whose backend has no `STRENGTH`, and drops the connection each time).
pub const METER_GIVE_UP: u32 = 8;

/// When a reading due at `due` and taken at `now` is next due: on its own beat, so passes that
/// do not fall on it do not slow it (a 0.25 s beat on 0.1 s passes is not every 0.3 s), or a
/// whole beat from now when it has fallen behind.
fn next_due(due: f64, now: f64, every: f64) -> f64 {
    if due + every > now {
        due + every
    } else {
        now + every
    }
}

/// The radio's readings for the recording running, and the operator's marks in it.
#[derive(Debug, Default)]
pub(super) struct RadioReads {
    next_meter: f64,
    next_dial: f64,
    next_settings: f64,
    /// S-meter questions unanswered in a row.
    meter_misses: u32,
    /// Marks pressed in the recording.
    pub(super) marks: u32,
}

impl<P: Ptt> Station<P> {
    /// Called every pass: put what the radio has said into the recording, and ask it again
    /// what is due.
    pub(super) fn read_radio(&mut self, now: f64) {
        for readings in self.ptt.inner_mut().take_readings() {
            let at = now - readings.at.elapsed().as_secs_f64();
            self.keep_readings(at, &readings);
        }
        if self.recording.is_none() || self.transmitting || self.movable().is_err() {
            return;
        }
        let reads = &self.radio_reads;
        let asked = Asked {
            meter: self.ptt.inner().reads_meter()
                && reads.meter_misses < METER_GIVE_UP
                && now >= reads.next_meter,
            dial: self.ptt.can_tune() && now >= reads.next_dial,
            settings: now >= reads.next_settings,
        };
        if !asked.any() {
            return;
        }
        self.ptt.inner_mut().request_readings(asked);
        let reads = &mut self.radio_reads;
        if asked.meter {
            reads.next_meter = next_due(reads.next_meter, now, METER_EVERY_S);
        }
        if asked.dial {
            reads.next_dial = next_due(reads.next_dial, now, DIAL_EVERY_S);
        }
        if asked.settings {
            reads.next_settings = next_due(reads.next_settings, now, SETTINGS_EVERY_S);
        }
    }

    /// A background reading, taken at `at` on the station's clock.
    fn keep_readings(&mut self, at: f64, readings: &Readings) {
        if readings.asked.meter {
            if readings.meter.is_some() {
                self.radio_reads.meter_misses = 0;
            } else {
                self.radio_reads.meter_misses += 1;
                if self.radio_reads.meter_misses == METER_GIVE_UP {
                    self.note(
                        "recording",
                        &format!(
                            "the radio did not answer {METER_GIVE_UP} questions for its S-meter \
                             in a row; this recording goes on without it"
                        ),
                    );
                }
            }
        }
        if let Some(hz) = readings.dial_hz {
            // fresher than the display's ten-second cadence, and the rules check reads it too
            self.read_dial(hz, at);
        }
        let Some(recording) = self.recording.as_mut() else {
            return;
        };
        if let Some(meter) = readings.meter {
            recording.meter(at, meter.value, meter.scale);
        }
        if let Some(hz) = readings.dial_hz {
            recording.dial(at, hz);
        }
        for (name, value) in &readings.settings {
            recording.setting(at, name, value);
        }
    }

    /// Mark the running recording: the operator pressed Mark. Kept in the sidecar as a `mark`
    /// event numbered from 1, with `note` after the number when one is given.
    ///
    /// # Errors
    /// When nothing is being recorded.
    pub fn mark_recording(&mut self, note: Option<&str>) -> Result<(u32, f64), String> {
        let now = self.now();
        let state = format!("{:?}", self.engine.state());
        let Some(recording) = self.recording.as_mut() else {
            return Err("nothing is being recorded".into());
        };
        self.radio_reads.marks += 1;
        let mark = self.radio_reads.marks;
        let detail = note
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map_or_else(|| mark.to_string(), |text| format!("{mark} {text}"));
        recording.event(now, "mark", &detail, &state);
        let t_s = recording.time_of(now);
        self.note("recording", &format!("mark {mark} at {t_s:.1} s"));
        Ok((mark, t_s))
    }

    /// What the running recording holds of the radio and the operator, for the panel: the
    /// marks pressed, the S-meter readings, and whether this keying interface reads the meter
    /// at all.
    #[must_use]
    pub fn recording_reads(&self) -> Option<serde_json::Value> {
        let recording = self.recording.as_ref()?;
        Some(serde_json::json!({
            "marks": self.radio_reads.marks,
            "meter_readings": recording.meter_readings(),
            "reads_meter": self.ptt.inner().reads_meter(),
        }))
    }
}
