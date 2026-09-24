//! Link-layer frame formats — the bytes inside a PHY DATA or CONTROL payload.
//!
//! Two containers exist at the physical layer: a DATA frame (LONG layout, mode signalled in
//! the pilot chips) and a CONTROL frame (SHORT layout, control mode, seven bytes). The link
//! layer puts its own header at the front of each.
//!
//! **DATA container**, three-byte header (five with an explicit length):
//!
//! | Offset | Field |
//! |---|---|
//! | 0 | kind (3 bits) \| flags (5 bits) |
//! | 1 | sequence number |
//! | 2 | session id |
//! | 3–4 | data length, only when `PARTIAL` is set |
//!
//! **Nothing in this header may change between transmissions of the same sequence number.**
//! A retransmission is the *same codeword* under another redundancy version, and the receiver
//! soft-combines them; a header that differed would make the combination meaningless. That is
//! why there is no burst position here — the receiver takes a frame's place in its burst from
//! its air time, and the end of a burst from the silence that follows.
//!
//! **CONTROL container**, seven bytes:
//!
//! | Offset | Field |
//! |---|---|
//! | 0 | kind (4 bits) \| flags (4 bits) |
//! | 1 | session id |
//! | 2 | base sequence number — the next one the receiver needs |
//! | 3–4 | bitmap: bit *i* set ⇔ `base + i` received |
//! | 5 | measured SNR, signed dB, 3 kHz reference; `0x7F` = unknown |
//! | 6 | recommended mode (4 bits) \| counter (4 bits) |
//!
//! Everything is big-endian. Callsigns pack six bits per character (`A–Z 0–9 - /`), nine
//! characters into seven bytes.

/// Selective-repeat window: the ACK bitmap width and the most frames in flight per burst.
pub const WINDOW: usize = 16;
/// Sequence numbers are eight bits.
pub const SEQ_MOD: usize = 256;
/// Most frames one burst may carry.
pub const MAX_BURST: usize = 16;
/// Characters a callsign may hold.
pub const CALL_MAX_CHARS: usize = 9;
/// Bytes a packed callsign occupies.
pub const CALL_BYTES: usize = 7;
/// Bytes of DATA header before the payload.
pub const DATA_HEADER: usize = 3;
/// Bytes in a CONTROL frame.
pub const CONTROL_BYTES: usize = 7;
/// SNR byte meaning "not measured".
pub const SNR_UNKNOWN: u8 = 0x7F;
/// The callsign alphabet; index 0 is the padding character.
pub const CALL_ALPHABET: &[u8] = b"\0ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-/";

/// What a DATA container carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataKind {
    /// User data.
    Data,
    /// A connection request, carrying both callsigns.
    ConnectReq,
    /// A connection acceptance.
    ConnectAck,
    /// Unproto: sent outside any session, addressed to nobody, carrying this station's
    /// callsign. It is how an operator answers "can anybody hear me?" without arranging a
    /// contact first, which on HF is most of what a new station needs to know.
    Beacon,
    /// A beacon with a destination (P7-1): "can *you* hear me, and how well?" — sent
    /// outside any session, answered with a [`ProbeAck`](Self::ProbeAck) carrying the SNR
    /// the answering station measured on it, so both operators see both directions of
    /// the path without arranging a contact. The one thing a receiver cannot measure is
    /// how it is heard; this is how it asks.
    Probe,
    /// The answer to a probe: the probed station's callsign, the prober's, and the SNR the
    /// probe arrived at. Answering is a *response* in the sense of §97.221(c), so a
    /// station that may only answer may answer this too; sending a probe is a call.
    ProbeAck,
}

impl DataKind {
    const fn to_bits(self) -> u8 {
        match self {
            Self::Data => 0,
            Self::ConnectReq => 1,
            Self::ConnectAck => 2,
            Self::Beacon => 3,
            Self::Probe => 4,
            Self::ProbeAck => 5,
        }
    }

    const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Data),
            1 => Some(Self::ConnectReq),
            2 => Some(Self::ConnectAck),
            3 => Some(Self::Beacon),
            4 => Some(Self::Probe),
            5 => Some(Self::ProbeAck),
            _ => None,
        }
    }
}

