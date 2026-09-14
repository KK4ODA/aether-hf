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

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command},
    sync::Mutex,
    time::{Duration, Instant},
};

use tauri::Manager as _;

/// Where the daemon listens by default. The shell does not currently offer to change it;
/// an operator who has moved it can run the daemon themselves and the shell will attach.
const CONTROL: &str = "127.0.0.1:8515";

/// How long to wait for a freshly started daemon to answer before giving up on it.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

/// How long to let the daemon finish releasing the radio before killing it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The daemon this shell started, if it started one.
struct Daemon(Mutex<Option<Child>>);

fn main() {
    let started = match ensure_daemon() {
        Ok(child) => child,
        Err(message) => {
            // Nothing to show it in yet, so the terminal is the only place it can go. A
            // packaged build has no terminal, which is why the panel also renders its own
            // "not connected" state and keeps retrying.
            eprintln!("aether: {message}");
            None
        }
    };

    tauri::Builder::default()
        .manage(Daemon(Mutex::new(started)))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                stop_daemon(&window.state::<Daemon>());
            }
        })
        .run(tauri::generate_context!())
        .expect("the desktop shell could not start");
}

/// Start the daemon unless one is already listening.
fn ensure_daemon() -> Result<Option<Child>, String> {
    if reachable() {
        // Somebody is already running one — a service, or a terminal. Attaching is right:
        // two modems on one sound card is not a situation to invent behaviour for.
        return Ok(None);
    }

    let binary = daemon_path()?;
    let config = config_path()?;
    let mut command = Command::new(&binary);
    command.arg("--config").arg(&config);
    if let Some(ui) = ui_dir() {
        command.env("AETHER_UI_DIR", ui);
    }
    let child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if reachable() {
            return Ok(Some(child));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "{} started but never answered on {CONTROL}",
        binary.display()
    ))
}

fn reachable() -> bool {
    CONTROL.parse().is_ok_and(|address| {
        TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok()
    })
}

/// Where the daemon binary is: beside this one, which is how it is packaged, and failing
/// that in the workspace's build output, which is where it is during development.
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

/// Where the station panel is: beside this binary in a package, or `app/ui` in a checkout.
fn ui_dir() -> Option<PathBuf> {
    let own = std::env::current_exe().ok()?;
    let dir = own.parent()?;
    for candidate in [dir.join("ui"), dir.join("../../../ui")] {
        if candidate.join("index.html").is_file() {
            return candidate.canonicalize().ok();
        }
    }
    None
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
fn stop_daemon(state: &tauri::State<'_, Daemon>) {
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
