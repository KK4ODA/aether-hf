//! Datagrams: another program's frame, carried outside any session (ADR-0019).
//!
//! A KISS client — APRS software, `VarAC`'s broadcasts — hands the modem whole frames and
//! expects each on the air as it is, one transmission, no acknowledgement. Aether carries one
//! as a *datagram*: split into DATA frames of kind [`DataKind::Datagram`] at the rung the station
//! sends datagrams at, and joined again by every station that decodes them, which hands the
//! frame to its own KISS clients.
//!
//! The DATA header numbers the pieces: `seq` holds the fragment's index in its high nibble and
//! the last index in its low one (sixteen fragments at most), `session` the datagram's number,
//! 1–255. Every fragment but the last is a full frame; the last is partial — and a remainder
//! one byte too long for that goes as two fragments. What a datagram carries ([`body`]) is the
//! sender's callsign, the frame's type and the frame. The Python model
//! (`aether_model/link/datagram.py`) is the specification; `tests/model_vectors.rs` holds this
//! port to it byte for byte.

use crate::frames::{
    CALL_BYTES, DATA_HEADER, DataHeader, DataKind, FrameError, decode_data, encode_data,
    pack_callsign, unpack_callsign,
};

/// The index and the last index share one byte, four bits each.
pub const MAX_FRAGMENTS: usize = 16;

/// The explicit length a partial DATA frame carries after its header.
const PARTIAL_LENGTH: usize = 2;

/// The frame types a datagram carries, as a VARA-style KISS client names them in the byte
/// after `FEND`: 0 an AX.25 frame, 1 an AX.25 frame with eight-byte address fields
/// (`VarAC`'s broadcasts), 2 unformatted data.
pub const FRAME_TYPES: [u8; 3] = [0, 1, 2];

/// What a datagram carries before the client's frame: the sender's callsign and the type.
pub const HEADER_BYTES: usize = CALL_BYTES + 1;

/// Why a datagram cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatagramError {
    /// Nothing to carry.
    Empty,
    /// The number must be 1–255.
    BadNumber,
    /// The rung's frames are too small to carry any of it.
    NoRoom,
    /// Needs more than [`MAX_FRAGMENTS`] fragments: the longest this rung carries.
    TooLong {
        /// The longest payload the rung carries.
        max: usize,
    },
    /// A frame type this version does not carry.
    BadType(u8),
    /// The callsign will not pack.
    BadCallsign,
    /// The DATA container refused a piece.
    Frame(FrameError),
}

impl core::fmt::Display for DatagramError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => f.write_str("an empty datagram carries nothing"),
            Self::BadNumber => f.write_str("a datagram's number is 1–255"),
            Self::NoRoom => f.write_str("the rung's frames carry no datagram"),
            Self::TooLong { max } => write!(
                f,
                "longer than the {max} bytes sixteen fragments carry at this rung"
            ),
            Self::BadType(t) => write!(f, "frame type {t} is not one a datagram carries"),
            Self::BadCallsign => f.write_str("the callsign will not pack"),
            Self::Frame(e) => write!(f, "{e:?}"),
        }
    }
}

impl core::error::Error for DatagramError {}

/// The longest datagram a rung whose frames carry `capacity` bytes can send.
#[must_use]
pub const fn max_payload(capacity: usize) -> usize {
    MAX_FRAGMENTS * capacity.saturating_sub(DATA_HEADER)
}

/// The pieces a payload goes as at a rung whose frames carry `capacity` bytes: full frames,
/// then a last piece short enough to carry its length — split in two when it is exactly one
/// byte too long for that.
///
/// # Errors
/// When the rung's frames cannot carry a partial piece at all.
pub fn split(payload: &[u8], capacity: usize) -> Result<Vec<&[u8]>, DatagramError> {
    let full = capacity.saturating_sub(DATA_HEADER);
    if full <= PARTIAL_LENGTH {
        return Err(DatagramError::NoRoom);
    }
    let mut pieces = Vec::new();
    let mut rest = payload;
    while rest.len() > full {
        let (piece, tail) = rest.split_at(full);
        pieces.push(piece);
        rest = tail;
    }
    if rest.len() == full - 1 {
        // too long to go partial (it needs two length bytes) and one short of full
        let (first, second) = rest.split_at(full - PARTIAL_LENGTH);
        pieces.push(first);
        pieces.push(second);
    } else {
        pieces.push(rest);
    }
    Ok(pieces)
}

