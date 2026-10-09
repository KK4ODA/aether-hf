//! What each control method does, as a pure function of a request and a station.
//!
//! Kept separate from the socket so the whole method surface can be tested by calling it,
//! which is also how the spec's claims about it are checked: that `disconnect` is orderly and
//! `abort` is not, that `capabilities` describes the mode table instead of a client
//! hard-coding it, that an unknown method is refused rather than ignored.

use base64_lite::{decode, encode};
use serde_json::{Value, json};

use crate::{
    control::protocol::{ApiError, Request, Response},
    ptt::Ptt,
    station::Station,
};

/// The smallest base64 there is, because pulling a crate in for forty lines of table lookup
/// is not a trade worth making.
mod base64_lite {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    /// Standard base64 with padding.
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let value = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for index in 0..4 {
                if index <= chunk.len() {
                    let shift = 18 - 6 * index;
                    out.push(ALPHABET[((value >> shift) & 0x3F) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// Standard base64, tolerating missing padding and whitespace.
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let mut bits: u32 = 0;
        let mut held = 0;
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        for character in text.bytes() {
            if character == b'=' || character.is_ascii_whitespace() {
                continue;
            }
            let value = ALPHABET.iter().position(|&c| c == character)? as u32;
            bits = (bits << 6) | value;
            held += 6;
            if held >= 8 {
                held -= 8;
                out.push(((bits >> held) & 0xFF) as u8);
            }
        }
        Some(out)
    }
}

/// Whether a method changes anything, which is what a read-only client is limited to.
#[must_use]
pub fn is_mutating(method: &str) -> bool {
    matches!(
        method,
        "connect"
            | "disconnect"
            | "abort"
            | "send"
            | "listen"
            | "callsigns.set"
            | "beacon"
            | "beacon.every"
            | "probe"
            | "test.start"
            | "test.abort"
            | "tune"
            | "drive.set"
            | "record.start"
            | "record.stop"
            | "record.notes"
            | "shutdown"
            | "config.set"
            | "ptt.test"
            | "heard.clear"
            | "sessions.clear"
            | "counters.reset"
            | "frequencies.set"
            | "frequency.set"
    ) || crate::control::profiles::is_mutating(method)
}

/// What the daemon knows that the modem does not: its configuration and where it came
/// from, its log, and how the audio has been behaving.
///
/// Held beside the station so `config.get`, `config.set` and `diagnostics` can reach it
/// without the modem having to know what a configuration file or a log is.
pub struct DaemonState {
    /// The current settings.
    pub config: crate::config::Config,
    /// Where they are written back to.
    pub path: std::path::PathBuf,
    /// The log, with its ring of recent entries.
    pub log: crate::log::Log,
    /// When the daemon started, for the bundle's own clock.
    pub started: std::time::SystemTime,
    /// What the audio is running on, as the sound card described itself.
    pub audio: String,
    /// Captured samples dropped because the modem fell behind, since the start.
    pub dropped_audio: u64,
    /// Samples of silence the sound card played inside a transmission because the modem
    /// had not handed it the next ones in time, since the start: holes on the air.
    pub starved_audio: u64,
    /// The slowest pass of the run loop so far, in milliseconds, and what it was doing.
    pub loop_slowest_ms: f64,
    /// Which phase the slowest pass spent its time in: `commands`, `capture`, `playback`.
    pub loop_slowest_phase: String,
    /// Passes of the run loop that took longer than the quarter second the sound card
    /// used to be kept ahead by — the stalls that put holes in bursts before a whole
    /// burst was queued at once.
    pub loop_stalls: u64,
    /// The latest the run loop's heartbeat — a thread that only sleeps — woke during a slow
    /// pass, in milliseconds: near the pass's own time, the machine was starving every
    /// thread; near nothing, the time was the modem's.
    pub loop_machine_late_ms: f64,
    /// Why the sound card could not be opened, when it could not. The station runs on
    /// silence until the devices are corrected, and the panel says so.
    pub audio_fault: Option<String>,
    /// How the configuration had to be read to start: from the copy kept before a newer
    /// version brought it forward, when that version's file is one this cannot read.
    pub config_note: Option<String>,
    /// How the machine's audio devices and serial ports are listed.
    ///
    /// A function rather than a call, because enumerating devices goes through the
    /// platform's audio API, and on a machine with no audio service at all — a CI runner —
    /// that has been seen to crash the process rather than return an error. The tests
    /// substitute a list; the daemon uses [`device_inventory`], on a thread of its own
    /// ([`Self::device_list`]).
    pub devices: fn() -> Value,
    /// The last listing of the devices, made off the run loop: read by everything that
    /// checks against them, refreshed at start and whenever a client asks for the list.
    pub device_list: crate::devices::DeviceList,
    /// How many listings had finished when the clients were last given one: the run loop
    /// publishes a `devices` event when another finishes with a different list.
    pub devices_told: u64,
    /// The list the clients were last given.
    pub devices_last_told: Option<Value>,
    /// Whether a supervisor — the desktop shell, systemd — will start this daemon again
    /// when it exits asking to be restarted. `AETHERD_SUPERVISED=1` in the environment says
    /// so; a daemon run from a terminal has nobody to do it, and the panel must not offer
    /// what would only stop the modem.
    pub supervised: bool,
    /// The stations heard, kept in a file beside the configuration.
    pub heard: crate::heard::HeardList,
    /// The sessions this station has had, kept beside it.
    pub sessions: crate::sessions::SessionLog,
    /// The remembered dials, kept beside it too.
    pub memories: crate::memories::Memories,
    /// The host interface, when one is listening.
    pub host: Option<HostStatus>,
    /// What the host interface knows that the KISS port obeys: a host attached, `CHAT ON`,
    /// `IGNOREKISSDCD ON` (ADR-0019). Shared by the two servers.
    pub host_flags: crate::kiss::HostFlags,
    /// The KISS port, when it is listening.
    pub kiss: Option<crate::kiss::KissServer>,
    /// Why the KISS port is not listening when it should be.
    pub kiss_error: Option<String>,
    /// The KISS settings last tried: a change starts the port again.
    pub kiss_tried: Option<crate::kiss::KissConfig>,
    /// When a port that would not open is tried again — whatever held it (VARA, a soundmodem
    /// on 8100) may have gone, and saving the same settings again changes nothing.
    pub kiss_retry_at: Option<std::time::Instant>,
    /// A handle on the control API, for the servers the daemon starts while it runs.
    pub control_handle: Option<crate::control::ControlHandle>,
    /// The profiles beside the configuration, and which one is active.
    pub profiles: crate::profile::Store,
    /// Whether the settings or the profiles changed since the run loop last told the
    /// clients: set by the methods, taken by the loop, which publishes a `profile` event.
    profiles_changed: bool,
    /// The shared zip on its way to the station that asked for it (`share.upload`).
    pub upload: crate::upload::Upload,
    /// How the upload reaches the network: a function, so the tests can stand in for Google.
    pub upload_http: fn() -> Box<dyn crate::upload::Http>,
    /// Debug mode's queue: the host program's sessions on their way to the project.
    pub debug: crate::debug::DebugUploads,
    /// The audio is a simulated channel (`[sim]`, `--channel`) or a dry run: its sessions are
    /// a bench's, never sent to the project — CI and the harness would fill its folder.
    pub simulated: bool,
}

/// The network, for an upload.
fn web() -> Box<dyn crate::upload::Http> {
    Box::new(crate::upload::Web::default())
}

/// The host (VARA-compatible) interface, as `status` reports it.
#[derive(Debug, Clone)]
pub struct HostStatus {
    /// Where the command port listens.
    pub command_address: String,
    /// Where the data port listens.
    pub data_address: String,
    /// Whether a host program is connected right now.
    pub connected: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl DaemonState {
    /// A state for a daemon running from a file.
    #[must_use]
    pub fn new(
        config: crate::config::Config,
        path: std::path::PathBuf,
        log: crate::log::Log,
    ) -> Self {
        Self {
            config,
            log,
            started: std::time::SystemTime::now(),
            audio: String::new(),
            dropped_audio: 0,
            starved_audio: 0,
            loop_slowest_ms: 0.0,
            loop_slowest_phase: String::new(),
            loop_stalls: 0,
            loop_machine_late_ms: 0.0,
            audio_fault: None,
            config_note: None,
            devices: device_inventory,
            upload: crate::upload::Upload::default(),
            upload_http: web,
            debug: crate::debug::DebugUploads::default(),
            simulated: false,
            device_list: crate::devices::DeviceList::default(),
            devices_told: 0,
            devices_last_told: None,
            supervised: std::env::var_os("AETHERD_SUPERVISED").is_some_and(|v| v == "1"),
            heard: crate::heard::HeardList::open(Some(path.with_file_name("heard.json"))),
            sessions: crate::sessions::SessionLog::open(Some(path.with_file_name("sessions.json"))),
            memories: crate::memories::Memories::open(Some(
                path.with_file_name("frequencies.json"),
            )),
            host: None,
            host_flags: crate::kiss::HostFlags::default(),
            kiss: None,
            kiss_error: None,
            kiss_tried: None,
            kiss_retry_at: None,
            control_handle: None,
            profiles: crate::profile::Store::open(Some(&path)),
            profiles_changed: false,
            path,
        }
    }

    /// The machine's devices as last listed, for everything that checks against them.
    ///
    /// The daemon lists at start and whenever a client asks for the list, on a thread of its
    /// own ([`crate::devices`]): listing on the run loop held the modem still for seconds at
    /// every profile switch. Before the first listing has finished this waits for it, up to
    /// [`crate::devices::FIRST_WAIT`] — at start only; a listing still not there reads as one
    /// that could not be taken, which checks nothing. A daemon that began no listing — a
    /// test's — lists here, with the substitute the test put in [`Self::devices`].
    #[must_use]
    pub fn inventory(&self) -> Value {
        if !self.device_list.begun() {
            return (self.devices)();
        }
        self.device_list
            .last(crate::devices::FIRST_WAIT)
            .unwrap_or_else(|| json!({ "error": "the devices are still being listed" }))
    }

    /// Begin listing the devices again in the background; the clients are given the new
    /// list as a `devices` event when it finishes, if it differs.
    pub fn refresh_devices(&self) {
        self.device_list.refresh(self.devices);
    }

    /// A listing that finished since the clients were last given one and differs from it,
    /// to publish as the `devices` event; `None` when there is nothing new.
    pub fn take_new_devices(&mut self) -> Option<Value> {
        let finished = self.device_list.finished();
        if finished == self.devices_told {
            return None;
        }
        self.devices_told = finished;
        let listing = self.device_list.last(std::time::Duration::ZERO)?;
        // a listing that failed is not a list: the clients keep the one they have
        if listing.get("error").is_some() || self.devices_last_told.as_ref() == Some(&listing) {
            return None;
        }
        self.devices_last_told = Some(listing.clone());
        Some(listing)
    }

    /// The settings or the profiles changed: the clients are told on the loop's next pass.
    pub fn note_profiles_changed(&mut self) {
        self.profiles_changed = true;
    }

    /// Whether the clients are owed a `profile` event, and forget that they were.
    pub fn take_profiles_changed(&mut self) -> bool {
        std::mem::take(&mut self.profiles_changed)
    }

    /// The host interface, as `status` reports it.
    fn host_json(&self) -> Value {
        match &self.host {
            Some(host) => json!({
                "enabled": true,
                "command_address": host.command_address,
                "data_address": host.data_address,
                "connected": host.connected.load(std::sync::atomic::Ordering::Relaxed),
                // `CHAT ON`: VARA's leave for the KISS port to transmit while a host is attached
                "chat": self.host_flags.chat.load(std::sync::atomic::Ordering::Relaxed),
            }),
            None => json!({ "enabled": false, "connected": false }),
        }
    }

    /// The KISS port, as `status.kiss` reports it: listening or why not, the clients, what
    /// has crossed, and why client frames are held when they are (ADR-0019).
    #[must_use]
    pub fn kiss_json(&self) -> Value {
        let settings = &self.config.kiss;
        let mut out = match &self.kiss {
            Some(server) => {
                let mut status = serde_json::to_value(server.status()).unwrap_or(Value::Null);
                status["exposed"] = json!(crate::kiss::server::exposed(&server.address));
                status
            }
            None => json!({
                "listening": false,
                "address": Value::Null,
                "error": self.kiss_error,
                "clients": [],
            }),
        };
        out["enabled"] = json!(settings.enabled);
        out["bind"] = json!(settings.bind);
        out["rung"] = json!(settings.rung);
        out["wait_for_clear"] = json!(settings.wait_for_clear);
        out["ignore_dcd"] = json!(
            self.host_flags
                .ignore_dcd
                .load(std::sync::atomic::Ordering::Relaxed)
        );
        out
    }
}

/// The audio devices and serial ports this machine reports, or why it could not say.
#[must_use]
pub fn device_inventory() -> Value {
    match crate::audio::list_devices() {
        Ok(devices) => json!({
            "devices": devices.iter().map(device_json).collect::<Vec<_>>(),
            "serial_ports": crate::ptt::list_serial_ports(),
            "gpio_interfaces": crate::ptt::list_gpio_interfaces(),
        }),
        Err(error) => json!({ "error": error.to_string() }),
    }
}

/// One audio device, as the API describes it.
fn device_json(device: &crate::audio::DeviceInfo) -> Value {
    json!({
        "name": device.name,
        "input": device.input,
        "output": device.output,
        "input_rates": device.input_rates,
        "output_rates": device.output_rates,
    })
}

/// Handle one request against a station.
pub fn dispatch<P: Ptt>(station: &mut Station<P>, request: &Request) -> Response {
    dispatch_with(station, None, request)
}

/// Handle one request, with access to the daemon's own state.
pub fn dispatch_with<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    request: &Request,
) -> Response {
    if crate::control::profiles::handles(&request.method) {
        return crate::control::profiles::dispatch(
            station,
            daemon,
            &request.method,
            &request.params,
            request.id.clone(),
        );
    }
    match request.method.as_str() {
        "config.get" => return config_get(daemon, request.id.clone()),
        "config.set" => return config_set(station, daemon, &request.params, request.id.clone()),
        "diagnostics" => return diagnostics(station, daemon, request.id.clone()),
        "share.prepare" => {
            return share_prepare(station, daemon, &request.params, request.id.clone());
        }
        "share.upload" => {
            return share_upload(station, daemon, &request.params, request.id.clone());
        }
        // whether a restart is something the panel can do for the operator is the
        // daemon's to know, not the station's
        "heard.list" => return heard_list(daemon.as_deref(), request.id.clone()),
        "heard.clear" => {
            let cleared = daemon.map_or(0, |d| d.heard.clear());
            return Response::ok(request.id.clone(), json!({ "cleared": cleared }));
        }
        "sessions.list" => {
            let remote = request.params.get("remote").and_then(Value::as_str);
            let sessions = daemon
                .as_ref()
                .map_or_else(Vec::new, |d| d.sessions.sessions(remote));
            return Response::ok(
                request.id.clone(),
                json!({
                    "sessions": sessions,
                    "limit": crate::sessions::LIMIT,
                    "path": daemon.as_ref().and_then(|d| d.sessions.path()).map(|p| p.display().to_string()),
                }),
            );
        }
        "sessions.clear" => {
            let Some(daemon) = daemon else {
                return Response::ok(request.id.clone(), json!({ "cleared": 0 }));
            };
            let cleared = daemon.sessions.clear();
            if let Err(error) = daemon.sessions.save() {
                return Response::failed(
                    request.id.clone(),
                    ApiError::new(
                        "cannot_save",
                        format!("the session history was cleared but not written: {error}"),
                        true,
                    ),
                );
            }
            return Response::ok(request.id.clone(), json!({ "cleared": cleared }));
        }
        "frequencies.list" => {
            let (memories, path) = daemon.as_ref().map_or_else(
                || (crate::memories::defaults(), None),
                |d| {
                    (
                        d.memories.entries().to_vec(),
                        d.memories.path().map(|p| p.display().to_string()),
                    )
                },
            );
            return Response::ok(
                request.id.clone(),
                json!({ "memories": memories, "path": path, "limit": crate::memories::LIMIT }),
            );
        }
        "frequencies.set" => return frequencies_set(daemon, &request.params, request.id.clone()),
        "kiss.status" => return Response::ok(request.id.clone(), kiss_status(daemon.as_deref())),
        "devices.list" => {
            return match daemon {
                Some(daemon) => devices_listed(daemon, request.id.clone()),
                None => devices(request.id.clone()),
            };
        }
        "kiss.disconnect" => return kiss_disconnect(daemon, &request.params, request.id.clone()),
        "status" => {
            return Response::ok(
                request.id.clone(),
                daemon_status(station, daemon.as_deref()),
            );
        }
        _ => {}
    }
    if let Some(refusal) = daemon
        .as_deref()
        .and_then(|daemon| unkeyed_without_a_host(daemon, request))
    {
        return refusal;
    }
    dispatch_station(station, request)
}

/// Why a station keyed by its host program starts nothing with no host program attached
/// (ADR-0025).
pub const NO_HOST_TO_KEY: &str = "This station is keyed by the host program, and none is \
    attached: nothing would reach the air. Start the host program (VarAC, Winlink Express), \
    or choose another way to key the radio in Setup, step 2.";

/// A station keyed by its host program transmits only while one is attached: with none,
/// the audio would go to a radio nobody keys. What the host program itself asks for comes
/// while it is attached, so only the panel's and the KISS programs' requests meet this.
fn unkeyed_without_a_host(daemon: &DaemonState, request: &Request) -> Option<Response> {
    if !matches!(daemon.config.ptt, crate::config::PttConfig::Host { .. }) {
        return None;
    }
    let attached = daemon
        .host
        .as_ref()
        .is_some_and(|host| host.connected.load(std::sync::atomic::Ordering::Relaxed));
    let params = &request.params;
    // zero is "stop", for a tone, drive bursts and a repeating beacon: always allowed
    let stopping = |key: &str| params.get(key).and_then(Value::as_f64) == Some(0.0);
    let transmits = match request.method.as_str() {
        "connect" | "beacon" | "probe" | "test.start" | "ptt.test" | "datagram.send" => true,
        "tune" => !stopping("duration_s"),
        "drive.set" => !stopping("bursts"),
        "beacon.every" => !stopping("minutes") && !params["minutes"].is_null(),
        _ => false,
    };
    (transmits && !attached).then(|| {
        Response::failed(
            request.id.clone(),
            ApiError::new("refused", NO_HOST_TO_KEY, true),
        )
    })
}

/// The KISS server's state, or a disabled one's when there is no daemon.
fn kiss_status(daemon: Option<&DaemonState>) -> Value {
    daemon.map_or_else(
        || json!({ "enabled": false, "listening": false, "clients": [] }),
        DaemonState::kiss_json,
    )
}

/// `status`: the station's, with what only the daemon knows.
fn daemon_status<P: Ptt>(station: &mut Station<P>, daemon: Option<&DaemonState>) -> Value {
    let mut result = status(station);
    result["supervised"] = json!(daemon.is_some_and(|d| d.supervised));
    result["host"] = daemon.map_or_else(
        || json!({ "enabled": false, "connected": false }),
        DaemonState::host_json,
    );
    result["kiss"] = kiss_status(daemon);
    result["audio_fault"] = json!(daemon.and_then(|d| d.audio_fault.clone()));
    result["config_note"] = json!(daemon.and_then(|d| d.config_note.clone()));
    result["upload"] = json!(daemon.map(|d| d.upload.progress()));
    // debug mode (ADR-0050): whether it is on, what waits, what went
    result["debug"] = daemon.map_or(Value::Null, |d| {
        let mut debug = json!(d.debug);
        debug["on"] = json!(d.config.record.send_to_project);
        debug
    });
    // which installation this daemon runs from: a shell that finds one already
    // listening decides from this whether it is its own to stop
    result["binary"] = json!(
        std::env::current_exe()
            .ok()
            .map(|p| p.display().to_string())
    );
    result
}

/// The configuration as a client may see it: with the secrets taken out.
///
/// A loopback client needs no token, so it must not be able to read the one that guards a
/// network bind — the point of the token is that reaching the port is not enough.
fn redacted(config: &crate::config::Config) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(config)?;
    if let Some(token) = value.pointer_mut("/control/token")
        && !token.is_null()
    {
        *token = Value::String("<set>".to_owned());
    }
    Ok(value)
}

fn config_get(daemon: Option<&mut DaemonState>, id: Option<String>) -> Response {
    let Some(daemon) = daemon else {
        return Response::failed(
            id,
            ApiError::new(
                "unsupported",
                "This daemon was started without a configuration file, so there is nothing \
                 to read or change.",
                false,
            ),
        );
    };
    match redacted(&daemon.config) {
        Ok(value) => Response::ok(
            id,
            json!({
                "config": value,
                "path": daemon.path.display().to_string(),
                "live_keys": crate::config::LIVE_KEYS,
            }),
        ),
        Err(error) => Response::failed(
            id,
            ApiError::new(
                "internal",
                format!("cannot read the settings: {error}"),
                false,
            ),
        ),
    }
}

/// Milliseconds since the Unix epoch, for a `SystemTime`.
fn unix_ms(time: std::time::SystemTime) -> u64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Everything a bug report needs, in one object.
///
/// The questions a maintainer asks first are "what version, what platform, what settings,
/// what was it doing", and the operator is rarely at the station when they are asked. This
/// answers all of them at once, with the secrets out and the recent log in, so the panel can
/// offer one "copy" button and an issue can be filed from a phone.
/// The zip another operator asks for: the logs, the history, the stations heard, the
/// diagnostic bundle and the recordings of a period, for the operator to attach to an email
/// (`share.rs`). The audio, megabytes a recording, only when asked for and only while idle:
/// the zip is written on the run loop.
fn share_prepare<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    params: &Value,
    id: Option<String>,
) -> Response {
    let Some(daemon) = daemon else {
        return Response::failed(
            id,
            ApiError::new(
                "unavailable",
                "Nothing to share: this modem keeps no files.",
                false,
            ),
        );
    };
    let (hours, remote, audio) = match share_params(params, id.clone()) {
        Ok(asked) => asked,
        Err(refused) => return refused,
    };
    if audio && station.state() != aether_link::State::Idle {
        return Response::failed(
            id,
            ApiError::new(
                "not_idle",
                "Cannot share the audio during a session: the recordings are large, and copying \
                 them would hold the modem up. Share without the audio, or after the session.",
                true,
            ),
        );
    }
    let now = unix_ms(std::time::SystemTime::now());
    let since_ms = now.saturating_sub((hours * 3_600_000.0) as u64);
    let places = crate::share::Places {
        config: daemon.path.clone(),
        recordings: station.record_dir().map(std::path::Path::to_path_buf),
        log_file: daemon.config.log.file.clone(),
    };
    let request = crate::share::Request {
        since_ms,
        remote: remote.clone(),
        audio,
        recordings: None,
    };
    let mut gathered = crate::share::gather(&places, &request);
    // the settings without their secrets, the devices, the status: the diagnostic bundle
    let callsign = station.engine().my_call.clone();
    let bundle = diagnostics(station, Some(&mut *daemon), None)
        .result
        .unwrap_or(Value::Null);
    gathered.entries.push((
        "diagnostics.json".into(),
        crate::share::Entry::Bytes(serde_json::to_vec_pretty(&bundle).unwrap_or_default()),
    ));
    let readme = format!(
        "Aether HF {} at {callsign}\nFiles shared {} UTC\nPeriod: the last {hours} h, from {} UTC\n\
         Sessions {}: {} recorded\nAudio: {}\n",
        env!("CARGO_PKG_VERSION"),
        crate::share::stamp(now),
        crate::share::stamp(since_ms),
        remote
            .as_deref()
            .map_or_else(|| "with any station".to_owned(), |r| format!("with {r}")),
        gathered.sessions,
        if audio { "included" } else { "left out" },
    );
    gathered.entries.push((
        "README.txt".into(),
        crate::share::Entry::Bytes(readme.into_bytes()),
    ));
    let state = format!("{:?}", station.state());
    match write_shared(daemon, &callsign, now, &gathered, &state) {
        Ok((path, bytes)) => Response::ok(
            id,
            json!({
                "path": path.display().to_string(),
                "name": path.file_name().map(|n| n.to_string_lossy().into_owned()),
                "bytes": bytes,
                "files": gathered.entries.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
                "sessions": gathered.sessions,
                "remote": remote,
                "hours": hours,
                "audio": audio,
                "callsign": callsign,
            }),
        ),
        Err(message) => Response::failed(id, ApiError::new("io", message, true)),
    }
}

/// Send a zip `share.prepare` wrote to the station that asked for it: to the upload script its
/// request link named, with the code it issued (`upload.rs`). The upload runs on a thread of its
/// own; `status.upload` says how it goes.
fn share_upload<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    params: &Value,
    id: Option<String>,
) -> Response {
    let refuse = |code: &str, message: &str| {
        Response::failed(id.clone(), ApiError::new(code, message, false))
    };
    let Some(daemon) = daemon else {
        return refuse("unavailable", "Nothing to send: this modem keeps no files.");
    };
    let text = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned()
    };
    let (name, endpoint, code) = (text("name"), text("endpoint"), text("code"));
    if !crate::upload::endpoint_allowed(&endpoint) {
        return refuse(
            "bad_params",
            "That is not an upload address: the request link should carry the address of the \
             asking station's upload script (https://script.google.com/…/exec).",
        );
    }
    // a code is no longer needed — the script bounds a public address by the day — but one
    // from an earlier request link still goes, and has to look like one
    if !code.is_empty() && !crate::upload::code_allowed(&code) {
        return refuse(
            "bad_params",
            "The request link's upload code is damaged: send without it, or ask for the link again.",
        );
    }
    // a name `share.prepare` gave, in `shared/`, and nothing else
    let plain = !name.is_empty()
        && std::path::Path::new(&name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        && !name.contains(['/', '\\'])
        && !name.contains("..");
    let file = crate::share::shared_dir(&daemon.path).join(&name);
    if !plain || !file.is_file() {
        return refuse(
            "not_found",
            "That zip is not in the shared folder: prepare the files again.",
        );
    }
    if daemon.upload.busy() {
        return Response::failed(
            id,
            ApiError::new("busy", "Another upload is under way.", true),
        );
    }
    let bytes = std::fs::metadata(&file).map_or(0, |m| m.len());
    let job = crate::upload::Job {
        file: file.clone(),
        name: name.clone(),
        endpoint,
        code,
        note: params
            .get("note")
            .and_then(Value::as_str)
            .map(|n| n.chars().take(2000).collect())
            .unwrap_or_default(),
        callsign: station.engine().my_call.clone(),
        zip: None,
    };
    if let Err(error) = daemon.upload.start(job, (daemon.upload_http)()) {
        return Response::failed(id, ApiError::new("busy", error, true));
    }
    daemon.log.record(
        crate::log::Level::Info,
        "share",
        &format!("sending {name} ({bytes} bytes) to the station that asked for it"),
        &format!("{:?}", station.state()),
    );
    Response::ok(id, json!({ "started": true, "name": name, "bytes": bytes }))
}