/// Flags on a DATA container.
pub mod data_flags {
    /// An explicit data length follows the header (the frame is not full).
    pub const PARTIAL: u8 = 0x01;
}

/// What a CONTROL container carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// Acknowledgement with a selective-repeat bitmap.
    Ack,
    /// Keep-alive from an idle sender, answered with an ACK.
    Poll,
    /// Hands the sending role to the peer.
    Turn,
    /// Requests that the session close.
    Disc,
    /// Confirms a close.
    DiscAck,
}

impl ControlKind {
    const fn to_bits(self) -> u8 {
        match self {
            Self::Ack => 0,
            Self::Poll => 1,
            Self::Turn => 2,
            Self::Disc => 3,
            Self::DiscAck => 4,
        }
    }

    const fn from_bits(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Ack),
            1 => Some(Self::Poll),
            2 => Some(Self::Turn),
            3 => Some(Self::Disc),
            4 => Some(Self::DiscAck),
            _ => None,
        }
    }
}

/// Flags on a CONTROL container.
pub mod control_flags {
    /// The receiving station has data to send.
    pub const WANT_TX: u8 = 0x1;
    /// The receiving station demands the sending role now.
    pub const BREAK: u8 = 0x2;
}

/// Anything malformed in a received frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The payload is shorter than its header requires.
    TooShort,
    /// The kind field holds a value this version does not define.
    UnknownKind(u8),
    /// A declared length runs past the end of the payload.
    BadLength,
    /// A callsign is empty, too long, or holds a character outside the alphabet.
    BadCallsign,
    /// A sequence number or session id outside its eight-bit range.
    OutOfRange,
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "frame is shorter than its header requires"),
            Self::UnknownKind(k) => write!(f, "unknown frame kind {k}"),
            Self::BadLength => write!(f, "declared length runs past the end of the frame"),
            Self::BadCallsign => write!(f, "callsign is empty, too long, or has bad characters"),
            Self::OutOfRange => write!(f, "value outside its eight-bit range"),
        }
    }
}

impl core::error::Error for FrameError {}

// ── callsigns ─────────────────────────────────────────────────────────

/// Pack a callsign into seven bytes, six bits per character.
///
/// # Errors
/// If the callsign is empty, longer than [`CALL_MAX_CHARS`], or holds a character outside
/// the alphabet.
pub fn pack_callsign(call: &str) -> Result<[u8; CALL_BYTES], FrameError> {
    let upper = call.trim().to_ascii_uppercase();
    if upper.is_empty() || upper.len() > CALL_MAX_CHARS {
        return Err(FrameError::BadCallsign);
    }
    let mut value: u64 = 0;
    for index in 0..CALL_MAX_CHARS {
        let character = upper.as_bytes().get(index).copied().unwrap_or(0);
        let position = CALL_ALPHABET
            .iter()
            .position(|&c| c == character)
            .ok_or(FrameError::BadCallsign)?;
        value = (value << 6) | position as u64;
    }
    value <<= 2; // the model pads to a whole number of bytes
    let bytes = value.to_be_bytes();
    let mut out = [0u8; CALL_BYTES];
    out.copy_from_slice(&bytes[8 - CALL_BYTES..]);
    Ok(out)
}

/// Unpack a callsign.
///
/// # Errors
/// If a six-bit group is outside the alphabet.
pub fn unpack_callsign(raw: &[u8]) -> Result<String, FrameError> {
    if raw.len() < CALL_BYTES {
        return Err(FrameError::TooShort);
    }
    let mut value: u64 = 0;
    for &byte in &raw[..CALL_BYTES] {
        value = (value << 8) | u64::from(byte);
    }
    value >>= 2;
    let mut out = String::with_capacity(CALL_MAX_CHARS);
    for index in 0..CALL_MAX_CHARS {
        let code = ((value >> (6 * (CALL_MAX_CHARS - 1 - index))) & 0x3F) as usize;
        let character = *CALL_ALPHABET.get(code).ok_or(FrameError::BadCallsign)?;
        if character != 0 {
            out.push(character as char);
        }
    }
    Ok(out)
}

// ── DATA container ────────────────────────────────────────────────────

