//! Debug mode: the sessions a host program ran, sent to the Aether project (ADR-0050).
//!
//! Field trials through a Winlink gateway on Aether need both sides of every session, and
//! asking a station for its files after the fact found the logs gone (ND1J, 2026-09-29) or
//! cost a PowerShell line in an email (KE4QCM, 2026-10-05). With `record.send_to_project` on
//! — the default during the trials — every session a host program runs is recorded, and a
//! minute after it ends the daemon writes one zip of its recording, audio included, the logs
//! and the settings without their secrets, and uploads it to the project's script
//! (`upload::PROJECT_ENDPOINT`) on the upload's own thread.
//!
//! This is the queue that decides *when*: only while the station is idle, never under an
//! upload of the operator's own, a minute after the last session so a gateway's back-to-back
//! sessions go together, and again after half an hour when one fails — three times, then the
//! files stay on this computer and the log says so. Nothing here touches the network or the
//! disk; the run loop does what it says.

use serde::Serialize;

/// How long after the last session ended before its files go: a client that calls again at
/// once (Winlink Express after a failed session) puts both sessions in one zip.
pub const QUIET_MS: u64 = 60_000;

/// How long before a failed upload is tried again, times the tries so far.
pub const RETRY_MS: u64 = 30 * 60_000;

/// Tries before the files are left on this computer.
pub const MAX_TRIES: u32 = 3;

/// The most sessions in one zip: a gateway's busy evening goes in several.
pub const MAX_PER_ZIP: usize = 6;

/// The most sessions waiting: past it the oldest is left on this computer.
pub const MAX_WAITING: usize = 24;

/// A session whose files are to go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pending {
    /// Its recording, by file stem.
    pub recording: String,
    /// The other station.
    pub remote: String,
    /// When it came up, milliseconds since the Unix epoch.
    pub started_ms: u64,
}

/// The queue, and what became of the last upload.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DebugUploads {
    /// Waiting to go.
    pub waiting: Vec<Pending>,
    /// In the upload under way.
    pub sending: Vec<Pending>,
    /// Sessions sent since the daemon started.
    pub sent: u32,
    /// Why the last upload failed, while it is still to be tried again.
    pub last_error: Option<String>,
    /// When the last session ended.
    #[serde(skip)]
    last_session_ms: u64,
    /// Failed tries of what is waiting at the front.
    #[serde(skip)]
    tries: u32,
    /// Not before this, after a failure.
    #[serde(skip)]
    retry_at_ms: Option<u64>,
}

impl DebugUploads {
    /// A session ended whose files are to go. The oldest waiting goes past the limit — it is
    /// returned, for the log to say so.
    pub fn session_ended(&mut self, pending: Pending, now_ms: u64) -> Option<Pending> {
        self.last_session_ms = now_ms;
        self.waiting.push(pending);
        (self.waiting.len() > MAX_WAITING).then(|| self.waiting.remove(0))
    }

    /// Whether the next zip is due: something waits, nothing is on its way, the station is
    /// idle and has been quiet a minute, and a failure's wait is over.
    #[must_use]
    pub fn due(&self, now_ms: u64, idle: bool, uploading: bool) -> bool {
        !self.waiting.is_empty()
            && self.sending.is_empty()
            && idle
            && !uploading
            && now_ms >= self.last_session_ms.saturating_add(QUIET_MS)
            && self.retry_at_ms.is_none_or(|at| now_ms >= at)
    }

    /// The sessions for the next zip, now on their way.
    pub fn take(&mut self) -> Vec<Pending> {
        let n = self.waiting.len().min(MAX_PER_ZIP);
        self.sending = self.waiting.drain(..n).collect();
        self.sending.clone()
    }