/// The DATA frames, `capacity` bytes each, that carry `payload` as datagram `number`.
///
/// # Errors
/// An empty payload, a number outside 1–255, or more than sixteen fragments.
pub fn fragments(
    payload: &[u8],
    number: u8,
    capacity: usize,
) -> Result<Vec<Vec<u8>>, DatagramError> {
    if number == 0 {
        return Err(DatagramError::BadNumber);
    }
    if payload.is_empty() {
        return Err(DatagramError::Empty);
    }
    let pieces = split(payload, capacity)?;
    if pieces.len() > MAX_FRAGMENTS {
        return Err(DatagramError::TooLong {
            max: max_payload(capacity),
        });
    }
    let last = u8::try_from(pieces.len() - 1).map_err(|_| DatagramError::TooLong {
        max: max_payload(capacity),
    })?;
    pieces
        .iter()
        .enumerate()
        .map(|(index, piece)| {
            let index = u8::try_from(index).unwrap_or(0);
            let header = DataHeader {
                kind: DataKind::Datagram,
                seq: (index << 4) | last,
                session: number,
            };
            encode_data(&header, piece, capacity).map_err(DatagramError::Frame)
        })
        .collect()
}

/// One decoded piece: which datagram, where in it, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    /// The datagram's number.
    pub number: u8,
    /// This piece's index.
    pub index: u8,
    /// The last piece's index.
    pub last: u8,
    /// Its bytes.
    pub body: Vec<u8>,
}

/// The fragment a decoded DATA frame carries, or `None` when it is not a datagram's.
#[must_use]
pub fn read_fragment(payload: &[u8]) -> Option<Fragment> {
    let (header, body) = decode_data(payload).ok()?;
    if header.kind != DataKind::Datagram || header.session == 0 {
        return None;
    }
    let (index, last) = (header.seq >> 4, header.seq & 0x0F);
    if index > last {
        return None;
    }
    Some(Fragment {
        number: header.session,
        index,
        last,
        body,
    })
}

/// What a datagram carries: the sending station's callsign — so every datagram identifies its
/// station in the emission itself — the frame's type, and the frame.
///
/// # Errors
/// A frame type this version does not carry, or a callsign that will not pack.
pub fn body(source: &str, frame_type: u8, frame: &[u8]) -> Result<Vec<u8>, DatagramError> {
    if !FRAME_TYPES.contains(&frame_type) {
        return Err(DatagramError::BadType(frame_type));
    }
    let call = pack_callsign(source).map_err(|_| DatagramError::BadCallsign)?;
    let mut out = Vec::with_capacity(HEADER_BYTES + frame.len());
    out.extend_from_slice(&call);
    out.push(frame_type);
    out.extend_from_slice(frame);
    Ok(out)
}

/// The sender, the frame type and the frame a joined datagram carries; `None` when it is not
/// one this version reads.
#[must_use]
pub fn parse_body(payload: &[u8]) -> Option<(String, u8, Vec<u8>)> {
    if payload.len() <= HEADER_BYTES || !FRAME_TYPES.contains(&payload[CALL_BYTES]) {
        return None;
    }
    let source = unpack_callsign(&payload[..CALL_BYTES]).ok()?;
    if source.is_empty() {
        return None;
    }
    Some((
        source,
        payload[CALL_BYTES],
        payload[HEADER_BYTES..].to_vec(),
    ))
}

#[derive(Debug, Clone)]
struct Partial {
    last: u8,
    pieces: Vec<Option<Vec<u8>>>,
    first_s: f64,
}

/// Joins the fragments of datagrams as they are decoded.
///
/// A datagram is handed on once every piece has arrived; one that stays incomplete longer than
/// `timeout_s` — a piece lost, since nothing is retransmitted — is dropped, and so are the
/// oldest when more than `keep` are waiting. A piece seen twice counts once.
#[derive(Debug, Clone)]
pub struct Reassembler {
    timeout_s: f64,
    keep: usize,
    waiting: Vec<((u8, u8), Partial)>,
    /// Datagrams given up on: timed out, or pushed out by newer ones.
    pub dropped: u64,
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new(120.0, 8)
    }
}

