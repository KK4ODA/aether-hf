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

pub mod codec;
pub mod constellation;
pub mod modes;
pub mod waveform;

pub use codec::{FrameCodec, coprime_stride};
pub use constellation::{Complex, Constellation, NoiseVar};
pub use modes::{CONTROL_MODE, FrameLayout, LONG, MODES, Mode, PREAMBLE_SYMBOLS, SHORT};
pub use waveform::{Bandwidth, Modulation, WIDE_2300, WaveformParams};
