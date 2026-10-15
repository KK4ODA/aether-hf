//! Protocol 8's engine against the reference model's, input by input (ADR-0069 step 2).
//!
//! `tools/make_granted_vectors.py` ran the model's engines through sessions — clean and weak,
//! both airs, fading, outages, false detections, late decodes and late keying, aborts, probes,
//! identifiers, the Test's ladder — and recorded every input each engine was handed and every
//! action it drained. Here the port is handed the same inputs: its decodes are answered from
//! the record (and the HARQ buffer it hands each one is checked against the model's), every
//! action is compared, and after every input so is what the engine says of itself — state,
//! role, session, level, rung, queues, deadline, counters. The engines draw the same random
//! numbers from the same seed, so nothing is left to chance: a difference is a bug in the port,
//! found at the input it went wrong on.
//!
//! Regenerate with `python tools/make_granted_vectors.py`, and only for a deliberate change to
//! the model engine.

use std::{cell::RefCell, collections::VecDeque, fs, path::PathBuf};

use aether_link::{
    Action, Container, GrantConfig, GrantStats, GrantedEngine, HarqBuffer, PhyTiming, SoftFrame,
    State, TxFrame, granted::Role,
};
use serde_json::Value;

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/granted_vectors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}).\nGenerate it with: python tools/make_granted_vectors.py",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("vector file is not valid JSON")
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

fn opt(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn int(v: &Value) -> usize {
    usize::try_from(v.as_u64().unwrap_or_else(|| panic!("not an integer: {v}"))).expect("usize")
}

fn opt_int(v: &Value) -> Option<usize> {
    v.as_u64().map(|n| usize::try_from(n).expect("usize"))
}

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

fn close_opt(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => close(a, b),
        _ => false,
    }
}

fn timing(doc: &Value) -> PhyTiming {
    PhyTiming {
        data_frame_s: num(&doc["data_frame_s"]),
        control_frame_s: num(&doc["control_frame_s"]),
        turnaround_s: num(&doc["turnaround_s"]),
        detect_latency_s: num(&doc["detect_latency_s"]),
        tx_latency_s: num(&doc["tx_latency_s"]),
        preamble_detect_s: opt(&doc["preamble_detect_s"]),
        data_capacity: doc["data_capacity"]
            .as_array()
            .expect("capacity")
            .iter()
            .map(int)
            .collect(),
        mode_threshold_db: doc["mode_threshold_db"]
            .as_array()
            .expect("thresholds")
            .iter()
            .map(num)
            .collect(),
        floor_data_frame_s: opt(&doc["floor_data_frame_s"]),
        floor_control_frame_s: opt(&doc["floor_control_frame_s"]),
        floor_modes: int(&doc["floor_modes"]),
        control_threshold_db: doc["control_threshold_db"]
            .as_array()
            .map(|pair| [num(&pair[0]), num(&pair[1])]),
        floor_margin_db: opt(&doc["floor_margin_db"]),
        floor_preamble_detect_s: opt(&doc["floor_preamble_detect_s"]),
    }
}

fn pair(doc: &Value) -> [f64; 2] {
    [num(&doc[0]), num(&doc[1])]
}

