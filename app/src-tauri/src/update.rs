//! Newer versions, and older ones.
//!
//! The shell looks for a newer version when it starts — on the channel the station's
//! configuration names — and asks before installing one. Every installer it installs is
//! kept, so the version before this one is always at hand: an update that turns out not to
//! work on somebody's machine is undone from the Help menu, without a network and without
//! remembering where it came from. Nothing here ever installs without a person saying yes.
//!
//! Signatures: each installer is signed at release time with an Ed25519 key, and the public
//! half is built into this binary (`plugins.updater.pubkey` in `tauri.conf.json`). The
//! updater refuses anything the key did not sign, so a compromised download host cannot
//! hand out a modem that keys somebody's radio.
//!
//! # The window
//!
//! What the operator sees is a window of the shell's own (`app/ui/update.html`, bundled
//! with the panel and served from the binary, so it works while the modem is stopped and the
//! network is down): the version they have, the version on offer, the release notes, and one
//! phase at a time — checking, available, downloading with progress, installing, restart
//! required, complete — or what went wrong and what to do about it. The shell keeps the
//! [`View`] and pushes every change to the window as an `update` event; the window asks for
//! it once on load and acts through the `update_*` commands. On Windows the installer
//! relaunches the shell, so "complete" is shown by the *next* start, which finds the note
//! this one left ([`after_start`]).

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter as _, Manager as _};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt as _;
use tauri_plugin_updater::{Update, UpdaterExt as _};

/// Where the releases are.
pub const REPOSITORY: &str = "https://github.com/KK4ODA/aether-hf";

/// The window's label, which its capability names.
const WINDOW: &str = "updater";

/// Which releases to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Tagged releases only.
    Stable,
    /// Betas, and any stable release newer than the beta in hand.
    Beta,
    /// The rolling nightly, and anything newer on the other channels.
    Nightly,
}

impl Channel {
    fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}

/// What the station's configuration says about updates.
#[derive(Debug, Clone, Copy)]
pub struct Preferences {
    /// Which channel to follow.
    pub channel: Channel,
    /// Whether to look at all on start.
    pub check: bool,
}

/// Whether `version` is a pre-release: a hyphen after the patch number (`0.2.0-beta.56`).
fn is_prerelease(version: &str) -> bool {
    version.contains('-')
}

/// The channel an installation follows unless its configuration says otherwise: a beta
/// build the betas, a release the stable channel — as the daemon's own default does.
fn channel_for_build(version: &str) -> Channel {
    if is_prerelease(version) {
        Channel::Beta
    } else {
        Channel::Stable
    }
}

/// The configuration's schema from which a beta build's `stable` is the operator's choice:
/// the daemon's migration to it turns a beta station's `stable` into `beta`, because the
/// panel had written `stable` into nearly every file whether or not anybody chose it.
const CHANNEL_CHOSEN_SCHEMA: i64 = 5;

/// Read the `[update]` section of the daemon's configuration file.
///
/// The daemon owns the file and the panel edits it; the shell reads it at every check, so
/// a channel chosen in Setup counts from the next one. A file that cannot be read means
/// the defaults: this build's channel, and do check.
pub fn preferences(config: &Path) -> Preferences {
    preferences_for(config, env!("CARGO_PKG_VERSION"))
}

