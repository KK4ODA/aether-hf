//! The control socket: WebSocket for a session, `POST /v1/<method>` for a one-shot.
//!
//! One blocking listener, one thread per connection. There is no async runtime here on
//! purpose: a control interface handles a handful of connections that spend their lives
//! waiting, and the modem it talks to is single-threaded because its clock is the audio it
//! has heard. A runtime would add a large dependency and a second concurrency model without
//! making anything here faster.
//!
//! # Binding and authentication
//!
//! The default bind is loopback, where anything that can open the socket is already running
//! as the operator and a token would protect nothing. Binding anywhere else is a different
//! proposition — that is a radio transmitter reachable from the network — so it **requires**
//! a token, and a configuration that asks for a non-loopback bind without one is refused at
//! startup rather than started open.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde_json::json;

use crate::control::protocol::{ApiError, ControlHandle, Request, Response};

/// How the control interface is exposed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlConfig {
    /// Address to listen on. Loopback unless the operator says otherwise.
    pub bind: String,
    /// Bearer token. Required for any non-loopback bind.
    pub token: Option<String>,
    /// Directory of static files to serve at `/`, for the station panel.
    ///
    /// A gateway is a headless machine, and the only practical way to look at one is a
    /// browser over an SSH tunnel. Serving the panel from the daemon is what makes that
    /// work without a second program to install (ADR-0001 §3, ADR-0005).
    pub ui_dir: Option<std::path::PathBuf>,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8515".to_owned(),
            token: None,
            ui_dir: None,
        }
    }
}

/// Why the control interface would not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerError {
    /// The address could not be parsed or bound.
    Bind(String),
    /// A non-loopback bind was asked for with no token.
    Unprotected(String),
}

impl core::fmt::Display for ServerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bind(detail) => write!(f, "control interface: {detail}"),
            Self::Unprotected(address) => write!(
                f,
                "refusing to listen on {address} without a token: that address is reachable \
                 from the network, and this interface can key a transmitter. Set a token, or \
                 bind to 127.0.0.1."
            ),
        }
    }
}

impl core::error::Error for ServerError {}

/// Whether an address is one only this machine can reach.
#[must_use]
pub fn is_loopback(address: &str) -> bool {
    address
        .parse::<std::net::SocketAddr>()
        .is_ok_and(|socket| socket.ip().is_loopback())
}

/// A running control interface. Stops when dropped.
pub struct ControlServer {
    /// What it is listening on, after any port-zero assignment.
    pub address: std::net::SocketAddr,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for ControlServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlServer")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // wake the accept loop so it notices; a connection to ourselves is the simplest way
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(200));
    }
}

impl ControlServer {
    /// Bind and start serving.
    ///
    /// # Errors
    /// If the address cannot be bound, or a non-loopback address was asked for without a
    /// token.
    pub fn start(config: &ControlConfig, handle: ControlHandle) -> Result<Self, ServerError> {
        if !is_loopback(&config.bind) && config.token.is_none() {
            return Err(ServerError::Unprotected(config.bind.clone()));
        }
        let listener = TcpListener::bind(&config.bind).map_err(|e| {
            ServerError::Bind(if e.kind() == std::io::ErrorKind::AddrInUse {
                format!(
                    "{} is already in use: another aetherd, or another program, is listening \
                     there. Stop it, or change [control] bind",
                    config.bind
                )
            } else {
                format!("cannot listen on {}: {e}", config.bind)
            })
        })?;
        let address = listener
            .local_addr()
            .map_err(|e| ServerError::Bind(format!("{e}")))?;

        let running = Arc::new(AtomicBool::new(true));
        let loop_running = Arc::clone(&running);
        let token = config.token.clone();
        let ui_dir = config
            .ui_dir
            .as_ref()
            .and_then(|dir| dir.canonicalize().ok());
        std::thread::Builder::new()
            .name("aetherd-control".to_owned())
            .spawn(move || {
                for stream in listener.incoming() {
                    if !loop_running.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let handle = handle.clone();
                    let token = token.clone();
                    let ui_dir = ui_dir.clone();
                    let running = Arc::clone(&loop_running);
                    let _ = std::thread::Builder::new()
                        .name("aetherd-control-conn".to_owned())
                        .spawn(move || {
                            serve(
                                &stream,
                                &handle,
                                token.as_deref(),
                                ui_dir.as_deref(),
                                &running,
                            );
                        });
                }
            })
            .map_err(|e| ServerError::Bind(format!("cannot start the listener thread: {e}")))?;

        Ok(Self { address, running })
    }
}