/// The static part of a DATA container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataHeader {
    /// What the frame carries.
    pub kind: DataKind,
    /// Sequence number.
    pub seq: u8,
    /// Session id.
    pub session: u8,
}

/// Encode a DATA container, padded to the PHY payload size.
///
/// # Errors
/// If the body does not fit the capacity.
pub fn encode_data(
    header: &DataHeader,
    body: &[u8],
    capacity: usize,
) -> Result<Vec<u8>, FrameError> {
    let partial = body.len() < capacity.saturating_sub(DATA_HEADER);
    let mut flags = 0u8;
    if partial {
        flags |= data_flags::PARTIAL;
    }
    let mut out = Vec::with_capacity(capacity);
    out.push((header.kind.to_bits() << 5) | (flags & 0x1F));
    out.push(header.seq);
    out.push(header.session);
    if partial {
        if body.len() > capacity.saturating_sub(DATA_HEADER + 2) {
            return Err(FrameError::BadLength);
        }
        let length = u16::try_from(body.len()).map_err(|_| FrameError::BadLength)?;
        out.extend_from_slice(&length.to_be_bytes());
    }
    out.extend_from_slice(body);
    if out.len() > capacity {
        return Err(FrameError::BadLength);
    }
    out.resize(capacity, 0);
    Ok(out)
}

/// Decode a DATA container into its header and body.
///
/// # Errors
/// If the payload is too short, the kind is unknown, or a declared length overruns.
pub fn decode_data(payload: &[u8]) -> Result<(DataHeader, Vec<u8>), FrameError> {
    if payload.len() < DATA_HEADER {
        return Err(FrameError::TooShort);
    }
    let kind_bits = payload[0] >> 5;
    let kind = DataKind::from_bits(kind_bits).ok_or(FrameError::UnknownKind(kind_bits))?;
    let flags = payload[0] & 0x1F;
    let header = DataHeader {
        kind,
        seq: payload[1],
        session: payload[2],
    };
    if flags & data_flags::PARTIAL != 0 {
        if payload.len() < DATA_HEADER + 2 {
            return Err(FrameError::TooShort);
        }
        let length = usize::from(u16::from_be_bytes([payload[3], payload[4]]));
        if length > payload.len() - (DATA_HEADER + 2) {
            return Err(FrameError::BadLength);
        }
        let start = DATA_HEADER + 2;
        return Ok((header, payload[start..start + length].to_vec()));
    }
    Ok((header, payload[DATA_HEADER..].to_vec()))
}

/// User bytes per full DATA frame for a PHY payload of the given size.
#[must_use]
pub const fn data_capacity(phy_payload_bytes: usize) -> usize {
    phy_payload_bytes.saturating_sub(DATA_HEADER)
}

/// Body of a connection request or acceptance: who is calling whom, and what they support.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectBody {
    /// Calling station.
    pub src: String,
    /// Called station.
    pub dst: String,
    /// Capability bits (reserved: compression, bandwidth options).
    pub caps: u8,
    /// Protocol version.
    pub version: u8,
    /// In an acceptance: the SNR (3 kHz) the request arrived at, whole decibels — what
    /// the caller starts its first burst from (P9-2). `None` in a request, and from a
    /// station of an earlier version, whose body stops at the version byte.
    pub snr_db: Option<f64>,
}

/// Capability bit 0: stream compression (deflate) — offered, and used only if both offer.
pub const CAP_COMPRESSION: u8 = 0x01;
/// Capability bits 1–2: the bandwidth of the waveform the frame was sent in. Not a
/// negotiation — a receiver knows the waveform from having decoded the frame — but a
/// statement, so a station can refuse a request that claims a bandwidth other than the
/// one it arrived in, and a station listening in more than one answers in the one it was
/// called in. A session lives its whole life in one bandwidth.
pub const CAP_BANDWIDTH_SHIFT: u8 = 1;
/// The mask of the bandwidth bits.
pub const CAP_BANDWIDTH_MASK: u8 = 0x03 << CAP_BANDWIDTH_SHIFT;

