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
    blanker::{NoiseBlanker, StreamingBlanker},
    codec::{FrameCodec, coprime_stride},
    constellation::{Constellation, NoiseVar},
    modes::{CONTROL_MODE, LONG, MODES, NARROW, SHORT, WIDE},
    ofdm::OfdmDemodulator,
    passband::{AudioToBaseband, BasebandToAudio, band_limit_taps, resample_taps},
    preamble::{FrameHeader, FrameType},
    rx::{FrameReceiver, FrameSync},
    tone,
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

/// The mode and layout a waveform case was generated with, by the layout's name.
fn mode_and_layout(
    air: &aether_phy::modes::AirInterface,
    case: &serde_json::Value,
) -> (aether_phy::modes::Mode, aether_phy::modes::FrameLayout) {
    match case["layout"].as_str().expect("layout") {
        "long" => (air.modes[int(case, "mode")], air.long),
        "short" => (air.control_mode(), air.short),
        other => panic!("unknown layout {other}"),
    }
}

/// The tone-floor kind with this name (ADR-0013), of either air.
fn tone_kind(name: &str) -> &'static tone::ToneKind {
    tone::all_kinds()
        .into_iter()
        .find(|k| k.name == name)
        .unwrap_or_else(|| panic!("no tone kind {name}"))
}

fn usizes(value: &Value) -> Vec<usize> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|v| v.as_u64().expect("an unsigned") as usize)
        .collect()
}

#[test]
fn tone_frames_match_the_model() {
    // ADR-0013: the data tones a payload maps to and the whole frame's tones are exact — the
    // codec chain, the Gray labels and the sync blocks' placement; the waveform, a phase
    // accumulated over raised-cosine glides, to the vector file's tolerance
    let doc = vectors();
    let tolerance = float(&doc, "tone_sample_tolerance");
    let cases = doc["tone_frames"].as_array().expect("tone frames");
    assert!(cases.len() >= 4);
    for case in cases {
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let rv = int(case, "rv");
        let label = format!("{} rv{rv}", kind.name);
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let data = codec.encode(&payload, rv as u8).expect("encode");
        assert_eq!(data, usizes(&case["data_tones"]), "{label}: data tones");
        let frame = tone::frame_tones(kind, &data, rv);
        assert_eq!(frame, usizes(&case["frame_tones"]), "{label}: frame tones");
        let x = tone::burst(&codec, &payload, rv as u8).expect("burst");
        assert_eq!(x.len(), int(case, "n_samples"), "{label}: sample count");
        let power: f64 = x.iter().map(|&(re, im)| re * re + im * im).sum::<f64>() / x.len() as f64;
        assert!(
            (power - float(case, "mean_power")).abs() < tolerance,
            "{label}: mean power {power}"
        );
        let stride = int(case, "stride");
        let want = case["strided_samples"].as_array().expect("samples");
        for (index, w) in want.iter().enumerate() {
            let got = x[index * stride];
            let (wr, wi) = (w[0].as_f64().expect("re"), w[1].as_f64().expect("im"));
            assert!(
                (got.0 - wr).abs() < tolerance && (got.1 - wi).abs() < tolerance,
                "{label}: sample {} is {got:?}, the model has ({wr}, {wi})",
                index * stride
            );
        }
    }
}

/// The generator's stand-in for noise under a tone frame: off-grid tones and a slow chirp.
fn tone_interference(n: usize) -> Vec<(f64, f64)> {
    let tau = 2.0 * std::f64::consts::PI;
    let freqs = [-243.1, -151.7, -63.3, 12.9, 97.7, 171.3, 238.9];
    (0..n)
        .map(|index| {
            let k = index as f64;
            let (mut re, mut im) = (0.0f64, 0.0f64);
            for (i, &f) in freqs.iter().enumerate() {
                let phase = tau * f / 8000.0 * k + 0.37 * i as f64;
                re += phase.cos();
                im += phase.sin();
            }
            let chirp = tau * (-180.0 * k / 8000.0 + 0.5 * 40.0 * (k / 8000.0).powi(2));
            re += chirp.cos();
            im += chirp.sin();
            (0.1 * re, 0.1 * im)
        })
        .collect()
}

