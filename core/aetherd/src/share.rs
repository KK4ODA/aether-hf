//! The files another operator needs to see what happened: one zip, for an email.
//!
//! A failed contact has two sides, and the side that was not heard is the one that cannot be
//! read from here. Asking for the other station's files meant a PowerShell line pasted into an
//! email (KE4QCM, 2026-10-05). The panel now writes the same zip: the daemon's logs (this run,
//! the run before, and the dated runs the desktop shell keeps), the session history, the
//! stations heard, the diagnostic bundle (the settings without their secrets), and the
//! recordings' sidecars from the chosen period — with one station, or with anybody. The audio
//! is left out unless asked for: a sidecar lists every frame with its SNR and is a few kB, a
//! recording is megabytes.
//!
//! Nothing is sent from here: the zip lands under `shared/` beside the configuration and the
//! panel opens an email for the operator to attach it to. A log never holds what was said —
//! the station logs how much, never what — and neither does a sidecar.
//!
//! The programs using the modem keep logs of their own, and a fault between a program and the
//! modem shows in both (2026-11-12: a `RadioPost` client's connection ended twice in mid-session,
//! and what the client saw was in a file on the other side of the room from anyone asking). A
//! program offers its log once it connects (`share.include`, [`ProgramLogs`]); the zips carry it
//! under `programs/`, with the copy before it, whenever it was written in the period. The modem
//! remembers the offer after the program has gone — that is when its log is wanted — and takes
//! only a file named `*.log`, at most [`PROGRAM_LOG_MAX_BYTES`]: a program's own word that the
//! file is a log, kept by the same rule as the modem's.
//!
//! The zip is written by hand (stored or deflated entries, a central directory, no ZIP64):
//! a few dozen lines against the format's published description (PKWARE APPNOTE 6.3.x §4),
//! with `flate2` — already the session compression's — for the deflate and the CRC.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The zips kept under `shared/`; older ones go when a new one is written.
pub const KEEP: usize = 5;

/// The programs' logs remembered, newest offer first: a station runs a few programs.
pub const PROGRAM_LOGS: usize = 8;
/// A log not offered again for this long is forgotten: its program is no longer used here.
pub const PROGRAM_LOG_FORGET_MS: u64 = 30 * 24 * 3_600_000;
/// A program's log larger than this is left out of a zip, and the README says so: `RadioPost`
/// sets its log aside at 2 MB, and a zip goes by email or a phone's upload.
pub const PROGRAM_LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// What to gather.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Sessions that began at or after this time, milliseconds since the Unix epoch; logs
    /// written to since then.
    pub since_ms: u64,
    /// Only the sessions with this station (its base callsign: `KK4ODA-1` finds `KK4ODA`'s
    /// too), or with anybody.
    pub remote: Option<String>,
    /// The recordings' audio as well as their sidecars.
    pub audio: bool,
    /// Only these recordings, by file stem — debug mode sends the sessions that ended, not
    /// the period's (ADR-0050). `None` takes every recording of the period.
    pub recordings: Option<Vec<String>>,
}

/// Where the files are on this machine.
#[derive(Debug, Clone)]
pub struct Places {
    /// The configuration file: the logs, the history and `shared/` are beside it.
    pub config: PathBuf,
    /// Where recordings go.
    pub recordings: Option<PathBuf>,
    /// The daemon's own log file, when `[log] file` names one (a gateway).
    pub log_file: Option<PathBuf>,
    /// The logs the programs using the modem offered ([`ProgramLogs`]).
    pub program_logs: Vec<ProgramLog>,
}

/// One file in the zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A file on disk, read when the zip is written.
    File(PathBuf),
    /// Bytes made here — the diagnostic bundle, the summary.
    Bytes(Vec<u8>),
}

/// What was gathered, before it is written.
#[derive(Debug, Clone, Default)]
pub struct Gathered {
    /// Name in the zip, and the entry.
    pub entries: Vec<(String, Entry)>,
    /// Recordings' sidecars that matched.
    pub sessions: usize,
    /// The programs' logs left out for their size, by path, for the README.
    pub too_large: Vec<PathBuf>,
}