/// Read the request line and headers, then decide what kind of connection this is.
fn serve(
    stream: &TcpStream,
    handle: &ControlHandle,
    token: Option<&str>,
    ui_dir: Option<&std::path::Path>,
    running: &AtomicBool,
) {
    let Ok(peer_loopback) = stream.peer_addr().map(|a| a.ip().is_loopback()) else {
        return;
    };
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut line = String::new();
        // a closed connection and a read error are the same thing here: there is no request
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let authorised = peer_loopback
        || token.is_none_or(|expected| {
            header("authorization")
                .is_some_and(|given| given.strip_prefix("Bearer ").map(str::trim) == Some(expected))
        });

    let upgrading = header("upgrade").is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    if upgrading {
        if !authorised {
            // a WebSocket client that cannot authenticate never gets a socket to argue on
            let _ = write_http(stream, 401, "application/json", &unauthorised_body());
            return;
        }
        let Some(key) = header("sec-websocket-key") else {
            let _ = write_http(
                stream,
                400,
                "text/plain",
                "a WebSocket upgrade needs a Sec-WebSocket-Key header",
            );
            return;
        };
        serve_websocket(stream, key, handle, running);
        return;
    }
    serve_rest(
        stream,
        &request_line,
        header("content-length"),
        &mut reader,
        handle,
        authorised,
        ui_dir,
    );
}

fn unauthorised_body() -> String {
    serde_json::to_string(&Response::failed(
        None,
        ApiError::new(
            "unauthorised",
            "This connection needs a bearer token. It is in the daemon's configuration.",
            false,
        ),
    ))
    .unwrap_or_else(|_| "{\"ok\":false}".to_owned())
}

fn write_http(mut stream: &TcpStream, status: u16, content_type: &str, body: &str) -> bool {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).is_ok()
}

/// `POST /v1/<method>` with the same body a WebSocket message would carry.
fn serve_rest(
    stream: &TcpStream,
    request_line: &str,
    content_length: Option<&str>,
    reader: &mut BufReader<TcpStream>,
    handle: &ControlHandle,
    authorised: bool,
    ui_dir: Option<&std::path::Path>,
) {
    if !authorised {
        write_http(stream, 401, "application/json", &unauthorised_body());
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    if method == "GET"
        && let Some(dir) = ui_dir
        && serve_file(stream, dir, path)
    {
        return;
    }

    if method != "POST" {
        let body = serde_json::to_string(&Response::failed(
            None,
            ApiError::new(
                "not_found",
                "The control interface answers POST /v1/<method>, or a WebSocket upgrade on /v1.",
                false,
            ),
        ))
        .unwrap_or_default();
        write_http(stream, 404, "application/json", &body);
        return;
    }

    let name = path.strip_prefix("/v1/").unwrap_or("").to_owned();
    let length: usize = content_length.and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; length.min(1 << 20)];
    if !body.is_empty() && reader.read_exact(&mut body).is_err() {
        write_http(stream, 400, "application/json", "{\"ok\":false}");
        return;
    }
    let params: serde_json::Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body).unwrap_or(json!({}))
    };

    let request = Request {
        id: None,
        method: name,
        params,
        token: None,
    };
    // written before the request stops counting as unwritten: a daemon this request stops —
    // `shutdown` — waits for the write before it exits (`ControlChannel::settle`)
    handle.call_and_deliver(request, |response| {
        let text = serde_json::to_string(&response).unwrap_or_default();
        write_http(
            stream,
            if response.ok { 200 } else { 400 },
            "application/json",
            &text,
        )
    });
}

/// Serve one file from the panel directory, if the path names one.
///
/// The path is resolved and then checked to be **inside** the directory it was resolved
/// against. Without that, `GET /../../etc/shadow` would be served by a process that an
/// operator has deliberately pointed at a network interface.
fn serve_file(stream: &TcpStream, dir: &std::path::Path, path: &str) -> bool {
    let requested = path.split('?').next().unwrap_or("/");
    let relative = match requested {
        "/" | "" => "index.html",
        other => other.trim_start_matches('/'),
    };
    if relative.is_empty() {
        return false;
    }
    let Ok(full) = dir.join(relative).canonicalize() else {
        return false;
    };
    if !full.starts_with(dir) || !full.is_file() {
        return false;
    }
    let Ok(body) = std::fs::read(&full) else {
        return false;
    };
    let content_type = match full.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    };
    let mut writer = stream;
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-cache\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer.write_all(head.as_bytes()).is_ok() && writer.write_all(&body).is_ok()
}