fn config(doc: &Value) -> GrantConfig {
    let levels = doc["levels_s"].as_array().expect("levels");
    GrantConfig {
        max_data_frames: int(&doc["max_data_frames"]),
        max_burst_s: opt(&doc["max_burst_s"]),
        max_retries: int(&doc["max_retries"]),
        connect_retries: int(&doc["connect_retries"]),
        disc_retries: int(&doc["disc_retries"]),
        disc_patience_exchanges: num(&doc["disc_patience_exchanges"]),
        disc_patience_s: num(&doc["disc_patience_s"]),
        disc_drain_s: num(&doc["disc_drain_s"]),
        leave_repeats: int(&doc["leave_repeats"]),
        floor_answer_margin_db: num(&doc["floor_answer_margin_db"]),
        answer_floor_margin_db: num(&doc["answer_floor_margin_db"]),
        family_readings: int(&doc["family_readings"]),
        link_timeout_s: num(&doc["link_timeout_s"]),
        link_timeout_exchanges: num(&doc["link_timeout_exchanges"]),
        initial_mode: int(&doc["initial_mode"]),
        max_mode: int(&doc["max_mode"]),
        ceiling: opt_int(&doc["ceiling"]),
        silence_step: int(&doc["silence_step"]),
        max_combines: int(&doc["max_combines"]),
        reencode_early_after: int(&doc["reencode_early_after"]),
        reencode_hopeless_db: num(&doc["reencode_hopeless_db"]),
        capabilities: u8::try_from(int(&doc["capabilities"])).expect("caps"),
        first_grant: int(&doc["first_grant"]),
        window_doublings: int(&doc["window_doublings"]),
        elide: doc["elide"].as_bool().expect("elide"),
        turnaround_s: num(&doc["turnaround_s"]),
        identifier_s: num(&doc["identifier_s"]),
        identifiers: doc["identifiers"]
            .as_object()
            .expect("identifiers")
            .iter()
            .map(|(call, seconds)| (call.clone(), num(seconds)))
            .collect(),
        answer_gap_s: num(&doc["answer_gap_s"]),
        floor_answer_memory: int(&doc["floor_answer_memory"]),
        hold_unknown: doc["hold_unknown"].as_bool().expect("hold_unknown"),
        decode_budget_s: pair(&doc["decode_budget_s"]),
        key_s: num(&doc["key_s"]),
        guard_s: num(&doc["guard_s"]),
        slack_s: num(&doc["slack_s"]),
        levels_s: [
            num(&levels[0]),
            num(&levels[1]),
            num(&levels[2]),
            num(&levels[3]),
        ],
        open_window_s: pair(&doc["open_window_s"]),
        keepalive_s: num(&doc["keepalive_s"]),
        silence_levels_after: int(&doc["silence_levels_after"]),
        on_time_to_lower: int(&doc["on_time_to_lower"]),
        need_ease_after: int(&doc["need_ease_after"]),
        ..GrantConfig::default()
    }
}

/// One decode the model's engine asked of a frame: the buffer it combined with, what came
/// out, and the buffer to keep.
type Decode = (Option<f64>, Option<Vec<u8>>, f64);

/// A frame as the model handed it to its engine, answering decodes from the record.
struct Replayed {
    control: bool,
    mode: usize,
    floor: bool,
    rv: u8,
    snr_db: f64,
    trusted: bool,
    follows: Option<u8>,
    t_start: f64,
    t_end: f64,
    decodes: RefCell<VecDeque<Decode>>,
    at: String,
}

impl SoftFrame for Replayed {
    fn container(&self) -> Container {
        if self.control {
            Container::Control
        } else {
            Container::Data
        }
    }
    fn mode(&self) -> usize {
        self.mode
    }
    fn floor(&self) -> bool {
        self.floor
    }
    fn rv(&self) -> u8 {
        self.rv
    }
    fn snr_db(&self) -> f64 {
        self.snr_db
    }
    fn trusted(&self) -> bool {
        self.trusted
    }
    fn follows(&self) -> Option<u8> {
        self.follows
    }
    fn t_start(&self) -> f64 {
        self.t_start
    }
    fn t_end(&self) -> f64 {
        self.t_end
    }
    fn decode(&self, buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
        let (want, payload, out) = self.decodes.borrow_mut().pop_front().unwrap_or_else(|| {
            panic!(
                "{}: the port decoded the frame more often than the model",
                self.at
            )
        });
        match (buffer, want) {
            (None, None) => {}
            (Some(got), Some(want)) => assert!(
                got.len() == 1 && close(got[0], want),
                "{}: combined with {got:?}, the model with {want}",
                self.at
            ),
            (got, want) => panic!("{}: decoded with {got:?}, the model with {want:?}", self.at),
        }
        (payload, vec![out])
    }
}

