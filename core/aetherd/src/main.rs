//! The Aether HF station daemon.
//!
//! Reads a configuration file, opens a sound card and a keying interface, and runs the modem.
//! What it does not yet have is a control interface — that is P3-4 — so for now it listens,
//! answers, and can be told to call one station from the command line.
//!
//! # The loop
//!
//! One thread, driven by audio. Captured samples go into the station, which advances its
//! clock by exactly the audio it has heard, and playback is topped up to a small backlog so
//! the sound card never runs dry mid-burst. There is deliberately no wall clock anywhere: a
//! station that thinks a second has passed while its sound card delivered half a second will
//! answer bursts into the middle of them.

use std::{path::PathBuf, time::Duration};

use aether_link::LinkConfig;
use aetherd::{
    audio::{AudioIo, Loopback, SoundCard, list_devices},
    config::{Config, EXAMPLE, PttConfig},
    control::{
        ControlServer, channel,
        methods::{ConfigState, dispatch_with, metrics},
        protocol::Event,
    },
    host::HostServer,
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

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("aetherd: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

struct Args {
    config: Option<PathBuf>,
    call: Option<String>,
    dry_run: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = Args {
        config: None,
        call: None,
        dry_run: false,
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
                    println!("{port}");
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
            other => return Err(format!("unknown argument {other:?}; try --help")),
        }
    }
    Ok(Some(args))
}

const USAGE: &str = "\
aetherd — the Aether HF station daemon

    aetherd --config station.toml [--call W4ODA] [--dry-run]

  -c, --config PATH   the station configuration (required to run)
      --call CALL     call this station once the modem is up
      --dry-run       run the modem against an audio loopback, keying nothing
      --list-devices  print the audio devices this machine offers
      --list-ports    print the serial ports this machine offers
      --example-config  print a commented configuration to start from
      --version       print the version
  -h, --help          print this
";

fn run() -> Result<(), String> {
    let Some(args) = parse_args()? else {
        return Ok(());
    };
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

    let station_config = StationConfig {
        callsign: config.callsign.clone(),
        link: LinkConfig {
            max_mode: config.radio.max_mode,
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
    };

    // A dry run keys nothing, whatever the file says. Somebody checking their configuration
    // must not put a carrier on the air to find out that they had the wrong serial port.
    let ptt: Box<dyn Ptt> = if args.dry_run {
        Box::new(NullPtt::default())
    } else {
        open_ptt(&config.ptt).map_err(|e| e.to_string())?
    };
    let mut station = Station::new(station_config, ptt, seed_from_callsign(&config.callsign));
    println!(
        "aetherd: {} keying via {}",
        config.callsign,
        station.ptt_description()
    );

    let mut audio: Box<dyn AudioIo> = if args.dry_run {
        println!("aetherd: dry run — audio loops back and nothing is keyed");
        Box::new(Loopback::new())
    } else {
        let card = SoundCard::open(&config.audio_config()).map_err(|e| e.to_string())?;
        println!("aetherd: audio {}", card.description);
        Box::new(card)
    };

    // A gateway is stopped by its service manager sending a signal. Whatever else happens on
    // the way out, the transmitter has to be released: a station killed mid-burst would
    // otherwise sit there keyed until somebody noticed, which on an unattended station could
    // be a very long time.
    let stopping = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&stopping);
    if let Err(error) = ctrlc::set_handler(move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }) {
        eprintln!("aetherd: cannot catch a stop signal ({error}); the radio may stay keyed");
    }

    let (handle, control) = channel();
    let _server = if config.control.enabled {
        let server = ControlServer::start(&config.control_config(), handle.clone())
            .map_err(|e| e.to_string())?;
        println!("aetherd: control interface on ws://{}/v1", server.address);
        Some(server)
    } else {
        println!("aetherd: control interface disabled");
        None
    };

    let _host = if config.host.enabled {
        let server = HostServer::start(&config.host_config(), handle).map_err(|e| e.to_string())?;
        println!(
            "aetherd: host interface on {} (data {}) — it reports itself as {}",
            server.command_address,
            server.data_address,
            aetherd::host::vara::version_string()
        );
        Some(server)
    } else {
        None
    };

    if let Some(call) = &args.call {
        station.connect(call).map_err(str::to_owned)?;
        println!("aetherd: calling {call}");
    }

    let mut settings = ConfigState {
        config: config.clone(),
        path,
    };
    serve(
        &config,
        &mut station,
        audio.as_mut(),
        &control,
        &mut settings,
        &stopping,
    )
}

/// The run loop: audio in, audio out, control requests answered between blocks.
///
/// Never returns; the daemon is stopped from outside.
fn serve(
    config: &Config,
    station: &mut Station<Box<dyn Ptt>>,
    audio: &mut dyn AudioIo,
    control: &aetherd::control::ControlChannel,
    settings: &mut ConfigState,
    stopping: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let block = (0.02 * f64::from(config.audio.sample_rate)) as usize;
    let backlog = (PLAYBACK_BACKLOG_S * f64::from(config.audio.sample_rate)) as usize;
    let mut reported_drops = 0;
    let mut last_metrics = std::time::Instant::now();
    let mut last_keyed = false;

    loop {
        if stopping.load(std::sync::atomic::Ordering::SeqCst) {
            println!("aetherd: stopping");
            // Report the failure but do not return on it: there is nothing left to try, and
            // exiting quietly would hide a radio that is still keyed.
            if let Err(error) = station.shut_down() {
                eprintln!("aetherd: the radio would not release: {error}");
            }
            return Ok(());
        }

        // Control requests are answered from this thread, between audio blocks. The modem
        // is single-threaded because its clock is the audio it has heard, and a connection
        // reaching in from another thread would be able to change the state mid-frame.
        for command in control.drain() {
            // `shutdown` is the daemon's to answer, not the station's: it runs the same
            // path a stop signal does, so a supervisor that cannot send a signal — the
            // desktop shell on Windows — still gets the transmitter released properly.
            let response = if command.request.method == "shutdown" {
                stopping.store(true, std::sync::atomic::Ordering::SeqCst);
                aetherd::control::protocol::Response::ok(
                    command.request.id.clone(),
                    json!({ "stopping": true }),
                )
            } else {
                dispatch_with(station, Some(settings), &command.request)
            };
            let _ = command.reply.send(response);
        }

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
            println!("aetherd: {event}");
            let (name, detail) = event.split_once(':').unwrap_or(("log", event.as_str()));
            control.publish(&Event::new(
                if name == "connected" || name == "disconnected" || name == "role" {
                    "state"
                } else {
                    "log"
                },
                json!({"name": name, "detail": detail, "state": format!("{:?}", station.state())}),
            ));
        }
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
            control.publish(&Event::new("ptt", json!({ "on": keyed })));
        }

        let dropped = audio.dropped();
        if dropped > reported_drops {
            eprintln!(
                "aetherd: dropped {} audio samples — the modem is behind",
                dropped - reported_drops
            );
            reported_drops = dropped;
        }

        if idle {
            std::thread::sleep(IDLE_SLEEP);
        }
    }
}

fn open_ptt(config: &PttConfig) -> Result<Box<dyn Ptt>, PttError> {
    Ok(match config {
        PttConfig::None => Box::new(NullPtt::default()),
        PttConfig::Serial { port, line } => Box::new(SerialPtt::open(port, (*line).into())?),
        PttConfig::Rigctld { address } => {
            Box::new(RigctldPtt::new(address, Duration::from_millis(500)))
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
