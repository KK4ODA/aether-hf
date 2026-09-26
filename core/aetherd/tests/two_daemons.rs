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
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

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

struct Daemon {
    child: Child,
    control: u16,
    log: PathBuf,
}

impl Daemon {
    fn start(dir: &Path, name: &str, callsign: &str, sim: &str) -> Self {
        Self::start_with(dir, name, callsign, sim, "")
    }

    /// Start with extra `[radio]` lines — the bandwidth, say.
    ///
    /// Every port is the system's choice (port 0), learned from the daemon's log. The tests
    /// used to pick ports themselves — bind one, let it go, write it into the file — and on a
    /// loaded CI runner another test's simulated channel was given the same port in between:
    /// the daemon under test could not listen, and the test spoke HTTP to a socket that never
    /// answers ("no body in \"\"").
    fn start_with(dir: &Path, name: &str, callsign: &str, sim: &str, radio: &str) -> Self {
        let config = dir.join(format!("{name}.toml"));
        std::fs::write(
            &config,
            format!(
                "schema_version = {schema}\ncallsign = \"{callsign}\"\n\
                 [radio]\nwait_for_clear = false\n{radio}\n\
                 [control]\nbind = \"127.0.0.1:0\"\n\
                 [record]\nauto = true\n\
                 [sim]\n{sim}\nsnr_db = 25.0\n",
                // the current schema, so the settings a test names mean what they say today
                schema = aetherd::config::SCHEMA_VERSION,
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
        let mut daemon = Self {
            child,
            control: 0,
            log,
        };
        let address = daemon.logged("its control interface", "listening on ws://", "/v1");
        daemon.control = address
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .unwrap_or_else(|| panic!("not an address: {address:?}"));
        daemon
    }

    /// What the daemon's log says between `before` and `after`, once it says it — an
    /// address it has bound, which the log gives as the system assigned it.
    fn logged(&mut self, what: &str, before: &str, after: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let text = std::fs::read_to_string(&self.log).unwrap_or_default();
            if let Some((found, _)) = text
                .split_once(before)
                .and_then(|(_, rest)| rest.split_once(after))
            {
                return found.to_owned();
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!("the daemon exited ({status}) before logging {what}:\n{text}");
            }
            assert!(
                Instant::now() < deadline,
                "the daemon never logged {what}:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Where a daemon started with `listen = "127.0.0.1:0"` listens for its simulated
    /// channel's other end.
    fn sim_address(&mut self) -> String {
        self.logged(
            "its simulated channel",
            "simulated channel (listening on ",
            ")",
        )
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
        // one that has already exited — asked to restart, say — has nothing left to stop
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = call(self.control, "shutdown", &json!({}));
        }
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
fn a_daemon_asked_to_restart_exits_asking_for_it() {
    // the desktop shell and systemd start the daemon again on this status and on no other:
    // a setting that needs a restart is applied that way, without the operator knowing
    // there was a restart to do
    let dir = std::env::temp_dir().join(format!("aether-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mut daemon = Daemon::start(&dir, "r", "W4ODA", "listen = \"127.0.0.1:0\"");
    assert_eq!(
        daemon.status()["supervised"],
        false,
        "nothing set AETHERD_SUPERVISED"
    );
    let answer = daemon.call("shutdown", &json!({ "restart": true }));
    assert_eq!(answer["result"]["restart"], true, "{answer}");
    let status = daemon.child.wait().expect("exit status");
    assert_eq!(status.code(), Some(75), "{status}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_daemons_complete_a_session_over_the_simulated_channel() {
    let dir = std::env::temp_dir().join(format!("aether-two-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut a = Daemon::start(&dir, "a", "W4ODA", "listen = \"127.0.0.1:0\"");
    let channel = a.sim_address();
    let b = Daemon::start(&dir, "b", "KK4XYZ", &format!("connect = \"{channel}\""));
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

#[test]
fn a_kiss_frame_crosses_from_one_daemons_kiss_port_to_the_others() {
    use aetherd::kiss::framing::{Decoded, Decoder, encode};
    // ADR-0019 end to end: an APRS program on one station's KISS port, and VarAC's type-1
    // frames beside it, heard by a program on the other station's — over the simulated
    // channel, as datagrams on the tone floor, with the frame types and lengths kept
    let dir = std::env::temp_dir().join(format!("aether-kiss-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let kiss = "\n[kiss]\nenabled = true\nbind = \"127.0.0.1:0\"";
    let mut a = Daemon::start_with(&dir, "a", "W4ODA", "listen = \"127.0.0.1:0\"", kiss);
    let channel = a.sim_address();
    let b = Daemon::start_with(
        &dir,
        "b",
        "KK4XYZ",
        &format!("connect = \"{channel}\""),
        kiss,
    );
    let port_of = |daemon: &Daemon| -> u16 {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(port) = daemon.status()["kiss"]["address"]
                .as_str()
                .and_then(|a| a.rsplit_once(':'))
                .and_then(|(_, p)| p.parse().ok())
            {
                return port;
            }
            assert!(Instant::now() < deadline, "the KISS port never opened");
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    let mut listener = TcpStream::connect(("127.0.0.1", port_of(&b))).expect("B's KISS port");
    listener
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("timeout");
    let mut sender = TcpStream::connect(("127.0.0.1", port_of(&a))).expect("A's KISS port");

    // an AX.25 UI frame, as an APRS program hands it over: no flags, no FCS
    let mut aprs: Vec<u8> = b"APRS  ".iter().map(|c| c << 1).collect();
    aprs.push(0xE0);
    aprs.extend(b"W4ODA ".iter().map(|c| c << 1));
    aprs.push(0x61);
    aprs.extend([0x03, 0xF0]);
    aprs.extend(b"!3346.00N/08418.00W-Aether KISS over the air ".iter());
    aprs.extend([0xC0, 0xDB]); // FEND and FESC inside the frame
    let varac = b"VarAC-style type-1 broadcast".to_vec();
    let mut wire = encode(0, 0, &aprs);
    wire.extend(encode(0, 1, &varac));
    sender.write_all(&wire).expect("send");

    let mut decoder = Decoder::new(4096);
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(150);
    let mut buffer = [0u8; 4096];
    while got.len() < 2 && Instant::now() < deadline {
        if let Ok(n) = listener.read(&mut buffer) {
            assert!(n > 0, "B's KISS port closed the connection");
            for decoded in decoder.push(&buffer[..n]) {
                if let Decoded::Frame(frame) = decoded {
                    got.push((frame.command, frame.data));
                }
            }
        }
    }
    assert_eq!(
        got,
        [(0, aprs), (1, varac)],
        "both frames, in order, as sent"
    );
    assert_eq!(a.status()["datagrams"]["sent"], 2);
    assert_eq!(b.status()["datagrams"]["heard"], 2);
    // and B heard W4ODA as a station sending datagrams
    let heard = b.call("heard.list", &json!({}));
    assert!(
        heard["result"]["stations"]
            .as_array()
            .is_some_and(|s| s.iter().any(|st| st["callsign"] == "W4ODA")),
        "{heard}"
    );
    drop(b);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A host program on a daemon's VARA-compatible port: lines out, lines in.
struct Host {
    stream: TcpStream,
    reader: std::io::BufReader<TcpStream>,
}

impl Host {
    fn attach(daemon: &Daemon) -> Self {
        let address = daemon.status()["host"]["command_address"]
            .as_str()
            .expect("the host port's address")
            .to_owned();
        let stream = TcpStream::connect(address).expect("command port");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        let reader = std::io::BufReader::new(stream.try_clone().expect("clone"));
        Self { stream, reader }
    }

    fn send(&mut self, line: &str) {
        self.stream
            .write_all(format!("{line}\r").as_bytes())
            .expect("write");
    }

    /// The next line `want` accepts, past the ones it does not — `PTT`, `BUFFER`, `IAMALIVE`.
    fn next(&mut self, want: impl Fn(&str) -> bool) -> String {
        use std::io::BufRead as _;
        let mut passed = Vec::new();
        loop {
            let mut raw = Vec::new();
            self.reader
                .read_until(b'\r', &mut raw)
                .unwrap_or_else(|e| panic!("{e}; heard only {passed:?}"));
            let line = String::from_utf8_lossy(&raw).trim().to_owned();
            if want(&line) {
                return line;
            }
            passed.push(line);
        }
    }

    /// Whether `DISCONNECTED` comes before the answer to a `VERSION` asked now.
    fn told_disconnected(&mut self) -> bool {
        std::thread::sleep(Duration::from_millis(300));
        self.send("VERSION");
        self.next(|l| l == "DISCONNECTED" || l.starts_with("VERSION")) == "DISCONNECTED"
    }
}

#[test]
fn a_host_hears_how_each_of_its_calls_ended() {
    // Winlink Express, Pat and VarAC wait for CONNECTED or DISCONNECTED after a CONNECT. A
    // call that ended without a session — aborted, or given up on — said neither, and a
    // DISCONNECT during a call went on calling. One daemon and nobody to answer it.
    let dir = std::env::temp_dir().join(format!("aether-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let host = "\n[host]\nenabled = true\nbind = \"127.0.0.1:0\"";
    let daemon = Daemon::start_with(&dir, "a", "W4ODA", "listen = \"127.0.0.1:0\"", host);
    let mut client = Host::attach(&daemon);
    let answer = |client: &mut Host| client.next(|l| l == "OK" || l == "WRONG");
    // the adapter says OK as it hands a command on, so the modem may not have it yet
    let calling = || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while daemon.status()["state"] != "connecting" {
            assert!(Instant::now() < deadline, "{}", daemon.status());
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    client.send("MYCALL W4ODA");
    assert_eq!(answer(&mut client), "OK");

    // ABORT during a call
    client.send("CONNECT W4ODA KK4XYZ");
    assert_eq!(answer(&mut client), "OK");
    calling();
    client.send("ABORT");
    assert_eq!(answer(&mut client), "OK");
    assert_eq!(client.next(|l| l == "DISCONNECTED"), "DISCONNECTED");
    assert_eq!(daemon.status()["state"], "idle");

    // DISCONNECT during a call stops it
    client.send("CONNECT W4ODA KK4XYZ");
    assert_eq!(answer(&mut client), "OK");
    calling();
    client.send("DISCONNECT");
    assert_eq!(answer(&mut client), "OK");
    assert_eq!(client.next(|l| l == "DISCONNECTED"), "DISCONNECTED");
    assert_eq!(daemon.status()["state"], "idle", "it went on calling");

    // a call aborted and another placed at once, without waiting: one DISCONNECTED each
    client.send("CONNECT W4ODA KK4XYZ\rABORT\rCONNECT W4ODA KK4XYZ");
    assert_eq!(client.next(|l| l == "DISCONNECTED"), "DISCONNECTED");
    assert!(
        !client.told_disconnected(),
        "the first call's end was reported twice"
    );
    calling();
    client.send("ABORT");
    assert_eq!(
        client.next(|l| l == "DISCONNECTED"),
        "DISCONNECTED",
        "the second call's end was taken for the first's"
    );
    assert!(!client.told_disconnected());
    drop(client);
    drop(daemon);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_daemons_complete_a_session_at_500_hz() {
    // the narrow waveform end to end through the daemon: its own mode table, the
    // capabilities a host reads `BW500` from, and a recording that says which waveform
    let dir = std::env::temp_dir().join(format!("aether-narrow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let radio = "bandwidth = 500\nmax_mode = 14\n";
    let mut a = Daemon::start_with(&dir, "a", "W4ODA", "listen = \"127.0.0.1:0\"", radio);
    let channel = a.sim_address();
    let b = Daemon::start_with(
        &dir,
        "b",
        "KK4XYZ",
        &format!("connect = \"{channel}\""),
        radio,
    );
    let caps = a.call("capabilities", &json!({}))["result"].clone();
    assert_eq!(caps["bandwidth_hz"], 500, "{caps}");
    assert_eq!(caps["modes"].as_array().map(Vec::len), Some(15));
    // the tone floor (ADR-0013) and its four-tone middle kinds (ADR-0015) lead the ladder;
    // the control mode, QPSK 1/2, is rung 5
    assert_eq!(caps["modes"][0]["name"], "tone-24");
    assert_eq!(caps["modes"][0]["floor"], true);
    assert_eq!(caps["modes"][2]["name"], "tone4x100-51");
    assert_eq!(caps["modes"][3]["floor"], true);
    assert_eq!(caps["modes"][4]["name"], "QPSK-1/3");
    assert_eq!(caps["modes"][5]["name"], "QPSK-1/2");
    assert_eq!(caps["modes"][5]["floor"], false);

    let message = "At 500 Hz: the bandwidth peer-to-peer contacts are made in, end to end.";
    let connected = a.call("connect", &json!({"remote": "KK4XYZ"}));
    assert_eq!(connected["ok"], true, "{connected}");
    a.wait_for_state(&b, "connected", "the narrow session never came up");
    let encoded = aetherd::control::methods::to_base64(message.as_bytes());
    let sent = a.call("send", &json!({"data": encoded}));
    assert_eq!(sent["ok"], true, "{sent}");
    let received = receive(b.control, message.len(), || b.status()["counters"].clone());
    assert_eq!(String::from_utf8_lossy(&received), message);
    let mode = b.status()["metrics"]["mode"].as_u64().unwrap_or(99);
    assert!(mode < 12, "a narrow mode index: {mode}");

    let closed = a.call("disconnect", &json!({}));
    assert_eq!(closed["ok"], true, "{closed}");
    a.wait_for_state(&b, "idle", "the narrow session never closed");
    assert_recorded(&dir, "W4ODA_KK4XYZ");
    // the sidecar says which waveform, so a replay runs the right receiver
    let sidecar = std::fs::read_dir(dir.join("recordings"))
        .expect("recordings")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .expect("a sidecar");
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar).expect("sidecar")).expect("json");
    assert_eq!(document["session"]["bandwidth_hz"], 500);
    drop(b);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_host_moves_its_station_to_500_hz_and_a_wide_station_answers_the_call() {
    // ADR-0026, end to end: VarAC's BW500 moves a 2 300 Hz station to 500 Hz; its call is
    // answered by another 2 300 Hz station, which moves for the session; the host hears the
    // bandwidth in CONNECTED; when the host goes, its station goes back to its own
    let dir = std::env::temp_dir().join(format!("aether-follow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let host = "\n[host]\nenabled = true\nbind = \"127.0.0.1:0\"";
    let mut a = Daemon::start_with(&dir, "a", "W4ODA", "listen = \"127.0.0.1:0\"", host);
    let channel = a.sim_address();
    let b = Daemon::start(&dir, "b", "KK4XYZ", &format!("connect = \"{channel}\""));
    let mut client = Host::attach(&a);
    let answer = |client: &mut Host| client.next(|l| l == "OK" || l == "WRONG");
    client.send("MYCALL W4ODA");
    assert_eq!(answer(&mut client), "OK");
    client.send("BW500");
    assert_eq!(answer(&mut client), "OK");
    let bandwidth = a.status()["bandwidth"].clone();
    assert_eq!(bandwidth["bandwidth_hz"], 500, "{bandwidth}");
    assert_eq!(bandwidth["why"], "host");
    assert_eq!(
        a.call("capabilities", &json!({}))["result"]["bandwidth_hz"],
        500
    );

    client.send("LISTEN ON");
    assert_eq!(answer(&mut client), "OK");
    client.send("CONNECT W4ODA KK4XYZ");
    assert_eq!(answer(&mut client), "OK");
    a.wait_for_state(&b, "connected", "the 500 Hz call was not answered");
    assert_eq!(
        client.next(|l| l.starts_with("CONNECTED")),
        "CONNECTED W4ODA KK4XYZ 500"
    );
    let theirs = b.status()["bandwidth"].clone();
    assert_eq!(theirs["bandwidth_hz"], 500, "{theirs}");
    assert_eq!(theirs["why"], "call");
    assert_eq!(theirs["caller"], "W4ODA");

    let message = "Answered at 500 Hz by a station set to 2300.";
    let encoded = aetherd::control::methods::to_base64(message.as_bytes());
    assert_eq!(a.call("send", &json!({"data": encoded}))["ok"], true);
    let received = receive(b.control, message.len(), || b.status()["counters"].clone());
    assert_eq!(String::from_utf8_lossy(&received), message);

    client.send("DISCONNECT");
    assert_eq!(answer(&mut client), "OK");
    assert_eq!(client.next(|l| l == "DISCONNECTED"), "DISCONNECTED");
    a.wait_for_state(&b, "idle", "the session never closed");
    // the host goes: its station goes back to its own at once, the station that was called
    // after a quiet spell
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(60);
    while a.status()["bandwidth"]["bandwidth_hz"] != 2300
        || b.status()["bandwidth"]["bandwidth_hz"] != 2300
    {
        assert!(
            Instant::now() < deadline,
            "a={} b={}",
            a.status()["bandwidth"],
            b.status()["bandwidth"]
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert_eq!(a.status()["bandwidth"]["why"], "configured");
    drop(b);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_daemons_run_a_test_session_over_the_simulated_channel() {
    let dir = std::env::temp_dir().join(format!("aether-two-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    // the ladder stops at the operator's fastest mode: six rungs, every one a tone kind — the
    // floor's two and the fast four (ADR-0014) — at a frame a rung and small transfers, so
    // the five-second frames keep the test inside its deadline
    let mut a = Daemon::start_with(
        &dir,
        "a",
        "W4ODA",
        "listen = \"127.0.0.1:0\"",
        "max_mode = 5",
    );
    let channel = a.sim_address();
    let b = Daemon::start(&dir, "b", "KK4XYZ", &format!("connect = \"{channel}\""));
    // the simulated link has just met: a moment for both ends' clocks to be running
    // before the probe, which is one frame with no retry
    std::thread::sleep(Duration::from_secs(3));

    let started = a.call(
        "test.start",
        &json!({ "remote": "KK4XYZ", "message_bytes": 256, "file_bytes": 512, "rung_frames": 1 }),
    );
    assert_eq!(started["ok"], true, "{started}");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let status = a.call("test.status", &json!({}))["result"].clone();
        if status["running"] == false {
            let results = &status["results"];
            assert_eq!(results["outcome"], "complete", "{results}");
            // The probe is one frame with no retry, sent the moment the run starts; on a
            // loaded runner it has gone unanswered while everything after it completed.
            // The probe has its own tests; here it is reported, not required.
            if results["probe"].is_null() {
                eprintln!("the probe went unanswered on this runner: {results}");
            } else {
                assert!(results["probe"]["heard_here_db"].is_number(), "{results}");
            }
            assert!(
                results["message"]["bps"].as_f64().is_some_and(|b| b > 0.0),
                "{results}"
            );
            assert!(
                results["file"]["bps"].as_f64().is_some_and(|b| b > 0.0),
                "{results}"
            );
            let ladder = results["ladder"].as_array().expect("rungs");
            assert_eq!(ladder.len(), 6, "{results}");
            assert!(
                ladder.iter().all(|r| r["decoded"] == r["frames"]),
                "{results}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the test session did not finish: {status}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    a.wait_for_state(&b, "idle", "the session never closed");
    // the run recorded itself, under a name that says so
    let sidecars: Vec<PathBuf> = std::fs::read_dir(dir.join("recordings"))
        .expect("dir")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.to_string_lossy().contains("W4ODA_KK4XYZ_test")
                && p.extension().is_some_and(|x| x == "json")
        })
        .collect();
    assert_eq!(sidecars.len(), 1, "{sidecars:?}");
    let document: Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecars[0]).expect("read")).expect("json");
    assert_eq!(document["session"]["test"]["outcome"], "complete");
    drop(b);
    drop(a);
    let _ = std::fs::remove_dir_all(&dir);
}
