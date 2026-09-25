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
    audio::{AudioIo, DEVICE_LATENCY_S, Loopback, SoundCard, list_devices},
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

/// A pass of the run loop slower than this is logged with what it was doing. It is the
/// quarter second the card used to be kept ahead by: a pass this slow would have starved
/// it, and still says the modem is not keeping up with its own audio.
const LOOP_STALL_MS: f64 = 250.0;
/// How often at most a slow pass is logged, so a slow machine does not fill its own log.
const LOOP_STALL_LOG_INTERVAL: Duration = Duration::from_secs(10);
/// How long to wait when there is nothing to do. Short enough that a burst is never late by
/// an audible amount; long enough that an idle station does not spin a core.
const IDLE_SLEEP: Duration = Duration::from_millis(5);
/// How often to push link metrics to a listening client. Often enough to watch a transfer,
/// rare enough that a client that only wants state changes is not flooded.
const METRICS_INTERVAL: Duration = Duration::from_millis(500);
/// How long a changed list of stations heard waits before it is written. A burst
/// of frames is one write, not one per frame.
const HEARD_SAVE_DELAY: Duration = Duration::from_secs(5);
/// If the sound card delivers no audio for this long while the radio is keyed, the card has
/// stopped under a keyed transmitter — a dropped USB device, a sample-rate change. The key
/// is released against the card's own clock, which is now frozen, so nothing else would ever
/// bring it up: the loop forces it up on the wall clock instead. Comfortably longer than the
/// slowest loop pass, and well inside the key-time watchdog's own limit.
const AUDIO_STALL_RELEASE: Duration = Duration::from_secs(2);
/// The most a stopping daemon waits for the replies still on their way to its clients —
/// above all the one to the `shutdown` that stopped it. Writing one takes a millisecond; this
/// is for a machine too loaded to schedule the thread that writes it, and it bounds a client
/// that has stopped reading.
const REPLY_GRACE: Duration = Duration::from_secs(2);

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
    /// The block a replay feeds the receiver in, in milliseconds; the daemon's own 20 by
    /// default. The receiver's cost is per call rather than per sample, so the same
    /// recording at 100 ms tells whether the loop's passes are search-bound.
    block_ms: Option<f64>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        config: None,
        call: None,
        dry_run: false,
        replay: None,
        expect: None,
        block_ms: None,
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
                let interfaces = aetherd::ptt::list_gpio_interfaces();
                if !interfaces.is_empty() {
                    println!();
                    println!(
                        "CM108-class interfaces, keyed through a GPIO pin ([ptt] kind = \"cm108\"):"
                    );
                    for interface in interfaces {
                        println!("{}\n    device = {:?}", interface.name, interface.path);
                    }
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
            "--block-ms" => {
                let value = argv
                    .next()
                    .ok_or("--block-ms needs a number of milliseconds")?;
                let ms: f64 = value
                    .parse()
                    .map_err(|_| format!("--block-ms: {value:?} is not a number"))?;
                if !(1.0..=1000.0).contains(&ms) {
                    return Err("--block-ms is between 1 and 1000".to_owned());
                }
                args.block_ms = Some(ms);
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
      --block-ms N    feed the replay in blocks of N ms (default 20, the daemon's own);
                      the receiver's cost per block is printed either way
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
        let block_s = args
            .block_ms
            .map_or(aetherd::replay::BLOCK_S, |ms| ms / 1000.0);
        return replay(wav, args.expect.as_deref(), block_s).map(|()| Exit::Done);
    }
    let path = args
        .config
        .ok_or("no configuration; try --example-config, then --config <path>")?;
    let (config, config_note) = Config::load_noting(&path).map_err(|e| e.to_string())?;
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
    if let Some(note) = config_note {
        // a version gone back to, finding the file a newer one wrote: on record, and on the
        // panel for the life of this run
        daemon.log.record(Level::Warn, "config", &note, "Idle");
        daemon.config_note = Some(note);
    }
    adopt_profile(&mut daemon);

    // A dry run keys nothing, whatever the file says. Somebody checking their configuration
    // must not put a carrier on the air to find out that they had the wrong serial port.
    // Neither does a simulated channel: there is no radio on the other end of a socket.
    let sim = config.sim_config();
    let ptt = keying(&config, args.dry_run || sim.is_some(), &mut daemon);
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

    let mut audio = sound(&config, args.dry_run, sim.as_ref(), &mut daemon)?;
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
    daemon.control_handle = Some(handle.clone());
    let _servers = start_servers(&config, handle, &mut daemon)?;
    sync_kiss(&mut station, &mut daemon);

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
fn replay(wav: &Path, expect: Option<&Path>, block_s: f64) -> Result<(), String> {
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
    let (found, timing) = aetherd::replay::replay_timed(wav, muted, bandwidth_hz, block_s)?;
    for frame in &found {
        println!("{}", describe(frame));
    }
    // The daemon's loop is single-threaded, so what the receiver costs per block is what
    // every pass of the loop costs, and used to be what put holes in transmissions when it
    // exceeded the quarter second kept queued at the sound card (ADR-0010). The replay is
    // where that cost can be measured without a radio.
    let block_ms = block_s * 1000.0;
    let audio_ms = timing.blocks as f64 * block_ms;
    println!(
        "receiver: {} blocks of {block_ms:.0} ms in {:.1} s ({:.2}x real time); slowest block \
         {:.0} ms at {:.2} s; {} over 100 ms, {} over the daemon's 250 ms playback backlog",
        timing.blocks,
        timing.total_ms / 1000.0,
        timing.total_ms / audio_ms.max(1.0),
        timing.max_ms,
        timing.max_at_s,
        timing.over_100_ms,
        timing.over_250_ms
    );
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

/// The first start with profiles: the running configuration becomes the *Default* profile,
/// so an upgrade changes nothing the operator can see and the Setup tab has a name to show.
/// Done once; a store that has been touched is left alone.
fn adopt_profile(daemon: &mut DaemonState) {
    let inventory = aetherd::profile::Inventory::from_json(&(daemon.devices)());
    let adopted = daemon.profiles.adopt(
        &daemon.config,
        daemon.memories.entries(),
        Some(&inventory),
        &aetherd::profile::now(),
    );
    match adopted {
        Ok(Some(entry)) => daemon.log.record(
            Level::Info,
            "profile",
            &format!(
                "the running settings were saved as the profile {:?} ({})",
                entry.name, entry.path
            ),
            "Idle",
        ),
        Ok(None) => {}
        Err(error) => daemon.log.record(
            Level::Warn,
            "profile",
            &format!("the running settings could not be saved as a profile: {error}"),
            "Idle",
        ),
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
        record_tx_audio: config.record.tx_audio,
        playback_lead_s: DEVICE_LATENCY_S,
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
        operator: config.operator.clone(),
        regulatory: regulatory_settings(config),
        ..StationConfig::default()
    }
}

/// The regulatory settings the station runs with (ADR-0018): the file's — except that a
/// daemon on a simulated channel, which keys no transmitter, is not judged unless its file
/// chooses a profile, so a bench pair needs no dial and no license class.
fn regulatory_settings(config: &Config) -> aetherd::regulatory::Settings {
    let simulated = config.sim.listen.is_some() || config.sim.connect.is_some();
    if simulated && config.regulatory.profile.is_empty() {
        aetherd::regulatory::Settings::unchecked()
    } else {
        config.regulatory.settings()
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
        let server = HostServer::start(&config.host_config(), handle, daemon.host_flags.clone())
            .map_err(|e| e.to_string())?;
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

/// Keep the KISS port as the configuration says (ADR-0019): started, stopped, or started
/// again in place when its settings change — without restarting the daemon, so a host
/// program or a panel attached meanwhile is not interrupted. A port that would not bind is
/// tried again every `KISS_RETRY`, its error logged once. The port's own notes go to the log
/// here.
fn sync_kiss(station: &mut Station<Box<dyn Ptt>>, daemon: &mut DaemonState) {
    let state = format!("{:?}", station.state());
    if let Some(server) = &daemon.kiss {
        for note in server.take_notes() {
            let level = if note.warn { Level::Warn } else { Level::Info };
            daemon.log.record(level, "kiss", &note.text, &state);
        }
    }
    let wanted = daemon.config.kiss_config();
    let desired = wanted.enabled.then_some(wanted);
    let changed = desired != daemon.kiss_tried;
    let retry = !changed
        && daemon.kiss.is_none()
        && desired.is_some()
        && daemon
            .kiss_retry_at
            .is_some_and(|at| std::time::Instant::now() >= at);
    if !changed && !retry {
        return;
    }
    daemon.kiss_tried.clone_from(&desired);
    if daemon.kiss.take().is_some() {
        daemon
            .log
            .record(Level::Info, "kiss", "the KISS port is closed", &state);
    }
    // the same failure again is not news: said once, and shown on the panel meanwhile
    let previous = daemon.kiss_error.take();
    daemon.kiss_retry_at = None;
    let Some(config) = desired else {
        station.clear_datagrams("the KISS port was switched off");
        return;
    };
    let Some(handle) = daemon.control_handle.clone() else {
        return;
    };
    match aetherd::kiss::KissServer::start(&config, handle, daemon.host_flags.clone()) {
        Ok(server) => {
            for note in server.take_notes() {
                let level = if note.warn { Level::Warn } else { Level::Info };
                daemon.log.record(level, "kiss", &note.text, &state);
            }
            daemon.kiss = Some(server);
        }
        Err(error) => {
            if previous.as_deref() != Some(error.as_str()) {
                daemon.log.record(Level::Error, "kiss", &error, &state);
            }
            daemon.kiss_error = Some(error);
            daemon.kiss_retry_at = Some(std::time::Instant::now() + KISS_RETRY);
        }
    }
}

/// How often a KISS port that would not open is tried again.
const KISS_RETRY: std::time::Duration = std::time::Duration::from_secs(10);

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
    // a change to the settings, the dials or the profiles moves the station on or off
    // its profile: every panel is told, so the mark by the profile's name is never stale
    if daemon.take_profiles_changed() {
        control.publish(&Event::new(
            "profile",
            aetherd::control::profiles::status_json(daemon),
        ));
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

/// What the sound card has lost so far, as last reported, so each loss is logged once.
#[derive(Debug, Default, Clone, Copy)]
struct AudioLosses {
    dropped: usize,
    starved: usize,
}

/// Report what the sound card lost since the last pass: captured audio the modem could not
/// keep up with, and — the one that matters on the air — silence the card had to play
/// inside a transmission because the next samples had not reached it. Both go to the log
/// and to the diagnostic bundle, never nowhere.
fn note_audio_losses(
    daemon: &mut DaemonState,
    audio: &dyn AudioIo,
    sample_rate: u32,
    station: &Station<Box<dyn Ptt>>,
    reported: &mut AudioLosses,
) {
    let dropped = audio.dropped();
    if dropped > reported.dropped {
        daemon.log.record(
            Level::Warn,
            "audio",
            &format!(
                "dropped {} captured samples: the modem is behind",
                dropped - reported.dropped
            ),
            &state_name(station),
        );
        reported.dropped = dropped;
        daemon.dropped_audio = dropped as u64;
    }
    let starved = audio.starved();
    if starved > reported.starved {
        let ms = (starved - reported.starved) as f64 * 1000.0 / f64::from(sample_rate);
        daemon.log.record(
            Level::Warn,
            "audio",
            &format!(
                "the sound card ran dry for {ms:.0} ms inside a transmission: a hole on the \
                 air (the modem loop stalled; see `loop` in diagnostics)"
            ),
            &state_name(station),
        );
        reported.starved = starved;
        daemon.starved_audio = starved as u64;
    }
}

/// Account for one pass of the run loop: keep the slowest on record for the diagnostic
/// bundle, and log a pass slower than [`LOOP_STALL_MS`] with where the time went — the
/// receiver's decode, a command, or the playback render — rate-limited so a slow machine
/// says so once in a while rather than in every line.
fn note_pass(
    daemon: &mut DaemonState,
    last_logged: &mut Option<std::time::Instant>,
    station: &Station<Box<dyn Ptt>>,
    [commands_ms, capture_ms, playback_ms]: [f64; 3],
) {
    let total_ms = commands_ms + capture_ms + playback_ms;
    let phase = if capture_ms >= commands_ms && capture_ms >= playback_ms {
        "capture"
    } else if playback_ms >= commands_ms {
        "playback"
    } else {
        "commands"
    };
    if total_ms > daemon.loop_slowest_ms {
        daemon.loop_slowest_ms = total_ms;
        phase.clone_into(&mut daemon.loop_slowest_phase);
    }
    if total_ms <= LOOP_STALL_MS {
        return;
    }
    daemon.loop_stalls += 1;
    let due = last_logged.is_none_or(|at| at.elapsed() >= LOOP_STALL_LOG_INTERVAL);
    if !due {
        return;
    }
    *last_logged = Some(std::time::Instant::now());
    daemon.log.record(
        Level::Warn,
        "loop",
        &format!(
            "a pass took {total_ms:.0} ms (commands {commands_ms:.0}, capture {capture_ms:.0}, \
             playback {playback_ms:.0}){}; {} such passes so far",
            if station.transmitting() {
                " while transmitting"
            } else {
                ""
            },
            daemon.loop_stalls
        ),
        &state_name(station),
    );
}

/// Hand the sound card everything the station has rendered, up to the backlog, and mark
/// whether it is transmitting. A whole burst goes over at once so nothing the loop does
/// afterwards can put a hole in it; a keying failure is logged and the burst abandoned
/// rather than taking the daemon down and stranding the key.
fn fill_card(
    station: &mut Station<Box<dyn Ptt>>,
    audio: &mut dyn AudioIo,
    backlog: usize,
    block: usize,
    daemon: &mut DaemonState,
    last_ptt_fault: &mut Option<std::time::Instant>,
) {
    let mut buffer = vec![0.0f32; block];
    while audio.queued() < backlog {
        let count = match station.playback(&mut buffer) {
            Ok(count) => count,
            Err(error) => {
                note_ptt_failure(daemon, station, &error, last_ptt_fault);
                break;
            }
        };
        if count == 0 {
            break;
        }
        audio.playback(&buffer[..count]);
    }
    audio.set_playing(station.transmitting());
}

/// The sound card stopped delivering audio while the radio was keyed. The key is released
/// against the card's own clock, which is frozen now, so nothing else would ever bring it
/// up — and the audio clock the session's timers ride is frozen too, so this is also the
/// only way the daemon notices a dead card at all. Force the key up before the transmitter
/// sits there keyed.
fn release_on_stall(daemon: &mut DaemonState, station: &mut Station<Box<dyn Ptt>>) {
    daemon.log.record(
        Level::Error,
        "audio",
        "the sound card stopped delivering audio while transmitting; releasing the key",
        &state_name(station),
    );
    if let Err(error) = station.abandon_tx() {
        daemon.log.record(
            Level::Error,
            "ptt",
            &format!("the radio would not release after the card stalled: {error}"),
            &state_name(station),
        );
    }
}

/// A keying failure in the run loop: log it, rate-limited so a dead rig does not fill the
/// log, and abandon the transmission in progress so the loop keeps receiving instead of
/// spinning on a burst it cannot key. The session is left to the engine's own timers, which
/// retry or end it — a keying hiccup should cost a burst, not the daemon and not the key.
fn note_ptt_failure(
    daemon: &mut DaemonState,
    station: &mut Station<Box<dyn Ptt>>,
    error: &PttError,
    last_logged: &mut Option<std::time::Instant>,
) {
    if last_logged.is_none_or(|at| at.elapsed() >= LOOP_STALL_LOG_INTERVAL) {
        *last_logged = Some(std::time::Instant::now());
        daemon.log.record(
            Level::Error,
            "ptt",
            &format!(
                "the radio would not key or release ({error}); the modem is receiving only \
                 until the keying interface recovers"
            ),
            &state_name(station),
        );
    }
    // best effort: drop the burst and bring the key up; a further error here is already
    // covered by the line above
    let _ = station.abandon_tx();
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
    // the card's queue holds a whole transmission, and the loop hands everything the
    // station has rendered to it at once — see the notes in `audio.rs`
    let backlog = ((config.radio.max_key_s + 1.0) * f64::from(config.audio.sample_rate)) as usize;
    let mut reported = AudioLosses::default();
    let mut last_metrics = std::time::Instant::now();
    let mut last_keyed = false;
    let mut last_stall_logged: Option<std::time::Instant> = None;
    let mut last_ptt_fault: Option<std::time::Instant> = None;
    let mut heard_changed_at: Option<std::time::Instant> = None;
    let mut last_audio = std::time::Instant::now();
    let mut wind_down: Option<std::time::Instant> = None;

    loop {
        if stopping.load(std::sync::atomic::Ordering::SeqCst)
            && wound_down(station, daemon, &mut wind_down)
        {
            stop(station, control, daemon, restarting);
            return Ok(());
        }

        let pass_began = std::time::Instant::now();
        answer_commands(station, control, daemon, stopping, restarting);
        sync_kiss(station, daemon);
        let commands_ms = pass_began.elapsed().as_secs_f64() * 1000.0;

        // the card's clock goes in ahead of the block, so a block that outlasts this
        // station's own transmission is muted only up to where the transmission ended
        station.device_played(audio.played());
        let captured = audio.capture();
        let idle = captured.is_empty();
        if !idle {
            last_audio = std::time::Instant::now();
            if let Err(error) = station.capture(&captured) {
                note_ptt_failure(daemon, station, &error, &mut last_ptt_fault);
            }
        } else if station.transmitting() && last_audio.elapsed() >= AUDIO_STALL_RELEASE {
            // The card has stopped delivering audio while the radio is keyed: nothing else
            // will bring the key up (see `release_on_stall`), so force it on the wall clock.
            release_on_stall(daemon, station);
            last_audio = std::time::Instant::now();
        }
        let capture_ms = pass_began.elapsed().as_secs_f64() * 1000.0 - commands_ms;

        // A transmission cut short leaves the rest of it in the card's queue: drop it.
        if station.take_device_flush() {
            audio.clear();
        }
        // Hand the card everything the station has rendered. A burst goes over whole, the
        // moment it is rendered, so nothing this loop does afterwards — a decode, a slow
        // disk, the scheduler — can put a hole in it; the station releases the key against
        // the card's own clock, when the last sample has really left.
        station.device_played(audio.played());
        fill_card(station, audio, backlog, block, daemon, &mut last_ptt_fault);
        let playback_ms = pass_began.elapsed().as_secs_f64() * 1000.0 - commands_ms - capture_ms;
        note_pass(
            daemon,
            &mut last_stall_logged,
            station,
            [commands_ms, capture_ms, playback_ms],
        );

        // frames before the state they produced: a host learns the SNR of the frame that
        // brought a session up (SN) before it hears CONNECTED, which is the order a modem
        // that reports frames as it decodes them gives, and what VarAC builds its
        // opening signal report from
        report_frames(station, control, daemon, &mut heard_changed_at);
        publish_station(station, control, daemon);

        // metrics are the operator's window into the link, so they go out while it runs
        if control.subscriber_count() > 0 && last_metrics.elapsed() >= METRICS_INTERVAL {
            last_metrics = std::time::Instant::now();
            control.publish(&Event::new("metrics", metrics(station)));
            station.reset_busy_peak();
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

        note_audio_losses(
            daemon,
            audio,
            config.audio.sample_rate,
            station,
            &mut reported,
        );

        if idle {
            std::thread::sleep(IDLE_SLEEP);
        }
    }
}

/// The longest a stop waits for a session's DISC and identifier to leave the air: a floor
/// control frame is 3.2 s and a callsign in Morse at 20 wpm about six.
const WIND_DOWN: std::time::Duration = std::time::Duration::from_secs(12);

/// Whether a daemon asked to stop may go now. A session is ended on the air first: its DISC,
/// so the other station is not left to time out, and — when this one identifies — the
/// identifier that ends the communication. Closing the desktop application in a session
/// used to leave the peer transmitting into nothing (ND1J, 2026-09-25). Bounded by
/// [`WIND_DOWN`]: a stop is a stop.
fn wound_down(
    station: &mut Station<Box<dyn Ptt>>,
    daemon: &mut DaemonState,
    wind_down: &mut Option<std::time::Instant>,
) -> bool {
    if wind_down.is_none() && !station.quiescent() {
        daemon.log.record(
            Level::Info,
            "daemon",
            "ending the session on the air before stopping",
            &state_name(station),
        );
        station.abort();
        *wind_down = Some(std::time::Instant::now());
    }
    wind_down.is_none_or(|since| station.quiescent() || since.elapsed() >= WIND_DOWN)
}

/// The way out: the log says why, the stations heard and the sessions are written, the
/// radio is released, and every reply still on its way to a client is written.
fn stop(
    station: &mut Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
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
    // a session the wind-down ended is history too
    note_sessions(station, None, daemon);
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
    // `shutdown` was answered on the pass before and its connection may not have written the
    // answer yet: exiting now would close the socket with nothing on it, and a restart the
    // panel or the desktop shell asked for would look to it like a crash
    if !control.settle(REPLY_GRACE) {
        daemon.log.record(
            Level::Warn,
            "control",
            "stopping with a reply still unwritten: a client stopped reading",
            &state_name(station),
        );
    }
}

/// What the station has to say goes to the log and the clients: its events (the session's
/// comings and goings as `state`, the rest as `log`), what became of each message sent with
/// a reference (`sent`), and the payload received (`data`).
fn publish_station(
    station: &mut Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
) {
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
    for delivery in station.take_deliveries() {
        // how much, and whether it arrived: never the text itself
        let said = if delivery.delivered {
            format!("{} bytes of text delivered", delivery.bytes)
        } else {
            format!(
                "{} bytes of text not delivered: {}",
                delivery.bytes,
                delivery.reason.as_deref().unwrap_or("the session ended")
            )
        };
        daemon
            .log
            .record(Level::Info, "sent", &said, &state_name(station));
        control.publish(&Event::new(
            "sent",
            serde_json::to_value(&delivery).unwrap_or(serde_json::Value::Null),
        ));
    }
    for decision in station.take_regulatory_reports() {
        report_regulatory(station, control, daemon, &decision);
    }
    publish_datagrams(station, control, daemon);
    let received = station.take_received();
    if !received.is_empty() {
        // The payload reaches panels through this event and host programs through the
        // host interface. It never goes to standard output: that is a log sink, and a
        // binary or compressed stream printed there is the garbage that filled the log
        // of the first radio-to-radio test.
        control.publish(&Event::new(
            "data",
            json!({"data": aetherd::control::methods::to_base64(&received)}),
        ));
    }
}

/// KISS clients' datagrams (ADR-0019): each one heard goes to the clients as a `datagram`
/// event — the frame, its type and who sent it — and what became of each one sent with a
/// reference as a `datagram-sent`, which is how a KISS ACKMODE client learns its frame left.
/// The log says a datagram came or went and how long it was; never what it said.
fn publish_datagrams(
    station: &mut Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
) {
    for heard in station.take_received_datagrams() {
        daemon.log.record(
            Level::Info,
            "kiss",
            &format!(
                "datagram from {}: {} bytes, type {}, at {:.1} dB",
                heard.source,
                heard.frame.len(),
                heard.frame_type,
                heard.snr_db
            ),
            &state_name(station),
        );
        control.publish(&Event::new(
            "datagram",
            json!({
                "source": heard.source,
                "frame_type": heard.frame_type,
                "data": aetherd::control::methods::to_base64(&heard.frame),
                "bytes": heard.frame.len(),
                "snr_db": heard.snr_db,
                "rung": heard.rung,
            }),
        ));
    }
    for report in station.take_datagram_reports() {
        if !report.sent {
            daemon.log.record(
                Level::Warn,
                "kiss",
                &format!(
                    "a datagram did not go out: {}",
                    report.reason.as_deref().unwrap_or("no reason given")
                ),
                &state_name(station),
            );
        }
        control.publish(&Event::new(
            "datagram-sent",
            json!({ "ref": report.reference, "sent": report.sent, "reason": report.reason }),
        ));
    }
}

/// A regulatory decision, logged as one structured line — everything an on-air test needs
/// to show why the modem did what it did — and published whole as a `regulatory` event.
fn report_regulatory(
    station: &Station<Box<dyn Ptt>>,
    control: &aetherd::control::ControlChannel,
    daemon: &mut DaemonState,
    d: &aetherd::regulatory::Decision,
) {
    use aetherd::regulatory::Verdict;
    let verdict = match d.verdict {
        Verdict::Legal => "permitted",
        Verdict::Warning => "permitted-with-warning",
        Verdict::Blocked => "blocked",
    };
    let rf = match (d.rf_low_hz, d.rf_high_hz) {
        (Some(lo), Some(hi)) => format!("{lo:.1}-{hi:.1}"),
        _ => "unknown".to_owned(),
    };
    let word = |w: Option<&'static str>| w.unwrap_or("unset");
    let line = format!(
        "decision={verdict} rule=\"{}\" code={} callsign={} dial_hz={} dial_source={} \
         sideband={} rf_hz={rf} bandwidth_hz={:.0} mode=\"{}\" emission={} control={} \
         direction={} session={} license={} reason=\"{}\"",
        d.rule,
        d.code,
        station.engine().my_call,
        d.dial_hz
            .map_or_else(|| "unknown".to_owned(), |hz| format!("{hz:.0}")),
        d.dial_source.map_or("none", |s| match s {
            aetherd::regulatory::DialSource::Radio => "radio",
            aetherd::regulatory::DialSource::Declared => "declared",
        }),
        word(d.sideband.map(aetherd::regulatory::Sideband::name)),
        d.bandwidth_hz,
        d.what,
        d.kind.name(),
        word(d.control.map(aetherd::regulatory::ControlMode::name)),
        serde_json::to_value(d.direction)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        state_name(station),
        word(d.license.map(aetherd::regulatory::LicenseClass::name)),
        d.summary,
    );
    let level = if d.allowed() {
        Level::Info
    } else {
        Level::Warn
    };
    daemon
        .log
        .record(level, "regulatory", &line, &state_name(station));
    let mut event = serde_json::to_value(d).unwrap_or(serde_json::Value::Null);
    if let Some(map) = event.as_object_mut() {
        map.insert("callsign".into(), json!(station.engine().my_call));
        map.insert("session".into(), json!(state_name(station)));
    }
    control.publish(&Event::new("regulatory", event));
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
    note_sessions(station, Some(control), daemon);
}

/// A session that ended joins the history, which is written at once — sessions end rarely
/// — and the clients are told.
fn note_sessions(
    station: &mut Station<Box<dyn Ptt>>,
    control: Option<&aetherd::control::ControlChannel>,
    daemon: &mut DaemonState,
) {
    let finished = station.take_finished_sessions();
    if finished.is_empty() {
        return;
    }
    let now_ms = unix_ms_now();
    for session in finished {
        let session = session.ended_at(now_ms);
        if let Some(control) = control {
            control.publish(&Event::new(
                "session",
                serde_json::to_value(&session).unwrap_or(serde_json::Value::Null),
            ));
        }
        daemon.sessions.add(session);
    }
    if let Err(error) = daemon.sessions.save() {
        daemon.log.record(
            Level::Warn,
            "sessions",
            &format!("the session history was not saved: {error}"),
            &state_name(station),
        );
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
        "datagram" => (Activity::Datagram, None),
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

/// The keying interface to run with — or one that says why there is none.
///
/// A serial port that is not there is a setting to correct, not a reason to refuse to run:
/// the Setup screen naming the port is served by this daemon, so a station that dies over a
/// bad port hides the only comfortable way to fix it. It starts receive-only instead, with
/// the reason on the panel, and puts nothing on the air until the setting is right.
fn keying(config: &Config, keys_nothing: bool, daemon: &mut DaemonState) -> Box<dyn Ptt> {
    // A dry run keys nothing, whatever the file says. Somebody checking their configuration
    // must not put a carrier on the air to find out that they had the wrong serial port.
    // Neither does a simulated channel: there is no radio on the other end of a socket.
    if keys_nothing {
        return Box::new(NullPtt::default());
    }
    match open_ptt(&config.ptt) {
        Ok(ptt) => ptt,
        Err(error) => {
            let reason = error.to_string();
            daemon.log.record(
                Level::Error,
                "ptt",
                &format!(
                    "{reason} — the modem is running receive-only and will not transmit; \
                     set the radio interface in Setup"
                ),
                "Idle",
            );
            Box::new(aetherd::ptt::BrokenPtt::new(reason))
        }
    }
}

/// The audio to run on — or silence that says why there is none.
///
/// Same reasoning as [`keying`]: the Setup screen naming the sound card is served by this
/// daemon, so dying over a card that is not plugged in hides the way to correct it. Silence
/// is paced by the clock, so the modem runs quietly rather than freezing.
///
/// # Errors
/// Only for a simulated channel that will not open, which is a developer's own doing.
fn sound(
    config: &Config,
    dry_run: bool,
    sim: Option<&aetherd::sim::SimConfig>,
    daemon: &mut DaemonState,
) -> Result<Box<dyn AudioIo>, String> {
    if dry_run {
        "dry run: audio loops back and nothing is keyed".clone_into(&mut daemon.audio);
        return Ok(Box::new(Loopback::new()));
    }
    if let Some(sim) = sim {
        let link = aetherd::sim::SimLink::open(sim)
            .map_err(|e| format!("cannot open the simulated channel: {e}"))?;
        daemon.audio.clone_from(&link.description);
        return Ok(Box::new(link));
    }
    Ok(match SoundCard::open(&config.audio_config()) {
        Ok(card) => {
            daemon.audio.clone_from(&card.description);
            Box::new(card)
        }
        Err(error) => {
            let reason = error.to_string();
            daemon.log.record(
                Level::Error,
                "audio",
                &format!(
                    "{reason} — the modem is running without audio and can neither hear \
                     nor transmit; set the modem devices in Setup"
                ),
                "Idle",
            );
            daemon.audio = format!("unavailable — {reason}");
            daemon.audio_fault = Some(reason);
            Box::new(aetherd::audio::Silence::new(config.audio.sample_rate))
        }
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
            // `source` is accepted so a saved configuration still loads, and ignored: a
            // Yaesu's keying command does not choose the input (see `ptt::CatProtocol`)
            source: _,
        } => {
            let protocol = match protocol {
                aetherd::config::CatProtocol::Yaesu => aetherd::ptt::CatProtocol::Yaesu,
                aetherd::config::CatProtocol::Kenwood => aetherd::ptt::CatProtocol::Kenwood,
                aetherd::config::CatProtocol::Icom => aetherd::ptt::CatProtocol::Icom {
                    // validated to be present when the protocol is Icom
                    address: civ_address.unwrap_or(0x00),
                },
            };
            Box::new(aetherd::ptt::CatPtt::open(port, *baud, protocol)?)
        }
        PttConfig::Cm108 { device, gpio } => {
            Box::new(aetherd::ptt::GpioPtt::open(device.as_deref(), *gpio)?)
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