impl Gathered {
    /// A line for the README: which programs' logs are in the zip, and which were too large.
    #[must_use]
    pub fn programs_line(&self) -> String {
        let names: Vec<&str> = self
            .entries
            .iter()
            .filter_map(|(n, _)| n.strip_prefix("programs/"))
            .collect();
        let mut line = if names.is_empty() {
            "Programs' logs: none written in the period".to_owned()
        } else {
            format!("Programs' logs: {}", names.join(", "))
        };
        for path in &self.too_large {
            let _ = write!(
                line,
                "; {} left out, over {} MB",
                path.display(),
                PROGRAM_LOG_MAX_BYTES / (1024 * 1024)
            );
        }
        line
    }
}

/// Gather the files for `request`. A file that is not there is left out; nothing here fails.
#[must_use]
pub fn gather(places: &Places, request: &Request) -> Gathered {
    let mut out = Gathered::default();
    let dir = places
        .config
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    // the logs: this run and the one before always — the fault is often "before I restarted"
    // — and the dated runs the shell keeps when they were written to in the period
    for name in ["aetherd.log", "aetherd.prev.log"] {
        let path = dir.join(name);
        if path.is_file() {
            out.entries.push((name.to_owned(), Entry::File(path)));
        }
    }
    if let Some(file) = &places.log_file {
        let file = if file.is_relative() {
            dir.join(file)
        } else {
            file.clone()
        };
        if file.is_file()
            && !out
                .entries
                .iter()
                .any(|(_, e)| *e == Entry::File(file.clone()))
        {
            let name = file.file_name().map_or_else(
                || "daemon.log".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            out.entries.push((format!("log/{name}"), Entry::File(file)));
        }
    }
    for path in files_in(&dir.join("logs")) {
        if written_since(&path, request.since_ms) {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.entries
                .push((format!("logs/{name}"), Entry::File(path)));
        }
    }
    for name in ["sessions.json", "heard.json"] {
        let path = dir.join(name);
        if path.is_file() {
            out.entries.push((name.to_owned(), Entry::File(path)));
        }
    }
    add_program_logs(&mut out, places, request);
    // the recordings of the period, with the station asked about when one is
    if let Some(recordings) = &places.recordings {
        let recordings = if recordings.is_relative() {
            dir.join(recordings)
        } else {
            recordings.clone()
        };
        let mut sidecars: Vec<PathBuf> = files_in(&recordings)
            .into_iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            })
            .filter(|p| began_since(p, request.since_ms))
            .filter(|p| with_station(p, request.remote.as_deref()))
            .filter(|p| named(p, request.recordings.as_deref()))
            .collect();
        sidecars.sort();
        for sidecar in sidecars {
            let name = sidecar
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.entries
                .push((format!("recordings/{name}"), Entry::File(sidecar.clone())));
            out.sessions += 1;
            if request.audio {
                let wav = sidecar.with_extension("wav");
                if wav.is_file() {
                    let name = wav
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    out.entries
                        .push((format!("recordings/{name}"), Entry::File(wav)));
                }
            }
        }
    }
    out
}

/// The programs' logs, and the copy each set aside before it, written in the period: under
/// `programs/`, a name taken twice numbered, a file over [`PROGRAM_LOG_MAX_BYTES`] noted
/// instead.
fn add_program_logs(out: &mut Gathered, places: &Places, request: &Request) {
    for log in &places.program_logs {
        let mut older = log.path.clone().into_os_string();
        older.push(".old");
        for path in [log.path.clone(), PathBuf::from(older)] {
            if !path.is_file() || !written_since(&path, request.since_ms) {
                continue;
            }
            if std::fs::metadata(&path).map_or(true, |m| m.len() > PROGRAM_LOG_MAX_BYTES) {
                out.too_large.push(path);
                continue;
            }
            let file = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut name = format!("programs/{file}");
            let mut n = 2;
            while out.entries.iter().any(|(taken, _)| *taken == name) {
                name = format!("programs/{n}-{file}");
                n += 1;
            }
            out.entries.push((name, Entry::File(path)));
        }
    }
}

