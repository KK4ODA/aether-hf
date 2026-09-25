//! What a KISS frame means: the VARA-compatible reading of the type byte (ADR-0019).
//!
//! Standard KISS gives the type byte's low nibble to commands — 0 data, 1 TXDELAY, 2 P,
//! 3 SLOTTIME, 4 TXTAIL, 5 FULLDUPLEX, 6 SETHARDWARE — and its high nibble to the port.
//! VARA's KISS port reads the same byte as a *frame type*: 0 an AX.25 frame (APRS clients),
//! 1 an AX.25 frame whose address fields are eight bytes (`VarAC`'s broadcasts), 2 unformatted
//! data (EA5HVK, "VARA KISS Interface", 2024). Both kinds of client connect to a port like
//! this one, and the two readings collide at 1 and 2. They are told apart by length: a
//! TXDELAY or P frame carries exactly one byte, and a VARA frame of type 1 or 2 carries a
//! frame — at least sixteen bytes of addresses for type 1, anything but a single byte in
//! practice for type 2. A one-byte type-2 payload would be read as P; that is the price.
//!
//! Also honoured: ACKMODE (0x0C: a two-byte identifier, then an AX.25 frame; the identifier is
//! sent back once the frame has gone out — BPQ32, `QtTermTCP` and Winlink Express use it to
//! pace their own AX.25 retries on a slow modem), P and SLOTTIME (they set the client's channel
//! access). TXDELAY, TXTAIL, FULLDUPLEX, SETHARDWARE and RETURN are accepted and ignored: the
//! modem keys the radio with its own lead and tail, is half duplex, and is a TCP port, not a
//! serial TNC to leave. A frame for any port but 0 is refused: this modem has one.

use super::framing::{KissFrame, command};

/// What a client's frame asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KissInput {
    /// A frame to send, with the type the receiving clients will see it as.
    Data {
        /// 0 an AX.25 frame, 1 AX.25 with eight-byte addresses, 2 unformatted.
        frame_type: u8,
        /// The frame.
        frame: Vec<u8>,
        /// The ACKMODE identifier to send back once it has gone out.
        ack: Option<[u8; 2]>,
    },
    /// A channel-access parameter: p-persistence, `p = (value + 1) / 256`.
    Persistence(u8),
    /// A channel-access parameter: the slot, in 10 ms units.
    SlotTime(u8),
    /// A parameter this modem accepts and does not use, named for the log.
    Ignored(&'static str),
    /// Not something this modem takes, and why.
    Refused(String),
}

/// Read one frame the VARA-compatible way.
#[must_use]
pub fn classify(frame: &KissFrame) -> KissInput {
    if frame.command == command::RETURN {
        return KissInput::Ignored("RETURN: a TCP port has no KISS mode to leave");
    }
    if frame.port != 0 {
        return KissInput::Refused(format!(
            "a frame for port {}: this modem has one port, 0",
            frame.port
        ));
    }
    let one = |name: &'static str| {
        if frame.data.len() == 1 {
            KissInput::Ignored(name)
        } else {
            KissInput::Refused(format!("{name} with {} bytes", frame.data.len()))
        }
    };
    match frame.command {
        command::DATA => data(0, &frame.data, None),
        // VARA's frame types 1 and 2, unless it is the one-byte parameter of the same number
        command::TX_DELAY if frame.data.len() == 1 => {
            KissInput::Ignored("TXDELAY: the modem keys with its own lead")
        }
        command::PERSISTENCE if frame.data.len() == 1 => KissInput::Persistence(frame.data[0]),
        command::TX_DELAY | command::PERSISTENCE => data(frame.command, &frame.data, None),
        command::SLOT_TIME if frame.data.len() == 1 => KissInput::SlotTime(frame.data[0]),
        command::SLOT_TIME => one("SLOTTIME"),
        command::TX_TAIL => one("TXTAIL: the modem holds the key with its own tail"),
        command::FULL_DUPLEX => one("FULLDUPLEX: the radio is half duplex"),
        command::SET_HARDWARE => KissInput::Ignored("SETHARDWARE"),
        command::ACK_MODE if frame.data.len() > 2 => {
            data(0, &frame.data[2..], Some([frame.data[0], frame.data[1]]))
        }
        command::ACK_MODE => KissInput::Refused("an ACKMODE frame with no frame in it".to_owned()),
        other => KissInput::Refused(format!("command 0x{other:02X} is not one this modem takes")),
    }
}

