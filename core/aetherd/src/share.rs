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
//! The zip is written by hand (stored or deflated entries, a central directory, no ZIP64):
//! a few dozen lines against the format's published description (PKWARE APPNOTE 6.3.x §4),
//! with `flate2` — already the session compression's — for the deflate and the CRC.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The zips kept under `shared/`; older ones go when a new one is written.
pub const KEEP: usize = 5;

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

/// Keep the newest `keep` zips in `dir`: their names sort by time.
pub fn prune(dir: &Path, keep: usize) {
    let mut zips: Vec<PathBuf> = files_in(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    zips.sort();
    let excess = zips.len().saturating_sub(keep);
    for old in &zips[..excess] {
        let _ = std::fs::remove_file(old);
    }
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

/// Whether a sidecar is of a session with `remote` (by base callsign), or any when `None`.
fn with_station(sidecar: &Path, remote: Option<&str>) -> bool {
    let Some(remote) = remote else {
        return true;
    };
    let wanted = base(remote);
    std::fs::read_to_string(sidecar)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|doc| doc["session"]["remote"].as_str().map(base))
        .is_some_and(|theirs| theirs == wanted)
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
            "2026-10-05T00:16:22Z info  connected: KE4QCM (irs)\n".repeat(50),
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
        let places = Places {
            config,
            recordings: Some(recordings),
            log_file: None,
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
            },
        );
        assert_eq!(got.sessions, 2);
        assert_eq!(
            names(&got),
            [
                "aetherd.log",
                "sessions.json",
                "recordings/20261005-001622_KK4ODA-1_KE4QCM.json",
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
            },
        );
        assert_eq!(got.sessions, 3);
        assert_eq!(
            got.entries
                .iter()
                .filter(|(n, _)| Path::new(n)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("wav")))
                .count(),
            3
        );
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
