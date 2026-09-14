//! The daemon's log: a line for a person, a record for a machine, and the last few hundred
//! entries kept for a diagnostic bundle.
//!
//! A bug report from an HF station is mostly a story — "it keyed, it heard the other side,
//! then it just sat there" — and the log is how that story gets told without the operator
//! having to remember it. So every entry carries a UTC timestamp, a name a machine can group
//! on, the detail a person reads, and the modem's state at the time; and the most recent
//! entries ride along in the `diagnostics` bundle, so a report is useful even when nobody
//! thought to capture a terminal. Lines can be plain text or JSON, one object per line, for
//! a journal or a log shipper.
//!
//! No logging crate: the daemon has one thread that logs, one place it logs from, and no
//! need for filtering by module. What it needs is a timestamp and a ring, which is less code
//! than a framework's configuration.

use std::{
    collections::VecDeque,
    fmt,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

/// How much a log entry matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Something happened.
    Info,
    /// Something happened that the operator should know about but that the station survived.
    Warn,
    /// Something failed.
    Error,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad`, not `write_str`, so a width in the format string lines the columns up
        f.pad(match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        })
    }
}

/// How lines are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// `2026-09-14T12:34:56.789Z info  connected: W4XYZ (connected)` — for a terminal.
    #[default]
    Text,
    /// One JSON object per line — for a journal, a log shipper, or `jq`.
    Json,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// When, in UTC, RFC 3339 with milliseconds.
    pub ts: String,
    /// How much it matters.
    pub level: Level,
    /// What kind of thing: `connected`, `watchdog`, `audio`, `control`, …
    pub event: String,
    /// The particulars, for a person.
    pub detail: String,
    /// The modem's state at the time.
    pub state: String,
}

impl Entry {
    fn text(&self) -> String {
        format!(
            "{} {:<5} {}: {} ({})",
            self.ts, self.level, self.event, self.detail, self.state
        )
    }
}

/// Where the log goes, and what it remembers.
pub struct Log {
    format: Format,
    keep: usize,
    recent: VecDeque<Entry>,
    sinks: Vec<Box<dyn Write>>,
    /// How many entries were dropped from the ring, so a bundle can say it is incomplete.
    forgotten: u64,
}

impl fmt::Debug for Log {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Log")
            .field("format", &self.format)
            .field("keep", &self.keep)
            .field("recent", &self.recent.len())
            .field("sinks", &self.sinks.len())
            .field("forgotten", &self.forgotten)
            .finish()
    }
}

impl Log {
    /// A log that writes nowhere and only remembers — for tests, and for a daemon whose
    /// output nobody is reading.
    #[must_use]
    pub fn memory(keep: usize) -> Self {
        Self {
            format: Format::Text,
            keep,
            recent: VecDeque::new(),
            sinks: Vec::new(),
            forgotten: 0,
        }
    }

    /// A log that also writes to standard output.
    #[must_use]
    pub fn to_stdout(format: Format, keep: usize) -> Self {
        let mut log = Self::memory(keep);
        log.format = format;
        log.sinks.push(Box::new(std::io::stdout()));
        log
    }

    /// Also append to a file. A desktop shell has no terminal, so without this nothing a
    /// packaged daemon says survives the session.
    ///
    /// # Errors
    /// If the file cannot be opened for appending.
    pub fn also_to_file(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.sinks.push(Box::new(file));
        Ok(())
    }

    /// Record one entry, write it out, and remember it.
    pub fn record(&mut self, level: Level, event: &str, detail: &str, state: &str) {
        let entry = Entry {
            ts: rfc3339(now_ms()),
            level,
            event: event.to_owned(),
            detail: detail.to_owned(),
            state: state.to_owned(),
        };
        let line = match self.format {
            Format::Text => entry.text(),
            // an entry is strings and an enum, which cannot fail to serialise
            Format::Json => serde_json::to_string(&entry).unwrap_or_default(),
        };
        for sink in &mut self.sinks {
            // A sink that fails is not a reason to stop the modem — a full disk, or a closed
            // pipe. The entry is still in the ring for a diagnostic bundle.
            let _ = writeln!(sink, "{line}");
            let _ = sink.flush();
        }
        self.recent.push_back(entry);
        while self.recent.len() > self.keep {
            self.recent.pop_front();
            self.forgotten += 1;
        }
    }

    /// The most recent entries, oldest first.
    #[must_use]
    pub fn recent(&self) -> &VecDeque<Entry> {
        &self.recent
    }

    /// How many entries have scrolled off the ring.
    #[must_use]
    pub fn forgotten(&self) -> u64 {
        self.forgotten
    }
}

/// Milliseconds since the Unix epoch, now.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// RFC 3339 in UTC with milliseconds, from milliseconds since the Unix epoch.
///
/// Written out rather than pulled in: a date crate is a large dependency for a daemon that
/// only ever needs "now, in UTC, as text". The civil-date arithmetic is the well-known
/// proleptic-Gregorian algorithm (Hinnant, "chrono-Compatible Low-Level Date Algorithms").
#[must_use]
pub fn rfc3339(ts_ms: u64) -> String {
    let seconds = ts_ms / 1000;
    let millis = ts_ms % 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60
    )
}

/// Year, month and day for a count of days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097); // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153; // March = 0
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    // both are in range by construction; the fallbacks are unreachable
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_rfc3339_utc_with_milliseconds() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        // a leap day, at the end of a leap year that the 100-year rule would have skipped
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
        assert_eq!(rfc3339(1_709_164_799_000), "2024-02-28T23:59:59.000Z");
        assert_eq!(rfc3339(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
        assert_eq!(rfc3339(4_102_444_800_000), "2100-01-01T00:00:00.000Z");
    }

    #[test]
    fn the_ring_keeps_the_newest_and_counts_what_it_forgot() {
        let mut log = Log::memory(3);
        for i in 0..5 {
            log.record(Level::Info, "tick", &i.to_string(), "Idle");
        }
        let kept: Vec<&str> = log.recent().iter().map(|e| e.detail.as_str()).collect();
        assert_eq!(kept, ["2", "3", "4"]);
        assert_eq!(log.forgotten(), 2);
    }

    #[test]
    fn json_lines_carry_every_field_and_text_lines_read_left_to_right() {
        let mut log = Log::memory(10);
        log.record(Level::Warn, "watchdog", "key time exceeded", "Connected");
        let entry = &log.recent()[0];
        let json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(entry).expect("serialise")).expect("parse");
        assert_eq!(json["level"], "warn");
        assert_eq!(json["event"], "watchdog");
        assert_eq!(json["detail"], "key time exceeded");
        assert_eq!(json["state"], "Connected");
        assert!(json["ts"].as_str().expect("ts").ends_with('Z'));
        let text = entry.text();
        assert!(
            text.contains("warn  watchdog: key time exceeded (Connected)"),
            "{text}"
        );
        assert!(text.starts_with("20"), "{text}");
    }

    #[test]
    fn a_file_sink_gets_every_line_appended() {
        let dir = std::env::temp_dir().join(format!("aetherd-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("aetherd.log");
        {
            let mut log = Log::memory(10);
            log.also_to_file(&path).expect("open");
            log.record(Level::Info, "start", "one", "Idle");
        }
        {
            let mut log = Log::memory(10);
            log.also_to_file(&path).expect("reopen");
            log.record(Level::Error, "stop", "two", "Idle");
        }
        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("start: one"));
        assert!(lines[1].contains("error stop: two"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
