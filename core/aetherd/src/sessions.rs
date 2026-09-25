//! The sessions this station has had.
//!
//! The stations-heard list (`heard.rs`) keeps one entry per callsign: who is out there and
//! how they were last heard, which is what an operator wants before calling somebody. What
//! it cannot say is what happened each time — when a session came up, how long it lasted,
//! who called whom, what crossed, how fast it ran and how it ended. That is this list: one
//! entry per session, newest first, written when the session ends. After three test
//! sessions with ND1J (2026-09-25) the only record of which had reached which rung was a
//! reading of the recordings' sidecars.
//!
//! Bounded, and kept in a file beside the configuration like the stations heard, so a
//! restart does not forget it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The most sessions kept; the oldest goes when a new one arrives past it.
pub const LIMIT: usize = 500;

/// Which end of the session this station was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// This station placed the call.
    Caller,
    /// The other station called this one.
    Called,
}

/// One session, as the list keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// The other station's callsign.
    pub remote: String,
    /// When it came up, milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// When it ended.
    pub ended_ms: u64,
    /// How long it was up, seconds.
    pub duration_s: f64,
    /// Which end this station was.
    pub role: Role,
    /// The air it ran on, hertz: 2300 or 500.
    pub bandwidth_hz: u32,
    /// The dial frequency, when the radio could say.
    pub frequency_hz: Option<u64>,
    /// Application bytes handed to the link, before compression.
    pub bytes_sent: usize,
    /// Link bytes the other station acknowledged — what actually crossed, after compression.
    pub bytes_acked: usize,
    /// Application bytes received, after decompression.
    pub bytes_received: usize,
    /// How it ended, in the link's words: `closed` (this station's disconnect,
    /// acknowledged), `closed (no disc ack)`, `peer disconnected`, `link timeout`,
    /// `no response`, `aborted`.
    pub end: String,
    /// The SNR of the last frame decoded from the other station, dB (3 kHz reference).
    pub snr_db: Option<f64>,
    /// The best SNR any of its decoded frames had.
    pub best_snr_db: Option<f64>,
    /// How the other station last said it heard this one, from its acknowledgements.
    pub heard_there_db: Option<f64>,
    /// The fastest rung this station sent data at.
    pub top_rung_sent: Option<usize>,
    /// The fastest rung of data this station decoded from the other.
    pub top_rung_heard: Option<usize>,
    /// Whether the session was a Test session's.
    pub test: bool,
    /// The recording it went into, by file name, when it was recorded.
    pub recording: Option<String>,
}

impl Session {
    /// The session with its wall-clock times, given when it ended: the station keeps its
    /// own clock, the audio's, and it is the daemon that knows the date.
    #[must_use]
    pub fn ended_at(mut self, ended_ms: u64) -> Self {
        self.ended_ms = ended_ms;
        self.started_ms = ended_ms.saturating_sub((self.duration_s.max(0.0) * 1000.0) as u64);
        self
    }
}

#[derive(Serialize, Deserialize)]
struct File {
    schema: u32,
    sessions: Vec<Session>,
}

/// The list, and the file it lives in.
#[derive(Debug)]
pub struct SessionLog {
    sessions: Vec<Session>,
    path: Option<PathBuf>,
}

impl SessionLog {
    /// A list kept in `path`, loaded from it if it exists. A file that cannot be read means
    /// an empty list, as for the stations heard: a history is not worth refusing to start.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let mut sessions = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<File>(&text).ok())
            .filter(|file| file.schema == 1)
            .map_or_else(Vec::new, |file| file.sessions);
        sessions.sort_by_key(|s| std::cmp::Reverse(s.ended_ms));
        sessions.truncate(LIMIT);
        Self { sessions, path }
    }

    /// Add a session that has ended.
    pub fn add(&mut self, session: Session) {
        self.sessions.insert(0, session);
        self.sessions.truncate(LIMIT);
    }

    /// The sessions, newest first — with one station, when `remote` names it.
    #[must_use]
    pub fn sessions(&self, remote: Option<&str>) -> Vec<Session> {
        let remote = remote.map(|r| r.trim().to_ascii_uppercase());
        self.sessions
            .iter()
            .filter(|s| remote.as_deref().is_none_or(|r| s.remote == r))
            .cloned()
            .collect()
    }

    /// Forget them all. Returns how many there were.
    pub fn clear(&mut self) -> usize {
        let count = self.sessions.len();
        self.sessions.clear();
        count
    }

    /// Where the list is kept, if anywhere.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Write the list, whole, to a temporary file renamed into place — a session ends
    /// rarely enough to write at once, and a crash mid-write leaves the previous list.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = File {
            schema: 1,
            sessions: self.sessions.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(remote: &str, ended_ms: u64) -> Session {
        Session {
            remote: remote.to_owned(),
            started_ms: 0,
            ended_ms,
            duration_s: 60.0,
            role: Role::Caller,
            bandwidth_hz: 2300,
            frequency_hz: Some(3_588_000),
            bytes_sent: 2048,
            bytes_acked: 2100,
            bytes_received: 12,
            end: "closed".to_owned(),
            snr_db: Some(4.5),
            best_snr_db: Some(6.0),
            heard_there_db: Some(-0.5),
            top_rung_sent: Some(3),
            top_rung_heard: Some(7),
            test: true,
            recording: Some("20260925-015107_KK4ODA-1_ND1J_test".to_owned()),
        }
    }

    #[test]
    fn a_session_is_placed_on_the_calendar_by_when_it_ended() {
        let s = session("ND1J", 0).ended_at(1_790_000_000_000);
        assert_eq!(s.ended_ms, 1_790_000_000_000);
        assert_eq!(s.started_ms, 1_790_000_000_000 - 60_000);
    }

    #[test]
    fn the_sessions_are_newest_first_bounded_and_found_by_station() {
        let mut log = SessionLog::open(None);
        for i in 0..(LIMIT as u64 + 5) {
            log.add(session(
                if i % 2 == 0 { "ND1J" } else { "W4TGA" },
                1_000 + i,
            ));
        }
        let all = log.sessions(None);
        assert_eq!(all.len(), LIMIT);
        assert_eq!(all[0].ended_ms, 1_000 + LIMIT as u64 + 4);
        let nd1j = log.sessions(Some("nd1j"));
        assert!(!nd1j.is_empty());
        assert!(nd1j.iter().all(|s| s.remote == "ND1J"));
        assert!(nd1j.windows(2).all(|w| w[0].ended_ms > w[1].ended_ms));
        assert_eq!(log.clear(), LIMIT);
        assert!(log.sessions(None).is_empty());
    }

    #[test]
    fn the_sessions_survive_a_restart_in_their_file() {
        let dir = std::env::temp_dir().join(format!("aether-sessions-{}", std::process::id()));
        let path = dir.join("sessions.json");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut log = SessionLog::open(Some(path.clone()));
            assert!(log.sessions(None).is_empty());
            log.add(session("ND1J", 1_000));
            log.add(session("W4TGA", 2_000));
            log.save().expect("written");
        }
        let log = SessionLog::open(Some(path.clone()));
        let sessions = log.sessions(None);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].remote, "W4TGA");
        assert_eq!(sessions[1], session("ND1J", 1_000));
        // a damaged file is an empty history, not a refusal to start
        std::fs::write(&path, "{not json").expect("write");
        assert!(SessionLog::open(Some(path)).sessions(None).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