/// [`preferences`] as a build of `version` reads them.
fn preferences_for(config: &Path, version: &str) -> Preferences {
    let text = std::fs::read_to_string(config).unwrap_or_default();
    let table: toml::Table = toml::from_str(&text).unwrap_or_default();
    let update = table.get("update").and_then(toml::Value::as_table);
    let schema = table
        .get("schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);
    let channel = match update
        .and_then(|u| u.get("channel"))
        .and_then(toml::Value::as_str)
    {
        Some("beta") => Channel::Beta,
        Some("nightly") => Channel::Nightly,
        // a file the daemon has not brought forward yet reads as it will once it has
        Some("stable") if schema >= CHANNEL_CHOSEN_SCHEMA || !is_prerelease(version) => {
            Channel::Stable
        }
        _ => channel_for_build(version),
    };
    let check = update
        .and_then(|u| u.get("check"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(true);
    Preferences { channel, check }
}

/// The manifests a channel reads, most specific first.
///
/// A beta installation also reads the stable manifest, so a stable release newer than its
/// beta is offered; a stable installation reads only the stable one, so it is never offered
/// a beta. GitHub keeps `releases/latest` clear of prereleases, which is what makes the
/// stable endpoint safe.
fn endpoints(channel: Channel) -> Vec<String> {
    let stable = format!("{REPOSITORY}/releases/latest/download/latest.json");
    let beta = format!("{REPOSITORY}/releases/download/channel-beta/latest.json");
    let nightly = format!("{REPOSITORY}/releases/download/nightly/latest.json");
    match channel {
        Channel::Stable => vec![stable],
        Channel::Beta => vec![beta, stable],
        Channel::Nightly => vec![nightly, beta, stable],
    }
}

/// Whether `candidate` is a newer version than `current`, by semantic versioning.
fn newer(candidate: &str, current: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(current),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

// ── what the window shows ───────────────────────────────────────────

/// One phase of an update, as the window shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "kebab-case")]
pub enum Phase {
    /// Asking the channel.
    Checking,
    /// Nothing newer.
    UpToDate,
    /// A beta following the stable channel, which has nothing newer to offer it: every
    /// release is a beta until the first stable one.
    NoStableYet,
    /// A newer version, waiting for a yes.
    Available {
        /// Its version.
        version: String,
        /// Its release notes, Markdown as the release carries them.
        notes: String,
        /// When it was published, RFC 3339, if the manifest says.
        date: Option<String>,
    },
    /// The installer is coming down.
    Downloading {
        /// Its version.
        version: String,
        /// Bytes so far.
        downloaded: u64,
        /// Bytes in all, when the server said.
        total: Option<u64>,
    },
    /// The modem is stopping and the installer is starting.
    Installing {
        /// Its version.
        version: String,
    },
    /// Installed; this process has to be started again to run it.
    RestartRequired {
        /// Its version.
        version: String,
    },
    /// This start is the first of a version an earlier start installed.
    Complete {
        /// The version now running.
        version: String,
        /// Its notes, kept from the offer.
        notes: String,
        /// The version it replaced.
        from: String,
    },
    /// An earlier start began installing a version that is not the one running now.
    Incomplete {
        /// The version that did not arrive.
        version: String,
    },
    /// Something went wrong.
    Error {
        /// What, in a sentence, with what to do about it.
        message: String,
        /// Whether trying again is worth offering.
        retry: bool,
    },
}

/// Everything the window renders.
#[derive(Debug, Clone, Serialize)]
pub struct View {
    /// The phase and its details.
    #[serde(flatten)]
    pub phase: Phase,
    /// The version running.
    pub current: String,
    /// The channel followed.
    pub channel: &'static str,
    /// Whether an earlier version is kept on this machine to go back to.
    pub can_restore: bool,
}

/// The shell's side of the window: the view it shows and the update it holds.
pub struct Updater {
    /// The daemon's configuration file, read for the channel at every check.
    config: Option<PathBuf>,
    view: Mutex<View>,
    pending: Mutex<Option<Update>>,
}

impl Updater {
    /// Fresh, reading its channel from `config` (this build's own without one).
    #[must_use]
    pub fn new(config: Option<PathBuf>, current: &str) -> Self {
        let updater = Self {
            config,
            view: Mutex::new(View {
                phase: Phase::UpToDate,
                current: current.to_owned(),
                channel: channel_for_build(current).name(),
                can_restore: previous_installer(current).is_some(),
            }),
            pending: Mutex::new(None),
        };
        let channel = updater.channel();
        if let Ok(mut view) = updater.view.lock() {
            view.channel = channel.name();
        }
        updater
    }

    /// The channel the configuration names now.
    fn channel(&self) -> Channel {
        self.config.as_deref().map_or_else(
            || channel_for_build(env!("CARGO_PKG_VERSION")),
            |path| preferences(path).channel,
        )
    }
}

/// Set the phase and tell the window.
fn show(app: &AppHandle, phase: Phase) {
    let state = app.state::<Updater>();
    let view = {
        let Ok(mut view) = state.view.lock() else {
            return;
        };
        view.phase = phase;
        view.can_restore = previous_installer(&view.current).is_some();
        view.clone()
    };
    let _ = app.emit_to(WINDOW, "update", &view);
}

/// Open the window, or bring it to the front if it is open.
fn open_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let built =
        tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::App("update.html".into()))
            .title("Aether HF updates")
            .inner_size(600.0, 560.0)
            .min_inner_size(440.0, 380.0)
            .resizable(true)
            .theme(Some(tauri::Theme::Dark))
            .build();
    if let Err(error) = built {
        // the window could not be made: the one thing left is a native box
        app.dialog()
            .message(format!("Could not open the updates window: {error}"))
            .title("Aether HF updates")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}

/// Look for a newer version on the channel and, if there is one, offer it in the window.
///
/// `quiet` is the start-up check: nothing newer means nothing said, and a channel that
/// cannot be reached means the same — a start with the network down should look like
/// nothing happened. From the menu the window opens first and says what it finds.
pub async fn check(app: AppHandle, quiet: bool) {
    let channel = {
        let state = app.state::<Updater>();
        let channel = state.channel();
        if let Ok(mut view) = state.view.lock() {
            view.channel = channel.name();
        }
        channel
    };
    if !quiet {
        show(&app, Phase::Checking);
        open_window(&app);
    }
    let mut best: Option<Update> = None;
    let mut failures = Vec::new();
    for endpoint in endpoints(channel) {
        let Ok(url) = endpoint.parse::<tauri::Url>() else {
            continue;
        };
        let updater = match app
            .updater_builder()
            .endpoints(vec![url])
            .and_then(tauri_plugin_updater::UpdaterBuilder::build)
        {
            Ok(updater) => updater,
            Err(error) => {
                failures.push(error.to_string());
                continue;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => {
                if best
                    .as_ref()
                    .is_none_or(|held| newer(&update.version, &held.version))
                {
                    best = Some(update);
                }
            }
            // a channel that has never published answers 404, which is "nothing newer",
            // not a failure to report
            Ok(None) | Err(tauri_plugin_updater::Error::ReleaseNotFound) => {}
            Err(error) => failures.push(error.to_string()),
        }
    }

    let Some(update) = best else {
        if quiet {
            return;
        }
        let current = app.package_info().version.to_string();
        if failures.is_empty() && channel == Channel::Stable && is_prerelease(&current) {
            // not "the newest": a beta on the stable channel is offered nothing while every
            // release is a beta, and saying so is what tells a tester to choose betas
            show(&app, Phase::NoStableYet);
        } else if failures.is_empty() {
            show(&app, Phase::UpToDate);
        } else {
            show(
                &app,
                Phase::Error {
                    message: format!(
                        "Could not check for updates: {}. Check the network and try again; \
                         the releases page always has the newest version.",
                        failures.join("; ")
                    ),
                    retry: true,
                },
            );
        }
        return;
    };

    let phase = Phase::Available {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default().trim().to_owned(),
        date: update.date.and_then(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .ok()
        }),
    };
    if let Ok(mut pending) = app.state::<Updater>().pending.lock() {
        *pending = Some(update);
    }
    show(&app, phase);
    open_window(&app);
}

/// Bring the installer down, reporting progress to the window ten times a second.
async fn download(app: &AppHandle, update: &Update) -> Result<Vec<u8>, String> {
    let version = update.version.clone();
    show(
        app,
        Phase::Downloading {
            version: version.clone(),
            downloaded: 0,
            total: None,
        },
    );
    let mut downloaded: u64 = 0;
    let mut reported = std::time::Instant::now();
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                // ten times a second is a progress bar; every chunk is a flicker
                if reported.elapsed() >= std::time::Duration::from_millis(100) {
                    reported = std::time::Instant::now();
                    show(
                        app,
                        Phase::Downloading {
                            version: version.clone(),
                            downloaded,
                            total,
                        },
                    );
                }
            },
            || {},
        )
        .await
        .map_err(|error| {
            format!(
                "The download did not complete: {error}. Nothing was changed; try again \
                 when the network is back."
            )
        })?;
    show(
        app,
        Phase::Downloading {
            version: update.version.clone(),
            downloaded: bytes.len() as u64,
            total: Some(bytes.len() as u64),
        },
    );
    Ok(bytes)
}

/// Download, keep a copy, stop the modem, install, restart.
async fn install(app: AppHandle) {
    let Some(update) = app
        .state::<Updater>()
        .pending
        .lock()
        .ok()
        .and_then(|pending| pending.clone())
    else {
        show(
            &app,
            Phase::Error {
                message: "There is no update waiting to be installed. Check again.".into(),
                retry: true,
            },
        );
        return;
    };
    let version = update.version.clone();
    let bytes = match download(&app, &update).await {
        Ok(bytes) => bytes,
        Err(message) => {
            show(
                &app,
                Phase::Error {
                    message,
                    retry: true,
                },
            );
            return;
        }
    };
    // Keep what is being installed, so the *next* update can be undone to this one — and
    // keep it before installing, when the bytes are certainly still here.
    if let Some(dir) = rollback_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(installer_name(&version)), &bytes);
    }
    show(
        &app,
        Phase::Installing {
            version: version.clone(),
        },
    );
    // The daemon's binary is about to be replaced, and a file in use cannot be. Stopping it
    // here also releases the transmitter properly, which a replaced binary would not — and
    // the installer is not started until the file really is free.
    crate::stop_daemon(&app.state::<crate::Daemon>());
    if let Err(error) = crate::release_daemon_binary(std::time::Duration::from_secs(15)) {
        show(
            &app,
            Phase::Error {
                message: format!("The update did not install: {error}"),
                retry: true,
            },
        );
        return;
    }
    // the next start finds this and says the update is complete — or that it is not
    let _ = Note {
        installing: version.clone(),
        from: app.package_info().version.to_string(),
        notes: update.body.clone().unwrap_or_default(),
        at: unix_now(),
    }
    .write();
    // On Windows this runs the installer and exits the process; the installer starts the
    // new version. Elsewhere it returns, and the new version needs a restart.
    if let Err(error) = update.install(bytes) {
        let _ = Note::remove();
        show(
            &app,
            Phase::Error {
                message: format!(
                    "The update did not install: {error}. The version you have is unchanged."
                ),
                retry: true,
            },
        );
        return;
    }
    show(&app, Phase::RestartRequired { version });
}

