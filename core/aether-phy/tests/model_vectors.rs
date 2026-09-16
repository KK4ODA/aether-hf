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
    modes::{CONTROL_MODE, LONG, MODES, SHORT},
    ofdm::OfdmDemodulator,
    passband::{AudioToBaseband, BasebandToAudio, band_limit_taps, resample_taps},
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

/// The mode and layout a waveform case was generated with: the ordinary layouts by name,
/// and the floor family's (ADR-0009) where the air has one.
fn mode_and_layout(
    air: &aether_phy::modes::AirInterface,
    case: &serde_json::Value,
) -> (aether_phy::modes::Mode, aether_phy::modes::FrameLayout) {
    match case["layout"].as_str().expect("layout") {
        "long" => (air.modes[int(case, "mode")], air.long),
        "short" => (air.control_mode(), air.short),
        "floor-long" => (
            air.modes[int(case, "mode")],
            air.floor_long.expect("floor layout"),
        ),
        "floor-short" => (
            air.control_mode_for(true),
            air.floor_short.expect("floor layout"),
        ),
        other => panic!("unknown layout {other}"),
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
    assert!(!cases.is_empty());
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
            "{} Hz {} rv{} ({})",
            params.bandwidth.hz(),
            case["mode_name"].as_str().unwrap_or("?"),
            int(case, "rv"),
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