#[test]
fn tone_frames_are_received_as_the_model_receives_them() {
    // a frame delayed, turned and buried under interference: the detector's start, kind and
    // redundancy version exact, its offset, statistic and SNR and the soft bits to the vector
    // file's tolerance, the payload exact — each by its own air's detector
    let doc = vectors();
    let tolerance = float(&doc, "tone_llr_tolerance");
    let close = |got: f64, want: f64| (got - want).abs() <= tolerance * want.abs().max(1.0);
    let cases = doc["tone_receive"].as_array().expect("tone receive");
    assert!(!cases.is_empty(), "cases is empty");
    let wide = tone::ToneDetector::for_kinds(&WIDE.tone_kinds());
    let narrow = tone::ToneDetector::for_kinds(&NARROW.tone_kinds());
    for case in cases {
        let detector = match int(case, "bandwidth_hz") {
            2300 => &wide,
            500 => &narrow,
            other => panic!("no air of {other} Hz"),
        };
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let label = kind.name;
        let rv = int(case, "rv") as u8;
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let x = tone::burst(&codec, &payload, rv).expect("burst");
        let (lead, n, cfo) = (
            int(case, "lead"),
            int(case, "n_samples"),
            float(case, "cfo_hz"),
        );
        let mut y = vec![(0.0f64, 0.0f64); n];
        y[lead..lead + x.len()].copy_from_slice(&x);
        let tau = 2.0 * std::f64::consts::PI;
        for (k, (sample, noise)) in y.iter_mut().zip(tone_interference(n)).enumerate() {
            let phase = tau * cfo * k as f64 / 8000.0;
            let (cos, sin) = (phase.cos(), phase.sin());
            *sample = (
                sample.0 * cos - sample.1 * sin + noise.0,
                sample.0 * sin + sample.1 * cos + noise.1,
            );
        }
        let found = detector.detect(&y, 8);
        assert_eq!(found.len(), 1, "{label}: {found:?}");
        let sync = found[0];
        assert_eq!(sync.start, int(case, "start"), "{label}: start");
        assert_eq!((sync.kind.name, sync.rv), (label, rv), "{label}: kind");
        assert!(
            close(sync.cfo_hz, float(case, "detected_cfo_hz")),
            "{label}: offset {}",
            sync.cfo_hz
        );
        assert!(
            close(sync.statistic, float(case, "statistic")),
            "{label}: statistic"
        );
        let frame = tone::demodulate(&y, kind, rv, sync.start, sync.cfo_hz).expect("in range");
        assert!(
            close(frame.snr_db, float(case, "snr_db")),
            "{label}: SNR {}",
            frame.snr_db
        );
        match (frame.echo, case["echo"].as_f64()) {
            (None, None) => {}
            (Some(got), Some(want)) => assert!(close(got, want), "{label}: echo {got}"),
            (got, want) => panic!("{label}: echo {got:?}, the model has {want:?}"),
        }
        let want = case["llr"].as_array().expect("llr");
        assert_eq!(frame.llr.len(), want.len(), "{label}: soft bits");
        for (index, (got, w)) in frame.llr.iter().zip(want).enumerate() {
            let w = w.as_f64().expect("llr");
            assert!(
                close(*got, w),
                "{label}: soft bit {index} is {got}, the model has {w}"
            );
        }
        let (decoded, _) = codec.decode(&frame.llr, rv, None).expect("decode");
        assert_eq!(
            decoded.as_deref(),
            Some(payload.as_slice()),
            "{label}: payload"
        );
    }
}

#[test]
fn tone_frames_on_two_rays_read_their_echo_as_the_model_does() {
    // ADR-0087: a frame and its echo some milliseconds later, turned and under interference,
    // demodulated at its known start and offset — the floor's own kinds read the echo, a fast
    // kind reads none
    let doc = vectors();
    let tolerance = float(&doc, "tone_llr_tolerance");
    let close = |got: f64, want: f64| (got - want).abs() <= tolerance * want.abs().max(1.0);
    let cases = doc["tone_echo"].as_array().expect("tone echo");
    assert!(!cases.is_empty(), "cases is empty");
    let tau = 2.0 * std::f64::consts::PI;
    for case in cases {
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let label = kind.name;
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let x = tone::burst(&codec, &payload, 0).expect("burst");
        let (lead, delay, n, cfo) = (
            int(case, "lead"),
            int(case, "delay"),
            int(case, "n_samples"),
            float(case, "cfo_hz"),
        );
        let (amp, phase) = (float(case, "amplitude"), float(case, "phase"));
        let ray = (amp * phase.cos(), amp * phase.sin());
        let mut y = vec![(0.0f64, 0.0f64); n];
        y[lead..lead + x.len()].copy_from_slice(&x);
        for (k, s) in x.iter().enumerate() {
            let at = &mut y[lead + delay + k];
            at.0 += ray.0 * s.0 - ray.1 * s.1;
            at.1 += ray.0 * s.1 + ray.1 * s.0;
        }
        for (k, (sample, noise)) in y.iter_mut().zip(tone_interference(n)).enumerate() {
            let turn = tau * cfo * k as f64 / 8000.0;
            let (cos, sin) = (turn.cos(), turn.sin());
            *sample = (
                sample.0 * cos - sample.1 * sin + noise.0,
                sample.0 * sin + sample.1 * cos + noise.1,
            );
        }
        // a carrier on one of the frame's bins: a narrowband signal in the span, whose bin the
        // echo share leaves out (ADR-0087 §3)
        if let Some(hz) = case["carrier_hz"].as_f64() {
            let amp = float(case, "carrier_amplitude");
            for (k, sample) in y.iter_mut().enumerate() {
                let turn = tau * hz * k as f64 / 8000.0;
                sample.0 += amp * turn.cos();
                sample.1 += amp * turn.sin();
            }
        }
        let frame = tone::demodulate(&y, kind, 0, lead, cfo).expect("in range");
        assert!(
            close(frame.snr_db, float(case, "snr_db")),
            "{label}: SNR {}",
            frame.snr_db
        );
        match (frame.echo, case["echo"].as_f64()) {
            (None, None) => {}
            (Some(got), Some(want)) => assert!(close(got, want), "{label}: echo {got}"),
            (got, want) => panic!("{label}: echo {got:?}, the model has {want:?}"),
        }
    }
}

