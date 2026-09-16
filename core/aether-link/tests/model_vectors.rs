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
    frames::{
        ConnectBody, ControlFrame, ControlKind, DataHeader, DataKind, ProbeBody, decode_data,
        encode_data, pack_callsign, unpack_callsign,
    },
    rate::{
        AWGN_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB, NARROW_FRAME_S, NARROW_PAYLOAD_BYTES,
        RateController, usable_modes, usable_modes_by_rate,
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
        other => panic!("unknown data kind {other}"),
    }
}

fn control_kind(name: &str) -> ControlKind {
    match name {
        "ACK" => ControlKind::Ack,
        "POLL" => ControlKind::Poll,
        "TURN" => ControlKind::Turn,
        "DISC" => ControlKind::Disc,
        "DISC_ACK" => ControlKind::DiscAck,
        other => panic!("unknown control kind {other}"),
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
fn control_frames_match_the_model() {
    for case in vectors()["control_frames"]
        .as_array()
        .expect("control frames")
    {
        let kind = control_kind(case["kind"].as_str().expect("kind"));
        let frame = ControlFrame {
            kind,
            session: int(case, "session") as u8,
            flags: int(case, "flags") as u8,
            base: int(case, "base") as u8,
            bitmap: int(case, "bitmap") as u16,
            snr_db: case["snr_db"].as_f64(),
            recommended_mode: int(case, "recommended_mode") as u8,
            counter: int(case, "counter") as u8,
        };
        let expected = from_hex(case["encoded"].as_str().expect("encoded"));
        assert_eq!(
            frame.encode().to_vec(),
            expected,
            "{kind:?}: bytes differ from the model"
        );

        let decoded = ControlFrame::decode(&expected).expect("decode");
        // the SNR is quantised to a signed byte, so compare against what the model read back
        match (decoded.snr_db, case["decoded_snr_db"].as_f64()) {
            (None, None) => {}
            (Some(got), Some(want)) => {
                assert!((got - want).abs() < 1e-12, "{kind:?}: SNR {got} vs {want}");
            }
            (got, want) => panic!("{kind:?}: SNR presence differs, {got:?} vs {want:?}"),
        }

        let expected_received: Vec<bool> = case["received"]
            .as_array()
            .expect("received")
            .iter()
            .map(|v| v.as_bool().expect("flag"))
            .collect();
        for (index, want) in expected_received.iter().enumerate() {
            let seq = (index * 17) as u8;
            assert_eq!(decoded.received(seq), *want, "{kind:?}: received({seq})");
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
            rc.observe(snr, ok, failed, mode);

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
