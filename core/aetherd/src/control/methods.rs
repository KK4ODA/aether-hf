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
            | "beacon"
            | "tune"
            | "record.start"
            | "record.stop"
            | "record.notes"
            | "shutdown"
            | "config.set"
            | "ptt.test"
    )
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
    /// What the machine reports as audio devices and serial ports, for the bundle.
    ///
    /// A function rather than a call, because enumerating devices goes through the
    /// platform's audio API, and on a machine with no audio service at all — a CI runner —
    /// that has been seen to crash the process rather than return an error. The tests
    /// substitute a list; the daemon uses [`device_inventory`].
    pub devices: fn() -> Value,
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
            path,
            log,
            started: std::time::SystemTime::now(),
            audio: String::new(),
            dropped_audio: 0,
            devices: device_inventory,
        }
    }
}

/// The audio devices and serial ports this machine reports, or why it could not say.
#[must_use]
pub fn device_inventory() -> Value {
    match crate::audio::list_devices() {
        Ok(devices) => json!({
            "devices": devices.iter().map(device_json).collect::<Vec<_>>(),
            "serial_ports": crate::ptt::list_serial_ports(),
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
    match request.method.as_str() {
        "config.get" => return config_get(daemon, request.id.clone()),
        "config.set" => return config_set(station, daemon, &request.params, request.id.clone()),
        "diagnostics" => return diagnostics(station, daemon, request.id.clone()),
        _ => {}
    }
    dispatch_station(station, request)
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
fn diagnostics<P: Ptt>(
    station: &Station<P>,
    daemon: Option<&mut DaemonState>,
    id: Option<String>,
) -> Response {
    // without the daemon's state there is nobody to ask about devices, and a bundle from a
    // bare station is a test fixture rather than a bug report
    let devices = daemon
        .as_ref()
        .map_or(Value::Null, |daemon| (daemon.devices)());
    let mut bundle = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "generated": crate::log::rfc3339(unix_ms(std::time::SystemTime::now())),
        "status": status(station),
        "capabilities": capabilities(),
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
    let id = request.id.clone();
    let params = &request.params;
    match request.method.as_str() {
        "status" => Response::ok(id, status(station)),
        "capabilities" => Response::ok(id, capabilities()),
        "connect" => connect(station, params, id),
        "beacon" => match station.beacon() {
            Ok(()) => Response::ok(id, json!({ "accepted": true })),
            Err(reason) => Response::failed(
                id,
                ApiError::new(
                    "not_idle",
                    format!("Cannot beacon: {reason}. A beacon is sent outside a session."),
                    true,
                ),
            ),
        },
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
            match station.tune(seconds) {
                Ok(()) => Response::ok(id, json!({ "accepted": true, "duration_s": seconds })),
                Err(reason) => Response::failed(
                    id,
                    ApiError::new("refused", format!("Cannot tune: {reason}."), true),
                ),
            }
        }
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
        "listen" => match params.get("enabled").and_then(Value::as_bool) {
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
        },
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
    match station.connect(remote) {
        Ok(()) => Response::ok(id, json!({ "session": station.engine().session() })),
        Err(reason) => Response::failed(
            id,
            ApiError::new(
                "already_connected",
                format!("Cannot call {remote}: {reason}."),
                false,
            ),
        ),
    }
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
    station.send(&bytes);
    Response::ok(id, json!({ "accepted": count }))
}

fn devices(id: Option<String>) -> Response {
    match crate::audio::list_devices() {
        Ok(devices) => Response::ok(
            id,
            json!({
                "devices": devices.iter().map(device_json).collect::<Vec<_>>(),
                "serial_ports": crate::ptt::list_serial_ports(),
            }),
        ),
        Err(error) => Response::failed(
            id,
            ApiError::new("audio_unavailable", error.to_string(), true),
        ),
    }
}

/// Everything a client needs to render the station's current state.
fn status<P: Ptt>(station: &Station<P>) -> Value {
    let engine = station.engine();
    json!({
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
        "remote": engine.remote_call,
        "session": engine.session(),
        "mode": engine.current_mode(),
        "transmitting": station.transmitting(),
        "channel_busy": station.channel_busy(),
        "compressing": station.compressing(),
        "compression_saving": station.compression_saving(),
        "uptime_s": station.now(),
        "ptt": station.ptt_description(),
        "queued_bytes": engine.tx_pending_bytes(),
        "version": env!("CARGO_PKG_VERSION"),
        "metrics": metrics(station),
        "recording": station.recording().map(|(path, seconds)| json!({
            "path": path.display().to_string(),
            "seconds": seconds,
        })),
        "counters": counters(station),
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
        "deferred_for_busy": station.stats.deferred_for_busy,
        "watchdog_trips": station.stats.watchdog_trips,
        "beacons_heard": station.stats.beacons_heard,
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
    json!({
        "mode": station.engine().current_mode(),
        "queued_bytes": station.engine().tx_pending_bytes(),
        "noise_floor_db": level(busy.floor_db),
        "level_db": level(busy.level_db),
        "channel_busy": station.channel_busy(),
        "transmitting": station.transmitting(),
        "audio": level_json(&station.audio_level()),
    })
}

/// What this modem can do, so a client discovers the mode table instead of hard-coding it.
///
/// This is what keeps the control API free of any mention of a modulation: an FM physical
/// layer would answer here with its own table and nothing else would change.
#[must_use]
pub fn capabilities() -> Value {
    use aether_phy::modes::{LONG, MODES};

    let modes: Vec<Value> = MODES
        .iter()
        .map(|mode| {
            json!({
                "index": mode.index,
                "name": mode.name(),
                "payload_bytes": mode.payload_bytes(&LONG),
                "net_bit_rate": mode.net_bit_rate(&LONG),
                "threshold_db": aether_link::AWGN_THRESHOLD_DB[mode.index],
            })
        })
        .collect();
    json!({
        "api_version": "0.1",
        "phy": "aether-hf",
        "bandwidths_hz": [2300],
        "modes": modes,
        "usable_modes": aether_link::usable_modes(),
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
                token: None,
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
    fn capabilities_carries_the_mode_table_and_names_no_modulation_in_its_shape() {
        let caps = capabilities();
        let modes = caps["modes"].as_array().expect("modes");
        assert_eq!(modes.len(), aether_phy::modes::MODES.len());
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

    fn daemon() -> DaemonState {
        let mut config = crate::config::Config::parse(crate::config::EXAMPLE).expect("example");
        config.control.token = Some("hunter2".to_owned());
        let mut daemon = DaemonState::new(
            config,
            std::path::PathBuf::from("station.toml"),
            crate::log::Log::memory(50),
        );
        // not the machine's own: enumerating audio devices on a machine with no audio
        // service crashes inside the platform API, and a test must not depend on a sound card
        daemon.devices = || {
            json!({
                "devices": [{"name": "USB Audio CODEC", "input": true, "output": true}],
                "serial_ports": ["COM3"],
            })
        };
        daemon
    }

    #[test]
    fn the_token_never_leaves_the_daemon() {
        let mut station = station();
        let mut daemon = daemon();
        let request = |method: &str| Request {
            id: Some("1".into()),
            method: method.to_owned(),
            params: json!({}),
            token: None,
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
                token: None,
            },
        );
        assert!(response.ok);
        let bundle = response.result.expect("result");
        assert_eq!(bundle["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(bundle["platform"]["os"], std::env::consts::OS);
        assert_eq!(bundle["status"]["state"], "idle");
        assert_eq!(bundle["config"]["callsign"], "N0CALL");
        assert_eq!(bundle["audio"]["dropped_samples"], 7);
        assert_eq!(bundle["devices"]["serial_ports"][0], "COM3");
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
        station.set_recording(Some(dir.clone()), false);
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
