//! Compile the air-interface constants into the crate.
//!
//! The Schmidl–Cox and chip sequences, the tone floor's sync patterns and frame kinds are
//! exported from the reference model by `tools/make_phy_tables.py`. They are fixed constants
//! that two stations must agree on exactly, so they are data, not something to be regenerated
//! by reimplementing the model's random number generator. One [`Tables`] per waveform: the
//! wide 2 300 Hz one and the narrow 500 Hz one, which have different carrier maps, different
//! chip sets, different acquisition thresholds and different OFDM modes on their ladders but
//! the same shape; and one `TONE` block for the tone floor (ADR-0013), the same on both.

use std::{env, fmt::Write as _, fs, path::PathBuf};

/// Hex-packed sign bits (1 meaning −1) back into `+1.0 / -1.0`.
fn unpack_signs(hex: &str, len: usize) -> Vec<f64> {
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bad hex"))
        .collect();
    (0..len)
        .map(|i| {
            if (bytes[i / 8] >> (7 - i % 8)) & 1 == 1 {
                -1.0
            } else {
                1.0
            }
        })
        .collect()
}

fn literal(values: &[f64]) -> String {
    let mut out = String::new();
    for value in values {
        let _ = write!(out, "{value:?}, ");
    }
    out
}

fn usizes(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|v| v.as_u64().expect("an unsigned").to_string())
        .collect()
}

/// One waveform's block of the JSON as Rust statics under `prefix`, plus its `Tables`.
fn waveform(out: &mut String, prefix: &str, doc: &serde_json::Value, n_rv: usize) {
    let sc_len = doc["sc_length"].as_u64().expect("sc_length") as usize;
    let chip_len = doc["chip_length"].as_u64().expect("chip_length") as usize;
    let n_modes = doc["n_modes"].as_u64().expect("n_modes") as usize;
    let bandwidth = doc["bandwidth_hz"].as_u64().expect("bandwidth_hz") as usize;
    let control_mode_index = doc["control_mode_index"].as_u64().unwrap_or(0) as usize;

    let even = usizes(&doc["even_carriers"]);
    let _ = writeln!(
        out,
        "static {prefix}_EVEN_CARRIERS: [usize; {}] = [{}];",
        even.len(),
        even.join(", ")
    );
    let ladder = usizes(&doc["ofdm_ladder"]);
    let _ = writeln!(
        out,
        "static {prefix}_OFDM_LADDER: [usize; {}] = [{}];",
        ladder.len(),
        ladder.join(", ")
    );
    for (name, key) in [("SC_DATA", "DATA"), ("SC_CONTROL", "CONTROL")] {
        let hex = doc["schmidl_cox"][key].as_str().expect("sc sequence");
        let _ = writeln!(
            out,
            "static {prefix}_{name}: [f64; {sc_len}] = [{}];",
            literal(&unpack_signs(hex, sc_len))
        );
    }
    // chips flat, row-major: sequence `i` is `[i * chip_length .. (i + 1) * chip_length]`
    let _ = writeln!(
        out,
        "static {prefix}_MODE_CHIPS: [f64; {}] = [",
        n_rv * n_modes * chip_len
    );
    for rv in 0..n_rv {
        for mode in 0..n_modes {
            let hex = doc["mode_chips"][format!("{rv}_{mode}")]
                .as_str()
                .expect("chip sequence");
            let _ = writeln!(out, "    {}", literal(&unpack_signs(hex, chip_len)));
        }
    }
    out.push_str("];\n");
    let pilots = doc["pilot_sequence"].as_array().expect("pilot_sequence");
    let _ = writeln!(
        out,
        "static {prefix}_PILOT_SEQUENCE: [(f64, f64); {}] = [",
        pilots.len()
    );
    for point in pilots {
        let (re, im) = (
            point[0].as_f64().expect("re"),
            point[1].as_f64().expect("im"),
        );
        let _ = writeln!(out, "    ({re:?}, {im:?}),");
    }
    out.push_str("];\n");
    let _ = writeln!(
        out,
        "pub(crate) static {prefix}: Tables = Tables {{\n    bandwidth_hz: {bandwidth},\n    \
         sc_length: {sc_len},\n    chip_length: {chip_len},\n    n_modes: {n_modes},\n    \
         chip_correlation_bound: {:?},\n    acquisition_threshold: {:?},\n    \
         even_carriers: &{prefix}_EVEN_CARRIERS,\n    sc_data: &{prefix}_SC_DATA,\n    \
         sc_control: &{prefix}_SC_CONTROL,\n    mode_chips: &{prefix}_MODE_CHIPS,\n    \
         pilot_sequence: &{prefix}_PILOT_SEQUENCE,\n    \
         control_mode_index: {control_mode_index},\n    \
         ofdm_ladder: &{prefix}_OFDM_LADDER,\n}};",
        doc["chip_correlation_bound"]
            .as_f64()
            .expect("chip_correlation_bound"),
        doc["acquisition_threshold"]
            .as_f64()
            .expect("acquisition_threshold"),
    );
}

