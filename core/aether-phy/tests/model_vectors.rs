//! Cross-validation against the Python reference model (roadmap P3-2).
//!
//! ADR-0001 makes the model the specification. What "agrees with it" means differs by layer,
//! and these tests are explicit about which is which:
//!
//! * **exact** — waveform numerology, frame layouts, the mode table, constellation points
//!   and bit labels, the interleaver permutation, and the coded bits a payload produces.
//!   Any difference is a bug in the core.
//! * **to a tolerance** — LLRs, which are floating-point evaluations of the same formula.
//!   Requiring identical doubles from two implementations would be testing the optimiser,
//!   not the modem; the tolerance used is carried in the vector file.
//!
//! Regenerate with `python tools/make_phy_vectors.py`, and only for a deliberate model
//! change.

use std::{fs, path::PathBuf};

use aether_phy::{
    codec::{FrameCodec, coprime_stride},
    constellation::{Constellation, NoiseVar},
    modes::{CONTROL_MODE, LONG, MODES, SHORT},
    ofdm::OfdmDemodulator,
    preamble::{FrameHeader, FrameType},
    tx::FrameTransmitter,
    waveform::{Modulation, WIDE_2300},
};
use serde_json::Value;

fn vectors() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/phy_vectors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}).\nGenerate it with: python tools/make_phy_vectors.py",
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

fn float(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("missing float {key}"))
}

fn unpack(hex: &str, len: usize) -> Vec<u8> {
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect();
    (0..len)
        .map(|i| (bytes[i / 8] >> (7 - i % 8)) & 1)
        .collect()
}

