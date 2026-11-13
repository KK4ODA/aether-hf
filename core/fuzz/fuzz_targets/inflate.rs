// SPDX-License-Identifier: MIT OR Apache-2.0
//! A compressed session's bytes off the air, decompressed for the program, in pieces the size the
//! first byte says, as the link hands them over.
#![no_main]

use aetherd::Decompressor;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&size, stream)) = data.split_first() else {
        return;
    };
    let mut decompressor = Decompressor::new(true);
    for piece in stream.chunks(usize::from(size).max(1)) {
        let _ = decompressor.push(piece);
    }
    let _ = decompressor.failed();
});
