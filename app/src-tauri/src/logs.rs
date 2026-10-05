//! The daemon's logs from earlier runs.
//!
//! Each run writes `aetherd.log` from scratch, so it describes that run, and the run before is
//! kept as `aetherd.prev.log` — "I restarted it and it came good" is the commonest way a fault
//! is reported. One generation was not enough: ND1J's log from a Saturday night was asked for
//! on the Monday, and two restarts later only Monday's runs were left. So every run's log is
//! also kept under `logs/`, named for when the run began, and the newest [`KEEP`] stay.

use std::path::{Path, PathBuf};

/// How many earlier runs' logs are kept in `logs/`.
pub const KEEP: usize = 10;

/// Set the last run's log aside before a new run writes `log` from scratch: a dated copy in
/// `logs/` beside it, and `aetherd.prev.log` as before. Older dated copies beyond `keep` go.
/// Every step is best effort — a log that cannot be kept is never a reason not to start.
pub fn set_aside(log: &Path, keep: usize) {
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
            prune(&dir, keep);
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

/// Keep the newest `keep` dated logs: the names sort by time.
fn prune(dir: &Path, keep: usize) {
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
    let excess = runs.len().saturating_sub(keep);
    for old in &runs[..excess] {
        let _ = std::fs::remove_file(old);
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
        set_aside(&log, KEEP);
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
        set_aside(&log, KEEP);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_runs_are_kept_and_the_oldest_go() {
        let dir = scratch("pruned");
        let runs = runs_dir(&dir.join("aetherd.log"));
        std::fs::create_dir_all(&runs).expect("logs/");
        for day in 10..25 {
            std::fs::write(runs.join(format!("aetherd-202609{day}-120000.log")), "x")
                .expect("write");
        }
        // not ours: left alone
        std::fs::write(runs.join("notes.txt"), "mine").expect("write");
        prune(&runs, KEEP);
        let mut left: Vec<String> = std::fs::read_dir(&runs)
            .expect("logs/")
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        left.sort();
        assert_eq!(left.len(), KEEP + 1, "{left:?}");
        assert_eq!(left[0], "aetherd-20260915-120000.log");
        assert!(left.contains(&"notes.txt".to_owned()));
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
        set_aside(&log, KEEP);
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
