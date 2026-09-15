//! The Aether HF desktop shell.
//!
//! A window around the station panel, and a supervisor for the daemon underneath it. The
//! panel itself is in `app/ui` and is the same files `aetherd` serves to a browser
//! (ADR-0001 §3, ADR-0005); this shell exists so a desktop user has one thing to install and
//! one thing to double-click, rather than a daemon to start and a browser to point at it.
//!
//! # What it does with the daemon
//!
//! It starts `aetherd` as a child process, waits for the control port to answer, and then
//! loads the panel from it. On the way out it asks the daemon to stop and waits for it —
//! which is the whole reason the shell supervises rather than merely launches. `aetherd`
//! releases the transmitter when it is asked to stop, and a shell that closed its window
//! while leaving an orphan behind could leave a radio keyed with nothing on screen to say so.
//!
//! If a daemon is already listening, the shell attaches to that one instead of starting a
//! second. Two modems on one sound card is not a situation to invent behaviour for.
//!
//! While the window is open the shell watches the daemon it started. A daemon that exits
//! asking to be started again (status 75, which is how the panel applies a setting that
//! needs a restart) is started again on the file it just wrote; one that stops for any other
//! reason is explained on screen, because the panel would otherwise sit at "not connected"
//! with the reason written in a log nobody is looking at.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod update;

use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::{
    Manager as _,
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogKind};
use tauri_plugin_opener::OpenerExt as _;

/// Where the daemon listens by default. The shell does not currently offer to change it;
/// an operator who has moved it can run the daemon themselves and the shell will attach.
const CONTROL: &str = "127.0.0.1:8515";

/// How long to wait for a freshly started daemon to answer before giving up on it.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// How long to let the daemon finish releasing the radio before killing it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The exit status with which the daemon asks to be started again (`RESTART_EXIT_CODE` in
/// `aetherd`; `EX_TEMPFAIL`). Any other exit is a stop, or a failure.
const RESTART_EXIT_CODE: i32 = 75;

/// How often the supervisor looks at the daemon it started.
const WATCH_INTERVAL: Duration = Duration::from_millis(250);

/// The daemon this shell started, if it started one.
pub struct Daemon(Mutex<Option<Child>>);

/// What the daemon needs to be started, kept so it can be started again.
struct Launch {
    resources: Option<PathBuf>,
}

/// Why the daemon could not be started, if it could not.
struct StartupError(Option<String>);

fn main() {
    let context = tauri::generate_context!();
    // Where the bundler put the panel: beside the binary on Windows, under /usr/lib on a
    // Debian package, inside the mounted image for an AppImage. Tauri's own resolver knows
    // every layout it produces, which is the reason to ask it rather than guess.
    let resources =
        tauri::utils::platform::resource_dir(context.package_info(), &tauri::Env::default()).ok();
    let launch = Launch {
        resources: resources.clone(),
    };
    let (started, failure) = match ensure_daemon(resources.as_deref()) {
        Ok(child) => (child, None),
        Err(message) => {
            eprintln!("aether: {message}");
            (None, Some(message))
        }
    };
    let preferences = config_path().map_or(
        update::Preferences {
            channel: update::Channel::Stable,
            check: false,
        },
        |path| update::preferences(&path),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Daemon(Mutex::new(started)))
        .manage(StartupError(failure))
        .manage(launch)
        .setup(move |app| {
            install_menu(app, preferences.channel)?;
            watch_daemon(app.handle().clone());
            // A packaged build has no terminal, so a daemon that would not start has to be
            // explained on screen: the panel would otherwise sit at "not connected" for ever,
            // with the reason — a sound card at the wrong rate, a serial port that is held —
            // written somewhere nobody is looking.
            if let Some(message) = &app.state::<StartupError>().0 {
                app.dialog()
                    .message(format!(
                        "{message}\n\nIf this began after an update, Help > Restore the \
                         previous version goes back."
                    ))
                    .title("Aether HF could not start the modem")
                    .kind(MessageDialogKind::Error)
                    .show(|_| {});
            } else if preferences.check {
                // quietly: a start with nothing newer should look like nothing happened
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(update::offer(handle, preferences.channel, true));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                stop_daemon(&window.state::<Daemon>());
            }
        })
        .run(context)
        .expect("the desktop shell could not start");
}

