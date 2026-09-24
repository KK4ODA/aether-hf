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
//! # What is here
//!
//! Everything from a payload to audio and back: the frame codec, OFDM, the preamble,
//! acquisition, the receiver, the tone floor under both mode tables (ADR-0013), the audio
//! front end at 48 kHz, the impulse blanker, and a streaming receiver that takes blocks of any
//! size and reports a frame as soon as acquisition finds it.
//!
//! The transmitter applies ADR-0004 peak reduction by default, at the target its frame's
//! constellation can absorb; `without_papr_reduction` turns it off for a benchmark or a
//! comparison against an unclipped reference.

pub mod blanker;
pub mod codec;
pub mod constellation;
pub mod fir;
pub mod modem;
pub mod modes;
pub mod ofdm;
pub mod papr;
pub mod passband;
pub mod preamble;
pub mod rx;
pub mod stream;
pub mod sync;
pub mod tone;
pub mod tx;
pub mod waveform;

mod tables {
    //! Air-interface constants exported from the reference model at build time.
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/preamble_tables.rs"));
}

pub use blanker::{BlankMode, NoiseBlanker, StreamingBlanker};
pub use codec::{FrameCodec, coprime_stride};
pub use constellation::{Complex, Constellation, NoiseVar};
pub use fir::{Fir, Sample};
pub use modem::{DecodedFrame, Modem, ModemError, Received};
pub use modes::{CONTROL_MODE, FrameLayout, LONG, MODES, Mode, PREAMBLE_SYMBOLS, Rung, SHORT};
pub use papr::{CLIP_TARGET_DB, CLIP_TARGET_DENSE_DB, ClipAndFilter, clip_target_db, papr_db};
pub use passband::{AudioToBaseband, BasebandToAudio};
pub use stream::{PendingFrame, StreamingReceiver};
pub use tone::{ToneFrame, ToneKind, ToneSync};
pub use waveform::{Bandwidth, Modulation, WIDE_2300, WaveformParams};
