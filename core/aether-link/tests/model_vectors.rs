//! Cross-validation against the Python reference model (roadmap P3-2).
//!
//! Frame formats are wire formats, so everything about them is compared for equality: a byte
//! in the wrong place is a station nobody can talk to.
//!
//! The rate controller is compared as a *trace* — the same observations fed to both
//! implementations, recommendation by recommendation. It is a state machine over
//! floating-point comparisons, and an off-by-one in the hysteresis or a difference in when
//! the margin decays would show up here and nowhere else.
//!
//! Regenerate with `python tools/make_link_vectors.py`, and only for a deliberate model
//! change.

use std::{fs, path::PathBuf};

use aether_link::{
    datagram::{Reassembler, body, fragments, parse_body, read_fragment},
    frames::{
        ConnectBody, DataHeader, DataKind, GrantFrame, GrantKind, Piece, ProbeBody, decode_data,
        encode_data, level_of, pack_callsign, pieces, unpack_callsign,
    },
    rate::{
        AWGN_THRESHOLD_DB, FRAME_S, NARROW_AWGN_THRESHOLD_DB, NARROW_FRAME_S, NARROW_PAYLOAD_BYTES,
        PAYLOAD_BYTES, RateController, usable_modes, usable_modes_by_rate,
    },
};
use serde_json::Value;

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/link_vectors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}).\nGenerate it with: python tools/make_link_vectors.py",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("vector file is not valid JSON")
}

fn int(value: &Value, key: &str) -> usize {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("missing integer {key}")) as usize
}

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect()
}

fn data_kind(name: &str) -> DataKind {
    match name {
        "DATA" => DataKind::Data,
        "CONNECT_REQ" => DataKind::ConnectReq,
        "CONNECT_ACK" => DataKind::ConnectAck,
        "BEACON" => DataKind::Beacon,
        "PROBE" => DataKind::Probe,
        "PROBE_ACK" => DataKind::ProbeAck,
        "DATAGRAM" => DataKind::Datagram,
        "MEASURE" => DataKind::Measure,
        other => panic!("unknown data kind {other}"),
    }
}

#[test]
fn the_threshold_table_matches_the_model() {
    let doc = vectors();
    let expected: Vec<f64> = doc["awgn_thresholds"]
        .as_array()
        .expect("thresholds")
        .iter()
        .map(|v| v.as_f64().expect("threshold"))
        .collect();
    assert_eq!(AWGN_THRESHOLD_DB.len(), expected.len());
    for (index, (got, want)) in AWGN_THRESHOLD_DB.iter().zip(&expected).enumerate() {
        assert!((got - want).abs() < 1e-12, "mode {index}: {got} vs {want}");
    }
    let expected_modes: Vec<usize> = doc["usable_modes"]
        .as_array()
        .expect("usable modes")
        .iter()
        .map(|v| v.as_u64().expect("mode") as usize)
        .collect();
    assert_eq!(usable_modes(), expected_modes);
    // the wide ladder's rungs (ADR-0013): the tone floor's two, then the OFDM modes
    let payload: Vec<usize> = doc["payload_bytes"]
        .as_array()
        .expect("payloads")
        .iter()
        .map(|v| v.as_u64().expect("bytes") as usize)
        .collect();
    assert_eq!(PAYLOAD_BYTES.to_vec(), payload);
    let frame_s: Vec<f64> = doc["frame_s"]
        .as_array()
        .expect("frame air times")
        .iter()
        .map(|v| v.as_f64().expect("seconds"))
        .collect();
    assert_eq!(FRAME_S.len(), frame_s.len());
    for (got, want) in FRAME_S.iter().zip(&frame_s) {
        assert!((got - want).abs() < 1e-9, "frame air time {got} vs {want}");
    }
}

#[test]
fn the_pipes_control_thresholds_match_the_model() {
    use aether_link::rate::{CONTROL_THRESHOLD_DB, NARROW_CONTROL_THRESHOLD_DB};
    let doc = vectors();
    for (key, table) in [
        ("control_thresholds", CONTROL_THRESHOLD_DB),
        ("narrow_control_thresholds", NARROW_CONTROL_THRESHOLD_DB),
    ] {
        let expected: Vec<f64> = doc[key]
            .as_array()
            .expect("control thresholds")
            .iter()
            .map(|v| v.as_f64().expect("threshold"))
            .collect();
        assert_eq!(expected.len(), 2, "{key}");
        for (family, (got, want)) in table.iter().zip(&expected).enumerate() {
            assert!(
                (got - want).abs() < 1e-12,
                "{key}[{family}]: {got} vs {want}"
            );
        }
    }
}

