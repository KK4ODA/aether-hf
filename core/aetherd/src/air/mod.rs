//! The physical layer as the station sees it: one air per station (ADR-0113).
//!
//! The station hands an air the sound card's audio and gets audio back; baseband, acquisition,
//! decoding and modulation stay inside it. Aether HF's air is [`hf::HfAir`]; Aether FM's will
//! sit beside it (ADR-0112).

pub mod hf;

pub use hf::HfAir;
