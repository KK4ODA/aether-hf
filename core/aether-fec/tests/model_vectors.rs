//! Cross-validation against the Python reference model (roadmap P3-1).
//!
//! ADR-0001 makes the model the specification and this crate the shipped implementation, and
//! requires them to agree bit-for-bit. `tools/make_fec_vectors.py` writes what the model
//! produces; this checks that the core produces exactly the same thing. A disagreement here
//! is a bug in the core, not a reason to regenerate the vectors — regenerate only when the
//! model has deliberately changed.

use std::{collections::HashMap, fs, path::PathBuf};

use aether_fec::{
    crc::{CRC6, CRC11, CRC16, CRC24A, CRC24B, CRC24C, Crc},
    ldpc::{FILLER_LLR, NrLdpcCode},
    rate_match::RateMatcher,
};
use serde_json::Value;

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/fec_vectors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}).\nGenerate it with: python tools/make_fec_vectors.py",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("vector file is not valid JSON")
}

/// Unpack hex-encoded, MSB-first bits back into one byte per bit.
fn unpack(hex: &str, len: usize) -> Vec<u8> {
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect();
    (0..len)
        .map(|i| (bytes[i / 8] >> (7 - i % 8)) & 1)
        .collect()
}

fn crc_by_name(name: &str) -> Crc {
    match name {
        "CRC24A" => CRC24A,
        "CRC24B" => CRC24B,
        "CRC24C" => CRC24C,
        "CRC16" => CRC16,
        "CRC11" => CRC11,
        "CRC6" => CRC6,
        other => panic!("unknown CRC {other}"),
    }
}

fn as_usize(value: &Value, key: &str) -> usize {
    value[key]
        .as_u64()
        .unwrap_or_else(|| panic!("missing {key}")) as usize
}

#[test]
fn crc_matches_the_model() {
    let doc = vectors();
    let cases = doc["crc"].as_array().expect("crc cases");
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["crc"].as_str().expect("crc name");
        let crc = crc_by_name(name);
        assert_eq!(crc.poly, as_usize(case, "poly") as u32, "{name} polynomial");
        assert_eq!(crc.width, as_usize(case, "width") as u32, "{name} width");

        let input_len = as_usize(case, "input_len");
        let input = unpack(case["input"].as_str().unwrap_or(""), input_len);
        let expected = unpack(
            case["remainder"].as_str().expect("remainder"),
            crc.width as usize,
        );

        assert_eq!(
            crc.remainder(&input),
            expected,
            "{name} remainder over {input_len} bits"
        );
        assert!(
            crc.check(&crc.attach(&input)),
            "{name} attach/check over {input_len} bits"
        );
    }
}

#[test]
fn ldpc_encoding_matches_the_model() {
    let doc = vectors();
    let cases = doc["ldpc"].as_array().expect("ldpc cases");
    assert!(!cases.is_empty());
    for case in cases {
        let bg = as_usize(case, "bg") as u8;
        let z = as_usize(case, "z");
        let code = NrLdpcCode::new(bg, z).expect("code construction");

        assert_eq!(code.k, as_usize(case, "k"), "bg{bg} z{z}: K");
        assert_eq!(code.n_full, as_usize(case, "n_full"), "bg{bg} z{z}: N");
        assert_eq!(code.n_cb, as_usize(case, "n_cb"), "bg{bg} z{z}: N_cb");

        let info = unpack(case["info"].as_str().expect("info"), code.k);
        let expected = unpack(case["codeword"].as_str().expect("codeword"), code.n_full);
        let produced = code.encode(&info).expect("encode");

        assert_eq!(
            produced, expected,
            "bg{bg} z{z}: codeword differs from the model"
        );
        assert!(
            code.syndrome_ok(&produced).expect("syndrome"),
            "bg{bg} z{z}"
        );
    }
}

#[test]
fn rate_matching_matches_the_model() {
    let doc = vectors();
    let cases = doc["rate_match"].as_array().expect("rate_match cases");
    assert!(!cases.is_empty());
    let mut codes: HashMap<(u8, usize), NrLdpcCode> = HashMap::new();
    for case in cases {
        let bg = as_usize(case, "bg") as u8;
        let z = as_usize(case, "z");
        let code = codes
            .entry((bg, z))
            .or_insert_with(|| NrLdpcCode::new(bg, z).expect("code construction"));

        let info_len = as_usize(case, "info_len");
        let e = as_usize(case, "e");
        let rv = as_usize(case, "rv") as u8;
        let label = format!("{} rv{rv}", case["mode_name"].as_str().unwrap_or("?"));

        let info = unpack(case["info"].as_str().expect("info"), code.k);
        let codeword = code.encode(&info).expect("encode");
        let matcher = RateMatcher::new(code, info_len, e, rv).expect("rate matcher");
        let produced = matcher.match_bits(&codeword).expect("match");
        let expected = unpack(case["matched"].as_str().expect("matched"), e);

        assert_eq!(produced.len(), e, "{label}: E");
        assert_eq!(
            produced, expected,
            "{label}: selected bits differ from the model"
        );
    }
}