/// The model's `tone_envelope`: a sum of rays `amplitude·exp(j(2π·f·k/8000 + phase))`, or a
/// gate of `floor` plus one where `cos(2π·f·k/8000)` is above `above`.
fn tone_envelope(spec: &Value, n: usize) -> Vec<(f64, f64)> {
    let tau = 2.0 * std::f64::consts::PI;
    match spec["kind"].as_str().expect("envelope kind") {
        "rays" => {
            let rays: Vec<(f64, f64, f64)> = spec["rays"]
                .as_array()
                .expect("rays")
                .iter()
                .map(|r| {
                    let r = r.as_array().expect("ray");
                    (
                        r[0].as_f64().expect("amplitude"),
                        r[1].as_f64().expect("frequency"),
                        r[2].as_f64().expect("phase"),
                    )
                })
                .collect();
            (0..n)
                .map(|k| {
                    rays.iter().fold((0.0, 0.0), |(re, im), &(a, f, p)| {
                        let phase = tau * f * k as f64 / 8000.0 + p;
                        (re + a * phase.cos(), im + a * phase.sin())
                    })
                })
                .collect()
        }
        "gate" => {
            let (floor, f, above) = (
                float(spec, "floor"),
                float(spec, "f_hz"),
                float(spec, "above"),
            );
            (0..n)
                .map(|k| {
                    let on = (tau * f * k as f64 / 8000.0).cos() > above;
                    (floor + if on { 1.0 } else { 0.0 }, 0.0)
                })
                .collect()
        }
        other => panic!("no envelope {other}"),
    }
}

