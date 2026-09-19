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

pub mod audio;
pub mod busy;
pub mod compress;
pub mod config;
pub mod control;
pub mod cwid;
pub mod grid;
pub mod heard;
pub mod host;
pub mod log;
pub mod memories;
pub mod profile;
pub mod ptt;
pub mod record;
pub mod replay;
pub mod settings;
pub mod sim;
pub mod spectrum;
pub mod station;

pub use audio::{AudioConfig, AudioError, AudioIo, Loopback, SoundCard};
pub use busy::{BusyConfig, BusyDetector};
pub use compress::{CAP_DEFLATE, Compressor, Decompressor};
pub use config::{Config, ConfigError, PttConfig};
pub use control::{ControlHandle, ControlServer};
pub use cwid::CwId;
pub use host::{HostConfig, HostServer};
pub use ptt::{
    NullPtt, Ptt, PttError, PttWatchdog, RigctldPtt, SerialLine, SerialPtt, WatchdogState,
};
pub use station::{PhyFrame, Station, StationConfig, StationStats, phy_timing};