impl Reassembler {
    /// A reassembler that waits `timeout_s` for a datagram's pieces and holds at most `keep`.
    #[must_use]
    pub fn new(timeout_s: f64, keep: usize) -> Self {
        Self {
            timeout_s,
            keep: keep.max(1),
            waiting: Vec::new(),
            dropped: 0,
        }
    }

    /// Take a piece; return the whole datagram when this piece completes it.
    pub fn add(&mut self, fragment: Fragment, now_s: f64) -> Option<Vec<u8>> {
        self.expire(now_s);
        let key = (fragment.number, fragment.last);
        let position = self.waiting.iter().position(|(k, _)| *k == key);
        let position = position.unwrap_or_else(|| {
            if self.waiting.len() >= self.keep {
                let oldest = self
                    .waiting
                    .iter()
                    .enumerate()
                    .min_by(|a, b| a.1.1.first_s.total_cmp(&b.1.1.first_s))
                    .map_or(0, |(i, _)| i);
                self.waiting.remove(oldest);
                self.dropped += 1;
            }
            self.waiting.push((
                key,
                Partial {
                    last: fragment.last,
                    pieces: vec![None; usize::from(fragment.last) + 1],
                    first_s: now_s,
                },
            ));
            self.waiting.len() - 1
        });
        let partial = &mut self.waiting[position].1;
        let slot = &mut partial.pieces[usize::from(fragment.index)];
        if slot.is_none() {
            *slot = Some(fragment.body);
        }
        if partial.pieces.iter().any(Option::is_none) {
            return None;
        }
        let (_, done) = self.waiting.remove(position);
        debug_assert_eq!(usize::from(done.last) + 1, done.pieces.len());
        Some(done.pieces.into_iter().flatten().flatten().collect())
    }

    /// Drop what has waited too long for a piece that is not coming.
    pub fn expire(&mut self, now_s: f64) {
        let before = self.waiting.len();
        let timeout = self.timeout_s;
        self.waiting.retain(|(_, p)| now_s - p.first_s <= timeout);
        self.dropped += (before - self.waiting.len()) as u64;
    }

    /// Datagrams with pieces still to come.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 131 + 7) % 256) as u8).collect()
    }

    #[test]
    fn what_goes_comes_back_whole_in_any_order() {
        for capacity in [24usize, 26, 144] {
            for size in [1, 20, 21, 22, 100, max_payload(capacity)] {
                let original = payload(size);
                let frames = fragments(&original, 9, capacity).expect("fits");
                let mut joiner = Reassembler::default();
                let mut out = None;
                for frame in frames.iter().rev() {
                    let piece = read_fragment(frame).expect("a fragment");
                    out = joiner.add(piece, 0.0).or(out);
                }
                assert_eq!(
                    out.as_deref(),
                    Some(&original[..]),
                    "cap {capacity} size {size}"
                );
                assert_eq!(joiner.waiting(), 0);
            }
        }
    }

    #[test]
    fn a_lost_piece_times_out_and_a_crowd_is_bounded() {
        let frames = fragments(&payload(60), 3, 24).expect("fits");
        let mut joiner = Reassembler::new(10.0, 2);
        for frame in &frames[..frames.len() - 1] {
            assert!(
                joiner
                    .add(read_fragment(frame).expect("piece"), 0.0)
                    .is_none()
            );
        }
        joiner.expire(11.0);
        assert_eq!((joiner.waiting(), joiner.dropped), (0, 1));
        for number in 1..=5u8 {
            let first = &fragments(&payload(60), number, 24).expect("fits")[0];
            joiner.add(
                read_fragment(first).expect("piece"),
                20.0 + f64::from(number),
            );
        }
        assert_eq!(joiner.waiting(), 2);
        assert_eq!(joiner.dropped, 4);
    }

    #[test]
    fn what_does_not_fit_is_refused() {
        assert_eq!(
            fragments(&payload(max_payload(24) + 1), 1, 24),
            Err(DatagramError::TooLong {
                max: max_payload(24)
            })
        );
        assert_eq!(fragments(&[], 1, 24), Err(DatagramError::Empty));
        assert_eq!(fragments(b"x", 0, 24), Err(DatagramError::BadNumber));
        assert_eq!(body("W1AW", 3, b"x"), Err(DatagramError::BadType(3)));
    }
}
