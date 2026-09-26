//! The two TCP ports a VARA-compatible host expects, wired to the control API.
//!
//! A command port that speaks [`vara`](crate::host::vara), and a data port beside it carrying
//! payload bytes in both directions. The adapter is a *client* of the modem's own control
//! API — it sends `connect`, `send` and `disconnect` and listens for `state`, `ptt` and
//! `data` events — so it needs no privileged access to the station and cannot get at the
//! modem's internals. Everything it can do, a scripted client could do too.
//!
//! One host at a time. The published interface has no notion of several hosts sharing a
//! radio, and two programs taking turns keying one transmitter is not a situation to invent
//! semantics for: a second connection is accepted and then closed, so the client sees a clean
//! refusal instead of a silent hang.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::json;

use crate::{
    control::{
        methods::to_base64,
        protocol::{ControlHandle, Request},
    },
    host::vara::{DEFAULT_COMMAND_PORT, HostAction, HostState, Notification},
};

/// How the host interface is exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostConfig {
    /// Whether to listen at all.
    pub enabled: bool,
    /// Address for the command port. The data port is the next one up.
    pub bind: String,
    /// Print every command line received and every line sent, to standard error. Every
    /// client differs a little in what it sends, and the first run against a new one is
    /// a conversation worth reading verbatim.
    pub trace: bool,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: format!("127.0.0.1:{DEFAULT_COMMAND_PORT}"),
            trace: false,
        }
    }
}

/// Why the host interface would not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// An address could not be parsed or bound.
    Bind(String),
}

impl core::fmt::Display for HostError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bind(detail) => write!(f, "host interface: {detail}"),
        }
    }
}

impl core::error::Error for HostError {}

/// How often to tell a quiet host that the modem is still there.
const KEEPALIVE: Duration = Duration::from_secs(10);
/// How long the command reader blocks before giving the notification path a turn.
const POLL: Duration = Duration::from_millis(50);

/// A running VARA-compatible host interface. Stops when dropped.
pub struct HostServer {
    /// The command port, after any port-zero assignment.
    pub command_address: std::net::SocketAddr,
    /// The data port.
    pub data_address: std::net::SocketAddr,
    running: Arc<AtomicBool>,
    /// Whether a host program holds the command port right now. Shared with the status
    /// display, which is how an operator sees that Winlink Express or Pat is attached.
    pub connected: Arc<AtomicBool>,
}

impl std::fmt::Debug for HostServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostServer")
            .field("command", &self.command_address)
            .field("data", &self.data_address)
            .finish_non_exhaustive()
    }
}

impl Drop for HostServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        for address in [self.command_address, self.data_address] {
            let _ = TcpStream::connect_timeout(&address, Duration::from_millis(200));
        }
    }
}

/// The payload stream shared between the data connection and the command loop.
#[derive(Debug, Default)]
struct DataPipe {
    /// Bytes the host wrote, waiting to be handed to the modem.
    to_radio: Vec<u8>,
    /// Bytes the modem delivered, waiting to go to the host.
    to_host: Vec<u8>,
}

impl HostServer {
    /// Bind both ports and start serving.
    ///
    /// # Errors
    /// If either address cannot be bound. The data port is the command port plus one, which
    /// is what every client of the published interface expects.
    pub fn start(
        config: &HostConfig,
        handle: ControlHandle,
        flags: crate::kiss::HostFlags,
    ) -> Result<Self, HostError> {
        let (command, data) = bind_pair(&config.bind)?;
        let command_address = command
            .local_addr()
            .map_err(|e| HostError::Bind(format!("{e}")))?;
        let data_address = data
            .local_addr()
            .map_err(|e| HostError::Bind(format!("{e}")))?;

        let running = Arc::new(AtomicBool::new(true));
        let pipe: Arc<Mutex<DataPipe>> = Arc::new(Mutex::new(DataPipe::default()));
        let busy = Arc::new(AtomicBool::new(false));
        let connected = Arc::clone(&flags.attached);

        spawn_data_loop(data, Arc::clone(&pipe), Arc::clone(&running))?;
        spawn_command_loop(
            command,
            handle,
            pipe,
            busy,
            Arc::clone(&running),
            flags,
            config.trace,
        )?;

        Ok(Self {
            command_address,
            data_address,
            running,
            connected,
        })
    }
}

/// Bind the command port and the data port beside it.
///
/// Every client of the published interface assumes the data port is the command port plus
/// one, so they have to be taken together. Asking for port zero means "any free pair", which
/// needs a search: the operating system hands out one ephemeral port at a time and has no
/// notion of a consecutive pair, so a candidate is probed and retried until both are free.
fn bind_pair(address: &str) -> Result<(TcpListener, TcpListener), HostError> {
    let wanted: std::net::SocketAddr = address
        .parse()
        .map_err(|e| HostError::Bind(format!("{address} is not an address: {e}")))?;

    if wanted.port() != 0 {
        let command = TcpListener::bind(wanted).map_err(|e| {
            HostError::Bind(if e.kind() == std::io::ErrorKind::AddrInUse {
                format!(
                    "{wanted} is already in use: another modem — VARA itself, perhaps — or \
                     another aetherd is listening there. Stop it, or change [host] bind"
                )
            } else {
                format!("cannot listen on {wanted}: {e}")
            })
        })?;
        let mut data_address = wanted;
        data_address.set_port(wanted.port() + 1);
        let data = TcpListener::bind(data_address).map_err(|e| {
            HostError::Bind(format!(
                "the command port bound but the data port {data_address} did not: {e}. \
                 Clients expect them to be consecutive, so both have to be free."
            ))
        })?;
        return Ok((command, data));
    }

    let mut last = String::new();
    for _ in 0..32 {
        // let the system pick a port, then claim that one and its neighbour explicitly
        let Ok(probe) = TcpListener::bind(wanted) else {
            continue;
        };
        let Ok(chosen) = probe.local_addr() else {
            continue;
        };
        drop(probe);
        let mut data_address = chosen;
        let Some(next) = chosen.port().checked_add(1) else {
            continue;
        };
        data_address.set_port(next);
        match (TcpListener::bind(chosen), TcpListener::bind(data_address)) {
            (Ok(command), Ok(data)) => return Ok((command, data)),
            (command, data) => {
                last = match (command, data) {
                    (Err(e), _) | (_, Err(e)) => e.to_string(),
                    _ => unreachable!("both succeeded in the arm above"),
                };
            }
        }
    }
    Err(HostError::Bind(format!(
        "could not find a free consecutive port pair: {last}"
    )))
}

