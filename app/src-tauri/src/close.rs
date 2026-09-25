//! Closing the window in the middle of something.
//!
//! Closing the desktop application stops the modem it started, and with it whatever the
//! modem was doing. ND1J closed Aether HF during a Test session (2026-09-25): the other
//! station was left sending into nothing until its link timed out, and the end of the
//! session was never identified. The daemon now ends a session on the air before it stops
//! (a DISC, and the identifier when it sends one); this asks the operator first, naming
//! what would be cut short, and lets them keep it running.

use serde_json::Value;

/// What closing now would interrupt, one line each, read from the daemon's `status`. Empty
/// when the modem is idle and silent: then the window closes without a question.
///
/// A recording or an attached host program is named only beside something else — an
/// operator closing an idle modem with Winlink attached has not asked to be stopped.
#[must_use]
pub fn interruptions(status: &Value) -> Vec<String> {
    let field = |key: &str| status[key].as_str().unwrap_or_default().to_owned();
    let remote = field("remote");
    let state = field("state");
    let probing = status["probing"].as_bool() == Some(true);
    let mut lines = Vec::new();
    let test = &status["test"];
    if test.is_object() {
        let with = test["remote"]
            .as_str()
            .map_or_else(|| remote.clone(), str::to_owned);
        match test["step"].as_str() {
            Some(step) => lines.push(format!("the Test session with {with}, at its {step} step")),
            None => lines.push(format!("the Test session with {with}")),
        }
    }
    match state.as_str() {
        "connected" => match status["queued_bytes"].as_u64() {
            Some(queued) if queued > 0 => lines.push(format!(
                "the session with {remote}, with {queued} bytes still to send"
            )),
            _ => lines.push(format!("the session with {remote}")),
        },
        "connecting" => lines.push(format!("the call to {remote}")),
        "disconnecting" => lines.push(format!("the disconnect from {remote}, still under way")),
        _ => {}
    }
    if probing {
        lines.push(format!("the probe of {remote}"));
    }
    if status["transmitting"].as_bool() == Some(true) && state == "idle" && !probing {
        lines.push("a transmission on the air now (a beacon, a tune tone or a keying test)".into());
    }
    if !lines.is_empty() {
        if status["recording"].is_object() {
            lines.push("the recording in progress (what it holds so far is kept)".into());
        }
        if status["host"]["connected"].as_bool() == Some(true) {
            lines.push("the host program attached to the modem, which loses it".into());
        }
    }
    lines
}

/// The question, with what would be interrupted and what closing does about it.
#[must_use]
pub fn message(interrupted: &[String]) -> String {
    let list: Vec<String> = interrupted
        .iter()
        .map(|line| format!("•  {line}"))
        .collect();
    format!(
        "Closing Aether HF stops the modem, and with it:\n\n{}\n\nIf you close, the modem \
         ends a session on the air before it stops — a disconnect, and its callsign in Morse \
         when it is set to identify — which can take a quarter of a minute after the window \
         has gone.",
        list.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn an_idle_modem_closes_without_a_question() {
        let idle = json!({
            "state": "idle", "remote": "", "transmitting": false, "probing": false,
            "test": null, "recording": null, "host": {"connected": true},
        });
        assert!(
            interruptions(&idle).is_empty(),
            "an attached host alone is not a question"
        );
        // a daemon too old to say whether it probes is read as not probing
        assert!(interruptions(&json!({"state": "idle"})).is_empty());
    }

    #[test]
    fn a_session_a_test_and_what_rides_with_them_are_named() {
        let busy = json!({
            "state": "connected", "remote": "ND1J", "transmitting": true, "probing": false,
            "queued_bytes": 1024,
            "test": {"remote": "ND1J", "step": "ladder"},
            "recording": {"path": "x.wav", "seconds": 12.0},
            "host": {"connected": true},
        });
        let lines = interruptions(&busy);
        assert_eq!(
            lines,
            vec![
                "the Test session with ND1J, at its ladder step",
                "the session with ND1J, with 1024 bytes still to send",
                "the recording in progress (what it holds so far is kept)",
                "the host program attached to the modem, which loses it",
            ]
        );
        let text = message(&lines);
        assert!(text.contains("•  the session with ND1J"));
        assert!(text.contains("a disconnect"));
    }

    #[test]
    fn a_call_a_probe_and_a_lone_transmission_are_each_a_question() {
        let calling = json!({"state": "connecting", "remote": "W4TGA", "transmitting": true});
        assert_eq!(interruptions(&calling), vec!["the call to W4TGA"]);
        let probing =
            json!({"state": "idle", "remote": "W4TGA", "transmitting": true, "probing": true});
        assert_eq!(interruptions(&probing), vec!["the probe of W4TGA"]);
        let tuning = json!({"state": "idle", "remote": "", "transmitting": true});
        assert_eq!(interruptions(&tuning).len(), 1);
        assert!(interruptions(&tuning)[0].starts_with("a transmission on the air"));
        let ending = json!({"state": "disconnecting", "remote": "ND1J"});
        assert_eq!(
            interruptions(&ending),
            vec!["the disconnect from ND1J, still under way"]
        );
    }
}