/// A log a program using the modem offered for the zips (`share.include`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramLog {
    /// The program, as it named itself when it offered the log.
    pub app: String,
    /// The file: an absolute path whose name ends `.log`.
    pub path: PathBuf,
    /// When it was last offered, milliseconds since the Unix epoch.
    pub offered_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct ProgramLogsFile {
    schema: u32,
    logs: Vec<ProgramLog>,
}

/// The programs' logs the modem remembers, kept in a file beside the configuration: a program
/// offers its log while it runs, and the zip that needs it is made after it has gone.
#[derive(Debug)]
pub struct ProgramLogs {
    entries: Vec<ProgramLog>,
    path: Option<PathBuf>,
}

impl ProgramLogs {
    /// The list kept in `path`, loaded from it if it exists; empty when it does not or cannot
    /// be read, or with no file at all.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<ProgramLogsFile>(&text).ok())
            .filter(|file| file.schema == 1)
            .map(|file| file.logs)
            .unwrap_or_default()
            .into_iter()
            .filter(|log| offerable(&log.path).is_ok())
            .collect();
        Self { entries, path }
    }

    /// The logs remembered, the newest offer first.
    #[must_use]
    pub fn entries(&self) -> &[ProgramLog] {
        &self.entries
    }

    /// The logs not forgotten by `now_ms`: what a zip made now takes.
    #[must_use]
    pub fn current(&self, now_ms: u64) -> Vec<ProgramLog> {
        self.entries
            .iter()
            .filter(|log| now_ms.saturating_sub(log.offered_ms) <= PROGRAM_LOG_FORGET_MS)
            .cloned()
            .collect()
    }

    /// Remember `path` as `app`'s log, and write the list. Whether it is news: a log not
    /// remembered before, or remembered under another program's name — a program offers its
    /// log at every connection, and only news is worth a line in the modem's log.
    ///
    /// # Errors
    /// For a path that is not absolute or whose name does not end `.log`, or when the list
    /// cannot be written.
    pub fn offer(&mut self, app: &str, path: &Path, now_ms: u64) -> Result<bool, String> {
        offerable(path)?;
        let app: String = app.trim().chars().take(80).collect();
        let app = if app.is_empty() {
            "a program".to_owned()
        } else {
            app
        };
        let before = self.entries.iter().position(|log| log.path == path);
        let news = before.is_none_or(|i| self.entries[i].app != app);
        if let Some(i) = before {
            self.entries.remove(i);
        }
        self.entries.insert(
            0,
            ProgramLog {
                app,
                path: path.to_path_buf(),
                offered_ms: now_ms,
            },
        );
        self.entries
            .retain(|log| now_ms.saturating_sub(log.offered_ms) <= PROGRAM_LOG_FORGET_MS);
        self.entries.truncate(PROGRAM_LOGS);
        self.save()
            .map_err(|e| format!("cannot remember the log: {e}"))?;
        Ok(news)
    }

    /// Write the list, whole, to a temporary file renamed into place.
    fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = ProgramLogsFile {
            schema: 1,
            logs: self.entries.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)
    }
}

/// Whether a program may offer `path` as its log: an absolute path, with no `..` in it, to a
/// file named `*.log`. The modem does not read it until a zip is made, so it need not exist
/// yet: a program names its log before it has written to it.
///
/// # Errors
/// What is wrong with it, in a sentence for the program's author.
pub fn offerable(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("a log is named by its absolute path, without `..`".to_owned());
    }
    let named_log = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("log"))
        && path.file_stem().is_some_and(|s| !s.is_empty());
    if !named_log {
        return Err("only a file whose name ends .log is taken as a program's log".to_owned());
    }
    Ok(())
}