// ── the note one start leaves for the next ──────────────────────────

/// What an install in progress writes down, for the start that follows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Note {
    installing: String,
    from: String,
    notes: String,
    at: u64,
}

impl Note {
    fn path() -> Option<PathBuf> {
        rollback_dir().and_then(|dir| dir.parent().map(|p| p.join("update-note.json")))
    }

    fn write(&self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("no data directory"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    fn read() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn remove() -> std::io::Result<()> {
        match Self::path() {
            Some(path) if path.exists() => std::fs::remove_file(path),
            _ => Ok(()),
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the previous start left behind, and what this one should say about it.
///
/// A note naming the version now running means the update went through: say so, once,
/// with its notes. A note naming another version means it did not — the installer was
/// cancelled, or failed — and the operator should hear that rather than wonder why nothing
/// changed. A note older than a day is stale and says nothing.
fn verdict(note: &Note, current: &str, now: u64) -> Option<Phase> {
    if now.saturating_sub(note.at) > 24 * 3600 {
        return None;
    }
    if note.installing == current {
        Some(Phase::Complete {
            version: current.to_owned(),
            notes: note.notes.trim().to_owned(),
            from: note.from.clone(),
        })
    } else if note.from == current {
        Some(Phase::Incomplete {
            version: note.installing.clone(),
        })
    } else {
        None
    }
}

/// On start: if the previous start was installing something, say how that went.
///
/// Returns whether a window was opened, so the start-up check can stay quiet.
pub fn after_start(app: &AppHandle) -> bool {
    let Some(note) = Note::read() else {
        return false;
    };
    let _ = Note::remove();
    let current = app.package_info().version.to_string();
    let Some(phase) = verdict(&note, &current, unix_now()) else {
        return false;
    };
    show(app, phase);
    open_window(app);
    true
}

// ── the window's commands ───────────────────────────────────────────
//
// Tauri hands a command its handle and its state by value; that is the shape it wants.

/// What the window shows; asked for once when the page loads.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn update_view(state: tauri::State<'_, Updater>) -> Result<View, String> {
    state
        .view
        .lock()
        .map(|view| view.clone())
        .map_err(|_| "the updater's state is poisoned".to_owned())
}

/// Look again.
#[tauri::command]
pub fn update_check(app: AppHandle) {
    tauri::async_runtime::spawn(check(app, false));
}

/// Install what is on offer.
#[tauri::command]
pub fn update_install(app: AppHandle) {
    tauri::async_runtime::spawn(install(app));
}

/// Start this process again, on the version just installed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn update_restart(app: AppHandle) {
    crate::stop_daemon(&app.state::<crate::Daemon>());
    app.restart();
}

/// Close the window.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn update_close(app: AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.close();
    }
}

/// Open a page of the repository in the browser — the releases page, or a link in the
/// notes. Only the repository's own pages, so a note cannot send anybody anywhere else.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn update_open(app: AppHandle, url: Option<String>) -> Result<(), String> {
    let target = url.unwrap_or_else(|| format!("{REPOSITORY}/releases"));
    if !target.starts_with(REPOSITORY) && !target.starts_with("https://github.com/KK4ODA/") {
        return Err("only the project's own pages open from here".to_owned());
    }
    app.opener()
        .open_url(target, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Go back to the version before this one, from the window.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub fn update_restore(app: AppHandle) {
    restore_previous(&app);
}

// ── kept installers ─────────────────────────────────────────────────

/// What an installer is called in the rollback directory. The version is in the name, so
/// the directory itself is the record of what has been installed.
fn installer_name(version: &str) -> String {
    if cfg!(windows) {
        format!("aether-hf_{version}-setup.exe")
    } else {
        format!("aether-hf_{version}.AppImage")
    }
}

/// The version an installer in the rollback directory holds, from its name.
fn version_of(name: &str) -> Option<semver::Version> {
    let rest = name.strip_prefix("aether-hf_")?;
    let end = rest.find("-setup.exe").or_else(|| rest.find(".AppImage"))?;
    semver::Version::parse(&rest[..end]).ok()
}

/// Where kept installers live: beside the daemon's log, in the user's local data.
pub fn rollback_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
    };
    base.map(|dir| dir.join("aether-hf").join("rollback"))
}

