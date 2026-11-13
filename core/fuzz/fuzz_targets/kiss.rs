// SPDX-License-Identifier: MIT OR Apache-2.0
//! A KISS client's byte stream, in pieces the size the first byte says, as the port reads it:
//! deframed with the server's limit, each frame classified and its client guessed.
#![no_main]

use aetherd::kiss::dialect::{classify, guess_client};
use aetherd::kiss::framing::{Decoded, Decoder};
use aetherd::kiss::server::MAX_FRAME;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&size, stream)) = data.split_first() else {
        return;
    };
    let mut decoder = Decoder::new(MAX_FRAME);
    for piece in stream.chunks(usize::from(size).max(1)) {
        for decoded in decoder.push(piece) {
            if let Decoded::Frame(frame) = decoded {
                let _ = classify(&frame);
                if let Some((&frame_type, rest)) = frame.data.split_first() {
                    let _ = guess_client(frame_type, rest, false);
                    let _ = guess_client(frame_type, rest, true);
                }
            }
        }
    }
});