fn spawn_data_loop(
    listener: TcpListener,
    pipe: Arc<Mutex<DataPipe>>,
    running: Arc<AtomicBool>,
) -> Result<(), HostError> {
    std::thread::Builder::new()
        .name("aetherd-host-data".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                if !running.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                serve_data(&stream, &pipe, &running);
            }
        })
        .map(|_| ())
        .map_err(|e| HostError::Bind(format!("cannot start the data thread: {e}")))
}

/// Move payload bytes both ways for one data connection.
fn serve_data(stream: &TcpStream, pipe: &Arc<Mutex<DataPipe>>, running: &AtomicBool) {
    if stream.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = stream;
    let mut buffer = [0u8; 8192];
    while running.load(Ordering::Relaxed) {
        match reader.read(&mut buffer) {
            Ok(0) => return, // the host closed the data port
            Ok(count) => {
                if let Ok(mut pipe) = pipe.lock() {
                    pipe.to_radio.extend_from_slice(&buffer[..count]);
                }
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Err(_) => return,
        }

        let outbound = pipe
            .lock()
            .map(|mut pipe| std::mem::take(&mut pipe.to_host))
            .unwrap_or_default();
        if !outbound.is_empty() && writer.write_all(&outbound).is_err() {
            return;
        }
    }
}

fn spawn_command_loop(
    listener: TcpListener,
    handle: ControlHandle,
    pipe: Arc<Mutex<DataPipe>>,
    busy: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    flags: crate::kiss::HostFlags,
    trace: bool,
) -> Result<(), HostError> {
    let taken = Arc::clone(&flags.attached);
    std::thread::Builder::new()
        .name("aetherd-host-cmd".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                if !running.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                if taken.swap(true, Ordering::SeqCst) {
                    // one host at a time: refuse plainly rather than interleave two of them
                    let _ = (&stream).write_all(b"WRONG\r");
                    continue;
                }
                // A connection is served on its own thread so the listener keeps accepting.
                // Without that a second host would not be refused — it would simply never be
                // answered, which looks to its operator exactly like a hung radio.
                let handle = handle.clone();
                let pipe = Arc::clone(&pipe);
                let busy = Arc::clone(&busy);
                let running = Arc::clone(&running);
                let flags = flags.clone();
                let spawned = std::thread::Builder::new()
                    .name("aetherd-host-conn".to_owned())
                    .spawn(move || {
                        serve_commands(&stream, &handle, &pipe, &busy, &running, &flags, trace);
                        // what this host said about the KISS port goes with it
                        flags.chat.store(false, Ordering::SeqCst);
                        flags.ignore_dcd.store(false, Ordering::SeqCst);
                        flags.attached.store(false, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    taken.store(false, Ordering::SeqCst);
                }
            }
        })
        .map(|_| ())
        .map_err(|e| HostError::Bind(format!("cannot start the command thread: {e}")))
}

/// One host connection: commands in, replies and notifications out.
#[allow(clippy::too_many_lines)]
fn serve_commands(
    stream: &TcpStream,
    handle: &ControlHandle,
    pipe: &Arc<Mutex<DataPipe>>,
    busy: &AtomicBool,
    running: &AtomicBool,
    flags: &crate::kiss::HostFlags,
    trace: bool,
) {
    if stream.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut host = HostState::default();
    // the bandwidth the station runs decides which `BW<n>` is accepted and what
    // `CONNECTED` reports; asked once, when the host connects
    let capabilities = handle
        .call(request("capabilities", json!({})))
        .ok()
        .and_then(|response| response.result)
        .unwrap_or(serde_json::Value::Null);
    if let Some(hz) = capabilities["bandwidth_hz"]
        .as_u64()
        .and_then(|hz| u32::try_from(hz).ok())
    {
        host.bandwidth_hz = hz;
    }
    // the net bit rate of each mode, for the BITRATE line a host shows as the link speed
    let bit_rates: Vec<u64> = capabilities["modes"]
        .as_array()
        .map(|modes| {
            modes
                .iter()
                .map(|mode| mode["net_bit_rate"].as_f64().unwrap_or(0.0).round() as u64)
                .collect()
        })
        .unwrap_or_default();
    let mut last_mode: Option<usize> = None;
    let events = handle.subscribe();
    let mut last_keepalive = std::time::Instant::now();
    let mut connected_to: Option<String> = None;
    // a call this host placed that has neither come up nor been said to have ended: a host
    // waits for CONNECTED or DISCONNECTED after its CONNECT, and would wait for ever on neither
    let mut calling = false;

    let say = |writer: &mut &TcpStream, line: &str| -> bool {
        if trace {
            eprintln!("host -> {line}");
        }
        // the published interface terminates every line with a carriage return
        writer.write_all(format!("{line}\r").as_bytes()).is_ok()
    };

    // A host that has just attached is told the queue is empty. VarAC sends nothing on
    // the data port until it has heard how full the modem's buffer is, and a modem that
    // only reported changes never told it — found on the bench, where a ping sat for
    // ninety seconds with both ends waiting for the other
    if !say(&mut writer, &Notification::Buffer(0).line()) {
        return;
    }
    // A modem whose sound card would not open runs on silence and can neither hear nor
    // transmit; VARA tells its host MISSING SOUNDCARD, and a gateway's host acts on it
    let audio_fault = handle
        .call(request("status", json!({})))
        .ok()
        .and_then(|reply| reply.result)
        .is_some_and(|status| !status["audio_fault"].is_null());
    if audio_fault && !say(&mut writer, &Notification::MissingSoundcard.line()) {
        return;
    }

    while running.load(Ordering::Relaxed) {
        // ── commands from the host ────────────────────────────────────
        let mut line = Vec::new();
        match read_line(&mut reader, &mut line) {
            LineResult::Closed => return,
            LineResult::Idle => {}
            LineResult::Line => {
                let text = String::from_utf8_lossy(&line).to_string();
                if trace {
                    eprintln!("host <- {}", text.trim());
                }
                let outcome = host.command(&text);
                // `CHAT ON` and `IGNOREKISSDCD ON` govern the KISS port while this host is here
                flags.chat.store(host.recorded.chat, Ordering::SeqCst);
                flags
                    .ignore_dcd
                    .store(host.recorded.ignore_kiss_dcd, Ordering::SeqCst);
                for reply in &outcome.replies {
                    if !say(&mut writer, reply) {
                        return;
                    }
                }
                match apply(&outcome.action, handle, &host, &mut writer, &say) {
                    Applied::Done => {}
                    // Armed as soon as the modem has taken the call. What the modem said
                    // before then — the `disconnected` of a call just aborted — was sent
                    // before the answer to the command that caused it, and has been read by
                    // now: the daemon publishes what a request sets in motion before its reply
                    Applied::Calling => calling = true,
                    Applied::Closed => return,
                }
            }
        }

        // ── payload the host wrote on the data port ───────────────────
        let outbound = pipe
            .lock()
            .map(|mut pipe| std::mem::take(&mut pipe.to_radio))
            .unwrap_or_default();
        if !outbound.is_empty() {
            let _ = handle.call(request("send", json!({ "data": to_base64(&outbound) })));
        }

        // ── what the modem has to say ─────────────────────────────────
        for event in events.try_iter() {
            match event.event.as_str() {
                "ptt" => {
                    let on = event.data["on"].as_bool().unwrap_or(false);
                    if !say(&mut writer, &Notification::Ptt(on).line()) {
                        return;
                    }
                }
                "data" => {
                    if let Some(text) = event.data["data"].as_str() {
                        if let Some(bytes) = from_base64(text)
                            && let Ok(mut pipe) = pipe.lock()
                        {
                            pipe.to_host.extend_from_slice(&bytes);
                        }
                    }
                }
                "state" => {
                    if !report_state(
                        &event.data,
                        &host,
                        &mut connected_to,
                        &mut calling,
                        &mut writer,
                        &say,
                    ) {
                        return;
                    }
                }
                // the other station's frames, as they arrive: the SNR each was received
                // at is what a host builds its signal reports from
                // every decoded frame, as a modem that reports what it hears: the SNR of
                // the call that brings a session up arrives before CONNECTED, which is
                // when VarAC builds its opening signal report
                "frame" => {
                    if event.data["decoded"].as_bool() == Some(true)
                        && let Some(snr) = event.data["snr_db"].as_f64()
                        && !say(&mut writer, &Notification::SignalToNoise(snr).line())
                    {
                        return;
                    }
                    // A beacon heard is a CQ frame to a VARA host, which lists who is on from
                    // these lines — VarAC's heard list of beacons and CQs — after the SN line
                    // that gives its strength. The beacon does not say which bandwidth its
                    // sender runs, so the line carries this station's.
                    if event.data["decoded"].as_bool() == Some(true)
                        && event.data["kind"] == "beacon"
                        && let Some(source) = event.data["from"].as_str()
                        && !say(
                            &mut writer,
                            &Notification::CqFrame {
                                source: source.to_owned(),
                                bandwidth_hz: host.bandwidth_hz,
                            }
                            .line(),
                        )
                    {
                        return;
                    }
                }
                "metrics" => {
                    // the link speed, as a host displays it: the mode the sender is using
                    if let Some(mode) = event.data["mode"].as_u64().map(|m| m as usize)
                        && connected_to.is_some()
                        && last_mode != Some(mode)
                    {
                        last_mode = Some(mode);
                        let bps = bit_rates.get(mode).copied().unwrap_or(0);
                        if !say(&mut writer, &Notification::BitRate { mode, bps }.line()) {
                            return;
                        }
                    }
                    if let Some(queued) = event.data["queued_bytes"].as_u64() {
                        let queued = queued as usize;
                        if queued != host.buffer {
                            host.buffer = queued;
                            if !say(&mut writer, &Notification::Buffer(queued).line()) {
                                return;
                            }
                        }
                    }
                    // During a session the channel is the session's: the detector marks
                    // it busy at every frame of the other station, and a host that honours
                    // DCD (VarAC with Ignore DCD off, which holds "busy" for ten seconds
                    // after each) would never find a moment to hand its data over. The
                    // modem does the turn-taking, so a session reads as a clear channel.
                    let now_busy = connected_to.is_none()
                        && event.data["channel_busy"].as_bool().unwrap_or(false);
                    if now_busy != busy.swap(now_busy, Ordering::Relaxed)
                        && !say(&mut writer, &Notification::Busy(now_busy).line())
                    {
                        return;
                    }
                }
                _ => {}
            }
        }

        // ── keep-alive ────────────────────────────────────────────────
        if last_keepalive.elapsed() >= KEEPALIVE {
            last_keepalive = std::time::Instant::now();
            if !say(&mut writer, &Notification::IAmAlive.line()) {
                return;
            }
        }
    }
}

/// Turn a state event into the lines the published interface uses.
///
/// `connected_to` is the session the host was told of with `CONNECTED`; `calling`, a call it
/// placed that has not come up.
fn report_state(
    data: &serde_json::Value,
    host: &HostState,
    connected_to: &mut Option<String>,
    calling: &mut bool,
    writer: &mut &TcpStream,
    say: &impl Fn(&mut &TcpStream, &str) -> bool,
) -> bool {
    let name = data["name"].as_str().unwrap_or("");
    let detail = data["detail"].as_str().unwrap_or("");
    match name {
        "connected" => {
            // the detail is "<call> (role)"; the callsign is what the host wants, and the
            // role decides the order of the two: the published form is CONNECTED <caller>
            // <called>, so a station that was called puts the caller first. Pat ignores a
            // CONNECTED whose second callsign is not its own — found with Pat itself over
            // the simulated channel, when the called side never learned it had been called.
            let mut words = detail.split_whitespace();
            let remote = words.next().unwrap_or("").to_owned();
            let called = words.next() == Some("(irs)");
            *connected_to = Some(remote.clone());
            // whatever the host was calling, what it hears now is a session
            *calling = false;
            // the modem says which of the station's callsigns the session runs under; a host
            // that never said MYCALL is told the modem's own name
            let mine = data["callsign"]
                .as_str()
                .filter(|call| !call.is_empty())
                .or(host.my_call())
                .unwrap_or("")
                .to_owned();
            let (caller, callee) = if called {
                (remote, mine.clone())
            } else {
                (mine.clone(), remote)
            };
            // the called side hears PENDING first, as it would from any modem: the engine
            // answers a call in one step, so the two arrive together
            (!called || say(writer, &Notification::Pending.line()))
                // a client that thinks the modem is speed-limited warns its user about it;
                // this one is free software and has no such limit, so say so before the
                // session
                && say(writer, &Notification::Registered(mine).line())
                && say(
                    writer,
                    &Notification::Connected {
                        caller,
                        called: callee,
                        bandwidth_hz: host.bandwidth_hz,
                    }
                    .line(),
                )
                // what VARA says of a link once it is up, and true here
                && say(writer, &Notification::EncryptionDisabled.line())
                // and that the station at the other end is not speed-limited: nobody is
                && say(writer, &Notification::LinkRegistered.line())
        }
        // A session the host was told about, or a call it placed that ended without one: no
        // answer, `ABORT`, `DISCONNECT` while calling, the rules. Anything else — a call the
        // panel placed — is not this host's business.
        "disconnected" => {
            let told = connected_to.take().is_some();
            let placed = std::mem::take(calling);
            if told || placed {
                say(writer, &Notification::Disconnected.line())
            } else {
                true
            }
        }
        _ => true,
    }
}

/// What carrying out a command came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    /// Done, with nothing to remember.
    Done,
    /// The modem took a call: how it ends — a session, or none — is the host's to hear.
    Calling,
    /// A line could not be written: the host has gone.
    Closed,
}