/// The newest kept installer older than what is running, if there is one.
/// The settings schema each release reads, from the version it arrived in — the daemon's
/// `config::SCHEMA_VERSION` through the releases. Going back to a version that reads an
/// older shape than the file has been brought to leaves it unable to start (beta.56 to
/// beta.52, 2026-09-25), so a restore looks here first. A schema bump adds a line; a test
/// holds the last one to the daemon's constant.
const SCHEMA_HISTORY: &[(&str, u32)] = &[
    ("0.2.0-beta.1", 1),
    ("0.2.0-beta.51", 2),
    ("0.2.0-beta.53", 3),
    ("0.2.0-beta.54", 4),
    ("0.2.0-beta.56", 5),
    ("0.2.0-beta.60", 6),
];

/// The settings schema a version reads, if it is one this build knows of.
fn schema_read_by(version: &semver::Version) -> Option<u32> {
    SCHEMA_HISTORY
        .iter()
        .filter_map(|(from, schema)| Some((semver::Version::parse(from).ok()?, *schema)))
        .filter(|(from, _)| from <= version)
        .map(|(_, schema)| schema)
        .next_back()
}

/// What a restore does with the settings file.
#[derive(Debug, PartialEq, Eq)]
enum SettingsPlan {
    /// The earlier version reads it as it is.
    Keep,
    /// The earlier version reads an older shape: the copy kept before the file was brought
    /// forward goes back in its place, and the file as it is now is kept beside it.
    Swap {
        /// The copy to put back.
        backup: PathBuf,
        /// The schema the file is at now.
        from: u32,
    },
    /// The earlier version reads an older shape, and no copy it can read was kept.
    Unreadable {
        /// The schema the file is at now.
        current: u32,
        /// The newest the earlier version reads.
        reads: u32,
    },
}

