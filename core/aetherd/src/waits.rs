//! What the run loop waited on outside the modem in the pass under way: the radio's keying
//! interface, the recording on disk and the sound card. A slow pass names the one that held it
//! (2026-10-10: a 17.5 s pass in KK4ODA-1's capture phase during broadband RFI, the receiver
//! cheap on the same audio, and nothing to say whether the CAT port, the disk or the sound card
//! had stopped answering).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// One thing the loop can wait on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wait {
    /// The keying interface: CAT, a serial line, `rigctld`, `FLRig`, a GPIO.
    Radio,
    /// Writing the recording.
    Recording,
    /// Reading the captured audio from the sound card.
    Card,
}

static RADIO_US: AtomicU64 = AtomicU64::new(0);
static RECORDING_US: AtomicU64 = AtomicU64::new(0);
static CARD_US: AtomicU64 = AtomicU64::new(0);

fn counter(which: Wait) -> &'static AtomicU64 {
    match which {
        Wait::Radio => &RADIO_US,
        Wait::Recording => &RECORDING_US,
        Wait::Card => &CARD_US,
    }
}

/// Run `work`, counting how long it took against `which`.
pub fn timed<T>(which: Wait, work: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    let out = work();
    let us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX);
    counter(which).fetch_add(us, Ordering::Relaxed);
    out
}

/// What was waited on since the last call, milliseconds: radio, recording, sound card.
#[must_use]
pub fn take() -> [f64; 3] {
    [Wait::Radio, Wait::Recording, Wait::Card]
        .map(|which| counter(which).swap(0, Ordering::Relaxed) as f64 / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_is_counted_once_against_its_own_kind() {
        let _ = take();
        timed(Wait::Recording, || {
            std::thread::sleep(std::time::Duration::from_millis(20));
        });
        // other tests run beside this one and count their own waits: only this one's is sure
        let [_, recording, _] = take();
        assert!(recording >= 19.0, "{recording}");
    }
}
