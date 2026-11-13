// SPDX-License-Identifier: MIT OR Apache-2.0
//! Datagram fragments off the air, joined. A datagram needs no session: any station on the
//! frequency can send one to every listener. The input is a run of frames, each a length byte
//! and that many bytes, read as the receiver reads them (`read_fragment`) and joined by a
//! `Reassembler` with its clock moving a second a frame.
#![no_main]

use aether_link::datagram::{self, Reassembler};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut joined = Reassembler::default();
    let mut rest = data;
    let mut now_s = 0.0;
    while let Some((&len, tail)) = rest.split_first() {
        let len = usize::from(len).min(tail.len());
        let (frame, tail) = tail.split_at(len);
        rest = tail;
        now_s += 1.0;
        if let Some(fragment) = datagram::read_fragment(frame)
            && let Some(whole) = joined.add(fragment, now_s)
        {
            let _ = datagram::parse_body(&whole);
        }
    }
    joined.expire(now_s + 1000.0);
    let _ = joined.waiting();
});