/// What to do with a settings file at schema `current`, going back to a version that reads
/// up to `reads` (unknown: as it always was), with `backups` kept beside it.
fn plan_settings(current: u32, reads: Option<u32>, backups: &[(u32, PathBuf)]) -> SettingsPlan {
    let Some(reads) = reads else {
        return SettingsPlan::Keep;
    };
    if reads >= current {
        return SettingsPlan::Keep;
    }
    backups
        .iter()
        .filter(|(schema, _)| *schema <= reads)
        .max_by_key(|(schema, _)| *schema)
        .map_or(SettingsPlan::Unreadable { current, reads }, |(_, path)| {
            SettingsPlan::Swap {
                backup: path.clone(),
                from: current,
            }
        })
}

/// The schema a settings file says it is at; a file that does not say is the first.
fn schema_of_file(path: &Path) -> u32 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::from_str::<toml::Table>(&text).ok())
        .and_then(|table| {
            table
                .get("schema_version")
                .and_then(toml::Value::as_integer)
        })
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1)
}

/// The copies the daemon kept beside a settings file when it brought it forward
/// (`<name>.bak-v<n>`), as (schema, path).
fn backups_beside(path: &Path) -> Vec<(u32, PathBuf)> {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{name}.bak-v");
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name();
            let schema = file.to_str()?.strip_prefix(&prefix)?.parse().ok()?;
            Some((schema, entry.path()))
        })
        .collect()
}