/// A WebSocket session: requests in, replies and events out.
///
/// Events are pushed from the same thread between reads, so a client that stops reading
/// slows only itself down. The read timeout is what makes that possible without a second
/// thread per connection.
fn serve_websocket(stream: &TcpStream, key: &str, handle: &ControlHandle, running: &AtomicBool) {
    // The request line and headers have already been read off the socket, so the upgrade is
    // completed here rather than by tungstenite's own handshake: all that is left of it is
    // the accept key, which is a hash of the client's.
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    let upgrade = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    if (&*stream).write_all(upgrade.as_bytes()).is_err() {
        return;
    }

    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    if cloned
        .set_read_timeout(Some(Duration::from_millis(50)))
        .is_err()
    {
        return;
    }
    let mut socket =
        tungstenite::WebSocket::from_raw_socket(cloned, tungstenite::protocol::Role::Server, None);

    let events = handle.subscribe();
    while running.load(Ordering::Relaxed) {
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let sent = match serde_json::from_str::<Request>(&text) {
                    // sent before the request stops counting as unwritten, as over REST
                    Ok(request) => handle
                        .call_and_deliver(request, |response| send_json(&mut socket, &response)),
                    Err(error) => send_json(
                        &mut socket,
                        &Response::failed(
                            None,
                            ApiError::new(
                                "bad_request",
                                format!("That is not a request this version understands: {error}"),
                                false,
                            ),
                        ),
                    ),
                };
                if sent.is_err() {
                    return;
                }
            }
            // a read timeout is how this thread gets a turn to push events; anything else
            // that goes wrong on the socket ends the connection
            Err(tungstenite::Error::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut => {}
            Ok(tungstenite::Message::Close(_)) | Err(_) => return,
            Ok(_) => {}
        }

        for event in events.try_iter() {
            if send_json(&mut socket, &event).is_err() {
                return;
            }
        }
    }
}