#[test]
fn the_narrow_threshold_table_matches_the_model() {
    let doc = vectors();
    let expected: Vec<f64> = doc["narrow_awgn_thresholds"]
        .as_array()
        .expect("narrow thresholds")
        .iter()
        .map(|v| v.as_f64().expect("threshold"))
        .collect();
    assert_eq!(NARROW_AWGN_THRESHOLD_DB.len(), expected.len());
    for (index, (got, want)) in NARROW_AWGN_THRESHOLD_DB.iter().zip(&expected).enumerate() {
        assert!(
            (got - want).abs() < 1e-12,
            "narrow mode {index}: {got} vs {want}"
        );
    }
    let payload: Vec<usize> = doc["narrow_payload_bytes"]
        .as_array()
        .expect("narrow payloads")
        .iter()
        .map(|v| v.as_u64().expect("bytes") as usize)
        .collect();
    assert_eq!(NARROW_PAYLOAD_BYTES.to_vec(), payload);
    let frame_s: Vec<f64> = doc["narrow_frame_s"]
        .as_array()
        .expect("narrow frame air times")
        .iter()
        .map(|v| v.as_f64().expect("seconds"))
        .collect();
    assert_eq!(NARROW_FRAME_S.len(), frame_s.len());
    for (got, want) in NARROW_FRAME_S.iter().zip(&frame_s) {
        assert!(
            (got - want).abs() < 1e-9,
            "narrow frame air time {got} vs {want}"
        );
    }
    let expected_modes: Vec<usize> = doc["narrow_usable_modes"]
        .as_array()
        .expect("narrow usable modes")
        .iter()
        .map(|v| v.as_u64().expect("mode") as usize)
        .collect();
    assert_eq!(
        usable_modes_by_rate(
            &NARROW_AWGN_THRESHOLD_DB,
            &NARROW_PAYLOAD_BYTES,
            &NARROW_FRAME_S
        ),
        expected_modes
    );
}

#[test]
fn callsign_packing_matches_the_model() {
    for case in vectors()["callsigns"].as_array().expect("callsigns") {
        let call = case["call"].as_str().expect("call");
        let expected = from_hex(case["packed"].as_str().expect("packed"));
        let packed = pack_callsign(call).expect("pack");
        assert_eq!(packed.to_vec(), expected, "{call}");
        assert_eq!(
            unpack_callsign(&packed).expect("unpack"),
            call,
            "{call} round trip"
        );
    }
}

#[test]
fn data_frame_encoding_matches_the_model() {
    for case in vectors()["data_frames"].as_array().expect("data frames") {
        let kind = data_kind(case["kind"].as_str().expect("kind"));
        let header = DataHeader {
            kind,
            seq: int(case, "seq") as u8,
            session: int(case, "session") as u8,
            piece: None,
        };
        let body = from_hex(case["body"].as_str().unwrap_or(""));
        let capacity = int(case, "capacity");
        let expected = from_hex(case["encoded"].as_str().expect("encoded"));
        let label = format!("{:?} cap={capacity} body={}", kind, body.len());

        let encoded = encode_data(&header, &body, capacity).expect("encode");
        assert_eq!(encoded.len(), capacity, "{label}: length");
        assert_eq!(encoded, expected, "{label}: bytes differ from the model");

        let (got_header, got_body) = decode_data(&encoded).expect("decode");
        assert_eq!(got_header, header, "{label}: header round trip");
        assert_eq!(got_body, body, "{label}: body round trip");
    }
}

#[test]
fn pieces_match_the_model() {
    // a stuck frame's body in pieces (ADR-0086): the same cuts, the same frames
    for case in vectors()["pieces"].as_array().expect("pieces") {
        let capacity = int(case, "capacity");
        let body = from_hex(case["body"].as_str().expect("body"));
        let expected = case["pieces"].as_array().expect("its pieces");
        let label = format!("cap={capacity} body={}", body.len());
        let got = pieces(&body, capacity).expect("pieces");
        assert_eq!(got.len(), expected.len(), "{label}: how many pieces");
        for ((piece, bytes), want) in got.iter().zip(expected) {
            assert_eq!(
                usize::from(piece.offset),
                int(want, "offset"),
                "{label}: offset"
            );
            assert_eq!(
                piece.last,
                want["last"].as_bool().expect("last"),
                "{label}: last"
            );
            assert_eq!(
                *bytes,
                from_hex(want["piece"].as_str().unwrap_or("")),
                "{label}: bytes"
            );
            let header = DataHeader {
                kind: DataKind::Data,
                seq: int(case, "seq") as u8,
                session: int(case, "session") as u8,
                piece: Some(Piece {
                    offset: piece.offset,
                    last: piece.last,
                }),
            };
            let encoded = encode_data(&header, bytes, capacity).expect("encode");
            let frame = from_hex(want["encoded"].as_str().expect("encoded"));
            assert_eq!(encoded, frame, "{label}: frame at {}", piece.offset);
            assert_eq!(
                decode_data(&frame).expect("decode"),
                (header, bytes.clone())
            );
        }
    }
}

