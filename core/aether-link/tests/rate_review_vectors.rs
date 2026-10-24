//! The rate controller review's prototype C2 (Gate 3, bench-only) against the model, input by
//! input: `tests/data/rate_review_vectors.json`'s controller traces, written by
//! `tools/make_rate_review_vectors.py` from `aether_model.link.rate_review.OuterLoopController`.
//! The same file's engine sessions are replayed by `granted_vectors.rs`.

use std::{fs, path::PathBuf};

use aether_link::rate_review::{FrameOutcome, OuterLoopController};
use serde_json::Value;

fn vectors() -> Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/rate_review_vectors.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}).\nGenerate it with: python tools/make_rate_review_vectors.py",
            path.display()
        )
    });
    serde_json::from_str(&text).expect("vector file is not valid JSON")
}

fn num(v: &Value) -> f64 {
    v.as_f64().expect("number")
}

fn opt(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn int(v: &Value) -> usize {
    usize::try_from(v.as_u64().expect("integer")).expect("fits")
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array().expect("array").iter().map(num).collect()
}

fn ints(v: &Value) -> Vec<usize> {
    v.as_array().expect("array").iter().map(int).collect()
}

fn close_opt(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() <= 1e-9 * x.abs().max(1.0),
        _ => false,
    }
}

#[test]
fn the_prototype_decides_as_the_model_does() {
    let doc = vectors();
    let traces = doc["controller_traces"]
        .as_array()
        .expect("controller traces");
    assert!(traces.len() >= 2, "too few traces to mean anything");
    for (n, trace) in traces.iter().enumerate() {
        let echo_limit: Vec<Option<f64>> = trace["echo_limit"]
            .as_array()
            .expect("echo limit")
            .iter()
            .map(opt)
            .collect();
        let mut ctl = OuterLoopController::for_table(
            &floats(&trace["thresholds"]),
            &ints(&trace["payload"]),
            &ints(&trace["capacity"]),
            &floats(&trace["frame_s"]),
            int(&trace["floor_modes"]),
            &echo_limit,
        );
        assert_eq!(
            ctl.modes(),
            ints(&trace["modes"]).as_slice(),
            "trace {n}: the usable rungs"
        );
        let targets = floats(&trace["targets"]);
        assert!(
            close_opt(Some(ctl.target(true)), Some(targets[0])),
            "trace {n}: floor target"
        );
        assert!(
            close_opt(Some(ctl.target(false)), Some(targets[1])),
            "trace {n}: target"
        );
        for (i, op) in trace["ops"].as_array().expect("ops").iter().enumerate() {
            if let Some(snr) = op.get("seed") {
                ctl.seed(num(snr));
            } else if let Some(frames) = op.get("frames") {
                let frames: Vec<FrameOutcome> = frames
                    .as_array()
                    .expect("frames")
                    .iter()
                    .map(|f| FrameOutcome {
                        mode: int(&f[0]),
                        air_s: num(&f[1]),
                        capacity: int(&f[2]),
                        detected: f[3].as_bool().expect("detected"),
                        decoded: f[4].as_bool().expect("decoded"),
                        combined: f[5].as_bool().expect("combined"),
                    })
                    .collect();
                ctl.observe_frames(&frames, opt(&op["snr_in"]));
            } else if let Some(share) = op.get("echo_in") {
                ctl.observe_echo(opt(share), num(&op["snr_in"]));
            } else if let Some(snr) = op.get("observe_snr") {
                ctl.observe_snr(opt(snr));
            } else {
                panic!("trace {n} op {i}: unknown input");
            }
            let at = format!("trace {n} op {i}");
            assert_eq!(ctl.recommend(), int(&op["recommend"]), "{at}: the rung");
            let [floor, ordinary] = ctl.offsets();
            assert!(
                close_opt(floor, opt(&op["floor_offset"])),
                "{at}: floor offset {floor:?}"
            );
            assert!(
                close_opt(ordinary, opt(&op["offset"])),
                "{at}: offset {ordinary:?}"
            );
            assert!(
                close_opt(ctl.snr_db(), opt(&op["snr"])),
                "{at}: smoothed SNR"
            );
            assert!(close_opt(ctl.echo(), opt(&op["echo"])), "{at}: echo");
        }
    }
}
