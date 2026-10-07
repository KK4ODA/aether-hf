//! The daemon's logs from earlier runs.
//!
//! Each run writes `aetherd.log` from scratch, so it describes that run, and the run before is
//! kept as `aetherd.prev.log` — "I restarted it and it came good" is the commonest way a fault
//! is reported. One generation was not enough: ND1J's log from a Saturday night was asked for
//! on the Monday, and two restarts later only Monday's runs were left. So every run's log is
//! also kept under `logs/`, named for when the run began. Ten runs were kept at first, and
//! that was not enough either: every beta update restarts the daemon, and when WC4Y's
//! sessions of 2026-10-05 were asked about two days later the evening's log was gone. Now a
//! run's log stays for [`KEEP_DAYS`] days, with the newest [`KEEP_AT_LEAST`] kept whatever
//! their age and no more than [`KEEP_AT_MOST`] in all.

use std::path::{Path, PathBuf};

/// How many days an earlier run's log is kept in `logs/`.
pub const KEEP_DAYS: u64 = 30;
/// The newest runs kept whatever their age: a station started now and then keeps its last few.
pub const KEEP_AT_LEAST: usize = 10;
/// The most runs kept whatever their age, so a daemon restarting in a loop cannot fill the disk.
pub const KEEP_AT_MOST: usize = 500;

/// Set the last run's log aside before a new run writes `log` from scratch: a dated copy in
/// `logs/` beside it, and `aetherd.prev.log` as before. Dated copies older than [`KEEP_DAYS`]
/// go ([`prune`]). Every step is best effort — a log that cannot be kept is never a reason not
/// to start.
pub fn set_aside(log: &Path, now: std::time::SystemTime) {
    if let Ok(meta) = std::fs::metadata(log) {
        let dir = runs_dir(log);
        if std::fs::create_dir_all(&dir).is_ok() {
            // named for when the run began, as its own first line says. Not the file's creation
            // time: Windows carries a deleted file's creation time over to a new file of the same
            // name made soon after ("file system tunnelling"), and every copy of the author's
            // log was dated the day the first one was made. The last write, when the first line
            // says nothing — a run that died before it logged.
            let name = match first_stamp(log) {
                Some(began) => format!("aetherd-{began}.log"),
                None => format!("aetherd-{}.log", stamp(meta.modified().ok())),
            };
            let _ = std::fs::copy(log, unique(&dir.join(name)));
            prune(&dir, now);
        }
    }
    let _ = std::fs::rename(log, previous(log));
}

/// Where the dated logs are kept: `logs/` beside the current one.
#[must_use]
pub fn runs_dir(log: &Path) -> PathBuf {
    log.parent()
        .map_or_else(|| PathBuf::from("logs"), |dir| dir.join("logs"))
}

/// The run before this one, beside the current log.
#[must_use]
pub fn previous(log: &Path) -> PathBuf {
    log.with_extension("prev.log")
}