#[test]
fn decoding_recovers_the_model_s_information_bits() {
    // Floating-point arithmetic is not required to be bit-identical across languages, so
    // what is pinned is the outcome, not the intermediate LLRs: from the same corrupted
    // input the core must recover exactly the information bits the model recovered.
    let doc = vectors();
    let cases = doc["decode"].as_array().expect("decode cases");
    assert!(!cases.is_empty());
    for case in cases {
        let bg = as_usize(case, "bg") as u8;
        let z = as_usize(case, "z");
        let info_len = as_usize(case, "info_len");
        let magnitude = case["magnitude"].as_f64().expect("magnitude");
        let code = NrLdpcCode::new(bg, z).expect("code construction");

        let info = unpack(case["info"].as_str().expect("info"), code.k);
        let codeword = unpack(case["codeword"].as_str().expect("codeword"), code.n_full);

        let mut llr: Vec<f64> = codeword
            .iter()
            .map(|&b| if b == 1 { -magnitude } else { magnitude })
            .collect();
        llr[info_len..code.k].fill(FILLER_LLR);
        for position in case["flipped"].as_array().expect("flipped") {
            let position = position.as_u64().expect("position") as usize;
            llr[position] = -llr[position];
        }

        let decoded = code.decode(&llr, 40, 0.8).expect("decode");
        assert_eq!(
            decoded.converged,
            case["converged"].as_bool().expect("converged"),
            "bg{bg} z{z}: convergence differs from the model"
        );
        if decoded.converged {
            assert_eq!(
                &decoded.bits[..info_len],
                &info[..info_len],
                "bg{bg} z{z}: recovered different information bits"
            );
        }
    }
}

#[test]
fn the_full_chain_round_trips_as_the_modem_uses_it() {
    // payload -> CRC -> fillers -> encode -> rate match -> (sign) -> recover -> decode
    let code = NrLdpcCode::new(2, 120).expect("code");
    let payload: Vec<u8> = (0..1000).map(|i| u8::from((i * 13) % 5 < 2)).collect();
    let with_crc = CRC24A.attach(&payload);
    let info_len = with_crc.len();
    let mut info = with_crc.clone();
    info.resize(code.k, 0);

    let codeword = code.encode(&info).expect("encode");
    let matcher = RateMatcher::new(&code, info_len, 2352, 0).expect("matcher");
    let on_air = matcher.match_bits(&codeword).expect("match");

    let llr: Vec<f64> = on_air
        .iter()
        .map(|&b| if b == 1 { -3.0 } else { 3.0 })
        .collect();
    let full = matcher.recover(&llr, None).expect("recover");
    let decoded = code.decode(&full, 25, 0.8).expect("decode");

    assert!(decoded.converged);
    assert_eq!(&decoded.bits[..info_len], &with_crc[..]);
    assert!(CRC24A.check(&decoded.bits[..info_len]));
}

#[test]
fn incremental_redundancy_decodes_what_one_transmission_cannot() {
    // The reason the redundancy version is carried outside the codeword: combining two
    // transmissions must decode a block that neither decodes alone.
    let code = NrLdpcCode::new(2, 120).expect("code");
    let info_len = 1176;
    let mut info: Vec<u8> = (0..info_len).map(|i| u8::from((i * 7) % 3 == 0)).collect();
    info.resize(code.k, 0);
    let codeword = code.encode(&info).expect("encode");

    // Corrupt one eighth of each transmission, in a different pattern per redundancy
    // version. That is well past what the rate-1/2 selection can correct on its own, and
    // comfortably inside what the combined rate-1/4 buffer can.
    let mut buffer: Option<Vec<f64>> = None;
    let mut decoded_alone = false;
    let mut decoded_combined = false;
    for rv in 0..2u8 {
        let matcher = RateMatcher::new(&code, info_len, 2352, rv).expect("matcher");
        let bits = matcher.match_bits(&codeword).expect("match");
        let llr: Vec<f64> = bits
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                let clean = if b == 1 { -2.0 } else { 2.0 };
                if (i * 7 + usize::from(rv) * 3) % 8 == 0 {
                    -clean
                } else {
                    clean
                }
            })
            .collect();

        let alone = matcher.recover(&llr, None).expect("recover");
        if rv == 0 && code.decode(&alone, 25, 0.8).expect("decode").converged {
            decoded_alone = true;
        }
        let combined = matcher.recover(&llr, buffer.as_deref()).expect("recover");
        let result = code.decode(&combined, 25, 0.8).expect("decode");
        if rv == 1 && result.converged && result.bits[..info_len] == info[..info_len] {
            decoded_combined = true;
        }
        buffer = Some(combined);
    }
    assert!(
        !decoded_alone,
        "the single-transmission case was not weak enough to be a test"
    );
    assert!(
        decoded_combined,
        "combining two redundancy versions failed to decode"
    );
}
