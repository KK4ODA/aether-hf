//! Payload compression, negotiated at connect time (roadmap P3-6).
//!
//! An HF link carries a few hundred bits per second at best, and text — which is most of what
//! Winlink and a keyboard-to-keyboard contact move — compresses well. Measured on the messages
//! in this module's tests: **27 % off a short one** (429 bytes, about a net check-in) and
//! **44 % off a longer one** (1.5 kB), where the coder has history to work with. At the
//! slowest mode's 197 bit/s that is minutes. It is the cheapest throughput there is.
//!
//! # Where it sits, and why
//!
//! Above the ARQ, on the byte stream, not on each frame.
//!
//! Compressing frames individually would be the obvious place, and it is the wrong one: a
//! frame is 26 bytes on the slowest mode and 732 on the fastest, and a compressor with no
//! history makes small blocks *larger*. Compressing the stream gives the coder the whole
//! message to work with.
//!
//! It works above the ARQ because the ARQ already guarantees what a stream decompressor
//! needs: the link layer delivers bytes **in order and exactly once**. Selective repeat, HARQ
//! and retransmission all happen underneath and are invisible here. What the decompressor
//! sees is the byte stream the compressor produced.
//!
//! # The algorithm is deflate, and that is a deliberate non-choice
//!
//! RFC 1951, with a sync flush after each *application write* so nothing sits in the coder
//! waiting for input that may never come. It is publicly specified, every language has it in
//! its standard library or one crate away, and an independent implementation can interoperate
//! by reading the RFC. Something cleverer would compress a little better and would make a
//! second implementation of this modem harder to write, which is the wrong trade for an open
//! mode.
//!
//! The flush is not free, and the cost is visible: the same 1.5 kB message written in one go
//! saves 44 %, and written in 144-byte pieces saves 27 %, because each flush closes a block
//! early. Frames are chunked *below* this, so a normal application — one that hands over a
//! message — pays nothing for it. One that writes a byte at a time gets very little benefit,
//! and a test pins that so the behaviour is known rather than discovered.
//!
//! # Negotiation
//!
//! The connect handshake carries a capability byte in both directions. Compression is used
//! only if **both** stations offered it, so a station that does not implement it, or has it
//! switched off, is never sent a stream it cannot read.

use std::io::Write;

use flate2::{Compression as Level, write::DeflateDecoder, write::DeflateEncoder};

/// Capability bit for stream compression, in the connect handshake's `caps` byte.
pub const CAP_DEFLATE: u8 = 0x01;

/// What this build offers a peer.
#[must_use]
pub fn offered_capabilities(compress: bool) -> u8 {
    if compress { CAP_DEFLATE } else { 0 }
}

/// Whether compression is used, given what each side offered.
///
/// Both have to offer it. A station that cannot decompress must never be sent a compressed
/// stream, and the only safe reading of a missing bit is that it cannot.
#[must_use]
pub fn negotiated(mine: u8, theirs: u8) -> bool {
    mine & theirs & CAP_DEFLATE != 0
}

/// One direction of a compressed byte stream.
///
/// Compression is a stream, so this is stateful by nature: what it emits for a given input
/// depends on everything that went before it.
pub struct Compressor {
    encoder: Option<DeflateEncoder<Vec<u8>>>,
    /// Bytes taken in since the session started.
    pub bytes_in: usize,
    /// Bytes emitted since the session started.
    pub bytes_out: usize,
}

impl std::fmt::Debug for Compressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compressor")
            .field("active", &self.encoder.is_some())
            .field("bytes_in", &self.bytes_in)
            .field("bytes_out", &self.bytes_out)
            .finish()
    }
}