/// The Help menu: updates, going back, and where things are.
fn install_menu(app: &tauri::App, channel: update::Channel) -> tauri::Result<()> {
    let check = MenuItemBuilder::with_id("check-updates", "Check for updates…").build(app)?;
    let restore =
        MenuItemBuilder::with_id("restore-previous", "Restore the previous version…").build(app)?;
    let logs = MenuItemBuilder::with_id("open-logs", "Open the configuration folder").build(app)?;
    let releases = MenuItemBuilder::with_id("open-releases", "Releases on GitHub").build(app)?;
    let help = SubmenuBuilder::new(app, "Help")
        .item(&check)
        .item(&restore)
        .separator()
        .item(&logs)
        .item(&releases)
        .build()?;
    let menu = MenuBuilder::new(app).item(&help).build()?;
    app.set_menu(menu)?;
    app.on_menu_event(move |app, event| match event.id().as_ref() {
        "check-updates" => {
            tauri::async_runtime::spawn(update::offer(app.clone(), channel, false));
        }
        "restore-previous" => update::restore_previous(app),
        "open-logs" => {
            if let Ok(config) = config_path()
                && let Some(dir) = config.parent()
            {
                let _ = app
                    .opener()
                    .open_path(dir.display().to_string(), None::<&str>);
            }
        }
        "open-releases" => {
            let _ = app
                .opener()
                .open_url(format!("{}/releases", update::REPOSITORY), None::<&str>);
        }
        _ => {}
    });
    Ok(())
}

/// Start the daemon unless one is already listening.
fn ensure_daemon(resources: Option<&std::path::Path>) -> Result<Option<Child>, String> {
    if reachable() {
        // Somebody is already running one — a service, or a terminal. Attaching is right:
        // two modems on one sound card is not a situation to invent behaviour for.
        return Ok(None);
    }

    let binary = daemon_path()?;
    let config = config_path()?;
    let mut command = Command::new(&binary);
    command.arg("--config").arg(&config);
    // so the daemon can tell the panel a restart is something it can do for the operator
    command.env("AETHERD_SUPERVISED", "1");
    // The daemon is a console program, and Windows gives a console program started from a
    // windowed one a console of its own: an empty black window beside the panel, since its
    // output goes to the log file below. CREATE_NO_WINDOW keeps it a background process.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    if let Some(ui) = ui_dir(resources) {
        command.env("AETHER_UI_DIR", ui);
    }
    // The daemon's output goes to a file, not a pipe: a pipe nobody drains would stall the
    // daemon once it filled, and a file is also the only record a packaged build keeps of
    // what the daemon said. Truncated on every start so it describes this run.
    let log = daemon_log_path(&config);
    if let Ok(file) = std::fs::File::create(&log) {
        if let Ok(errors) = file.try_clone() {
            command.stderr(errors);
        }
        command.stdout(file);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if reachable() {
            return Ok(Some(child));
        }
        if let Ok(Some(status)) = child.try_wait() {
            // it gave up before it listened; what it said on the way out is the reason
            let said = tail_of(&log, 12);
            return Err(if said.is_empty() {
                format!("the modem daemon stopped ({status}) without saying why")
            } else {
                format!(
                    "The modem daemon stopped:\n\n{said}\n\nFix the setting it names, then start Aether HF again. The configuration is {}.",
                    config.display()
                )
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "{} started but never answered on {CONTROL}",
        binary.display()
    ))
}

/// Keep an eye on the daemon this shell started, for as long as the window is open.
///
/// Started again when it asks to be; explained when it stops on its own. A daemon that
/// `stop_daemon` took out of the slot is not watched, so closing the window never starts
/// one. The child is polled rather than waited on: a thread blocked in `wait()` would hold
/// the slot's lock against `stop_daemon`.
fn watch_daemon(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("aether-daemon-watch".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(WATCH_INTERVAL);
                let exited = {
                    let daemon = app.state::<Daemon>();
                    let Ok(mut slot) = daemon.0.lock() else {
                        return;
                    };
                    let Some(child) = slot.as_mut() else { continue };
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *slot = None;
                            status
                        }
                        Ok(None) => continue,
                        Err(_) => return,
                    }
                };
                if exited.code() == Some(RESTART_EXIT_CODE) {
                    let resources = app.state::<Launch>().resources.clone();
                    match ensure_daemon(resources.as_deref()) {
                        Ok(child) => {
                            if let Ok(mut slot) = app.state::<Daemon>().0.lock() {
                                *slot = child;
                            }
                        }
                        Err(message) => explain(&app, &message),
                    }
                } else {
                    let said = config_path()
                        .map(|config| tail_of(&daemon_log_path(&config), 12))
                        .unwrap_or_default();
                    explain(
                        &app,
                        &if said.is_empty() {
                            format!("The modem daemon stopped ({exited}) without saying why.")
                        } else {
                            format!("The modem daemon stopped ({exited}):\n\n{said}")
                        },
                    );
                }
            }
        })
        .ok();
}

