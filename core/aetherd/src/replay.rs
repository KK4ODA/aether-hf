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

use aether_phy::{AudioToBaseband, Complex, StreamingReceiver, WIDE_2300, WaveformParams};
use serde_json::Value;

use crate::record::{FrameRecord, read_wav};

/// The audio block a replay is fed in: twenty milliseconds, as the daemon's loop uses.
pub const BLOCK_S: f64 = 0.02;

/// An interval, in seconds of the recording, during which the receiver is muted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Muted {
    /// Start, inclusive.
    pub from_s: f64,
    /// End, exclusive; `f64::INFINITY` when the key was never released on record.
    pub until_s: f64,
}

/// A move to the other bandwidth during a recording (ADR-0026): from `at_s` on the station ran
/// `bandwidth_hz`, with a receiver built at that moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AirChange {
    /// When, in seconds of the recording.
    pub at_s: f64,
    /// The bandwidth from then on, in hertz.
    pub bandwidth_hz: u32,
}

/// What a sidecar says a replay should reproduce.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Expectation {
    /// The frames the recording station found.
    pub frames: Vec<FrameRecord>,
    /// When its transmitter was keyed, which is when its receiver was deaf.
    pub muted: Vec<Muted>,
    /// The waveform the station ran, in hertz: 2300 for a sidecar from before the
    /// narrow waveform existed, which never said.
    pub bandwidth_hz: u32,
    /// Where it moved to the other bandwidth, in order: a host program's `BW` command or a
    /// call in the narrower one (ADR-0026). Empty for most recordings.
    pub airs: Vec<AirChange>,
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
        let mut airs = Vec::new();
        let mut keyed_at: Option<f64> = None;
        for event in document["events"].as_array().into_iter().flatten() {
            let t = event["t_s"].as_f64().unwrap_or(0.0);
            if event["event"] == "air" {
                if let Some(hz) = event["detail"]
                    .as_str()
                    .and_then(|detail| detail.parse::<u32>().ok())
                {
                    airs.push(AirChange {
                        at_s: t,
                        bandwidth_hz: hz,
                    });
                }
                continue;
            }
            if event["event"] != "ptt" {
                continue;
            }
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
        let bandwidth_hz = document["session"]["bandwidth_hz"]
            .as_u64()
            .and_then(|hz| u32::try_from(hz).ok())
            .unwrap_or(2300);
        Ok(Self {
            frames,
            muted,
            bandwidth_hz,
            airs,
        })
    }
}

/// The waveform of a bandwidth this version runs.
fn params_for(bandwidth_hz: u32) -> Result<WaveformParams, String> {
    match bandwidth_hz {
        2300 => Ok(WIDE_2300),
        500 => Ok(aether_phy::waveform::NARROW_500),
        other => Err(format!("no waveform for {other} Hz in this version")),
    }
}

/// Run a recording through the receiver as its sidecar says it was made: the waveform it
/// started with, muted where the station transmitted, and a new receiver wherever the station
/// moved to the other bandwidth (ADR-0026).
///
/// # Errors
/// As [`replay`].
pub fn replay_expecting(
    wav: &Path,
    expectation: &Expectation,
    block_s: f64,
) -> Result<(Vec<FrameRecord>, BlockTiming), String> {
    let params = params_for(expectation.bandwidth_hz)?;
    let mut airs = Vec::new();
    for change in &expectation.airs {
        airs.push((change.at_s, params_for(change.bandwidth_hz)?));
    }
    let samples = read_audio(wav, params)?;
    Ok(run_moving(
        &samples,
        params,
        &expectation.muted,
        &airs,
        block_s,
    ))
}

/// Run a recording through the receiver, in the waveform it was made with.
///
/// Returns every frame found, in the sidecar's terms, with `t_s` relative to the start of
/// the recording.
///
/// # Errors
/// If the file cannot be read, is not at the modem's sample rate, or names a bandwidth
/// this version has no waveform for.
pub fn replay(wav: &Path, muted: &[Muted], bandwidth_hz: u32) -> Result<Vec<FrameRecord>, String> {
    replay_timed(wav, muted, bandwidth_hz, BLOCK_S).map(|(found, _)| found)
}

/// [`replay`] fed in blocks of `block_s` seconds, and how long the receiver took per block.
///
/// # Errors
/// As [`replay`].
pub fn replay_timed(
    wav: &Path,
    muted: &[Muted],
    bandwidth_hz: u32,
    block_s: f64,
) -> Result<(Vec<FrameRecord>, BlockTiming), String> {
    let params = params_for(bandwidth_hz)?;
    let samples = read_audio(wav, params)?;
    Ok(run_timed(&samples, params, muted, block_s))
}