    /// The upload under way is over. Sent, the sessions go; failed, they wait at the front
    /// for another try, unless they have had their tries — then they are returned, left on
    /// this computer.
    pub fn finished(&mut self, error: Option<String>, now_ms: u64) -> Vec<Pending> {
        let sending = std::mem::take(&mut self.sending);
        match error {
            None => {
                self.sent += u32::try_from(sending.len()).unwrap_or(u32::MAX);
                self.tries = 0;
                self.retry_at_ms = None;
                self.last_error = None;
                Vec::new()
            }
            Some(error) => {
                self.tries += 1;
                if self.tries >= MAX_TRIES {
                    self.tries = 0;
                    self.retry_at_ms = None;
                    self.last_error = Some(error);
                    return sending;
                }
                self.retry_at_ms = Some(now_ms + RETRY_MS * u64::from(self.tries));
                self.last_error = Some(error);
                let mut waiting = sending;
                waiting.append(&mut self.waiting);
                self.waiting = waiting;
                Vec::new()
            }
        }
    }

    /// Whether an upload of the queue's is under way.
    #[must_use]
    pub fn sending(&self) -> bool {
        !self.sending.is_empty()
    }

    /// Forget what waits: debug mode was turned off.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.tries = 0;
        self.retry_at_ms = None;
        self.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(call: &str, at: u64) -> Pending {
        Pending {
            recording: format!("rec-{call}-{at}"),
            remote: call.into(),
            started_ms: at,
        }
    }

    #[test]
    fn a_session_goes_a_minute_after_the_last_one_and_only_while_idle() {
        let mut queue = DebugUploads::default();
        assert!(!queue.due(0, true, false), "nothing waits");
        queue.session_ended(session("W4TGA", 0), 1_000);
        assert!(!queue.due(30_000, true, false), "too soon after it ended");
        // another session at once: both wait for the quiet after the second
        queue.session_ended(session("ND1J", 40_000), 50_000);
        assert!(!queue.due(1_000 + QUIET_MS, true, false));
        let later = 50_000 + QUIET_MS;
        assert!(!queue.due(later, false, false), "not during a session");
        assert!(
            !queue.due(later, true, true),
            "not over the operator's own upload"
        );
        assert!(queue.due(later, true, false));
        let taken = queue.take();
        assert_eq!(taken.len(), 2);
        assert!(!queue.due(later, true, false), "one upload at a time");
        let left = queue.finished(None, later);
        assert!(left.is_empty(), "{left:?}");
        assert_eq!(queue.sent, 2);
        assert!(queue.waiting.is_empty() && !queue.sending());
    }

    #[test]
    fn a_failure_waits_longer_each_time_and_then_leaves_the_files_here() {
        let mut queue = DebugUploads::default();
        queue.session_ended(session("KE4QCM", 0), 0);
        let mut now = QUIET_MS;
        for tries in 1..MAX_TRIES {
            assert!(queue.due(now, true, false));
            queue.take();
            let left = queue.finished(Some("offline".into()), now);
            assert!(left.is_empty(), "{left:?}");
            assert_eq!(queue.last_error.as_deref(), Some("offline"));
            let wait = RETRY_MS * u64::from(tries);
            assert!(!queue.due(now + wait - 1, true, false));
            now += wait;
        }
        assert!(queue.due(now, true, false));
        queue.take();
        let left = queue.finished(Some("offline".into()), now);
        assert_eq!(left, [session("KE4QCM", 0)]);
        assert!(queue.waiting.is_empty(), "{:?}", queue.waiting);
        // a new session starts afresh
        queue.session_ended(session("ND1J", now), now);
        assert!(queue.due(now + QUIET_MS, true, false));
    }

    #[test]
    fn a_busy_gateway_sends_in_several_zips_and_keeps_a_bounded_queue() {
        let mut queue = DebugUploads::default();
        let mut dropped = Vec::new();
        for i in 0..(MAX_WAITING as u64 + 2) {
            dropped.extend(queue.session_ended(session("N0CALL", i), i));
        }
        assert_eq!(dropped, [session("N0CALL", 0), session("N0CALL", 1)]);
        assert_eq!(queue.take().len(), MAX_PER_ZIP);
        assert_eq!(queue.waiting.len(), MAX_WAITING - MAX_PER_ZIP);
    }
}
