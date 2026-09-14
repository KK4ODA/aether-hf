//! Session recordings: the audio the radio delivered, and what the modem made of it.
//!
//! Field validation (roadmap Phase 6) is built on recorded sessions. A recording is two
//! files with one name: a mono 16-bit WAV of everything the sound card captured — the
//! channel as this station heard it, at the modem's own 48 kHz — and a JSON sidecar saying
//! what the modem did with it: every frame it found, with mode, SNR, offset and whether it
//! decoded; every session event; when the transmitter was keyed; the counters at the end;
//! and whatever the operator wrote down about the band and the other station.
//!
//! What is *not* recorded is the audio this station played. The WAV is for replaying the
//! receiver, and the sidecar says what was sent and when; a stereo capture-and-playback
//! file would double the size for an analysis nobody has asked for yet.
//!
//! The WAV's header is patched every few seconds so a daemon that dies mid-session leaves a
//! file that plays, and the sidecar is rewritten on the same schedule so the story survives
//! the same failure.

use std::{
    io::{Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The sidecar's `format` field: bumped when its shape changes.
pub const FORMAT: &str = "aether-hf-session/1";

/// One frame the receiver found, as the sidecar records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameRecord {
    /// Seconds since the recording started, by the station's audio clock.
    pub t_s: f64,
    /// `data` or `control`.
    pub kind: String,
    /// Mode index read from the pilot chips (or the control mode).
    pub mode: usize,
    /// Redundancy version.
    pub rv: u8,
    /// Signal to noise, 3 kHz reference.
    pub snr_3k_db: f64,
    /// Carrier offset the receiver removed.
    pub cfo_hz: f64,
    /// Whether the payload came through.
    pub decoded: bool,
    /// Payload length when it did.
    pub bytes: usize,
}

/// A recording in progress.
#[derive(Debug)]
pub struct Recording {
    wav: PathBuf,
    sidecar: PathBuf,
    file: std::io::BufWriter<std::fs::File>,
    sample_rate: u32,
    samples: u64,
    /// Samples written at the last header patch.
    patched_at: u64,
    started: String,
    /// The station clock when the recording began, so `t_s` starts at zero.
    t0: f64,
    frames: Vec<FrameRecord>,
    events: Vec<Value>,
    /// Who and what: callsigns, settings, the operator's notes.
    meta: Value,
}

/// What `stop` reports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The audio file.
    pub wav: PathBuf,
    /// The sidecar.
    pub sidecar: PathBuf,
    /// Length of the audio.
    pub seconds: f64,
    /// Frames the receiver found.
    pub frames: usize,
    /// Of those, frames that decoded.
    pub decoded: usize,
}

/// How often the header and sidecar are refreshed, in samples (five seconds at 48 kHz).
const PATCH_EVERY: u64 = 5 * 48_000;

impl Recording {
    /// Begin a recording under `dir`, named after the time and the stations.
    ///
    /// `meta` is what the sidecar says about the session besides what the modem observes:
    /// callsigns, the settings that matter, the operator's notes.
    ///
    /// # Errors
    /// If the directory cannot be made or the file cannot be created.
    pub fn start(
        dir: &Path,
        name: &str,
        sample_rate: u32,
        now_s: f64,
        meta: Value,
    ) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let wav = dir.join(format!("{name}.wav"));
        let sidecar = dir.join(format!("{name}.json"));
        let mut file = std::io::BufWriter::new(std::fs::File::create(&wav)?);
        file.write_all(&wav_header(sample_rate, 0))?;
        let recording = Self {
            wav,
            sidecar,
            file,
            sample_rate,
            samples: 0,
            patched_at: 0,
            started: crate::log::rfc3339(unix_ms()),
            t0: now_s,
            frames: Vec::new(),
            events: Vec::new(),
            meta,
        };
        recording.write_sidecar(None)?;
        Ok(recording)
    }

    /// The audio file being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.wav
    }

    /// Seconds of audio so far.
    #[must_use]
    pub fn seconds(&self) -> f64 {
        self.samples as f64 / f64::from(self.sample_rate)
    }

    /// Everything the sound card delivered, in order.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn captured(&mut self, audio: &[f32]) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(audio.len() * 2);
        for &sample in audio {
            // symmetric 16-bit: ±1.0 maps to ±32767, and anything beyond is clipped rather
            // than wrapped — a clipped sample is loud, a wrapped one is a click
            let scaled = (f64::from(sample) * 32_767.0)
                .round()
                .clamp(-32_768.0, 32_767.0);
            bytes.extend_from_slice(&(scaled as i16).to_le_bytes());
        }
        self.file.write_all(&bytes)?;
        self.samples += audio.len() as u64;
        if self.samples - self.patched_at >= PATCH_EVERY {
            self.patch_header()?;
            self.write_sidecar(None)?;
            self.patched_at = self.samples;
        }
        Ok(())
    }

    /// A frame the receiver found.
    pub fn frame(&mut self, mut record: FrameRecord) {
        record.t_s -= self.t0;
        self.frames.push(record);
    }

    /// Something that happened: a session event, the transmitter keyed or released.
    pub fn event(&mut self, now_s: f64, event: &str, detail: &str, state: &str) {
        self.events.push(json!({
            "t_s": now_s - self.t0,
            "event": event,
            "detail": detail,
            "state": state,
        }));
    }

    /// Finish: patch the header, write the sidecar with the counters, close.
    ///
    /// # Errors
    /// If the files cannot be written.
    pub fn finish(mut self, counters: &Value) -> std::io::Result<Summary> {
        self.patch_header()?;
        self.file.flush()?;
        self.write_sidecar(Some(counters))?;
        Ok(Summary {
            wav: self.wav.clone(),
            sidecar: self.sidecar.clone(),
            seconds: self.seconds(),
            frames: self.frames.len(),
            decoded: self.frames.iter().filter(|f| f.decoded).count(),
        })
    }

    fn patch_header(&mut self) -> std::io::Result<()> {
        self.file.flush()?;
        let file = self.file.get_mut();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&wav_header(self.sample_rate, self.samples))?;
        file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    fn write_sidecar(&self, counters: Option<&Value>) -> std::io::Result<()> {
        let ended = counters.is_some().then(|| crate::log::rfc3339(unix_ms()));
        let document = json!({
            "format": FORMAT,
            "version": env!("CARGO_PKG_VERSION"),
            "started": self.started,
            "ended": ended,
            "audio": {
                "file": self.wav.file_name().map(|n| n.to_string_lossy().into_owned()),
                "sample_rate": self.sample_rate,
                "channels": 1,
                "encoding": "pcm16",
                "samples": self.samples,
                "seconds": self.seconds(),
            },
            "session": self.meta,
            "events": self.events,
            "frames": self.frames,
            "counters": counters,
        });
        // written beside and renamed over, so a reader never sees half a document
        let temporary = self.sidecar.with_extension("json.new");
        std::fs::write(&temporary, serde_json::to_string_pretty(&document)?)?;
        std::fs::rename(&temporary, &self.sidecar)
    }
}