/// A tone-floor kind as a `ToneKind` literal.
fn tone_kind(doc: &serde_json::Value) -> String {
    format!(
        "ToneKind {{ name: {:?}, payload_bytes: {}, data_symbols: {}, patterns: &[{}], \
         control: {} }}",
        doc["name"].as_str().expect("name"),
        doc["payload_bytes"].as_u64().expect("payload_bytes"),
        doc["data_symbols"].as_u64().expect("data_symbols"),
        usizes(&doc["patterns"]).join(", "),
        doc["control"].as_bool().expect("control"),
    )
}

/// The tone floor's block (ADR-0013).
fn tone(out: &mut String, doc: &serde_json::Value) {
    let f = |key: &str| doc[key].as_f64().unwrap_or_else(|| panic!("tone {key}"));
    let u = |key: &str| doc[key].as_u64().unwrap_or_else(|| panic!("tone {key}"));
    let det = &doc["detector"];
    let df = |key: &str| {
        det[key]
            .as_f64()
            .unwrap_or_else(|| panic!("detector {key}"))
    };
    let du = |key: &str| {
        det[key]
            .as_u64()
            .unwrap_or_else(|| panic!("detector {key}"))
    };
    let patterns = doc["sync_patterns"].as_array().expect("sync_patterns");
    let sync = u("sync_symbols");
    let _ = writeln!(
        out,
        "static TONE_SYNC_PATTERNS: [[usize; {sync}]; {}] = [",
        patterns.len()
    );
    for p in patterns {
        let _ = writeln!(out, "    [{}],", usizes(p).join(", "));
    }
    out.push_str("];\n");
    let data: Vec<String> = doc["data"]
        .as_array()
        .expect("tone data kinds")
        .iter()
        .map(tone_kind)
        .collect();
    let _ = writeln!(
        out,
        "static TONE_DATA_KINDS: [ToneKind; {}] = [\n    {},\n];",
        data.len(),
        data.join(",\n    ")
    );
    let _ = writeln!(
        out,
        "pub(crate) static TONE: ToneTables = ToneTables {{\n    fs: {:?},\n    \
         symbol_samples: {},\n    tones: {},\n    ramp_samples: {},\n    edge_samples: {},\n    \
         gain_db: {:?},\n    sync_symbols: {sync},\n    sync_patterns: &TONE_SYNC_PATTERNS,\n    \
         control: {},\n    data: &TONE_DATA_KINDS,\n    hop_div: {},\n    bin_div: {},\n    \
         clip: {:?},\n    max_cfo_hz: {:?},\n    threshold: {:?},\n    min_hits: {},\n    \
         min_block_hits: {},\n    min_first_hits: {},\n    announce_threshold: {:?},\n    \
         lookahead: {},\n    \
         announce_lookahead: {},\n}};",
        f("fs"),
        u("symbol_samples"),
        u("tones"),
        u("ramp_samples"),
        u("edge_samples"),
        f("gain_db"),
        tone_kind(&doc["control"]),
        du("hop_div"),
        du("bin_div"),
        df("clip"),
        df("max_cfo_hz"),
        df("threshold"),
        du("min_hits"),
        du("min_block_hits"),
        du("min_first_hits"),
        df("announce_threshold"),
        du("lookahead"),
        du("announce_lookahead"),
    );
}

