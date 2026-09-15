//! The Aether HF station daemon.
//!
//! Reads a configuration file, opens a sound card and a keying interface, and runs the modem,
//! with the control API and the VARA-compatible host interface listening beside it.
//!
//! # The loop
//!
//! One thread, driven by audio. Captured samples go into the station, which advances its
//! clock by exactly the audio it has heard, and playback is topped up to a small backlog so
//! the sound card never runs dry mid-burst. There is deliberately no wall clock anywhere: a
//! station that thinks a second has passed while its sound card delivered half a second will
//! answer bursts into the middle of them.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use aether_link::LinkConfig;
use aetherd::{
    audio::{AudioIo, Loopback, SoundCard, list_devices},
    config::{Config, EXAMPLE, PttConfig},
    control::{
        ControlServer, channel,
        methods::{DaemonState, HostStatus, dispatch_with, frame_json, is_mutating, metrics},
        protocol::Event,
    },
    host::HostServer,
    log::{Level, Log},
    ptt::{NullPtt, Ptt, PttError, RigctldPtt, SerialPtt, list_serial_ports},
    station::{Station, StationConfig},
};
use serde_json::json;

/// How much audio to keep queued for the sound card. Enough to ride out a scheduling hiccup,
/// short enough that keying and audio stay in step.
const PLAYBACK_BACKLOG_S: f64 = 0.25;
/// How long to wait when there is nothing to do. Short enough that a burst is never late by
/// an audible amount; long enough that an idle station does not spin a core.
const IDLE_SLEEP: Duration = Duration::from_millis(5);
/// How often to push link metrics to a listening client. Often enough to watch a transfer,
/// rare enough that a client that only wants state changes is not flooded.
const METRICS_INTERVAL: Duration = Duration::from_millis(500);
/// How long a changed list of stations heard waits before it is written. A burst
/// of frames is one write, not one per frame.
const HEARD_SAVE_DELAY: Duration = Duration::from_secs(5);

/// The exit status that asks a supervisor to start the daemon again.
///
/// A setting that needs a restart — the sound card, the keying port, the callsign's
/// section — should not need the operator to know that: the panel asks the daemon to stop
/// this way, and the desktop shell or systemd (`RestartForceExitStatus=`) starts it again on
/// the file it just wrote. 75 is `EX_TEMPFAIL` in `sysexits.h`, "try again later", which is
/// exactly the request.
const RESTART_EXIT_CODE: u8 = 75;

