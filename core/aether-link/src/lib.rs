//! Aether HF link layer.
//!
//! The production port of `model/aether_model/link/`. The Python model remains the
//! specification (ADR-0001), and `tests/model_vectors.rs` checks this crate against vectors
//! it generates.
//!
//! Frame formats and the rate controller are here. The ARQ engine and session state machine
//! are still to come; until they are, this crate can build and parse every frame the protocol
//! uses and decide which mode to ask for, but it cannot run a session.

pub mod frames;
pub mod rate;

pub use frames::{
    CONTROL_BYTES, ConnectBody, ControlFrame, ControlKind, DATA_HEADER, DataHeader, DataKind,
    FrameError, WINDOW,
};
pub use rate::{AWGN_THRESHOLD_DB, RateConfig, RateController, usable_modes};