fn from_hex(hex: &str) -> Vec<u8> {
    (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect()
}

fn modulation_by_name(name: &str) -> Modulation {
    match name {
        "BPSK" => Modulation::Bpsk,
        "QPSK" => Modulation::Qpsk,
        "PSK8" => Modulation::Psk8,
        "QAM16" => Modulation::Qam16,
        "QAM64" => Modulation::Qam64,
        other => panic!("unknown modulation {other}"),
    }
}

#[test]
fn waveform_numerology_matches_the_model() {
    let doc = vectors();
    let w = &doc["waveform"];
    let p = WIDE_2300;
    assert_eq!(p.fft_size, int(w, "fft_size"));
    assert_eq!(p.cp_samples, int(w, "cp_samples"));
    assert_eq!(p.taper_samples, int(w, "taper_samples"));
    assert_eq!(p.audio_rate, int(w, "audio_rate"));
    assert_eq!(p.symbol_samples(), int(w, "symbol_samples"));
    assert_eq!(p.n_carriers(), int(w, "n_carriers"));
    assert_eq!(p.n_pilot_carriers(), int(w, "n_pilot_carriers"));
    assert_eq!(p.n_data_carriers(), int(w, "n_data_carriers"));
    assert_eq!(p.resample_factor(), int(w, "resample_factor"));

    for (name, got) in [
        ("fs_baseband", p.fs_baseband),
        ("centre_hz", p.centre_hz),
        ("subcarrier_spacing_hz", p.subcarrier_spacing_hz()),
        ("useful_symbol_s", p.useful_symbol_s()),
        ("symbol_period_s", p.symbol_period_s()),
        ("symbol_rate_bd", p.symbol_rate_bd()),
        ("effective_cp_s", p.effective_cp_s()),
        ("occupied_bandwidth_hz", p.occupied_bandwidth_hz()),
    ] {
        let expected = float(w, name);
        assert!(
            (got - expected).abs() <= 1e-12 * expected.abs().max(1.0),
            "{name}: {got} vs {expected}"
        );
    }
}

#[test]
fn frame_layouts_match_the_model() {
    let doc = vectors();
    for case in doc["layouts"].as_array().expect("layouts") {
        let name = case["name"].as_str().expect("name");
        let layout = match name {
            "long" => LONG,
            "short" => SHORT,
            other => panic!("unknown layout {other}"),
        };
        assert_eq!(layout.data_symbols, int(case, "data_symbols"), "{name}");
        assert_eq!(layout.total_symbols(), int(case, "total_symbols"), "{name}");
        assert_eq!(layout.samples(), int(case, "samples"), "{name}");
        assert_eq!(layout.qam_symbols(), int(case, "qam_symbols"), "{name}");
        assert_eq!(
            layout.n_payload_symbols(),
            int(case, "n_payload_symbols"),
            "{name}"
        );
        let expected: Vec<usize> = case["pilot_symbol_indices"]
            .as_array()
            .expect("pilot indices")
            .iter()
            .map(|v| v.as_u64().expect("index") as usize)
            .collect();
        assert_eq!(layout.pilot_symbol_indices(), expected, "{name}");
        assert!(
            (layout.duration_s() - float(case, "duration_s")).abs() < 1e-12,
            "{name}"
        );
    }
}

#[test]
fn the_mode_table_matches_the_model() {
    let doc = vectors();
    let cases = doc["modes"].as_array().expect("modes");
    assert_eq!(cases.len(), MODES.len());
    for (case, mode) in cases.iter().zip(&MODES) {
        let name = case["name"].as_str().expect("name");
        assert_eq!(mode.index, int(case, "index"), "{name}");
        assert_eq!(mode.name(), name);
        assert_eq!(
            mode.modulation.bits_per_symbol(),
            int(case, "bits_per_symbol"),
            "{name}"
        );
        assert_eq!(mode.rate_num, int(case, "rate_num"), "{name}");
        assert_eq!(mode.rate_den, int(case, "rate_den"), "{name}");
        assert_eq!(mode.coded_bits(&LONG), int(case, "coded_bits"), "{name}");
        assert_eq!(mode.info_bits(&LONG), int(case, "info_bits"), "{name}");
        assert_eq!(
            mode.payload_bytes(&LONG),
            int(case, "payload_bytes"),
            "{name}"
        );
        assert_eq!(
            usize::from(mode.base_graph(&LONG)),
            int(case, "base_graph"),
            "{name}"
        );
        assert_eq!(
            mode.lifting_size(&LONG),
            int(case, "lifting_size"),
            "{name}"
        );
        assert!(
            (mode.net_bit_rate(&LONG) - float(case, "net_bit_rate")).abs() < 1e-9,
            "{name}: net bit rate"
        );
    }
}

#[test]
fn constellations_match_the_model() {
    let doc = vectors();
    let symbol_tolerance = doc["symbol_tolerance"].as_f64().expect("symbol_tolerance");
    let llr_tolerance = doc["llr_tolerance"].as_f64().expect("llr_tolerance");

    for case in doc["constellations"].as_array().expect("constellations") {
        let name = case["modulation"].as_str().expect("modulation");
        let modulation = modulation_by_name(name);
        let c = Constellation::new(modulation);

        // points, in label order — exact up to the tolerance of a square root
        let expected_points = case["points"].as_array().expect("points");
        assert_eq!(
            c.points().len(),
            expected_points.len(),
            "{name}: point count"
        );
        for (index, (got, want)) in c.points().iter().zip(expected_points).enumerate() {
            let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
            assert!(
                (got.0 - wr).abs() < symbol_tolerance && (got.1 - wi).abs() < symbol_tolerance,
                "{name} point {index}: ({}, {}) vs ({wr}, {wi})",
                got.0,
                got.1
            );
        }
        assert!(
            (c.min_distance() - float(case, "min_distance")).abs() < 1e-12,
            "{name}: minimum distance"
        );

        // mapping is exact: the same bits must give the same points
        let bits: Vec<u8> = case["bits"]
            .as_array()
            .expect("bits")
            .iter()
            .map(|v| v.as_u64().expect("bit") as u8)
            .collect();
        let symbols = c.map(&bits);
        let expected_symbols = case["symbols"].as_array().expect("symbols");
        assert_eq!(
            symbols.len(),
            expected_symbols.len(),
            "{name}: symbol count"
        );
        for (index, (got, want)) in symbols.iter().zip(expected_symbols).enumerate() {
            let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
            assert!(
                (got.0 - wr).abs() < symbol_tolerance && (got.1 - wi).abs() < symbol_tolerance,
                "{name} symbol {index}"
            );
        }
        // hard decisions recover the bits exactly
        assert_eq!(c.hard(&symbols), bits, "{name}: hard decisions");

        // LLRs to a tolerance
        let noise = float(case, "noise_var");
        let got_llr = c.llr(&symbols, NoiseVar::Uniform(noise));
        let expected_llr = case["llr"].as_array().expect("llr");
        assert_eq!(got_llr.len(), expected_llr.len(), "{name}: LLR count");
        for (index, (got, want)) in got_llr.iter().zip(expected_llr).enumerate() {
            let want = want.as_f64().expect("llr value");
            assert!(
                (got - want).abs() <= llr_tolerance * want.abs().max(1.0),
                "{name} LLR {index}: {got} vs {want}"
            );
        }
    }
}

#[test]
fn the_interleaver_matches_the_model() {
    let doc = vectors();
    for case in doc["interleaver"].as_array().expect("interleaver") {
        let e = int(case, "e");
        let stride = int(case, "stride");
        assert_eq!(coprime_stride(e), stride, "stride for E={e}");
        let head: Vec<usize> = case["permutation_head"]
            .as_array()
            .expect("permutation head")
            .iter()
            .map(|v| v.as_u64().expect("position") as usize)
            .collect();
        for (k, &expected) in head.iter().enumerate() {
            assert_eq!((k * stride) % e, expected, "E={e}, k={k}");
        }
    }
}

#[test]
fn coded_bits_match_the_model_for_every_mode_and_redundancy_version() {
    // The strongest exact check in this file: it exercises CRC, LDPC encoding, rate matching
    // and interleaving together, for all fourteen modes and all four redundancy versions.
    let doc = vectors();
    let cases = doc["codec"].as_array().expect("codec cases");
    assert_eq!(
        cases.len(),
        MODES.len() * 4 + 1,
        "expected every mode at every RV, plus control"
    );

    for case in cases {
        let label = format!(
            "{} rv{} ({})",
            case["mode_name"].as_str().unwrap_or("?"),
            int(case, "rv"),
            case["layout"].as_str().unwrap_or("?")
        );
        let (mode, layout) = match case["layout"].as_str().expect("layout") {
            "long" => (MODES[int(case, "mode")], LONG),
            "short" => (CONTROL_MODE, SHORT),
            other => panic!("unknown layout {other}"),
        };
        let codec = FrameCodec::new(mode, layout).expect("codec");
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let rv = int(case, "rv") as u8;

        let produced = codec.encode_bits(&payload, rv).expect("encode");
        let expected = unpack(
            case["interleaved_bits"].as_str().expect("bits"),
            int(case, "coded_bits"),
        );
        assert_eq!(
            produced.len(),
            int(case, "coded_bits"),
            "{label}: coded bit count"
        );
        assert_eq!(
            produced, expected,
            "{label}: coded bits differ from the model"
        );

        let symbols = codec.encode(&payload, rv).expect("encode");
        assert_eq!(
            symbols.len(),
            int(case, "n_symbols"),
            "{label}: symbol count"
        );

        // and the first few symbols, as a direct check that mapping did not drift
        for (index, want) in case["symbols_head"]
            .as_array()
            .expect("head")
            .iter()
            .enumerate()
        {
            let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
            assert!(
                (symbols[index].0 - wr).abs() < 1e-12 && (symbols[index].1 - wi).abs() < 1e-12,
                "{label}: symbol {index}"
            );
        }
    }
}

#[test]
fn a_frame_survives_the_full_round_trip_for_every_mode() {
    for &mode in &MODES {
        let codec = FrameCodec::new(mode, LONG).expect("codec");
        let payload: Vec<u8> = (0..codec.payload_bytes)
            .map(|i| ((i * 61) % 256) as u8)
            .collect();
        let symbols = codec.encode(&payload, 0).expect("encode");
        let (decoded, _) = codec
            .decode(&symbols, NoiseVar::Uniform(0.004), 0, None)
            .expect("decode");
        assert_eq!(decoded.as_deref(), Some(&payload[..]), "{}", mode.name());
    }
}

#[test]
fn whole_frames_match_the_model() {
    // The strongest check of the physical layer: build the same frame the model builds and
    // compare it at the carrier level, which exercises the preamble, pilot placement, the
    // mode and RV chips, the transform scaling and the windowing together. A strided set of
    // time samples goes with it, so a scaling or windowing error that happened to cancel at
    // the carriers would still be caught.
    //
    // ADR-0004 peak reduction is off on both sides; the Rust transmitter does not implement
    // it yet, and comparing against a model that does would compare two different waveforms.
    let doc = vectors();
    let cases = doc["waveform_frames"].as_array().expect("waveform frames");
    assert!(!cases.is_empty());

    let tx = FrameTransmitter::default();
    let demodulator = OfdmDemodulator::new(WIDE_2300);
    let period = WIDE_2300.symbol_samples();

    for case in cases {
        let label = format!(
            "{} rv{} ({})",
            case["mode_name"].as_str().unwrap_or("?"),
            int(case, "rv"),
            case["layout"].as_str().unwrap_or("?")
        );
        let (mode, layout) = match case["layout"].as_str().expect("layout") {
            "long" => (MODES[int(case, "mode")], LONG),
            "short" => (CONTROL_MODE, SHORT),
            other => panic!("unknown layout {other}"),
        };
        let rv = int(case, "rv") as u8;
        let header = match case["frame_type"].as_str().expect("frame type") {
            "DATA" => FrameHeader::new(FrameType::Data, mode.index, rv).expect("header"),
            "CONTROL" => FrameHeader::control(),
            other => panic!("unknown frame type {other}"),
        };

        let codec = FrameCodec::new(mode, layout).expect("codec");
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let qam = codec.encode(&payload, rv).expect("encode");
        let waveform = tx.baseband(&header, &layout, &qam).expect("baseband");

        assert_eq!(
            waveform.len(),
            int(case, "n_samples"),
            "{label}: sample count"
        );

        let power: f64 = waveform
            .iter()
            .map(|&(re, im)| re * re + im * im)
            .sum::<f64>()
            / waveform.len() as f64;
        assert!(
            (power - float(case, "mean_power")).abs() < 1e-9,
            "{label}: mean power {power} vs {}",
            float(case, "mean_power")
        );
        let peak = waveform
            .iter()
            .map(|&(re, im)| re * re + im * im)
            .fold(0.0, f64::max);
        let papr = 10.0 * (peak / power).log10();
        assert!(
            (papr - float(case, "papr_db")).abs() < 1e-6,
            "{label}: PAPR"
        );

        // strided time samples
        let stride = int(case, "stride");
        let expected_samples = case["strided_samples"].as_array().expect("strided samples");
        for (index, want) in expected_samples.iter().enumerate() {
            let got = waveform[index * stride];
            let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
            assert!(
                (got.0 - wr).abs() < 1e-9 && (got.1 - wi).abs() < 1e-9,
                "{label}: sample {} ({}, {}) vs ({wr}, {wi})",
                index * stride,
                got.0,
                got.1
            );
        }

        // carrier values of every symbol the demodulator can reach
        let expected_carriers = case["carriers"].as_array().expect("carriers");
        for (symbol, want_symbol) in expected_carriers.iter().enumerate() {
            let got = demodulator
                .carriers(&waveform, symbol * period)
                .expect("in range");
            let want = want_symbol.as_array().expect("carrier values");
            assert_eq!(
                got.len(),
                want.len(),
                "{label}: symbol {symbol} carrier count"
            );
            for (carrier, (g, w)) in got.iter().zip(want).enumerate() {
                let (wr, wi) = (w[0].as_f64().expect("re"), w[1].as_f64().expect("im"));
                assert!(
                    (g.0 - wr).abs() < 1e-9 && (g.1 - wi).abs() < 1e-9,
                    "{label}: symbol {symbol} carrier {carrier}: ({}, {}) vs ({wr}, {wi})",
                    g.0,
                    g.1
                );
            }
        }
    }
}
