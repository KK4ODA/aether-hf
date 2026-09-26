//! The KISS TCP port, wired to the modem through its own control API (ADR-0019).
//!
//! Like the VARA-compatible host interface, this is a *client* of the control API: it sends
//! `datagram.send` and listens for `datagram` and `datagram-sent` events, so it needs no
//! privileged access to the station and cannot key the radio any way a script could not. The
//! station queues, judges (the regulatory gate) and keys; this only speaks KISS.
//!
//! * Several clients at once, up to a limit (VARA 4.8 takes several; Dire Wolf three per port).
//!   What one sends joins the one datagram queue; what the radio hears goes to every client,
//!   with the frame type it came with and its exact length (`VarAC` reads nothing else).
//! * TCP delivers a stream: frames are reassembled across reads and split out of one read by
//!   the decoder, and a malformed frame is dropped and counted, never sent.
//! * A full queue stops the client being read — TCP's own backpressure — until there is room,
//!   rather than dropping frames.
//! * "Winlink priority", as VARA has it: while a program holds the VARA-compatible command port
//!   without having said `CHAT ON`, frames from KISS clients are not sent; `VarAC` says it, and
//!   Winlink Express does not. `IGNOREKISSDCD ON` there sends them without waiting for a clear
//!   channel.
//! * A client that vanishes leaves nothing behind: its frames already queued go out, and the
//!   modem's keying is the station's, which no client ever holds.

use std::{
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::json;

use super::dialect::{KissInput, classify, guess_client, persistence_of, slot_of};
use super::framing::{self, Decoded, Decoder, command};
use crate::control::{
    methods::{from_base64, to_base64},
    protocol::{ControlHandle, Request},
};

/// The default KISS port: VARA's (HF and FM), UZ7HO's, and what Winlink Express's help and
/// `QtTermTCP` assume, so a client set up for VARA finds this one unchanged.
pub const DEFAULT_PORT: u16 = 8100;

/// The longest frame taken from a client, type byte included: the KISS specification asks for
/// at least 1024; a datagram carries less at most rungs, and says so when it cannot.
pub const MAX_FRAME: usize = 2048;

/// p-persistence when the client sets none: KISS's own default, P = 63.
pub const DEFAULT_PERSISTENCE: f64 = 0.25;

/// The slot when the client sets none: KISS's own default, SLOTTIME = 10.
pub const DEFAULT_SLOT_S: f64 = 0.1;

/// How long a reader waits for bytes before checking whether it should stop.
const POLL: Duration = Duration::from_millis(50);

/// How often a full queue is tried again.
const RETRY: Duration = Duration::from_millis(250);

/// How long a refusal of the same kind stays out of the log once said.
const QUIET: Duration = Duration::from_secs(60);

/// How the KISS port is exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KissConfig {
    /// Whether to listen at all.
    pub enabled: bool,
    /// The address to listen on; loopback unless the operator says otherwise.
    pub bind: String,
    /// The rung datagrams go at.
    pub rung: usize,
    /// Wait for a clear channel before a datagram (VARA's KISS DCD).
    pub wait_for_clear: bool,
    /// The most clients at once.
    pub max_clients: usize,
    /// Log every frame, its type, length and queueing, not only the comings and goings.
    pub trace: bool,
}

impl Default for KissConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: format!("127.0.0.1:{DEFAULT_PORT}"),
            rung: 1,
            wait_for_clear: true,
            max_clients: 4,
            trace: false,
        }
    }
}

/// What the VARA-compatible host interface knows that the KISS port and the station obey:
/// whether a host program holds the command port, whether it said `CHAT ON`, whether it said
/// `IGNOREKISSDCD ON`, and whether it said `LISTEN ON`. Shared with the host server and the
/// run loop; all false without one.
#[derive(Debug, Clone, Default)]
pub struct HostFlags {
    /// A host program holds the command port.
    pub attached: Arc<AtomicBool>,
    /// It said `CHAT ON`: KISS may transmit while it is attached.
    pub chat: Arc<AtomicBool>,
    /// It said `IGNOREKISSDCD ON`: datagrams do not wait for a clear channel.
    pub ignore_dcd: Arc<AtomicBool>,
    /// It said `LISTEN ON` (or `LISTEN CQ`, or `CHAT ON`, which includes it): calls to the
    /// station are answered. VARA's default is off, and so is a program's here until it says.
    pub listening: Arc<AtomicBool>,
}

/// One client, as the status shows it.
#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    /// A number for this connection, to disconnect it by.
    pub id: u64,
    /// Where it connected from.
    pub peer: String,
    /// A guess at what it is, from what it sent.
    pub app: String,
    /// When it connected, milliseconds since the epoch.
    pub since_ms: u64,
    /// Frames it sent that were queued.
    pub frames_in: u64,
    /// Frames sent to it.
    pub frames_out: u64,
    /// Frames of its that were dropped: malformed, refused, not sent.
    pub dropped: u64,
}

