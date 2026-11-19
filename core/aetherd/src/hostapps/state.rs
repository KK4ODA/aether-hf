//! The finder as the daemon runs it: off the run loop, its last result kept, and the folders
//! the operator added remembered beside the configuration.
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

use super::find::{Found, Install};

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
}

/// What the station remembers of the finder (`hostapps.json`): not a setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    /// The folders the operator added, as named.
    #[serde(default)]
    pub folders: Vec<String>,
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
            .retain(|f| !super::find::same_path(Path::new(f), folder));
        if keep {
            self.remembered.folders.push(text);
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let json = serde_json::to_string_pretty(&self.remembered).unwrap_or_default();
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
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

    /// Whether a find finished since the clients were last told, and note that they are now.
    pub fn take_news(&mut self) -> bool {
        let finished = self.shared.lock().map_or(0, |s| s.finished);
        if finished == self.told {
            return false;
        }
        self.told = finished;
        true
    }
}
