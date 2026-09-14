//! Two real `aetherd` processes, a simulated channel between them, a session over the
//! control API — the daemon as it ships, end to end, with no sound card and no radio.
//!
//! This is the test the unit tests cannot be: the binary's argument parsing, its
//! configuration file, the control server, the run loop's pacing against a real clock, and
//! the `[sim]` backend, all at once. It takes as long as a session takes. It found two
//! engine bugs on its first run that the deterministic simulator never could: a reply that
//! arrived during a re-poll was accepted and then never acted on, and the engine waited
//! for replies from when it *handed over* a burst rather than from when the burst left the
//! sound card.

use std::{
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

/// A free port, briefly bound and released.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

/// `POST /v1/<method>` over loopback, hand-rolled: the test carries no HTTP client.
fn call(port: u16, method: &str, params: &Value) -> Value {
    let body = params.to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("control port");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let request = format!(
        "POST /v1/{method} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    let (_, payload) = response
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("no body in {response:?}"));
    serde_json::from_str(payload).unwrap_or_else(|e| panic!("{e}: {payload:?}"))
}

fn wait_for_port(port: u16, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "{what} never listened on {port}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct Daemon {
    child: Child,
    control: u16,
}

impl Daemon {
    fn start(dir: &Path, name: &str, callsign: &str, sim: &str) -> Self {
        let control = free_port();
        let config = dir.join(format!("{name}.toml"));
        std::fs::write(
            &config,
            format!(
                "callsign = \"{callsign}\"\n\
                 [radio]\nwait_for_clear = false\n\
                 [control]\nbind = \"127.0.0.1:{control}\"\n\
                 [record]\nauto = true\n\
                 [sim]\n{sim}\nsnr_db = 25.0\n"
            ),
        )
        .expect("config");
        let log: PathBuf = dir.join(format!("{name}.log"));
        let child = Command::new(env!("CARGO_BIN_EXE_aetherd"))
            .arg("--config")
            .arg(&config)
            .stdout(Stdio::from(std::fs::File::create(&log).expect("log")))
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start aetherd");
        Self { child, control }
    }

    fn call(&self, method: &str, params: &Value) -> Value {
        call(self.control, method, params)
    }

    fn status(&self) -> Value {
        self.call("status", &json!({}))["result"].clone()
    }

    fn wait_for_state(&self, other: &Self, state: &str, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.status()["state"] != state || other.status()["state"] != state {
            assert!(
                Instant::now() < deadline,
                "{what}: a={} b={}",
                self.status()["state"],
                other.status()["state"]
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = call(self.control, "shutdown", &json!({}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Everything `data` events deliver on a daemon's WebSocket until `expected` bytes are in.
fn receive(port: u16, expected: usize, counters: impl Fn() -> Value) -> Vec<u8> {
    let (mut listener, _) =
        tungstenite::connect(format!("ws://127.0.0.1:{port}/v1")).expect("websocket");
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = listener.get_mut() {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("timeout");
    }
    let mut received = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    while received.len() < expected {
        assert!(
            Instant::now() < deadline,
            "delivered {} of {expected} bytes; counters: {}",
            received.len(),
            counters()
        );
        match listener.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let frame: Value = serde_json::from_str(&text).expect("event json");
                if frame["event"] == "data"
                    && let Some(payload) = frame["data"]["data"].as_str()
                {
                    received
                        .extend(aetherd::control::methods::from_base64(payload).expect("base64"));
                }
            }
            // anything else, including a read timeout, is "nothing yet"
            Ok(_) | Err(tungstenite::Error::Io(_)) => {}
            Err(error) => panic!("websocket: {error}"),
        }
    }
    received
}

/// A closed recording of the session, with at least one decoded frame in it.
fn assert_recorded(dir: &Path, stations: &str) {
    let sidecar = std::fs::read_dir(dir.join("recordings"))
        .unwrap_or_else(|e| panic!("no recordings directory: {e}"))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|e| e == "json") && p.to_string_lossy().contains(stations)
        })
        .unwrap_or_else(|| panic!("{stations} did not record its session"));
    let text = std::fs::read_to_string(&sidecar).expect("sidecar");
    let document: Value = serde_json::from_str(&text).expect("json");
    assert!(
        document["ended"].is_string(),
        "{stations}: the recording was not closed"
    );
    assert!(
        document["frames"]
            .as_array()
            .is_some_and(|f| f.iter().any(|x| x["decoded"] == true)),
        "{stations}: the recording has no decoded frame"
    );
}

#[test]
fn two_daemons_complete_a_session_over_the_simulated_channel() {
    let dir = std::env::temp_dir().join(format!("aether-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let channel = free_port();

    let a = Daemon::start(
        &dir,
        "a",
        "W4ODA",
        &format!("listen = \"127.0.0.1:{channel}\""),
    );
    let b = Daemon::start(
        &dir,
        "b",
        "KK4XYZ",
        &format!("connect = \"127.0.0.1:{channel}\""),
    );
    wait_for_port(a.control, "daemon a");
    wait_for_port(b.control, "daemon b");
    assert_eq!(a.status()["state"], "idle");
    assert_eq!(b.status()["callsign"], "KK4XYZ");

    // a calls b, sends a message the moment it is connected — as a host would — and b
    // receives it: over the socket, paced by the clock
    let message = "Two daemons, one machine, no radio: the whole thing end to end.";
    let connected = a.call("connect", &json!({"remote": "KK4XYZ"}));
    assert_eq!(connected["ok"], true, "{connected}");
    a.wait_for_state(&b, "connected", "the session never came up");
    let encoded = aetherd::control::methods::to_base64(message.as_bytes());
    let sent = a.call("send", &json!({"data": encoded}));
    assert_eq!(sent["ok"], true, "{sent}");
    let received = receive(b.control, message.len(), || b.status()["counters"].clone());
    assert_eq!(String::from_utf8_lossy(&received), message);

    let closed = a.call("disconnect", &json!({}));
    assert_eq!(closed["ok"], true, "{closed}");
    a.wait_for_state(&b, "idle", "the session never closed");

    // and, with [record] auto on, both sides recorded the session
    assert_recorded(&dir, "W4ODA_KK4XYZ");
    assert_recorded(&dir, "KK4XYZ_W4ODA");
    drop(b);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}