/// The port's state, for `status.kiss` and the panel.
#[derive(Debug, Clone, Default, Serialize)]
pub struct KissStatus {
    /// Listening.
    pub listening: bool,
    /// The address, once bound.
    pub address: Option<String>,
    /// Why it is not listening, when it should be.
    pub error: Option<String>,
    /// The clients connected now.
    pub clients: Vec<ClientInfo>,
    /// Why frames from clients are not being sent, when they are not.
    pub paused: Option<String>,
    /// Frames queued from clients since the port opened.
    pub frames_in: u64,
    /// Frames sent to clients since the port opened.
    pub frames_out: u64,
    /// Malformed frames dropped since the port opened.
    pub malformed: u64,
}

/// A line for the daemon's log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// A warning rather than information.
    pub warn: bool,
    /// What to say.
    pub text: String,
}

struct Client {
    info: ClientInfo,
    writer: TcpStream,
}

struct Shared {
    config: KissConfig,
    running: AtomicBool,
    clients: Mutex<Vec<Client>>,
    status: Mutex<KissStatus>,
    notes: Mutex<Vec<Note>>,
    quiet: Mutex<Vec<(String, Instant)>>,
    host: HostFlags,
    next_id: AtomicU64,
}

impl Shared {
    fn note(&self, warn: bool, text: impl Into<String>) {
        if let Ok(mut notes) = self.notes.lock() {
            if notes.len() >= 512 {
                notes.remove(0);
            }
            notes.push(Note {
                warn,
                text: text.into(),
            });
        }
    }

    /// Say it, unless the same was said within the quiet period.
    fn note_once(&self, key: &str, warn: bool, text: impl Into<String>) {
        let now = Instant::now();
        if let Ok(mut quiet) = self.quiet.lock() {
            quiet.retain(|(_, at)| now.duration_since(*at) < QUIET);
            if quiet.iter().any(|(k, _)| k == key) {
                return;
            }
            quiet.push((key.to_owned(), now));
        }
        self.note(warn, text);
    }

    fn trace(&self, text: impl FnOnce() -> String) {
        if self.config.trace {
            self.note(false, text());
        }
    }

    fn with_status(&self, change: impl FnOnce(&mut KissStatus)) {
        if let Ok(mut status) = self.status.lock() {
            change(&mut status);
        }
    }

    fn with_client(&self, id: u64, change: impl FnOnce(&mut ClientInfo)) {
        if let Ok(mut clients) = self.clients.lock()
            && let Some(client) = clients.iter_mut().find(|c| c.info.id == id)
        {
            change(&mut client.info);
        }
    }

    /// Why a client's frame may not go now, if it may not.
    fn paused(&self) -> Option<String> {
        (self.host.attached.load(Ordering::SeqCst) && !self.host.chat.load(Ordering::SeqCst)).then(
            || {
                "a host program holds the VARA-compatible command port and has not said CHAT ON \
                 (Winlink priority)"
                    .to_owned()
            },
        )
    }
}

/// A running KISS port. Stops, and disconnects its clients, when dropped.
pub struct KissServer {
    shared: Arc<Shared>,
    /// Where it listens.
    pub address: SocketAddr,
    /// The thread that holds the listening socket, waited for on the way out.
    accept: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for KissServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KissServer")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl Drop for KissServer {
    fn drop(&mut self) {
        self.shared.running.store(false, Ordering::SeqCst);
        let _ = self.disconnect(None);
        // the accept loop is blocked in accept(): a connection of our own wakes it to stop
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
        // and the port is free only once that thread has let go of the socket: a port
        // restarted in place on the same address (a changed setting) found it still held
        if let Some(accept) = self.accept.take() {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !accept.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            if accept.is_finished() {
                let _ = accept.join();
            }
        }
    }
}

/// Whether an address can be reached from other machines: anything but loopback.
#[must_use]
pub fn exposed(address: &SocketAddr) -> bool {
    !address.ip().is_loopback()
}