fn main() -> std::process::ExitCode {
    match run() {
        Ok(Exit::Done) => std::process::ExitCode::SUCCESS,
        Ok(Exit::Restart) => std::process::ExitCode::from(RESTART_EXIT_CODE),
        Err(message) => {
            eprintln!("aetherd: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// How a run ended: quietly, or asking to be started again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exit {
    Done,
    Restart,
}

struct Args {
    config: Option<PathBuf>,
    call: Option<String>,
    dry_run: bool,
    replay: Option<PathBuf>,
    expect: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        config: None,
        call: None,
        dry_run: false,
        replay: None,
        expect: None,
    };
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "--version" => {
                println!("aetherd {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--example-config" => {
                print!("{EXAMPLE}");
                return Ok(None);
            }
            "--list-devices" => {
                let devices =
                    list_devices().map_err(|e| format!("cannot list audio devices: {e}"))?;
                if devices.is_empty() {
                    println!("no audio devices");
                }
                for device in devices {
                    let kind = match (device.input, device.output) {
                        (true, true) => "in/out",
                        (true, false) => "in",
                        (false, true) => "out",
                        (false, false) => "-",
                    };
                    println!("{:<8} {}", kind, device.name);
                }
                return Ok(None);
            }
            "--list-ports" => {
                let ports = list_serial_ports();
                if ports.is_empty() {
                    println!("no serial ports");
                }
                for port in ports {
                    println!("{:<12} {}", port.name, port.description);
                }
                return Ok(None);
            }
            "--dry-run" => args.dry_run = true,
            "-c" | "--config" => {
                args.config = Some(PathBuf::from(argv.next().ok_or("--config needs a path")?));
            }
            "--call" => {
                args.call = Some(argv.next().ok_or("--call needs a callsign")?);
            }
            "--replay" => {
                args.replay = Some(PathBuf::from(
                    argv.next().ok_or("--replay needs a WAV file")?,
                ));
            }
            "--expect" => {
                args.expect = Some(PathBuf::from(
                    argv.next().ok_or("--expect needs a session sidecar")?,
                ));
            }
            other => return Err(format!("unknown argument {other:?}; try --help")),
        }
    }
    Ok(Some(args))
}

const USAGE: &str = "\
aetherd — the Aether HF station daemon

    aetherd --config station.toml [--call W4ODA] [--dry-run]
    aetherd --replay session.wav [--expect session.json]

  -c, --config PATH   the station configuration (required to run)
      --call CALL     call this station once the modem is up
      --dry-run       run the modem against an audio loopback, keying nothing
      --replay WAV    run a recording through the receiver and list what it finds
      --expect JSON   the recording's sidecar: mute where the transmitter was keyed,
                      and fail if fewer frames decode than did on the day
      --list-devices  print the audio devices this machine offers
      --list-ports    print the serial ports this machine offers
      --example-config  print a commented configuration to start from
      --version       print the version
  -h, --help          print this
";

fn run() -> Result<Exit, String> {
    let Some(args) = parse_args()? else {
        return Ok(Exit::Done);
    };
    if let Some(wav) = &args.replay {
        return replay(wav, args.expect.as_deref()).map(|()| Exit::Done);
    }
    let path = args
        .config
        .ok_or("no configuration; try --example-config, then --config <path>")?;
    let config = Config::load(&path).map_err(|e| e.to_string())?;
    // Canonicalise so `config.set` writes where the operator thinks it does even when the
    // daemon was started with a relative path — but strip Windows' extended-length prefix,
    // which is correct and unreadable and would be shown to a human.
    let path = path.canonicalize().map_or(path.clone(), |full| {
        let text = full.display().to_string();
        // the prefix is a literal backslash-backslash-question-backslash
        text.strip_prefix("\\\\?\\")
            .map_or_else(|| full.clone(), std::path::PathBuf::from)
    });

    // The log comes up before anything that can fail loudly, so that what fails is on record.
    let log = open_log(&config, &path)?;
    let mut daemon = DaemonState::new(config.clone(), path, log);

    // A dry run keys nothing, whatever the file says. Somebody checking their configuration
    // must not put a carrier on the air to find out that they had the wrong serial port.
    // Neither does a simulated channel: there is no radio on the other end of a socket.
    let sim = config.sim_config();
    let ptt: Box<dyn Ptt> = if args.dry_run || sim.is_some() {
        Box::new(NullPtt::default())
    } else {
        open_ptt(&config.ptt).map_err(|e| e.to_string())?
    };
    let mut station = Station::new(
        station_config(&config, &daemon.path),
        ptt,
        seed_from_callsign(&config.callsign),
    );
    daemon.log.record(
        Level::Info,
        "ptt",
        &format!(
            "{} keying via {}",
            config.callsign,
            station.ptt_description()
        ),
        "Idle",
    );

    let mut audio: Box<dyn AudioIo> = if args.dry_run {
        "dry run: audio loops back and nothing is keyed".clone_into(&mut daemon.audio);
        Box::new(Loopback::new())
    } else if let Some(sim) = &sim {
        let link = aetherd::sim::SimLink::open(sim)
            .map_err(|e| format!("cannot open the simulated channel: {e}"))?;
        daemon.audio.clone_from(&link.description);
        Box::new(link)
    } else {
        let card = SoundCard::open(&config.audio_config()).map_err(|e| e.to_string())?;
        daemon.audio.clone_from(&card.description);
        Box::new(card)
    };
    daemon
        .log
        .record(Level::Info, "audio", &daemon.audio, "Idle");

    // A gateway is stopped by its service manager sending a signal. Whatever else happens on
    // the way out, the transmitter has to be released: a station killed mid-burst would
    // otherwise sit there keyed until somebody noticed, which on an unattended station could
    // be a very long time.
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let restarting = std::sync::atomic::AtomicBool::new(false);
    let flag = std::sync::Arc::clone(&stopping);
    if let Err(error) = ctrlc::set_handler(move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }) {
        daemon.log.record(
            Level::Error,
            "daemon",
            &format!("cannot catch a stop signal ({error}); the radio may stay keyed on exit"),
            "Idle",
        );
    }

    let (handle, control) = channel();
    let _servers = start_servers(&config, handle, &mut daemon)?;

    if let Some(call) = &args.call {
        station.connect(call).map_err(str::to_owned)?;
        daemon.log.record(
            Level::Info,
            "connect",
            &format!("calling {call}"),
            "Connecting",
        );
    }

    serve(
        &config,
        &mut station,
        audio.as_mut(),
        &control,
        &mut daemon,
        &stopping,
        &restarting,
    )?;
    Ok(if restarting.load(std::sync::atomic::Ordering::SeqCst) {
        Exit::Restart
    } else {
        Exit::Done
    })
}

/// `--replay`: the receiver over a recording, held to its sidecar if one is given.
fn replay(wav: &Path, expect: Option<&Path>) -> Result<(), String> {
    use aetherd::replay::{Expectation, compare, describe};
    // the sidecar beside the WAV is the expectation unless told otherwise
    let beside = wav.with_extension("json");
    let expectation = match expect {
        Some(path) => Some(Expectation::from_sidecar(path)?),
        None if beside.is_file() => Some(Expectation::from_sidecar(&beside)?),
        None => None,
    };
    let muted = expectation.as_ref().map_or(&[][..], |e| e.muted.as_slice());
    let bandwidth_hz = expectation.as_ref().map_or(2300, |e| e.bandwidth_hz);
    let found = aetherd::replay::replay(wav, muted, bandwidth_hz)?;
    for frame in &found {
        println!("{}", describe(frame));
    }
    let Some(expectation) = expectation else {
        println!(
            "{} frames found, {} decoded (no sidecar to compare with)",
            found.len(),
            found.iter().filter(|f| f.decoded).count()
        );
        return Ok(());
    };
    let verdict = compare(&expectation.frames, &found);
    println!(
        "recorded {} decoded frames; replay found {} and decoded {}",
        verdict.recorded, verdict.found, verdict.replayed
    );
    if verdict.holds() {
        Ok(())
    } else {
        Err(format!(
            "the replay decoded {} frames where the recording decoded {}: the receiver \
             has lost something it once had",
            verdict.replayed, verdict.recorded
        ))
    }
}

/// The log, on standard output and — if asked — in a file beside the configuration.
fn open_log(config: &Config, path: &std::path::Path) -> Result<Log, String> {
    let mut log = Log::to_stdout(config.log.format, config.log.keep);
    if let Some(file) = &config.log.file {
        // relative to the configuration file, which is the one place the operator knows
        let file = if file.is_relative() {
            path.parent().map_or(file.clone(), |dir| dir.join(file))
        } else {
            file.clone()
        };
        log.also_to_file(&file)
            .map_err(|e| format!("cannot open the log file {}: {e}", file.display()))?;
    }
    log.record(
        Level::Info,
        "daemon",
        &format!(
            "aetherd {} starting from {}",
            env!("CARGO_PKG_VERSION"),
            path.display()
        ),
        "Idle",
    );
    Ok(log)
}

/// What the station is told from the configuration file.
fn station_config(config: &Config, config_path: &std::path::Path) -> StationConfig {
    // recordings live beside the configuration unless told otherwise, and a relative
    // directory is relative to it — the one place the operator already knows
    let beside = config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let record_dir = match &config.record.dir {
        Some(dir) if dir.is_absolute() => dir.clone(),
        Some(dir) => beside.join(dir),
        None => beside.join("recordings"),
    };
    StationConfig {
        // validated: the bandwidth names a waveform this version has
        params: config
            .radio
            .params()
            .unwrap_or(aether_phy::waveform::WIDE_2300),
        answer_only: config.radio.answer_only,
        record_dir: Some(record_dir),
        record_auto: config.record.auto,
        record_notes: config.record.notes.clone(),
        playback_lead_s: PLAYBACK_BACKLOG_S,
        callsign: config.callsign.clone(),
        link: LinkConfig {
            max_mode: config.radio.fastest_mode(),
            ..LinkConfig::default()
        },
        busy: config.busy_config(),
        tx_level: config.audio.tx_level,
        max_key_s: config.radio.max_key_s,
        wait_for_clear: config.radio.wait_for_clear,
        compress: config.radio.compress,
        cw_id: config.radio.cw_id.then(|| aetherd::cwid::CwId {
            wpm: config.radio.cw_id_wpm,
            ..aetherd::cwid::CwId::default()
        }),
        cw_id_interval_s: config.radio.cw_id_interval_s,
        ..StationConfig::default()
    }
}

/// The control API and, if asked for, the VARA-compatible host interface. Held for as long
/// as the daemon runs: dropping them closes the ports.
fn start_servers(
    config: &Config,
    handle: aetherd::control::ControlHandle,
    daemon: &mut DaemonState,
) -> Result<(Option<ControlServer>, Option<HostServer>), String> {
    let control = if config.control.enabled {
        let server = ControlServer::start(&config.control_config(), handle.clone())
            .map_err(|e| e.to_string())?;
        daemon.log.record(
            Level::Info,
            "control",
            &format!("listening on ws://{}/v1", server.address),
            "Idle",
        );
        Some(server)
    } else {
        daemon
            .log
            .record(Level::Info, "control", "disabled", "Idle");
        None
    };

    let host = if config.host.enabled {
        let server = HostServer::start(&config.host_config(), handle).map_err(|e| e.to_string())?;
        daemon.host = Some(HostStatus {
            command_address: server.command_address.to_string(),
            data_address: server.data_address.to_string(),
            connected: std::sync::Arc::clone(&server.connected),
        });
        daemon.log.record(
            Level::Info,
            "host",
            &format!(
                "listening on {} (data {}), reporting itself as {}",
                server.command_address,
                server.data_address,
                aetherd::host::vara::version_string()
            ),
            "Idle",
        );
        Some(server)
    } else {
        None
    };
    Ok((control, host))
}

/// What the log says on the way out: a stop, or a stop that asked for a start.
fn stop_reason(restarting: &std::sync::atomic::AtomicBool) -> &'static str {
    if restarting.load(std::sync::atomic::Ordering::SeqCst) {
        "stopping to be started again"
    } else {
        "stopping"
    }
}

