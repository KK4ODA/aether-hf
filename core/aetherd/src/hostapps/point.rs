//! A program's own settings set to reach Aether (ADR-0133 §7, the author's decision of
//! 2026-11-19).
//!
//! What a program must change to use Aether — a VARA TNC on another computer, its own launch
//! of a TNC, and its keying when Aether keys the radio — is written into its settings file when
//! the operator sets it up, rather than said and left to be done by hand. Each program's reader
//! says which settings ([`Edit`]s); this module compares them with the file ([`changes`]),
//! writes them ([`point`]) and, to go back to VARA, puts back the ones still as Aether set them
//! ([`put_back`], ADR-0133 §8):
//!
//! * only the settings named, every other byte of the file kept (`crate::ini::Ini`), and the
//!   file as it was kept beside it (`<name>.aether.bak` when set up, `<name>.aether-undo.bak`
//!   when put back);
//! * never while the program runs: Winlink's programs read their settings when they start and
//!   write them back when they close, so a change made under one is lost, and it would not be
//!   seen before the next start anyway;
//! * never the setting that publishes a gateway's channels to Winlink, or any account, address
//!   or position: the readers name only a TNC's address, port and launch, and the radio's
//!   keying and control.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{Files, Program, find::Install};
use crate::ini::Ini;

/// Aether as a program should reach it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The host interface's command port.
    pub host_port: u16,
    /// Aether's application, for a program that starts its TNC itself.
    pub app: Option<String>,
    /// Aether keys the radio: the program gives up its keying, and its radio control when that
    /// is on the port Aether keys on (only one program can hold a port).
    pub aether_keys: bool,
}

/// One setting a program's reader would have written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The file, as the reader lists it.
    pub file: &'static str,
    /// The section.
    pub section: &'static str,
    /// The key.
    pub key: &'static str,
    /// The value wanted.
    pub to: String,
    /// Why, for the operator.
    pub why: &'static str,
}

/// A setting that differs from what the program needs: what it is and what it will be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// The file.
    pub file: String,
    /// The section.
    pub section: String,
    /// The key.
    pub key: String,
    /// What it says now; `None` when the file does not have it.
    pub from: Option<String>,
    /// What it will say.
    pub to: String,
    /// Why.
    pub why: String,
}

/// What [`point`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pointed {
    /// The program.
    pub program: Program,
    /// The install's folder.
    pub folder: String,
    /// The settings changed; empty when the program already said them all.
    pub changes: Vec<Change>,
    /// The files written, each with the copy of it as it was.
    pub written: Vec<Written>,
}

/// A setting as a program had it before Aether set it: what going back to VARA puts back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Original {
    /// The file.
    pub file: String,
    /// The section.
    pub section: String,
    /// The key.
    pub key: String,
    /// What it said; `None` when the file did not have it.
    pub was: Option<String>,
    /// What Aether set it to: it is put back only while it still says this.
    pub set_to: String,
}

/// What [`put_back`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PutBack {
    /// The settings put back, each as it was (`from`) and as it is again (`to`).
    pub put_back: Vec<Change>,
    /// The settings left alone: changed since Aether set them, so the operator's.
    pub left: Vec<Change>,
    /// The files written, each with the copy of it as it was.
    pub written: Vec<Written>,
}

/// A file written, and where it was kept as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Written {
    /// The file.
    pub path: String,
    /// The copy as it was.
    pub backup: String,
}

/// Whether a value is the one wanted: a program's yes or no in any case, a number as a number.
fn same(now: &str, wanted: &str) -> bool {
    let now = now.trim();
    now.eq_ignore_ascii_case(wanted)
        || now
            .parse::<f64>()
            .ok()
            .zip(wanted.parse::<f64>().ok())
            .is_some_and(|(a, b)| (a - b).abs() < f64::EPSILON)
}

/// The settings of a program's files that differ from what it needs to reach Aether. Pure.
#[must_use]
pub fn changes(program: Program, files: &Files, target: &Target) -> Vec<Change> {
    let edits = (program.reader().point)(files, target);
    edits
        .into_iter()
        .filter_map(|edit| {
            let from = files
                .get(edit.file)
                .and_then(|bytes| Ini::new(bytes.clone()).get(edit.section, edit.key));
            if from.as_deref().is_some_and(|now| same(now, &edit.to)) {
                return None;
            }
            Some(Change {
                file: edit.file.to_owned(),
                section: edit.section.to_owned(),
                key: edit.key.to_owned(),
                from,
                to: edit.to,
                why: edit.why.to_owned(),
            })
        })
        .collect()
}