impl KissServer {
    /// Bind the port and start serving.
    ///
    /// # Errors
    /// The address will not parse or bind — most often because VARA or a sound-card modem
    /// already listens on 8100.
    pub fn start(
        config: &KissConfig,
        handle: ControlHandle,
        host: HostFlags,
    ) -> Result<Self, String> {
        let wanted: SocketAddr = config
            .bind
            .parse()
            .map_err(|e| format!("{} is not an address: {e}", config.bind))?;
        let listener = TcpListener::bind(wanted).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                format!(
                    "{wanted} is already in use: VARA's KISS port, a sound-card modem (UZ7HO) or \
                     another aetherd is listening there. Stop it, or choose another port"
                )
            } else {
                format!("cannot listen on {wanted}: {e}")
            }
        })?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared {
            config: config.clone(),
            running: AtomicBool::new(true),
            clients: Mutex::new(Vec::new()),
            status: Mutex::new(KissStatus {
                listening: true,
                address: Some(address.to_string()),
                ..KissStatus::default()
            }),
            notes: Mutex::new(Vec::new()),
            quiet: Mutex::new(Vec::new()),
            host,
            next_id: AtomicU64::new(1),
        });
        shared.note(false, format!("listening on {address}"));
        if exposed(&address) {
            shared.note(
                true,
                format!(
                    "{address} can be reached from other machines: anyone who can connect to it \
                     can make this station transmit"
                ),
            );
        }
        spawn_hub(Arc::clone(&shared), &handle)?;
        let accept = spawn_accept(listener, Arc::clone(&shared), handle)?;
        Ok(Self {
            shared,
            address,
            accept: Some(accept),
        })
    }

    /// The port's state now.
    #[must_use]
    pub fn status(&self) -> KissStatus {
        let mut status = self
            .shared
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        status.clients = self
            .shared
            .clients
            .lock()
            .map(|clients| clients.iter().map(|c| c.info.clone()).collect())
            .unwrap_or_default();
        status.paused = self.shared.paused();
        status
    }

    /// Lines for the daemon's log since the last call.
    #[must_use]
    pub fn take_notes(&self) -> Vec<Note> {
        self.shared
            .notes
            .lock()
            .map(|mut notes| std::mem::take(&mut *notes))
            .unwrap_or_default()
    }

    /// Close one client's connection, or every client's. Returns how many were closed.
    #[must_use]
    pub fn disconnect(&self, id: Option<u64>) -> usize {
        let Ok(clients) = self.shared.clients.lock() else {
            return 0;
        };
        let mut closed = 0;
        for client in clients
            .iter()
            .filter(|c| id.is_none_or(|id| c.info.id == id))
        {
            let _ = client.writer.shutdown(Shutdown::Both);
            closed += 1;
        }
        closed
    }

    /// The configuration it runs with.
    #[must_use]
    pub fn config(&self) -> &KissConfig {
        &self.shared.config
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn request(method: &str, params: serde_json::Value) -> Request {
    Request {
        id: None,
        method: method.to_owned(),
        params,
    }
}

fn spawn_accept(
    listener: TcpListener,
    shared: Arc<Shared>,
    handle: ControlHandle,
) -> Result<std::thread::JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("aetherd-kiss-accept".to_owned())
        .spawn(move || {
            for stream in listener.incoming() {
                if !shared.running.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                admit(stream, &shared, &handle);
            }
        })
        .map_err(|e| format!("cannot start the KISS accept thread: {e}"))
}

/// Take a new connection, or turn it away when the port is full.
fn admit(stream: TcpStream, shared: &Arc<Shared>, handle: &ControlHandle) {
    let peer = stream
        .peer_addr()
        .map_or_else(|_| "unknown".to_owned(), |a| a.to_string());
    let full = shared
        .clients
        .lock()
        .map_or(true, |c| c.len() >= shared.config.max_clients);
    if full {
        shared.note(
            true,
            format!(
                "refused a client from {peer}: {} are connected already",
                shared.config.max_clients
            ),
        );
        let _ = stream.shutdown(Shutdown::Both);
        return;
    }
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let _ = stream.set_nodelay(true);
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut clients) = shared.clients.lock() {
        clients.push(Client {
            info: ClientInfo {
                id,
                peer: peer.clone(),
                app: "a KISS client".to_owned(),
                since_ms: now_ms(),
                frames_in: 0,
                frames_out: 0,
                dropped: 0,
            },
            writer,
        });
    }
    shared.note(false, format!("client {id} connected from {peer}"));
    let for_thread = Arc::clone(shared);
    let handle = handle.clone();
    let spawned = std::thread::Builder::new()
        .name("aetherd-kiss-client".to_owned())
        .spawn(move || {
            serve_client(&stream, id, &for_thread, &handle);
            forget(&for_thread, id);
        });
    if spawned.is_err() {
        forget(shared, id);
    }
}

/// A client gone: off the list, and said so.
fn forget(shared: &Shared, id: u64) {
    if let Ok(mut clients) = shared.clients.lock() {
        clients.retain(|c| c.info.id != id);
    }
    shared.note(false, format!("client {id} disconnected"));
}

/// The client's channel access, as its P and SLOTTIME set it.
struct Access {
    persistence: f64,
    slot_s: f64,
}