#[test]
fn four_tone_frames_that_fade_within_a_block_are_read_as_the_model_reads_them() {
    // ADR-0080 §2.4: a frame under an envelope that moves within a sync block is read with
    // the Nakagami metric of the order its sync blocks' wobble gives — orders 4, 3 and 1 here
    let doc = vectors();
    let tolerance = float(&doc, "tone_llr_tolerance");
    let close = |got: f64, want: f64| (got - want).abs() <= tolerance * want.abs().max(1.0);
    let cases = doc["tone_faded"].as_array().expect("tone faded");
    assert!(!cases.is_empty(), "cases is empty");
    for case in cases {
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let label = kind.name;
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let x = tone::burst(&codec, &payload, 0).expect("burst");
        let (lead, n, cfo) = (
            int(case, "lead"),
            int(case, "n_samples"),
            float(case, "cfo_hz"),
        );
        let mut y = vec![(0.0f64, 0.0f64); n];
        y[lead..lead + x.len()].copy_from_slice(&x);
        let tau = 2.0 * std::f64::consts::PI;
        let envelope = tone_envelope(&case["envelope"], n);
        for (k, ((sample, noise), env)) in y
            .iter_mut()
            .zip(tone_interference(n))
            .zip(envelope)
            .enumerate()
        {
            let faded = (
                sample.0 * env.0 - sample.1 * env.1,
                sample.0 * env.1 + sample.1 * env.0,
            );
            let phase = tau * cfo * k as f64 / 8000.0;
            let (cos, sin) = (phase.cos(), phase.sin());
            *sample = (
                faded.0 * cos - faded.1 * sin + noise.0,
                faded.0 * sin + faded.1 * cos + noise.1,
            );
        }
        let layout = kind.layout(0);
        let energies = tone::tone_energies(&y, lead, cfo, kind.symbols()).expect("in range");
        let noise = tone::sync_noise(&energies, &layout, kind);
        let normalised: Vec<f64> = energies.iter().map(|e| e / noise).collect();
        let wobble = tone::fade_wobble(&normalised, &layout);
        assert!(
            close(wobble, float(case, "wobble")),
            "{label}: wobble {wobble}"
        );
        let frame = tone::demodulate(&y, kind, 0, lead, cfo).expect("in range");
        let want = case["llr"].as_array().expect("llr");
        assert_eq!(frame.llr.len(), want.len(), "{label}: soft bits");
        for (index, (got, w)) in frame.llr.iter().zip(want).enumerate() {
            let w = w.as_f64().expect("llr");
            assert!(
                close(*got, w),
                "{label}: soft bit {index} is {got}, the model has {w}"
            );
        }
        let (decoded, _) = codec.decode(&frame.llr, 0, None).expect("decode");
        assert_eq!(
            decoded.as_deref() == Some(payload.as_slice()),
            case["decoded"].as_bool().expect("decoded"),
            "{label}: decoded"
        );
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
    // Every frame appears twice, with ADR-0004 peak reduction off and on. The unreduced pass
    // compares the modulator alone; the reduced one compares the clipper as well, and the
    // peak-to-average ratio it reaches — already checked below — is what says the two
    // clippers agree rather than merely both being clippers.
    let doc = vectors();
    let cases = doc["waveform_frames"].as_array().expect("waveform frames");
    assert!(!cases.is_empty(), "cases is empty");
    assert!(
        cases.iter().any(|c| c["peak_reduced"] == true)
            && cases.iter().any(|c| c["peak_reduced"] == false),
        "the vectors must cover both, or one of the two paths goes untested"
    );

    assert!(
        cases.iter().any(|c| c["bandwidth_hz"] == 500),
        "the vectors must cover the narrow waveform too"
    );

    for case in cases {
        // the wide cases from before the narrow waveform existed carry no bandwidth
        let params = match case["bandwidth_hz"].as_u64().unwrap_or(2300) {
            2300 => WIDE_2300,
            500 => aether_phy::waveform::NARROW_500,
            other => panic!("no waveform for {other} Hz"),
        };
        let air = aether_phy::modes::air_interface(params);
        let demodulator = OfdmDemodulator::new(params);
        let period = params.symbol_samples();
        let label = format!(
            "{} Hz {} rv{} follows {} ({})",
            params.bandwidth.hz(),
            case["mode_name"].as_str().unwrap_or("?"),
            int(case, "rv"),
            int(case, "follows"),
            case["layout"].as_str().unwrap_or("?")
        );
        let (mode, layout) = mode_and_layout(&air, case);
        let peak_reduced = case["peak_reduced"].as_bool().expect("peak_reduced");
        let label = format!(
            "{label}, peak reduction {}",
            if peak_reduced { "on" } else { "off" }
        );
        let tx = if peak_reduced {
            FrameTransmitter::new(params)
        } else {
            FrameTransmitter::new(params).without_papr_reduction()
        };
        let rv = int(case, "rv") as u8;
        let header = match case["frame_type"].as_str().expect("frame type") {
            // the burst countdown turns the chips (ADR-0041)
            "DATA" => FrameHeader::new(FrameType::Data, mode.index, rv)
                .and_then(|header| header.with_follows(int(case, "follows") as u8))
                .expect("header"),
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
        let sampled: Vec<(f64, f64)> =
            (0..case["strided_samples"].as_array().expect("samples").len())
                .map(|index| waveform[index * stride])
                .collect();
        expect_complex(
            &sampled,
            &case["strided_samples"],
            &format!("{label}: samples"),
        );

        // carrier values of every symbol the demodulator can reach
        let expected_carriers = case["carriers"].as_array().expect("carriers");
        for (symbol, want) in expected_carriers.iter().enumerate() {
            let got = demodulator
                .carriers(&waveform, symbol * period)
                .expect("in range");
            expect_complex(&got, want, &format!("{label}: symbol {symbol}"));
        }
    }
}

/// `SplitMix64` draws mapped to `[−1, 1)`, real then imaginary: the vector generator's noise.
fn pseudo_noise(n: usize, seed: u64) -> Vec<(f64, f64)> {
    let mut state = seed;
    let mut draw = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 * 2f64.powi(-53) * 2.0 - 1.0
    };
    (0..n).map(|_| (draw(), draw())).collect()
}

/// `input` turned by `hz` from its first sample, at the baseband rate.
fn turned(input: &[(f64, f64)], hz: f64, phase: f64) -> impl Iterator<Item = (f64, f64)> + '_ {
    input.iter().enumerate().map(move |(index, &(re, im))| {
        let angle = 2.0 * std::f64::consts::PI * hz * index as f64 / 8000.0 + phase;
        let (cos, sin) = (angle.cos(), angle.sin());
        (re * cos - im * sin, re * sin + im * cos)
    })
}

/// `Σ amplitude · x[k − delay] · exp(j(2π·f·k/8000 + phase))` over `[amplitude, f, phase,
/// delay]` rays, as the generator builds it.
fn ofdm_rays(input: &[(f64, f64)], rays: &Value) -> Vec<(f64, f64)> {
    let mut output = vec![(0.0, 0.0); input.len()];
    for ray in rays.as_array().expect("rays") {
        let ray = ray.as_array().expect("ray");
        let amplitude = ray[0].as_f64().expect("amplitude");
        let hz = ray[1].as_f64().expect("frequency");
        let phase = ray[2].as_f64().expect("phase");
        let delay = ray[3].as_f64().expect("delay") as usize;
        let mut delayed = vec![(0.0, 0.0); delay];
        delayed.extend_from_slice(&input[..input.len() - delay]);
        for (out, (re, im)) in output.iter_mut().zip(turned(&delayed, hz, phase)) {
            out.0 += amplitude * re;
            out.1 += amplitude * im;
        }
    }
    output
}

/// One `ofdm_receive` case's received samples and the sync it is received at, with its codec,
/// payload and a label.
struct OfdmInput {
    samples: Vec<(f64, f64)>,
    sync: FrameSync,
    params: aether_phy::waveform::WaveformParams,
    codec: FrameCodec,
    payload: Vec<u8>,
    label: String,
}

