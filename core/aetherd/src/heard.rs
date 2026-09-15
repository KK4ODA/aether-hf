//! The stations this one has heard.
//!
//! Every frame that carries a callsign is a sighting: a beacon, a connect request or an
//! answer overheard between two other stations, and every frame of a session with the
//! station at the other end. The list keeps one entry per callsign with when it was first
//! and last heard, how often, how strong, on what frequency and in what mode, and what it
//! was doing — which is most of what an operator wants to know before calling somebody,
//! and all of what a station left listening overnight can tell them in the morning.
//!
//! It is bounded, so a busy channel cannot grow it without limit, and it is kept in a
//! file beside the configuration, so a restart does not forget the night. The file is
//! written when something changed and a moment has passed, never on every frame.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The most stations kept. The quietest-longest is dropped when a new one arrives past it.
pub const LIMIT: usize = 200;

/// What a station was heard doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activity {
    /// A beacon, addressed to nobody.
    Beacon,
    /// A connect request, addressed to somebody.
    Calling,
    /// An answer to a connect request, or to a probe.
    Answering,
    /// A probe (ADR-0006), addressed to somebody.
    Probing,
    /// The other end of a session with this station.
    Connected,
}

/// One station, as the list keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeardStation {
    /// The callsign, as it was packed on the air.
    pub callsign: String,
    /// Milliseconds since the Unix epoch, first and last.
    pub first_heard_ms: u64,
    /// The most recent sighting.
    pub last_heard_ms: u64,
    /// Frames heard from it.
    pub count: u32,
    /// SNR of the last frame, dB, 3 kHz reference.
    pub snr_db: f64,
    /// The best SNR any of its frames had.
    pub best_snr_db: f64,
    /// The dial frequency at the last sighting, when the radio could say.
    pub frequency_hz: Option<u64>,
    /// The mode of the last frame.
    pub mode: usize,
    /// What it was doing when last heard.
    pub activity: Activity,
    /// Who it was calling or answering, if anybody.
    pub detail: Option<String>,
    /// Whether a session with it has ever been up from here.
    pub connected: bool,
}

/// One frame with a callsign in it.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    /// Whose frame it was.
    pub callsign: String,
    /// When, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// How strong.
    pub snr_db: f64,
    /// In what mode, when the frame's mode says something about the station: a control
    /// frame is always sent in the lowest, and says nothing.
    pub mode: Option<usize>,
    /// On what dial frequency, when known.
    pub frequency_hz: Option<u64>,
    /// What it was doing.
    pub activity: Activity,
    /// Who it was doing it to.
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct File {
    schema: u32,
    stations: Vec<HeardStation>,
}

/// The list, and the file it lives in.
#[derive(Debug)]
pub struct HeardList {
    stations: Vec<HeardStation>,
    path: Option<PathBuf>,
    dirty: bool,
}