struct Replay<'a> {
    blobs: &'a [Vec<u8>],
}

impl Replay<'_> {
    fn blob(&self, v: &Value) -> Vec<u8> {
        self.blobs[int(v)].clone()
    }

    fn frame(&self, doc: &Value, at: &str) -> Replayed {
        let decodes = doc["decodes"]
            .as_array()
            .expect("decodes")
            .iter()
            .map(|d| {
                (
                    opt(&d["in"]),
                    (!d["payload"].is_null()).then(|| self.blob(&d["payload"])),
                    num(&d["out"]),
                )
            })
            .collect();
        Replayed {
            control: doc["control"].as_bool().expect("control"),
            mode: int(&doc["mode"]),
            floor: doc["floor"].as_bool().expect("floor"),
            rv: u8::try_from(int(&doc["rv"])).expect("rv"),
            snr_db: num(&doc["snr_db"]),
            trusted: doc["trusted"].as_bool().expect("trusted"),
            follows: opt_int(&doc["follows"]).map(|f| u8::try_from(f).expect("follows")),
            t_start: num(&doc["t_start"]),
            t_end: num(&doc["t_end"]),
            decodes: RefCell::new(decodes),
            at: at.to_owned(),
        }
    }

    fn tx_frame_matches(&self, got: &TxFrame, want: &Value) -> bool {
        let control = want["control"].as_bool().expect("control");
        (got.container == Container::Control) == control
            && got.payload == self.blob(&want["payload"])
            && got.mode == int(&want["mode"])
            && usize::from(got.rv) == int(&want["rv"])
            && got.floor == want["floor"].as_bool().expect("floor")
            && usize::from(got.follows) == int(&want["follows"])
    }

    fn check_actions(&self, got: &[Action], want: &[Value], at: &str) {
        assert_eq!(
            got.len(),
            want.len(),
            "{at}: {} actions, the model {}: {got:?}",
            got.len(),
            want.len()
        );
        for (index, (action, w)) in got.iter().zip(want).enumerate() {
            let ok = match action {
                Action::Transmit {
                    frames,
                    duration_s,
                    cause,
                    not_after,
                    not_before,
                    identifier_s,
                    listen_first,
                } => w.get("transmit").is_some_and(|wf| {
                    let wf = wf.as_array().expect("frames");
                    frames.len() == wf.len()
                        && frames
                            .iter()
                            .zip(wf)
                            .all(|(f, x)| self.tx_frame_matches(f, x))
                        && close(*duration_s, num(&w["duration_s"]))
                        && *cause == w["cause"].as_str().expect("cause")
                        && close_opt(*not_after, opt(&w["not_after"]))
                        && close_opt(*not_before, opt(&w["not_before"]))
                        && close(*identifier_s, num(&w["identifier_s"]))
                        && Some(*listen_first) == w["listen_first"].as_bool()
                }),
                Action::Deliver(data) => w.get("deliver").is_some_and(|b| *data == self.blob(b)),
                Action::Event { name, detail } => {
                    w.get("event").is_some_and(|n| n.as_str() == Some(name))
                        && w["detail"].as_str() == Some(detail.as_str())
                }
            };
            assert!(ok, "{at}: action {index} is {action:?}, the model's {w}");
        }
    }
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Connecting => "connecting",
        State::Connected => "connected",
        State::Disconnecting => "disconnecting",
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::None => "none",
        Role::Master => "master",
        Role::Slave => "slave",
    }
}

