//! Aether HF link layer.
//!
//! The production port of `model/aether_model/link/`. The Python model remains the
//! specification (ADR-0001), and `tests/model_vectors.rs` and `tests/granted_vectors.rs` check
//! this crate against vectors it generates.
//!
//! * [`frames`] — the wire formats, data and control.
//! * [`rate`] — the receiver-side mode recommendation.
//! * [`phy`] — what the engine needs from a physical layer, and nothing more.
//! * [`granted`] — the link engine, granted slots (ADR-0069).
//! * [`datagram`] — another program's frames outside sessions (ADR-0019).

pub mod datagram;
pub mod frames;
pub mod granted;
pub mod phy;
pub mod rate;

pub use frames::{
    CONTROL_BYTES, ConnectBody, DATA_HEADER, DataHeader, DataKind, FrameError, GrantFrame,
    GrantKind, ProbeBody, WINDOW,
};
pub use granted::{Action, GrantConfig, GrantStats, GrantedEngine, LadderRung, ProbeResult, State};
pub use phy::{Container, HarqBuffer, PhyTiming, SoftFrame, TxFrame};
pub use rate::{AWGN_THRESHOLD_DB, RateConfig, RateController, usable_modes};
