//! Host interfaces: how other people's software talks to this modem.
//!
//! * [`vara`] — the published VARA-compatible command protocol, as a state machine with no
//!   sockets in it, so the whole command surface is tested by calling it.
//! * [`server`] — the two TCP ports, wired to the modem through its own control API.
//!
//! Compatibility here is at the *host* interface only. The air interface is Aether's own and
//! is specified in `docs/spec/air-interface.md`; two Aether stations talk to each other, never
//! to VARA. Nothing in this module is derived from VARA's internals.

pub mod server;
pub mod vara;

pub use server::{HostConfig, HostError, HostServer};
pub use vara::{Compression, HostAction, HostState, Notification, SessionKind};