/// Put `backup` in the settings file's place, keeping the file as it is as
/// `<name>.newer-v<from>` — the name the daemon gives a newer file it could not read.
fn swap_settings(config: &Path, backup: &Path, from: u32) -> Result<(), String> {
    let name = config.file_name().map_or_else(
        || "station.toml".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let kept = config.with_file_name(format!("{name}.newer-v{from}"));
    std::fs::copy(config, &kept).map_err(|e| format!("{}: {e}", kept.display()))?;
    let incoming = config.with_file_name(format!("{name}.restoring"));
    std::fs::copy(backup, &incoming).map_err(|e| format!("{}: {e}", incoming.display()))?;
    std::fs::rename(&incoming, config).map_err(|e| format!("{}: {e}", config.display()))
}

fn previous_installer(current: &str) -> Option<(semver::Version, PathBuf)> {
    let current = semver::Version::parse(current).ok()?;
    let dir = rollback_dir()?;
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let version = version_of(path.file_name()?.to_str()?)?;
            (version < current).then_some((version, path))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
}

/// Go back to the version before this one, after asking.
pub fn restore_previous(app: &AppHandle) {
    let current = app.package_info().version.to_string();
    let Some((version, installer)) = previous_installer(&current) else {
        let handle = app.clone();
        app.dialog()
            .message(format!(
                "There is no earlier version kept on this machine. Every version is on the \
                 releases page; installing one from there over {current} also works."
            ))
            .title("Restore the previous version")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Open the releases page".into(),
                "Close".into(),
            ))
            .show(move |open| {
                if open {
                    let _ = handle
                        .opener()
                        .open_url(format!("{REPOSITORY}/releases"), None::<&str>);
                }
            });
        return;
    };
    let config = crate::config_path().ok();
    let plan = config.as_deref().map_or(SettingsPlan::Keep, |path| {
        plan_settings(
            schema_of_file(path),
            schema_read_by(&version),
            &backups_beside(path),
        )
    });
    let handle = app.clone();
    let settings = match &plan {
        SettingsPlan::Keep => "Your settings are not touched.".to_owned(),
        SettingsPlan::Swap { backup, from } => format!(
            "{version} reads an older form of settings file than this version wrote, so the \
             copy kept when the settings were brought forward, {}, goes back in its place; \
             your settings as they are now are kept beside it as station.toml.newer-v{from}.",
            backup
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        ),
        SettingsPlan::Unreadable { current: at, reads } => {
            let handle = app.clone();
            app.dialog()
                .message(format!(
                    "{version} reads an older form of settings file (schema {reads}) than \
                     this version wrote (schema {at}), and no copy it can read was kept, so \
                     it could not start. Keep this version, or install {version} from the \
                     releases page and set it up again."
                ))
                .title("Restore the previous version")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Open the releases page".into(),
                    "Keep this version".into(),
                ))
                .show(move |open| {
                    if open {
                        let _ = handle
                            .opener()
                            .open_url(format!("{REPOSITORY}/releases"), None::<&str>);
                    }
                });
            return;
        }
    };
    app.dialog()
        .message(format!(
            "Go back from {current} to {version}? The modem will stop, {version} will \
             install over this version, and Aether HF will start again. {settings}"
        ))
        .title("Restore the previous version")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            format!("Install {version}"),
            "Keep this version".into(),
        ))
        .show(move |yes| {
            if yes {
                let swap = match (plan, config) {
                    (SettingsPlan::Swap { backup, from }, Some(config)) => {
                        Some((config, backup, from))
                    }
                    _ => None,
                };
                run_installer(&handle, &installer, swap);
            }
        });
}

/// Run a kept installer over this installation, and get out of its way — first putting
/// back the settings the earlier version can read, when it cannot read these.
fn run_installer(app: &AppHandle, installer: &Path, swap: Option<(PathBuf, PathBuf, u32)>) {
    crate::stop_daemon(&app.state::<crate::Daemon>());
    if let Err(error) = crate::release_daemon_binary(std::time::Duration::from_secs(15)) {
        report(app, format!("Could not go back: {error}"));
        return;
    }
    // the modem is stopped, so nothing writes the file under us
    if let Some((config, backup, from)) = swap
        && let Err(error) = swap_settings(&config, &backup, from)
    {
        report(
            app,
            format!("Could not put back the earlier settings: {error}"),
        );
        return;
    }
    if cfg!(windows) {
        // the installer replaces the files this process is running from, so this process
        // has to be gone; NSIS waits for it
        match std::process::Command::new(installer).arg("/S").spawn() {
            Ok(_) => app.exit(0),
            Err(error) => report(app, format!("Could not start the installer: {error}")),
        }
    } else {
        // an AppImage is one file: put the old one back where this one is, and restart
        match std::env::current_exe().and_then(|own| std::fs::copy(installer, own)) {
            Ok(_) => app.restart(),
            Err(error) => report(
                app,
                format!("Could not put the earlier version in place: {error}"),
            ),
        }
    }
}

