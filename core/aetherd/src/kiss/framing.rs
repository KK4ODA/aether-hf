//! KISS framing: the byte stream a KISS client and a TNC exchange (K9NG/KA9Q, 1987).
//!
//! A frame is `FEND · type · data · FEND`, with `FEND` and `FESC` inside it sent as `FESC TFEND`
//! and `FESC TFESC`. The type byte carries the port in its high nibble and the command in its low
//! one; `0xFF` is RETURN. TCP delivers the stream in pieces of any size, so [`Decoder`] keeps its
//! state between reads: a frame may arrive in many reads, several frames in one, and an escape
//! split across two. Nothing here knows about AX.25 or the radio — a frame is bytes.
//!
//! A frame this decoder cannot vouch for is dropped whole and counted, never passed on in part:
//! an escape followed by anything but `TFEND` or `TFESC` (the specification leaves the byte's
//! fate open; transmitting a guess would put corrupted data on the air), an escape still open
//! when the frame ends, and a frame longer than the limit.

/// Frame end: delimits frames.
pub const FEND: u8 = 0xC0;
/// Frame escape: the next byte is a transposed `FEND` or `FESC`.
pub const FESC: u8 = 0xDB;
/// Transposed frame end: after `FESC`, stands for a data `FEND`.
pub const TFEND: u8 = 0xDC;
/// Transposed frame escape: after `FESC`, stands for a data `FESC`.
pub const TFESC: u8 = 0xDD;

/// The KISS commands, the low nibble of the type byte.
pub mod command {
    /// A frame to transmit, or one received.
    pub const DATA: u8 = 0x00;
    /// Key-up delay, in 10 ms units.
    pub const TX_DELAY: u8 = 0x01;
    /// p-persistence, `p = (value + 1) / 256`.
    pub const PERSISTENCE: u8 = 0x02;
    /// Slot interval, in 10 ms units.
    pub const SLOT_TIME: u8 = 0x03;
    /// Time to hold the key after the frame, in 10 ms units (obsolete in the specification).
    pub const TX_TAIL: u8 = 0x04;
    /// Nonzero for full duplex.
    pub const FULL_DUPLEX: u8 = 0x05;
    /// Hardware-specific settings.
    pub const SET_HARDWARE: u8 = 0x06;
    /// A data frame the TNC acknowledges once sent (the ACKMODE extension, used by BPQ32 and
    /// others): two bytes of the client's identifier, then the frame.
    pub const ACK_MODE: u8 = 0x0C;
    /// The whole type byte `0xFF`: leave KISS mode.
    pub const RETURN: u8 = 0xFF;
}

/// One frame as the client sent it: port, command and the unescaped bytes after the type byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KissFrame {
    /// The port, 0–15 (the type byte's high nibble); 0 for RETURN.
    pub port: u8,
    /// The command (the type byte's low nibble), or `0xFF` for RETURN.
    pub command: u8,
    /// Everything after the type byte, unescaped.
    pub data: Vec<u8>,
}

/// Why a frame was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// `FESC` followed by something other than `TFEND` or `TFESC`.
    BadEscape(u8),
    /// The frame ended while an escape was open.
    DanglingEscape,
    /// Longer than the limit; the rest of it was skipped.
    TooLong,
}

impl core::fmt::Display for Malformed {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadEscape(byte) => write!(f, "0x{byte:02X} after FESC"),
            Self::DanglingEscape => f.write_str("the frame ended inside an escape"),
            Self::TooLong => f.write_str("the frame is longer than the limit"),
        }
    }
}

/// What a stream of bytes turned out to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decoded {
    /// A complete frame.
    Frame(KissFrame),
    /// A frame that was dropped, and why.
    Malformed(Malformed),
}

/// Frames out of a byte stream that arrives in pieces.
#[derive(Debug, Clone)]
pub struct Decoder {
    /// The longest frame taken, type byte included, after unescaping.
    limit: usize,
    /// Inside a frame: bytes since its opening `FEND`.
    in_frame: bool,
    /// The last byte was `FESC`.
    escaped: bool,
    /// This frame is already condemned; its bytes are skipped until its `FEND`.
    condemned: Option<Malformed>,
    buffer: Vec<u8>,
}