impl Compressor {
    /// A compressor, or a pass-through when compression was not negotiated.
    #[must_use]
    pub fn new(active: bool) -> Self {
        Self {
            // Level 9: the link is the bottleneck by three orders of magnitude, so spending
            // milliseconds of processor time to save bytes of air time is always the right
            // way round.
            encoder: active.then(|| DeflateEncoder::new(Vec::new(), Level::best())),
            bytes_in: 0,
            bytes_out: 0,
        }
    }

    /// Whether anything is actually being compressed.
    #[must_use]
    pub fn active(&self) -> bool {
        self.encoder.is_some()
    }

    /// How much smaller the stream has been made, as a fraction of the input.
    ///
    /// Zero before anything has been sent.
    #[must_use]
    pub fn saving(&self) -> f64 {
        if self.bytes_in == 0 {
            return 0.0;
        }
        1.0 - self.bytes_out as f64 / self.bytes_in as f64
    }

    /// Take application bytes and return what should go on the air.
    ///
    /// The coder is flushed, so everything given here is in the returned bytes: a compressor
    /// that held the last few back would stall a message waiting for input that never comes.
    pub fn push(&mut self, data: &[u8]) -> Vec<u8> {
        self.bytes_in += data.len();
        let Some(encoder) = self.encoder.as_mut() else {
            self.bytes_out += data.len();
            return data.to_vec();
        };
        // A write to an in-memory buffer cannot fail, and a flush of a deflate stream cannot
        // either; both are infallible in practice and treated as such rather than propagating
        // an error the caller could do nothing about.
        let _ = encoder.write_all(data);
        let _ = encoder.flush();
        let out = std::mem::take(encoder.get_mut());
        self.bytes_out += out.len();
        out
    }
}

/// The other direction.
pub struct Decompressor {
    decoder: Option<DeflateDecoder<Vec<u8>>>,
    /// Bytes taken off the air since the session started.
    pub bytes_in: usize,
    /// Bytes handed to the application since the session started.
    pub bytes_out: usize,
    /// Set once the stream has been found to be malformed; nothing more is delivered.
    failed: bool,
}

impl std::fmt::Debug for Decompressor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decompressor")
            .field("active", &self.decoder.is_some())
            .field("bytes_in", &self.bytes_in)
            .field("bytes_out", &self.bytes_out)
            .field("failed", &self.failed)
            .finish()
    }
}

impl Decompressor {
    /// A decompressor, or a pass-through when compression was not negotiated.
    #[must_use]
    pub fn new(active: bool) -> Self {
        Self {
            decoder: active.then(|| DeflateDecoder::new(Vec::new())),
            bytes_in: 0,
            bytes_out: 0,
            failed: false,
        }
    }

    /// Whether anything is actually being decompressed.
    #[must_use]
    pub fn active(&self) -> bool {
        self.decoder.is_some()
    }