/// One client: frames in, until it goes or the port closes.
fn serve_client(stream: &TcpStream, id: u64, shared: &Shared, handle: &ControlHandle) {
    if stream.set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let mut reader = stream;
    let mut decoder = Decoder::new(MAX_FRAME);
    let mut access = Access {
        persistence: DEFAULT_PERSISTENCE,
        slot_s: DEFAULT_SLOT_S,
    };
    let mut buffer = [0u8; 4096];
    let mut acks: u64 = 0;
    while shared.running.load(Ordering::SeqCst) {
        let count = match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => count,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => return,
        };
        for decoded in decoder.push(&buffer[..count]) {
            match decoded {
                Decoded::Malformed(why) => {
                    shared.with_status(|s| s.malformed += 1);
                    shared.with_client(id, |c| c.dropped += 1);
                    shared.note_once(
                        &format!("malformed {id}"),
                        true,
                        format!("client {id} sent a malformed frame ({why}); it was dropped"),
                    );
                }
                Decoded::Frame(frame) => {
                    shared.trace(|| {
                        format!(
                            "client {id}: frame, command 0x{:02X}, {} bytes",
                            frame.command,
                            frame.data.len()
                        )
                    });
                    match classify(&frame) {
                        KissInput::Data {
                            frame_type,
                            frame,
                            ack,
                        } => {
                            let reference = ack.map(|ack| {
                                acks += 1;
                                format!("kiss:{id}:{:02x}{:02x}:{acks}", ack[0], ack[1])
                            });
                            if !submit(
                                shared,
                                handle,
                                id,
                                frame_type,
                                &frame,
                                reference.as_deref(),
                                &access,
                            ) {
                                return;
                            }
                        }
                        KissInput::Persistence(value) => {
                            access.persistence = persistence_of(value);
                            shared.trace(|| format!("client {id}: P {value}"));
                        }
                        KissInput::SlotTime(value) => {
                            access.slot_s = slot_of(value);
                            shared.trace(|| format!("client {id}: SLOTTIME {value}"));
                        }
                        KissInput::Ignored(what) => {
                            shared.trace(|| format!("client {id}: {what} (accepted, not used)"));
                        }
                        KissInput::Refused(why) => {
                            shared.with_client(id, |c| c.dropped += 1);
                            shared.note_once(
                                &format!("refused {id} {why}"),
                                true,
                                format!("client {id}: {why}; the frame was dropped"),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Hand a client's frame to the modem. A full queue is waited out — the client is not read
/// meanwhile, which is TCP's own backpressure; anything else is said and the frame dropped.
/// Returns false when the port is closing.
fn submit(
    shared: &Shared,
    handle: &ControlHandle,
    id: u64,
    frame_type: u8,
    frame: &[u8],
    reference: Option<&str>,
    access: &Access,
) -> bool {
    shared.with_client(id, |c| {
        if c.frames_in == 0 && c.dropped == 0 {
            guess_client(frame_type, frame, reference.is_some()).clone_into(&mut c.app);
        }
    });
    if let Some(why) = shared.paused() {
        shared.with_client(id, |c| c.dropped += 1);
        shared.note_once(
            "paused",
            true,
            format!("frames from KISS clients are not sent: {why}"),
        );
        return true;
    }
    let wait_for_clear =
        shared.config.wait_for_clear && !shared.host.ignore_dcd.load(Ordering::SeqCst);
    let params = json!({
        "data": to_base64(frame),
        "frame_type": frame_type,
        "ref": reference,
        "rung": shared.config.rung,
        "wait_for_clear": wait_for_clear,
        "persistence": access.persistence,
        "slot_s": access.slot_s,
    });
    loop {
        if !shared.running.load(Ordering::SeqCst) {
            return false;
        }
        match handle.call(request("datagram.send", params.clone())) {
            Ok(response) if response.ok => {
                shared.with_status(|s| s.frames_in += 1);
                shared.with_client(id, |c| c.frames_in += 1);
                shared.trace(|| {
                    format!(
                        "client {id}: {} bytes, type {frame_type}, queued ({} fragments, {:.0} s on the air)",
                        frame.len(),
                        response.result.as_ref().map_or(0, |r| r["fragments"].as_u64().unwrap_or(0)),
                        response.result.as_ref().map_or(0.0, |r| r["air_s"].as_f64().unwrap_or(0.0)),
                    )
                });
                return true;
            }
            Ok(response)
                if response
                    .error
                    .as_ref()
                    .is_some_and(|e| e.code == "queue_full") =>
            {
                std::thread::sleep(RETRY);
            }
            Ok(response) => {
                shared.with_client(id, |c| c.dropped += 1);
                let why = response
                    .error
                    .map_or_else(|| "refused".to_owned(), |e| e.message);
                shared.note_once(
                    &format!("send {id} {why}"),
                    true,
                    format!("client {id}: {why}"),
                );
                return true;
            }
            Err(error) => {
                shared.note_once(
                    "modem",
                    true,
                    format!("the modem did not take a datagram: {}", error.message),
                );
                std::thread::sleep(RETRY);
            }
        }
    }
}

/// What the modem says, to the clients: frames heard go to every one; an ACKMODE frame's
/// identifier goes back to the client that sent it once the frame has left.
fn spawn_hub(shared: Arc<Shared>, handle: &ControlHandle) -> Result<(), String> {
    let events = handle.subscribe();
    std::thread::Builder::new()
        .name("aetherd-kiss-hub".to_owned())
        .spawn(move || {
            while shared.running.load(Ordering::SeqCst) {
                let Ok(event) = events.recv_timeout(POLL) else {
                    continue;
                };
                match event.event.as_str() {
                    "datagram" => {
                        let frame_type = event.data["frame_type"]
                            .as_u64()
                            .and_then(|t| u8::try_from(t).ok())
                            .unwrap_or(0);
                        let Some(frame) = event.data["data"].as_str().and_then(from_base64)
                        else {
                            continue;
                        };
                        let wire = framing::encode(0, frame_type, &frame);
                        let sent = broadcast(&shared, &wire);
                        shared.trace(|| {
                            format!(
                                "a {}-byte frame of type {frame_type} from {} sent to {sent} client(s)",
                                frame.len(),
                                event.data["source"].as_str().unwrap_or("?")
                            )
                        });
                    }
                    "datagram-sent" => {
                        if let Some(reference) = event.data["ref"].as_str()
                            && event.data["sent"].as_bool() == Some(true)
                        {
                            acknowledge(&shared, reference);
                        }
                    }
                    _ => {}
                }
            }
        })
        .map(|_| ())
        .map_err(|e| format!("cannot start the KISS event thread: {e}"))
}

/// Send one wire frame to every client; a client whose socket fails is disconnected.
fn broadcast(shared: &Shared, wire: &[u8]) -> usize {
    let Ok(mut clients) = shared.clients.lock() else {
        return 0;
    };
    let mut sent = 0;
    for client in clients.iter_mut() {
        if (&client.writer).write_all(wire).is_ok() {
            client.info.frames_out += 1;
            sent += 1;
        } else {
            let _ = client.writer.shutdown(Shutdown::Both);
        }
    }
    drop(clients);
    shared.with_status(|s| s.frames_out += sent as u64);
    sent
}

/// Tell an ACKMODE client its frame went out: `kiss:<client>:<id>:<n>` names both.
fn acknowledge(shared: &Shared, reference: &str) {
    let mut parts = reference.split(':');
    if parts.next() != Some("kiss") {
        return;
    }
    let (Some(client), Some(ack)) = (
        parts.next().and_then(|c| c.parse::<u64>().ok()),
        parts.next().filter(|a| a.len() == 4),
    ) else {
        return;
    };
    let (Ok(hi), Ok(lo)) = (
        u8::from_str_radix(&ack[..2], 16),
        u8::from_str_radix(&ack[2..], 16),
    ) else {
        return;
    };
    let wire = framing::encode(0, command::ACK_MODE, &[hi, lo]);
    if let Ok(clients) = shared.clients.lock()
        && let Some(target) = clients.iter().find(|c| c.info.id == client)
    {
        let _ = (&target.writer).write_all(&wire);
    }
    shared.trace(|| format!("client {client}: ACKMODE {ack} acknowledged"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{
        Event, channel,
        protocol::{ApiError, ControlChannel, Response},
    };

    /// What the stub modem was sent, in order: the params of every `datagram.send` it took.
    type Taken = Arc<Mutex<Vec<serde_json::Value>>>;

    /// A stub modem: takes datagrams into a queue of `limit`, lets one go every `drain`, and
    /// publishes whatever the test hands it.
    struct Stub {
        taken: Taken,
        publish: std::sync::mpsc::Sender<Event>,
        stop: Arc<AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn stub(control: ControlChannel, limit: usize, drain: Duration) -> Stub {
        let taken: Taken = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (publish, published) = std::sync::mpsc::channel::<Event>();
        let (log, flag) = (Arc::clone(&taken), Arc::clone(&stop));
        let worker = std::thread::spawn(move || {
            let mut queued = 0usize;
            let mut last_drain = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                if last_drain.elapsed() >= drain {
                    queued = queued.saturating_sub(1);
                    last_drain = Instant::now();
                }
                for command in control.drain() {
                    let response = if command.request.method != "datagram.send" {
                        Response::ok(command.request.id.clone(), json!({}))
                    } else if queued >= limit {
                        Response::failed(
                            command.request.id.clone(),
                            ApiError::new("queue_full", "full", true),
                        )
                    } else {
                        queued += 1;
                        if let Ok(mut log) = log.lock() {
                            log.push(command.request.params.clone());
                        }
                        Response::ok(
                            command.request.id.clone(),
                            json!({"accepted": true, "fragments": 3, "air_s": 16.0}),
                        )
                    };
                    let _ = command.reply.send(response);
                }
                for event in published.try_iter() {
                    control.publish(&event);
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        Stub {
            taken,
            publish,
            stop,
            worker: Some(worker),
        }
    }

    fn server(
        limit: usize,
        drain: Duration,
        config: &KissConfig,
        host: HostFlags,
    ) -> (KissServer, Stub) {
        let (handle, control) = channel();
        let stub = stub(control, limit, drain);
        let server = KissServer::start(config, handle, host).expect("start");
        (server, stub)
    }

    fn loopback() -> KissConfig {
        KissConfig {
            enabled: true,
            bind: "127.0.0.1:0".to_owned(),
            ..KissConfig::default()
        }
    }

    fn connect(server: &KissServer) -> TcpStream {
        let stream = TcpStream::connect(server.address).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("timeout");
        // until the port has admitted it, a frame could beat the client onto the list
        wait_for(
            || !server.status().clients.is_empty(),
            "the client is admitted",
        );
        stream
    }

    fn wait_for(mut done: impl FnMut() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Every KISS frame the client receives within `wait`.
    fn read_frames(stream: &mut TcpStream, wait: Duration) -> Vec<framing::KissFrame> {
        let mut decoder = Decoder::new(MAX_FRAME);
        let mut frames = Vec::new();
        let deadline = Instant::now() + wait;
        let mut buffer = [0u8; 4096];
        while Instant::now() < deadline {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    for decoded in decoder.push(&buffer[..n]) {
                        if let Decoded::Frame(frame) = decoded {
                            frames.push(frame);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        frames
    }

    fn ax25(n: usize) -> Vec<u8> {
        // "APRS" and "N0CALL", shifted, then control and PID, then text with every special byte
        let mut frame: Vec<u8> = b"APRS  ".iter().map(|c| c << 1).collect();
        frame.push(0xE0);
        frame.extend(b"N0CALL".iter().map(|c| c << 1));
        frame.push(0x61);
        frame.extend([0x03, 0xF0]);
        frame.extend((0..n).map(|i| [framing::FEND, framing::FESC, b'a', framing::TFEND][i % 4]));
        frame
    }

    fn taken(stub: &Stub) -> Vec<serde_json::Value> {
        stub.taken.lock().map(|t| t.clone()).unwrap_or_default()
    }

    #[test]
    fn a_frame_reaches_the_modem_however_tcp_cuts_it_up() {
        let (server, stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let mut client = connect(&server);
        let aprs = ax25(40);
        let varac = ax25(25);
        let mut wire = framing::encode(0, 0, &aprs);
        wire.extend(framing::encode(0, 1, &varac)); // VarAC's type 1
        wire.extend(framing::encode(0, 2, b"unformatted")); // VARA's type 2
        wire.extend(framing::encode(0, 1, &[30])); // a real TXDELAY: not data
        wire.extend(framing::encode(0, 2, &[127])); // a real P: not data
        // one byte at a time: every escape split from what it escapes
        for byte in &wire {
            client.write_all(&[*byte]).expect("write");
        }
        wait_for(|| taken(&stub).len() == 3, "three datagrams");
        let seen = taken(&stub);
        let data = |i: usize| from_base64(seen[i]["data"].as_str().expect("data")).expect("b64");
        assert_eq!((data(0), seen[0]["frame_type"].as_u64()), (aprs, Some(0)));
        assert_eq!((data(1), seen[1]["frame_type"].as_u64()), (varac, Some(1)));
        assert_eq!(
            (data(2), seen[2]["frame_type"].as_u64()),
            (b"unformatted".to_vec(), Some(2))
        );
        // the P it sent is its channel access from then on
        client
            .write_all(&framing::encode(0, 0, &ax25(5)))
            .expect("write");
        wait_for(|| taken(&stub).len() == 4, "a fourth");
        let p = taken(&stub)[3]["persistence"].as_f64().expect("p");
        assert!((p - 0.5).abs() < 1e-9, "P 127 is p = 0.5, got {p}");
        assert_eq!(taken(&stub)[3]["rung"].as_u64(), Some(1));
        assert_eq!(taken(&stub)[3]["wait_for_clear"].as_bool(), Some(true));
    }

    #[test]
    fn what_the_radio_hears_goes_to_every_client_as_it_came() {
        let (server, stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let mut one = connect(&server);
        let mut two = TcpStream::connect(server.address).expect("second");
        two.set_read_timeout(Some(Duration::from_millis(100)))
            .expect("timeout");
        wait_for(|| server.status().clients.len() == 2, "two clients");
        let frame = ax25(30);
        stub.publish
            .send(Event::new(
                "datagram",
                json!({"source": "KK4XYZ", "frame_type": 1, "data": to_base64(&frame)}),
            ))
            .expect("publish");
        for client in [&mut one, &mut two] {
            let frames = read_frames(client, Duration::from_millis(600));
            assert_eq!(frames.len(), 1);
            assert_eq!(
                (frames[0].port, frames[0].command),
                (0, 1),
                "type 1 stays type 1"
            );
            assert_eq!(frames[0].data, frame, "exact length and content");
        }
        assert_eq!(server.status().frames_out, 2);
    }

    #[test]
    fn an_ackmode_frame_is_acknowledged_once_it_has_gone() {
        let (server, stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let mut client = connect(&server);
        let mut body = vec![0xAB, 0xCD];
        body.extend(ax25(10));
        client
            .write_all(&framing::encode(0, command::ACK_MODE, &body))
            .expect("write");
        wait_for(|| taken(&stub).len() == 1, "the frame");
        let reference = taken(&stub)[0]["ref"]
            .as_str()
            .expect("a reference")
            .to_owned();
        assert_eq!(
            from_base64(taken(&stub)[0]["data"].as_str().expect("data")),
            Some(ax25(10)),
            "the identifier is not sent on the air"
        );
        // nothing until the modem says it went
        assert!(read_frames(&mut client, Duration::from_millis(200)).is_empty());
        stub.publish
            .send(Event::new(
                "datagram-sent",
                json!({"ref": reference, "sent": true}),
            ))
            .expect("publish");
        let frames = read_frames(&mut client, Duration::from_millis(600));
        assert_eq!(frames.len(), 1);
        assert_eq!(
            (frames[0].command, frames[0].data.clone()),
            (0x0C, vec![0xAB, 0xCD])
        );
    }

    /// Every KISS frame the client receives until it has `count` of them or `wait` has passed,
    /// through one decoder — a frame cut between two reads is still one frame.
    fn read_count(stream: &mut TcpStream, count: usize, wait: Duration) -> Vec<framing::KissFrame> {
        let mut decoder = Decoder::new(MAX_FRAME);
        let mut frames = Vec::new();
        let deadline = Instant::now() + wait;
        let mut buffer = [0u8; 4096];
        while frames.len() < count && Instant::now() < deadline {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    for decoded in decoder.push(&buffer[..n]) {
                        if let Decoded::Frame(frame) = decoded {
                            frames.push(frame);
                        }
                    }
                }
                Err(_) => {}
            }
        }
        frames
    }

    #[test]
    fn four_clients_writing_ragged_pieces_at_once_lose_nothing_and_keep_their_order() {
        // the stress test: four programs each write a hundred frames full of bytes that must be
        // escaped, in pieces of 1–64 bytes cut anywhere, all at once, while the radio hears
        // twenty frames that go to all four. Every frame reaches the modem once, each client's
        // in its own order; every client gets the twenty, whole and in order; nothing is
        // counted malformed or dropped.
        const CLIENTS: usize = 4;
        const FRAMES: u8 = 100;
        let (server, stub) = server(
            10_000,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let streams: Vec<TcpStream> = (0..CLIENTS)
            .map(|_| {
                let stream = TcpStream::connect(server.address).expect("connect");
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .expect("timeout");
                stream
            })
            .collect();
        wait_for(|| server.status().clients.len() == CLIENTS, "four clients");
        let heard: Vec<Vec<u8>> = (0..20u8)
            .map(|n| {
                let mut frame = ax25(12);
                frame.push(n);
                frame
            })
            .collect();
        let workers: Vec<_> = streams
            .into_iter()
            .enumerate()
            .map(|(id, mut stream)| {
                let expected = heard.len();
                std::thread::spawn(move || {
                    let tag = u8::try_from(id).expect("a few clients");
                    let mut wire = Vec::new();
                    for n in 0..FRAMES {
                        let mut frame = ax25(usize::from(n % 37));
                        frame.extend([tag, n]);
                        wire.extend(framing::encode(0, 0, &frame));
                    }
                    let mut seed = 0x9E37_79B9_u32 ^ u32::from(tag + 1);
                    let mut at = 0;
                    while at < wire.len() {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        let end = (at + 1 + (seed % 64) as usize).min(wire.len());
                        stream.write_all(&wire[at..end]).expect("write");
                        at = end;
                    }
                    read_count(&mut stream, expected, Duration::from_secs(10))
                })
            })
            .collect();
        for frame in &heard {
            stub.publish
                .send(Event::new(
                    "datagram",
                    json!({"source": "KK4XYZ", "frame_type": 0, "data": to_base64(frame)}),
                ))
                .expect("publish");
        }
        for worker in workers {
            let received = worker.join().expect("a client");
            let data: Vec<Vec<u8>> = received.into_iter().map(|f| f.data).collect();
            assert_eq!(
                data, heard,
                "every client hears every frame, whole and in order"
            );
        }
        let total = CLIENTS * usize::from(FRAMES);
        wait_for(|| taken(&stub).len() == total, "four hundred frames");
        let mut order = vec![Vec::new(); CLIENTS];
        for params in taken(&stub) {
            let frame = from_base64(params["data"].as_str().expect("data")).expect("b64");
            let (tag, n) = (frame[frame.len() - 2], frame[frame.len() - 1]);
            assert_eq!(
                &frame[..frame.len() - 2],
                ax25(usize::from(n % 37)).as_slice(),
                "frame {n} of client {tag} arrived as it was written"
            );
            order[usize::from(tag)].push(n);
        }
        for (tag, seen) in order.iter().enumerate() {
            assert_eq!(
                seen,
                &(0..FRAMES).collect::<Vec<u8>>(),
                "client {tag}'s frames, once each, in order"
            );
        }
        let status = server.status();
        assert_eq!(status.malformed, 0);
        assert_eq!(status.frames_in, total as u64);
        assert!(status.clients.iter().all(|c| c.dropped == 0));
    }

    #[test]
    fn a_full_queue_holds_the_client_back_without_losing_or_reordering() {
        // four places, one freed every 20 ms: forty frames sent at once all arrive, in order
        let (server, stub) = server(
            4,
            Duration::from_millis(20),
            &loopback(),
            HostFlags::default(),
        );
        let mut client = connect(&server);
        let mut wire = Vec::new();
        for n in 0..40u8 {
            let mut frame = ax25(4);
            frame.push(n);
            wire.extend(framing::encode(0, 0, &frame));
        }
        client.write_all(&wire).expect("write");
        wait_for(|| taken(&stub).len() == 40, "forty frames");
        let order: Vec<u8> = taken(&stub)
            .iter()
            .map(|p| {
                *from_base64(p["data"].as_str().expect("data"))
                    .expect("b64")
                    .last()
                    .expect("n")
            })
            .collect();
        assert_eq!(order, (0..40).collect::<Vec<u8>>());
        assert_eq!(server.status().clients[0].dropped, 0);
    }

    #[test]
    fn malformed_frames_and_other_ports_are_dropped_and_the_client_stays() {
        let (server, stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let mut client = connect(&server);
        let mut wire = vec![
            framing::FEND,
            0x00,
            b'x',
            framing::FESC,
            b'Q',
            framing::FEND,
        ];
        wire.extend(framing::encode(3, 0, &ax25(3))); // port 3: this modem has one
        wire.extend(vec![0x33; 5000]); // noise outside any frame, and far too long
        wire.extend(framing::encode(0, 0, &ax25(8)));
        client.write_all(&wire).expect("write");
        wait_for(|| taken(&stub).len() == 1, "the good frame");
        let status = server.status();
        // the bad escape and the frame too long to take; the other port refused besides
        assert_eq!(status.malformed, 2);
        assert_eq!(status.clients[0].dropped, 3);
        assert_eq!(
            from_base64(taken(&stub)[0]["data"].as_str().expect("data")),
            Some(ax25(8))
        );
    }

    #[test]
    fn a_host_without_chat_on_holds_kiss_frames_as_vara_does() {
        let host = HostFlags::default();
        let (server, stub) = server(16, Duration::from_secs(60), &loopback(), host.clone());
        let mut client = connect(&server);
        host.attached.store(true, Ordering::SeqCst);
        client
            .write_all(&framing::encode(0, 0, &ax25(4)))
            .expect("write");
        wait_for(|| server.status().clients[0].dropped == 1, "held back");
        assert!(taken(&stub).is_empty(), "Winlink priority");
        assert!(server.status().paused.is_some());
        // CHAT ON, and IGNOREKISSDCD ON: sent, without waiting for a clear channel
        host.chat.store(true, Ordering::SeqCst);
        host.ignore_dcd.store(true, Ordering::SeqCst);
        client
            .write_all(&framing::encode(0, 0, &ax25(4)))
            .expect("write");
        wait_for(|| taken(&stub).len() == 1, "sent");
        assert_eq!(taken(&stub)[0]["wait_for_clear"].as_bool(), Some(false));
        assert!(server.status().paused.is_none());
    }

    #[test]
    fn the_port_turns_away_clients_past_its_limit_and_lets_them_go_on_request() {
        let config = KissConfig {
            max_clients: 1,
            ..loopback()
        };
        let (server, _stub) = server(16, Duration::from_secs(60), &config, HostFlags::default());
        let mut first = connect(&server);
        let mut second = TcpStream::connect(server.address).expect("second");
        second
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut byte = [0u8; 1];
        assert!(matches!(second.read(&mut byte), Ok(0)), "turned away");
        assert_eq!(server.status().clients.len(), 1);
        assert_eq!(server.disconnect(None), 1);
        first
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        assert!(
            matches!(first.read(&mut byte), Ok(0) | Err(_)),
            "disconnected"
        );
        wait_for(|| server.status().clients.is_empty(), "forgotten");
    }

    #[test]
    fn a_client_that_vanishes_mid_frame_leaves_the_port_usable() {
        let (server, stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        {
            let mut crashed = connect(&server);
            let wire = framing::encode(0, 0, &ax25(20));
            crashed
                .write_all(&wire[..wire.len() / 2])
                .expect("half a frame");
        }
        wait_for(
            || server.status().clients.is_empty(),
            "the crashed client forgotten",
        );
        let mut client = connect(&server);
        client
            .write_all(&framing::encode(0, 0, &ax25(6)))
            .expect("write");
        wait_for(|| taken(&stub).len() == 1, "the next client's frame");
        assert_eq!(
            from_base64(taken(&stub)[0]["data"].as_str().expect("data")),
            Some(ax25(6)),
            "nothing of the half frame"
        );
    }

    #[test]
    fn closing_the_port_disconnects_its_clients_and_frees_it() {
        let (server, _stub) = server(
            16,
            Duration::from_secs(60),
            &loopback(),
            HostFlags::default(),
        );
        let address = server.address;
        let mut client = connect(&server);
        drop(server);
        client
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("timeout");
        let mut byte = [0u8; 1];
        assert!(matches!(client.read(&mut byte), Ok(0) | Err(_)));
        let again = KissConfig {
            bind: address.to_string(),
            ..loopback()
        };
        let (handle, control) = channel();
        let _stub = stub(control, 16, Duration::from_secs(60));
        let reopened = KissServer::start(&again, handle, HostFlags::default());
        assert!(
            reopened.is_ok(),
            "the same port binds again: {:?}",
            reopened.err()
        );
    }

    #[test]
    fn an_exposed_address_is_said_to_be_exposed() {
        assert!(!exposed(&"127.0.0.1:8100".parse().expect("addr")));
        assert!(exposed(&"0.0.0.0:8100".parse().expect("addr")));
        assert!(exposed(&"192.168.1.10:8100".parse().expect("addr")));
    }
}
