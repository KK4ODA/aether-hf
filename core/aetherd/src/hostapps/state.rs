//! The finder as the daemon runs it: off the run loop, its last result kept, and what it
//! remembers beside the configuration (`hostapps.json`): the folders the operator added, the
//! setups set up for Aether and, for each install, every setting Aether wrote into it with the
//! value it had before — what going back to VARA puts back (ADR-0133 §8).
//!
//! Finding lists the processes, the registry's uninstall entries and the shortcuts, and reads
//! each install's files: a fraction of a second, which the run loop must not wait out (it held
//! the modem still once for the audio devices, [`crate::devices`]). A find runs on a thread of
//! its own; `hostapps.find` answers with the last result and begins another, and the clients
//! are given the new one as a `hostapps` event when it finishes.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

use super::{
    find::{Found, Install, same_path},
    import::Keying,
    point::{Change, Original},
};

/// The file beside the configuration that remembers the folders the operator added.
pub const STATE_FILE: &str = "hostapps.json";

/// How installs are found: a function, so a test can stand in for the computer.
pub type Source = fn(&[PathBuf]) -> Found;

/// Find on this computer, with the folders added.
#[must_use]
pub fn this_computer(added: &[PathBuf]) -> Found {
    super::find::found(added)
}

#[derive(Debug, Default)]
struct Shared {
    last: Option<Vec<Install>>,
    took_ms: Vec<(&'static str, u64)>,
    busy: bool,
    finished: u64,
    copy: Option<FolderCopy>,
}

/// A copy of an install made before it is set up, so the one that works with VARA stays as it
/// is: made off the run loop (a Winlink Express folder holds its mail too), then found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderCopy {
    /// The install copied.
    pub from: String,
    /// The new folder.
    pub to: String,
    /// `copying`, `done` (copied and found: the copy is in the last find) or `failed`.
    pub state: CopyState,
    /// Why it failed.
    pub error: Option<String>,
    /// Files and bytes copied.
    pub files: u64,
    /// Bytes copied.
    pub bytes: u64,
    /// The folder remembered as added: done once, when the clients are told.
    #[serde(skip)]
    noted: bool,
}

/// Where a copy stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyState {
    /// Under way.
    Copying,
    /// Copied and found.
    Done,
    /// Not made; the folder taken out again.
    Failed,
}

/// A setup set up for Aether: its profile made and its program set to reach Aether.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetUp {
    /// The install's folder.
    pub folder: String,
    /// The setup's name in the program.
    pub setup: String,
    /// The Aether profile made from it.
    pub profile: String,
    /// Who keys the radio in it.
    pub keying: Option<Keying>,
    /// When, as the profiles write a time.
    pub at: String,
}

/// A setting Aether wrote into an install, with what it said before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Written {
    /// The install's folder.
    pub folder: String,
    /// The setting, as it was and as Aether set it.
    #[serde(flatten)]
    pub original: Original,
}

/// What the station remembers of the finder (`hostapps.json`): not a setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    /// The folders the operator added, as named.
    #[serde(default)]
    pub folders: Vec<String>,
    /// The setups set up for Aether.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub set_up: Vec<SetUp>,
    /// Every setting Aether wrote into an install, with what it said before: the first value,
    /// kept through later setups of the same install.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub written: Vec<Written>,
}

/// The finder: its last result, the one under way, and the folders added.
#[derive(Debug, Clone)]
pub struct Finder {
    shared: Arc<Mutex<Shared>>,
    /// How installs are found.
    pub source: Source,
    /// Find on the caller's thread: a test's.
    pub inline: bool,
    /// What runs now, asked before a program's settings are written: the system's, or a
    /// test's.
    pub running: fn() -> Vec<PathBuf>,
    /// The state file; `None` for a daemon without a configuration file.
    path: Option<PathBuf>,
    /// What it holds.
    pub remembered: Remembered,
    /// Finds finished when the clients were last told.
    pub told: u64,
}