fn ofdm_receive_input(case: &Value) -> OfdmInput {
    let params = match int(case, "bandwidth_hz") {
        2300 => WIDE_2300,
        500 => aether_phy::waveform::NARROW_500,
        other => panic!("no waveform for {other} Hz"),
    };
    let air = aether_phy::modes::air_interface(params);
    let control = case["mode"].as_i64().expect("mode") < 0;
    let (mode, layout) = if control {
        (air.control_mode(), air.short)
    } else {
        (air.modes[int(case, "mode")], air.long)
    };
    let rv = int(case, "rv") as u8;
    let header = if control {
        FrameHeader::control()
    } else {
        FrameHeader::new(FrameType::Data, mode.index, rv)
            .and_then(|header| header.with_follows(int(case, "follows") as u8))
            .expect("header")
    };
    let label = format!("{} Hz {} rv{rv}", params.bandwidth.hz(), mode.name());
    let codec = FrameCodec::new(mode, layout).expect("codec");
    let payload = from_hex(case["payload"].as_str().expect("payload"));
    let frame = FrameTransmitter::new(params)
        .baseband(
            &header,
            &layout,
            &codec.encode(&payload, rv).expect("encode"),
        )
        .expect("baseband");
    let lead = int(case, "lead");
    let mut clean = vec![(0.0, 0.0); int(case, "n_samples")];
    clean[lead..lead + frame.len()].copy_from_slice(&frame);
    let cfo = float(case, "cfo_hz");
    let level = float(case, "noise");
    let draws = pseudo_noise(clean.len(), case["noise_seed"].as_u64().expect("seed"));
    let samples = turned(&ofdm_rays(&clean, &case["rays"]), cfo, 0.0)
        .zip(draws)
        .map(|((re, im), (nre, nim))| (re + level * nre, im + level * nim))
        .collect();
    let sync = FrameSync {
        start: lead,
        cfo_hz: cfo - 0.8,
        frame_type: if control {
            FrameType::Control
        } else {
            FrameType::Data
        },
        timing_peak: 1.0,
        type_confidence: 1.0,
    };
    OfdmInput {
        samples,
        sync,
        params,
        codec,
        payload,
        label,
    }
}

#[test]
fn ofdm_frames_are_received_as_the_model_receives_them() {
    // ADR-0082: a frame on a steady channel and one whose two paths beat at 9–12 Hz, on both
    // airs — the receiver decides the same about the channel's motion, follows it with the
    // same weight, and reports the same offset, SNR, symbols and noise variances
    let doc = vectors();
    let tol = float(&doc, "ofdm_receive_tolerance");
    let cases = doc["ofdm_receive"].as_array().expect("ofdm receive");
    assert!(
        cases.iter().any(|c| c["motion_weight"].is_null())
            && cases.iter().any(|c| !c["motion_weight"].is_null()),
        "the vectors must cover a steady channel and a moving one"
    );
    let close = |got: f64, want: f64| (got - want).abs() <= tol * (1.0 + want.abs());
    for case in cases {
        let OfdmInput {
            samples,
            sync,
            params,
            codec,
            payload,
            label,
        } = ofdm_receive_input(case);
        let frame = FrameReceiver::new(params)
            .receive(&samples, &sync, None)
            .expect("receive");
        assert_eq!(frame.mode, int(case, "received_mode"), "{label}: mode");
        assert_eq!(
            usize::from(frame.rv),
            int(case, "received_rv"),
            "{label}: rv"
        );
        assert_eq!(
            usize::from(frame.follows),
            int(case, "received_follows"),
            "{label}: follows"
        );
        match case["motion_weight"].as_f64() {
            None => assert!(frame.motion_weight.is_none(), "{label}: taken as moving"),
            Some(want) => {
                let got = frame.motion_weight.expect("taken as steady");
                assert!(close(got, want), "{label}: weight {got} vs {want}");
            }
        }
        for (key, got) in [
            ("received_cfo_hz", frame.cfo_hz),
            ("snr_carrier_db", frame.snr_carrier_db),
            ("snr_3k_db", frame.snr_3k_db),
        ] {
            let want = float(case, key);
            assert!(close(got, want), "{label}: {key} {got} vs {want}");
        }
        let want = case["symbols"].as_array().expect("symbols");
        assert_eq!(frame.symbols.len(), want.len(), "{label}: symbol count");
        for (index, (got, want)) in frame.symbols.iter().zip(want).enumerate() {
            let pair = want.as_array().expect("pair");
            let (re, im) = (pair[0].as_f64().expect("re"), pair[1].as_f64().expect("im"));
            assert!(
                close(got.0, re) && close(got.1, im),
                "{label}: symbol {index} {got:?} vs ({re}, {im})"
            );
        }
        let want = case["noise_var"].as_array().expect("noise_var");
        for (index, (&got, want)) in frame.noise_var.iter().zip(want).enumerate() {
            let want = want.as_f64().expect("variance");
            assert!(close(got, want), "{label}: noise {index} {got} vs {want}");
        }
        let (decoded, _) = codec
            .decode(
                &frame.symbols,
                NoiseVar::PerSymbol(&frame.noise_var),
                frame.rv,
                None,
            )
            .expect("decode");
        assert_eq!(
            decoded.as_deref() == Some(payload.as_slice()),
            case["decoded"].as_bool().expect("decoded"),
            "{label}: decoded"
        );
    }
}