/// An install's settings files, as its reader lists them.
///
/// # Errors
/// If a file there cannot be read for another reason than not being there.
pub fn files_of(install: &Install) -> Result<Files, String> {
    let mut files = Files::new();
    for name in install.program.reader().files {
        let path = Path::new(&install.folder).join(name);
        match std::fs::read(&path) {
            Ok(bytes) => {
                files.insert((*name).to_owned(), bytes);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        }
    }
    Ok(files)
}

/// What going back to VARA would do to an install's files now: each original as a change from
/// the file's value to the one it had, split into those still as Aether set them (put back)
/// and those changed since (left). Pure.
#[must_use]
pub fn going_back(files: &Files, originals: &[Original]) -> (Vec<Change>, Vec<Change>) {
    let mut put = Vec::new();
    let mut left = Vec::new();
    for original in originals {
        let now = files
            .get(&original.file)
            .and_then(|bytes| Ini::new(bytes.clone()).get(&original.section, &original.key));
        let change = Change {
            file: original.file.clone(),
            section: original.section.clone(),
            key: original.key.clone(),
            from: now.clone(),
            to: original.was.clone().unwrap_or_default(),
            why: if original.was.is_some() {
                "as it was before Aether set it".to_owned()
            } else {
                "not in the file before Aether set it: taken out".to_owned()
            },
        };
        if now.as_deref().is_some_and(|n| same(n, &original.set_to)) {
            put.push(change);
        } else {
            left.push(change);
        }
    }
    (put, left)
}

/// Write a file whole or not at all: to a new file first, then over the old one.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".aether-new");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, bytes).map_err(|e| write_error(path, &e))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        write_error(path, &e)
    })
}

/// Why a file could not be written, with what to do when Windows refused.
fn write_error(path: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "Windows did not let Aether write {} ({error}): a program under Program Files needs \
             an administrator to change its settings there",
            path.display()
        )
    } else {
        format!("cannot write {}: {error}", path.display())
    }
}

/// Why a running program's files are not written.
fn refuse_running(name: &str) -> String {
    format!(
        "{name} is running: close it first. It reads its settings when it starts and writes them \
         back when it closes, so a change made now would be lost"
    )
}

/// Set an install's program to reach Aether: the settings [`changes`] names, written into its
/// files, each file kept as it was beside it first. `running` is whether the program runs
/// now, asked of the system just before: a program that runs is refused.
///
/// # Errors
/// If the program runs, or a file cannot be read or written.
pub fn point(install: &Install, target: &Target, running: bool) -> Result<Pointed, String> {
    let name = install.program.name();
    if running {
        return Err(refuse_running(name));
    }
    let files = files_of(install)?;
    let changes = changes(install.program, &files, target);
    let mut written = Vec::new();
    for file in install.program.reader().files {
        let mine: Vec<&Change> = changes.iter().filter(|c| c.file == *file).collect();
        if mine.is_empty() {
            continue;
        }
        let Some(before) = files.get(*file) else {
            return Err(format!("{name} has no {file} to change"));
        };
        let mut ini = Ini::new(before.clone());
        for change in &mine {
            ini.set(&change.section, &change.key, &change.to);
        }
        let path = Path::new(&install.folder).join(file);
        let backup = Path::new(&install.folder).join(format!("{file}.aether.bak"));
        std::fs::write(&backup, before).map_err(|e| write_error(&backup, &e))?;
        write_atomically(&path, ini.bytes())?;
        written.push(Written {
            path: path.display().to_string(),
            backup: backup.display().to_string(),
        });
    }
    Ok(Pointed {
        program: install.program,
        folder: install.folder.clone(),
        changes,
        written,
    })
}