/// Write `entries` as a zip at `path`, through a temporary file so a reader never sees half
/// of one. Text is deflated; audio, which deflate barely shrinks, is stored.
///
/// # Errors
/// If a file cannot be read or the zip cannot be written.
pub fn write_zip(
    path: &Path,
    entries: &[(String, Entry)],
    modified_ms: u64,
) -> std::io::Result<u64> {
    let temporary = path.with_extension("zip.part");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&temporary)?);
    let (date, time) = dos_time(modified_ms);
    let mut central = Vec::new();
    let mut offset: u64 = 0;
    for (name, entry) in entries {
        let data = match entry {
            Entry::File(file) => std::fs::read(file)?,
            Entry::Bytes(bytes) => bytes.clone(),
        };
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        let stored = Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
        let body = if stored {
            data.clone()
        } else {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&data)?;
            encoder.finish()?
        };
        let method: u16 = if stored { 0 } else { 8 };
        let too_big = |n: usize| u32::try_from(n).map_err(|_| std::io::Error::other("over 4 GB"));
        let (packed, plain) = (too_big(body.len())?, too_big(data.len())?);
        let here = u32::try_from(offset).map_err(|_| std::io::Error::other("over 4 GB"))?;
        let name_bytes = name.as_bytes();
        let name_len =
            u16::try_from(name_bytes.len()).map_err(|_| std::io::Error::other("name"))?;
        // a local header (APPNOTE §4.3.7): version 2.0, the UTF-8 name flag (bit 11)
        let mut local = Vec::with_capacity(30 + name_bytes.len());
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&0x0800u16.to_le_bytes());
        local.extend_from_slice(&method.to_le_bytes());
        local.extend_from_slice(&time.to_le_bytes());
        local.extend_from_slice(&date.to_le_bytes());
        local.extend_from_slice(&crc.sum().to_le_bytes());
        local.extend_from_slice(&packed.to_le_bytes());
        local.extend_from_slice(&plain.to_le_bytes());
        local.extend_from_slice(&name_len.to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name_bytes);
        out.write_all(&local)?;
        out.write_all(&body)?;
        // its central directory record (§4.3.12)
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.sum().to_le_bytes());
        central.extend_from_slice(&packed.to_le_bytes());
        central.extend_from_slice(&plain.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&here.to_le_bytes());
        central.extend_from_slice(name_bytes);
        offset += (local.len() + body.len()) as u64;
    }
    let count = u16::try_from(entries.len()).map_err(|_| std::io::Error::other("too many"))?;
    let at = u32::try_from(offset).map_err(|_| std::io::Error::other("over 4 GB"))?;
    let size = u32::try_from(central.len()).map_err(|_| std::io::Error::other("over 4 GB"))?;
    out.write_all(&central)?;
    // the end of the central directory (§4.3.16)
    let mut end = Vec::with_capacity(22);
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0; 4]); // this disk, the directory's disk
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&size.to_le_bytes());
    end.extend_from_slice(&at.to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&end)?;
    out.flush()?;
    drop(out);
    std::fs::rename(&temporary, path)?;
    Ok(offset + central.len() as u64 + end.len() as u64)
}

/// Where the zips go: `shared/` beside the configuration.
#[must_use]
pub fn shared_dir(config: &Path) -> PathBuf {
    config
        .parent()
        .map_or_else(|| PathBuf::from("shared"), |dir| dir.join("shared"))
}

/// Keep the newest `keep` zips in `dir`, by the time stamp in each name. The names start with
/// the callsign (`aether-KK4ODA-2-20261102-153518.zip`), so sorting the names whole put a zip
/// just written under a new callsign among the oldest, and it was deleted at once: a station
/// whose folder held five zips of another callsign could send nothing ("That zip is not in the
/// shared folder"). A name with no stamp counts as the oldest.
pub fn prune(dir: &Path, keep: usize) {
    let mut zips: Vec<(u64, PathBuf)> = files_in(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .map(|p| {
            let when = p
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(stamp_in)
                .unwrap_or(0);
            (when, p)
        })
        .collect();
    zips.sort();
    let excess = zips.len().saturating_sub(keep);
    for (_, old) in &zips[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

/// The last `YYYYMMDD-HHMMSS` stamp in a file name, as Unix milliseconds.
fn stamp_in(name: &str) -> Option<u64> {
    (0..name.len().saturating_sub(14))
        .rev()
        .filter(|&i| name.is_char_boundary(i) && name.is_char_boundary(i + 15))
        .find_map(|i| stamp_ms(&name[i..i + 15]))
}

/// `20261005-004737`, in UTC, the way recordings are named.
#[must_use]
pub fn stamp(ms: u64) -> String {
    let (y, mo, d, h, mi, s) = civil(ms);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        },
    )
}

fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    u64::try_from(since.as_millis()).ok()
}