impl From<bool> for Applied {
    /// Whatever had to be said was: done, or the host has gone.
    fn from(said: bool) -> Self {
        if said { Self::Done } else { Self::Closed }
    }
}

/// Carry out what a command asked for, through the modem's own control API.
fn apply(
    action: &HostAction,
    handle: &ControlHandle,
    host: &HostState,
    writer: &mut &TcpStream,
    say: &impl Fn(&mut &TcpStream, &str) -> bool,
) -> Applied {
    match action {
        HostAction::None | HostAction::Listen(_) => Applied::Done,
        HostAction::Callsigns(calls) => {
            // the reply already said OK, which is the truth: the modem takes the list, and
            // when a session is up it takes effect as that session ends
            let _ = handle.call(request("callsigns.set", json!({ "callsigns": calls })));
            Applied::Done
        }
        HostAction::Connect { from, to } => {
            // CONNECT names the calling station; when it is one the host registered, the
            // session runs under it, which is how a station with a club or tactical call
            // besides its own chooses between them
            let mut params = json!({ "remote": to });
            if host.answers_to(from) {
                params["callsign"] = json!(from);
            }
            let response = handle.call(request("connect", params));
            match response {
                Ok(reply) if reply.ok => Applied::Calling,
                // the modem refused: the host has to hear that the call did not happen
                _ => say(writer, &Notification::Disconnected.line()).into(),
            }
        }
        HostAction::Disconnect => {
            // A host that says DISCONNECT while its call is going out means "stop calling",
            // as a VARA host does; the modem's own `disconnect` keeps calling and closes the
            // session once it is up. Nothing of a call is there to close in order, so it is
            // abandoned, as the panel's Stop calling does — and the host hears DISCONNECTED
            // for it.
            let calling = handle
                .call(request("status", json!({})))
                .ok()
                .and_then(|reply| reply.result)
                .is_some_and(|status| status["state"] == "connecting");
            let method = if calling { "abort" } else { "disconnect" };
            let _ = handle.call(request(method, json!({})));
            Applied::Done
        }
        HostAction::Abort => {
            let _ = handle.call(request("abort", json!({})));
            Applied::Done
        }
        HostAction::CqFrame => {
            let _ = handle.call(request("beacon", json!({})));
            Applied::Done
        }
        // `TUNE OFF`: cut the tone short, if one is playing or queued
        HostAction::Tune(seconds) if *seconds <= 0.0 => {
            let _ = handle.call(request("tune", json!({ "duration_s": 0.0 })));
            Applied::Done
        }
        HostAction::Tune(seconds) => {
            // the modem bounds a tone at ten seconds; a host asking for more gets ten, and
            // the reply already said OK because the command was understood
            let bounded = seconds.min(10.0);
            let _ = handle.call(request("tune", json!({ "duration_s": bounded })));
            Applied::Done
        }
        // the transmit level as decibels below full scale, the one scale a sine amplitude
        // has an honest reading on
        HostAction::TuneLevel => {
            let level = handle
                .call(request("config.get", json!({})))
                .ok()
                .filter(|reply| reply.ok)
                .and_then(|reply| reply.result)
                .and_then(|result| result["config"]["audio"]["tx_level"].as_f64())
                .filter(|level: &f64| *level > 0.0);
            match level {
                Some(level) => say(
                    writer,
                    &format!("TUNE {}", (20.0 * f64::log10(level)).round() as i64),
                ),
                None => say(writer, "WRONG"),
            }
            .into()
        }
    }
}