/// Answer what clients have asked since the last audio block.
///
/// Control requests are answered from the modem's thread, between blocks. The modem is
/// single-threaded because its clock is the audio it has heard, and a connection reaching
/// in from another thread would be able to change the state mid-frame.
fn answer_commands(
    station: &mut Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
    stopping: &std::sync::atomic::AtomicBool,
    restarting: &std::sync::atomic::AtomicBool,
) {
    for command in control.drain() {
        // `shutdown` is the daemon's to answer, not the station's: it runs the same path a
        // stop signal does, so a supervisor that cannot send a signal — the desktop shell
        // on Windows — still gets the transmitter released properly. `restart: true` is the
        // same stop with a different exit status, for a supervisor to act on.
        let response = if command.request.method == "shutdown" {
            let restart = command.request.params["restart"].as_bool().unwrap_or(false);
            stopping.store(true, std::sync::atomic::Ordering::SeqCst);
            if restart {
                restarting.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            aetherd::control::protocol::Response::ok(
                command.request.id.clone(),
                json!({ "stopping": true, "restart": restart, "supervised": daemon.supervised }),
            )
        } else {
            dispatch_with(station, Some(daemon), &command.request)
        };
        // What a client asked for, and whether it got it, is most of what a bug report
        // needs. Reads are not logged: a panel polls, and the ring would hold nothing else.
        if is_mutating(&command.request.method) {
            daemon.log.record(
                if response.ok {
                    Level::Info
                } else {
                    Level::Warn
                },
                "control",
                &describe(&command.request, &response),
                &state_name(station),
            );
        }
        let _ = command.reply.send(response);
    }
}

/// A request and its answer, in one line, with the payload of a `send` left out.
fn describe(
    request: &aetherd::control::Request,
    response: &aetherd::control::protocol::Response,
) -> String {
    let params = if request.method == "send" {
        // the bytes are the operator's traffic, and they do not belong in a bug report
        request
            .params
            .get("data")
            .and_then(|d| d.as_str())
            .map_or_else(String::new, |data| {
                format!("{{\"data\": <{} base64 chars>}}", data.len())
            })
    } else {
        request.params.to_string()
    };
    match &response.error {
        None => format!("{} {params}: ok", request.method),
        Some(error) => format!(
            "{} {params}: {} ({})",
            request.method, error.message, error.code
        ),
    }
}

/// The modem's state, as the log records it.
fn state_name(station: &Station<Box<dyn Ptt>>) -> String {
    format!("{:?}", station.state())
}

/// How much a station event matters, from its name.
fn level_of(name: &str) -> Level {
    match name {
        "error" => Level::Error,
        "watchdog" | "timeout" | "failed" => Level::Warn,
        _ => Level::Info,
    }
}

/// The run loop: audio in, audio out, control requests answered between blocks.
///
/// Never returns; the daemon is stopped from outside.
fn serve(
    config: &Config,
    station: &mut Station<Box<dyn Ptt>>,
    audio: &mut dyn AudioIo,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
    stopping: &std::sync::atomic::AtomicBool,
    restarting: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let block = (0.02 * f64::from(config.audio.sample_rate)) as usize;
    let backlog = (PLAYBACK_BACKLOG_S * f64::from(config.audio.sample_rate)) as usize;
    let mut reported_drops = 0;
    let mut last_metrics = std::time::Instant::now();
    let mut last_keyed = false;
    let mut heard_changed_at: Option<std::time::Instant> = None;

    loop {
        if stopping.load(std::sync::atomic::Ordering::SeqCst) {
            stop(station, daemon, restarting);
            return Ok(());
        }

        answer_commands(station, control, daemon, stopping, restarting);

        let captured = audio.capture();
        let idle = captured.is_empty();
        if !idle {
            station.capture(&captured).map_err(|e| e.to_string())?;
        }

        // top the sound card up, so it never runs dry in the middle of a burst
        let mut buffer = vec![0.0f32; block];
        while audio.queued() < backlog {
            let count = station.playback(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            audio.playback(&buffer[..count]);
        }

        for event in station.take_events() {
            let (name, detail) = event.split_once(':').unwrap_or(("log", event.as_str()));
            daemon
                .log
                .record(level_of(name), name, detail, &state_name(station));
            control.publish(&Event::new(
                if name == "connected" || name == "disconnected" || name == "role" {
                    "state"
                } else {
                    "log"
                },
                json!({
                    "name": name,
                    "detail": detail,
                    "state": format!("{:?}", station.state()),
                    // the callsign a session runs under is whichever of the station's the
                    // caller asked for, so a host cannot know it without being told
                    "callsign": station.engine().my_call,
                    "remote": station.engine().remote_call,
                }),
            ));
        }
        report_frames(station, control, daemon, &mut heard_changed_at);
        let received = station.take_received();
        if !received.is_empty() {
            control.publish(&Event::new(
                "data",
                json!({"data": aetherd::control::methods::to_base64(&received)}),
            ));
            // with no client attached there is still somewhere for it to go
            print!("{}", String::from_utf8_lossy(&received));
        }

        // metrics are the operator's window into the link, so they go out while it runs
        if control.subscriber_count() > 0 && last_metrics.elapsed() >= METRICS_INTERVAL {
            last_metrics = std::time::Instant::now();
            control.publish(&Event::new("metrics", metrics(station)));
        }
        // `ptt` reports the transmitter, not the session: a host uses it to know when the
        // radio is keyed, and a state change is a different thing entirely
        let keyed = station.transmitting();
        if keyed != last_keyed {
            last_keyed = keyed;
            daemon.log.record(
                Level::Info,
                "ptt",
                if keyed { "keyed" } else { "released" },
                &state_name(station),
            );
            control.publish(&Event::new("ptt", json!({ "on": keyed })));
        }

        let dropped = audio.dropped();
        if dropped > reported_drops {
            daemon.log.record(
                Level::Warn,
                "audio",
                &format!(
                    "dropped {} captured samples: the modem is behind",
                    dropped - reported_drops
                ),
                &state_name(station),
            );
            reported_drops = dropped;
            daemon.dropped_audio = dropped as u64;
        }

        if idle {
            std::thread::sleep(IDLE_SLEEP);
        }
    }
}

/// The way out: the log says why, the stations heard are written, the radio is released.
fn stop(
    station: &mut Station<Box<dyn Ptt>>,
    daemon: &mut DaemonState,
    restarting: &std::sync::atomic::AtomicBool,
) {
    daemon.log.record(
        Level::Info,
        "daemon",
        stop_reason(restarting),
        &state_name(station),
    );
    if let Err(error) = daemon.heard.save() {
        daemon.log.record(
            Level::Warn,
            "heard",
            &format!("the stations heard were not saved: {error}"),
            &state_name(station),
        );
    }
    // Report the failure but do not return on it: there is nothing left to try, and
    // exiting quietly would hide a radio that is still keyed.
    if let Err(error) = station.shut_down() {
        daemon.log.record(
            Level::Error,
            "ptt",
            &format!("the radio would not release: {error}"),
            &state_name(station),
        );
        eprintln!("aetherd: the radio would not release: {error}");
    }
}

/// Every frame the physical layer found goes out as it is, decoded or not: a display
/// plots them, and the ones with a callsign in them join the stations heard — which is
/// written a few seconds after it last changed, so a burst of frames is one write.
fn report_frames(
    station: &mut Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
    heard_changed_at: &mut Option<std::time::Instant>,
) {
    let frames = station.take_frame_reports();
    if !frames.is_empty() {
        let frequency_hz = station.frequency_hz();
        let at_ms = unix_ms_now();
        for frame in &frames {
            control.publish(&Event::new("frame", frame_json(frame)));
            if let Some(sighting) = sighting_of(frame, at_ms, frequency_hz) {
                let entry = daemon.heard.note(sighting);
                heard_changed_at.get_or_insert_with(std::time::Instant::now);
                control.publish(&Event::new(
                    "heard",
                    serde_json::to_value(&entry).unwrap_or(serde_json::Value::Null),
                ));
            }
        }
    }
    if heard_changed_at.is_some_and(|at| at.elapsed() >= HEARD_SAVE_DELAY) {
        *heard_changed_at = None;
        if let Err(error) = daemon.heard.save() {
            daemon.log.record(
                Level::Warn,
                "heard",
                &format!("the stations heard were not saved: {error}"),
                &state_name(station),
            );
        }
    }
}

/// Milliseconds since the Unix epoch, now.
fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// What a frame says about who sent it, for the stations-heard list.
///
/// A frame with a callsign in it — a beacon, a connect request, an answer, a probe — names its
/// sender outright. A data or control frame names nobody, but during a session one
/// with the session's id is the other station's, which the station attributes on the
/// way here. Anything else is heard and not counted: a frame from nobody is not a
/// station.
fn sighting_of(
    frame: &aetherd::station::FrameReport,
    at_ms: u64,
    frequency_hz: Option<u64>,
) -> Option<aetherd::heard::Sighting> {
    use aetherd::heard::{Activity, Sighting};
    let callsign = frame.from.clone()?;
    let (activity, detail) = match frame.kind {
        "beacon" => (Activity::Beacon, None),
        "connect" => (Activity::Calling, frame.to.clone()),
        "answer" | "probe-answer" => (Activity::Answering, frame.to.clone()),
        "probe" => (Activity::Probing, frame.to.clone()),
        _ => (Activity::Connected, None),
    };
    Some(Sighting {
        callsign,
        at_ms,
        snr_db: frame.snr_db,
        mode: (frame.kind != "control").then_some(frame.mode),
        frequency_hz,
        activity,
        detail,
    })
}

fn open_ptt(config: &PttConfig) -> Result<Box<dyn Ptt>, PttError> {
    Ok(match config {
        PttConfig::None => Box::new(NullPtt::default()),
        PttConfig::Serial { port, line } => Box::new(SerialPtt::open(port, (*line).into())?),
        PttConfig::Rigctld { address } => {
            Box::new(RigctldPtt::new(address, Duration::from_millis(500)))
        }
        PttConfig::Cat {
            port,
            protocol,
            baud,
            civ_address,
            source,
        } => {
            let protocol = match protocol {
                aetherd::config::CatProtocol::Yaesu => aetherd::ptt::CatProtocol::Yaesu {
                    data: *source == aetherd::config::CatSource::Data,
                },
                aetherd::config::CatProtocol::Kenwood => aetherd::ptt::CatProtocol::Kenwood,
                aetherd::config::CatProtocol::Icom => aetherd::ptt::CatProtocol::Icom {
                    // validated to be present when the protocol is Icom
                    address: civ_address.unwrap_or(0x00),
                },
            };
            Box::new(aetherd::ptt::CatPtt::open(port, *baud, protocol)?)
        }
    })
}

/// A seed for the backoff generator, derived from the callsign.
///
/// Two stations calling each other at the same instant have to desynchronise, and a fixed
/// seed would have them back off by exactly the same amount every time. The callsign is
/// something the two are guaranteed to differ in.
fn seed_from_callsign(call: &str) -> u64 {
    call.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_callsigns_get_different_backoff_seeds() {
        // two stations that back off identically collide on every retry
        let a = seed_from_callsign("W4ODA");
        let b = seed_from_callsign("KK4XYZ");
        assert_ne!(a, b);
        assert_ne!(seed_from_callsign("W4ODA"), seed_from_callsign("W4ODB"));
        assert_eq!(
            a,
            seed_from_callsign("W4ODA"),
            "and the same call is stable"
        );
    }

    #[test]
    fn the_usage_text_mentions_every_flag_the_parser_takes() {
        for flag in [
            "--config",
            "--call",
            "--dry-run",
            "--list-devices",
            "--list-ports",
            "--example-config",
            "--version",
            "--help",
        ] {
            assert!(USAGE.contains(flag), "usage does not mention {flag}");
        }
    }
}