fn main() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/preamble_tables.json");
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}); run tools/make_phy_tables.py",
            path.display()
        )
    });
    let doc: serde_json::Value = serde_json::from_str(&text).expect("preamble table is not JSON");
    let n_rv = doc["n_rv"].as_u64().expect("n_rv") as usize;

    let mut out = String::new();
    out.push_str("// @generated by build.rs from data/preamble_tables.json - do not edit.\n\n");
    let _ = writeln!(out, "pub(crate) const N_RV: usize = {n_rv};");
    out.push_str(
        "/// One waveform's air-interface constants.\n\
         #[derive(Debug)]\n\
         pub(crate) struct Tables {\n    \
             pub bandwidth_hz: usize,\n    \
             pub sc_length: usize,\n    \
             pub chip_length: usize,\n    \
             pub n_modes: usize,\n    \
             pub chip_correlation_bound: f64,\n    \
             pub acquisition_threshold: f64,\n    \
             pub even_carriers: &'static [usize],\n    \
             pub sc_data: &'static [f64],\n    \
             pub sc_control: &'static [f64],\n    \
             /// Row-major: sequence `i` is `[i * chip_length .. (i + 1) * chip_length]`.\n    \
             pub mode_chips: &'static [f64],\n    \
             pub pilot_sequence: &'static [(f64, f64)],\n    \
             pub control_mode_index: usize,\n    \
             /// The OFDM modes on the ladder, ascending, above the tone floor's rungs.\n    \
             pub ofdm_ladder: &'static [usize],\n\
         }\n\n\
         /// One kind of tone-floor frame (ADR-0013): what it carries, how long it is, and the\n\
         /// sync patterns that name it, one per redundancy version.\n\
         #[derive(Debug, PartialEq, Eq)]\n\
         pub struct ToneKind {\n    \
             /// Its name, as the model and the reports give it.\n    \
             pub name: &'static str,\n    \
             /// Payload bytes a frame carries.\n    \
             pub payload_bytes: usize,\n    \
             /// Symbols of coded data.\n    \
             pub data_symbols: usize,\n    \
             /// Indices into the sync patterns, by redundancy version.\n    \
             pub patterns: &'static [usize],\n    \
             /// Whether it is the control frame.\n    \
             pub control: bool,\n\
         }\n\n\
         /// The tone floor's constants (ADR-0013).\n\
         #[derive(Debug)]\n\
         pub(crate) struct ToneTables {\n    \
             pub fs: f64,\n    \
             pub symbol_samples: usize,\n    \
             pub tones: usize,\n    \
             pub ramp_samples: usize,\n    \
             pub edge_samples: usize,\n    \
             pub gain_db: f64,\n    \
             pub sync_symbols: usize,\n    \
             pub sync_patterns: &'static [[usize; 8]],\n    \
             pub control: ToneKind,\n    \
             pub data: &'static [ToneKind],\n    \
             pub hop_div: usize,\n    \
             pub bin_div: usize,\n    \
             pub clip: f64,\n    \
             pub max_cfo_hz: f64,\n    \
             pub threshold: f64,\n    \
             pub min_hits: usize,\n    \
             pub min_block_hits: usize,\n    \
             pub min_first_hits: usize,\n    \
             pub announce_threshold: f64,\n    \
             pub lookahead: usize,\n    \
             pub announce_lookahead: usize,\n\
         }\n\n",
    );
    let waveforms = doc["waveforms"].as_object().expect("waveforms");
    let mut names = Vec::new();
    for (name, block) in waveforms {
        waveform(&mut out, name, block, n_rv);
        names.push(name.clone());
    }
    tone(&mut out, &doc["tone"]);
    let _ = writeln!(
        out,
        "/// The tables of the waveform with this nominal bandwidth, if it has any.\n\
         pub(crate) fn for_bandwidth(hz: usize) -> Option<&'static Tables> {{\n    \
             [{}].into_iter().find(|t| t.bandwidth_hz == hz)\n\
         }}",
        names
            .iter()
            .map(|n| format!("&{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("preamble_tables.rs");
    fs::write(&dest, out).expect("cannot write generated preamble tables");
}
