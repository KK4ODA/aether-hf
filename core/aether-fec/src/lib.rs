//! Forward error correction for the Aether HF modem.
//!
//! 3GPP TS 38.212 §5.1 CRCs and §5.3.2 LDPC (base graphs 1 and 2), with §5.4.2 rate matching
//! and its soft inverse for hybrid ARQ.
//!
//! This is the production port of the Python reference model in `model/aether_model/fec/`.
//! The two share the extracted base-graph tables (see `build.rs`) and are checked against
//! each other bit-for-bit by `tests/model_vectors.rs`, whose vectors the model generates.
//! Where they ever disagree, the model is the specification and this is the bug.
//!
//! ```
//! use aether_fec::{crc::CRC24A, ldpc::NrLdpcCode, rate_match::RateMatcher};
//!
//! let code = NrLdpcCode::new(2, 30)?;
//! let payload: Vec<u8> = (0..200).map(|i| u8::from(i % 3 == 0)).collect();
//! let mut info = CRC24A.attach(&payload);
//! let info_len = info.len();
//! info.resize(code.k, 0); // fillers
//!
//! let codeword = code.encode(&info)?;
//! let matcher = RateMatcher::new(&code, info_len, 600, 0)?;
//! let on_air = matcher.match_bits(&codeword)?;
//! assert_eq!(on_air.len(), 600);
//! # Ok::<(), aether_fec::ldpc::FecError>(())
//! ```

pub mod crc;
pub mod ldpc;
pub mod rate_match;

mod tables {
    //! Base-graph tables generated from the reference model's JSON at build time.
    #![allow(missing_docs)]
    include!(concat!(env!("OUT_DIR"), "/base_graphs.rs"));
}

pub use crc::{CRC6, CRC11, CRC16, CRC24A, CRC24B, CRC24C, Crc};
pub use ldpc::{Decoded, FILLER_LLR, FecError, NrLdpcCode};
pub use rate_match::RateMatcher;