/// A daemon that stopped, or would not start again, explained on screen.
fn explain(app: &tauri::AppHandle, message: &str) {
    app.dialog()
        .message(format!(
            "{message}\n\nFix the setting it names if it names one, then start Aether HF \
             again. Help > Open the configuration folder has the log."
        ))
        .title("Aether HF lost the modem")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

/// Where the daemon's own output is kept: beside the configuration, which is the one place
/// an operator already knows to look.
fn daemon_log_path(config: &std::path::Path) -> PathBuf {
    config.parent().map_or_else(
        || PathBuf::from("aetherd.log"),
        |dir| dir.join("aetherd.log"),
    )
}

/// The last few lines of a file, for showing a person.
fn tail_of(path: &std::path::Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n").trim().to_owned()
}

fn reachable() -> bool {
    CONTROL.parse().is_ok_and(|address| {
        TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok()
    })
}

/// Where the daemon binary is: beside this one, which is where the bundler's sidecar lands
/// on every platform, and failing that in the workspace's build output, which is where it
/// is during development.
fn daemon_path() -> Result<PathBuf, String> {
    let name = if cfg!(windows) {
        "aetherd.exe"
    } else {
        "aetherd"
    };
    let mut tried = Vec::new();
    if let Ok(own) = std::env::current_exe()
        && let Some(dir) = own.parent()
    {
        let beside = dir.join(name);
        if beside.is_file() {
            return Ok(beside);
        }
        tried.push(beside);
        // the workspace's own build output, from app/src-tauri/target/<profile>/
        let checkout = dir.join("../../../../core/target/release").join(name);
        if checkout.is_file() {
            return Ok(checkout);
        }
        tried.push(checkout);
    }
    Err(format!(
        "cannot find the modem daemon. Looked in: {}",
        tried
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Where the station panel is: in the package's resources, beside this binary, or `app/ui`
/// in a checkout, in that order.
fn ui_dir(resources: Option<&std::path::Path>) -> Option<PathBuf> {
    let own = std::env::current_exe().ok()?;
    let dir = own.parent()?;
    let mut candidates = Vec::new();
    if let Some(resources) = resources {
        candidates.push(resources.join("ui"));
    }
    candidates.push(dir.join("ui"));
    candidates.push(dir.join("../../../ui"));
    candidates
        .into_iter()
        .find(|candidate| candidate.join("index.html").is_file())
        .and_then(|found| found.canonicalize().ok())
}

/// Where the operator's configuration lives.
fn config_path() -> Result<PathBuf, String> {
    if let Ok(from_environment) = std::env::var("AETHER_CONFIG") {
        return Ok(PathBuf::from(from_environment));
    }
    let base = dirs_config().ok_or("cannot find a configuration directory for this user")?;
    let dir = base.join("aether-hf");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = dir.join("station.toml");
    if !path.exists() {
        // A first run has nothing to start from, and a daemon that refuses to start because
        // its configuration is missing is a poor welcome. This one starts receive-only with
        // no keying, which is exactly the right thing to hand somebody who has not set up yet.
        std::fs::write(&path, FIRST_RUN)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(path)
}

/// What a first run gets: a station that listens and keys nothing.
const FIRST_RUN: &str = "\
# Written on first run. Nothing here transmits: the callsign is a placeholder and there is no
# keying, so this station listens and does nothing else until you change it in Setup.
callsign = \"N0CALL\"

[ptt]
kind = \"none\"

[radio]
max_key_s = 30.0
wait_for_clear = true
";

fn dirs_config() -> Option<PathBuf> {
    // Two environment variables rather than a dependency: this is the whole of what a
    // directories crate would be used for here.
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }
}

/// Ask the daemon to stop, and wait for it.
///
/// It releases the transmitter when it is asked to stop. A shell that closed its window and
/// left an orphan behind could leave a radio keyed with nothing on screen to say so, which is
/// the worst failure this software has.
pub fn stop_daemon(state: &tauri::State<'_, Daemon>) {
    let Ok(mut slot) = state.0.lock() else { return };
    let Some(mut child) = slot.take() else { return };

    // Ask over the control API rather than killing: a killed process never runs its
    // shutdown path, and with `rigctld` keying that is a rig left in transmit with nothing
    // on screen to say so. A serial line would drop when the port closed; a network one
    // would not. The request goes over loopback, which needs no token.
    ask_to_stop();
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        match child.try_wait() {
            // exited, or the handle is gone: either way there is nothing left to wait for
            Ok(Some(_)) | Err(_) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    // It did not stop when asked. Killing it now is the lesser evil: leaving an orphan
    // behind would keep the sound card and the ports, and the next start would fail.
    let _ = child.kill();
    let _ = child.wait();
}

/// Make sure no daemon is left holding its binary before an installer replaces it.
///
/// `stop_daemon` handles the one this shell started. This handles the rest: a daemon the
/// shell attached to rather than started, one whose process is still winding down after it
/// answered the stop, and a copy left behind by something else — any of them keeps
/// `aetherd.exe` open, and an installer that meets an open file stops with a dialog nobody
/// wants to see ("Error opening file for writing", seen on two updates in a row). The test
/// is the one the installer applies: can the file be opened for writing.
///
/// # Errors
/// If, after asking, waiting and finally stopping a stray daemon by force, the file is
/// still held — with what to do about it.
pub fn release_daemon_binary(patience: Duration) -> Result<(), String> {
    let binary = daemon_path()?;
    let writable =
        |path: &std::path::Path| std::fs::OpenOptions::new().write(true).open(path).is_ok();
    let deadline = Instant::now() + patience;
    let mut asked = false;
    let mut forced = false;
    while Instant::now() < deadline {
        if !reachable() && writable(&binary) {
            return Ok(());
        }
        if !asked {
            // a daemon this shell does not own is still asked politely, so it releases the
            // transmitter on its way out
            ask_to_stop();
            asked = true;
        } else if !forced && Instant::now() + Duration::from_secs(3) > deadline {
            forced = true;
            stop_stray_daemons(&binary);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(format!(
        "{} is still in use, so the installer cannot replace it. Close every copy of Aether \
         HF (the Task Manager lists aetherd.exe), then run the update again.",
        binary.display()
    ))
}

/// Stop any `aetherd` process running from this installation, by force: the last resort
/// before an installer that would fail anyway. Matched by path, so a daemon installed
/// elsewhere — a gateway's — is left alone.
#[cfg(windows)]
fn stop_stray_daemons(binary: &std::path::Path) {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let script = format!(
        "Get-Process aetherd -ErrorAction SilentlyContinue | Where-Object {{ $_.Path -eq '{}' }} \
         | Stop-Process -Force",
        binary.display().to_string().replace('\'', "''")
    );
    let _ = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(not(windows))]
fn stop_stray_daemons(binary: &std::path::Path) {
    let _ = Command::new("pkill")
        .args(["-f", &binary.display().to_string()])
        .status();
}

/// `POST /v1/shutdown`, hand-written so the shell carries no HTTP client of its own.
fn ask_to_stop() {
    use std::io::Write as _;
    let Ok(address) = CONTROL.parse() else { return };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(500)) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let request = concat!(
        "POST /v1/shutdown HTTP/1.1
",
        "Host: 127.0.0.1
",
        "Content-Length: 2
",
        "Connection: close
",
        "
",
        "{}",
    );
    let _ = stream.write_all(request.as_bytes());
}