fn data(frame_type: u8, bytes: &[u8], ack: Option<[u8; 2]>) -> KissInput {
    if bytes.is_empty() {
        return KissInput::Refused("an empty frame".to_owned());
    }
    KissInput::Data {
        frame_type,
        frame: bytes.to_vec(),
        ack,
    }
}

/// p-persistence as the KISS P parameter gives it.
#[must_use]
pub fn persistence_of(value: u8) -> f64 {
    (f64::from(value) + 1.0) / 256.0
}

/// The slot, in seconds, as the KISS SLOTTIME parameter gives it.
#[must_use]
pub fn slot_of(value: u8) -> f64 {
    f64::from(value) * 0.01
}

/// A guess at who is connected, from what it sends — for the panel only; nothing depends on it.
#[must_use]
pub fn guess_client(frame_type: u8, frame: &[u8], ack: bool) -> &'static str {
    if frame_type == 1 {
        return "VarAC";
    }
    if ack {
        return "a packet program (ACKMODE)";
    }
    // an APRS destination ("tocall") is AP followed by the program's letters
    if frame_type == 0 && frame.len() >= 2 && frame[0] >> 1 == b'A' && frame[1] >> 1 == b'P' {
        return "an APRS program";
    }
    "a KISS client"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(port: u8, command: u8, data: &[u8]) -> KissFrame {
        KissFrame {
            port,
            command,
            data: data.to_vec(),
        }
    }

    #[test]
    fn vara_frame_types_and_standard_parameters_are_told_apart_by_length() {
        let ax25 = [0x82u8; 20];
        assert_eq!(
            classify(&frame(0, 0, &ax25)),
            KissInput::Data {
                frame_type: 0,
                frame: ax25.to_vec(),
                ack: None
            }
        );
        // VarAC's type 1 and VARA's type 2 carry frames
        for kind in [1u8, 2] {
            assert_eq!(
                classify(&frame(0, kind, &ax25)),
                KissInput::Data {
                    frame_type: kind,
                    frame: ax25.to_vec(),
                    ack: None
                }
            );
        }
        // a real TXDELAY or P carries one byte
        assert!(matches!(
            classify(&frame(0, 1, &[50])),
            KissInput::Ignored(_)
        ));
        assert_eq!(classify(&frame(0, 2, &[63])), KissInput::Persistence(63));
        assert_eq!(classify(&frame(0, 3, &[10])), KissInput::SlotTime(10));
        for command in [4u8, 5] {
            assert!(matches!(
                classify(&frame(0, command, &[0])),
                KissInput::Ignored(_)
            ));
        }
        assert!(matches!(
            classify(&frame(0, 6, &[1, 2, 3])),
            KissInput::Ignored(_)
        ));
        assert!(matches!(
            classify(&frame(0, command::RETURN, &[])),
            KissInput::Ignored(_)
        ));
    }

    #[test]
    fn ackmode_carries_its_identifier_and_other_ports_are_refused() {
        assert_eq!(
            classify(&frame(0, 0x0C, &[0x12, 0x34, 0x82, 0x84])),
            KissInput::Data {
                frame_type: 0,
                frame: vec![0x82, 0x84],
                ack: Some([0x12, 0x34])
            }
        );
        assert!(matches!(
            classify(&frame(0, 0x0C, &[0x12, 0x34])),
            KissInput::Refused(_)
        ));
        assert!(matches!(
            classify(&frame(1, 0, &[1, 2, 3])),
            KissInput::Refused(_)
        ));
        assert!(matches!(classify(&frame(0, 0, &[])), KissInput::Refused(_)));
        assert!(matches!(
            classify(&frame(0, 9, &[1])),
            KissInput::Refused(_)
        ));
    }

    #[test]
    fn channel_access_parameters_mean_what_kiss_says() {
        assert!((persistence_of(63) - 0.25).abs() < 1e-12);
        assert!((persistence_of(255) - 1.0).abs() < 1e-12);
        assert!((slot_of(10) - 0.1).abs() < 1e-12);
    }

    #[test]
    fn the_client_is_guessed_from_what_it_sends() {
        // "APRS" shifted left, as an AX.25 destination
        let aprs: Vec<u8> = b"APRS  ".iter().map(|c| c << 1).collect();
        assert_eq!(guess_client(0, &aprs, false), "an APRS program");
        assert_eq!(guess_client(1, &aprs, false), "VarAC");
        assert_eq!(
            guess_client(0, &[0x82; 14], true),
            "a packet program (ACKMODE)"
        );
        assert_eq!(guess_client(2, b"hello", false), "a KISS client");
    }
}
