//! Aether HF physical layer.
//!
//! The production port of `model/aether_model/waveform.py`, `phy/constellation.py`,
//! `frame/modes.py` and `frame/codec.py`. The Python model remains the specification
//! (ADR-0001); `tests/model_vectors.rs` checks this crate against vectors it generates.
//!
//! What "agrees with the model" means differs by layer, and the tests say which is which:
//! integer-valued things — the mode table, frame layouts, the interleaver permutation,
//! constellation bit labels — are **bit-exact**, while floating-point DSP is checked to a
//! stated numerical tolerance, because no two implementations of the same arithmetic are
//! required to produce identical doubles.
//!
//! # Not yet ported
//!
//! The transmitter here does not apply the ADR-0004 peak reduction the model applies by
//! default, so it is compared against the model with that switched off. Acquisition, channel
//! estimation and the receiver are still to come; until they are, this crate can build a
//! frame and take one apart given its position, but it cannot find one in a stream.

pub mod codec;
pub mod constellation;
pub mod modes;
pub mod ofdm;
pub mod preamble;
pub mod tx;
pub mod waveform;

mod tables {
    //! Air-interface constants exported from the reference model at build time.
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/preamble_tables.rs"));
}

pub use codec::{FrameCodec, coprime_stride};
pub use constellation::{Complex, Constellation, NoiseVar};
pub use modes::{CONTROL_MODE, FrameLayout, LONG, MODES, Mode, PREAMBLE_SYMBOLS, SHORT};
pub use waveform::{Bandwidth, Modulation, WIDE_2300, WaveformParams};