/// Go back to VARA: put back each setting Aether set that still says what Aether set it to,
/// leave the ones changed since, each file kept as it was beside it first
/// (`<name>.aether-undo.bak`). `running` as for [`point`].
///
/// # Errors
/// If the program runs, or a file cannot be read or written.
pub fn put_back(
    install: &Install,
    originals: &[Original],
    running: bool,
) -> Result<PutBack, String> {
    let name = install.program.name();
    if running {
        return Err(refuse_running(name));
    }
    let files = files_of(install)?;
    let (put, left) = going_back(&files, originals);
    let mut written = Vec::new();
    for file in install.program.reader().files {
        let mine: Vec<&Change> = put.iter().filter(|c| c.file == *file).collect();
        if mine.is_empty() {
            continue;
        }
        let Some(before) = files.get(*file) else {
            continue;
        };
        let mut ini = Ini::new(before.clone());
        for change in &mine {
            let was = originals
                .iter()
                .find(|o| {
                    o.file == change.file && o.section == change.section && o.key == change.key
                })
                .and_then(|o| o.was.clone());
            match was {
                Some(value) => {
                    ini.set(&change.section, &change.key, &value);
                }
                None => {
                    ini.remove(&change.section, &change.key);
                }
            }
        }
        let path = Path::new(&install.folder).join(file);
        let backup = Path::new(&install.folder).join(format!("{file}.aether-undo.bak"));
        std::fs::write(&backup, before).map_err(|e| write_error(&backup, &e))?;
        write_atomically(&path, ini.bytes())?;
        written.push(Written {
            path: path.display().to_string(),
            backup: backup.display().to_string(),
        });
    }
    Ok(PutBack {
        put_back: put,
        left,
        written,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostapps::{find::FoundBy, winlink_express};

    /// Winlink Express's file with VARA HF on another computer and its launch on, a personal
    /// key beside them, and LF line ends.
    const INI: &str = "[Properties]\nCallsign=N0CALL\nCall signs=N0CALL\n\
        [N0CALL]\nPassword=secret\nMode=Vara HF P2P\n\
        [Vara TNC]\nTCP Port=8300\nTNC Address=192.168.1.9\nAuto Launch Vara=True\n\
        Vara Program File=C:\\VARA\\VARA.exe\n";

    fn target() -> Target {
        Target {
            host_port: 8304,
            app: Some("C:\\Aether HF\\aether-hf.exe".into()),
            aether_keys: false,
        }
    }

    fn install(dir: &Path) -> Install {
        Install {
            program: Program::WinlinkExpress,
            program_name: "Winlink Express",
            folder: dir.display().to_string(),
            exe: dir.join("RMS Express.exe").display().to_string(),
            found_by: vec![FoundBy::Added],
            running: false,
            version: None,
            callsigns: vec!["N0CALL".into()],
            setups: Vec::new(),
            notes: Vec::new(),
        }
    }

    fn files(text: &str) -> Files {
        Files::from([(
            winlink_express::SETTINGS.to_owned(),
            text.as_bytes().to_vec(),
        )])
    }

    #[test]
    fn the_changes_are_what_differs_and_nothing_personal() {
        let found = changes(Program::WinlinkExpress, &files(INI), &target());
        let keys: Vec<(&str, &str)> = found
            .iter()
            .map(|c| (c.key.as_str(), c.to.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("TNC Address", "127.0.0.1"),
                ("TCP Port", "8304"),
                ("Auto Launch Vara", "False"),
            ]
        );
        assert_eq!(found[0].from.as_deref(), Some("192.168.1.9"));
        assert!(found.iter().all(|c| c.section == "Vara TNC"));
        // pointed already: nothing to change
        let done = "[Properties]\nCallsign=N0CALL\n[Vara TNC]\nTCP Port=8300\n\
            TNC Address=127.0.0.1\nAuto Launch Vara=false\n";
        assert!(
            changes(Program::WinlinkExpress, &files(done), &target()).is_empty(),
            "{:?}",
            changes(Program::WinlinkExpress, &files(done), &target())
        );
    }

    #[test]
    fn a_running_program_is_refused_and_a_closed_one_written_with_a_backup() {
        let dir = std::env::temp_dir().join(format!("aether-point-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(winlink_express::SETTINGS);
        std::fs::write(&path, INI).expect("write");
        let install = install(&dir);

        let refused = point(&install, &target(), true).expect_err("running");
        assert!(refused.contains("close it first"), "{refused}");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), INI);

        let done = point(&install, &target(), false).expect("pointed");
        assert_eq!(done.changes.len(), 3);
        assert_eq!(done.written.len(), 1);
        let after = std::fs::read_to_string(&path).expect("read");
        let ini = Ini::new(after.clone().into_bytes());
        assert_eq!(ini.get("Vara TNC", "TCP Port").as_deref(), Some("8304"));
        assert_eq!(
            ini.get("Vara TNC", "TNC Address").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(
            ini.get("Vara TNC", "Auto Launch Vara").as_deref(),
            Some("False")
        );
        // every other byte kept: the password, the program path, the line ends
        assert_eq!(ini.get("N0CALL", "Password").as_deref(), Some("secret"));
        assert!(after.contains("Vara Program File=C:\\VARA\\VARA.exe\n"));
        assert!(!after.contains('\r'));
        let kept = std::fs::read_to_string(dir.join("RMS Express.ini.aether.bak")).expect("bak");
        assert_eq!(kept, INI);

        // again: nothing differs, nothing written
        let again = point(&install, &target(), false).expect("again");
        assert!(again.changes.is_empty() && again.written.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
