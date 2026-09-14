//! Aether HF daemon.
//!
//! The parts that turn the modem into a station: keying a transmitter and refusing to leave
//! it keyed, deciding whether the channel is free, moving audio to and from a sound card, and
//! the loop that ties the link engine to the physical layer.
//!
//! Everything that can be tested without hardware is separated from everything that cannot.
//! `ptt` and `busy` are pure logic over an injected clock and injected samples, so the rules
//! that keep a station lawful and polite are checked in the test suite rather than on the
//! air.

pub mod busy;
pub mod ptt;

pub use busy::{BusyConfig, BusyDetector};
pub use ptt::{NullPtt, Ptt, PttError, PttWatchdog, RigctldPtt, WatchdogState};
