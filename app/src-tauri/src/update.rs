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

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager as _};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt as _;
use tauri_plugin_updater::{Update, UpdaterExt as _};

/// Where the releases are.
pub const REPOSITORY: &str = "https://github.com/KK4ODA/aether-hf";

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

/// What the station's configuration says about updates.
#[derive(Debug, Clone, Copy)]
pub struct Preferences {
    /// Which channel to follow.
    pub channel: Channel,
    /// Whether to look at all on start.
    pub check: bool,
}

/// Read the `[update]` section of the daemon's configuration file.
///
/// The daemon owns the file and the panel edits it; the shell only reads it, once, on the
/// way up. A file that cannot be read means the defaults: stable, and do check.
pub fn preferences(config: &Path) -> Preferences {
    let text = std::fs::read_to_string(config).unwrap_or_default();
    let table: toml::Table = toml::from_str(&text).unwrap_or_default();
    let update = table.get("update").and_then(toml::Value::as_table);
    let channel = match update
        .and_then(|u| u.get("channel"))
        .and_then(toml::Value::as_str)
    {
        Some("beta") => Channel::Beta,
        Some("nightly") => Channel::Nightly,
        _ => Channel::Stable,
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

/// Look for a newer version on the channel and, if there is one, ask.
///
/// `quiet` is the start-up check: nothing newer means nothing said. From the menu, nothing
/// newer is an answer worth giving.
pub async fn offer(app: AppHandle, channel: Channel, quiet: bool) {
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
        if !quiet {
            let text = if failures.is_empty() {
                format!(
                    "You have the newest version, {}.",
                    app.package_info().version
                )
            } else {
                format!("Could not check for updates:\n\n{}", failures.join("\n"))
            };
            app.dialog()
                .message(text)
                .title("Aether HF updates")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
        }
        return;
    };

    let notes = update
        .body
        .as_deref()
        .map_or(String::new(), |body| format!("\n\n{}", body.trim()));
    let text = format!(
        "Aether HF {} is available; you have {}.{notes}\n\nInstall it now? The modem will \
         stop, the update will install, and Aether HF will start again. The version you \
         have now is kept, and Help > Restore the previous version brings it back.",
        update.version, update.current_version
    );
    let handle = app.clone();
    app.dialog()
        .message(text)
        .title("A newer Aether HF")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install and restart".into(),
            "Not now".into(),
        ))
        .show(move |yes| {
            if yes {
                tauri::async_runtime::spawn(install(handle, update));
            }
        });
}

/// Download, keep a copy, stop the modem, install, restart.
async fn install(app: AppHandle, update: Update) {
    let bytes = match update.download(|_, _| {}, || {}).await {
        Ok(bytes) => bytes,
        Err(error) => {
            report(&app, format!("The download did not complete: {error}"));
            return;
        }
    };
    // Keep what is being installed, so the *next* update can be undone to this one — and
    // keep it before installing, when the bytes are certainly still here.
    if let Some(dir) = rollback_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(installer_name(&update.version)), &bytes);
    }
    // The daemon's binary is about to be replaced, and a file in use cannot be. Stopping it
    // here also releases the transmitter properly, which a replaced binary would not.
    crate::stop_daemon(&app.state::<crate::Daemon>());
    if let Err(error) = update.install(bytes) {
        report(&app, format!("The update did not install: {error}"));
        return;
    }
    app.restart();
}

/// Say what went wrong, in a box.
fn report(app: &AppHandle, text: String) {
    app.dialog()
        .message(text)
        .title("Aether HF updates")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

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
    let handle = app.clone();
    app.dialog()
        .message(format!(
            "Go back from {current} to {version}? The modem will stop, {version} will \
             install over this version, and Aether HF will start again. Your settings are \
             not touched."
        ))
        .title("Restore the previous version")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            format!("Install {version}"),
            "Keep this version".into(),
        ))
        .show(move |yes| {
            if yes {
                run_installer(&handle, &installer);
            }
        });
}

/// Run a kept installer over this installation, and get out of its way.
fn run_installer(app: &AppHandle, installer: &Path) {
    crate::stop_daemon(&app.state::<crate::Daemon>());
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
    fn preferences_come_from_the_update_section_and_default_sensibly() {
        let dir = std::env::temp_dir().join(format!("aether-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("station.toml");

        std::fs::write(&path, "callsign = \"W4ODA\"\n").expect("write");
        let p = preferences(&path);
        assert_eq!(p.channel, Channel::Stable);
        assert!(p.check);

        std::fs::write(
            &path,
            "callsign = \"W4ODA\"\n[update]\nchannel = \"beta\"\ncheck = false\n",
        )
        .expect("write");
        let p = preferences(&path);
        assert_eq!(p.channel, Channel::Beta);
        assert!(!p.check);

        let p = preferences(&dir.join("missing.toml"));
        assert_eq!(p.channel, Channel::Stable);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
