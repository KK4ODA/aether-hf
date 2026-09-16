//! Aether HF link layer.
//!
//! The production port of `model/aether_model/link/`. The Python model remains the
//! specification (ADR-0001), and `tests/model_vectors.rs` checks this crate against vectors
//! it generates.
//!
//! * [`frames`] — the wire formats, data and control.
//! * [`rate`] — the receiver-side mode recommendation.
//! * [`phy`] — what the engine needs from a physical layer, and nothing more.
//! * [`engine`] — the ARQ engine and session state machine.
//! * [`sim`] — two engines over a lossy pipe, for testing the protocol without DSP.

pub mod engine;
pub mod frames;
pub mod phy;
pub mod rate;
pub mod sim;

pub use engine::{Action, LadderRung, LinkConfig, LinkEngine, LinkStats, ProbeResult, Role, State};
pub use frames::{
    CONTROL_BYTES, ConnectBody, ControlFrame, ControlKind, DATA_HEADER, DataHeader, DataKind,
    FrameError, ProbeBody, WINDOW,
};
pub use phy::{Container, HarqBuffer, PhyTiming, SoftFrame, TxFrame};
pub use rate::{AWGN_THRESHOLD_DB, RateConfig, RateController, usable_modes};
pub use sim::{SimFrame, TwoStationSim};