/// Say what went wrong, in a box: for the paths that run without the window.
fn report(app: &AppHandle, text: String) {
    app.dialog()
        .message(text)
        .title("Aether HF updates")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stable_installation_is_never_offered_a_prerelease() {
        assert_eq!(endpoints(Channel::Stable).len(), 1);
        assert!(endpoints(Channel::Stable)[0].contains("/releases/latest/"));
        // and the others read the stable manifest too, so a newer stable is never missed
        assert!(
            endpoints(Channel::Beta)
                .iter()
                .any(|e| e.contains("/releases/latest/"))
        );
        assert!(
            endpoints(Channel::Nightly)
                .iter()
                .any(|e| e.contains("/releases/latest/"))
        );
    }

    #[test]
    fn versions_compare_as_semantic_versions() {
        assert!(newer("0.3.0", "0.2.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        // a prerelease is older than the release it precedes...
        assert!(!newer("0.3.0-beta.1", "0.3.0"));
        assert!(newer("0.3.0", "0.3.0-beta.1"));
        // ...which is why the nightly is stamped as the *next* patch version
        assert!(newer("0.2.1-nightly.20260914.abc1234", "0.2.0"));
        assert!(newer(
            "0.2.1-nightly.20260915.abc1234",
            "0.2.1-nightly.20260914.ffffff0"
        ));
        assert!(!newer("garbage", "0.2.0"));
    }

    #[test]
    fn a_kept_installer_says_its_version_in_its_name() {
        assert_eq!(
            version_of("aether-hf_0.2.0-setup.exe"),
            Some(semver::Version::new(0, 2, 0))
        );
        assert_eq!(
            version_of("aether-hf_0.3.0-beta.1.AppImage").map(|v| v.to_string()),
            Some("0.3.0-beta.1".to_owned())
        );
        assert_eq!(version_of("SHA256SUMS"), None);
        assert_eq!(
            version_of(&installer_name("0.2.0")),
            Some(semver::Version::new(0, 2, 0))
        );
    }

    #[test]
    fn every_release_is_known_by_the_settings_it_reads() {
        let reads = |v: &str| schema_read_by(&semver::Version::parse(v).expect("version"));
        assert_eq!(reads("0.2.0-beta.9"), Some(1));
        assert_eq!(reads("0.2.0-beta.50"), Some(1));
        assert_eq!(reads("0.2.0-beta.52"), Some(2));
        assert_eq!(reads("0.2.0-beta.53"), Some(3));
        assert_eq!(reads("0.2.0-beta.55"), Some(4));
        assert_eq!(reads("0.2.0-beta.56"), Some(5));
        assert_eq!(reads("0.2.0"), reads(env!("CARGO_PKG_VERSION")));
        assert_eq!(reads("0.1.0"), None);
        // the table's last line is the schema the daemon of this build writes: a schema bump
        // without a line here fails
        let config = include_str!("../../../core/aetherd/src/config.rs");
        let current: u32 = config
            .lines()
            .find_map(|line| line.strip_prefix("pub const SCHEMA_VERSION: u32 = "))
            .and_then(|rest| rest.trim_end_matches(';').parse().ok())
            .expect("the daemon's schema");
        assert_eq!(SCHEMA_HISTORY.last().map(|(_, s)| *s), Some(current));
        assert_eq!(reads(env!("CARGO_PKG_VERSION")), Some(current));
    }

    #[test]
    fn going_back_puts_back_the_settings_the_earlier_version_reads() {
        let kept = |n: u32| (n, PathBuf::from(format!("station.toml.bak-v{n}")));
        let backups = [kept(1), kept(2), kept(4)];
        assert_eq!(plan_settings(5, Some(5), &backups), SettingsPlan::Keep);
        assert_eq!(plan_settings(4, Some(5), &backups), SettingsPlan::Keep);
        assert_eq!(plan_settings(5, None, &backups), SettingsPlan::Keep);
        // beta.56 to beta.52: the copy beta.52 wrote goes back
        assert_eq!(
            plan_settings(5, Some(2), &backups),
            SettingsPlan::Swap {
                backup: PathBuf::from("station.toml.bak-v2"),
                from: 5
            }
        );
        // to a version that reads 3, the newest it can read — 2 — brought forward by it
        assert_eq!(
            plan_settings(5, Some(3), &backups),
            SettingsPlan::Swap {
                backup: PathBuf::from("station.toml.bak-v2"),
                from: 5
            }
        );
        assert_eq!(
            plan_settings(5, Some(2), &[kept(4)]),
            SettingsPlan::Unreadable {
                current: 5,
                reads: 2
            }
        );
        // and the swap keeps the newer file beside the one put back
        let dir = std::env::temp_dir().join(format!("aether-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let config = dir.join("station.toml");
        std::fs::write(&config, "schema_version = 5\n").expect("write");
        std::fs::write(dir.join("station.toml.bak-v2"), "schema_version = 2\n").expect("write");
        assert_eq!(schema_of_file(&config), 5);
        let found = backups_beside(&config);
        assert_eq!(found.len(), 1);
        swap_settings(&config, &found[0].1, 5).expect("swap");
        assert_eq!(schema_of_file(&config), 2);
        assert_eq!(
            std::fs::read_to_string(dir.join("station.toml.newer-v5")).expect("kept"),
            "schema_version = 5\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preferences_come_from_the_update_section_and_default_sensibly() {
        let dir = std::env::temp_dir().join(format!("aether-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("station.toml");

        std::fs::write(&path, "callsign = \"W4ODA\"\n").expect("write");
        let p = preferences(&path);
        assert_eq!(p.channel, channel_for_build(env!("CARGO_PKG_VERSION")));
        assert!(p.check);
        // a file that names no channel follows the build: a beta the betas
        assert_eq!(
            preferences_for(&path, "0.2.0-beta.56").channel,
            Channel::Beta
        );
        assert_eq!(preferences_for(&path, "0.2.0").channel, Channel::Stable);
        // a beta's "stable" from before the daemon's migration reads as the migration will
        // leave it; from schema 5 on it is a choice, and kept
        std::fs::write(
            &path,
            "schema_version = 4\ncallsign = \"W4ODA\"\n[update]\nchannel = \"stable\"\n",
        )
        .expect("write");
        assert_eq!(
            preferences_for(&path, "0.2.0-beta.56").channel,
            Channel::Beta
        );
        assert_eq!(preferences_for(&path, "0.2.0").channel, Channel::Stable);
        std::fs::write(
            &path,
            "schema_version = 5\ncallsign = \"W4ODA\"\n[update]\nchannel = \"stable\"\n",
        )
        .expect("write");
        assert_eq!(
            preferences_for(&path, "0.2.0-beta.56").channel,
            Channel::Stable
        );

        std::fs::write(
            &path,
            "callsign = \"W4ODA\"\n[update]\nchannel = \"beta\"\ncheck = false\n",
        )
        .expect("write");
        let p = preferences(&path);
        assert_eq!(p.channel, Channel::Beta);
        assert!(!p.check);

        let p = preferences_for(&dir.join("missing.toml"), "0.2.0");
        assert_eq!(p.channel, Channel::Stable);
        let p = preferences_for(&dir.join("missing.toml"), "0.2.0-beta.56");
        assert_eq!(p.channel, Channel::Beta);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_next_start_reads_the_note_the_install_left() {
        let note = Note {
            installing: "0.2.0-beta.14".into(),
            from: "0.2.0-beta.13".into(),
            notes: "## What's new\n- a dashboard\n".into(),
            at: 1_000_000,
        };
        // the version now running is the one installed: complete, with its notes
        match verdict(&note, "0.2.0-beta.14", 1_000_100) {
            Some(Phase::Complete {
                version,
                notes,
                from,
            }) => {
                assert_eq!(version, "0.2.0-beta.14");
                assert_eq!(from, "0.2.0-beta.13");
                assert!(notes.starts_with("## What's new"));
            }
            other => panic!("{other:?}"),
        }
        // still the old version: the install did not happen, and the operator hears so
        match verdict(&note, "0.2.0-beta.13", 1_000_100) {
            Some(Phase::Incomplete { version }) => assert_eq!(version, "0.2.0-beta.14"),
            other => panic!("{other:?}"),
        }
        // some third version, or a note from last week: nothing to say
        assert!(verdict(&note, "0.3.0", 1_000_100).is_none());
        assert!(verdict(&note, "0.2.0-beta.14", 1_000_000 + 2 * 24 * 3600).is_none());
    }

    #[test]
    fn the_view_serialises_flat_for_the_window() {
        let view = View {
            phase: Phase::Downloading {
                version: "0.3.0".into(),
                downloaded: 10,
                total: Some(100),
            },
            current: "0.2.0".into(),
            channel: "beta",
            can_restore: false,
        };
        let json = serde_json::to_value(&view).expect("json");
        assert_eq!(json["phase"], "downloading");
        assert_eq!(json["version"], "0.3.0");
        assert_eq!(json["downloaded"], 10);
        assert_eq!(json["current"], "0.2.0");
        assert_eq!(json["channel"], "beta");
        let json = serde_json::to_value(View {
            phase: Phase::RestartRequired {
                version: "0.3.0".into(),
            },
            current: "0.2.0".into(),
            channel: "stable",
            can_restore: true,
        })
        .expect("json");
        assert_eq!(json["phase"], "restart-required");
        assert_eq!(json["can_restore"], true);
    }
}