fn request(method: &str, params: serde_json::Value) -> Request {
    Request {
        id: None,
        method: method.to_owned(),
        params,
    }
}

enum LineResult {
    Line,
    Idle,
    Closed,
}

/// Read one command, which the published interface terminates with a carriage return.
fn read_line(reader: &mut BufReader<TcpStream>, out: &mut Vec<u8>) -> LineResult {
    match reader.read_until(b'\r', out) {
        Ok(0) => LineResult::Closed,
        Ok(_) => {
            while out.last().is_some_and(|&c| c == b'\r' || c == b'\n') {
                out.pop();
            }
            LineResult::Line
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut =>
        {
            LineResult::Idle
        }
        Err(_) => LineResult::Closed,
    }
}

/// The base64 the control API uses, in reverse.
fn from_base64(text: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{
        Event, channel,
        protocol::{ControlChannel, Response},
    };

    /// What the stub modem was asked, in order: method and params.
    type Seen = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    /// A stub modem: answers every request, remembers it, and can publish events on demand.
    fn stub_modem(control: ControlChannel) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>, Seen) {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let worker = std::thread::spawn(move || {
            let mut seen_connect = false;
            while !flag.load(Ordering::Relaxed) {
                for command in control.drain() {
                    if command.request.method == "connect" {
                        seen_connect = true;
                    }
                    if let Ok(mut log) = log.lock() {
                        log.push((
                            command.request.method.clone(),
                            command.request.params.clone(),
                        ));
                    }
                    // the one request the adapter reads a value from: the transmit level
                    let result = if command.request.method == "config.get" {
                        json!({ "config": { "audio": { "tx_level": 0.25 } } })
                    } else {
                        json!({})
                    };
                    let _ = command
                        .reply
                        .send(Response::ok(command.request.id.clone(), result));
                }
                if seen_connect {
                    seen_connect = false;
                    control.publish(&Event::new(
                        "state",
                        json!({"name": "connected", "detail": "KK4XYZ (iss)", "callsign": "W4ODA"}),
                    ));
                    // the peer's answer, then a third station's beacon, then one of the
                    // peer's frames that did not decode: one SN line among the three
                    // the other station's frame marks the channel busy: not reported while
                    // the session is up
                    control.publish(&Event::new(
                        "metrics",
                        json!({"queued_bytes": 0, "channel_busy": true, "mode": 0}),
                    ));
                    for frame in [
                        json!({"kind": "answer", "decoded": true, "from": "KK4XYZ", "to": "W4ODA", "snr_db": 12.4}),
                        json!({"kind": "beacon", "decoded": true, "from": "N0CALL", "snr_db": 3.0}),
                        json!({"kind": "data", "decoded": false, "from": "KK4XYZ", "snr_db": -1.0}),
                    ] {
                        control.publish(&Event::new("frame", frame));
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        (worker, stop, seen)
    }

    struct Client {
        command: TcpStream,
        reader: BufReader<TcpStream>,
    }

    impl Client {
        fn connect(server: &HostServer) -> Self {
            let command = TcpStream::connect(server.command_address).expect("command port");
            command
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let reader = BufReader::new(command.try_clone().expect("clone"));
            Self { command, reader }
        }

        fn send(&mut self, line: &str) {
            self.command
                .write_all(format!("{line}\r").as_bytes())
                .expect("write");
        }

        fn line(&mut self) -> String {
            let mut out = Vec::new();
            self.reader.read_until(b'\r', &mut out).expect("read");
            String::from_utf8_lossy(&out).trim().to_owned()
        }

        /// Read until a line satisfies the predicate, so a keep-alive cannot fail a test.
        fn expect(&mut self, want: impl Fn(&str) -> bool) -> String {
            for _ in 0..20 {
                let line = self.line();
                if want(&line) {
                    return line;
                }
            }
            panic!("the modem never said what was expected");
        }
    }

    fn server() -> (
        HostServer,
        std::thread::JoinHandle<()>,
        Arc<AtomicBool>,
        Seen,
    ) {
        let (handle, control) = channel();
        let (worker, stop, seen) = stub_modem(control);
        let server = HostServer::start(
            &HostConfig {
                enabled: true,
                bind: "127.0.0.1:0".to_owned(),
                trace: false,
            },
            handle,
            crate::kiss::HostFlags::default(),
        )
        .expect("start");
        (server, worker, stop, seen)
    }

    #[test]
    fn the_data_port_is_the_command_port_plus_one() {
        // every client of the published interface assumes it; a modem that put it elsewhere
        // would connect and then hang with no data
        let (server, worker, stop, _) = server();
        assert_eq!(
            server.data_address.port(),
            server.command_address.port() + 1
        );
        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");
    }

    #[test]
    fn a_host_can_set_up_and_call() {
        let (server, worker, stop, seen) = server();
        let mut client = Client::connect(&server);
        // the first thing a host hears is that the queue is empty; VarAC waits for it
        assert_eq!(client.expect(|l| l.starts_with("BUFFER")), "BUFFER 0");

        client.send("MYCALL W4ODA");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("BW2300");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("LISTEN ON");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("VERSION");
        let version = client.expect(|l| l.starts_with("VERSION"));
        assert!(version.contains("Aether"), "{version}");
        // VarAC asks the transmit level after every connection: a sine amplitude of 0.25
        // is 12 dB below full scale
        client.send("TUNE ?");
        assert_eq!(client.expect(|l| l.starts_with("TUNE")), "TUNE -12");

        client.send("CONNECT W4ODA KK4XYZ");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        let connected = client.expect(|l| l.starts_with("CONNECTED"));
        assert_eq!(connected, "CONNECTED W4ODA KK4XYZ 2300");
        // and, as VARA says of a registered peer, that the other end is not speed-limited
        assert_eq!(client.expect(|l| l.starts_with("LINK")), "LINK REGISTERED");
        // every decoded frame arrives as an SN line — whole decibels — and an undecoded
        // one does not: VarAC's signal reports, and its ping, are built from them
        assert_eq!(client.expect(|l| l.starts_with("SN")), "SN 12");
        assert_eq!(client.expect(|l| l.starts_with("SN")), "SN 3");
        // the third station's beacon is a CQ frame to a VARA host, after the SN line that
        // gives its strength — how VarAC lists the beacons and CQs it hears
        assert_eq!(
            client.expect(|l| l.starts_with("CQFRAME")),
            "CQFRAME N0CALL 2300"
        );
        std::thread::sleep(Duration::from_millis(100));
        client.send("BUFFER");
        let next = client
            .expect(|l| l.starts_with("SN") || l.starts_with("BUFFER") || l.starts_with("BUSY"));
        assert_eq!(
            next, "BUFFER 0",
            "an undecoded frame, or the session's own busy channel, was reported"
        );

        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");

        // Winlink Express at both ends of the simulated channel: each end's MYCALL went to
        // the adapter and no further, so the modems kept the callsigns in their
        // configuration files, and the call to KK4ODA-2 was never answered. The host owns
        // the operator's callsign, so MYCALL has to reach the modem, and CONNECT has to say
        // which of them the session runs under.
        let seen = seen.lock().expect("seen");
        assert!(
            seen.iter().any(|(method, params)| method == "callsigns.set"
                && params["callsigns"] == json!(["W4ODA"])),
            "{seen:?}"
        );
        assert!(
            seen.iter().any(|(method, params)| method == "connect"
                && params["remote"] == "KK4XYZ"
                && params["callsign"] == "W4ODA"),
            "{seen:?}"
        );
    }

    #[test]
    fn a_station_that_was_called_puts_the_caller_first() {
        // Pat over the simulated channel: the called side's Pat never learned it had been
        // called, because CONNECTED named this station first and a host ignores a CONNECTED
        // whose second callsign is not its own. The published form is <caller> <called>.
        let (handle, control) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let mut announced = false;
            while !flag.load(Ordering::Relaxed) {
                for command in control.drain() {
                    let _ = command
                        .reply
                        .send(Response::ok(command.request.id.clone(), json!({})));
                }
                if !announced && control.subscriber_count() > 0 {
                    announced = true;
                    // somebody called us: the modem reports the peer and our role
                    control.publish(&Event::new(
                        "state",
                        json!({"name": "connected", "detail": "KK4XYZ (irs)"}),
                    ));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let server = HostServer::start(
            &HostConfig {
                enabled: true,
                bind: "127.0.0.1:0".to_owned(),
                trace: false,
            },
            handle,
            crate::kiss::HostFlags::default(),
        )
        .expect("start");
        let mut client = Client::connect(&server);
        client.send("MYCALL W4ODA");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("LISTEN ON");
        // the call comes whenever it comes: before or after LISTEN ON is answered
        let pending = client.expect(|l| l == "PENDING" || l.starts_with("CONNECTED"));
        assert_eq!(pending, "PENDING", "the called side hears PENDING first");
        let connected = client.expect(|l| l.starts_with("CONNECTED"));
        assert_eq!(connected, "CONNECTED KK4XYZ W4ODA 2300");
        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");
    }

    /// A stub modem the test scripts: `answer` gives each request's result, and publishes
    /// what the request set in motion before that — the order the daemon keeps — and the test
    /// publishes events of its own whenever it likes.
    struct Modem {
        control: Arc<Mutex<ControlChannel>>,
        seen: Seen,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Modem {
        fn start(
            answer: impl Fn(&str, &ControlChannel) -> serde_json::Value + Send + 'static,
        ) -> (Self, HostServer) {
            let (handle, control) = channel();
            let control = Arc::new(Mutex::new(control));
            let stop = Arc::new(AtomicBool::new(false));
            let seen: Seen = Arc::new(Mutex::new(Vec::new()));
            let worker = {
                let (control, stop, seen) =
                    (Arc::clone(&control), Arc::clone(&stop), Arc::clone(&seen));
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        if let Ok(control) = control.lock() {
                            for command in control.drain() {
                                let method = command.request.method.clone();
                                if let Ok(mut seen) = seen.lock() {
                                    seen.push((method.clone(), command.request.params.clone()));
                                }
                                let result = answer(&method, &control);
                                let _ = command
                                    .reply
                                    .send(Response::ok(command.request.id.clone(), result));
                            }
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                })
            };
            let server = HostServer::start(
                &HostConfig {
                    enabled: true,
                    bind: "127.0.0.1:0".to_owned(),
                    trace: false,
                },
                handle,
                crate::kiss::HostFlags::default(),
            )
            .expect("start");
            let modem = Self {
                control,
                seen,
                stop,
                worker: Some(worker),
            };
            (modem, server)
        }

        /// An event, as the run loop publishes one.
        fn publish(&self, name: &str, data: serde_json::Value) {
            self.control
                .lock()
                .expect("control")
                .publish(&Event::new(name, data));
        }

        /// The methods the adapter asked for, in order.
        fn methods(&self) -> Vec<String> {
            self.seen
                .lock()
                .expect("seen")
                .iter()
                .map(|(method, _)| method.clone())
                .collect()
        }
    }

    impl Drop for Modem {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    /// The `state` event the modem publishes when a call or a session ends.
    fn ended(how: &str) -> serde_json::Value {
        json!({"name": "disconnected", "detail": how, "state": "Idle",
               "callsign": "W4ODA", "remote": "KK4XYZ"})
    }

    /// Whether the host hears `DISCONNECTED` before the answer to a `VERSION` asked now —
    /// after a pause long enough for the adapter to have passed on anything already said.
    fn told_disconnected_before_version(client: &mut Client) -> bool {
        std::thread::sleep(Duration::from_millis(200));
        client.send("VERSION");
        client.expect(|l| l == "DISCONNECTED" || l.starts_with("VERSION")) == "DISCONNECTED"
    }

    #[test]
    fn a_call_nobody_answers_ends_in_disconnected() {
        // A host waits for CONNECTED or DISCONNECTED after its CONNECT. A call the modem gave
        // up on ("no answer") produced neither, and a host waiting on it waited for ever.
        let (modem, server) = Modem::start(|_, _| json!({}));
        let mut client = Client::connect(&server);
        assert_eq!(client.expect(|l| l.starts_with("BUFFER")), "BUFFER 0");
        // a call this host did not place — the panel's — is none of its business
        modem.publish("state", ended("no answer"));
        assert!(!told_disconnected_before_version(&mut client));

        client.send("CONNECT W4ODA KK4XYZ");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        assert!(!told_disconnected_before_version(&mut client));
        modem.publish("state", ended("no answer"));
        assert_eq!(client.expect(|l| l == "DISCONNECTED"), "DISCONNECTED");
        // and told once
        assert!(!told_disconnected_before_version(&mut client));
    }

    #[test]
    fn a_call_the_host_aborts_ends_in_disconnected() {
        let (_modem, server) = Modem::start(|method, control| {
            if method == "abort" {
                control.publish(&Event::new("state", ended("aborted")));
            }
            json!({})
        });
        let mut client = Client::connect(&server);
        client.send("CONNECT W4ODA KK4XYZ");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("ABORT");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        assert_eq!(
            client.expect(|l| l == "DISCONNECTED" || l.starts_with("CONNECTED")),
            "DISCONNECTED"
        );
    }

    #[test]
    fn disconnect_while_calling_stops_the_call() {
        // A VARA host's DISCONNECT during a call means "stop calling". The modem's own
        // `disconnect` kept calling and closed the session once it came up, so a host that
        // had given up on a call could find itself in a session it no longer expected.
        let state = Arc::new(Mutex::new("idle"));
        let shared = Arc::clone(&state);
        let (modem, server) = Modem::start(move |method, control| {
            let mut state = shared.lock().expect("state");
            match method {
                "connect" => *state = "connecting",
                "abort" => {
                    *state = "idle";
                    control.publish(&Event::new("state", ended("aborted")));
                }
                _ => {}
            }
            json!({ "state": *state })
        });
        let mut client = Client::connect(&server);
        client.send("CONNECT W4ODA KK4XYZ");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        client.send("DISCONNECT");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        assert_eq!(client.expect(|l| l == "DISCONNECTED"), "DISCONNECTED");
        let methods = modem.methods();
        assert!(
            methods.iter().any(|m| m == "abort") && !methods.iter().any(|m| m == "disconnect"),
            "{methods:?}"
        );

        // in a session it is the orderly close it always was
        *state.lock().expect("state") = "connected";
        client.send("DISCONNECT");
        assert_eq!(client.expect(|l| l == "OK" || l == "WRONG"), "OK");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !modem.methods().iter().any(|m| m == "disconnect") {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                modem.methods()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            modem.methods().iter().filter(|m| *m == "abort").count(),
            1,
            "a session was aborted"
        );
    }

    #[test]
    fn a_call_aborted_and_placed_again_at_once_is_told_apart() {
        // A host that does not wait for the DISCONNECTED of the call it aborted: the first
        // call's end must not be taken for the second's, nor the second go unreported. The
        // modem publishes what the abort set in motion before its answer, which is what
        // lets the adapter tell the two apart.
        let (modem, server) = Modem::start(|method, control| {
            if method == "abort" {
                control.publish(&Event::new("state", ended("aborted")));
            }
            json!({})
        });
        let mut client = Client::connect(&server);
        assert_eq!(client.expect(|l| l.starts_with("BUFFER")), "BUFFER 0");
        client.send("CONNECT W4ODA KK4XYZ\rABORT\rCONNECT W4ODA KK4XYZ");
        assert_eq!(client.expect(|l| l == "DISCONNECTED"), "DISCONNECTED");
        // the second call is still out, and nothing more is said of it
        assert!(!told_disconnected_before_version(&mut client));
        // until it ends
        modem.publish("state", ended("no answer"));
        assert_eq!(client.expect(|l| l == "DISCONNECTED"), "DISCONNECTED");
    }

    #[test]
    fn payload_written_to_the_data_port_reaches_the_modem() {
        let (handle, control) = channel();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                for command in control.drain() {
                    if command.request.method == "send"
                        && let Some(data) = command.request.params["data"].as_str()
                        && let Ok(mut seen) = recorder.lock()
                    {
                        seen.push(data.to_owned());
                    }
                    let _ = command.reply.send(Response::ok(None, json!({})));
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        let server = HostServer::start(
            &HostConfig {
                enabled: true,
                bind: "127.0.0.1:0".to_owned(),
                trace: false,
            },
            handle,
            crate::kiss::HostFlags::default(),
        )
        .expect("start");
        let _client = Client::connect(&server);
        let mut data = TcpStream::connect(server.data_address).expect("data port");
        data.write_all(b"hello over the air").expect("write");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Ok(seen) = seen.lock()
                && !seen.is_empty()
            {
                got.clone_from(&seen);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");

        assert!(!got.is_empty(), "nothing reached the modem");
        let decoded: Vec<u8> = got
            .iter()
            .flat_map(|t| from_base64(t).unwrap_or_default())
            .collect();
        assert_eq!(decoded, b"hello over the air");
    }

    #[test]
    fn a_second_host_is_refused_rather_than_interleaved() {
        // two programs taking turns keying one transmitter is not a situation to invent
        // semantics for, and a silent hang is the worst possible answer
        let (server, worker, stop, _) = server();
        let mut first = Client::connect(&server);
        first.send("MYCALL W4ODA");
        assert_eq!(first.expect(|l| l == "OK"), "OK");

        let mut second = Client::connect(&server);
        assert_eq!(second.line(), "WRONG");

        stop.store(true, Ordering::Relaxed);
        worker.join().expect("worker");
    }

    #[test]
    fn asking_for_any_port_still_gets_a_consecutive_pair() {
        // the operating system hands out one ephemeral port at a time and has no notion of a
        // pair, so this is a search rather than a single bind
        for _ in 0..5 {
            let (command, data) = bind_pair("127.0.0.1:0").expect("a free pair");
            let command = command.local_addr().expect("addr");
            let data = data.local_addr().expect("addr");
            assert_eq!(data.port(), command.port() + 1);
        }
    }

    #[test]
    fn a_taken_data_port_is_reported_rather_than_quietly_moved() {
        // a client that found the command port and then could not find the data port would
        // connect and hang; the operator has to be told which port is in the way
        // take a known-free pair, release it, then put something on the data port only
        let (command, data) = bind_pair("127.0.0.1:0").expect("a free pair");
        let command_port = command.local_addr().expect("addr").port();
        let data_port = data.local_addr().expect("addr").port();
        drop(command);
        drop(data);
        let _squatter = TcpListener::bind(("127.0.0.1", data_port)).expect("squatter");

        let Err(error) = bind_pair(&format!("127.0.0.1:{command_port}")) else {
            panic!("binding over an occupied data port should fail");
        };
        let HostError::Bind(message) = error;
        assert!(
            message.contains("consecutive"),
            "the message does not explain the problem: {message}"
        );
        assert!(
            message.contains(&data_port.to_string()),
            "the message does not say which port is in the way: {message}"
        );
    }

    #[test]
    fn a_host_attaching_to_a_modem_without_its_sound_card_hears_missing_soundcard() {
        // VARA's word for a sound card that has gone; a gateway's host acts on it
        let (_modem, server) = Modem::start(|method, _| {
            if method == "status" {
                json!({"state": "idle", "audio_fault": "the device is not there"})
            } else {
                json!({})
            }
        });
        let mut client = Client::connect(&server);
        assert_eq!(client.expect(|l| l.starts_with("BUFFER")), "BUFFER 0");
        assert_eq!(
            client.expect(|l| l.starts_with("MISSING")),
            "MISSING SOUNDCARD"
        );
    }

    #[test]
    fn a_host_attaching_to_a_healthy_modem_hears_no_missing_soundcard() {
        let (_modem, server) = Modem::start(|method, _| {
            if method == "status" {
                json!({"state": "idle", "audio_fault": null})
            } else {
                json!({})
            }
        });
        let mut client = Client::connect(&server);
        assert_eq!(client.expect(|l| l.starts_with("BUFFER")), "BUFFER 0");
        client.send("VERSION");
        assert!(
            client
                .expect(|l| l.starts_with("VERSION") || l.starts_with("MISSING"))
                .starts_with("VERSION"),
            "a healthy modem was reported without its sound card"
        );
    }

    #[test]
    fn base64_matches_what_the_control_api_produces() {
        for length in 0..40usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 53 % 256) as u8).collect();
            let text = to_base64(&bytes);
            assert_eq!(
                from_base64(&text).as_deref(),
                Some(bytes.as_slice()),
                "{length}"
            );
        }
    }
}