/// The bandwidth code stated in a capability byte: 0 = 2 300 Hz, 1 = 500 Hz, 2 = 2 750 Hz.
#[must_use]
pub const fn bandwidth_code(caps: u8) -> u8 {
    (caps & CAP_BANDWIDTH_MASK) >> CAP_BANDWIDTH_SHIFT
}

/// The code for a bandwidth in hertz, if it is one the air interface names.
#[must_use]
pub const fn bandwidth_code_of(bandwidth_hz: usize) -> Option<u8> {
    match bandwidth_hz {
        2300 => Some(0),
        500 => Some(1),
        2750 => Some(2),
        _ => None,
    }
}

/// A capability byte with its bandwidth bits set for `bandwidth_hz`.
///
/// # Panics
/// If the bandwidth is not one the air interface names.
#[must_use]
pub const fn with_bandwidth(caps: u8, bandwidth_hz: usize) -> u8 {
    let Some(code) = bandwidth_code_of(bandwidth_hz) else {
        panic!("no bandwidth code for this bandwidth");
    };
    (caps & !CAP_BANDWIDTH_MASK) | (code << CAP_BANDWIDTH_SHIFT)
}

/// The link protocol a station speaks, in the connect body's version byte. 2 since the tone
/// floor (ADR-0013): a mode number is a rung of the air's ladder — on the 2 300 Hz air two
/// above the OFDM mode of version 1 — so a session between the two would run on numbers that
/// mean different frames at either end; a station ignores a call or an acceptance of another
/// version, and says so. 3 since the fast kinds (ADR-0014): four more rungs on the 2 300 Hz
/// air, between the floor's two and the OFDM modes, and a control frame whose recommended
/// mode has five bits and its counter three. 4 since the narrow middle kinds (ADR-0015): two
/// more rungs on the 500 Hz air, between the floor's two and the OFDM modes (the 2 300 Hz
/// ladder is as it was, but one number says what both ladders are).
pub const PROTOCOL_VERSION: u8 = 4;

/// Bytes a connect body occupies; one of an earlier version is one byte shorter.
pub const CONNECT_BODY_BYTES: usize = 2 * CALL_BYTES + 3;

/// The SNR byte shared by the CONTROL frame and the probe body: signed whole decibels
/// (3 kHz reference), clamped to ±40, [`SNR_UNKNOWN`] for "not measured".
///
/// Ties round to even, matching the reference model. Rust's `round` goes half away from
/// zero and Python's goes half to even, so an SNR of exactly 12.5 would otherwise be
/// reported as 13 dB by one station and 12 dB by the other. Harmless for rate control,
/// but the two would disagree byte for byte, and a wire format that two correct
/// implementations encode differently is a bug.
#[must_use]
pub fn snr_byte(snr_db: Option<f64>) -> u8 {
    match snr_db {
        None => SNR_UNKNOWN,
        Some(value) => {
            let clamped = value.round_ties_even().clamp(-40.0, 40.0) as i32;
            (clamped as i8) as u8
        }
    }
}

/// The SNR an [`snr_byte`] carries, `None` for "not measured".
#[must_use]
pub fn snr_from_byte(byte: u8) -> Option<f64> {
    (byte != SNR_UNKNOWN).then(|| f64::from(byte as i8))
}

/// Body of a probe or its answer: who is asking whom, the SNR the answer reports, and the
/// capability bits (the bandwidth the frame was sent in, so a probe in another bandwidth
/// than it arrived in is ignored like a connect request would be).
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeBody {
    /// The station asking (a probe) or answering (an answer).
    pub src: String,
    /// The station asked, or the one being answered.
    pub dst: String,
    /// An answer: the SNR (3 kHz) the probe arrived at, whole decibels. A probe: `None`.
    pub snr_db: Option<f64>,
    /// Capability bits, as the connect body's.
    pub caps: u8,
}

/// Bytes a probe body occupies.
pub const PROBE_BODY_BYTES: usize = 2 * CALL_BYTES + 2;