#[test]
fn connect_bodies_match_the_model() {
    for case in vectors()["connect_bodies"]
        .as_array()
        .expect("connect bodies")
    {
        let body = ConnectBody {
            src: case["src"].as_str().expect("src").to_owned(),
            dst: case["dst"].as_str().expect("dst").to_owned(),
            snr_db: case["snr_db"].as_f64(),
            caps: int(case, "caps") as u8,
            version: int(case, "version") as u8,
            turnaround_s: case["turnaround_s"].as_f64().unwrap_or(0.0),
            identifier_s: case["identifier_s"].as_f64().unwrap_or(0.0),
        };
        let expected = from_hex(case["encoded"].as_str().expect("encoded"));
        assert_eq!(
            body.encode().expect("encode"),
            expected,
            "{} -> {}",
            body.src,
            body.dst
        );
        let decoded = ConnectBody::decode(&expected).expect("decode");
        assert_eq!(
            (&decoded.src, &decoded.dst, decoded.caps, decoded.version),
            (&body.src, &body.dst, body.caps, body.version)
        );
        // the SNR is quantised to a signed byte, so compare against what the model read back
        match (decoded.snr_db, case["decoded_snr_db"].as_f64()) {
            (None, None) => {}
            (Some(got), Some(want)) => assert!((got - want).abs() < 1e-12, "SNR {got} vs {want}"),
            (got, want) => panic!("SNR presence differs, {got:?} vs {want:?}"),
        }
        // protocol 8's turnaround and identifier, in their steps (ADR-0069)
        let turnaround = case["decoded_turnaround_s"].as_f64().expect("turnaround");
        let identifier = case["decoded_identifier_s"].as_f64().expect("identifier");
        assert!(
            (decoded.turnaround_s - turnaround).abs() < 1e-12,
            "turnaround {} vs {turnaround}",
            decoded.turnaround_s
        );
        assert!(
            (decoded.identifier_s - identifier).abs() < 1e-12,
            "identifier {} vs {identifier}",
            decoded.identifier_s
        );
        assert_eq!(usize::from(level_of(body.caps)), int(case, "level"));
    }
}

fn grant_kind(name: &str) -> GrantKind {
    match name {
        "HEADER" => GrantKind::Header,
        "ANSWER" => GrantKind::Answer,
        "DISC" => GrantKind::Disc,
        "DISC_ACK" => GrantKind::DiscAck,
        other => panic!("unknown grant kind {other}"),
    }
}

fn grant_frame(case: &Value) -> GrantFrame {
    GrantFrame {
        kind: grant_kind(case["kind"].as_str().expect("kind")),
        session: int(case, "session") as u8,
        count: int(case, "count") as u8,
        data_floor: case["data_floor"].as_bool().expect("data_floor"),
        level: int(case, "level") as u8,
        fam: case["fam"].as_bool().expect("fam"),
        flag: case["flag"].as_bool().expect("flag"),
        ident: case["ident"].as_bool().expect("ident"),
        measure: case["measure"].as_bool().expect("measure"),
        base: int(case, "base") as u8,
        bitmap: int(case, "bitmap") as u16,
        snr_db: case["snr_db"].as_f64(),
        rung: int(case, "rung") as u8,
        n: int(case, "n") as u8,
    }
}