impl Finder {
    /// The finder of a daemon whose state file is `path`.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let remembered = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            shared: Arc::default(),
            source: this_computer,
            inline: false,
            running: super::find::running_now,
            path,
            remembered,
            told: 0,
        }
    }

    /// The folders added, as paths.
    #[must_use]
    pub fn added(&self) -> Vec<PathBuf> {
        self.remembered.folders.iter().map(PathBuf::from).collect()
    }

    /// Remember a folder the operator added, or forget one; then write the file.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn remember(&mut self, folder: &Path, keep: bool) -> Result<(), String> {
        let text = folder.display().to_string();
        self.remembered
            .folders
            .retain(|f| !same_path(Path::new(f), folder));
        if keep {
            self.remembered.folders.push(text);
        }
        self.save()
    }

    /// Write what is remembered.
    fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(&self.remembered).unwrap_or_default();
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// The record of a setup set up for Aether.
    #[must_use]
    pub fn set_up_of(&self, folder: &str, setup: &str) -> Option<&SetUp> {
        self.remembered
            .set_up
            .iter()
            .find(|s| s.setup == setup && same_path(Path::new(&s.folder), Path::new(folder)))
    }

    /// The other setups of an install that are set up for Aether.
    #[must_use]
    pub fn others_set_up(&self, folder: &str, setup: &str) -> Vec<String> {
        self.remembered
            .set_up
            .iter()
            .filter(|s| s.setup != setup && same_path(Path::new(&s.folder), Path::new(folder)))
            .map(|s| s.setup.clone())
            .collect()
    }

    /// The settings Aether wrote into an install, with what they said before.
    #[must_use]
    pub fn originals(&self, folder: &str) -> Vec<Original> {
        self.remembered
            .written
            .iter()
            .filter(|w| same_path(Path::new(&w.folder), Path::new(folder)))
            .map(|w| w.original.clone())
            .collect()
    }

    /// Remember a setup set up, and the settings written for it: a setting written before
    /// keeps the value it had first.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn note_set_up(&mut self, record: SetUp, changes: &[Change]) -> Result<(), String> {
        for change in changes {
            let known = self.remembered.written.iter_mut().find(|w| {
                same_path(Path::new(&w.folder), Path::new(&record.folder))
                    && w.original.file == change.file
                    && w.original.section.eq_ignore_ascii_case(&change.section)
                    && w.original.key.eq_ignore_ascii_case(&change.key)
            });
            if let Some(known) = known {
                known.original.set_to.clone_from(&change.to);
            } else {
                self.remembered.written.push(Written {
                    folder: record.folder.clone(),
                    original: Original {
                        file: change.file.clone(),
                        section: change.section.clone(),
                        key: change.key.clone(),
                        was: change.from.clone(),
                        set_to: change.to.clone(),
                    },
                });
            }
        }
        self.remembered.set_up.retain(|s| {
            !(s.setup == record.setup && same_path(Path::new(&s.folder), Path::new(&record.folder)))
        });
        self.remembered.set_up.push(record);
        self.save()
    }

    /// Forget a setup set up; with `program` (the last of its install), forget what was
    /// written into the install too: it has been put back.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn forget_set_up(
        &mut self,
        folder: &str,
        setup: &str,
        program: bool,
    ) -> Result<(), String> {
        self.remembered
            .set_up
            .retain(|s| !(s.setup == setup && same_path(Path::new(&s.folder), Path::new(folder))));
        if program {
            self.remembered
                .written
                .retain(|w| !same_path(Path::new(&w.folder), Path::new(folder)));
        }
        self.save()
    }

    /// Copy an install's folder to a new one off the run loop, then find again with it added;
    /// the clients are told when it is done ([`Self::take_news`] remembers the new folder).
    /// One copy at a time.
    ///
    /// # Errors
    /// If a copy is under way.
    pub fn copy(&self, from: &Path, to: &Path) -> Result<(), String> {
        let mut added = self.added();
        added.push(to.to_path_buf());
        let copy = FolderCopy {
            from: from.display().to_string(),
            to: to.display().to_string(),
            state: CopyState::Copying,
            error: None,
            files: 0,
            bytes: 0,
            noted: false,
        };
        {
            let Ok(mut shared) = self.shared.lock() else {
                return Err("the finder is not answering".to_owned());
            };
            if shared
                .copy
                .as_ref()
                .is_some_and(|c| c.state == CopyState::Copying)
            {
                return Err("a copy is under way: wait for it".to_owned());
            }
            shared.copy = Some(copy);
            shared.finished += 1;
        }
        let shared = Arc::clone(&self.shared);
        let source = self.source;
        let (from, to) = (from.to_path_buf(), to.to_path_buf());
        let work = move || {
            let copied = copy_tree(&from, &to);
            let found = copied.is_ok().then(|| source(&added));
            if let Ok(mut state) = shared.lock() {
                if let Some(found) = found {
                    state.last = Some(found.installs);
                    state.took_ms = found.took_ms;
                }
                if let Some(copy) = state.copy.as_mut() {
                    match copied {
                        Ok((files, bytes)) => {
                            copy.state = CopyState::Done;
                            copy.files = files;
                            copy.bytes = bytes;
                        }
                        Err(why) => {
                            copy.state = CopyState::Failed;
                            copy.error = Some(why);
                        }
                    }
                }
                state.finished += 1;
            }
        };
        if self.inline {
            work();
            return Ok(());
        }
        std::thread::Builder::new()
            .name("hostapps-copy".into())
            .spawn(work)
            .map(|_| ())
            .map_err(|e| format!("cannot start the copy: {e}"))
    }

    /// The copy under way or last made.
    #[must_use]
    pub fn last_copy(&self) -> Option<FolderCopy> {
        self.shared.lock().ok().and_then(|s| s.copy.clone())
    }

    /// Put an install read again in the last find's place for it, and tell the clients: after
    /// its program's settings were written.
    pub fn replace(&self, install: Install) {
        let Ok(mut shared) = self.shared.lock() else {
            return;
        };
        if let Some(last) = shared.last.as_mut()
            && let Some(old) = last
                .iter_mut()
                .find(|i| super::find::same_path(Path::new(&i.folder), Path::new(&install.folder)))
        {
            *old = install;
            shared.finished += 1;
        }
    }

    /// Begin a find, unless one is under way (it answers for this one too).
    pub fn refresh(&self) {
        let added = self.added();
        if self.inline {
            let found = (self.source)(&added);
            if let Ok(mut shared) = self.shared.lock() {
                shared.last = Some(found.installs);
                shared.took_ms = found.took_ms;
                shared.finished += 1;
            }
            return;
        }
        {
            let Ok(mut shared) = self.shared.lock() else {
                return;
            };
            if shared.busy {
                return;
            }
            shared.busy = true;
        }
        let shared = Arc::clone(&self.shared);
        let source = self.source;
        let spawned = std::thread::Builder::new()
            .name("hostapps".into())
            .spawn(move || {
                let found = source(&added);
                if let Ok(mut state) = shared.lock() {
                    state.last = Some(found.installs);
                    state.took_ms = found.took_ms;
                    state.busy = false;
                    state.finished += 1;
                }
            });
        if spawned.is_err()
            && let Ok(mut state) = self.shared.lock()
        {
            state.busy = false;
        }
    }

    /// The last result, and whether a find is under way.
    #[must_use]
    pub fn last(&self) -> (Option<Vec<Install>>, bool) {
        self.shared
            .lock()
            .map_or((None, false), |s| (s.last.clone(), s.busy))
    }

    /// How long each stage of the last find took, in milliseconds.
    #[must_use]
    pub fn took_ms(&self) -> Vec<(&'static str, u64)> {
        self.shared
            .lock()
            .map_or_else(|_| Vec::new(), |s| s.took_ms.clone())
    }

    /// Whether a find (or a copy) finished since the clients were last told, and note that
    /// they are now. A copy done is remembered as a folder added, so every find finds it.
    pub fn take_news(&mut self) -> bool {
        let (finished, copied) = self.shared.lock().map_or((0, None), |mut s| {
            let copied = s.copy.as_mut().and_then(|c| {
                (c.state == CopyState::Done && !c.noted).then(|| {
                    c.noted = true;
                    c.to.clone()
                })
            });
            (s.finished, copied)
        });
        if let Some(folder) = copied {
            let _ = self.remember(Path::new(&folder), true);
        }
        if finished == self.told {
            return false;
        }
        self.told = finished;
        true
    }
}