/// `20260927-011200`, in UTC, the way recording names are written; `unknown` when the system
/// gave no time.
fn stamp(at: Option<std::time::SystemTime>) -> String {
    let Some(at) = at else {
        return "unknown".to_owned();
    };
    let t = time::OffsetDateTime::from(at);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// The run's start, from its log's first line — `2026-10-04T21:32:27.810Z info  daemon: …` —
/// as `20261004-213227`; `None` when the line carries no such time.
fn first_stamp(log: &Path) -> Option<String> {
    use std::io::BufRead as _;
    let file = std::fs::File::open(log).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(file).read_line(&mut line).ok()?;
    let at = line.get(..19)?;
    let b = at.as_bytes();
    let shape = b.len() == 19
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b'T'
        && b[13] == b':'
        && b[16] == b':';
    let digits: String = at.chars().filter(char::is_ascii_digit).collect();
    (shape && digits.len() == 14).then(|| format!("{}-{}", &digits[..8], &digits[8..]))
}

/// `path`, or `path` with `-2`, `-3`… before the extension when it is taken: two runs begun in
/// the same second keep both logs.
fn unique(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aetherd");
    // a thousand runs begun in one second is not a thing that happens
    (2..1000)
        .map(|n| path.with_file_name(format!("{stem}-{n}.log")))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

/// Remove the dated logs that began more than [`KEEP_DAYS`] before `now`, keeping the newest
/// [`KEEP_AT_LEAST`] whatever their age, and the oldest beyond [`KEEP_AT_MOST`] whatever theirs.
/// A run's age is its name's start time — the names sort by time — or, for a log named
/// `unknown`, its last write.
fn prune(dir: &Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut runs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let dated = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("aetherd-"));
            let log = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("log"));
            dated && log
        })
        .collect();
    runs.sort();
    let cutoff = now
        .checked_sub(std::time::Duration::from_secs(KEEP_DAYS * 86_400))
        .map(|at| format!("aetherd-{}", stamp(Some(at))));
    let old = |path: &PathBuf| {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with("aetherd-unknown") {
            let written = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            return written
                .and_then(|at| now.duration_since(at).ok())
                .is_some_and(|age| age.as_secs() > KEEP_DAYS * 86_400);
        }
        cutoff.as_deref().is_some_and(|cut| name < cut)
    };
    let protected = runs.len().saturating_sub(KEEP_AT_LEAST);
    let over = runs.len().saturating_sub(KEEP_AT_MOST);
    for (i, path) in runs.iter().enumerate() {
        if i < over || (i < protected && old(path)) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-logs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn each_run_is_kept_dated_and_the_last_as_prev() {
        let dir = scratch("kept");
        let log = dir.join("aetherd.log");
        let run = "2026-10-04T21:32:27.810Z info  daemon: aetherd starting\n";
        std::fs::write(&log, run).expect("write");
        set_aside(&log, std::time::SystemTime::now());
        assert!(!log.exists(), "the new run starts from a clean file");
        assert_eq!(std::fs::read_to_string(previous(&log)).expect("prev"), run);
        let kept: Vec<_> = std::fs::read_dir(runs_dir(&log))
            .expect("logs/")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(kept.len(), 1);
        let name = kept[0].file_name().into_string().expect("name");
        // named for when the run began, from its first line — not the file's creation time,
        // which Windows carries over from the log deleted before it
        assert_eq!(name, "aetherd-20261004-213227.log");
        assert_eq!(
            kept[0].path().extension().and_then(|e| e.to_str()),
            Some("log")
        );
        assert_eq!(std::fs::read_to_string(kept[0].path()).expect("dated"), run);
        // a start with no log before it keeps nothing and fails nothing
        set_aside(&log, std::time::SystemTime::now());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn left_in(runs: &Path) -> Vec<String> {
        let mut left: Vec<String> = std::fs::read_dir(runs)
            .expect("logs/")
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        left.sort();
        left
    }

    /// 2026-10-07 12:00:00 UTC.
    fn october_7() -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_791_374_400)
    }

    #[test]
    fn a_months_runs_are_kept_and_older_ones_go() {
        // WC4Y's evening of 2026-10-05 was asked about two days and a dozen restarts later
        let dir = scratch("pruned");
        let runs = runs_dir(&dir.join("aetherd.log"));
        std::fs::create_dir_all(&runs).expect("logs/");
        // two runs a day from 2026-08-29 to 2026-10-07
        for day in 0..40u64 {
            let at = october_7() - std::time::Duration::from_secs((39 - day) * 86_400);
            for hour in ["010000", "130000"] {
                let date = &stamp(Some(at))[..8];
                std::fs::write(runs.join(format!("aetherd-{date}-{hour}.log")), "x")
                    .expect("write");
            }
        }
        // not ours: left alone
        std::fs::write(runs.join("notes.txt"), "mine").expect("write");
        prune(&runs, october_7());
        let left = left_in(&runs);
        assert!(left.contains(&"notes.txt".to_owned()));
        // 30 days back from noon on the 7th: the 7th of September at 13:00 stays, 01:00 goes
        assert_eq!(left[0], "aetherd-20260907-130000.log", "{left:?}");
        assert!(left.contains(&"aetherd-20261005-010000.log".to_owned()));
        assert_eq!(left.len(), 2 * 30 + 1 + 1, "{left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_last_few_runs_stay_whatever_their_age_and_a_loop_cannot_fill_the_disk() {
        let dir = scratch("bounds");
        let runs = runs_dir(&dir.join("aetherd.log"));
        std::fs::create_dir_all(&runs).expect("logs/");
        // a station last started in the spring keeps its last runs
        for day in 10..25 {
            std::fs::write(runs.join(format!("aetherd-202604{day}-120000.log")), "x")
                .expect("write");
        }
        prune(&runs, october_7());
        let left = left_in(&runs);
        assert_eq!(left.len(), KEEP_AT_LEAST, "{left:?}");
        assert_eq!(left[0], "aetherd-20260415-120000.log");
        // a daemon restarting in a loop today is held to the most kept
        for n in 0..(KEEP_AT_MOST + 20) {
            std::fs::write(
                runs.join(format!("aetherd-20261007-{:02}{:02}00.log", n / 60, n % 60)),
                "x",
            )
            .expect("write");
        }
        prune(&runs, october_7());
        let left = left_in(&runs);
        assert_eq!(left.len(), KEEP_AT_MOST, "{}", left.len());
        assert!(left.iter().all(|n| n.starts_with("aetherd-20261007")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_runs_in_one_second_keep_both() {
        let dir = scratch("same");
        let taken = dir.join("aetherd-20260927-011200.log");
        std::fs::write(&taken, "one").expect("write");
        assert_eq!(
            unique(&taken).file_name().and_then(|n| n.to_str()),
            Some("aetherd-20260927-011200-2.log")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_whose_first_line_has_no_time_is_named_for_its_last_write() {
        let dir = scratch("untimed");
        let log = dir.join("aetherd.log");
        std::fs::write(&log, "thread 'main' panicked\n").expect("write");
        assert_eq!(first_stamp(&log), None);
        set_aside(&log, std::time::SystemTime::now());
        let name = std::fs::read_dir(runs_dir(&log))
            .expect("logs/")
            .filter_map(Result::ok)
            .map(|e| e.file_name().into_string().expect("name"))
            .next()
            .expect("kept");
        assert!(name.starts_with("aetherd-20"), "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_stamp_is_utc_and_sorts() {
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_471_520);
        assert_eq!(stamp(Some(at)), "20260927-011200");
        assert_eq!(stamp(None), "unknown");
    }
}
