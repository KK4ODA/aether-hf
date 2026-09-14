//! Replaying a recording through the receiver, and holding it to what was recorded.
//!
//! A recorded session is only worth keeping if it can be run again. `aetherd --replay`
//! feeds a recording's audio through the same front end and streaming receiver a live
//! station uses — with the receiver muted where the sidecar says the transmitter was keyed,
//! as it was at the time — and lists every frame it finds. With the sidecar as the
//! expectation, it fails when fewer frames decode than did on the day: a change to the
//! receiver that loses a frame on real air is a regression, whatever the simulator says.
//!
//! What it does not do is drive the link engine. The engine would answer what it hears,
//! and a station answering a recording is not a replay; the frames are the receiver's
//! business and they are what the sidecar recorded.

use std::path::Path;

use aether_phy::{
    AudioToBaseband, Complex, StreamingReceiver, WIDE_2300, WaveformParams, preamble::FrameType,
};
use serde_json::Value;

use crate::record::{FrameRecord, read_wav};

/// The audio block a replay is fed in: twenty milliseconds, as the daemon's loop uses.
const BLOCK_S: f64 = 0.02;

/// An interval, in seconds of the recording, during which the receiver is muted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Muted {
    /// Start, inclusive.
    pub from_s: f64,
    /// End, exclusive; `f64::INFINITY` when the key was never released on record.
    pub until_s: f64,
}

/// What a sidecar says a replay should reproduce.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Expectation {
    /// The frames the recording station found.
    pub frames: Vec<FrameRecord>,
    /// When its transmitter was keyed, which is when its receiver was deaf.
    pub muted: Vec<Muted>,
}

impl Expectation {
    /// Read a sidecar.
    ///
    /// # Errors
    /// If the file cannot be read or is not a session sidecar.
    pub fn from_sidecar(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let document: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if document["format"] != crate::record::FORMAT {
            return Err(format!(
                "{} is not a session sidecar ({} expected)",
                path.display(),
                crate::record::FORMAT
            ));
        }
        let frames: Vec<FrameRecord> = serde_json::from_value(document["frames"].clone())
            .map_err(|e| format!("{}: frames: {e}", path.display()))?;
        let mut muted = Vec::new();
        let mut keyed_at: Option<f64> = None;
        for event in document["events"].as_array().into_iter().flatten() {
            if event["event"] != "ptt" {
                continue;
            }
            let t = event["t_s"].as_f64().unwrap_or(0.0);
            match (event["detail"].as_str(), keyed_at) {
                (Some("keyed"), None) => keyed_at = Some(t),
                (Some("released"), Some(from_s)) => {
                    muted.push(Muted { from_s, until_s: t });
                    keyed_at = None;
                }
                _ => {}
            }
        }
        if let Some(from_s) = keyed_at {
            muted.push(Muted {
                from_s,
                until_s: f64::INFINITY,
            });
        }
        Ok(Self { frames, muted })
    }
}

/// Run a recording through the receiver.
///
/// Returns every frame found, in the sidecar's terms, with `t_s` relative to the start of
/// the recording.
///
/// # Errors
/// If the file cannot be read, or is not at the modem's sample rate.
pub fn replay(wav: &Path, muted: &[Muted]) -> Result<Vec<FrameRecord>, String> {
    let params: WaveformParams = WIDE_2300;
    let audio = read_wav(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let rate = u32::try_from(params.audio_rate).unwrap_or(48_000);
    if audio.sample_rate != rate {
        return Err(format!(
            "{} is at {} Hz and the modem runs at {rate} Hz; resample it first",
            wav.display(),
            audio.sample_rate
        ));
    }
    Ok(run(&audio.samples, params, muted))
}

/// The receiver over samples already in memory.
#[must_use]
pub fn run(samples: &[f32], params: WaveformParams, muted: &[Muted]) -> Vec<FrameRecord> {
    let mut front = AudioToBaseband::new(params);
    let mut receiver = StreamingReceiver::new(params, 6.0, true);
    let block = (BLOCK_S * params.audio_rate as f64) as usize;
    let fs = params.fs_baseband;
    let rate = params.audio_rate as f64;
    let mut found = Vec::new();
    let mut seen = 0usize;
    let mut absorb = |baseband: &[Complex], receiver: &mut StreamingReceiver| {
        for decoded in receiver.feed(baseband) {
            found.push(FrameRecord {
                t_s: decoded.frame.sync.start as f64 / fs,
                kind: if decoded.frame.sync.frame_type == FrameType::Control {
                    "control".to_owned()
                } else {
                    "data".to_owned()
                },
                mode: decoded.frame.mode,
                rv: decoded.frame.rv,
                snr_3k_db: decoded.frame.snr_3k_db,
                cfo_hz: decoded.frame.cfo_hz,
                decoded: decoded.ok(),
                bytes: decoded.payload.as_ref().map_or(0, Vec::len),
                control: if decoded.frame.sync.frame_type == FrameType::Control {
                    decoded
                        .payload
                        .as_deref()
                        .and_then(crate::record::describe_control)
                } else {
                    None
                },
            });
        }
        receiver.take_preambles();
    };
    for chunk in samples.chunks(block) {
        let baseband = front.process(chunk);
        let t = seen as f64 / rate;
        seen += chunk.len();
        // muted where the station was transmitting: what the card captured then was its
        // own sidetone, and the live receiver never saw it
        let deaf = muted.iter().any(|m| t >= m.from_s && t < m.until_s);
        if deaf {
            let silence = vec![(0.0, 0.0); baseband.len()];
            absorb(&silence, &mut receiver);
        } else {
            absorb(&baseband, &mut receiver);
        }
    }
    // let the last frame out: the receiver holds samples back for its filters
    let tail = vec![0.0f32; block * 8];
    let baseband = front.process(&tail);
    absorb(&baseband, &mut receiver);
    found
}

/// How a replay compares with what was recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// Decoded frames on the day.
    pub recorded: usize,
    /// Decoded frames now.
    pub replayed: usize,
    /// Frames found now, decoded or not.
    pub found: usize,
}