/// Debug mode (ADR-0050): send the sessions `pending` names — their recordings with the audio,
/// the logs, the session history and the diagnostic bundle — to the Aether project. The zip
/// is gathered here, which only lists files, and written and uploaded on the upload's thread.
/// Returns the zip's name.
///
/// # Errors
/// Another upload is under way, or its thread would not start.
pub fn start_debug_upload<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    pending: &[crate::debug::Pending],
) -> Result<String, String> {
    let now = unix_ms(std::time::SystemTime::now());
    let since_ms = pending
        .iter()
        .map(|p| p.started_ms)
        .min()
        .unwrap_or(now)
        .saturating_sub(10 * 60_000);
    let places = crate::share::Places {
        config: daemon.path.clone(),
        recordings: station.record_dir().map(std::path::Path::to_path_buf),
        log_file: daemon.config.log.file.clone(),
    };
    let request = crate::share::Request {
        since_ms,
        remote: None,
        audio: true,
        recordings: Some(pending.iter().map(|p| p.recording.clone()).collect()),
    };
    let mut gathered = crate::share::gather(&places, &request);
    let callsign = station.engine().my_call.clone();
    let bundle = diagnostics(station, Some(&mut *daemon), None)
        .result
        .unwrap_or(Value::Null);
    gathered.entries.push((
        "diagnostics.json".into(),
        crate::share::Entry::Bytes(serde_json::to_vec_pretty(&bundle).unwrap_or_default()),
    ));
    let remotes: Vec<&str> = pending.iter().map(|p| p.remote.as_str()).collect();
    let readme = format!(
        "Aether HF {} at {callsign}\nSent by debug mode {} UTC\nSessions a host program ran: {} \
         ({} recorded)\nAudio: included\n",
        env!("CARGO_PKG_VERSION"),
        crate::share::stamp(now),
        remotes.join(", "),
        gathered.sessions,
    );
    gathered.entries.push((
        "README.txt".into(),
        crate::share::Entry::Bytes(readme.into_bytes()),
    ));
    let safe_call: String = callsign
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = format!("aether-{safe_call}-{}-debug.zip", crate::share::stamp(now));
    let job = crate::upload::Job {
        file: crate::share::shared_dir(&daemon.path).join(&name),
        name: name.clone(),
        endpoint: crate::upload::PROJECT_ENDPOINT.to_owned(),
        code: String::new(),
        note: format!(
            "Debug mode: {} session{} with {}.",
            pending.len(),
            if pending.len() == 1 { "" } else { "s" },
            remotes.join(", ")
        ),
        callsign,
        zip: Some((gathered.entries, now)),
    };
    daemon.upload.start(job, (daemon.upload_http)())?;
    Ok(name)
}