/// Compare complex values against a `[[re, im], ...]` array from the vector file.
fn expect_complex(got: &[(f64, f64)], want: &Value, label: &str) {
    let want = want
        .as_array()
        .unwrap_or_else(|| panic!("{label}: not an array"));
    assert_eq!(got.len(), want.len(), "{label}: count");
    for (index, (g, w)) in got.iter().zip(want).enumerate() {
        let (wr, wi) = (w[0].as_f64().expect("re"), w[1].as_f64().expect("im"));
        assert!(
            (g.0 - wr).abs() < 1e-9 && (g.1 - wi).abs() < 1e-9,
            "{label}: entry {index} is ({}, {}), the model has ({wr}, {wi})",
            g.0,
            g.1
        );
    }
}

/// The same deterministic multi-tone input the generator builds. No RNG, because a vector
/// file that recorded only every k-th sample could not say what the others were, and a
/// filter's output at one sample depends on many.
fn passband_input(n: usize) -> Vec<(f64, f64)> {
    (0..n)
        .map(|i| {
            let k = i as f64;
            let tau = 2.0 * std::f64::consts::PI;
            let real = (tau * 0.037 * k).cos() + 0.5 * (tau * 0.011 * k + 0.7).cos();
            let imag = (tau * 0.023 * k).sin() - 0.3 * (tau * 0.005 * k).sin();
            (0.3 * real, 0.3 * imag)
        })
        .collect()
}

#[test]
fn the_audio_front_end_matches_the_model() {
    // The filter taps are a *design*, not a measurement: two implementations of the same
    // window method either agree to the last bit or one of them is wrong, so they are
    // compared exactly. What flows through them is floating-point arithmetic over a hundred
    // and forty taps, so those are compared to a tolerance.
    let case = &vectors()["passband"];

    for (name, got) in [
        ("band_limit_taps", band_limit_taps(&WIDE_2300)),
        ("resample_taps", resample_taps(&WIDE_2300)),
    ] {
        let want = case[name]
            .as_array()
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(got.len(), want.len(), "{name}: length");
        for (index, (g, w)) in got.iter().zip(want).enumerate() {
            let w = w.as_f64().expect("tap");
            assert!(
                (g - w).abs() < 1e-15,
                "{name}: tap {index} is {g}, the model has {w}"
            );
        }
    }

    let mut tx = BasebandToAudio::new(WIDE_2300);
    let mut rx = AudioToBaseband::new(WIDE_2300);
    assert_eq!(tx.tx_delay_samples(), int(case, "tx_delay_samples"));
    assert_eq!(rx.rx_delay_samples(), int(case, "rx_delay_samples"));

    let baseband = passband_input(int(case, "n_samples"));
    let audio = tx.process(&baseband);
    let back = rx.process(&audio);
    let stride = int(case, "stride");

    let want_audio = case["audio_out"].as_array().expect("audio_out");
    assert_eq!(
        audio.len().div_ceil(stride),
        want_audio.len(),
        "audio length"
    );
    for (index, want) in want_audio.iter().enumerate() {
        let want = want.as_f64().expect("audio sample");
        let got = f64::from(audio[index * stride]);
        // the model returns float32 audio, so the comparison is at single precision
        assert!(
            (got - want).abs() < 1e-6,
            "audio sample {}: {got} vs {want}",
            index * stride
        );
    }

    let want_back = case["baseband_back"].as_array().expect("baseband_back");
    assert_eq!(
        back.len().div_ceil(stride),
        want_back.len(),
        "baseband length"
    );
    for (index, want) in want_back.iter().enumerate() {
        let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
        let (gr, gi) = back[index * stride];
        assert!(
            (gr - wr).abs() < 1e-6 && (gi - wi).abs() < 1e-6,
            "baseband sample {}: ({gr}, {gi}) vs ({wr}, {wi})",
            index * stride
        );
    }

    // the receive mixer moved to a peer's offset partway through (ADR-0079)
    let mut moved = AudioToBaseband::new(WIDE_2300);
    let half = int(case, "moved_after_audio");
    let mut back = moved.process(&audio[..half]);
    moved.set_offset_hz(case["moved_offset_hz"].as_f64().expect("moved_offset_hz"));
    back.extend(moved.process(&audio[half..]));
    let want_moved = case["baseband_moved"].as_array().expect("baseband_moved");
    assert_eq!(
        back.len().div_ceil(stride),
        want_moved.len(),
        "moved length"
    );
    for (index, want) in want_moved.iter().enumerate() {
        let (wr, wi) = (want[0].as_f64().expect("re"), want[1].as_f64().expect("im"));
        let (gr, gi) = back[index * stride];
        assert!(
            (gr - wr).abs() < 1e-6 && (gi - wi).abs() < 1e-6,
            "moved sample {}: ({gr}, {gi}) vs ({wr}, {wi})",
            index * stride
        );
    }
}