#[test]
fn grant_frames_match_the_model() {
    for case in vectors()["grant_frames"].as_array().expect("grant frames") {
        let frame = grant_frame(case);
        let expected = from_hex(case["encoded"].as_str().expect("encoded"));
        assert_eq!(
            frame.encode().to_vec(),
            expected,
            "{frame:?}: bytes differ from the model"
        );
        let decoded = GrantFrame::decode(&expected).expect("decode");
        assert_eq!(decoded, grant_frame(&case["decoded"]), "{frame:?}: decoded");
        let received = case["received"].as_array().expect("received");
        let sweep = 256usize.div_ceil(13);
        for (index, want) in received.iter().enumerate() {
            let seq = if index < sweep {
                (index * 13) as u8
            } else {
                decoded.base.wrapping_add((index - sweep) as u8)
            };
            assert_eq!(
                decoded.received(seq),
                want.as_bool().expect("flag"),
                "received({seq})"
            );
        }
        assert_eq!(
            decoded.reports(),
            case["reports"].as_bool().expect("reports"),
            "{frame:?}: reports"
        );
        for (index, want) in case["measured"]
            .as_array()
            .expect("measured")
            .iter()
            .enumerate()
        {
            // the model's indices run from -1, the port's from 0
            if index == 0 {
                assert!(!want.as_bool().expect("flag"), "measured(-1)");
                continue;
            }
            assert_eq!(
                decoded.measured(index - 1),
                want.as_bool().expect("flag"),
                "measured({})",
                index - 1
            );
        }
    }
}

#[test]
fn datagram_fragments_match_the_model() {
    for case in vectors()["datagrams"].as_array().expect("datagrams") {
        let capacity = int(case, "capacity");
        let number = u8::try_from(int(case, "number")).expect("a number");
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let expected: Vec<Vec<u8>> = case["fragments"]
            .as_array()
            .expect("fragments")
            .iter()
            .map(|f| from_hex(f.as_str().expect("hex")))
            .collect();
        let label = format!("cap={capacity} size={}", payload.len());
        let made = fragments(&payload, number, capacity).expect("fits");
        assert_eq!(made, expected, "{label}: fragments differ from the model");
        let mut joiner = Reassembler::default();
        let joined = made
            .iter()
            .filter_map(|f| joiner.add(read_fragment(f).expect("a fragment"), 0.0))
            .next_back();
        assert_eq!(joined, Some(payload), "{label}: joined");
    }
}

#[test]
fn datagram_bodies_match_the_model() {
    for case in vectors()["datagram_bodies"]
        .as_array()
        .expect("datagram bodies")
    {
        let source = case["source"].as_str().expect("source");
        let frame_type = u8::try_from(int(case, "frame_type")).expect("a type");
        let frame = from_hex(case["frame"].as_str().expect("frame"));
        let expected = from_hex(case["body"].as_str().expect("body"));
        let made = body(source, frame_type, &frame).expect("a body");
        assert_eq!(made, expected, "{source}: body differs from the model");
        assert_eq!(
            parse_body(&made),
            Some((source.to_owned(), frame_type, frame)),
            "{source}: round trip"
        );
    }
}

#[test]
fn probe_bodies_match_the_model() {
    for case in vectors()["probe_bodies"].as_array().expect("probe bodies") {
        let body = ProbeBody {
            src: case["src"].as_str().expect("src").to_owned(),
            dst: case["dst"].as_str().expect("dst").to_owned(),
            snr_db: case["snr_db"].as_f64(),
            caps: int(case, "caps") as u8,
        };
        let expected = from_hex(case["encoded"].as_str().expect("encoded"));
        let label = format!("{} > {} at {:?}", body.src, body.dst, body.snr_db);
        assert_eq!(body.encode().expect("encode"), expected, "{label}");
        let decoded = ProbeBody::decode(&expected).expect("decode");
        assert_eq!(
            (&decoded.src, &decoded.dst, decoded.caps),
            (&body.src, &body.dst, body.caps)
        );
        // the SNR is quantised to a signed byte, so compare against what the model read back
        match (decoded.snr_db, case["decoded_snr_db"].as_f64()) {
            (None, None) => {}
            (Some(got), Some(want)) => {
                assert!((got - want).abs() < 1e-12, "{label}: SNR {got} vs {want}");
            }
            (got, want) => panic!("{label}: SNR presence differs, {got:?} vs {want:?}"),
        }
    }
}