fn written_since(path: &Path, since_ms: u64) -> bool {
    modified_ms(path).is_some_and(|ms| ms >= since_ms)
}

/// Whether a recording began in the period: by the UTC stamp its name starts with, or, for a
/// name without one, by when it was last written.
fn began_since(path: &Path, since_ms: u64) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    match name.get(..15).and_then(stamp_ms) {
        Some(began) => began >= since_ms,
        None => written_since(path, since_ms),
    }
}

/// Whether a sidecar is one of `stems`, or any when `None`.
fn named(sidecar: &Path, stems: Option<&[String]>) -> bool {
    let Some(stems) = stems else {
        return true;
    };
    let stem = sidecar.file_stem().unwrap_or_default().to_string_lossy();
    stems.iter().any(|s| *s == stem)
}

/// Whether a sidecar is of a session with `remote` (by base callsign), or any when `None`.
///
/// The other station is `session.remote`, or for a Test `session.test.remote` — a Test's
/// sidecar is written when the station is idle again, and its `session.remote` is null, so
/// every Test was left out of a request for the files of the sessions with a station: WC4Y's
/// two uploads of 2026-10-10 carried no recording at all. A sidecar that names nobody is
/// matched by its file name, `<start>_<this station>_<other station>[_test]`.
fn with_station(sidecar: &Path, remote: Option<&str>) -> bool {
    let Some(remote) = remote else {
        return true;
    };
    let wanted = base(remote);
    let doc = std::fs::read_to_string(sidecar)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let named = doc.as_ref().and_then(|doc| {
        doc["session"]["remote"]
            .as_str()
            .or_else(|| doc["session"]["test"]["remote"].as_str())
            .map(base)
    });
    let theirs = named.or_else(|| {
        let stem = sidecar.file_stem()?.to_string_lossy().into_owned();
        stem.split('_').nth(2).map(base)
    });
    theirs.is_some_and(|theirs| theirs == wanted)
}

fn base(call: &str) -> String {
    crate::station::base_callsign(call).to_ascii_uppercase()
}