/// The same deterministic blanker input the generator builds: silence, then a burst with a
/// peaky envelope, with impulses of known size dropped into both.
fn blanker_input(n: usize) -> Vec<(f64, f64)> {
    let tau = 2.0 * std::f64::consts::PI;
    let mut out: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let k = i as f64;
            let envelope = if i < n / 4 { 0.02 } else { 1.0 };
            let phase = tau * 0.7f64.mul_add((tau * 0.0013 * k).sin(), 0.031 * k);
            let peaks =
                0.8f64.mul_add((tau * 0.0071 * k).cos(), 1.0) + 0.5 * (tau * 0.017 * k).cos();
            let amplitude = envelope * peaks;
            (amplitude * phase.cos(), amplitude * phase.sin())
        })
        .collect();
    for (index, size) in [(n / 8, 30.0), (n / 2, 50.0), (3 * n / 4, 8.0)] {
        let angle = index as f64;
        out[index] = (size * angle.cos(), size * angle.sin());
    }
    out
}

#[test]
fn the_impulse_blanker_matches_the_model() {
    // The blanker decides which samples to throw away, so what matters is *which*: an
    // implementation that blanks a different set is a different receiver, and two stations
    // that disagree about it will disagree about what decodes. The indices are therefore
    // compared exactly; the envelope estimate behind them is floating point and is compared
    // to a tolerance.
    let case = &vectors()["blanker"];
    let blanker = NoiseBlanker::default();
    assert_eq!(blanker.window(), int(case, "window"));
    assert_eq!(
        blanker.robust_span_samples(),
        int(case, "robust_span_samples")
    );

    let samples = blanker_input(int(case, "n_samples"));
    let stride = int(case, "stride");

    let envelope = blanker.envelope_rms(&samples);
    let want = case["envelope_rms"].as_array().expect("envelope_rms");
    assert_eq!(
        envelope.len().div_ceil(stride),
        want.len(),
        "envelope length"
    );
    for (index, w) in want.iter().enumerate() {
        let w = w.as_f64().expect("envelope sample");
        let got = envelope[index * stride];
        assert!(
            (got - w).abs() < 1e-12,
            "envelope at {}: {got} vs {w}",
            index * stride
        );
    }

    let result = blanker.process(&samples);
    let blanked: Vec<usize> = result
        .blanked
        .iter()
        .enumerate()
        .filter_map(|(index, &hot)| hot.then_some(index))
        .collect();
    let want: Vec<usize> = case["blanked"]
        .as_array()
        .expect("blanked")
        .iter()
        .map(|v| v.as_u64().expect("index") as usize)
        .collect();
    assert_eq!(blanked, want, "the offline blanker removed a different set");

    for (block, expected) in case["streaming"].as_object().expect("streaming") {
        let block: usize = block.parse().expect("block size");
        let mut streaming = StreamingBlanker::default();
        assert_eq!(
            streaming.latency_samples(),
            int(expected, "latency_samples"),
            "block {block}: latency"
        );
        let mut out: Vec<(f64, f64)> = Vec::new();
        for chunk in samples.chunks(block) {
            out.extend(streaming.process(chunk));
        }
        out.extend(streaming.flush());
        assert_eq!(out.len(), samples.len(), "block {block}: length");

        let blanked: Vec<usize> = out
            .iter()
            .enumerate()
            .filter_map(|(index, &s)| (s == (0.0, 0.0)).then_some(index))
            .collect();
        let want: Vec<usize> = expected["blanked"]
            .as_array()
            .expect("blanked")
            .iter()
            .map(|v| v.as_u64().expect("index") as usize)
            .collect();
        assert_eq!(
            blanked, want,
            "block {block}: the streaming blanker removed a different set"
        );
    }
}

#[test]
fn a_frame_after_the_receivers_own_silence_is_found_where_the_model_finds_it() {
    // a frame straight after exact silence — a station's own transmission, muted — and the
    // hypothesis a block-spacing early, first block in the silence and middle block on the
    // frame's first: its block hits and verdict exact, and then the frame the detector takes
    let doc = vectors();
    let cases = doc["tone_after_silence"].as_array().expect("cases");
    assert!(!cases.is_empty(), "cases is empty");
    let kind = &tone::data_kinds()[1];
    let codec = tone::ToneCodec::new(kind).expect("codec");
    let detector = tone::ToneDetector::new();
    for case in cases {
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let (lead, n) = (int(case, "lead"), int(case, "n_samples"));
        let x = tone::burst(&codec, &payload, 0).expect("burst");
        let mut y = vec![(0.0f64, 0.0f64); n];
        y[lead..lead + x.len()].copy_from_slice(&x);
        for (sample, noise) in y.iter_mut().zip(tone_interference(n)).skip(lead) {
            sample.0 += noise.0;
            sample.1 += noise.1;
        }
        let early = int(case, "early_start");
        let want: Vec<usize> = case["early_block_hits"]
            .as_array()
            .expect("hits")
            .iter()
            .map(|v| v.as_u64().expect("hits") as usize)
            .collect();
        assert_eq!(
            detector.block_hits(&y, kind, 0, early, 0.0).to_vec(),
            want,
            "block hits"
        );
        let phantom = tone::ToneSync {
            start: early,
            cfo_hz: 0.0,
            kind,
            rv: 0,
            statistic: 0.0,
            middle: 0,
            follows: Some(0),
        };
        assert_eq!(
            detector.confirmed(&y, &phantom),
            case["early_confirmed"].as_bool().expect("verdict")
        );
        let found = detector.detect(&y, 8);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].start, int(case, "start"));
        assert_eq!((found[0].kind.name, found[0].rv), (kind.name, 0));
    }
}