/// A canonical 44-byte header for mono 16-bit PCM.
fn wav_header(sample_rate: u32, samples: u64) -> [u8; 44] {
    let data_bytes = u32::try_from(samples * 2).unwrap_or(u32::MAX);
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    header[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    header[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    header[28..32].copy_from_slice(&(sample_rate * 2).to_le_bytes());
    header[32..34].copy_from_slice(&2u16.to_le_bytes());
    header[34..36].copy_from_slice(&16u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    header
}

/// The name a recording gets: when, who, and with whom.
#[must_use]
pub fn session_name(callsign: &str, remote: Option<&str>) -> String {
    let stamp = crate::log::rfc3339(unix_ms());
    // 2026-09-14T12:34:56.789Z -> 20260914-123456
    let compact: String = stamp
        .chars()
        .take(19)
        .filter(|c| c.is_ascii_digit() || *c == 'T')
        .collect::<String>()
        .replace('T', "-");
    let clean = |call: &str| call.replace(['/', '\\', ':'], "-");
    match remote {
        Some(remote) => format!("{compact}_{}_{}", clean(callsign), clean(remote)),
        None => format!("{compact}_{}_listen", clean(callsign)),
    }
}

/// A recorded WAV read back: the samples, and the rate they were taken at.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    /// Samples per second.
    pub sample_rate: u32,
    /// Mono samples, ±1.0.
    pub samples: Vec<f32>,
}

/// Read a mono or multi-channel PCM WAV (16-bit, or 32-bit float), taking the first channel.
///
/// Enough of the format for what this project writes and what a sound-card recorder or the
/// reference model produces; not a general WAV reader. A truncated `data` length — a daemon
/// that died before its last header patch — is taken as "to the end of the file".
///
/// # Errors
/// If the file cannot be read or is not a PCM WAV this can use.
pub fn read_wav(path: &Path) -> std::io::Result<Wav> {
    let bytes = std::fs::read(path)?;
    let bad = |what: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, what.to_owned());
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut pos = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None; // tag, channels, rate, bits
    let mut data: Option<(usize, usize)> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            let tag = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            let channels = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            let rate = u32::from_le_bytes([
                bytes[body + 4],
                bytes[body + 5],
                bytes[body + 6],
                bytes[body + 7],
            ]);
            let bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
            format = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let declared = body.saturating_add(size).min(bytes.len());
            // A header that says less than the file holds is a recording whose writer died
            // between header patches: the audio runs to the end of the file, unless what
            // follows the declared end is another chunk, in which case the header is right.
            let end = if declared < bytes.len() && !looks_like_chunk(&bytes[declared..]) {
                bytes.len()
            } else {
                declared
            };
            data = Some((body, end));
            break;
        }
        pos = body + size + (size % 2);
    }
    let (tag, channels, rate, bits) = format.ok_or_else(|| bad("no fmt chunk"))?;
    let (start, end) = data.ok_or_else(|| bad("no data chunk"))?;
    let channels = usize::from(channels.max(1));
    let samples = match (tag, bits) {
        (1, 16) => bytes[start..end]
            .chunks_exact(2 * channels)
            .map(|frame| f32::from(i16::from_le_bytes([frame[0], frame[1]])) / 32_768.0)
            .collect(),
        (3, 32) => bytes[start..end]
            .chunks_exact(4 * channels)
            .map(|frame| f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]))
            .collect(),
        _ => return Err(bad("only 16-bit PCM and 32-bit float WAV are read")),
    };
    Ok(Wav {
        sample_rate: rate,
        samples,
    })
}