/// `20261005-004737` → milliseconds since the Unix epoch.
fn stamp_ms(stamp: &str) -> Option<u64> {
    let bytes = stamp.as_bytes();
    if bytes.len() != 15
        || bytes[8] != b'-'
        || !stamp
            .chars()
            .enumerate()
            .all(|(i, c)| i == 8 || c.is_ascii_digit())
    {
        return None;
    }
    let field = |r: std::ops::Range<usize>| stamp[r].parse::<i64>().ok();
    let (year, month, day) = (field(0..4)?, field(4..6)?, field(6..8)?);
    let (hour, minute, second) = (field(9..11)?, field(11..13)?, field(13..15)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(((days * 24 + hour) * 60 + minute) * 60 + second)
        .ok()
        .map(|secs| secs * 1000)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day, hour, minute, second) in UTC (Hinnant's `civil_from_days`).
fn civil(ms: u64) -> (i64, i64, i64, i64, i64, i64) {
    let secs = i64::try_from(ms / 1000).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// The MS-DOS date and time a zip entry carries (APPNOTE §4.4.6), in UTC.
fn dos_time(ms: u64) -> (u16, u16) {
    let (y, mo, d, h, mi, s) = civil(ms);
    let year = u16::try_from((y - 1980).clamp(0, 127)).unwrap_or(0);
    let pack = |v: i64| u16::try_from(v).unwrap_or(0);
    (
        (year << 9) | (pack(mo) << 5) | pack(d),
        (pack(h) << 11) | (pack(mi) << 5) | pack(s / 2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-share-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// Read a zip the way an unzipper does — from the central directory — and return each
    /// entry's name and contents, checking every CRC.
    fn unzip(path: &Path) -> Vec<(String, Vec<u8>)> {
        let data = std::fs::read(path).expect("zip");
        let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().expect("4"));
        let end = data.len() - 22;
        assert_eq!(u32_at(end), 0x0605_4b50, "the end record");
        let count = usize::from(u16_at(end + 10));
        let mut at = u32_at(end + 16) as usize;
        let mut out = Vec::new();
        for _ in 0..count {
            assert_eq!(u32_at(at), 0x0201_4b50, "a central record");
            let method = u16_at(at + 10);
            let crc = u32_at(at + 16);
            let packed = u32_at(at + 20) as usize;
            let name_len = usize::from(u16_at(at + 28));
            let local = u32_at(at + 42) as usize;
            let name = String::from_utf8(data[at + 46..at + 46 + name_len].to_vec()).expect("utf8");
            assert_eq!(u32_at(local), 0x0403_4b50, "a local header");
            let start =
                local + 30 + usize::from(u16_at(local + 26)) + usize::from(u16_at(local + 28));
            let body = &data[start..start + packed];
            let plain = if method == 8 {
                let mut v = Vec::new();
                flate2::read::DeflateDecoder::new(body)
                    .read_to_end(&mut v)
                    .expect("inflate");
                v
            } else {
                body.to_vec()
            };
            let mut check = flate2::Crc::new();
            check.update(&plain);
            assert_eq!(check.sum(), crc, "{name}");
            out.push((name, plain));
            at += 46 + name_len + usize::from(u16_at(at + 30)) + usize::from(u16_at(at + 32));
        }
        out
    }

    #[test]
    fn a_zip_reads_back_whole() {
        let dir = scratch("zip");
        let file = dir.join("aetherd.log");
        std::fs::write(
            &file,
            "2026-10-05T00:16:22Z info  connected: KE4QCM (slave)\n".repeat(50),
        )
        .expect("write");
        let wav = dir.join("a.wav");
        std::fs::write(&wav, [1u8, 2, 3, 4]).expect("write");
        let zip = dir.join("out.zip");
        let entries = vec![
            ("aetherd.log".to_owned(), Entry::File(file.clone())),
            ("recordings/a.wav".to_owned(), Entry::File(wav)),
            ("summary.txt".to_owned(), Entry::Bytes(b"KK4ODA-1".to_vec())),
        ];
        let bytes = write_zip(&zip, &entries, 1_791_161_257_000).expect("written");
        assert_eq!(bytes, std::fs::metadata(&zip).expect("zip").len());
        assert!(!zip.with_extension("zip.part").exists());
        let back = unzip(&zip);
        assert_eq!(back.len(), 3);
        assert_eq!(back[0].0, "aetherd.log");
        assert_eq!(back[0].1, std::fs::read(&file).expect("log"));
        assert_eq!(back[1], ("recordings/a.wav".to_owned(), vec![1, 2, 3, 4]));
        assert_eq!(back[2].1, b"KK4ODA-1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_period_and_the_station_choose_the_recordings() {
        let dir = scratch("gather");
        let config = dir.join("station.toml");
        std::fs::write(&config, "").expect("write");
        std::fs::write(dir.join("aetherd.log"), "run").expect("write");
        std::fs::write(dir.join("sessions.json"), "{}").expect("write");
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).expect("dir");
        let sidecar = |name: &str, remote: &str| {
            std::fs::write(
                recordings.join(format!("{name}.json")),
                format!(r#"{{"session":{{"remote":"{remote}"}}}}"#),
            )
            .expect("write");
            std::fs::write(recordings.join(format!("{name}.wav")), "RIFF").expect("write");
        };
        sidecar("20261005-001622_KK4ODA-1_KE4QCM", "KE4QCM");
        sidecar("20261005-004737_KK4ODA-1_KE4QCM-1", "KE4QCM-1");
        sidecar("20261005-003000_KK4ODA-1_ND1J", "ND1J");
        sidecar("20261004-120000_KK4ODA-1_KE4QCM", "KE4QCM");
        // a Test's sidecar names the other station only in its report (WC4Y's, 2026-10-10)
        std::fs::write(
            recordings.join("20261005-002500_KK4ODA-1_KE4QCM_test.json"),
            r#"{"session":{"remote":null,"test":{"remote":"KE4QCM"}}}"#,
        )
        .expect("write");
        let places = Places {
            config,
            recordings: Some(recordings),
            log_file: None,
            program_logs: Vec::new(),
        };
        let since = stamp_ms("20261005-000000").expect("stamp");
        let names = |g: &Gathered| g.entries.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
        // KE4QCM's sessions of the night, under either of his names, and no audio
        let got = gather(
            &places,
            &Request {
                since_ms: since,
                remote: Some("ke4qcm".into()),
                audio: false,
                recordings: None,
            },
        );
        assert_eq!(got.sessions, 3);
        assert_eq!(
            names(&got),
            [
                "aetherd.log",
                "sessions.json",
                "recordings/20261005-001622_KK4ODA-1_KE4QCM.json",
                "recordings/20261005-002500_KK4ODA-1_KE4QCM_test.json",
                "recordings/20261005-004737_KK4ODA-1_KE4QCM-1.json",
            ]
        );
        // anybody's, with the audio
        let got = gather(
            &places,
            &Request {
                since_ms: since,
                remote: None,
                audio: true,
                recordings: None,
            },
        );
        assert_eq!(got.sessions, 4, "the Test's sidecar too");
        assert_eq!(
            got.entries
                .iter()
                .filter(|(n, _)| Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("wav")))
                .count(),
            3
        );
        // debug mode's: the one session that ended, its audio with it, whoever it was with
        let got = gather(
            &places,
            &Request {
                since_ms: since,
                remote: None,
                audio: true,
                recordings: Some(vec!["20261005-004737_KK4ODA-1_KE4QCM-1".into()]),
            },
        );
        assert_eq!(got.sessions, 1);
        assert!(
            names(&got)
                .iter()
                .filter(|n| n.starts_with("recordings/"))
                .eq([
                    "recordings/20261005-004737_KK4ODA-1_KE4QCM-1.json",
                    "recordings/20261005-004737_KK4ODA-1_KE4QCM-1.wav"
                ]
                .iter()),
            "{:?}",
            names(&got)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The mobile's `RadioPost` client lost its connection to the modem twice on 2026-11-12, and
    /// its log was on the laptop, not in the zip. A program offers its log; the zip carries it,
    /// and the one set aside before it, while they were written in the period — after the
    /// program has gone too, which is when they are wanted.
    #[test]
    fn a_programs_log_goes_in_the_zip_after_the_program_has_gone() {
        let dir = scratch("programs");
        let config = dir.join("station.toml");
        std::fs::write(&config, "").expect("write");
        let home = dir.join("home").join(".radiopost");
        std::fs::create_dir_all(&home).expect("dir");
        let log = home.join("radiopost.log");
        std::fs::write(&log, "2026-11-12T00:55:54Z the connection to Aether ended").expect("write");
        std::fs::write(home.join("radiopost.log.old"), "before").expect("write");
        let gateway = dir.join(".radiopost-gateway").join("radiopost.log");
        std::fs::create_dir_all(gateway.parent().expect("dir")).expect("dir");
        std::fs::write(&gateway, "another program, the same name").expect("write");
        let now = modified_ms(&log).expect("written");

        let kept = dir.join("program-logs.json");
        let mut logs = ProgramLogs::open(Some(kept.clone()));
        assert_eq!(
            logs.offer("RadioPost client 0.1.0-beta.8", &log, now),
            Ok(true)
        );
        assert_eq!(
            logs.offer("RadioPost client 0.1.0-beta.8", &log, now),
            Ok(false),
            "offered again at the next connection: no news"
        );
        assert_eq!(logs.offer("RadioPost Gateway", &gateway, now), Ok(true));
        // what is not a log is refused, and nothing is remembered of it
        for refused in [
            dir.join("station.toml"),
            dir.join("home").join(".ssh").join("id_ed25519"),
            PathBuf::from("radiopost.log"),
            home.join("..").join("..").join("station.log"),
            home.join(".log"),
        ] {
            assert!(
                logs.offer("RadioPost client", &refused, now).is_err(),
                "{} was taken",
                refused.display()
            );
        }
        // the program has gone, and the modem restarted: the offer is still there
        let logs = ProgramLogs::open(Some(kept));
        assert_eq!(logs.entries().len(), 2);
        assert_eq!(logs.entries()[0].app, "RadioPost Gateway");

        let places = Places {
            config,
            recordings: None,
            log_file: None,
            program_logs: logs.current(now),
        };
        let request = |since_ms| Request {
            since_ms,
            remote: None,
            audio: false,
            recordings: None,
        };
        let got = gather(&places, &request(now.saturating_sub(3_600_000)));
        let names: Vec<&str> = got.entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "programs/radiopost.log",
                "programs/2-radiopost.log",
                "programs/radiopost.log.old",
            ],
            "the gateway's first, then the client's under another name, and the one before it"
        );
        assert_eq!(
            got.programs_line(),
            "Programs' logs: radiopost.log, 2-radiopost.log, radiopost.log.old"
        );
        // not written in the period: left out
        let got = gather(&places, &request(now + 60_000));
        assert!(got.entries.is_empty(), "{:?}", got.entries);
        assert_eq!(
            got.programs_line(),
            "Programs' logs: none written in the period"
        );
        // forgotten a month after the last offer
        assert_eq!(logs.current(now + PROGRAM_LOG_FORGET_MS + 1), Vec::new());
        // a log grown past the limit stays out, and the README says so
        std::fs::write(
            &log,
            vec![b'x'; usize::try_from(PROGRAM_LOG_MAX_BYTES).expect("size") + 1],
        )
        .expect("write");
        let got = gather(&places, &request(0));
        assert_eq!(got.too_large, std::slice::from_ref(&log));
        assert!(
            got.programs_line().contains("left out, over 8 MB"),
            "{}",
            got.programs_line()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The laptop's folder (2026-11-02): five zips sent as KK4ODA, then the callsign changed. The
    /// zip just written, under KK2ODA or KK4ODA-2, sorted first by name and was pruned at once.
    #[test]
    fn the_newest_zips_stay_whatever_their_callsign() {
        let dir = scratch("prune");
        let older = [
            "aether-KK4ODA-20261025-034400.zip",
            "aether-KK4ODA-20261025-034427-debug.zip",
            "aether-KK4ODA-20261025-051435.zip",
            "aether-KK4ODA-20261026-120000.zip",
            "aether-KK4ODA-20261027-120000.zip",
        ];
        for name in older {
            std::fs::write(dir.join(name), b"zip").expect("write");
        }
        std::fs::write(dir.join("notes.zip"), b"zip").expect("write");
        std::fs::write(dir.join("README.txt"), b"text").expect("write");
        for newer in [
            "aether-KK2ODA-20261102-153518.zip",
            "aether-KK4ODA-2-20261102-153650.zip",
        ] {
            std::fs::write(dir.join(newer), b"zip").expect("write");
            prune(&dir, KEEP);
            assert!(
                dir.join(newer).is_file(),
                "{newer} was pruned as it was written"
            );
        }
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "README.txt",
                "aether-KK2ODA-20261102-153518.zip",
                "aether-KK4ODA-2-20261102-153650.zip",
                "aether-KK4ODA-20261025-051435.zip",
                "aether-KK4ODA-20261026-120000.zip",
                "aether-KK4ODA-20261027-120000.zip",
            ],
            "the five newest zips by their stamps, and nothing that is not a zip"
        );
        assert_eq!(
            stamp_in("aether-KK4ODA-2-20261102-153650.zip"),
            stamp_ms("20261102-153650")
        );
        assert_eq!(
            stamp_in("aether-KK4ODA-20261025-034427-debug.zip"),
            stamp_ms("20261025-034427")
        );
        assert_eq!(stamp_in("notes.zip"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_clock_reads_both_ways() {
        let ms = stamp_ms("20261005-004737").expect("stamp");
        assert_eq!(ms, 1_791_161_257_000);
        assert_eq!(stamp(ms), "20261005-004737");
        assert_eq!(stamp(0), "19700101-000000");
        assert_eq!(stamp_ms("20261305-004737"), None);
        assert_eq!(stamp_ms("aetherd.log"), None);
        // 2026-10-05 00:47:36, as MS-DOS time: two-second resolution
        assert_eq!(
            dos_time(ms),
            ((46 << 9) | (10 << 5) | 5, (47 << 5) | (37 / 2))
        );
    }
}