fn send_json<T: serde::Serialize, S: Read + Write>(
    socket: &mut tungstenite::WebSocket<S>,
    value: &T,
) -> Result<(), ()> {
    let text = serde_json::to_string(value).map_err(|_| ())?;
    socket
        .send(tungstenite::Message::Text(text.into()))
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{channel, protocol::Request};

    #[test]
    fn loopback_addresses_are_recognised() {
        assert!(is_loopback("127.0.0.1:8515"));
        assert!(is_loopback("[::1]:8515"));
        assert!(!is_loopback("0.0.0.0:8515"));
        assert!(!is_loopback("192.168.1.10:8515"));
        assert!(!is_loopback("not an address"));
    }

    #[test]
    fn a_reachable_bind_without_a_token_is_refused() {
        // this interface can key a transmitter; an open one on a network address is not a
        // configuration to start and warn about
        let (handle, _channel) = channel();
        let error = ControlServer::start(
            &ControlConfig {
                bind: "0.0.0.0:0".to_owned(),
                token: None,
                ui_dir: None,
            },
            handle,
        )
        .expect_err("an open network bind was accepted");
        assert!(matches!(error, ServerError::Unprotected(_)), "{error}");
        assert!(
            error.to_string().contains("127.0.0.1"),
            "the message does not say what to do instead: {error}"
        );
    }

    #[test]
    fn a_loopback_bind_needs_no_token() {
        let (handle, _channel) = channel();
        let server = ControlServer::start(
            &ControlConfig {
                bind: "127.0.0.1:0".to_owned(),
                token: None,
                ui_dir: None,
            },
            handle,
        )
        .expect("loopback should start");
        assert!(server.address.ip().is_loopback());
        assert_ne!(server.address.port(), 0);
    }

    /// Drive one REST request end to end against a stub modem.
    fn rest_request(
        server: &ControlServer,
        method: &str,
        body: &str,
        token: Option<&str>,
    ) -> String {
        use std::io::{Read, Write};
        let mut stream = TcpStream::connect(server.address).expect("connect");
        let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
        let request = format!(
            "POST /v1/{method} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).expect("write");
        let mut text = String::new();
        stream.read_to_string(&mut text).expect("read");
        text
    }

    #[test]
    fn a_rest_one_shot_reaches_the_modem_and_comes_back() {
        let (handle, control) = channel();
        let server = ControlServer::start(
            &ControlConfig {
                bind: "127.0.0.1:0".to_owned(),
                token: None,
                ui_dir: None,
            },
            handle,
        )
        .expect("start");

        let modem = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while std::time::Instant::now() < deadline {
                let commands = control.drain();
                let answered = !commands.is_empty();
                for command in commands {
                    let id = command.request.id.clone();
                    let method = command.request.method.clone();
                    let _ = command
                        .reply
                        .send(Response::ok(id, json!({"method": method})));
                }
                if answered {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("the request never reached the modem");
        });

        let response = rest_request(&server, "status", "{}", None);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("\"method\":\"status\""), "{response}");
        modem.join().expect("modem thread");
    }

    #[test]
    fn the_panel_is_served_and_cannot_be_escaped_from() {
        // this process can key a transmitter and an operator may point it at a network
        // interface, so a path that climbs out of the panel directory has to be refused
        let dir = std::env::temp_dir().join(format!("aether-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("index.html"), b"<h1>panel</h1>").expect("write");
        let secret = dir.parent().expect("parent").join("aether-secret.txt");
        std::fs::write(&secret, b"not for you").expect("write");

        let (handle, _control) = channel();
        let server = ControlServer::start(
            &ControlConfig {
                bind: "127.0.0.1:0".to_owned(),
                token: None,
                ui_dir: Some(dir.clone()),
            },
            handle,
        )
        .expect("start");

        let fetch = |path: &str| {
            use std::io::{Read, Write};
            let mut stream = TcpStream::connect(server.address).expect("connect");
            stream
                .write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
                .expect("write");
            let mut text = String::new();
            let _ = stream.read_to_string(&mut text);
            text
        };

        let page = fetch("/");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("<h1>panel</h1>"), "{page}");
        assert!(page.contains("text/html"), "{page}");

        for climb in [
            "/../aether-secret.txt",
            "/..%2Faether-secret.txt",
            "/./../aether-secret.txt",
        ] {
            let answer = fetch(climb);
            assert!(
                !answer.contains("not for you"),
                "{climb} escaped the panel directory: {answer}"
            );
        }

        let missing = fetch("/nothing-here.js");
        assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

        let _ = std::fs::remove_file(&secret);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_get_is_told_what_the_interface_actually_speaks() {
        use std::io::{Read, Write};
        let (handle, _control) = channel();
        let server = ControlServer::start(
            &ControlConfig {
                bind: "127.0.0.1:0".to_owned(),
                token: None,
                ui_dir: None,
            },
            handle,
        )
        .expect("start");

        let mut stream = TcpStream::connect(server.address).expect("connect");
        stream
            .write_all(b"GET /v1 HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("write");
        let mut text = String::new();
        stream.read_to_string(&mut text).expect("read");
        assert!(text.starts_with("HTTP/1.1 404"), "{text}");
        assert!(text.contains("POST /v1/"), "{text}");
    }

    #[test]
    fn a_websocket_client_gets_replies_and_unsolicited_events() {
        let (handle, control) = channel();
        let server = ControlServer::start(
            &ControlConfig {
                bind: "127.0.0.1:0".to_owned(),
                token: None,
                ui_dir: None,
            },
            handle,
        )
        .expect("start");

        let modem = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut answered = false;
            while std::time::Instant::now() < deadline {
                for command in control.drain() {
                    let id = command.request.id.clone();
                    let _ = command
                        .reply
                        .send(Response::ok(id, json!({"state": "idle"})));
                    answered = true;
                }
                if answered && control.subscriber_count() > 0 {
                    control.publish(&crate::control::Event::new("ptt", json!({"on": true})));
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let url = format!("ws://{}/v1", server.address);
        let mut socket = None;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok((connected, _)) = tungstenite::connect(&url) {
                socket = Some(connected);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut socket = socket.expect("the control interface never accepted a WebSocket");

        socket
            .send(tungstenite::Message::Text(
                r#"{"id":"1","method":"status"}"#.into(),
            ))
            .expect("send");

        let mut saw_reply = false;
        let mut saw_event = false;
        for _ in 0..50 {
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    if text.contains("\"ok\":true") {
                        saw_reply = true;
                    }
                    if text.contains("\"event\":\"ptt\"") {
                        saw_event = true;
                    }
                }
                Ok(_) => {}
                Err(error) => panic!("websocket: {error}"),
            }
            if saw_reply && saw_event {
                break;
            }
        }
        assert!(saw_reply, "no reply to the request");
        assert!(saw_event, "no unsolicited event");
        modem.join().expect("modem thread");
    }

    #[test]
    fn a_request_the_modem_never_answers_does_not_hang_the_client_for_ever() {
        // the modem end is dropped, which is what happens when the daemon is shutting down
        let (handle, control) = channel();
        drop(control);
        let error = handle
            .call(Request {
                id: None,
                method: "status".into(),
                params: json!({}),
                token: None,
            })
            .expect_err("it should fail");
        assert_eq!(error.code, "modem_stopped");
    }
}