/// A recording's samples, at the rate the modem runs.
fn read_audio(wav: &Path, params: WaveformParams) -> Result<Vec<f32>, String> {
    let audio = read_wav(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
    let rate = u32::try_from(params.audio_rate).unwrap_or(48_000);
    if audio.sample_rate != rate {
        return Err(format!(
            "{} is at {} Hz and the modem runs at {rate} Hz; resample it first",
            wav.display(),
            audio.sample_rate
        ));
    }
    Ok(audio.samples)
}

/// How long the receiver took per block, which is what the daemon's single loop spends on
/// it between two top-ups of the sound card. The loop keeps a quarter second queued, so a
/// block that costs more than that starves the card in the middle of a transmission — the
/// number a replay can measure without a radio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockTiming {
    /// Blocks fed.
    pub blocks: usize,
    /// The slowest block, in milliseconds of wall clock.
    pub max_ms: f64,
    /// Where in the recording the slowest block was, in seconds.
    pub max_at_s: f64,
    /// Blocks that took over 100 ms.
    pub over_100_ms: usize,
    /// Blocks that took over 250 ms — the daemon's playback backlog.
    pub over_250_ms: usize,
    /// Wall clock over the whole run, in milliseconds.
    pub total_ms: f64,
}

/// The receiver over samples already in memory.
#[must_use]
pub fn run(samples: &[f32], params: WaveformParams, muted: &[Muted]) -> Vec<FrameRecord> {
    run_timed(samples, params, muted, BLOCK_S).0
}

/// [`run`] fed in blocks of `block_s` seconds, and how long each block took.
#[must_use]
pub fn run_timed(
    samples: &[f32],
    params: WaveformParams,
    muted: &[Muted],
    block_s: f64,
) -> (Vec<FrameRecord>, BlockTiming) {
    run_moving(samples, params, muted, &[], block_s)
}

/// The receiver as the station ran it: a new front end and receiver wherever `airs` says the
/// station moved to another waveform (ADR-0026), counting from there as the live one did.
fn run_moving(
    samples: &[f32],
    params: WaveformParams,
    muted: &[Muted],
    airs: &[(f64, WaveformParams)],
    block_s: f64,
) -> (Vec<FrameRecord>, BlockTiming) {
    let mut timing = BlockTiming {
        blocks: 0,
        max_ms: 0.0,
        max_at_s: 0.0,
        over_100_ms: 0,
        over_250_ms: 0,
        total_ms: 0.0,
    };
    let started = std::time::Instant::now();
    let mut front = AudioToBaseband::new(params);
    let mut receiver = StreamingReceiver::new(params, 6.0, true);
    let mut air = aether_phy::modes::air_interface(params);
    // where the receiver running now started, in baseband samples of the recording
    let mut origin = 0usize;
    let mut baseband_seen = 0usize;
    let mut next_air = 0usize;
    let block = ((block_s * params.audio_rate as f64) as usize).max(1);
    let fs = params.fs_baseband;
    let rate = params.audio_rate as f64;
    let mut found = Vec::new();
    let mut seen = 0usize;
    for chunk in samples.chunks(block) {
        let began = std::time::Instant::now();
        let t = seen as f64 / rate;
        while let Some(&(_, moved)) = airs.get(next_air).filter(|(at_s, _)| *at_s <= t) {
            front = AudioToBaseband::new(moved);
            receiver = StreamingReceiver::new(moved, 6.0, true);
            air = aether_phy::modes::air_interface(moved);
            origin = baseband_seen;
            next_air += 1;
        }
        let baseband = front.process(chunk);
        seen += chunk.len();
        // muted where the station was transmitting: what the card captured then was its
        // own sidetone, and the live receiver never saw it
        let deaf = muted.iter().any(|m| t >= m.from_s && t < m.until_s);
        if deaf {
            let silence = vec![(0.0, 0.0); baseband.len()];
            absorb(&silence, &mut receiver, &air, origin, fs, &mut found);
        } else {
            absorb(&baseband, &mut receiver, &air, origin, fs, &mut found);
        }
        baseband_seen += baseband.len();
        let ms = began.elapsed().as_secs_f64() * 1000.0;
        timing.blocks += 1;
        if ms > timing.max_ms {
            timing.max_ms = ms;
            timing.max_at_s = t;
        }
        if ms > 100.0 {
            timing.over_100_ms += 1;
        }
        if ms > 250.0 {
            timing.over_250_ms += 1;
        }
    }
    // let the last frame out: the receiver holds samples back for its filters
    let tail = vec![0.0f32; block * 8];
    let baseband = front.process(&tail);
    absorb(&baseband, &mut receiver, &air, origin, fs, &mut found);
    timing.total_ms = started.elapsed().as_secs_f64() * 1000.0;
    (found, timing)
}

