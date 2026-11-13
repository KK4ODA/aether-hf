// SPDX-License-Identifier: MIT OR Apache-2.0
//! A host program's command lines on the VARA-compatible port, each ended by a carriage return,
//! through the port's state machine in order.
#![no_main]

use aetherd::host::HostState;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut host = HostState::default();
    for line in data.split(|&b| b == b'\r') {
        let _ = host.command(&String::from_utf8_lossy(line));
    }
});