impl ProbeBody {
    /// Serialise it.
    ///
    /// # Errors
    /// If either callsign cannot be packed.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let mut out = Vec::with_capacity(PROBE_BODY_BYTES);
        out.extend_from_slice(&pack_callsign(&self.src)?);
        out.extend_from_slice(&pack_callsign(&self.dst)?);
        out.push(snr_byte(self.snr_db));
        out.push(self.caps);
        Ok(out)
    }

    /// Parse it.
    ///
    /// # Errors
    /// If the body is short or a callsign is malformed.
    pub fn decode(body: &[u8]) -> Result<Self, FrameError> {
        if body.len() < PROBE_BODY_BYTES {
            return Err(FrameError::TooShort);
        }
        Ok(Self {
            src: unpack_callsign(&body[..CALL_BYTES])?,
            dst: unpack_callsign(&body[CALL_BYTES..2 * CALL_BYTES])?,
            snr_db: snr_from_byte(body[2 * CALL_BYTES]),
            caps: body[2 * CALL_BYTES + 1],
        })
    }
}

impl ConnectBody {
    /// Serialise it.
    ///
    /// # Errors
    /// If either callsign cannot be packed.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let mut out = Vec::with_capacity(CONNECT_BODY_BYTES);
        out.extend_from_slice(&pack_callsign(&self.src)?);
        out.extend_from_slice(&pack_callsign(&self.dst)?);
        out.push(self.caps);
        out.push(self.version);
        out.push(snr_byte(self.snr_db));
        Ok(out)
    }

    /// Parse it.
    ///
    /// # Errors
    /// If the body is short or a callsign is malformed.
    pub fn decode(body: &[u8]) -> Result<Self, FrameError> {
        if body.len() < 2 * CALL_BYTES + 2 {
            return Err(FrameError::TooShort);
        }
        Ok(Self {
            src: unpack_callsign(&body[..CALL_BYTES])?,
            dst: unpack_callsign(&body[CALL_BYTES..2 * CALL_BYTES])?,
            caps: body[2 * CALL_BYTES],
            version: body[2 * CALL_BYTES + 1],
            // a body from an earlier version stops here: not measured
            snr_db: body
                .get(2 * CALL_BYTES + 2)
                .and_then(|&byte| snr_from_byte(byte)),
        })
    }
}

// ── CONTROL container ─────────────────────────────────────────────────

/// A control frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlFrame {
    /// What it is.
    pub kind: ControlKind,
    /// Session id.
    pub session: u8,
    /// Flags (see [`control_flags`]).
    pub flags: u8,
    /// Next sequence number the receiver needs.
    pub base: u8,
    /// Bit *i* set means `base + i` has been received.
    pub bitmap: u16,
    /// Measured SNR in dB (3 kHz reference), or `None` if not measured.
    pub snr_db: Option<f64>,
    /// Mode the receiver recommends: a rung of the air's ladder, five bits — room for 32; the
    /// 2 300 Hz ladder has 20 since the fast kinds (ADR-0014).
    pub recommended_mode: u8,
    /// The sender's count of its acknowledgements, modulo 8: a label for logs, which no
    /// receiver acts on.
    pub counter: u8,
}

impl ControlFrame {
    /// Serialise into exactly [`CONTROL_BYTES`] bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; CONTROL_BYTES] {
        let snr = snr_byte(self.snr_db);
        [
            (self.kind.to_bits() << 4) | (self.flags & 0x0F),
            self.session,
            self.base,
            ((self.bitmap >> 8) & 0xFF) as u8,
            (self.bitmap & 0xFF) as u8,
            snr,
            ((self.recommended_mode & 0x1F) << 3) | (self.counter & 0x07),
        ]
    }

    /// Parse one.
    ///
    /// # Errors
    /// If the payload is short or the kind is unknown.
    pub fn decode(payload: &[u8]) -> Result<Self, FrameError> {
        if payload.len() < CONTROL_BYTES {
            return Err(FrameError::TooShort);
        }
        let kind_bits = payload[0] >> 4;
        let kind = ControlKind::from_bits(kind_bits).ok_or(FrameError::UnknownKind(kind_bits))?;
        let snr_db = snr_from_byte(payload[5]);
        Ok(Self {
            kind,
            session: payload[1],
            flags: payload[0] & 0x0F,
            base: payload[2],
            bitmap: (u16::from(payload[3]) << 8) | u16::from(payload[4]),
            snr_db,
            recommended_mode: payload[6] >> 3,
            counter: payload[6] & 0x07,
        })
    }

    /// ACK semantics: has `seq` been received — either below the base, or set in the bitmap?
    #[must_use]
    pub fn received(&self, seq: u8) -> bool {
        let distance = seq_distance(seq, self.base);
        if distance >= WINDOW {
            // far behind the base means already acknowledged and slid past
            return distance >= SEQ_MOD / 2;
        }
        (self.bitmap >> distance) & 1 == 1
    }
}