/// What `share.prepare` was asked for: the hours back, the station, the audio.
///
/// # Errors
/// The response refusing a period outside 15 minutes to 14 days.
fn share_params(
    params: &Value,
    id: Option<String>,
) -> Result<(f64, Option<String>, bool), Response> {
    let hours = params.get("hours").and_then(Value::as_f64).unwrap_or(3.0);
    if !(0.25..=24.0 * 14.0).contains(&hours) {
        return Err(Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "Share files from the last 15 minutes to 14 days.",
                false,
            ),
        ));
    }
    let remote = params
        .get("remote")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_ascii_uppercase);
    let audio = params
        .get("audio")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((hours, remote, audio))
}

/// Write the zip under `shared/` beside the configuration, keep the newest few, and log it.
///
/// # Errors
/// What could not be written, in a sentence.
fn write_shared(
    daemon: &mut DaemonState,
    callsign: &str,
    now: u64,
    gathered: &crate::share::Gathered,
    state: &str,
) -> Result<(std::path::PathBuf, u64), String> {
    let dir = crate::share::shared_dir(&daemon.path);
    let safe_call: String = callsign
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = dir.join(format!(
        "aether-{safe_call}-{}.zip",
        crate::share::stamp(now)
    ));
    let bytes = std::fs::create_dir_all(&dir)
        .and_then(|()| crate::share::write_zip(&path, &gathered.entries, now))
        .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
    crate::share::prune(&dir, crate::share::KEEP);
    daemon.log.record(
        crate::log::Level::Info,
        "share",
        &format!(
            "files for another operator: {} ({} files, {} sessions, {bytes} bytes)",
            path.display(),
            gathered.entries.len(),
            gathered.sessions
        ),
        state,
    );
    Ok((path, bytes))
}

/// Every station heard, most recent first, and where the list is kept.
fn heard_list(daemon: Option<&DaemonState>, id: Option<String>) -> Response {
    let stations = daemon.map_or_else(Vec::new, |d| d.heard.stations().to_vec());
    Response::ok(
        id,
        json!({
            "stations": stations,
            "limit": crate::heard::LIMIT,
            "path": daemon.and_then(|d| d.heard.path()).map(|p| p.display().to_string()),
        }),
    )
}

fn diagnostics<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    id: Option<String>,
) -> Response {
    // without the daemon's state there is nobody to ask about devices, and a bundle from a
    // bare station is a test fixture rather than a bug report
    let devices = daemon
        .as_ref()
        .map_or(Value::Null, |daemon| daemon.inventory());
    let mut bundle = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "cpus": std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        },
        "generated": crate::log::rfc3339(unix_ms(std::time::SystemTime::now())),
        // what `status` answers, the daemon's own part with it: whether a host program or a
        // KISS program was attached is often the first question a report raises
        "status": daemon_status(station, daemon.as_deref()),
        "capabilities": capabilities(station.params()),
        "devices": devices,
        "config": Value::Null,
        "path": Value::Null,
        "log": Value::Array(Vec::new()),
    });
    if let Some(daemon) = daemon
        && let Some(object) = bundle.as_object_mut()
    {
        object.insert(
            "config".into(),
            redacted(&daemon.config).unwrap_or(Value::Null),
        );
        object.insert("path".into(), json!(daemon.path.display().to_string()));
        object.insert(
            "started".into(),
            json!(crate::log::rfc3339(unix_ms(daemon.started))),
        );
        object.insert(
            "audio".into(),
            json!({
                "description": daemon.audio,
                "dropped_samples": daemon.dropped_audio,
                "starved_samples": daemon.starved_audio,
            }),
        );
        object.insert(
            "loop".into(),
            json!({
                "slowest_ms": (daemon.loop_slowest_ms * 10.0).round() / 10.0,
                "slowest_phase": daemon.loop_slowest_phase,
                "stalls": daemon.loop_stalls,
                "machine_late_ms": (daemon.loop_machine_late_ms * 10.0).round() / 10.0,
            }),
        );
        object.insert("log".into(), json!(daemon.log.recent()));
        object.insert("log_forgotten".into(), json!(daemon.log.forgotten()));
    }
    Response::ok(id, bundle)
}

fn config_set<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    params: &Value,
    id: Option<String>,
) -> Response {
    let Some(settings) = daemon else {
        return Response::failed(
            id,
            ApiError::new(
                "unsupported",
                "This daemon was started without a configuration file, so there is nothing \
                 to change.",
                false,
            ),
        );
    };

    // Merge into a copy: a refused change must leave the station running on what it had.
    let mut candidate = settings.config.clone();
    let changed = match candidate.merge(params) {
        Ok(changed) => changed,
        Err(error) => {
            return Response::failed(id, ApiError::new("bad_params", error.to_string(), false));
        }
    };
    if let Err(error) = candidate.save(&settings.path) {
        return Response::failed(id, ApiError::new("cannot_save", error.to_string(), true));
    }

    let restart_required: Vec<&String> = changed
        .iter()
        .filter(|key| !crate::config::Config::is_live(key))
        .collect();
    station.apply_live(&candidate);
    settings.config = candidate;
    settings.note_profiles_changed();

    Response::ok(
        id,
        json!({
            "changed": changed,
            "restart_required": restart_required,
            "path": settings.path.display().to_string(),
        }),
    )
}

fn dispatch_station<P: Ptt>(station: &mut Station<P>, request: &Request) -> Response {
    if let Some(refusal) = refused_before_transmitting(station, request) {
        return refusal;
    }
    let id = request.id.clone();
    let params = &request.params;
    match request.method.as_str() {
        "status" => Response::ok(id, status(station)),
        "capabilities" => Response::ok(id, capabilities(station.params())),
        "spectrum" => Response::ok(id, spectrum(station)),
        "constellation" => Response::ok(id, constellation(station)),
        "connect" => connect(station, params, id),
        "probe" => probe(station, params, id),
        "frequency.set" => tune_to(station, params, id),
        "counters.reset" => {
            station.reset_counters();
            Response::ok(id, json!({ "counters": counters(station) }))
        }
        "test.start" => test_start(station, params, id),
        "test.status" => Response::ok(id, station.test_status()),
        "test.abort" => Response::ok(id, json!({ "aborted": station.abort_test() })),
        "callsigns.set" => set_callsigns(station, params, id),
        "regulatory.check" => Response::ok(id, regulatory_check(station, params)),
        "regulatory.profile" => Response::ok(id, regulatory_profiles(station)),
        "beacon" => beacon(station, params, id),
        "beacon.every" => beacon_every(station, params, id),
        "bandwidth.set" => bandwidth_set(station, params, id),
        "ptt.test" => {
            let seconds = params
                .get("duration_s")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            match station.key_test(seconds) {
                Ok(()) => Response::ok(id, json!({ "accepted": true, "duration_s": seconds })),
                Err(reason) => Response::failed(
                    id,
                    ApiError::new("refused", format!("Cannot key: {reason}."), true),
                ),
            }
        }
        "tune" => {
            let seconds = params
                .get("duration_s")
                .and_then(Value::as_f64)
                .unwrap_or(3.0);
            // zero is "stop": the tone is bounded either way, but an operator whose ALC
            // is where it should be does not want the rest of it
            if seconds == 0.0 {
                let stopped = station.tune_stop();
                return Response::ok(id, json!({ "stopped": stopped }));
            }
            match station.tune(seconds) {
                Ok(()) => Response::ok(id, json!({ "accepted": true, "duration_s": seconds })),
                Err(reason) => Response::failed(
                    id,
                    ApiError::new("refused", format!("Cannot tune: {reason}."), true),
                ),
            }
        }
        "drive.set" => drive_set(station, params, id),
        "audio.level" => Response::ok(id, level_json(&station.audio_level())),
        "record.start" | "record.stop" | "record.notes" => record(station, request),
        "disconnect" => {
            // orderly: what is queued is sent and acknowledged first
            station.disconnect();
            Response::ok(id, json!({ "accepted": true, "orderly": true }))
        }
        "abort" => {
            station.abort();
            Response::ok(id, json!({ "accepted": true, "orderly": false }))
        }
        "send" => send(station, params, id),
        "datagram.send" => datagram_send(station, params, id),
        "listen" => listen(params, id),
        "devices.list" => devices(id),
        other => Response::failed(
            id,
            ApiError::new(
                "unknown_method",
                format!("This version does not have a method called {other:?}."),
                false,
            ),
        ),
    }
}

/// `listen`: whether the station answers calls.
fn listen(params: &Value, id: Option<String>) -> Response {
    match params.get("enabled").and_then(Value::as_bool) {
        None => Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "Listening is on or off: {\"enabled\": true}.",
                false,
            ),
        ),
        // A station always answers a call; there is nothing to switch off yet, and
        // saying so is better than accepting a setting that does nothing.
        Some(true) => Response::ok(id, json!({ "enabled": true })),
        Some(false) => Response::failed(
            id,
            ApiError::new(
                "unsupported",
                "This version always answers a call. Stop the daemon to stop listening.",
                false,
            ),
        ),
    }
}

fn connect<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let Some(remote) = params.get("remote").and_then(Value::as_str) else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "A callsign to call is required: {\"remote\": \"KK4XYZ\"}.",
                false,
            ),
        );
    };
    let as_call = params.get("callsign").and_then(Value::as_str);
    match station.connect_as(remote, as_call) {
        Ok(()) => Response::ok(id, json!({ "session": station.engine().session() })),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                if reason.starts_with("not one of") {
                    "bad_params"
                } else if answer_only(reason) {
                    "refused"
                } else {
                    "already_connected"
                },
                format!("Cannot call {remote}: {reason}."),
                false,
            ),
        ),
    }
}

/// Whether a station's refusal is its `[radio] answer_only` setting: `refused`, and not to be
/// tried again until the setting changes — not the `not_idle` or `already_connected` of a
/// session or a probe that will be over in a while.
fn answer_only(reason: &str) -> bool {
    reason.contains("answer-only")
}

/// `beacon`: one frame with this station's callsign, outside any session.
fn beacon<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    // the name a host program gives its beacon (VarAC's `KK4ODA-9`): this station's
    // callsign as its base, or it is refused (ADR-0024)
    let name = params.get("callsign").and_then(Value::as_str);
    match station.beacon(name) {
        Ok(()) => Response::ok(id, json!({ "accepted": true })),
        Err(reason) if reason == crate::station::BEACON_NOT_OURS => Response::failed(
            id,
            ApiError::new("bad_params", format!("Cannot beacon: {reason}."), false),
        ),
        // the one pressed before goes as soon as the channel clears; a second would follow it
        Err(reason) if reason == crate::station::BEACON_ALREADY_WAITING => Response::failed(
            id,
            ApiError::new(
                "not_idle",
                format!(
                    "Cannot beacon: {reason}. It goes as soon as the channel is clear, or on \
                     another dial if you pick one."
                ),
                true,
            ),
        ),
        // a session ends, and the beacon can go then; answer-only, or a callsign that will
        // not go into one, is not a matter of waiting
        Err(reason) if !answer_only(reason) && station.state() != aether_link::State::Idle => {
            Response::failed(
                id,
                ApiError::new(
                    "not_idle",
                    format!("Cannot beacon: {reason}. A beacon is sent outside a session."),
                    true,
                ),
            )
        }
        Err(reason) => Response::failed(
            id,
            ApiError::new("refused", format!("Cannot beacon: {reason}."), false),
        ),
    }
}

/// The remembered dials, replaced whole and written beside the configuration.
fn frequencies_set(
    daemon: Option<&mut DaemonState>,
    params: &Value,
    id: Option<String>,
) -> Response {
    let Some(daemon) = daemon else {
        return Response::failed(
            id,
            ApiError::new("refused", "This modem keeps no list of dials.", false),
        );
    };
    let entries: Vec<crate::memories::Memory> = match params
        .get("memories")
        .map(|v| serde_json::from_value(v.clone()))
    {
        Some(Ok(entries)) => entries,
        _ => {
            return Response::failed(
                id,
                ApiError::new(
                    "bad_params",
                    "A list is required: {\"memories\": [{\"hz\": 14107000, \"name\": \"20 m\"}]}.",
                    false,
                ),
            );
        }
    };
    if let Err(reason) = daemon.memories.replace(entries) {
        return Response::failed(id, ApiError::new("bad_params", reason, false));
    }
    if let Err(error) = daemon.memories.save() {
        return Response::failed(
            id,
            ApiError::new(
                "refused",
                format!("The list could not be written: {error}."),
                true,
            ),
        );
    }
    daemon.note_profiles_changed();
    Response::ok(
        id,
        json!({ "memories": daemon.memories.entries(), "path": daemon.memories.path().map(|p| p.display().to_string()) }),
    )
}