#[test]
fn counted_tone_frames_are_the_models() {
    // a tone frame's middle sync block carries how many frames of its burst follow it
    // (ADR-0068): the frame's tones exact, the waveform to the model's tolerance
    let doc = vectors();
    let tolerance = float(&doc, "tone_sample_tolerance");
    let cases = doc["tone_counted_frames"]
        .as_array()
        .expect("counted frames");
    assert!(cases.len() >= 4);
    for case in cases {
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let (rv, follows) = (int(case, "rv"), int(case, "follows"));
        let label = format!("{} rv{rv} follows{follows}", kind.name);
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let data = codec.encode(&payload, rv as u8).expect("encode");
        let (frame, _) = tone::frame_symbols_with(kind, &data, rv, follows);
        assert_eq!(frame, usizes(&case["frame_tones"]), "{label}: frame tones");
        let x = tone::burst_with(&codec, &payload, rv as u8, follows as u8).expect("burst");
        assert_eq!(x.len(), int(case, "n_samples"), "{label}: sample count");
        let stride = int(case, "stride");
        let want = case["strided_samples"].as_array().expect("samples");
        for (index, w) in want.iter().enumerate() {
            let got = x[index * stride];
            let (wr, wi) = (w[0].as_f64().expect("re"), w[1].as_f64().expect("im"));
            assert!(
                (got.0 - wr).abs() < tolerance && (got.1 - wi).abs() < tolerance,
                "{label}: sample {} is {got:?}, the model has ({wr}, {wi})",
                index * stride
            );
        }
    }
}

#[test]
fn counted_tone_frames_are_received_as_the_model_receives_them() {
    // the detector reads the middle block's count as the model does, and the frame decodes
    let doc = vectors();
    let tolerance = float(&doc, "tone_llr_tolerance");
    let close = |got: f64, want: f64| (got - want).abs() <= tolerance * want.abs().max(1.0);
    let cases = doc["tone_counted_receive"]
        .as_array()
        .expect("counted receive");
    assert!(!cases.is_empty(), "cases is empty");
    let wide = tone::ToneDetector::for_kinds(&WIDE.tone_kinds());
    let narrow = tone::ToneDetector::for_kinds(&NARROW.tone_kinds());
    for case in cases {
        let detector = match int(case, "bandwidth_hz") {
            2300 => &wide,
            500 => &narrow,
            other => panic!("no air of {other} Hz"),
        };
        let kind = tone_kind(case["kind"].as_str().expect("kind"));
        let label = kind.name;
        let follows = int(case, "follows") as u8;
        let payload = from_hex(case["payload"].as_str().expect("payload"));
        let codec = tone::ToneCodec::new(kind).expect("codec");
        let x = tone::burst_with(&codec, &payload, 0, follows).expect("burst");
        let (lead, n, cfo) = (
            int(case, "lead"),
            int(case, "n_samples"),
            float(case, "cfo_hz"),
        );
        let mut y = vec![(0.0f64, 0.0f64); n];
        y[lead..lead + x.len()].copy_from_slice(&x);
        let tau = 2.0 * std::f64::consts::PI;
        for (k, (sample, noise)) in y.iter_mut().zip(tone_interference(n)).enumerate() {
            let phase = tau * cfo * k as f64 / 8000.0;
            let (cos, sin) = (phase.cos(), phase.sin());
            *sample = (
                sample.0 * cos - sample.1 * sin + noise.0,
                sample.0 * sin + sample.1 * cos + noise.1,
            );
        }
        let found = detector.detect(&y, 8);
        assert_eq!(found.len(), 1, "{label}: {found:?}");
        let sync = found[0];
        assert_eq!(sync.start, int(case, "start"), "{label}: start");
        assert_eq!((sync.kind.name, sync.rv), (label, 0), "{label}: kind");
        assert_eq!(
            usize::from(sync.middle),
            int(case, "middle"),
            "{label}: middle"
        );
        assert_eq!(
            sync.follows.map(usize::from),
            case["detected_follows"].as_u64().map(|v| v as usize),
            "{label}: count"
        );
        assert!(
            close(sync.cfo_hz, float(case, "detected_cfo_hz")),
            "{label}: offset {}",
            sync.cfo_hz
        );
        assert!(
            close(sync.statistic, float(case, "statistic")),
            "{label}: statistic"
        );
        let frame = tone::demodulate_with(&y, kind, 0, sync.middle, sync.start, sync.cfo_hz)
            .expect("in range");
        let (decoded, _) = codec.decode(&frame.llr, 0, None).expect("decode");
        assert_eq!(
            decoded.as_deref(),
            Some(payload.as_slice()),
            "{label}: payload"
        );
    }
}
