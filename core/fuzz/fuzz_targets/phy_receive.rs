// SPDX-License-Identifier: MIT OR Apache-2.0
//! The physical layer's receiver, fed a real signal damaged. Random samples almost never look
//! like a frame, so the signal starts as a real one — a control frame and a data frame on the
//! ladder's middle rung, with quiet between, rendered once for each air — and the input damages
//! it: stretches overwritten, scaled, silenced, spliced from elsewhere, noise added. Then it is
//! fed to a receiver in pieces the input sizes, as the sound card hands it over: acquisition,
//! synchronisation, demodulation and decoding all run on what is nearly a frame.
//!
//! What is checked besides panics: the signal undamaged gives both frames back whole, however it
//! is cut into pieces.
#![no_main]

use std::sync::OnceLock;

use aether_phy::waveform::{NARROW_500, WIDE_2300, WaveformParams};
use aether_phy::{Complex, Modem, StreamingReceiver};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

/// A burst as it left a transmitter, at baseband.
struct Signal {
    params: WaveformParams,
    baseband: Vec<Complex>,
    payloads: Vec<Vec<u8>>,
}

fn render(params: WaveformParams) -> Signal {
    let mut modem = Modem::new(params, false);
    let quiet = vec![(0.0, 0.0); (params.fs_baseband * 0.1) as usize];
    let control: Vec<u8> = (0..modem.payload_bytes(None)).map(|i| i as u8).collect();
    let rung = modem.air().ladder().len() / 2;
    let data: Vec<u8> = (0..modem.rung_payload_bytes(rung))
        .map(|i| (i * 7) as u8)
        .collect();
    let mut baseband = quiet.clone();
    baseband.extend(modem.control_burst(&control, 0).expect("a control frame"));
    baseband.extend(&quiet);
    baseband.extend(modem.rung_burst(&data, rung, 0).expect("a data frame"));
    // the receiver hands a frame over up to 0.2 s after its end, by how the audio was cut
    baseband.extend(vec![(0.0, 0.0); (params.fs_baseband * 0.3) as usize]);
    Signal {
        params,
        baseband,
        payloads: vec![control, data],
    }
}

fn signal(narrow: bool) -> &'static Signal {
    static WIDE: OnceLock<Signal> = OnceLock::new();
    static NARROW: OnceLock<Signal> = OnceLock::new();
    if narrow {
        NARROW.get_or_init(|| render(NARROW_500))
    } else {
        WIDE.get_or_init(|| render(WIDE_2300))
    }
}

/// A sample from two input bytes each way, within what a sound card at full scale can give.
fn sample(u: &mut Unstructured<'_>) -> Result<Complex> {
    let re = f64::from(u.arbitrary::<i16>()?) / 8192.0;
    let im = f64::from(u.arbitrary::<i16>()?) / 8192.0;
    Ok((re, im))
}

/// The input's damage to the signal; how many patches it made.
fn damage(signal: &mut [Complex], u: &mut Unstructured<'_>) -> Result<u8> {
    let len = signal.len();
    let patches = u.int_in_range(0u8..=12)?;
    for _ in 0..patches {
        let at = u.choose_index(len)?;
        let span = usize::from(u.arbitrary::<u16>()?).min(len - at);
        match u.int_in_range(0u8..=5)? {
            0 => {
                let value = sample(u)?;
                signal[at..at + span].fill(value);
            }
            1 => {
                let gain = f64::from(u.int_in_range(-400i16..=400)?) / 40.0;
                for s in &mut signal[at..at + span] {
                    *s = (s.0 * gain, s.1 * gain);
                }
            }
            2 => signal[at..at + span].fill((0.0, 0.0)),
            3 => {
                let from = u.choose_index(len)?;
                let span = span.min(len - from);
                signal.copy_within(from..from + span, at.min(len - span));
            }
            4 => {
                // an impulse: a burst of static
                for s in signal[at..at + span.min(64)].iter_mut() {
                    let hit = sample(u)?;
                    *s = (s.0 + hit.0 * 4.0, s.1 + hit.1 * 4.0);
                }
            }
            _ => {
                // a carrier on the channel, at a frequency the input picks
                let step = f64::from(u.arbitrary::<u16>()?) / 65536.0 * std::f64::consts::TAU;
                let level = f64::from(u.int_in_range(0u8..=200)?) / 100.0;
                for (k, s) in signal[at..at + span].iter_mut().enumerate() {
                    let phase = step * k as f64;
                    *s = (s.0 + level * phase.cos(), s.1 + level * phase.sin());
                }
            }
        }
    }
    Ok(patches)
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(narrow) = u.arbitrary::<bool>() else {
        return;
    };
    let clean = signal(narrow);
    let mut heard = clean.baseband.clone();
    let Ok(patches) = damage(&mut heard, &mut u) else {
        return;
    };
    let piece = usize::from(u.arbitrary::<u16>().unwrap_or(4096)).max(1);
    let mut receiver = StreamingReceiver::new(clean.params, 6.0, true);
    let mut payloads = Vec::new();
    for block in heard.chunks(piece) {
        payloads.extend(receiver.feed(block).into_iter().filter_map(|f| f.payload));
    }
    if patches == 0 {
        assert_eq!(
            payloads, clean.payloads,
            "the undamaged signal, in pieces of {piece} samples"
        );
    }
});