impl Decoder {
    /// A decoder that refuses frames longer than `limit` bytes (type byte included).
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            in_frame: false,
            escaped: false,
            condemned: None,
            buffer: Vec::new(),
        }
    }

    /// Take the next piece of the stream; return every frame it completed, in order.
    ///
    /// Bytes before the first `FEND` are line noise from before the client synchronised and
    /// are skipped, as the specification has a TNC do. Empty frames — `FEND FEND`, which clients
    /// send to flush a receiver — are not frames.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Decoded> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte == FEND {
                if self.in_frame {
                    if let Some(reason) = self.finish() {
                        out.push(reason);
                    }
                }
                self.in_frame = true;
                continue;
            }
            if !self.in_frame || self.condemned.is_some() {
                continue;
            }
            let value = if self.escaped {
                self.escaped = false;
                match byte {
                    TFEND => FEND,
                    TFESC => FESC,
                    other => {
                        self.condemned = Some(Malformed::BadEscape(other));
                        continue;
                    }
                }
            } else if byte == FESC {
                self.escaped = true;
                continue;
            } else {
                byte
            };
            if self.buffer.len() >= self.limit {
                self.condemned = Some(Malformed::TooLong);
                self.buffer.clear();
                continue;
            }
            self.buffer.push(value);
        }
        out
    }

    /// The frame that just ended: what it was, or why it is dropped, or nothing if empty.
    fn finish(&mut self) -> Option<Decoded> {
        let condemned = self.condemned.take();
        let dangling = std::mem::replace(&mut self.escaped, false);
        let bytes = std::mem::take(&mut self.buffer);
        if let Some(reason) = condemned {
            return Some(Decoded::Malformed(reason));
        }
        if dangling {
            return Some(Decoded::Malformed(Malformed::DanglingEscape));
        }
        let (&kind, data) = bytes.split_first()?;
        let (port, command) = if kind == command::RETURN {
            (0, command::RETURN)
        } else {
            (kind >> 4, kind & 0x0F)
        };
        Some(Decoded::Frame(KissFrame {
            port,
            command,
            data: data.to_vec(),
        }))
    }
}