impl HeardList {
    /// A list kept in `path`, loaded from it if it exists. A file that cannot be read —
    /// missing, or from a version that wrote something else — means an empty list; the
    /// stations heard are not worth refusing to start over.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let stations = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<File>(&text).ok())
            .filter(|file| file.schema == 1)
            .map_or_else(Vec::new, |file| file.stations);
        let mut list = Self {
            stations,
            path,
            dirty: false,
        };
        list.stations.truncate(LIMIT);
        list.sort();
        list
    }

    /// Record a sighting. Returns the entry as it now stands.
    pub fn note(&mut self, sighting: Sighting) -> HeardStation {
        let callsign = sighting.callsign.trim().to_ascii_uppercase();
        let entry = if let Some(entry) = self.stations.iter_mut().find(|s| s.callsign == callsign) {
            entry.last_heard_ms = entry.last_heard_ms.max(sighting.at_ms);
            entry.count = entry.count.saturating_add(1);
            entry.snr_db = sighting.snr_db;
            entry.best_snr_db = entry.best_snr_db.max(sighting.snr_db);
            if sighting.frequency_hz.is_some() {
                entry.frequency_hz = sighting.frequency_hz;
            }
            if let Some(mode) = sighting.mode {
                entry.mode = mode;
            }
            entry.activity = sighting.activity;
            entry.detail = sighting.detail;
            entry.connected |= sighting.activity == Activity::Connected;
            entry.clone()
        } else {
            let entry = HeardStation {
                callsign,
                first_heard_ms: sighting.at_ms,
                last_heard_ms: sighting.at_ms,
                count: 1,
                snr_db: sighting.snr_db,
                best_snr_db: sighting.snr_db,
                frequency_hz: sighting.frequency_hz,
                mode: sighting.mode.unwrap_or(0),
                activity: sighting.activity,
                detail: sighting.detail,
                connected: sighting.activity == Activity::Connected,
            };
            self.stations.push(entry.clone());
            entry
        };
        self.sort();
        // newest first, so what falls off the end is what was heard longest ago
        self.stations.truncate(LIMIT);
        self.dirty = true;
        entry
    }

    /// The stations, most recently heard first.
    #[must_use]
    pub fn stations(&self) -> &[HeardStation] {
        &self.stations
    }

    /// Forget them all. Returns how many there were.
    pub fn clear(&mut self) -> usize {
        let count = self.stations.len();
        self.stations.clear();
        self.dirty = true;
        count
    }

    /// Whether there is something to write.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Where the list is kept, if anywhere.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Write the list if it changed. Written whole to a temporary file and renamed into
    /// place, so a crash mid-write leaves the previous list rather than half of one.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn save(&mut self) -> std::io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = &self.path else {
            self.dirty = false;
            return Ok(());
        };
        let file = File {
            schema: 1,
            stations: self.stations.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)?;
        self.dirty = false;
        Ok(())
    }

    fn sort(&mut self) {
        self.stations
            .sort_by_key(|s| std::cmp::Reverse(s.last_heard_ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sighting(call: &str, at_ms: u64, snr_db: f64) -> Sighting {
        Sighting {
            callsign: call.to_owned(),
            at_ms,
            snr_db,
            mode: Some(3),
            frequency_hz: Some(14_107_000),
            activity: Activity::Beacon,
            detail: None,
        }
    }

    #[test]
    fn a_station_heard_twice_is_one_entry_with_its_history() {
        let mut list = HeardList::open(None);
        list.note(sighting("w4tga", 1_000, 8.0));
        let entry = list.note(Sighting {
            activity: Activity::Calling,
            detail: Some("KK4ODA".to_owned()),
            frequency_hz: None,
            mode: None,
            ..sighting("W4TGA", 5_000, 3.5)
        });
        assert_eq!(list.stations().len(), 1);
        assert_eq!(entry.callsign, "W4TGA");
        assert_eq!(entry.first_heard_ms, 1_000);
        assert_eq!(entry.last_heard_ms, 5_000);
        assert_eq!(entry.count, 2);
        assert!((entry.snr_db - 3.5).abs() < 1e-9);
        assert!((entry.best_snr_db - 8.0).abs() < 1e-9);
        // a frequency the radio could not give this time does not erase the last one, and
        // a control frame says nothing about the mode
        assert_eq!(entry.frequency_hz, Some(14_107_000));
        assert_eq!(entry.mode, 3);
        assert_eq!(entry.activity, Activity::Calling);
        assert_eq!(entry.detail.as_deref(), Some("KK4ODA"));
        assert!(!entry.connected);
        list.note(Sighting {
            activity: Activity::Connected,
            ..sighting("W4TGA", 6_000, 4.0)
        });
        assert!(list.stations()[0].connected);
    }

    #[test]
    fn the_list_is_bounded_and_drops_what_was_heard_longest_ago() {
        let mut list = HeardList::open(None);
        for i in 0..(LIMIT as u64 + 20) {
            list.note(sighting(&format!("N{i}X"), 1_000 + i, 5.0));
        }
        assert_eq!(list.stations().len(), LIMIT);
        assert_eq!(list.stations()[0].callsign, format!("N{}X", LIMIT + 19));
        assert!(!list.stations().iter().any(|s| s.callsign == "N0X"));
        // hearing an old one again brings it back to the top
        list.note(sighting("N25X", 9_999, 5.0));
        assert_eq!(list.stations()[0].callsign, "N25X");
        assert_eq!(list.stations().len(), LIMIT);
    }

    #[test]
    fn the_list_survives_a_restart_in_its_file() {
        let dir = std::env::temp_dir().join(format!("aether-heard-{}", std::process::id()));
        let path = dir.join("heard.json");
        let _ = std::fs::remove_dir_all(&dir);
        {
            let mut list = HeardList::open(Some(path.clone()));
            assert!(list.stations().is_empty());
            list.note(sighting("W4TGA", 1_000, 8.0));
            list.note(sighting("KK4XYZ", 2_000, 2.0));
            assert!(list.dirty());
            list.save().expect("written");
            assert!(!list.dirty());
            list.save().expect("nothing to write is fine");
        }
        let mut list = HeardList::open(Some(path.clone()));
        assert_eq!(list.stations().len(), 2);
        assert_eq!(list.stations()[0].callsign, "KK4XYZ");
        assert_eq!(list.stations()[1].first_heard_ms, 1_000);
        assert_eq!(list.clear(), 2);
        list.save().expect("written");
        assert!(HeardList::open(Some(path.clone())).stations().is_empty());
        // a file from some other version, or a damaged one, is an empty list, not a refusal
        std::fs::write(&path, "{not json").expect("write");
        assert!(HeardList::open(Some(path)).stations().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
