//! KISS over TCP: the interface packet and APRS software uses to talk to a TNC (ADR-0019).
//!
//! * [`framing`] — the KISS byte stream: frames, escapes and the type byte, decoded from
//!   pieces as TCP delivers them.
//! * [`dialect`] — what a frame means, read the VARA-compatible way: VARA's frame types 0, 1
//!   and 2 beside the standard parameters, and ACKMODE.
//! * [`server`] — the TCP port, a client of the control API like the VARA host interface.
//!
//! Compatibility here is with the programs that use VARA's KISS port — APRS clients, `VarAC`'s
//! broadcasts, packet programs — at the *host* interface. On the air a KISS frame is an Aether
//! datagram, which Aether stations hear and VARA stations do not.

pub mod dialect;
pub mod framing;
pub mod server;

pub use server::{
    DEFAULT_PERSISTENCE, DEFAULT_PORT, DEFAULT_SLOT_S, HostFlags, KissConfig, KissServer,
    KissStatus, Note,
};