/// A Test session (P6-7): probe, call, a message, the mode ladder, a file, disconnect —
/// recorded, and reported by `test.status` while it runs and after. Whether the rules let this
/// station start an exchange here was asked before this (`refused_before_transmitting`).
fn test_start<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let plan = match crate::station::TestPlan::from_params(params) {
        Ok(plan) => plan,
        Err(reason) => {
            return Response::failed(id, ApiError::new("bad_params", reason, false));
        }
    };
    let remote = plan.remote.clone();
    match station.start_test(plan) {
        Ok(()) => Response::ok(id, json!({ "accepted": true })),
        Err(reason) => {
            // a session, a probe or another test is over in a while
            let (code, retryable) = if reason.contains("not one of") {
                ("bad_params", false)
            } else if answer_only(&reason) {
                ("refused", false)
            } else {
                ("not_idle", true)
            };
            Response::failed(
                id,
                ApiError::new(
                    code,
                    format!("Cannot start a test session with {remote}: {reason}."),
                    retryable,
                ),
            )
        }
    }
}

/// The radio tuned to a dial, over CAT or `rigctld`; refused in a session or with a
/// keying interface that cannot ask.
/// `drive.set`: real bursts, so the rig's ALC is shown the peaks traffic presents.
///
/// Its own function because `dispatch_station` is at clippy's line budget.
fn drive_set<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let bursts = params
        .get("bursts")
        .and_then(Value::as_u64)
        .map_or(4, |n| n as usize);
    // zero is "stop", as it is for a tune tone
    if bursts == 0 {
        let stopped = station.tune_stop();
        return Response::ok(id, json!({ "stopped": stopped }));
    }
    match station.set_drive(bursts) {
        Ok(()) => Response::ok(id, json!({ "accepted": true, "bursts": bursts })),
        Err(reason) => Response::failed(
            id,
            ApiError::new("refused", format!("Cannot set drive: {reason}."), true),
        ),
    }
}

fn tune_to<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let Some(hz) = params.get("hz").and_then(Value::as_u64) else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "A frequency is required: {\"hz\": 14107000}.",
                false,
            ),
        );
    };
    match station.tune_to(hz) {
        Ok(()) => Response::ok(id, json!({ "hz": hz })),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                "refused",
                format!("Cannot tune the radio: {reason}."),
                false,
            ),
        ),
    }
}

/// A probe (ADR-0006): the question a session would answer, without the session.
fn probe<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let Some(remote) = params.get("remote").and_then(Value::as_str) else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "A callsign to probe is required: {\"remote\": \"KK4XYZ\"}.",
                false,
            ),
        );
    };
    let as_call = params.get("callsign").and_then(Value::as_str);
    match station.probe(remote, as_call) {
        Ok(()) => Response::ok(id, json!({ "accepted": true })),
        // a probe already out, or a session: over in a while
        Err(reason) => {
            let (code, retryable) = if reason.starts_with("not one of") {
                ("bad_params", false)
            } else if answer_only(reason) {
                ("refused", false)
            } else {
                ("not_idle", true)
            };
            Response::failed(
                id,
                ApiError::new(
                    code,
                    format!(
                        "Cannot probe {remote}: {reason}. The answer, or its absence, is reported as a probe event."
                    ),
                    retryable,
                ),
            )
        }
    }
}

/// The callsigns the station answers to, from a host program or the panel.
fn set_callsigns<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let calls: Option<Vec<String>> =
        params
            .get("callsigns")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            });
    let Some(calls) = calls else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "A list of callsigns is required: {\"callsigns\": [\"KK4XYZ\", \"KK4XYZ-T\"]}.",
                false,
            ),
        );
    };
    match station.set_callsigns(&calls) {
        Ok(applied) => Response::ok(
            id,
            json!({
                "callsigns": if applied { station.callsigns().to_vec() } else { calls },
                "applied": applied,
            }),
        ),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                "bad_params",
                format!(
                    "Cannot use these callsigns: {reason}. A callsign is letters, digits, '-' and '/', at most nine characters."
                ),
                false,
            ),
        ),
    }
}

/// A transmission the regulatory policy refuses (ADR-0018): the verdict's own words, and the
/// whole decision for a client that shows it.
fn refused_by_rules(
    id: Option<String>,
    what: &str,
    decision: &crate::regulatory::Decision,
) -> Response {
    let mut response = Response::failed(
        id,
        ApiError::new(
            "regulatory",
            format!(
                "Cannot {what} — {}. {}",
                decision.summary,
                sentence(&decision.detail)
            ),
            true,
        ),
    );
    response.result = Some(json!({ "decision": decision }));
    response
}

/// Anything that would transmit asks the regulatory policy first, and a refusal says why
/// (ADR-0018). This is the courtesy of an early answer; the gate in front of the transmitter
/// decides again for every burst, whatever asked for it.
fn refused_before_transmitting<P: Ptt>(
    station: &mut Station<P>,
    request: &Request,
) -> Option<Response> {
    use crate::regulatory::EmissionKind;
    let params = &request.params;
    // `None`: the station would start an exchange; `Some`: an operator's test of that kind
    let (what, test) = match request.method.as_str() {
        "probe" => ("probe", None),
        "connect" => ("call", None),
        "beacon" => ("beacon", None),
        // a probe and a call, to begin with
        "test.start" => ("start a test session", None),
        // zero is "stop", for a tune tone and for drive bursts: stopping is always allowed
        "tune" if params.get("duration_s").and_then(Value::as_f64) != Some(0.0) => {
            ("tune", Some(EmissionKind::Test))
        }
        "drive.set" if params.get("bursts").and_then(Value::as_u64) != Some(0) => {
            ("set drive", Some(EmissionKind::Data))
        }
        "ptt.test" => ("key", Some(EmissionKind::Nothing)),
        _ => return None,
    };
    let verdict = match test {
        None => station.check_originate(),
        Some(kind) => station.check_operator(kind),
    };
    verdict
        .err()
        .map(|decision| refused_by_rules(request.id.clone(), what, &decision))
}

/// `regulatory.profile`: the profile in force, as data, and the profiles this build knows.
fn regulatory_profiles<P: Ptt>(station: &Station<P>) -> Value {
    let known: Vec<Value> = crate::regulatory::profile::KNOWN
        .iter()
        .map(|(id, name)| json!({ "id": id, "name": name }))
        .collect();
    json!({ "profile": station.regulatory_profile(), "known": known })
}

/// A clause as a sentence of its own: its first letter in capitals.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `regulatory.check`: what the rules would say about a transmission, with any of the
/// station's facts replaced — a dial, the control, the class, the sideband, who began the
/// exchange, a rung — for the panel's diagnostics and for "what if" questions.
fn regulatory_check<P: Ptt>(station: &mut Station<P>, params: &Value) -> Value {
    use crate::regulatory::{ControlMode, Direction, LicenseClass, Sideband};
    let word = |key: &str| params.get(key).and_then(Value::as_str);
    let query = crate::station::RegulatoryQuery {
        dial_hz: params.get("dial_hz").and_then(Value::as_f64),
        control: word("control").and_then(ControlMode::parse),
        license: word("license_class").and_then(LicenseClass::parse),
        sideband: word("sideband").and_then(Sideband::parse),
        direction: match word("direction") {
            Some("originate") => Some(Direction::Originate),
            Some("respond") => Some(Direction::Respond),
            Some("operator") => Some(Direction::Operator),
            _ => None,
        },
        rung: params
            .get("rung")
            .and_then(Value::as_u64)
            .and_then(|r| usize::try_from(r).ok()),
    };
    station.regulatory_check(&query)
}

/// `datagram.send`: a KISS client's frame, to go out as a datagram outside any session
/// (ADR-0019). It waits in the station's queue while a session is up; a full queue is a
/// retryable `queue_full`, which the KISS port turns into TCP backpressure.
fn datagram_send<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    use crate::station::{DATAGRAM_QUEUE, DatagramRefusal, DatagramRequest};
    let Some(frame) = params
        .get("data")
        .and_then(Value::as_str)
        .and_then(from_base64)
    else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "A frame is required, base64: {\"data\": \"...\"}.",
                false,
            ),
        );
    };
    let number = |key: &str| params.get(key).and_then(Value::as_f64);
    let request = DatagramRequest {
        frame_type: params
            .get("frame_type")
            .and_then(Value::as_u64)
            .and_then(|t| u8::try_from(t).ok())
            .unwrap_or(0),
        frame,
        reference: params.get("ref").and_then(Value::as_str).map(str::to_owned),
        rung: params
            .get("rung")
            .and_then(Value::as_u64)
            .and_then(|r| usize::try_from(r).ok()),
        wait_for_clear: params
            .get("wait_for_clear")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        persistence: number("persistence").unwrap_or(crate::kiss::DEFAULT_PERSISTENCE),
        slot_s: number("slot_s").unwrap_or(crate::kiss::DEFAULT_SLOT_S),
    };
    match station.send_datagram(request) {
        Ok(queued) => Response::ok(
            id,
            json!({
                "accepted": true,
                "queued": queued.queued,
                "limit": DATAGRAM_QUEUE,
                "fragments": queued.fragments,
                "bursts": queued.bursts,
                "air_s": queued.air_s,
                "rung": queued.rung,
            }),
        ),
        Err(DatagramRefusal::QueueFull) => Response::failed(
            id,
            ApiError::new(
                "queue_full",
                format!(
                    "{DATAGRAM_QUEUE} datagrams are waiting already; try again when one has gone."
                ),
                true,
            ),
        ),
        Err(DatagramRefusal::Invalid(why)) => Response::failed(
            id,
            ApiError::new(
                "bad_params",
                format!("Cannot send the datagram: {why}."),
                false,
            ),
        ),
        Err(DatagramRefusal::NotAllowed(why)) => Response::failed(
            id,
            ApiError::new(
                "refused",
                format!("Cannot send the datagram: {why}."),
                false,
            ),
        ),
    }
}

/// `kiss.disconnect {client?}`: close one KISS client's connection, or every one's.
fn kiss_disconnect(
    daemon: Option<&mut DaemonState>,
    params: &Value,
    id: Option<String>,
) -> Response {
    let Some(server) = daemon.and_then(|d| d.kiss.as_ref()) else {
        return Response::failed(
            id,
            ApiError::new("not_listening", "The KISS port is not listening.", false),
        );
    };
    let client = params.get("client").and_then(Value::as_u64);
    let closed = server.disconnect(client);
    Response::ok(id, json!({ "disconnected": closed }))
}

fn send<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let Some(text) = params.get("data").and_then(Value::as_str) else {
        return Response::failed(
            id,
            ApiError::new(
                "bad_params",
                "Data to send is required, base64 encoded: {\"data\": \"...\"}.",
                false,
            ),
        );
    };
    let Some(bytes) = decode(text) else {
        return Response::failed(
            id,
            ApiError::new("bad_params", "The data field is not valid base64.", false),
        );
    };
    // a reference makes the message followed: a `sent` event says when the other station
    // has all of it, or that the session ended first
    let reference = match params.get("ref") {
        None | Some(Value::Null) => None,
        Some(Value::String(reference))
            if !reference.is_empty()
                && reference.len() <= 64
                && reference.chars().all(|c| c.is_ascii_graphic()) =>
        {
            Some(reference.as_str())
        }
        Some(_) => {
            return Response::failed(
                id,
                ApiError::new(
                    "bad_params",
                    "A reference is 1 to 64 printable ASCII characters: {\"ref\": \"m1\"}.",
                    false,
                ),
            );
        }
    };
    if !station.connected() {
        // accepting it would look like success and lose the data at the first disconnect
        return Response::failed(
            id,
            ApiError::new(
                "not_connected",
                "There is no session to send on. Connect first.",
                true,
            ),
        );
    }
    let count = bytes.len();
    if let Some(reference) = reference {
        station.send_tracked(&bytes, reference);
        Response::ok(id, json!({ "accepted": count, "ref": reference }))
    } else {
        station.send(&bytes);
        Response::ok(id, json!({ "accepted": count }))
    }
}

/// `devices.list` on the daemon: the last listing, made off the run loop, and a new one
/// begun — a device plugged in since reaches the clients as a `devices` event when that
/// listing finishes with a different list. The first listing, begun at start, is waited for.
fn devices_listed(daemon: &mut DaemonState, id: Option<String>) -> Response {
    let listing = daemon.inventory();
    daemon.refresh_devices();
    if let Some(error) = listing.get("error").and_then(Value::as_str) {
        return Response::failed(id, ApiError::new("audio_unavailable", error, true));
    }
    // what the client now has: a new listing that finds the same is not news
    daemon.devices_last_told = Some(listing.clone());
    Response::ok(id, listing)
}

/// `devices.list` without the daemon (a bare station): listed now.
fn devices(id: Option<String>) -> Response {
    match crate::audio::list_devices() {
        Ok(devices) => Response::ok(
            id,
            json!({
                "devices": devices.iter().map(device_json).collect::<Vec<_>>(),
                "serial_ports": crate::ptt::list_serial_ports(),
                "gpio_interfaces": crate::ptt::list_gpio_interfaces(),
            }),
        ),
        Err(error) => Response::failed(
            id,
            ApiError::new("audio_unavailable", error.to_string(), true),
        ),
    }
}

/// `beacon.every {minutes}`: beacon on a timer, the first now; `null` or `0` stops it.
fn beacon_every<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let minutes = params.get("minutes").and_then(Value::as_f64);
    let seconds = minutes.filter(|m| *m > 0.0).map(|m| m * 60.0);
    match station.beacon_every(seconds) {
        Ok(()) => Response::ok(id, station.beacon_status()),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                if answer_only(&reason) {
                    "refused"
                } else {
                    "bad_params"
                },
                format!("Cannot beacon on a timer: {reason}."),
                false,
            ),
        ),
    }
}