/// The sequence number `n` places after `seq`.
#[must_use]
pub const fn seq_after(seq: u8, n: u8) -> u8 {
    seq.wrapping_add(n)
}

/// Forward distance from `base` to `seq`, 0 … 255.
#[must_use]
pub fn seq_distance(seq: u8, base: u8) -> usize {
    usize::from(seq.wrapping_sub(base))
}

/// Whether `seq` lies in the window that starts at `base`.
#[must_use]
pub fn in_window(seq: u8, base: u8, width: usize) -> bool {
    seq_distance(seq, base) < width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callsigns_round_trip() {
        for call in ["W4ODA", "KK4XYZ", "M0ABC", "VK2DEF/P", "A1", "9Z9ZZZ9ZZ"] {
            let packed = pack_callsign(call).expect("pack");
            assert_eq!(packed.len(), CALL_BYTES);
            assert_eq!(unpack_callsign(&packed).expect("unpack"), call);
        }
    }

    #[test]
    fn bad_callsigns_are_rejected() {
        assert!(pack_callsign("").is_err());
        assert!(pack_callsign("TOOLONGCALL").is_err());
        assert!(pack_callsign("W4ODA!").is_err());
        assert!(unpack_callsign(&[0, 1]).is_err());
    }

    #[test]
    fn data_frames_round_trip_full_and_partial() {
        let capacity = 144;
        for body_len in [
            0usize,
            5,
            capacity - DATA_HEADER - 2,
            capacity - DATA_HEADER,
        ] {
            let body: Vec<u8> = (0..body_len).map(|i| (i % 251) as u8).collect();
            let header = DataHeader {
                kind: DataKind::Data,
                seq: 200,
                session: 42,
            };
            let frame = encode_data(&header, &body, capacity).expect("encode");
            assert_eq!(frame.len(), capacity);
            let (got_header, got_body) = decode_data(&frame).expect("decode");
            assert_eq!(got_header, header);
            assert_eq!(got_body, body, "body length {body_len}");
        }
    }

    #[test]
    fn a_retransmission_encodes_identically() {
        // HARQ combines identical codewords, so the encoded bytes must depend on nothing
        // beyond (header, body, capacity).
        let header = DataHeader {
            kind: DataKind::Data,
            seq: 7,
            session: 3,
        };
        let body: Vec<u8> = (0..100).collect();
        assert_eq!(
            encode_data(&header, &body, 144).unwrap(),
            encode_data(&header, &body, 144).unwrap()
        );
    }

    #[test]
    fn connect_bodies_round_trip() {
        let body = ConnectBody {
            src: "W4ODA".into(),
            dst: "KK4XYZ".into(),
            caps: 0b101,
            version: 1,
            snr_db: None,
        };
        let encoded = body.encode().expect("encode");
        assert_eq!(encoded.len(), CONNECT_BODY_BYTES);
        assert_eq!(ConnectBody::decode(&encoded).expect("decode"), body);
    }

    #[test]
    fn control_frames_round_trip() {
        let frame = ControlFrame {
            kind: ControlKind::Ack,
            session: 9,
            flags: control_flags::WANT_TX,
            base: 100,
            bitmap: 0b1011,
            snr_db: Some(-7.0),
            recommended_mode: 6,
            counter: 5,
        };
        let got = ControlFrame::decode(&frame.encode()).expect("decode");
        assert_eq!(got, frame);
    }

    #[test]
    fn snr_ties_round_to_even() {
        // pinned because it differs between languages: see the note in `encode`
        let frame = |snr: f64| ControlFrame {
            kind: ControlKind::Ack,
            session: 0,
            flags: 0,
            base: 0,
            bitmap: 0,
            snr_db: Some(snr),
            recommended_mode: 0,
            counter: 0,
        };
        assert_eq!(
            ControlFrame::decode(&frame(12.5).encode()).unwrap().snr_db,
            Some(12.0)
        );
        assert_eq!(
            ControlFrame::decode(&frame(13.5).encode()).unwrap().snr_db,
            Some(14.0)
        );
        assert_eq!(
            ControlFrame::decode(&frame(-12.5).encode()).unwrap().snr_db,
            Some(-12.0)
        );
    }

    #[test]
    fn snr_is_clamped_to_the_reportable_range() {
        let frame = |snr: f64| ControlFrame {
            kind: ControlKind::Ack,
            session: 0,
            flags: 0,
            base: 0,
            bitmap: 0,
            snr_db: Some(snr),
            recommended_mode: 0,
            counter: 0,
        };
        assert_eq!(
            ControlFrame::decode(&frame(99.0).encode()).unwrap().snr_db,
            Some(40.0)
        );
        assert_eq!(
            ControlFrame::decode(&frame(-99.0).encode()).unwrap().snr_db,
            Some(-40.0)
        );
    }

    #[test]
    fn an_unmeasured_snr_survives_the_round_trip() {
        let frame = ControlFrame {
            kind: ControlKind::Poll,
            session: 1,
            flags: 0,
            base: 0,
            bitmap: 0,
            snr_db: None,
            recommended_mode: 0,
            counter: 0,
        };
        assert_eq!(ControlFrame::decode(&frame.encode()).unwrap().snr_db, None);
    }

    #[test]
    fn ack_received_semantics() {
        let ack = ControlFrame {
            kind: ControlKind::Ack,
            session: 0,
            flags: 0,
            base: 10,
            bitmap: 0b0101,
            snr_db: None,
            recommended_mode: 0,
            counter: 0,
        };
        assert!(ack.received(10) && ack.received(12));
        assert!(!ack.received(11) && !ack.received(13));
        // below the base: already acknowledged
        assert!(ack.received(9) && ack.received(5));
    }

    #[test]
    fn sequence_arithmetic_wraps() {
        assert_eq!(seq_after(255, 1), 0);
        assert_eq!(seq_distance(0, 255), 1);
        assert!(in_window(3, 0, WINDOW));
        assert!(!in_window(200, 0, WINDOW));
    }

    #[test]
    fn a_beacon_carries_a_callsign_and_nothing_else() {
        // unproto: no session, no sequence, just who is transmitting
        let header = DataHeader {
            kind: DataKind::Beacon,
            seq: 0,
            session: 0,
        };
        let body = pack_callsign("W4ODA").expect("pack");
        let frame = encode_data(&header, &body, 26).expect("encode");
        let (got_header, got_body) = decode_data(&frame).expect("decode");
        assert_eq!(got_header, header);
        assert_eq!(unpack_callsign(&got_body).expect("unpack"), "W4ODA");
    }

    #[test]
    fn every_data_kind_survives_the_header_byte() {
        for kind in [
            DataKind::Data,
            DataKind::ConnectReq,
            DataKind::ConnectAck,
            DataKind::Beacon,
        ] {
            let header = DataHeader {
                kind,
                seq: 5,
                session: 9,
            };
            let frame = encode_data(&header, b"x", 26).expect("encode");
            assert_eq!(decode_data(&frame).expect("decode").0.kind, kind);
        }
    }

    #[test]
    fn malformed_frames_are_errors_not_panics() {
        assert!(matches!(decode_data(&[0, 1]), Err(FrameError::TooShort)));
        assert!(matches!(
            decode_data(&[0xE0, 0, 0]),
            Err(FrameError::UnknownKind(7))
        ));
        assert!(matches!(
            ControlFrame::decode(&[0, 1]),
            Err(FrameError::TooShort)
        ));
        assert!(matches!(
            ControlFrame::decode(&[0xF0, 0, 0, 0, 0, 0, 0]),
            Err(FrameError::UnknownKind(15))
        ));
        // a PARTIAL frame whose declared length overruns
        assert!(matches!(
            decode_data(&[0x01, 0, 0, 0xFF, 0xFF, 1, 2]),
            Err(FrameError::BadLength)
        ));
    }
}