/// One frame for the wire: `FEND`, the type byte, the data, `FEND`, escaped as they must be.
/// The type byte is escaped as well: port 12's data command is `0xC0`, which is `FEND`.
#[must_use]
pub fn encode(port: u8, command: u8, data: &[u8]) -> Vec<u8> {
    let kind = if command == command::RETURN {
        command::RETURN
    } else {
        ((port & 0x0F) << 4) | (command & 0x0F)
    };
    let mut out = Vec::with_capacity(data.len() + data.len() / 16 + 4);
    out.push(FEND);
    for &byte in std::iter::once(&kind).chain(data) {
        match byte {
            FEND => out.extend_from_slice(&[FESC, TFEND]),
            FESC => out.extend_from_slice(&[FESC, TFESC]),
            other => out.push(other),
        }
    }
    out.push(FEND);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(port: u8, command: u8, data: &[u8]) -> Decoded {
        Decoded::Frame(KissFrame {
            port,
            command,
            data: data.to_vec(),
        })
    }

    #[test]
    fn a_frame_is_escaped_on_the_way_out() {
        assert_eq!(
            encode(0, command::DATA, b"AB"),
            [FEND, 0x00, b'A', b'B', FEND]
        );
        // FEND and FESC inside the data are transposed
        assert_eq!(
            encode(0, command::DATA, &[FEND, 0x01, FESC]),
            [FEND, 0x00, FESC, TFEND, 0x01, FESC, TFESC, FEND]
        );
        // the type byte too: port 12's data frame would otherwise read as a frame end
        assert_eq!(
            encode(12, command::DATA, &[0x55]),
            [FEND, FESC, TFEND, 0x55, FEND]
        );
        assert_eq!(encode(0, command::RETURN, &[]), [FEND, 0xFF, FEND]);
        // TFEND and TFESC on their own are ordinary bytes
        assert_eq!(
            encode(0, command::DATA, &[TFEND, TFESC]),
            [FEND, 0x00, TFEND, TFESC, FEND]
        );
    }

    #[test]
    fn what_goes_out_comes_back_whatever_it_holds() {
        let every_byte: Vec<u8> = (0..=255u8).collect();
        let mut decoder = Decoder::new(4096);
        for port in [0u8, 1, 12, 15] {
            let decoded = decoder.push(&encode(port, command::DATA, &every_byte));
            assert_eq!(decoded, [frame(port, command::DATA, &every_byte)]);
        }
    }

    #[test]
    fn a_stream_in_pieces_gives_the_same_frames_as_in_one() {
        let payloads: [&[u8]; 4] = [
            b"first frame",
            &[FEND, FESC, FEND, FESC, 0x00],
            b"",
            &[0xC0; 40],
        ];
        let mut stream = Vec::new();
        for (index, payload) in payloads.iter().enumerate() {
            stream.extend(encode(index as u8, command::DATA, payload));
        }
        let whole = Decoder::new(4096).push(&stream);
        assert_eq!(whole.len(), 4);
        // every split point, including one between FESC and what it escapes
        for size in 1..=stream.len() {
            let mut decoder = Decoder::new(4096);
            let mut pieces = Vec::new();
            for chunk in stream.chunks(size) {
                pieces.extend(decoder.push(chunk));
            }
            assert_eq!(pieces, whole, "read size {size}");
        }
        // a frame with nothing after the type byte is still a frame
        assert_eq!(whole[2], frame(2, command::DATA, b""));
    }

    #[test]
    fn several_frames_in_one_read_come_out_in_order() {
        let mut stream = Vec::new();
        for n in 0..10u8 {
            stream.extend(encode(0, command::DATA, &[n; 3]));
        }
        let decoded = Decoder::new(4096).push(&stream);
        let firsts: Vec<u8> = decoded
            .iter()
            .map(|d| match d {
                Decoded::Frame(f) => f.data[0],
                Decoded::Malformed(_) => 0xFF,
            })
            .collect();
        assert_eq!(firsts, (0..10u8).collect::<Vec<_>>());
    }

    #[test]
    fn noise_before_the_first_frame_and_empty_frames_are_nothing() {
        let mut decoder = Decoder::new(4096);
        let mut stream = b"garbage before sync".to_vec();
        stream.extend([FEND, FEND, FEND]);
        stream.extend(encode(0, command::DATA, b"x"));
        assert_eq!(decoder.push(&stream), [frame(0, command::DATA, b"x")]);
        // shared FENDs between frames, as many clients send them
        let decoded = decoder.push(&[FEND, 0x00, b'a', FEND, 0x00, b'b', FEND]);
        assert_eq!(
            decoded,
            [frame(0, command::DATA, b"a"), frame(0, command::DATA, b"b")]
        );
    }

    #[test]
    fn a_malformed_frame_is_dropped_whole_and_the_next_one_is_read() {
        let mut decoder = Decoder::new(4096);
        // an escape of something that is not TFEND or TFESC
        let mut stream = vec![FEND, 0x00, b'a', FESC, 0x41, b'b', FEND];
        stream.extend(encode(0, command::DATA, b"after"));
        assert_eq!(
            decoder.push(&stream),
            [
                Decoded::Malformed(Malformed::BadEscape(0x41)),
                frame(0, command::DATA, b"after")
            ]
        );
        // an escape left open when the frame ends
        assert_eq!(
            decoder.push(&[FEND, 0x00, b'a', FESC, FEND]),
            [Decoded::Malformed(Malformed::DanglingEscape)]
        );
        // too long: skipped to its end, and the decoder is ready for the next
        let mut decoder = Decoder::new(16);
        let mut stream = encode(0, command::DATA, &[0x33; 64]);
        stream.extend(encode(0, command::DATA, b"ok"));
        assert_eq!(
            decoder.push(&stream),
            [
                Decoded::Malformed(Malformed::TooLong),
                frame(0, command::DATA, b"ok")
            ]
        );
    }

    #[test]
    fn parameters_and_return_are_read_as_commands() {
        let mut decoder = Decoder::new(4096);
        let mut stream = encode(0, command::TX_DELAY, &[50]);
        stream.extend(encode(1, command::PERSISTENCE, &[63]));
        stream.extend([FEND, 0xFF, FEND]);
        assert_eq!(
            decoder.push(&stream),
            [
                frame(0, command::TX_DELAY, &[50]),
                frame(1, command::PERSISTENCE, &[63]),
                frame(0, command::RETURN, &[]),
            ]
        );
    }
}