/// Where a copy of an install goes: beside it, named for it and Aether (`C:\RMS Express` →
/// `C:\RMS Express Aether`, then `… Aether 2` up to 9 when that is taken); `None` when every
/// name is taken or the folder has no parent.
#[must_use]
pub fn copy_destination(folder: &Path) -> Option<PathBuf> {
    let parent = folder.parent()?;
    let name = folder.file_name()?.to_string_lossy().into_owned();
    (1..=9)
        .map(|n| {
            if n == 1 {
                parent.join(format!("{name} Aether"))
            } else {
                parent.join(format!("{name} Aether {n}"))
            }
        })
        .find(|candidate| !candidate.exists())
}

/// Copy a folder and everything in it to a new folder, which must not exist yet: the files
/// and bytes copied. Links are not followed. A copy that fails is taken out again, so nothing
/// half made is left to be found.
///
/// # Errors
/// If the new folder exists, or anything cannot be read or written.
pub fn copy_tree(from: &Path, to: &Path) -> Result<(u64, u64), String> {
    fn walk(from: &Path, to: &Path, count: &mut (u64, u64)) -> std::io::Result<()> {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = to.join(entry.file_name());
            if kind.is_dir() {
                walk(&entry.path(), &target, count)?;
            } else if kind.is_file() {
                count.1 += std::fs::copy(entry.path(), &target)?;
                count.0 += 1;
            }
        }
        Ok(())
    }
    if to.exists() {
        return Err(format!("{} is there already", to.display()));
    }
    let mut count = (0, 0);
    walk(from, to, &mut count).map_err(|e| {
        let _ = std::fs::remove_dir_all(to);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "Windows did not let Aether copy {} to {} ({e}): a folder under Program Files \
                 needs an administrator",
                from.display(),
                to.display()
            )
        } else {
            format!("cannot copy {} to {}: {e}", from.display(), to.display())
        }
    })?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_goes_beside_the_install_and_never_over_a_folder() {
        let dir = std::env::temp_dir().join(format!("aether-copy-{}", std::process::id()));
        let from = dir.join("RMS Express");
        std::fs::create_dir_all(from.join("N0CALL").join("Messages")).expect("dirs");
        std::fs::write(from.join("RMS Express.ini"), b"[Properties]\n").expect("ini");
        std::fs::write(from.join("N0CALL").join("Messages").join("A.mime"), b"mail").expect("mail");
        let to = copy_destination(&from).expect("a name");
        assert_eq!(to, dir.join("RMS Express Aether"));
        let (files, bytes) = copy_tree(&from, &to).expect("copied");
        assert_eq!((files, bytes), (2, 17));
        assert!(to.join("N0CALL").join("Messages").join("A.mime").is_file());
        assert_eq!(
            copy_destination(&from),
            Some(dir.join("RMS Express Aether 2"))
        );
        let refused = copy_tree(&from, &to).expect_err("there already");
        assert!(refused.contains("there already"), "{refused}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_setting_written_twice_keeps_the_value_it_had_first() {
        let mut finder = Finder::open(None);
        let change = |from: &str, to: &str| Change {
            file: "RMS Express.ini".into(),
            section: "Vara TNC".into(),
            key: "Auto Launch Vara".into(),
            from: Some(from.into()),
            to: to.into(),
            why: String::new(),
        };
        let record = |setup: &str| SetUp {
            folder: "C:\\RMS Express".into(),
            setup: setup.into(),
            profile: format!("Winlink Express {setup}"),
            keying: Some(Keying::Program),
            at: String::new(),
        };
        finder
            .note_set_up(record("N0CALL"), &[change("True", "False")])
            .expect("noted");
        finder
            .note_set_up(record("N1CALL"), &[change("False", "Off")])
            .expect("noted");
        let originals = finder.originals("c:/rms express");
        assert_eq!(originals.len(), 1);
        assert_eq!(originals[0].was.as_deref(), Some("True"));
        assert_eq!(originals[0].set_to, "Off");
        assert_eq!(
            finder.others_set_up("C:\\RMS Express", "N0CALL"),
            ["N1CALL"]
        );
        finder
            .forget_set_up("C:\\RMS Express", "N1CALL", false)
            .expect("forgot");
        assert_eq!(
            finder.originals("C:\\RMS Express").len(),
            1,
            "N0CALL still uses it"
        );
        finder
            .forget_set_up("C:\\RMS Express", "N0CALL", true)
            .expect("forgot");
        let left = finder.originals("C:\\RMS Express");
        assert!(left.is_empty(), "{left:?}");
        assert!(finder.set_up_of("C:\\RMS Express", "N0CALL").is_none());
    }
}