impl Verdict {
    /// Whether the replay is at least as good as the day: a receiver may improve, but
    /// losing a frame that once decoded is the regression this exists to catch.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.replayed >= self.recorded
    }
}

/// Compare a replay's frames with a sidecar's.
#[must_use]
pub fn compare(expected: &[FrameRecord], found: &[FrameRecord]) -> Verdict {
    Verdict {
        recorded: expected.iter().filter(|f| f.decoded).count(),
        replayed: found.iter().filter(|f| f.decoded).count(),
        found: found.len(),
    }
}

/// One line per frame, for a person.
#[must_use]
pub fn describe(frame: &FrameRecord) -> String {
    format!(
        "{:>8.2} s  {:<7} mode {:>2} rv {}  {:>+6.1} dB  cfo {:>+6.1} Hz  {}",
        frame.t_s,
        frame.kind,
        frame.mode,
        frame.rv,
        frame.snr_3k_db,
        frame.cfo_hz,
        match (&frame.control, frame.decoded) {
            (Some(control), _) => control.clone(),
            (None, true) => format!("decoded {} bytes", frame.bytes),
            (None, false) => "failed".to_owned(),
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sidecar_yields_its_frames_and_the_keyed_intervals() {
        let dir = std::env::temp_dir().join(format!("aether-replay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("s.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "format": crate::record::FORMAT,
                "frames": [
                    {"t_s": 1.0, "kind": "control", "mode": 0, "rv": 0, "snr_3k_db": 5.0,
                     "cfo_hz": 0.0, "decoded": true, "bytes": 10},
                    {"t_s": 3.0, "kind": "data", "mode": 4, "rv": 1, "snr_3k_db": 2.0,
                     "cfo_hz": 0.0, "decoded": false, "bytes": 0}
                ],
                "events": [
                    {"t_s": 1.5, "event": "ptt", "detail": "keyed", "state": "Connected"},
                    {"t_s": 2.5, "event": "ptt", "detail": "released", "state": "Connected"},
                    {"t_s": 4.0, "event": "ptt", "detail": "keyed", "state": "Connected"}
                ]
            })
            .to_string(),
        )
        .expect("write");
        let expectation = Expectation::from_sidecar(&path).expect("sidecar");
        assert_eq!(expectation.frames.len(), 2);
        assert_eq!(
            expectation.muted,
            vec![
                Muted {
                    from_s: 1.5,
                    until_s: 2.5
                },
                Muted {
                    from_s: 4.0,
                    until_s: f64::INFINITY
                }
            ]
        );
        assert_eq!(compare(&expectation.frames, &[]).recorded, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_replay_finds_the_frames_a_transmitter_put_into_the_audio() {
        // a frame straight from the transmitter, through the audio front ends, at a clean
        // level: the receiver must find it and decode it, and a mute over it must hide it
        let params: WaveformParams = WIDE_2300;
        let mut transmitter = aether_phy::Modem::new(params, false);
        let mut to_audio = aether_phy::BasebandToAudio::new(params);
        let payload: Vec<u8> = (0..26u8).collect();
        let frame = transmitter
            .data_burst(&payload, aether_phy::MODES[0], 0)
            .expect("a mode-0 frame");
        let mut audio = vec![0.0f32; params.audio_rate];
        audio.extend(to_audio.process(&frame));
        audio.extend(to_audio.flush());
        audio.extend(vec![0.0f32; params.audio_rate]);
        let audio: Vec<f32> = audio.iter().map(|x| x * 0.25).collect();

        let found = run(&audio, params, &[]);
        assert!(
            found.iter().any(|f| f.decoded && f.bytes == 26),
            "the replay did not decode the frame: {found:?}"
        );
        let deaf = run(
            &audio,
            params,
            &[Muted {
                from_s: 0.0,
                until_s: f64::INFINITY,
            }],
        );
        assert!(deaf.is_empty(), "a muted replay still found {deaf:?}");
        let verdict = compare(&found, &deaf);
        assert!(!verdict.holds());
    }
}
