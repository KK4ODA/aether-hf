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
//! | 6 | recommended mode (5 bits) \| burst family (1 bit) \| follows (2 bits) |
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
    /// A fragment of another program's frame — an AX.25 frame a KISS client handed the
    /// modem — sent outside any session with no acknowledgement (ADR-0019). `seq` holds the
    /// fragment's index in its high nibble and the last index in its low one; `session` the
    /// datagram's number, 1–255. See [`crate::datagram`].
    Datagram,
    /// A frame that measures a rung of the path (protocol 9, ADR-0077): filler at the rung
    /// measured, sent once in a transmission of its own and never delivered or sent again. Its
    /// `seq` is its index among the transmission's measurement frames, not a sequence number: it
    /// takes none, so a frame of it that does not arrive leaves nothing to deliver. The other
    /// station reports which arrived ([`REPORT`]).
    Measure,
}

impl DataKind {
    /// Whether frames of this kind are sent outside any session — a beacon, a probe or its
    /// answer, a datagram — and so never belong to a session's stream, whatever their
    /// session byte holds.
    #[must_use]
    pub const fn outside_sessions(self) -> bool {
        matches!(
            self,
            Self::Beacon | Self::Probe | Self::ProbeAck | Self::Datagram
        )
    }

    const fn to_bits(self) -> u8 {
        match self {
            Self::Data => 0,
            Self::ConnectReq => 1,
            Self::ConnectAck => 2,
            Self::Beacon => 3,
            Self::Probe => 4,
            Self::ProbeAck => 5,
            Self::Datagram => 6,
            Self::Measure => 7,
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
            6 => Some(Self::Datagram),
            7 => Some(Self::Measure),
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
    /// (TURN) Sent at the end of a burst that emptied the sender's queue: the turn is the
    /// receiving station's for the taking, with its acknowledgement (ADR-0047).
    pub const OFFER: u8 = 0x4;
    /// (ACK) The turn offered is taken: this station's own data follows the acknowledgement
    /// in the same transmission (ADR-0047).
    pub const TAKEN: u8 = 0x8;
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

/// The kind a DATA container's first byte names, read without the rest of it.
#[must_use]
pub fn data_kind(payload: &[u8]) -> Option<DataKind> {
    payload.first().and_then(|&b| DataKind::from_bits(b >> 5))
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
    /// Protocol 8: how long the sender's radio takes to hear again after it stops sending —
    /// its VOX hold and receiver recovery — in 0.1 s steps up to 1.5 s. The other station's
    /// slot starts no sooner after this station's transmission, and its transmissions no
    /// sooner after this station's slot (ADR-0069). Absent before protocol 8.
    pub turnaround_s: f64,
    /// Protocol 8: how long the sender's Morse identifier lasts, in 0.5 s steps (0 for none):
    /// a master grants a slot that much longer when the slave's identifier is due. Absent
    /// before protocol 8.
    pub identifier_s: f64,
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

/// The bandwidth in hertz a capability byte states; `None` for the reserved code.
#[must_use]
pub const fn bandwidth_hz_of(caps: u8) -> Option<usize> {
    match bandwidth_code(caps) {
        0 => Some(2300),
        1 => Some(500),
        2 => Some(2750),
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

/// Link protocol 7, the turn engine's ([`LinkEngine`](crate::LinkEngine)), in the connect
/// body's version byte — kept until granted slots replace that engine (ADR-0069 Step 5). 2
/// since the tone floor (ADR-0013): a mode number is a rung of the air's ladder — on the 2 300 Hz air two
/// above the OFDM mode of version 1 — so a session between the two would run on numbers that
/// mean different frames at either end; a station ignores a call or an acceptance of another
/// version, and says so. 3 since the fast kinds (ADR-0014): four more rungs on the 2 300 Hz
/// air, between the floor's two and the OFDM modes, and a control frame whose recommended
/// mode has five bits and its counter three. 4 since the narrow middle kinds (ADR-0015): two
/// more rungs on the 500 Hz air, between the floor's two and the OFDM modes (the 2 300 Hz
/// ladder is as it was, but one number says what both ladders are). 5 since the burst countdown
/// (ADR-0041): an ordinary data frame's mode chips are turned by the number of frames of its
/// burst that follow it, which a receiver of version 4 cannot read — it takes a turned frame for
/// noise. 6 since the countdown counts in pairs (ADR-0046): a 5 reading 1 where 6 says 1 or 2
/// would answer a frame early. 7 since the turn offered at the end of a burst (ADR-0047): a
/// TURN's `OFFER` flag and an acknowledgement's `TAKEN`, which a 6 would take for a TURN to
/// answer and an acknowledgement alone.
pub const TURNS_VERSION: u8 = 7;

/// The largest countdown a data frame carries (ADR-0041): the physical layer turns the frame's
/// mode chips a quarter turn a step, so four values.
pub const MAX_FOLLOWS: u8 = 3;

/// The countdown a data frame carries for the frames of its burst that follow it: half of them,
/// rounded up (ADR-0046) — 0 for the last frame, 1 for one or two following, 2 for three or
/// four, 3 for five or six. It never says fewer than follow, so a receiver that hears only the
/// burst's first frames does not answer over the rest, as "three or more" let it; it says at
/// most one more, and only the last frame's loss makes that matter. Exact for bursts up to
/// seven frames; the default burst is six.
#[must_use]
pub fn countdown_of(following: usize) -> u8 {
    u8::try_from(following.div_ceil(2)).map_or(MAX_FOLLOWS, |n| n.min(MAX_FOLLOWS))
}

/// The most frames a countdown says may follow ([`countdown_of`]).
#[must_use]
pub fn most_following(countdown: u8) -> u8 {
    2 * countdown
}

/// Bytes a connect body occupies; one of an earlier version is one byte shorter.
pub const CONNECT_BODY_BYTES: usize = 2 * CALL_BYTES + 3;

/// Protocol 8's connect body: the turnaround and the identifier's length after the SNR. With a
/// DATA header and an explicit length it fills the 24 bytes of the tone floor's slowest rung.
pub const CONNECT_BODY_8_BYTES: usize = CONNECT_BODY_BYTES + 2;

/// The first link protocol of granted slots (ADR-0069): the caller is the session's master and
/// the only station that keys on its own initiative; every master transmission grants the other
/// station one slot, defined by what it states; a station keys only in a slot granted by a frame
/// it decoded in time. Its control frame ([`GrantFrame`]) and its connect body's two more bytes
/// are not version 7's, and neither is any of its timing.
pub const GRANTED_VERSION: u8 = 8;

/// The link protocol a station speaks, in the connect body's version byte; what a release
/// states (`link_protocol`) and the shell's `LINK_PROTOCOL` is held to; a station ignores a
/// call of another, and says so. 9 since the measurement frames (ADR-0077): a
/// [`DataKind::Measure`] frame, the control frame's `measure` bit in the place of version 8's
/// identifier request inside a session, and an acknowledgement that reports a measurement
/// ([`REPORT`]), none of which a station of version 8 reads.
pub const PROTOCOL_VERSION: u8 = 9;

/// Capability bits 3–4, in a request (a call, a probe) of protocol 8: the gap level of the
/// slot its answer goes in (ADR-0069). A caller raises it every second try, so a called
/// station whose computer decodes late is reached; an answer carries 0.
pub const CAP_LEVEL_SHIFT: u8 = 3;
/// The mask of the level bits.
pub const CAP_LEVEL_MASK: u8 = 0x03 << CAP_LEVEL_SHIFT;

/// A capability byte with the answer slot's gap level set (bits 3–4).
#[must_use]
pub const fn with_level(caps: u8, level: u8) -> u8 {
    (caps & !CAP_LEVEL_MASK) | ((level & 0x03) << CAP_LEVEL_SHIFT)
}

/// The answer slot's gap level a request's capability byte states.
#[must_use]
pub const fn level_of(caps: u8) -> u8 {
    (caps & CAP_LEVEL_MASK) >> CAP_LEVEL_SHIFT
}

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
        if self.version >= GRANTED_VERSION {
            // whole steps up, as the model's ceil less a hair: 0.25 s says 0.3, 0.2 says 0.2
            let turnaround = (self.turnaround_s * 10.0 - 1e-9).ceil().clamp(0.0, 15.0);
            let identifier = (self.identifier_s * 2.0 - 1e-9).ceil().clamp(0.0, 255.0);
            out.push(turnaround as u8);
            out.push(identifier as u8);
        }
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
        let version = body[2 * CALL_BYTES + 1];
        let (turnaround_s, identifier_s) =
            if version >= GRANTED_VERSION && body.len() >= CONNECT_BODY_8_BYTES {
                (
                    f64::from(body[2 * CALL_BYTES + 3] & 0x0F) / 10.0,
                    f64::from(body[2 * CALL_BYTES + 4]) / 2.0,
                )
            } else {
                (0.0, 0.0)
            };
        Ok(Self {
            src: unpack_callsign(&body[..CALL_BYTES])?,
            dst: unpack_callsign(&body[CALL_BYTES..2 * CALL_BYTES])?,
            caps: body[2 * CALL_BYTES],
            version,
            // a body from an earlier version stops here: not measured
            snr_db: body
                .get(2 * CALL_BYTES + 2)
                .and_then(|&byte| snr_from_byte(byte)),
            turnaround_s,
            identifier_s,
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
    /// How many frames of a burst follow this control frame in the same transmission, in
    /// slots of that burst's data frame (0–3, ADR-0068): an acknowledgement that took the turn
    /// states its own burst after it (ADR-0047), so a receiver that reads it and nothing of the
    /// burst still knows when the burst ends. Until protocol 8 these bits were a counter of
    /// acknowledgements, a label for logs.
    pub follows: u8,
    /// The family of the burst [`follows`](Self::follows) counts — the length of its slots.
    pub burst_floor: bool,
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
            ((self.recommended_mode & 0x1F) << 3)
                | (u8::from(self.burst_floor) << 2)
                | (self.follows & 0x03),
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
            follows: payload[6] & 0x03,
            burst_floor: payload[6] & 0x04 != 0,
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

// ── protocol 8: the CONTROL container ────────────────────────────────

/// What a protocol 8 control frame is (ADR-0069).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantKind {
    /// The master's: what this transmission carries, what it acknowledges, and the slot it
    /// grants the slave.
    Header,
    /// The slave's, in its slot: what it acknowledges, what follows, what it has queued.
    Answer,
    /// Either's: the session ends. A master's grants the `DISC_ACK`'s slot; a slave's goes in a
    /// slot it was granted, the last it uses.
    Disc,
    /// The answer to a DISC.
    DiscAck,
}

impl GrantKind {
    /// The kind's two bits.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        match self {
            Self::Header => 0,
            Self::Answer => 1,
            Self::Disc => 2,
            Self::DiscAck => 3,
        }
    }

    /// The kind two bits name (every value names one).
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0x03 {
            0 => Self::Header,
            1 => Self::Answer,
            2 => Self::Disc,
            _ => Self::DiscAck,
        }
    }
}

/// The six-bit SNR field's "not measured": −32 in two's complement, outside −31 … +31.
pub const GRANT_SNR_UNKNOWN: u8 = 0x20;

/// An acknowledgement's bitmap bit 0, set: the bitmap reports the other station's last
/// measurement — bit `i + 1` set means its measurement frame `i` arrived — and acknowledges
/// nothing at or above `base` (protocol 9, ADR-0077). In a plain acknowledgement the bit is
/// always clear: `base` is the first sequence number not received.
pub const REPORT: u16 = 0x0001;

/// Protocol 8's control frame (ADR-0069): seven bytes, 56 bits.
///
/// ```text
/// 0: kind (2) | count (3) | data family (1) | level (2)
/// 1: session (7) | fam (1)
/// 2: flag (1) | id / measure (1) | SNR (6)
/// 3: base       4–5: bitmap         (selective acknowledgement, as version 7's)
/// 6: rung (5) | n (3)
/// ```
///
/// `count`: data frames after this one in its transmission, exactly (0–6), and their family
/// (`data_floor`) — with it a receiver that reads this frame and none of them still knows where
/// the transmission ends. `level`: in a header, the gap level of the slot it grants; in an
/// answer, the level its sender needs. `fam`: in a header, the family the answer goes in; in an
/// answer, the family its sender needs headers in. `flag`: the header's slot is an open window;
/// the answer asks to leave. `ident` (a DISC, a DISC_ACK): the slot includes the slave's closing
/// Morse identifier (ADR-0075). `measure` (a header, an answer; the same bit, protocol 9): the
/// data frames after this one are measurement frames ([`DataKind::Measure`]), and with none
/// after it, the sender's last were and it has not read their report — it sends no data until
/// it has (ADR-0077). `base`/`bitmap`: what the sender has received of the other station's
/// data, or with [`REPORT`] set, which of its last measurement's frames arrived. `snr_db`: how
/// the sender hears the other station, −31 … +31 dB (ADR-0021). `rung`: recommended for the other station's data. `n`: frames granted (or an
/// open window's size step) / frames queued, at most 7. `session` is the session number's low
/// seven bits: data frames carry all eight.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::struct_excessive_bools)] // the frame's flag bits, as the wire has them
pub struct GrantFrame {
    /// What it is.
    pub kind: GrantKind,
    /// The session number's low seven bits.
    pub session: u8,
    /// Data frames after it in its transmission.
    pub count: u8,
    /// The family of those data frames.
    pub data_floor: bool,
    /// A header's slot's gap level; an answer's sender's need.
    pub level: u8,
    /// A header: the answer's family; an answer: the family its sender needs headers in.
    pub fam: bool,
    /// A header: the slot is an open window; an answer: its sender asks to leave.
    pub flag: bool,
    /// A DISC or a DISC_ACK: the slot includes the slave's closing identifier.
    pub ident: bool,
    /// A header or an answer: measurement frames follow, or the sender's last were.
    pub measure: bool,
    /// Next sequence number the sender needs.
    pub base: u8,
    /// Bit *i* set means `base + i` has been received.
    pub bitmap: u16,
    /// How the sender hears the other station, whole decibels, `None` if not measured.
    pub snr_db: Option<f64>,
    /// The rung recommended for the other station's data.
    pub rung: u8,
    /// Frames granted (or an open window's size step) / frames queued.
    pub n: u8,
}

impl GrantFrame {
    /// A frame of a kind and session with every other field zero.
    #[must_use]
    pub const fn new(kind: GrantKind, session: u8) -> Self {
        Self {
            kind,
            session,
            count: 0,
            data_floor: false,
            level: 0,
            fam: false,
            flag: false,
            ident: false,
            measure: false,
            base: 0,
            bitmap: 0,
            snr_db: None,
            rung: 0,
            n: 0,
        }
    }

    /// Serialise into exactly [`CONTROL_BYTES`] bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; CONTROL_BYTES] {
        let snr = match self.snr_db {
            None => GRANT_SNR_UNKNOWN,
            Some(value) => ((value.round_ties_even().clamp(-31.0, 31.0) as i8) as u8) & 0x3F,
        };
        [
            (self.kind.to_bits() << 6)
                | (self.count.min(7) << 3)
                | (u8::from(self.data_floor) << 2)
                | (self.level & 0x03),
            ((self.session & 0x7F) << 1) | u8::from(self.fam),
            (u8::from(self.flag) << 7) | (u8::from(self.ident || self.measure) << 6) | snr,
            self.base,
            ((self.bitmap >> 8) & 0xFF) as u8,
            (self.bitmap & 0xFF) as u8,
            ((self.rung & 0x1F) << 3) | (self.n.min(7) & 0x07),
        ]
    }

    /// Parse one.
    ///
    /// # Errors
    /// If the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, FrameError> {
        if payload.len() < CONTROL_BYTES {
            return Err(FrameError::TooShort);
        }
        let raw = payload[2] & 0x3F;
        let snr_db = (raw != GRANT_SNR_UNKNOWN).then(|| {
            f64::from(if raw >= 32 {
                i16::from(raw) - 64
            } else {
                i16::from(raw)
            })
        });
        let kind = GrantKind::from_bits(payload[0] >> 6);
        let bit = payload[2] & 0x40 != 0;
        let closing = matches!(kind, GrantKind::Disc | GrantKind::DiscAck);
        Ok(Self {
            kind,
            count: (payload[0] >> 3) & 0x07,
            data_floor: payload[0] & 0x04 != 0,
            level: payload[0] & 0x03,
            session: payload[1] >> 1,
            fam: payload[1] & 0x01 != 0,
            flag: payload[2] & 0x80 != 0,
            ident: bit && closing,
            measure: bit && !closing,
            snr_db,
            base: payload[3],
            bitmap: (u16::from(payload[4]) << 8) | u16::from(payload[5]),
            rung: payload[6] >> 3,
            n: payload[6] & 0x07,
        })
    }

    /// Acknowledgement semantics, as [`ControlFrame::received`]; a report acknowledges only
    /// what is below `base`.
    #[must_use]
    pub fn received(&self, seq: u8) -> bool {
        let distance = seq_distance(seq, self.base);
        if distance >= WINDOW {
            return distance >= SEQ_MOD / 2;
        }
        !self.reports() && (self.bitmap >> distance) & 1 == 1
    }

    /// Whether the bitmap reports the other station's last measurement ([`REPORT`]).
    #[must_use]
    pub const fn reports(&self) -> bool {
        self.bitmap & REPORT != 0
    }

    /// In a report: whether the measurement's frame `index` arrived.
    #[must_use]
    pub const fn measured(&self, index: usize) -> bool {
        self.reports() && index < WINDOW - 1 && (self.bitmap >> (index + 1)) & 1 == 1
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
            turnaround_s: 0.0,
            identifier_s: 0.0,
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
            follows: 3,
            burst_floor: true,
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
            follows: 0,
            burst_floor: false,
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
            follows: 0,
            burst_floor: false,
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
            follows: 0,
            burst_floor: false,
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
            follows: 0,
            burst_floor: false,
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
            DataKind::Probe,
            DataKind::ProbeAck,
            DataKind::Datagram,
            DataKind::Measure,
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
        // the kind's three bits are all in use since protocol 9: 7 is a measurement frame
        assert!(matches!(
            decode_data(&[0xE0, 0, 0]),
            Ok((
                DataHeader {
                    kind: DataKind::Measure,
                    ..
                },
                _
            ))
        ));
        assert_eq!(data_kind(&[0xE0]), Some(DataKind::Measure));
        assert_eq!(data_kind(&[]), None);
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