/// Feed the receiver a block and keep what it found, in the sidecar's terms: `origin` is where
/// this receiver's count starts in the recording's.
fn absorb(
    baseband: &[Complex],
    receiver: &mut StreamingReceiver,
    air: &aether_phy::modes::AirInterface,
    origin: usize,
    fs: f64,
    found: &mut Vec<FrameRecord>,
) {
    for decoded in receiver.feed(baseband) {
        let control = decoded.frame.is_control();
        // the rung a DATA frame was sent at, as the station reports it; chips naming an
        // OFDM mode on no rung of the ladder are noise, which the station drops too
        let Some(rung) = (if control {
            Some(0)
        } else {
            decoded.frame.rung(air)
        }) else {
            continue;
        };
        let detected = decoded.frame.detect_confidence(air);
        found.push(FrameRecord {
            t_s: (origin + decoded.frame.start()) as f64 / fs,
            kind: if control {
                "control".to_owned()
            } else {
                "data".to_owned()
            },
            mode: rung,
            rv: decoded.frame.rv(),
            snr_3k_db: decoded.frame.snr_3k_db(),
            cfo_hz: crate::station::reported_cfo(
                decoded.ok(),
                decoded.frame.mode_confidence(),
                detected,
                decoded.frame.cfo_hz(),
            ),
            confidence: decoded.frame.mode_confidence(),
            detect_confidence: detected,
            decoded: decoded.ok(),
            bytes: decoded.payload.as_ref().map_or(0, Vec::len),
            control: if control {
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
    let cfo = frame
        .cfo_hz
        .map_or_else(|| "     —".to_owned(), |hz| format!("{hz:>+6.1}"));
    // both confidences, because they answer different questions: the chips say how sure
    // the mode reading is (DATA frames only), the acquisition says whether there was a
    // frame there at all — the number that separates a phantom from a signal
    format!(
        "{:>8.2} s  {:<7} mode {:>2} rv {}  {:>+6.1} dB  cfo {cfo} Hz  acq {:>4.2} chip {:>4.2}  {}",
        frame.t_s,
        frame.kind,
        frame.mode,
        frame.rv,
        frame.snr_3k_db,
        frame.detect_confidence,
        frame.confidence,
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
                    {"t_s": 3.5, "event": "air", "detail": "500", "state": "Idle"},
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
        // a move to the other bandwidth (ADR-0026) is where the replay builds its receiver again
        assert_eq!(
            expectation.airs,
            vec![AirChange {
                at_s: 3.5,
                bandwidth_hz: 500
            }]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recording_that_moves_between_bandwidths_replays_with_the_receiver_of_each() {
        // ADR-0026: a station moves between its two waveforms and its recording goes on; the
        // replay builds its receiver again where the sidecar says, as the station did, and
        // places what it finds in the recording's own time
        let clip = |params: WaveformParams| -> Vec<f32> {
            let mut modem = aether_phy::Modem::new(params, false);
            let air = modem.air();
            // the first OFDM rung: a frame only this air's receiver decodes
            let rung = air.floor_modes();
            let bytes = air.ladder()[rung].payload_bytes();
            let payload: Vec<u8> = (0..bytes).map(|i| (i * 7 % 256) as u8).collect();
            let frame = modem.rung_burst(&payload, rung, 0).expect("a frame");
            let mut to_audio = aether_phy::BasebandToAudio::new(params);
            let mut audio = vec![0.0f32; params.audio_rate];
            audio.extend(to_audio.process(&frame));
            audio.extend(to_audio.flush());
            audio.extend(vec![0.0f32; params.audio_rate]);
            audio.iter().map(|x| x * 0.25).collect()
        };
        let narrow = aether_phy::waveform::NARROW_500;
        let wide = clip(WIDE_2300);
        let moved_at = wide.len() as f64 / WIDE_2300.audio_rate as f64;
        let audio: Vec<f32> = wide.iter().chain(clip(narrow).iter()).copied().collect();
        let (found, _) = run_moving(&audio, WIDE_2300, &[], &[(moved_at, narrow)], BLOCK_S);
        let decoded: Vec<&FrameRecord> = found.iter().filter(|f| f.decoded).collect();
        assert_eq!(decoded.len(), 2, "{found:?}");
        assert!(decoded[0].t_s < moved_at, "{found:?}");
        assert!(decoded[1].t_s > moved_at + 0.5, "{found:?}");
        // without the move the wide receiver never decodes the narrow frame
        let (unmoved, _) = run_moving(&audio, WIDE_2300, &[], &[], BLOCK_S);
        assert_eq!(
            unmoved.iter().filter(|f| f.decoded).count(),
            1,
            "{unmoved:?}"
        );
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
