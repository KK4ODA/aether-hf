//! The field regression tier: every recorded session in `field/sessions/` replays through
//! the receiver, and every frame that decoded on the day has to decode now.
//!
//! A session gets here from `aetherd --replay` on a recording that is worth keeping — a
//! path, a band, a condition the simulator does not reproduce — with the WAV and the
//! sidecar committed together. The receiver may improve; it may not lose a frame it once
//! had. With no sessions yet the test passes and says so.

use std::path::PathBuf;

use aetherd::replay::{Expectation, compare, replay};

fn sessions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../field/sessions")
}

#[test]
fn every_recorded_session_still_decodes_what_it_decoded_on_the_day() {
    let dir = sessions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("no field sessions at {}", dir.display());
        return;
    };
    let mut sidecars: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    sidecars.sort();
    let mut failures = Vec::new();
    for sidecar in &sidecars {
        let wav = sidecar.with_extension("wav");
        let expectation = Expectation::from_sidecar(sidecar).expect("a session sidecar");
        let found = replay(&wav, &expectation.muted, expectation.bandwidth_hz)
            .expect("a readable recording");
        let verdict = compare(&expectation.frames, &found);
        eprintln!(
            "{}: recorded {} decoded, replay decoded {} of {} found",
            sidecar.file_name().unwrap().to_string_lossy(),
            verdict.recorded,
            verdict.replayed,
            verdict.found
        );
        if !verdict.holds() {
            failures.push(format!(
                "{}: {} decoded on the day, {} now",
                sidecar.display(),
                verdict.recorded,
                verdict.replayed
            ));
        }
    }
    eprintln!("{} field session(s) replayed", sidecars.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