#[test]
fn the_rate_controller_follows_the_same_trace_as_the_model() {
    for case in vectors()["rate_traces"].as_array().expect("rate traces") {
        let name = case["name"].as_str().expect("name");
        let observations = case["observations"].as_array().expect("observations");
        let track = case["track"].as_array().expect("track");
        assert_eq!(observations.len(), track.len(), "{name}");

        let mut rc = RateController::default();
        for (step, (observation, expected)) in observations.iter().zip(track).enumerate() {
            let snr = observation["snr_db"].as_f64();
            let ok = observation["ok"].as_u64().expect("ok") as usize;
            let failed = observation["failed"].as_u64().expect("failed") as usize;
            let mode = observation["mode"].as_u64().map(|m| m as usize);
            if observation["snr_only"].as_bool() == Some(true) {
                rc.observe_snr(snr);
            } else {
                rc.observe(snr, ok, failed, mode);
            }

            let want_mode = expected["recommend"].as_u64().expect("recommend") as usize;
            assert_eq!(
                rc.recommend(),
                want_mode,
                "{name} step {step}: recommendation"
            );

            let want_margin = expected["margin_db"].as_f64().expect("margin");
            assert!(
                (rc.margin_db() - want_margin).abs() < 1e-9,
                "{name} step {step}: margin {} vs {want_margin}",
                rc.margin_db()
            );
            match (rc.snr_db(), expected["snr_db"].as_f64()) {
                (None, None) => {}
                (Some(got), Some(want)) => assert!(
                    (got - want).abs() < 1e-9,
                    "{name} step {step}: smoothed SNR {got} vs {want}"
                ),
                (got, want) => {
                    panic!("{name} step {step}: SNR presence differs, {got:?} vs {want:?}")
                }
            }
        }
    }
}

#[test]
fn the_echo_limits_match_the_model() {
    // ADR-0087: the echo share above which each rung is kept off the ladder, by mode
    use aether_link::rate::{ECHO_LIMIT, NARROW_ECHO_LIMIT};
    let doc = vectors();
    for (key, table) in [
        ("echo_limits", ECHO_LIMIT.as_slice()),
        ("narrow_echo_limits", NARROW_ECHO_LIMIT.as_slice()),
    ] {
        let want: Vec<Option<f64>> = doc[key]
            .as_array()
            .expect("echo limits")
            .iter()
            .map(Value::as_f64)
            .collect();
        assert_eq!(table, want.as_slice(), "{key}");
    }
}

#[test]
fn the_rate_controller_reads_the_echo_as_the_model_does() {
    // ADR-0087: the 500 Hz controller with its echo limits, the same seeds, readings and
    // bursts on both sides — the recommendation, the margin, the SNR and the echo after each
    use aether_link::rate::{NARROW_ECHO_LIMIT, RateConfig};
    for case in vectors()["echo_traces"].as_array().expect("echo traces") {
        let name = case["name"].as_str().expect("name");
        let steps = case["steps"].as_array().expect("steps");
        let track = case["track"].as_array().expect("track");
        assert_eq!(steps.len(), track.len(), "{name}");
        let mut rc = RateController::for_table_timed(
            RateConfig::default(),
            &NARROW_AWGN_THRESHOLD_DB,
            &NARROW_PAYLOAD_BYTES,
            &NARROW_FRAME_S,
        )
        .with_floor(5, None)
        .with_echo_limit(&NARROW_ECHO_LIMIT);
        for (index, (step, want)) in steps.iter().zip(track).enumerate() {
            let args = step["args"].as_array().expect("args");
            match step["what"].as_str().expect("what") {
                "echo" => rc.observe_echo(args[0].as_f64(), args[1].as_f64().expect("snr")),
                "seed" => rc.seed(
                    args[0].as_f64().expect("snr"),
                    args[1].as_bool().expect("lower bound"),
                ),
                _ => rc.observe(
                    args[0].as_f64(),
                    args[1].as_u64().expect("ok") as usize,
                    args[2].as_u64().expect("failed") as usize,
                    args[3].as_u64().map(|m| m as usize),
                ),
            }
            let at = format!("{name} step {index}");
            assert_eq!(
                rc.recommend() as u64,
                want["recommend"].as_u64().expect("recommend"),
                "{at}: recommendation"
            );
            let margin = want["margin_db"].as_f64().expect("margin");
            assert!((rc.margin_db() - margin).abs() < 1e-9, "{at}: margin");
            match (rc.snr_db(), want["snr_db"].as_f64()) {
                (None, None) => {}
                (Some(got), Some(w)) => assert!((got - w).abs() < 1e-9, "{at}: SNR {got}"),
                (got, w) => panic!("{at}: SNR {got:?} vs {w:?}"),
            }
            match (rc.echo(), want["echo"].as_f64()) {
                (None, None) => {}
                (Some(got), Some(w)) => assert!((got - w).abs() < 1e-11, "{at}: echo {got}"),
                (got, w) => panic!("{at}: echo {got:?} vs {w:?}"),
            }
        }
    }
}