/// `bandwidth.set {hz}`: the bandwidth a host program asked for with VARA's `BW500`, `BW2300`
/// or `BW2750` — 2750 is 2300 here — or `null` to go back to the station's own (ADR-0026). The
/// station moves between sessions only: refused, and retryable, while anything is under way.
fn bandwidth_set<P: Ptt>(station: &mut Station<P>, params: &Value, id: Option<String>) -> Response {
    let hz = match params.get("hz") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64() {
            Some(hz @ (500 | 2300 | 2750)) => usize::try_from(hz).ok(),
            _ => {
                return Response::failed(
                    id,
                    ApiError::new(
                        "bad_params",
                        "The bandwidth is 500, 2300 or 2750 hertz, or null for the station's \
                         own: {\"hz\": 500}.",
                        false,
                    ),
                );
            }
        },
    };
    match station.host_bandwidth(hz) {
        Ok(_) => Response::ok(id, station.bandwidth_status()),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                "refused",
                format!(
                    "The station stays at {} Hz for now: {reason}.",
                    station.bandwidth_hz()
                ),
                true,
            ),
        ),
    }
}

/// Everything a client needs to render the station's current state.
fn status<P: Ptt>(station: &mut Station<P>) -> Value {
    let frequency_hz = station.frequency_hz();
    let (sent_pending, sent_recent) = station.delivery_status();
    let engine = station.engine();
    json!({
        "frequency_hz": frequency_hz,
        "state": match station.state() {
            aether_link::State::Idle => "idle",
            aether_link::State::Connecting => "connecting",
            aether_link::State::Connected => "connected",
            aether_link::State::Disconnecting => "disconnecting",
        },
        "role": match station.role() {
            aether_link::Role::None => "none",
            aether_link::Role::Iss => "iss",
            aether_link::Role::Irs => "irs",
        },
        "callsign": engine.my_call,
        "callsigns": engine.callsigns,
        "remote": engine.remote_call,
        "session": engine.session(),
        "mode": engine.current_mode(),
        "transmitting": station.transmitting(),
        "probing": engine.probing(),
        // a disconnect was asked for and the DISC has not gone yet: the sender finishing
        // what is queued, the receiver waiting for a burst to end (ADR-0023)
        "closing": engine.disconnect_requested(),
        "channel_busy": station.channel_busy(),
        "compressing": station.compressing(),
        "compression_saving": station.compression_saving(),
        "uptime_s": station.now(),
        "ptt": station.ptt_description(),
        "ptt_fault": station.ptt_fault(),
        "can_tune": station.can_tune(),
        "queued_bytes": engine.tx_pending_bytes(),
        "version": env!("CARGO_PKG_VERSION"),
        "link": link_json(station),
        "metrics": metrics(station),
        "recording": station.recording().map(|(path, seconds)| json!({
            "path": path.display().to_string(),
            "seconds": seconds,
        })),
        "test": station.test_brief(),
        // the repeating beacon and the beacons sent (ND1J's questions, 2026-09-25)
        "beacon": station.beacon_status(),
        // the Morse identifier: the speed set and the speed the rules let it be sent at
        "identifier": station.identifier_status(),
        // where the station stands with the rules (ADR-0018): the indicator, the ceiling on
        // the link's rungs, the gate's last decision and the dials where its waveforms fit
        "regulatory": station.regulatory_status(),
        // KISS clients' datagrams: waiting, sent and heard (ADR-0019)
        "datagrams": station.datagram_status(),
        // the messages sent with a reference that the other station does not yet have all
        // of, and the last ones resolved: a panel that missed a `sent` event looks here
        "sent": { "pending": sent_pending, "recent": sent_recent },
        "counters": counters(station),
        "recordings_dir": station.record_dir().map(|p| p.display().to_string()),
        // the bandwidth it runs, its own, a host program's request, and why (ADR-0026)
        "bandwidth": station.bandwidth_status(),
        // whether calls are answered: always, unless a host program attached has not said
        // LISTEN ON
        "answering": station.answering(),
    })
}

/// The recording methods: start, stop, and the operator's notes for the next one.
fn record<P: Ptt>(station: &mut Station<P>, request: &Request) -> Response {
    let id = request.id.clone();
    let params = &request.params;
    match request.method.as_str() {
        "record.start" => {
            let name = params.get("name").and_then(Value::as_str);
            let notes = params.get("notes").and_then(Value::as_str);
            match station.start_recording(name, notes) {
                Ok(path) => Response::ok(id, json!({ "path": path.display().to_string() })),
                Err(reason) => Response::failed(
                    id,
                    ApiError::new("refused", format!("Cannot record: {reason}."), true),
                ),
            }
        }
        "record.stop" => match station.stop_recording() {
            Some(summary) => Response::ok(id, serde_json::to_value(summary).unwrap_or(Value::Null)),
            None => Response::failed(
                id,
                ApiError::new("refused", "Nothing is being recorded.", false),
            ),
        },
        "record.notes" => {
            // what the operator knows and the modem cannot: band, frequency, distance
            let notes = params
                .get("notes")
                .and_then(Value::as_str)
                .map(str::to_owned);
            station.set_record_notes(notes);
            Response::ok(id, json!({ "accepted": true }))
        }
        _ => unreachable!("dispatched here by name"),
    }
}

/// The counters `status` shows, which a recording's sidecar also carries at its end.
#[must_use]
pub fn counters<P: Ptt>(station: &Station<P>) -> Value {
    let stats = station.engine().stats;
    json!({
        "frames_sent": stats.frames_sent,
        "frames_resent": stats.frames_resent,
        "frames_received": stats.frames_received,
        "frames_failed": stats.frames_failed,
        "harq_rescues": stats.harq_rescues,
        "bytes_delivered": stats.bytes_delivered,
        "bursts": stats.bursts,
        "turns": stats.turns,
        "ack_timeouts": stats.ack_timeouts,
        "transmissions": station.stats.transmissions,
        "frames_detected": station.stats.frames_detected,
        "frames_under_own_tx": station.stats.frames_under_own_tx,
        "deferred_for_busy": station.stats.deferred_for_busy,
        "deferred_for_gap": station.stats.deferred_for_gap,
        // faint arrivals taken for the next frame of a burst being acknowledged (ADR-0040)
        "follow_on_heeded": station.stats.follow_on_heeded,
        "watchdog_trips": station.stats.watchdog_trips,
        "beacons_sent": station.stats.beacons_sent,
        "beacons_heard": station.stats.beacons_heard,
        // calls and probes kept unanswered for want of the host program's LISTEN ON (ADR-0026)
        "calls_unanswered": station.stats.calls_unanswered,
        "probes_sent": stats.probes_sent,
        "probes_answered": stats.probes_answered,
        "probe_replies": stats.probe_replies,
        "frames_reencoded": stats.frames_reencoded,
        // a chat's requests for the turn (ADR-0027)
        "turn_requests": stats.turn_requests,
        // the turn offered at the end of a burst, and taken with an acknowledgement (ADR-0047)
        "turn_offers": stats.turn_offers,
        "turns_taken": stats.turns_taken,
        "acceptances_inferred": stats.acceptances_inferred,
    })
}

/// A level reading, as the API reports it.
fn level_json(reading: &crate::station::LevelReading) -> Value {
    json!({
        "rms_dbfs": reading.rms_dbfs,
        "peak_dbfs": reading.peak_dbfs,
        "clipping": reading.clipping,
        "settled": reading.settled,
        "advice": reading.advice(),
    })
}

/// The numbers that change while a link is running.
#[must_use]
pub fn metrics<P: Ptt>(station: &Station<P>) -> Value {
    let busy = station.busy_detector();
    // Null until the detector has heard enough to know: JSON has no way to say "minus
    // infinity", and a client that plotted one as a number would draw a cliff. Not knowing
    // and knowing it is quiet are different things, and the difference is worth showing.
    let level = |value: f64| {
        if busy.settled() && value.is_finite() {
            json!(value)
        } else {
            Value::Null
        }
    };
    let (rate_snr_db, margin_db) = station.engine().rate_readings();
    let last = station.last_frame();
    json!({
        "mode": station.engine().current_mode(),
        "queued_bytes": station.engine().tx_pending_bytes(),
        "noise_floor_db": level(busy.floor_db),
        "level_db": level(busy.level_db),
        // the largest level-over-floor since the last reading: the decision is made forty
        // times a second on a 50 ms quantity, so the excursions that trip the threshold
        // are the ones a twice-a-second sample almost never lands on
        "excess_peak_db": level(busy.excess_peak_db()),
        "channel_busy": station.channel_busy(),
        // which path last marked it: "level" (the threshold) or "frame" (an acquisition)
        "busy_reason": busy.reason().map(|reason| match reason {
            crate::busy::BusyReason::Level { .. } => "level",
            crate::busy::BusyReason::Frame { .. } => "frame",
            crate::busy::BusyReason::Shape { .. } => "shape",
        }),
        // the passband's highest spectral bin over its median, dB: flat noise reads about
        // 6, a narrowband signal 15 and up, and the AGC cannot change it
        "shape_db": level(busy.shape_db),
        "transmitting": station.transmitting(),
        "receiving": station.receiving(),
        "audio": level_json(&station.audio_level()),
        // the peak of the last burst this station transmitted, after the drive level: what
        // the rig's ALC was actually shown, which a tune tone understates by 3-4.6 dB
        "tx_peak_dbfs": station.tx_peak_dbfs(),
        // the last frame the receiver found: its SNR is the reading an operator calls
        // "the SNR", its offset is what the other station's dial is off by
        "snr_db": last.map(|f| f.snr_db),
        "cfo_hz": last.and_then(|f| {
            crate::station::reported_cfo(f.decoded, f.confidence, f.detect_confidence, f.cfo_hz)
        }),
        "last_frame_s": last.map(|f| f.t_s),
        // what the other station reports hearing this one at
        "peer_snr_db": station.engine().peer_snr_db(),
        // the rate controller's own view: the smoothed SNR it acts on and its margin
        "rate_snr_db": rate_snr_db,
        "margin_db": margin_db,
        "throughput_bps": station.throughput_bps(),
        // the receiver's passband, learned from the noise, against the width the modem needs:
        // materially narrower means the radio's filter is set too narrow for the signal
        "rx_passband_hz": station.rx_passband_hz(),
        "occupied_hz": station.occupied_bandwidth_hz(),
        "link": link_json(station),
    })
}

/// The session's account, or null when no session is up.
fn link_json<P: Ptt>(station: &Station<P>) -> Value {
    match station.link() {
        Some(link) => json!({
            "started_s": link.started_s,
            "seconds": station.now() - link.started_s,
            "remote": station.engine().remote_call,
            "bytes_sent": link.bytes_sent,
            "bytes_received": link.bytes_received,
        }),
        None => Value::Null,
    }
}

/// One frame, as the `frame` event and the `constellation` method report it.
#[must_use]
pub fn frame_json(frame: &crate::station::FrameReport) -> Value {
    json!({
        "t_s": frame.t_s,
        "kind": frame.kind,
        "mode": frame.mode,
        "rv": frame.rv,
        "snr_db": frame.snr_db,
        "cfo_hz": crate::station::reported_cfo(
            frame.decoded,
            frame.confidence,
            frame.detect_confidence,
            frame.cfo_hz,
        ),
        "confidence": frame.confidence,
        "detect_confidence": frame.detect_confidence,
        "decoded": frame.decoded,
        "bytes": frame.bytes,
        "from": frame.from,
        "to": frame.to,
        "control": frame.control,
        "bandwidth_hz": frame.bandwidth_hz,
        "follows": frame.follows,
    })
}

/// A spectrum of what the sound card is delivering, for the panel to draw.
///
/// Polled rather than streamed: it costs a transform per call and nothing otherwise,
/// so a gateway with nobody watching pays nothing. `bins_db` is empty until a whole
/// window of audio has been heard.
fn spectrum<P: Ptt>(station: &Station<P>) -> Value {
    let params = station.params();
    let spectrum = station.spectrum();
    let half = params.bandwidth.hz() as f64 / 2.0;
    json!({
        "bin_hz": spectrum.as_ref().map_or(0.0, |s| s.bin_hz),
        "bins_db": spectrum.as_ref().map_or(&[][..], |s| s.bins_db.as_slice()),
        // where this modem's signal sits, so the display can mark its edges
        "passband_hz": [params.centre_hz - half, params.centre_hz + half],
        "transmitting": station.transmitting(),
    })
}

/// The last frame's equalised constellation, with the frame it came from.
fn constellation<P: Ptt>(station: &Station<P>) -> Value {
    match station.constellation() {
        Some((frame, symbols)) => json!({
            "frame": frame_json(frame),
            "points": symbols
                .iter()
                .map(|(i, q)| [(*i * 1000.0).round() / 1000.0, (*q * 1000.0).round() / 1000.0])
                .collect::<Vec<_>>(),
        }),
        None => json!({ "frame": Value::Null, "points": [] }),
    }
}

/// What this modem can do, so a client discovers the mode table instead of hard-coding it.
///
/// This is what keeps the control API free of any mention of a modulation: an FM physical
/// layer would answer here with its own table and nothing else would change. The table
/// is the one of the waveform the station runs (`bandwidth_hz`); `bandwidths_hz` lists
/// what this version has.
#[must_use]
pub fn capabilities(params: aether_phy::waveform::WaveformParams) -> Value {
    use aether_phy::waveform::Bandwidth;

    let air = aether_phy::modes::air_interface(params);
    let ladder = air.ladder();
    let (thresholds, payload): (&[f64], &[usize]) = match params.bandwidth {
        Bandwidth::Narrow500 => (
            &aether_link::rate::NARROW_AWGN_THRESHOLD_DB,
            &aether_link::rate::NARROW_PAYLOAD_BYTES,
        ),
        _ => (
            &aether_link::AWGN_THRESHOLD_DB,
            &aether_link::rate::PAYLOAD_BYTES,
        ),
    };
    // the rungs of the air's ladder: the tone floor's two (ADR-0013), whose frames are five
    // times as long as an ordinary one, then the OFDM modes
    let modes: Vec<Value> = ladder
        .iter()
        .enumerate()
        .map(|(index, rung)| {
            json!({
                "index": index,
                "name": rung.name(),
                "payload_bytes": rung.payload_bytes(),
                "net_bit_rate": rung.net_bps(),
                "floor": rung.is_floor(),
                "threshold_db": thresholds[index],
            })
        })
        .collect();
    json!({
        "api_version": "0.1",
        "phy": "aether-hf",
        "bandwidth_hz": params.bandwidth.hz(),
        "bandwidths_hz": [2300, 500],
        "modes": modes,
        // by bytes per second, since a floor rung's frame is five times as long
        "usable_modes": aether_link::rate::usable_modes_by_rate(
            thresholds,
            payload,
            &ladder
                .iter()
                .map(aether_phy::Rung::duration_s)
                .collect::<Vec<f64>>(),
        ),
        "reports_preambles": true,
        "snr_reference_hz": 3000,
    })
}

