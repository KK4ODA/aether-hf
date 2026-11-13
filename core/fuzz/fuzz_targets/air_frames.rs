// SPDX-License-Identifier: MIT OR Apache-2.0
//! A frame's payload off the air, once its CRC has passed: every decoder the receiver runs on one.
//! Anyone on the frequency can send a frame whose CRC passes, since every encoding is published.
#![no_main]

use aether_link::{datagram, frames};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = frames::data_kind(data);
    if let Ok((_, body)) = frames::decode_data(data) {
        let _ = datagram::parse_body(&body);
    }
    let _ = datagram::read_fragment(data);
    let _ = frames::ProbeBody::decode(data);
    let _ = frames::ConnectBody::decode(data);
    if let Ok(grant) = frames::GrantFrame::decode(data) {
        for seq in 0..=u8::MAX {
            let _ = grant.received(seq);
        }
    }
    let _ = frames::unpack_callsign(data);
    let _ = datagram::parse_body(data);
});