    /// Whether the stream has been found to be malformed.
    ///
    /// The ARQ delivers exactly what was sent, so this can only mean the two stations
    /// disagreed about whether the stream was compressed at all. Nothing further is
    /// delivered, because guessing at the rest would hand the application rubbish.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.failed
    }

    /// Take bytes off the air and return what the application should see.
    pub fn push(&mut self, data: &[u8]) -> Vec<u8> {
        self.bytes_in += data.len();
        let Some(decoder) = self.decoder.as_mut() else {
            self.bytes_out += data.len();
            return data.to_vec();
        };
        if self.failed {
            return Vec::new();
        }
        if decoder.write_all(data).is_err() || decoder.flush().is_err() {
            self.failed = true;
            return Vec::new();
        }
        let out = std::mem::take(decoder.get_mut());
        self.bytes_out += out.len();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A realistic Winlink message: headers, prose, and a signature.
    fn message() -> Vec<u8> {
        let body = "\
MID: A1B2C3D4E5F6\r\n\
Date: 2026/09/14 03:00\r\n\
From: W4ODA\r\n\
To: KK4XYZ\r\n\
Subject: Weekly net check-in\r\n\
\r\n\
Checked in to the Thursday evening net on 40 metres. Conditions were poor for the first\r\n\
half hour and improved markedly after sunset. Fourteen stations logged, three of them\r\n\
portable. No traffic to pass. Next net same time next week, same frequency, and the\r\n\
alternate is the usual one if the band is unusable.\r\n\
\r\n\
73 de W4ODA\r\n";
        body.as_bytes().to_vec()
    }

    #[test]
    fn negotiation_needs_both_sides_to_offer_it() {
        // a station that cannot decompress must never be sent a compressed stream, and a
        // missing bit can only safely be read as "cannot"
        assert!(negotiated(CAP_DEFLATE, CAP_DEFLATE));
        assert!(!negotiated(CAP_DEFLATE, 0));
        assert!(!negotiated(0, CAP_DEFLATE));
        assert!(!negotiated(0, 0));
        // an unknown capability bit does not turn compression on by accident
        assert!(!negotiated(0x80, 0x80));
    }

    #[test]
    fn offered_capabilities_says_what_this_build_can_do() {
        assert_eq!(offered_capabilities(true), CAP_DEFLATE);
        assert_eq!(offered_capabilities(false), 0);
    }

    /// A longer message: several paragraphs of varied prose, as a real one would be.
    fn long_message() -> Vec<u8> {
        let mut out = message();
        for paragraph in [
            "The antenna is an inverted vee at about ten metres, fed with ladder line into a\r\n             balanced tuner. It is not resonant anywhere in particular, which matters less than\r\n             people say once there is a tuner in the path.\r\n\r\n",
            "Propagation on the higher bands has been unreliable all month. Twenty metres opens\r\n             late and closes early; fifteen has been usable perhaps one afternoon in four. Forty\r\n             remains the workhorse after dark and will be until the flux recovers.\r\n\r\n",
            "Equipment notes: the interface is a home-made one, an isolation transformer on each\r\n             audio line and an opto-isolator across the keying line, in a diecast box. Total cost\r\n             was under twenty pounds and it has never given a moment of trouble.\r\n\r\n",
            "Requests for the next net: a volunteer to take the log, and somebody with a decent\r\n             signal into the north to relay for the two stations who have been unreadable at this\r\n             end for a fortnight. Anyone able to help, please say so on the air or by mail.\r\n\r\n",
        ] {
            out.extend_from_slice(paragraph.as_bytes());
        }
        out
    }

    #[test]
    fn a_short_message_survives_the_round_trip_and_gets_meaningfully_smaller() {
        // 429 bytes is about a net check-in, and it is the hard case: deflate has almost no
        // history to work with. A quarter off is still a quarter off the air time.
        let original = message();
        let mut tx = Compressor::new(true);
        let mut rx = Decompressor::new(true);
        let wire = tx.push(&original);
        assert_eq!(rx.push(&wire), original);
        assert!(
            tx.saving() > 0.25,
            "{} bytes became {} — a saving of only {:.1} %",
            original.len(),
            wire.len(),
            100.0 * tx.saving()
        );
    }

    #[test]
    fn a_longer_message_does_much_better() {
        // this is where compression earns its place: the coder has history, and a Winlink
        // message is usually this length or more
        let original = long_message();
        let mut tx = Compressor::new(true);
        let mut rx = Decompressor::new(true);
        let wire = tx.push(&original);
        assert_eq!(rx.push(&wire), original);
        assert!(
            tx.saving() > 0.40,
            "{} bytes became {} — a saving of only {:.1} %",
            original.len(),
            wire.len(),
            100.0 * tx.saving()
        );
    }

    #[test]
    fn the_cost_of_flushing_is_known_rather_than_discovered() {
        // The flush is what stops the end of a message sitting in the coder, and it closes a
        // block early every time. An application that hands over a whole message pays almost
        // nothing; one that writes in small pieces pays a lot. Both numbers are pinned here
        // so the behaviour is a documented property and not a surprise in the field.
        let original = long_message();
        let mut whole = Compressor::new(true);
        whole.push(&original);
        assert!(
            whole.saving() > 0.40,
            "one write saves only {:.1} %",
            100.0 * whole.saving()
        );

        let mut piecemeal = Compressor::new(true);
        for chunk in original.chunks(144) {
            piecemeal.push(chunk);
        }
        assert!(
            piecemeal.saving() > 0.20,
            "writing in pieces saves only {:.1} %, which is not worth doing at all",
            100.0 * piecemeal.saving()
        );
        assert!(
            piecemeal.saving() < whole.saving(),
            "the flush appears to be free, which cannot be right"
        );
    }

    #[test]
    fn the_stream_survives_being_written_in_arbitrary_pieces() {
        // the application writes when it likes and the link takes what it can, so neither end
        // sees the message in the shape the other produced it
        let original = message();
        let mut tx = Compressor::new(true);
        let mut rx = Decompressor::new(true);
        let mut got = Vec::new();
        for chunk in original.chunks(37) {
            let wire = tx.push(chunk);
            for piece in wire.chunks(11) {
                got.extend(rx.push(piece));
            }
        }
        assert_eq!(got, original);
    }

    #[test]
    fn nothing_is_ever_held_back_waiting_for_more_input() {
        // A compressor that kept the last few bytes in its window would stall the end of
        // every message until something else was sent. This is what the flush is for.
        let mut tx = Compressor::new(true);
        let mut rx = Decompressor::new(true);
        for line in ["first\r\n", "second\r\n", "third\r\n"] {
            let wire = tx.push(line.as_bytes());
            assert!(!wire.is_empty(), "{line:?} produced nothing to send");
            assert_eq!(
                String::from_utf8_lossy(&rx.push(&wire)),
                line,
                "{line:?} did not come out immediately"
            );
        }
    }

    #[test]
    fn a_pass_through_is_exactly_a_pass_through() {
        let mut tx = Compressor::new(false);
        let mut rx = Decompressor::new(false);
        assert!(!tx.active() && !rx.active());
        let original = message();
        let wire = tx.push(&original);
        assert_eq!(wire, original, "an inactive compressor changed the bytes");
        assert_eq!(rx.push(&wire), original);
        assert!((tx.saving() - 0.0).abs() < 1e-12);
    }

    #[test]
    fn incompressible_data_is_not_made_much_worse() {
        // deflate on random bytes costs a few per cent; what matters is that it is bounded,
        // because a link that ran slower with compression on would be a trap
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let random: Vec<u8> = (0..4096)
            .map(|_| {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
            })
            .collect();
        let mut tx = Compressor::new(true);
        let mut rx = Decompressor::new(true);
        let wire = tx.push(&random);
        assert_eq!(rx.push(&wire), random);
        assert!(
            wire.len() < random.len() * 11 / 10,
            "{} bytes became {}",
            random.len(),
            wire.len()
        );
    }

    #[test]
    fn a_malformed_stream_stops_rather_than_delivering_rubbish() {
        // the ARQ delivers exactly what was sent, so this can only mean the two stations
        // disagreed about whether the stream was compressed at all
        let mut rx = Decompressor::new(true);
        let out = rx.push(&[0xFF; 64]);
        assert!(out.is_empty() || rx.failed());
        let _ = rx.push(b"more");
        assert!(rx.failed(), "it kept trying to read a stream it had lost");
        assert!(rx.push(b"and more").is_empty());
    }

    #[test]
    fn the_saving_is_reported_so_an_operator_can_see_it() {
        let mut tx = Compressor::new(true);
        assert!((tx.saving() - 0.0).abs() < 1e-12, "nothing sent yet");
        tx.push(&message());
        assert!(tx.saving() > 0.0 && tx.saving() < 1.0, "{}", tx.saving());
        assert_eq!(tx.bytes_in, message().len());
    }
}