/// Whether bytes begin with a RIFF chunk header: four printable ASCII characters and a size.
fn looks_like_chunk(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && bytes[..4].iter().all(|b| (b' '..=b'~').contains(b))
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-rec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_recording_round_trips_through_the_wav_it_writes() {
        let dir = temp_dir("roundtrip");
        let audio: Vec<f32> = (0..96_000).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
        let mut recording =
            Recording::start(&dir, "test", 48_000, 10.0, json!({"callsign": "W4ODA"}))
                .expect("start");
        for block in audio.chunks(4_096) {
            recording.captured(block).expect("write");
        }
        recording.frame(FrameRecord {
            t_s: 11.5,
            kind: "data".into(),
            mode: 4,
            rv: 0,
            snr_3k_db: 7.25,
            cfo_hz: -3.0,
            decoded: true,
            bytes: 144,
        });
        recording.event(12.0, "connected", "W4XYZ", "Connected");
        let summary = recording
            .finish(&json!({"frames_sent": 3}))
            .expect("finish");
        assert!((summary.seconds - 2.0).abs() < 1e-9);
        assert_eq!(summary.frames, 1);
        assert_eq!(summary.decoded, 1);

        let back = read_wav(&summary.wav).expect("read");
        assert_eq!(back.sample_rate, 48_000);
        assert_eq!(back.samples.len(), audio.len());
        let worst = back
            .samples
            .iter()
            .zip(&audio)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1.0 / 32_000.0,
            "16-bit round trip is off by {worst}"
        );

        let sidecar: Value =
            serde_json::from_str(&std::fs::read_to_string(&summary.sidecar).expect("sidecar"))
                .expect("json");
        assert_eq!(sidecar["format"], FORMAT);
        assert_eq!(sidecar["audio"]["samples"], 96_000);
        assert_eq!(sidecar["session"]["callsign"], "W4ODA");
        // times are relative to the start of the recording
        assert!((sidecar["frames"][0]["t_s"].as_f64().unwrap() - 1.5).abs() < 1e-9);
        assert!((sidecar["events"][0]["t_s"].as_f64().unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(sidecar["counters"]["frames_sent"], 3);
        assert!(sidecar["ended"].is_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recording_cut_short_still_plays_to_where_it_got() {
        // the header is patched every five seconds; a daemon that died a second after the
        // last patch leaves a file whose declared length is short — and the reader takes
        // the data that is actually there, not the header's word for it
        let dir = temp_dir("short");
        let mut recording = Recording::start(&dir, "short", 48_000, 0.0, json!({})).expect("start");
        for _ in 0..7 {
            recording.captured(&vec![0.25f32; 48_000]).expect("write");
        }
        // no finish: drop it as a crash would
        let wav = recording.path().to_path_buf();
        recording.file.flush().expect("flush");
        drop(recording);
        let back = read_wav(&wav).expect("read");
        assert_eq!(
            back.samples.len(),
            7 * 48_000,
            "the un-patched tail was dropped"
        );
        // and the sidecar written at the last patch is a complete document
        let sidecar: Value = serde_json::from_str(
            &std::fs::read_to_string(wav.with_extension("json")).expect("sidecar"),
        )
        .expect("json");
        assert_eq!(sidecar["audio"]["samples"], 5 * 48_000);
        assert!(sidecar["ended"].is_null());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_reader_takes_the_first_channel_of_a_float_file() {
        // what the reference model's HAL and most recorders write: stereo 32-bit float
        let dir = temp_dir("float");
        let path = dir.join("stereo.wav");
        let mut bytes = Vec::new();
        let frames = 100u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + frames * 8).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes()); // float
        bytes.extend_from_slice(&2u16.to_le_bytes()); // stereo
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000u32 * 8).to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(frames * 8).to_le_bytes());
        for i in 0..frames {
            bytes.extend_from_slice(&(i as f32 / 100.0).to_le_bytes());
            bytes.extend_from_slice(&(-1.0f32).to_le_bytes());
        }
        std::fs::write(&path, bytes).expect("write");
        let back = read_wav(&path).expect("read");
        assert_eq!(back.samples.len(), 100);
        assert!((back.samples[42] - 0.42).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_names_are_sortable_and_safe_for_a_file_system() {
        let name = session_name("KK4ODA/M", Some("W4XYZ"));
        assert!(name.ends_with("_KK4ODA-M_W4XYZ"), "{name}");
        assert_eq!(
            name.len(),
            "20260914-123456".len() + "_KK4ODA-M_W4XYZ".len(),
            "{name}"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert!(session_name("W4ODA", None).ends_with("_W4ODA_listen"));
    }
}