fn stats(s: &GrantStats) -> [(&'static str, usize); 17] {
    [
        ("frames_sent", s.frames_sent),
        ("frames_resent", s.frames_resent),
        ("frames_received", s.frames_received),
        ("frames_failed", s.frames_failed),
        ("harq_rescues", s.harq_rescues),
        ("bytes_delivered", s.bytes_delivered),
        ("bytes_acked", s.bytes_acked),
        ("probes_sent", s.probes_sent),
        ("probes_answered", s.probes_answered),
        ("probe_replies", s.probe_replies),
        ("frames_reencoded", s.frames_reencoded),
        ("left_answered", s.left_answered),
        ("transmissions", s.transmissions),
        ("headers_elided", s.headers_elided),
        ("silent_slots", s.silent_slots),
        ("forfeits", s.forfeits),
        ("dropped", s.dropped),
    ]
}

/// What the engine says of itself against what the model's said.
fn check_digest(e: &GrantedEngine, want: &Value, at: &str) {
    let mut wrong = Vec::new();
    let mut check = |name: &str, ok: bool, got: String| {
        if !ok {
            wrong.push(format!("{name}: {got}, the model {}", want[name]));
        }
    };
    check(
        "state",
        want["state"].as_str() == Some(state_name(e.state())),
        state_name(e.state()).to_owned(),
    );
    check(
        "role",
        want["role"].as_str() == Some(role_name(e.role())),
        role_name(e.role()).to_owned(),
    );
    check(
        "session",
        int(&want["session"]) == usize::from(e.session()),
        e.session().to_string(),
    );
    check(
        "level",
        int(&want["level"]) == e.level(),
        e.level().to_string(),
    );
    check(
        "current_mode",
        int(&want["current_mode"]) == e.current_mode(),
        e.current_mode().to_string(),
    );
    check(
        "tx_pending",
        int(&want["tx_pending"]) == e.tx_pending_bytes(),
        e.tx_pending_bytes().to_string(),
    );
    check(
        "tx_undelivered",
        int(&want["tx_undelivered"]) == e.tx_undelivered_bytes(),
        e.tx_undelivered_bytes().to_string(),
    );
    check(
        "next_deadline",
        close_opt(e.next_deadline(), opt(&want["next_deadline"])),
        format!("{:?}", e.next_deadline()),
    );
    check(
        "connect_tries",
        int(&want["connect_tries"]) == e.connect_tries(),
        e.connect_tries().to_string(),
    );
    check(
        "probing",
        want["probing"].as_bool() == Some(e.probing()),
        e.probing().to_string(),
    );
    check(
        "peer_snr_db",
        close_opt(e.peer_snr_db(), opt(&want["peer_snr_db"])),
        format!("{:?}", e.peer_snr_db()),
    );
    check(
        "identifier_owed",
        want["identifier_owed"].as_bool() == Some(e.identifier_owed()),
        e.identifier_owed().to_string(),
    );
    for (name, value) in stats(&e.stats) {
        if int(&want["stats"][name]) != value {
            wrong.push(format!(
                "stats.{name}: {value}, the model {}",
                want["stats"][name]
            ));
        }
    }
    assert!(wrong.is_empty(), "{at}: {}", wrong.join("; "));
}

#[allow(clippy::too_many_lines)]
fn replay_station(replay: &Replay<'_>, session: &str, doc: &Value) {
    let call = doc["call"].as_str().expect("call");
    let seed = doc["seed"].as_u64().expect("seed");
    let mut engine = GrantedEngine::new(call, timing(&doc["timing"]), config(&doc["config"]), seed);
    let mut said: Option<&Value> = None;
    for (step, entry) in doc["trace"].as_array().expect("trace").iter().enumerate() {
        let at = format!("{session}/{call} step {step}");
        if let Some(want) = entry.get("drain") {
            let got = engine.drain();
            replay.check_actions(&got, want.as_array().expect("actions"), &at);
            continue;
        }
        let name = entry["in"].as_str().expect("input");
        let at = format!("{at} ({name})");
        match name {
            "connect" => engine
                .connect(
                    entry["remote"].as_str().expect("remote"),
                    entry["as"].as_str(),
                )
                .expect("connect"),
            "probe" => engine
                .probe(
                    entry["remote"].as_str().expect("remote"),
                    entry["as"].as_str(),
                )
                .expect("probe"),
            "disconnect" => engine.disconnect(),
            "abort" => engine.abort(),
            "send" => engine.send(&replay.blob(&entry["data"])),
            "set_callsigns" => {
                let calls: Vec<&str> = entry["calls"]
                    .as_array()
                    .expect("calls")
                    .iter()
                    .map(|c| c.as_str().expect("call"))
                    .collect();
                engine.set_callsigns(&calls).expect("callsigns");
            }
            "set_identifier_s" => engine.set_identifier_s(num(&entry["seconds"])),
            "set_identifiers" => {
                let lengths: Vec<(String, f64)> = entry["lengths"]
                    .as_object()
                    .expect("lengths")
                    .iter()
                    .map(|(call, seconds)| (call.clone(), num(seconds)))
                    .collect();
                engine.set_identifiers(&lengths);
            }
            "set_answer_gap" => engine.set_answer_gap(num(&entry["seconds"])),
            "pin_mode" => engine
                .pin_mode(opt_int(&entry["mode"]), opt_int(&entry["body"]))
                .expect("pin"),
            "take_ladder" => {
                let rungs = engine.take_ladder();
                let want = entry["rungs"].as_array().expect("rungs");
                assert_eq!(rungs.len(), want.len(), "{at}: ladder {rungs:?}");
                for (rung, w) in rungs.iter().zip(want) {
                    assert!(
                        rung.mode == int(&w["mode"])
                            && rung.frames == int(&w["frames"])
                            && rung.decoded == int(&w["decoded"])
                            && close_opt(rung.snr_db, opt(&w["snr_db"])),
                        "{at}: rung {rung:?}, the model's {w}"
                    );
                }
            }
            "set_ceiling" => engine.set_ceiling(opt_int(&entry["rung"])),
            "set_max_burst_s" => engine.set_max_burst_s(opt(&entry["seconds"])),
            "set_max_mode" => engine.set_max_mode(int(&entry["rung"])),
            "tick" => engine.tick(num(&entry["now"])),
            "on_frame" => {
                let frame = replay.frame(&entry["frame"], &at);
                engine.on_frame(&frame, num(&entry["now"]));
                assert!(
                    frame.decodes.borrow().is_empty(),
                    "{at}: the port decoded the frame fewer times than the model"
                );
            }
            "on_preamble" => engine.on_preamble(
                num(&entry["t_start"]),
                num(&entry["now"]),
                opt(&entry["frame_s"]),
            ),
            "on_tx_done" => engine.on_tx_done(num(&entry["now"]), opt(&entry["frames_end"])),
            "on_tx_dropped" => engine.on_tx_dropped(num(&entry["now"])),
            "on_tx_cut_unread" => engine.on_tx_cut_unread(num(&entry["now"])),
            "on_tx_delayed" => engine.on_tx_delayed(num(&entry["seconds"])),
            other => panic!("{at}: unknown input {other}"),
        }
        if let Some(after) = entry.get("after") {
            said = Some(after);
        }
        if let Some(want) = said {
            check_digest(&engine, want, &at);
        }
    }
}

#[test]
fn the_port_does_what_the_model_does_input_by_input() {
    let doc = vectors();
    let blobs: Vec<Vec<u8>> = doc["blobs"]
        .as_array()
        .expect("blobs")
        .iter()
        .map(|b| from_hex(b.as_str().expect("hex")))
        .collect();
    let replay = Replay { blobs: &blobs };
    let sessions = doc["sessions"].as_array().expect("sessions");
    assert!(sessions.len() >= 10, "too few sessions to mean anything");
    for session in sessions {
        let name = session["name"].as_str().expect("name");
        for station in session["stations"].as_array().expect("stations") {
            replay_station(&replay, name, station);
        }
    }
}
