//! The machine's audio devices and serial ports, listed off the run loop and kept.
//!
//! Listing is slow: every device is opened through the platform's audio API, and on Windows a
//! device's formats used to be asked for one at a time (16.6 s for twelve devices on the
//! author's machine, 11.3 s of it one laptop output). The run loop is one thread, and it listed
//! at every profile switch, save and import and every time a panel connected: the modem stood
//! still that long, dropped captured audio ("the modem is behind"), and the panel's call timed
//! out while the switch went on without it (2026-09-26). The daemon now lists at start and
//! whenever a client asks for the list, on a thread of its own, and everything that checks
//! against the devices reads the last listing.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use serde_json::Value;

/// How long a caller waits for the first listing when none has finished yet — at the start
/// only, when the listing begun there is still under way.
pub const FIRST_WAIT: Duration = Duration::from_secs(5);

/// The listing kept, and whether another is under way.
#[derive(Debug, Default)]
struct State {
    /// The last listing to finish.
    last: Option<Value>,
    /// A listing is under way on its own thread.
    busy: bool,
    /// Whether a listing was ever begun: a daemon that began none (a test's) lists when asked.
    begun: bool,
    /// Listings finished: a change is a new list that the clients may not have seen.
    finished: u64,
}

/// The machine's devices, listed in the background. Clones share one list.
#[derive(Debug, Clone, Default)]
pub struct DeviceList {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl DeviceList {
    /// Begin a listing with `source` on a thread of its own, unless one is under way — the
    /// one under way answers for this too.
    pub fn refresh(&self, source: fn() -> Value) {
        let (lock, _) = &*self.shared;
        {
            let Ok(mut state) = lock.lock() else {
                return;
            };
            state.begun = true;
            if state.busy {
                return;
            }
            state.busy = true;
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("devices".into())
            .spawn(move || {
                let listing = source();
                let (lock, done) = &*shared;
                if let Ok(mut state) = lock.lock() {
                    state.last = Some(listing);
                    state.busy = false;
                    state.finished += 1;
                }
                done.notify_all();
            });
        if spawned.is_err()
            && let Ok(mut state) = lock.lock()
        {
            // no thread: nothing is under way, and the next ask begins again
            state.busy = false;
        }
    }

    /// Whether a listing was ever begun.
    #[must_use]
    pub fn begun(&self) -> bool {
        self.shared.0.lock().is_ok_and(|state| state.begun)
    }

    /// The last listing to finish. When none has and one is under way, this waits for it up
    /// to `wait`; `None` if there is still none.
    #[must_use]
    pub fn last(&self, wait: Duration) -> Option<Value> {
        let (lock, done) = &*self.shared;
        let state = lock.lock().ok()?;
        if state.last.is_some() || !state.busy {
            return state.last.clone();
        }
        let (state, _) = done
            .wait_timeout_while(state, wait, |state| state.last.is_none() && state.busy)
            .ok()?;
        state.last.clone()
    }

    /// How many listings have finished: the run loop tells the clients when this moves and
    /// the list is not the one they were last given.
    #[must_use]
    pub fn finished(&self) -> u64 {
        self.shared.0.lock().map_or(0, |state| state.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn slow() -> Value {
        std::thread::sleep(Duration::from_millis(150));
        json!({"devices": [{"name": "USB Audio CODEC", "input": true, "output": true}]})
    }

    #[test]
    fn a_listing_is_made_off_the_callers_thread_and_kept() {
        let list = DeviceList::default();
        assert!(!list.begun());
        assert_eq!(
            list.last(Duration::ZERO),
            None,
            "nothing listed, nothing waited for"
        );
        let asked = std::time::Instant::now();
        list.refresh(slow);
        assert!(
            asked.elapsed() < Duration::from_millis(100),
            "the caller waited for the listing"
        );
        assert!(list.begun());
        // the first listing is waited for, the next ones are not
        let first = list.last(FIRST_WAIT).expect("the first listing");
        assert_eq!(first["devices"][0]["name"], "USB Audio CODEC");
        assert_eq!(list.finished(), 1);
        list.refresh(slow);
        let asked = std::time::Instant::now();
        assert_eq!(list.last(FIRST_WAIT), Some(first), "the last listing, kept");
        assert!(asked.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn a_listing_under_way_answers_for_a_second_ask() {
        let list = DeviceList::default();
        list.refresh(slow);
        list.refresh(slow);
        let _ = list.last(FIRST_WAIT);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(list.finished(), 1, "two listings for one");
    }

    #[test]
    fn the_wait_for_the_first_listing_is_bounded() {
        let list = DeviceList::default();
        list.refresh(slow);
        assert_eq!(list.last(Duration::from_millis(10)), None);
    }
}