/// Base64, exposed so the socket layer can encode received payloads the same way.
#[must_use]
pub fn to_base64(bytes: &[u8]) -> String {
    encode(bytes)
}

/// The inverse, for a client reading `data` events.
#[must_use]
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    decode(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ptt::NullPtt,
        station::{Station, StationConfig},
    };

    fn station() -> Station<NullPtt> {
        Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        )
    }

    fn call(station: &mut Station<NullPtt>, method: &str, params: Value) -> Response {
        dispatch(
            station,
            &Request {
                id: Some("1".into()),
                method: method.to_owned(),
                params,
            },
        )
    }

    #[test]
    fn base64_round_trips_including_the_awkward_lengths() {
        for length in 0..40usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 37 % 256) as u8).collect();
            let text = encode(&bytes);
            assert_eq!(decode(&text).as_deref(), Some(bytes.as_slice()), "{length}");
        }
        // the canonical vectors, so an off-by-one in the padding cannot hide
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(decode("Zm9vYmFy").as_deref(), Some(&b"foobar"[..]));
        assert_eq!(decode("Zm9vYmFy\n").as_deref(), Some(&b"foobar"[..]));
        assert!(decode("not base64!").is_none());
    }

    #[test]
    fn status_describes_an_idle_station() {
        let mut station = station();
        let response = call(&mut station, "status", json!({}));
        assert!(response.ok);
        let result = response.result.expect("result");
        assert_eq!(result["state"], "idle");
        assert_eq!(result["role"], "none");
        assert_eq!(result["callsign"], "W4ODA");
        assert_eq!(result["transmitting"], false);
        assert!(result["counters"]["frames_sent"].is_number());
    }

    #[test]
    fn counters_reset_zeroes_the_tallies_and_leaves_the_station_idle() {
        let mut station = station();
        station.stats.transmissions = 7;
        station.stats.frames_detected = 42;
        station.stats.beacons_heard = 3;
        let response = call(&mut station, "counters.reset", json!({}));
        assert!(response.ok);
        let result = response.result.expect("result");
        assert_eq!(result["counters"]["transmissions"], 0);
        assert_eq!(result["counters"]["frames_detected"], 0);
        assert_eq!(result["counters"]["frames_sent"], 0);
        // resetting a display tally is not a session change
        assert_eq!(station.engine().state(), aether_link::State::Idle);
        assert!(is_mutating("counters.reset"));
    }

    #[test]
    fn status_says_where_recordings_go() {
        let mut station = station();
        // the default station has no record directory; a configured one is reported as-is
        let response = call(&mut station, "status", json!({}));
        assert!(response.result.expect("result")["recordings_dir"].is_null());
    }

    #[test]
    fn capabilities_carries_the_mode_table_and_names_no_modulation_in_its_shape() {
        let caps = capabilities(aether_phy::waveform::WIDE_2300);
        let modes = caps["modes"].as_array().expect("modes");
        // the rungs of the ladder: the tone floor's six (ADR-0013, ADR-0014), then the
        // fourteen OFDM modes
        assert_eq!(modes.len(), aether_phy::modes::WIDE.n_rungs());
        assert_eq!(modes[0]["floor"], true);
        assert_eq!(modes[2]["name"], "tone50-51");
        assert_eq!(modes[5]["floor"], true);
        assert_eq!(modes[6]["name"], "BPSK-1/5");
        assert_eq!(modes[6]["floor"], false);
        assert!(modes[0]["threshold_db"].is_number());
        assert!(modes[0]["payload_bytes"].is_number());
        assert_eq!(caps["snr_reference_hz"], 3000);
        // the keys are PHY-agnostic; the values are this PHY's
        for key in [
            "api_version",
            "phy",
            "bandwidths_hz",
            "modes",
            "usable_modes",
        ] {
            assert!(caps.get(key).is_some(), "capabilities is missing {key}");
        }
    }

    #[test]
    fn probe_needs_a_callsign_and_goes_out_once_at_a_time() {
        let mut station = station();
        let missing = call(&mut station, "probe", json!({}));
        assert_eq!(missing.error.expect("refused").code, "bad_params");
        let first = call(&mut station, "probe", json!({"remote": "KK4XYZ"}));
        assert!(first.ok, "{:?}", first.error);
        assert_eq!(station.engine().stats.probes_sent, 1);
        // one at a time: the second is refused, and told to try again later
        let second = call(&mut station, "probe", json!({"remote": "KK4XYZ"}));
        let error = second.error.expect("refused");
        assert_eq!(error.code, "not_idle");
        assert!(error.retryable, "{error:?}");
        assert!(is_mutating("probe"));
        let counters = counters(&station);
        assert_eq!(counters["probes_sent"], 1);
        assert_eq!(counters["probe_replies"], 0);
    }

    #[test]
    fn connect_needs_a_callsign_and_refuses_a_second_session() {
        let mut station = station();
        let missing = call(&mut station, "connect", json!({}));
        assert!(!missing.ok);
        assert_eq!(missing.error.expect("error").code, "bad_params");

        let first = call(&mut station, "connect", json!({"remote": "KK4XYZ"}));
        assert!(first.ok, "{:?}", first.error);

        let second = call(&mut station, "connect", json!({"remote": "M0ABC"}));
        assert!(!second.ok);
        let error = second.error.expect("error");
        assert_eq!(error.code, "already_connected");
        assert!(
            error.message.contains("M0ABC"),
            "the message does not say which call failed: {}",
            error.message
        );
    }

    #[test]
    fn an_answer_only_station_says_so_rather_than_that_it_is_busy() {
        // `already_connected` for a call and a retryable `not_idle` for a beacon sent a client
        // to wait for a session that was not there: the setting refuses, and waiting will not
        // change it
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                answer_only: true,
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        for (method, params) in [
            ("connect", json!({"remote": "KK4XYZ"})),
            ("beacon", json!({})),
            ("beacon.every", json!({"minutes": 15})),
            ("probe", json!({"remote": "KK4XYZ"})),
            ("test.start", json!({"remote": "KK4XYZ"})),
            ("datagram.send", json!({"data": to_base64(b"CQ")})),
        ] {
            let error = call(&mut station, method, params).error.expect("refused");
            assert_eq!(error.code, "refused", "{method}: {error:?}");
            assert!(!error.retryable, "{method}");
            assert!(error.message.contains("answer-only"), "{method}: {error:?}");
        }
        assert_eq!(station.state(), aether_link::State::Idle);
        assert!(!station.test_running());
    }

    #[test]
    fn an_empty_datagram_is_refused_rather_than_sent() {
        // base64 of nothing was queued and keyed: a callsign and a type on the air, which
        // every station that decoded them dropped
        let mut station = station();
        for data in ["", "===="] {
            let response = call(&mut station, "datagram.send", json!({ "data": data }));
            let error = response.error.expect("refused");
            assert_eq!(error.code, "bad_params", "{data:?}: {error:?}");
            assert!(!error.retryable);
            assert!(error.message.contains("empty"), "{}", error.message);
        }
        let status = call(&mut station, "status", json!({}))
            .result
            .expect("status");
        assert_eq!(status["datagrams"]["queued"], 0);
        // one byte is a frame
        let one = call(
            &mut station,
            "datagram.send",
            json!({ "data": to_base64(b"x") }),
        );
        assert!(one.ok, "{:?}", one.error);
    }

    #[test]
    fn what_a_session_holds_up_is_worth_asking_again() {
        let mut busy = crate::station::tests_support::connected_station();
        for (method, params) in [
            ("beacon", json!({})),
            ("probe", json!({"remote": "M0ABC"})),
            ("test.start", json!({"remote": "M0ABC"})),
        ] {
            let error = call(&mut busy, method, params).error.expect("refused");
            assert_eq!(error.code, "not_idle", "{method}: {error:?}");
            assert!(error.retryable, "{method}");
        }
        let error = call(&mut busy, "connect", json!({"remote": "M0ABC"}))
            .error
            .expect("refused");
        assert_eq!(error.code, "already_connected");
    }

    #[test]
    fn a_test_session_the_rules_refuse_says_so_with_the_decision() {
        // as a call is refused: the decision whole, for a client that shows it — a Test
        // session's refusal used to be `not_idle`, with only the decision's summary
        let mut station = Station::new(
            StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                // no profile chosen: nothing may go on the air
                regulatory: crate::regulatory::Settings {
                    profile: String::new(),
                    ..crate::regulatory::Settings::unchecked()
                },
                ..StationConfig::default()
            },
            NullPtt::default(),
            1,
        );
        for method in ["test.start", "connect"] {
            let response = call(&mut station, method, json!({"remote": "KK4XYZ"}));
            let error = response.error.expect("refused");
            assert_eq!(error.code, "regulatory", "{method}: {error:?}");
            assert!(error.message.starts_with("Cannot "), "{}", error.message);
            let result = response.result.expect("the decision");
            assert_eq!(
                result["decision"]["code"], "no_profile",
                "{method}: {result}"
            );
        }
        assert!(!station.test_running());
    }

    #[test]
    fn disconnect_and_abort_say_which_they_are() {
        // the distinction matters to an operator watching a transfer, which is why the spec
        // has both, so the reply has to make it visible
        let mut station = station();
        call(&mut station, "connect", json!({"remote": "KK4XYZ"}));
        let orderly = call(&mut station, "disconnect", json!({}));
        assert_eq!(orderly.result.expect("result")["orderly"], true);

        call(&mut station, "connect", json!({"remote": "KK4XYZ"}));
        let abrupt = call(&mut station, "abort", json!({}));
        assert_eq!(abrupt.result.expect("result")["orderly"], false);
    }

    #[test]
    fn send_refuses_data_with_nowhere_to_go() {
        // accepting it would look like success and lose the data at the first disconnect
        let mut station = station();
        let response = call(&mut station, "send", json!({"data": to_base64(b"hello")}));
        assert!(!response.ok);
        assert_eq!(response.error.expect("error").code, "not_connected");
    }

    #[test]
    fn send_rejects_data_that_is_not_base64() {
        let mut station = station();
        let response = call(&mut station, "send", json!({"data": "not base64!"}));
        assert!(!response.ok);
        assert_eq!(response.error.expect("error").code, "bad_params");
    }

    #[test]
    fn the_displays_read_the_modem_and_never_a_guess() {
        let mut station = station();
        // nothing heard: the spectrum has no bins and the constellation no points, and a
        // client can tell that from a quiet channel
        let response = call(&mut station, "spectrum", json!({}));
        assert!(response.ok);
        let result = response.result.expect("result");
        assert_eq!(result["bins_db"].as_array().map(Vec::len), Some(0));
        let width =
            result["passband_hz"][1].as_f64().unwrap() - result["passband_hz"][0].as_f64().unwrap();
        assert!((width - 2300.0).abs() < 1e-9, "{width}");
        let response = call(&mut station, "constellation", json!({}));
        assert!(response.ok);
        let result = response.result.expect("result");
        assert!(result["frame"].is_null());
        assert_eq!(result["points"].as_array().map(Vec::len), Some(0));
        let response = call(&mut station, "status", json!({}));
        let result = response.result.expect("result");
        assert!(result["link"].is_null(), "no session, no account");
        assert!(result["metrics"]["snr_db"].is_null(), "no frame, no SNR");
        assert_eq!(result["metrics"]["receiving"], false);
        assert_eq!(result["metrics"]["throughput_bps"], 0.0);
        assert!(
            result["frequency_hz"].is_null(),
            "a null keying line cannot ask"
        );
        assert_eq!(result["host"]["enabled"], false);

        // a window of a tone: the spectrum has it
        let fs = aether_phy::waveform::WIDE_2300.audio_rate as f64;
        let tone: Vec<f32> = (0..8192)
            .map(|n| 0.3 * (std::f64::consts::TAU * 1000.0 * f64::from(n) / fs).sin() as f32)
            .collect();
        station.capture(&tone).expect("capture");
        let result = call(&mut station, "spectrum", json!({}))
            .result
            .expect("result");
        let bins = result["bins_db"].as_array().expect("bins");
        assert!(!bins.is_empty(), "bins is empty");
        let bin_hz = result["bin_hz"].as_f64().expect("bin_hz");
        let peak = bins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.as_f64().unwrap().total_cmp(&b.1.as_f64().unwrap()))
            .map(|(i, _)| i as f64 * bin_hz)
            .expect("peak");
        assert!((peak - 1000.0).abs() < 2.0 * bin_hz, "peak at {peak} Hz");
        assert!(
            !is_mutating("spectrum") && !is_mutating("constellation") && !is_mutating("heard.list")
        );
        assert!(is_mutating("heard.clear"));
    }

    #[test]
    fn a_beacon_repeats_when_asked_and_status_says_so() {
        let mut station = station();
        let request = |method: &str, params: Value| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params,
        };
        assert!(is_mutating("beacon.every"));
        let response = dispatch_with(
            &mut station,
            None,
            &request("beacon.every", json!({"minutes": 1})),
        );
        let error = response.error.expect("a minute is refused");
        assert_eq!(error.code, "bad_params");
        let response = dispatch_with(
            &mut station,
            None,
            &request("beacon.every", json!({"minutes": 15})),
        );
        assert_eq!(response.result.expect("accepted")["every_s"], 900.0);
        let response = dispatch_with(&mut station, None, &request("status", json!({})));
        let result = response.result.expect("status");
        assert_eq!(result["beacon"]["every_s"], 900.0);
        assert!(result["identifier"]["enabled"].is_boolean());
        let response = dispatch_with(
            &mut station,
            None,
            &request("beacon.every", json!({"minutes": 0})),
        );
        assert!(response.result.expect("stopped")["every_s"].is_null());
    }

    #[test]
    fn a_zip_of_the_files_another_operator_needs_is_written_beside_the_configuration() {
        // KE4QCM, 2026-10-05: the files of the side that was not heard were a PowerShell line
        // pasted into an email. `share.prepare` writes the same zip under `shared/`
        let dir = std::env::temp_dir().join(format!("aether-share-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("aetherd.log"), "a run\n").expect("write");
        let mut station = station();
        let mut daemon = daemon();
        daemon.path = dir.join("station.toml");
        let ask = |params: Value| Request {
            id: Some("1".into()),
            method: "share.prepare".to_owned(),
            params,
        };
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &ask(json!({"hours": 3, "remote": "kk4oda-1"})),
        );
        let result = response.result.expect("result");
        let path = std::path::PathBuf::from(result["path"].as_str().expect("path"));
        assert!(path.is_file(), "{path:?}");
        assert_eq!(path.parent(), Some(dir.join("shared").as_path()));
        assert_eq!(
            std::fs::metadata(&path).expect("zip").len(),
            result["bytes"].as_u64().expect("bytes")
        );
        let files: Vec<&str> = result["files"]
            .as_array()
            .expect("files")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(files, ["aetherd.log", "diagnostics.json", "README.txt"]);
        assert_eq!(result["remote"], "KK4ODA-1");
        assert_eq!(result["sessions"], 0);
        assert_eq!(result["audio"], false);
        // a period outside 15 minutes to 14 days is refused
        let refused = dispatch_with(&mut station, Some(&mut daemon), &ask(json!({"hours": 0})));
        assert_eq!(refused.error.expect("refused").code, "bad_params");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shared_zip_goes_to_the_upload_script_the_request_named() {
        // the request link carries the asking station's upload script and a code; the zip
        // goes there on a thread of its own and `status.upload` follows it
        struct Script;
        impl crate::upload::Http for Script {
            fn post(&self, _: &str, body: &str) -> Result<crate::upload::Reply, String> {
                let asked: Value = serde_json::from_str(body).expect("json");
                let answer = if asked["action"] == "begin" {
                    json!({"ok": true, "upload_url": "https://www.googleapis.com/upload/s", "to": "KK4ODA"})
                } else {
                    json!({"ok": true, "link": "https://drive.google.com/file/d/F/view"})
                };
                Ok(crate::upload::Reply {
                    status: 200,
                    body: answer.to_string(),
                    ..crate::upload::Reply::default()
                })
            }
            fn get(&self, _: &str) -> Result<crate::upload::Reply, String> {
                Err("not asked".into())
            }
            fn put(&self, _: &str, _: &str, _: &[u8]) -> Result<crate::upload::Reply, String> {
                Ok(crate::upload::Reply {
                    status: 201,
                    body: json!({"id": "F"}).to_string(),
                    ..crate::upload::Reply::default()
                })
            }
        }
        let dir = std::env::temp_dir().join(format!("aether-upload-api-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shared")).expect("temp dir");
        std::fs::write(dir.join("shared").join("aether-W4ODA-x.zip"), b"PK zip").expect("zip");
        let mut station = station();
        let mut daemon = daemon();
        daemon.path = dir.join("station.toml");
        daemon.upload_http = || Box::new(Script);
        let ask = |params: Value| Request {
            id: Some("1".into()),
            method: "share.upload".to_owned(),
            params,
        };
        let good = json!({
            "name": "aether-W4ODA-x.zip",
            "endpoint": "https://script.google.com/macros/s/AKfy/exec",
            "code": "W4TGA-7k2m9q",
            "note": "last night's Test",
        });
        for (key, bad, code) in [
            ("endpoint", "https://example.org/upload", "bad_params"),
            ("code", "x", "bad_params"),
            ("name", "../station.toml", "not_found"),
            ("name", "aether-W4ODA-gone.zip", "not_found"),
        ] {
            let mut params = good.clone();
            params[key] = json!(bad);
            let refused = dispatch_with(&mut station, Some(&mut daemon), &ask(params));
            assert_eq!(refused.error.expect("refused").code, code, "{key} {bad}");
        }
        // no code at all is fine: the project's upload needs none
        let mut codeless = good.clone();
        codeless["code"] = json!("");
        let accepted = dispatch_with(&mut station, Some(&mut daemon), &ask(codeless));
        assert!(accepted.error.is_none(), "{:?}", accepted.error);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while daemon.upload.busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let started = dispatch_with(&mut station, Some(&mut daemon), &ask(good));
        assert_eq!(started.result.expect("started")["bytes"], 6);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let status = loop {
            let status = dispatch_with(
                &mut station,
                Some(&mut daemon),
                &Request {
                    id: Some("2".into()),
                    method: "status".to_owned(),
                    params: json!({}),
                },
            )
            .result
            .expect("status");
            if status["upload"]["state"] == "done" || std::time::Instant::now() > deadline {
                break status;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(status["upload"]["state"], "done", "{}", status["upload"]);
        assert_eq!(status["upload"]["sent"], 6);
        assert_eq!(status["upload"]["to"], "KK4ODA");
        assert_eq!(
            status["upload"]["link"],
            "https://drive.google.com/file/d/F/view"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn debug_mode_sends_the_sessions_recording_with_its_audio_to_the_project() {
        // what the script was asked and sent, for the test to read: the fake is a plain
        // function, so it keeps them here
        static SEEN: std::sync::Mutex<Vec<(String, Vec<u8>)>> = std::sync::Mutex::new(Vec::new());
        struct Script;
        impl crate::upload::Http for Script {
            fn post(&self, url: &str, body: &str) -> Result<crate::upload::Reply, String> {
                SEEN.lock()
                    .expect("seen")
                    .push((url.to_owned(), body.as_bytes().to_vec()));
                let asked: Value = serde_json::from_str(body).expect("json");
                let answer = if asked["action"] == "begin" {
                    json!({"ok": true, "upload_url": "https://www.googleapis.com/upload/s", "to": "KK4ODA"})
                } else {
                    json!({"ok": true, "link": "https://drive.google.com/file/d/F/view"})
                };
                Ok(crate::upload::Reply {
                    status: 200,
                    body: answer.to_string(),
                    ..crate::upload::Reply::default()
                })
            }
            fn get(&self, _: &str) -> Result<crate::upload::Reply, String> {
                Err("not asked".into())
            }
            fn put(&self, url: &str, _: &str, body: &[u8]) -> Result<crate::upload::Reply, String> {
                SEEN.lock()
                    .expect("seen")
                    .push((url.to_owned(), body.to_vec()));
                Ok(crate::upload::Reply {
                    status: 201,
                    body: json!({"id": "F"}).to_string(),
                    ..crate::upload::Reply::default()
                })
            }
        }
        let dir = std::env::temp_dir().join(format!("aether-debug-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let recordings = dir.join("recordings");
        std::fs::create_dir_all(&recordings).expect("temp dir");
        let stem = "20261007-140000_KK4ODA-10_ND1J";
        std::fs::write(
            recordings.join(format!("{stem}.json")),
            r#"{"session":{"remote":"ND1J"}}"#,
        )
        .expect("sidecar");
        std::fs::write(recordings.join(format!("{stem}.wav")), b"RIFF-the-audio").expect("wav");
        // another session's recording stays out of it
        std::fs::write(
            recordings.join("20261007-130000_KK4ODA-10_W4TGA.json"),
            r#"{"session":{"remote":"W4TGA"}}"#,
        )
        .expect("sidecar");
        let mut station = station();
        station.set_recording(Some(recordings.clone()), false, "");
        let mut daemon = daemon();
        daemon.path = dir.join("station.toml");
        daemon.upload_http = || Box::new(Script);
        let pending = [crate::debug::Pending {
            recording: stem.into(),
            remote: "ND1J".into(),
            started_ms: 1_791_381_600_000,
        }];
        let name = start_debug_upload(&mut station, &mut daemon, &pending).expect("started");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while daemon.upload.busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let progress = daemon.upload.progress();
        assert_eq!(progress.state, "done", "{progress:?}");
        assert_eq!(progress.name.as_deref(), Some(name.as_str()));
        let seen = SEEN.lock().expect("seen").clone();
        // to the project's script, with no code
        let begin: Value = serde_json::from_slice(&seen[0].1).expect("begin");
        assert_eq!(seen[0].0, crate::upload::PROJECT_ENDPOINT);
        assert_eq!(begin["code"], "");
        // the zip went up whole: the session's sidecar and audio, not the other session's
        let zip: Vec<u8> = seen
            .iter()
            .filter(|(url, _)| url.starts_with("https://www.googleapis.com/"))
            .flat_map(|(_, body)| body.clone())
            .collect();
        let has = |needle: &[u8]| zip.windows(needle.len()).any(|w| w == needle);
        assert!(has(format!("recordings/{stem}.wav").as_bytes()));
        assert!(has(b"RIFF-the-audio"), "the audio is stored, not deflated");
        assert!(has(format!("recordings/{stem}.json").as_bytes()));
        assert!(!has(b"W4TGA.json"));
        assert!(has(b"diagnostics.json") && has(b"README.txt"));
        // and the status says it is on
        let status = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &Request {
                id: Some("1".into()),
                method: "status".to_owned(),
                params: json!({}),
            },
        )
        .result
        .expect("status");
        assert_eq!(status["debug"]["on"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_panel_sends_to_the_same_project_address_as_debug_mode() {
        let panel = include_str!("../../../../app/ui/app.js");
        assert!(
            panel.contains(&format!(
                "const PROJECT_UPLOAD_URL = \"{}\";",
                crate::upload::PROJECT_ENDPOINT
            )),
            "app/ui/app.js's PROJECT_UPLOAD_URL and upload::PROJECT_ENDPOINT differ"
        );
        assert!(crate::upload::endpoint_allowed(
            crate::upload::PROJECT_ENDPOINT
        ));
    }

    #[test]
    fn the_stations_heard_are_listed_and_forgotten() {
        let mut station = station();
        let mut daemon = daemon();
        let request = |method: &str| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params: json!({}),
        };
        let response = dispatch_with(&mut station, Some(&mut daemon), &request("heard.list"));
        let result = response.result.expect("result");
        assert_eq!(result["stations"].as_array().map(Vec::len), Some(0));
        assert_eq!(result["limit"], crate::heard::LIMIT);
        daemon.heard.note(crate::heard::Sighting {
            callsign: "W4TGA".to_owned(),
            at_ms: 1_700_000_000_000,
            snr_db: 7.5,
            mode: Some(2),
            frequency_hz: Some(7_101_000),
            activity: crate::heard::Activity::Beacon,
            detail: None,
            bandwidth_hz: Some(500),
        });
        let response = dispatch_with(&mut station, Some(&mut daemon), &request("heard.list"));
        let result = response.result.expect("result");
        let stations = result["stations"].as_array().expect("stations");
        assert_eq!(stations.len(), 1);
        assert_eq!(stations[0]["callsign"], "W4TGA");
        assert_eq!(stations[0]["activity"], "beacon");
        assert_eq!(stations[0]["frequency_hz"], 7_101_000);
        assert_eq!(stations[0]["best_snr_db"], 7.5);
        let response = dispatch_with(&mut station, Some(&mut daemon), &request("heard.clear"));
        assert_eq!(response.result.expect("result")["cleared"], 1);
        let response = dispatch_with(&mut station, Some(&mut daemon), &request("heard.list"));
        assert_eq!(
            response.result.expect("result")["stations"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        // without the daemon's state there is no list, and the answer is an empty one
        let response = dispatch(&mut station, &request("heard.list"));
        assert!(response.ok);
        assert_eq!(
            response.result.expect("result")["stations"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn a_message_reference_is_checked_and_its_deliveries_are_in_status() {
        let mut station = station();
        let request = |params: Value| Request {
            id: Some("1".into()),
            method: "send".to_owned(),
            params,
        };
        let data = super::encode(b"hello\n");
        for bad in [
            json!(""),
            json!(7),
            json!("with space"),
            json!("x".repeat(65)),
        ] {
            let response = dispatch(&mut station, &request(json!({"data": data, "ref": bad})));
            assert_eq!(response.error.expect("refused").code, "bad_params", "{bad}");
        }
        // a good reference with no session: refused as any send is
        let response = dispatch(&mut station, &request(json!({"data": data, "ref": "m1"})));
        assert_eq!(response.error.expect("refused").code, "not_connected");
        let status = dispatch(
            &mut station,
            &Request {
                id: Some("2".into()),
                method: "status".to_owned(),
                params: json!({}),
            },
        );
        let sent = &status.result.expect("status")["sent"];
        assert_eq!(sent["pending"].as_array().map(Vec::len), Some(0));
        assert_eq!(sent["recent"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn the_sessions_are_listed_by_station_and_forgotten() {
        let mut station = station();
        let mut daemon = daemon();
        let request = |method: &str, params: Value| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params,
        };
        let session = |remote: &str, ended_ms: u64| crate::sessions::Session {
            remote: remote.to_owned(),
            started_ms: 0,
            ended_ms,
            duration_s: 95.5,
            role: crate::sessions::Role::Caller,
            bandwidth_hz: 500,
            frequency_hz: Some(3_588_000),
            bytes_sent: 1024,
            bytes_acked: 1100,
            bytes_received: 0,
            end: "closed".to_owned(),
            snr_db: Some(5.0),
            best_snr_db: Some(7.5),
            heard_there_db: Some(-1.0),
            top_rung_sent: Some(5),
            top_rung_heard: Some(7),
            test: true,
            host: false,
            recording: None,
        };
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &request("sessions.list", json!({})),
        );
        let result = response.result.expect("result");
        assert_eq!(result["sessions"].as_array().map(Vec::len), Some(0));
        assert_eq!(result["limit"], crate::sessions::LIMIT);
        daemon.sessions.add(session("ND1J", 1_000));
        daemon.sessions.add(session("W4TGA", 2_000));
        daemon.sessions.add(session("ND1J", 3_000));
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &request("sessions.list", json!({ "remote": "nd1j" })),
        );
        let result = response.result.expect("result");
        let sessions = result["sessions"].as_array().expect("sessions");
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0]["ended_ms"], 3_000);
        assert_eq!(sessions[0]["role"], "caller");
        assert_eq!(sessions[0]["top_rung_sent"], 5);
        assert_eq!(sessions[0]["test"], true);
        assert!(is_mutating("sessions.clear") && !is_mutating("sessions.list"));
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &request("sessions.clear", json!({})),
        );
        assert_eq!(response.result.expect("result")["cleared"], 3);
        let unexpected = daemon.sessions.sessions(None);
        assert!(unexpected.is_empty(), "{unexpected:?}");
        // without the daemon's state there is no history, and the answer is an empty one
        let response = dispatch(&mut station, &request("sessions.list", json!({})));
        assert_eq!(
            response.result.expect("result")["sessions"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
    }

    fn daemon() -> DaemonState {
        let mut config = crate::config::Config::parse(crate::config::EXAMPLE).expect("example");
        config.control.token = Some("hunter2".to_owned());
        let mut daemon = DaemonState::new(
            config,
            std::path::PathBuf::from("station.toml"),
            crate::log::Log::memory(50),
        );
        // and no file under the working directory for the stations heard, the dials or
        // the profiles
        daemon.heard = crate::heard::HeardList::open(None);
        daemon.sessions = crate::sessions::SessionLog::open(None);
        daemon.memories = crate::memories::Memories::open(None);
        daemon.profiles = crate::profile::Store::open(None);
        // not the machine's own: enumerating audio devices on a machine with no audio
        // service crashes inside the platform API, and a test must not depend on a sound card
        daemon.devices = || {
            json!({
                "devices": [{"name": "USB Audio CODEC", "input": true, "output": true}],
                "serial_ports": [{"name": "COM3", "description": "Prolific PL2303GC USB Serial COM Port"}],
            })
        };
        daemon
    }

    #[test]
    fn the_devices_are_listed_off_the_loop_and_a_new_list_is_told_once() {
        // listing on the run loop held a profile switch for seconds (2026-09-26): the daemon
        // lists on a thread of its own, answers with the last listing, and tells the clients
        // of a new one only when it differs
        use std::sync::atomic::{AtomicUsize, Ordering};
        static LISTINGS: AtomicUsize = AtomicUsize::new(0);
        fn listing() -> Value {
            let n = LISTINGS.fetch_add(1, Ordering::SeqCst);
            let mut devices =
                vec![json!({"name": "USB Audio CODEC", "input": true, "output": true})];
            if n >= 2 {
                // plugged in after the panel asked
                devices.push(json!({"name": "FTDX10 CODEC", "input": true, "output": true}));
            }
            json!({ "devices": devices, "serial_ports": [] })
        }
        let finished = |daemon: &DaemonState, n: u64| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while daemon.device_list.finished() < n {
                assert!(
                    std::time::Instant::now() < deadline,
                    "listing {n} never finished"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        let mut station = station();
        let mut daemon = daemon();
        daemon.devices = listing;
        // the start's listing, then a panel's `devices.list`: the first is waited for, and
        // another is begun
        daemon.refresh_devices();
        let answer = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &Request {
                id: Some("1".into()),
                method: "devices.list".into(),
                params: json!({}),
            },
        );
        assert!(answer.ok, "{answer:?}");
        let result = answer.result.expect("result");
        assert_eq!(result["devices"].as_array().map(Vec::len), Some(1));
        finished(&daemon, 2);
        assert_eq!(
            daemon.take_new_devices(),
            None,
            "the same list again is not news"
        );
        // a device plugged in: the next listing is news, once
        daemon.refresh_devices();
        finished(&daemon, 3);
        let news = daemon.take_new_devices().expect("the new list");
        assert_eq!(news["devices"][1]["name"], "FTDX10 CODEC");
        assert_eq!(daemon.take_new_devices(), None, "told twice");
        // and a profile check reads the listing without listing again
        let before = LISTINGS.load(Ordering::SeqCst);
        let inventory = crate::profile::Inventory::from_json(&daemon.inventory());
        assert_eq!(inventory.inputs.len(), 2);
        assert_eq!(
            LISTINGS.load(Ordering::SeqCst),
            before,
            "listed on the caller's thread"
        );
    }

    #[test]
    fn a_station_keyed_by_its_host_starts_nothing_with_no_host_attached() {
        // with nobody to key the radio, the audio would go to a radio nobody keys (ADR-0025)
        let mut station = station();
        let mut daemon = daemon();
        daemon.config.ptt = crate::config::PttConfig::Host { lead_ms: 150 };
        let request = |method: &str, params: Value| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params,
        };
        for (method, params) in [
            ("connect", json!({"remote": "KK4XYZ"})),
            ("beacon", json!({})),
            ("probe", json!({"remote": "KK4XYZ"})),
            ("tune", json!({"duration_s": 3.0})),
            ("beacon.every", json!({"minutes": 30})),
        ] {
            let response = dispatch_with(&mut station, Some(&mut daemon), &request(method, params));
            assert!(!response.ok, "{method} was accepted");
            let error = response.error.expect("error");
            assert_eq!(error.message, NO_HOST_TO_KEY, "{method}");
        }
        // stopping, and anything that does not transmit, is always allowed
        for (method, params) in [
            ("tune", json!({"duration_s": 0.0})),
            ("beacon.every", json!({"minutes": 0})),
            ("status", json!({})),
        ] {
            let response = dispatch_with(&mut station, Some(&mut daemon), &request(method, params));
            assert!(
                response
                    .error
                    .is_none_or(|error| error.message != NO_HOST_TO_KEY),
                "{method} was refused for want of a host"
            );
        }
        // a station that keys the radio itself is not asked for a host
        daemon.config.ptt = crate::config::PttConfig::None;
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &request("beacon", json!({})),
        );
        assert!(
            response
                .error
                .is_none_or(|error| error.message != NO_HOST_TO_KEY),
            "a station keying the radio itself was asked for a host"
        );
    }

    #[test]
    fn bandwidth_set_moves_the_station_between_sessions_and_says_why_not_otherwise() {
        // a host program's BW commands, through the control API (ADR-0026)
        let mut station = station();
        let moved = call(&mut station, "bandwidth.set", json!({"hz": 500}));
        assert!(moved.ok, "{:?}", moved.error);
        let result = moved.result.expect("result");
        assert_eq!(result["bandwidth_hz"], 500);
        assert_eq!(result["home_hz"], 2300);
        assert_eq!(result["why"], "host");
        let status = call(&mut station, "status", json!({}))
            .result
            .expect("status");
        assert_eq!(status["bandwidth"]["bandwidth_hz"], 500);
        assert_eq!(status["answering"], true);
        let caps = call(&mut station, "capabilities", json!({}))
            .result
            .expect("capabilities");
        assert_eq!(caps["bandwidth_hz"], 500);
        // 2750 is 2300 here; null is the station's own
        let wide = call(&mut station, "bandwidth.set", json!({"hz": 2750}));
        assert_eq!(wide.result.expect("result")["bandwidth_hz"], 2300);
        let own = call(&mut station, "bandwidth.set", json!({"hz": null}));
        assert_eq!(own.result.expect("result")["host_hz"], Value::Null);
        // a bandwidth nobody runs is a bad request; a call going out is a refusal to retry
        let odd = call(&mut station, "bandwidth.set", json!({"hz": 1800}));
        assert_eq!(odd.error.expect("error").code, "bad_params");
        call(&mut station, "connect", json!({"remote": "KK4XYZ"}));
        let busy = call(&mut station, "bandwidth.set", json!({"hz": 500}));
        let error = busy.error.expect("error");
        assert_eq!(error.code, "refused");
        assert!(error.retryable);
        assert!(
            error.message.contains("a call is going out"),
            "{}",
            error.message
        );
    }

    #[test]
    fn the_token_never_leaves_the_daemon() {
        let mut station = station();
        let mut daemon = daemon();
        let request = |method: &str| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params: json!({}),
        };
        for method in ["config.get", "diagnostics"] {
            let response = dispatch_with(&mut station, Some(&mut daemon), &request(method));
            assert!(response.ok, "{method}");
            let text = serde_json::to_string(&response.result).expect("json");
            assert!(
                !text.contains("hunter2"),
                "{method} leaked the token: {text}"
            );
            assert_eq!(
                response.result.expect("result")["config"]["control"]["token"],
                "<set>"
            );
        }
    }

    #[test]
    fn a_diagnostic_bundle_has_what_a_bug_report_needs() {
        let mut station = station();
        let mut daemon = daemon();
        daemon.audio = "loopback".to_owned();
        daemon.dropped_audio = 7;
        daemon.log.record(
            crate::log::Level::Warn,
            "watchdog",
            "key time exceeded",
            "Idle",
        );
        let response = dispatch_with(
            &mut station,
            Some(&mut daemon),
            &Request {
                id: Some("1".into()),
                method: "diagnostics".to_owned(),
                params: json!({}),
            },
        );
        assert!(response.ok);
        let bundle = response.result.expect("result");
        assert_eq!(bundle["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(bundle["platform"]["os"], std::env::consts::OS);
        assert_eq!(bundle["status"]["state"], "idle");
        // the status as `status` answers it, the daemon's part included: a report about a
        // host program or a KISS program starts from whether one was attached
        for key in [
            "host",
            "kiss",
            "supervised",
            "audio_fault",
            "config_note",
            "binary",
        ] {
            assert!(
                bundle["status"].get(key).is_some(),
                "the bundle's status has no {key}"
            );
        }
        assert_eq!(bundle["status"]["host"]["enabled"], false);
        assert_eq!(bundle["status"]["kiss"]["enabled"], false);
        assert_eq!(bundle["config"]["callsign"], "N0CALL");
        assert_eq!(bundle["audio"]["dropped_samples"], 7);
        assert_eq!(bundle["devices"]["serial_ports"][0]["name"], "COM3");
        assert_eq!(bundle["log"][0]["event"], "watchdog");
        assert_eq!(bundle["log"][0]["level"], "warn");
        assert!(
            bundle["generated"]
                .as_str()
                .expect("generated")
                .ends_with('Z')
        );
        assert!(bundle["capabilities"]["modes"].is_array());
        // and it is read-only: a client limited to reading may ask for it
        assert!(!is_mutating("diagnostics"));
    }

    #[test]
    fn a_recording_needs_a_directory_and_reports_what_it_held() {
        let mut station = station();
        let refused = call(&mut station, "record.start", json!({}));
        assert!(!refused.ok, "recorded with nowhere to put it");
        assert_eq!(refused.error.expect("error").code, "refused");

        let dir = std::env::temp_dir().join(format!("aether-recapi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        station.set_recording(Some(dir.clone()), false, "");
        let started = call(
            &mut station,
            "record.start",
            json!({"name": "api", "notes": "40 m"}),
        );
        assert!(started.ok, "{:?}", started.error);
        assert!(
            call(&mut station, "status", json!({}))
                .result
                .expect("status")["recording"]
                .is_object()
        );
        // a second start while one runs is refused, not silently a new file
        assert!(!call(&mut station, "record.start", json!({})).ok);
        station.capture(&vec![0.0f32; 48_000]).expect("capture");
        let stopped = call(&mut station, "record.stop", json!({}));
        assert!(stopped.ok);
        let summary = stopped.result.expect("summary");
        assert!((summary["seconds"].as_f64().expect("seconds") - 1.0).abs() < 1e-9);
        let sidecar: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("api.json")).expect("sidecar"))
                .expect("json");
        assert_eq!(sidecar["session"]["notes"], "40 m");
        assert_eq!(sidecar["session"]["callsign"], "W4ODA");
        assert!(sidecar["counters"]["frames_detected"].is_number());
        assert!(
            !call(&mut station, "record.stop", json!({})).ok,
            "stopped twice"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_keying_test_is_bounded_and_refused_during_a_session() {
        let mut station = station();
        let long = call(&mut station, "ptt.test", json!({"duration_s": 60}));
        assert!(!long.ok, "a minute of carrier is not a test");
        let ok = call(&mut station, "ptt.test", json!({"duration_s": 1.0}));
        assert!(ok.ok, "{:?}", ok.error);

        let mut busy = crate::station::tests_support::connected_station();
        let refused = call(&mut busy, "ptt.test", json!({"duration_s": 1.0}));
        assert!(!refused.ok, "it keyed a test into the middle of a session");
        assert_eq!(refused.error.expect("error").code, "refused");
    }

    #[test]
    fn a_tune_tone_is_bounded() {
        let mut station = station();
        assert!(!call(&mut station, "tune", json!({"duration_s": 30})).ok);
        assert!(!call(&mut station, "tune", json!({"duration_s": 0.1})).ok);
        assert!(call(&mut station, "tune", json!({"duration_s": 2.0})).ok);
    }

    #[test]
    fn the_level_meter_answers_before_it_has_heard_anything() {
        // "still listening" is an answer; a number that means nothing is not
        let mut station = station();
        let response = call(&mut station, "audio.level", json!({}));
        assert!(response.ok);
        let result = response.result.expect("result");
        assert_eq!(result["settled"], false);
        assert_eq!(result["advice"], "Still listening.");
    }

    #[test]
    fn an_unknown_method_is_refused_rather_than_ignored() {
        let mut station = station();
        let response = call(&mut station, "teleport", json!({}));
        assert!(!response.ok);
        let error = response.error.expect("error");
        assert_eq!(error.code, "unknown_method");
        assert!(!error.retryable, "retrying a typo will not help");
    }

    #[test]
    fn the_mutating_methods_are_the_ones_that_change_something() {
        for method in [
            "connect",
            "disconnect",
            "abort",
            "send",
            "listen",
            "beacon",
            "config.set",
        ] {
            assert!(is_mutating(method), "{method} should be mutating");
        }
        for method in ["status", "capabilities", "devices.list"] {
            assert!(!is_mutating(method), "{method} should be read only");
        }
    }
}
