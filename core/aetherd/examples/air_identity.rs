//! Step 10.1's identity check (ADR-0113 §4): two real stations through a seeded channel, every
//! output written out, so two builds can be compared line for line.
//!
//! It uses only the station's public interface as it stood before the PHY boundary, so the same
//! file builds against both trees. Run with `cargo run --release --example air_identity > out`.

use std::collections::VecDeque;

use aether_link::State;
use aether_phy::{AudioToBaseband, BasebandToAudio, Complex, WIDE_2300, waveform::NARROW_500};
use aetherd::{Station, StationConfig, ptt::NullPtt, station::TestPlan};

/// The block the sound card hands over: 4 096 samples, as the station tests use, unless
/// `AIR_IDENTITY_SAMPLES` says otherwise (the daemon reads 20 ms, 960).
fn block() -> usize {
    std::env::var("AIR_IDENTITY_SAMPLES")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(4096)
}
const RATE: f64 = 48_000.0;

/// A rig off frequency, as the station tests have it.
struct Shift {
    hz: f64,
    n: usize,
    down: AudioToBaseband,
    up: BasebandToAudio,
    out: VecDeque<f32>,
}

impl Shift {
    fn new(hz: f64) -> Self {
        Self {
            hz,
            n: 0,
            down: AudioToBaseband::new(WIDE_2300),
            up: BasebandToAudio::new(WIDE_2300),
            out: std::iter::repeat_n(0.0, block()).collect(),
        }
    }

    fn process(&mut self, audio: &[f32]) -> Vec<f32> {
        let fs = WIDE_2300.fs_baseband;
        let turned: Vec<Complex> = self
            .down
            .process(audio)
            .into_iter()
            .map(|(re, im)| {
                let phase = 2.0 * std::f64::consts::PI * self.hz * self.n as f64 / fs;
                self.n += 1;
                let (sin, cos) = phase.sin_cos();
                (re * cos - im * sin, re * sin + im * cos)
            })
            .collect();
        self.out.extend(self.up.process(&turned));
        self.out.drain(..audio.len()).collect()
    }
}

struct Pair {
    name: &'static str,
    a: Station<NullPtt>,
    b: Station<NullPtt>,
    gain: f32,
    noise: f32,
    state: u64,
    played: [u64; 2],
    shift_b: Option<(Shift, Shift)>,
    block: usize,
    hash: [u64; 2],
}

fn fnv(hash: &mut u64, samples: &[f32]) {
    for s in samples {
        for byte in s.to_bits().to_le_bytes() {
            *hash ^= u64::from(byte);
            *hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

impl Pair {
    fn new(
        name: &'static str,
        gain: f32,
        noise: f32,
        a: impl Fn(StationConfig) -> StationConfig,
        b: impl Fn(StationConfig) -> StationConfig,
    ) -> Self {
        let base = |call: &str| StationConfig {
            callsign: call.to_owned(),
            wait_for_clear: false,
            ..StationConfig::default()
        };
        Self {
            name,
            a: Station::new(a(base("W4ODA")), NullPtt::default(), 1),
            b: Station::new(b(base("KK4XYZ")), NullPtt::default(), 2),
            gain,
            noise,
            state: 0x1234_5678_9abc_def0,
            played: [0, 0],
            shift_b: None,
            block: 0,
            hash: [0xcbf2_9ce4_8422_2325; 2],
        }
    }

    fn gauss(&mut self) -> f32 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let u1 = ((self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64
            / (1u64 << 53) as f64)
            .max(1e-12);
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let u2 =
            (self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }

    fn say(&mut self) {
        let block = self.block;
        for (who, station) in [("a", &mut self.a), ("b", &mut self.b)] {
            for event in station.take_events() {
                println!("{block} {who} event {event}");
            }
            for report in station.take_frame_reports() {
                println!("{block} {who} frame {report:?}");
            }
            for tag in station.take_tags() {
                println!("{block} {who} tag {tag}");
            }
            for delivery in station.take_deliveries() {
                println!("{block} {who} delivery {delivery:?}");
            }
            let received = station.take_received();
            if !received.is_empty() {
                let mut h = 0xcbf2_9ce4_8422_2325u64;
                for byte in &received {
                    h ^= u64::from(*byte);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
                println!("{block} {who} received {} bytes {h:016x}", received.len());
            }
        }
    }

    fn run(
        &mut self,
        seconds: f64,
        mut done: impl FnMut(&Station<NullPtt>, &Station<NullPtt>) -> bool,
    ) {
        let size = block();
        let blocks = (seconds * RATE / size as f64) as usize;
        let mut from_a = vec![0.0f32; size];
        let mut from_b = vec![0.0f32; size];
        for _ in 0..blocks {
            let [start_a, start_b] = self.played;
            self.a.device_played(start_a);
            self.b.device_played(start_b);
            self.played[0] += size as u64;
            self.played[1] += size as u64;
            let handed_a = self.a.playback(&mut from_a).expect("playback");
            let handed_b = self.b.playback(&mut from_b).expect("playback");
            self.a.device_drains_at(start_a + handed_a as u64);
            self.b.device_drains_at(start_b + handed_b as u64);
            fnv(&mut self.hash[0], &from_a);
            fnv(&mut self.hash[1], &from_b);
            let (heard_a, heard_b) = match &mut self.shift_b {
                // b's rig off frequency: a hears b shifted, b hears a shifted back
                Some((to_a, to_b)) => (to_a.process(&from_b), to_b.process(&from_a)),
                None => (from_b.clone(), from_a.clone()),
            };
            let mut to_a = vec![0.0f32; size];
            let mut to_b = vec![0.0f32; size];
            for index in 0..size {
                to_b[index] = heard_b[index].mul_add(self.gain, self.gauss() * self.noise);
                to_a[index] = heard_a[index].mul_add(self.gain, self.gauss() * self.noise);
            }
            self.a.capture(&to_a).expect("capture");
            self.b.capture(&to_b).expect("capture");
            self.block += 1;
            self.say();
            if self.block % 64 == 0 {
                println!(
                    "{} audio {:016x} {:016x}",
                    self.block, self.hash[0], self.hash[1]
                );
            }
            if done(&self.a, &self.b) {
                return;
            }
        }
    }

    fn finish(&mut self) {
        println!(
            "== {} end block {} a {:?} b {:?} audio {:016x} {:016x}",
            self.name,
            self.block,
            self.a.state(),
            self.b.state(),
            self.hash[0],
            self.hash[1]
        );
    }
}

fn both_idle(a: &Station<NullPtt>, b: &Station<NullPtt>) -> bool {
    a.state() == State::Idle && b.state() == State::Idle && !a.transmitting() && !b.transmitting()
}

fn connected(a: &Station<NullPtt>, b: &Station<NullPtt>) -> bool {
    a.connected() && b.connected()
}

fn bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn session(mut pair: Pair, a_sends: usize, b_sends: usize, budget_s: f64) {
    println!("== {}", pair.name);
    pair.a.connect("KK4XYZ").expect("idle");
    pair.run(90.0, connected);
    pair.a.send(&bytes(a_sends, 1));
    if b_sends > 0 {
        pair.b.send(&bytes(b_sends, 2));
    }
    pair.run(budget_s, |a, b| {
        a.queued_bytes() == 0 && b.queued_bytes() == 0 && a.connected()
    });
    pair.a.disconnect();
    pair.run(90.0, both_idle);
    pair.finish();
}

fn main() {
    let narrow = |c: StationConfig| StationConfig {
        params: NARROW_500,
        ..c
    };
    let same = |c: StationConfig| c;

    session(
        Pair::new("wide-clean", 1.0, 0.0005, same, same),
        3000,
        1500,
        240.0,
    );
    session(
        Pair::new("narrow-weak", 1.0, 0.3, narrow, narrow),
        800,
        300,
        400.0,
    );
    session(
        Pair::new("wide-noisy", 1.0, 0.6, same, same),
        1000,
        0,
        300.0,
    );
    // a 500 Hz station calls a 2 300 Hz one, which moves to answer (ADR-0026)
    session(
        Pair::new("narrow-calls-wide", 1.0, 0.05, narrow, same),
        600,
        200,
        300.0,
    );
    // under the OFDM rungs: tone frames, phantoms, HARQ, re-encoding
    session(
        Pair::new("wide-floor", 1.0, 1.2, same, same),
        400,
        100,
        400.0,
    );
    session(
        Pair::new("narrow-floor", 1.0, 0.8, narrow, narrow),
        300,
        0,
        400.0,
    );

    let mut offset = Pair::new("offset", 1.0, 0.05, same, same);
    offset.shift_b = Some((Shift::new(60.0), Shift::new(-60.0)));
    session(offset, 2000, 500, 240.0);

    // an abort mid-transfer
    let mut abort = Pair::new("abort", 1.0, 0.02, same, same);
    println!("== {}", abort.name);
    abort.a.connect("KK4XYZ").expect("idle");
    abort.run(90.0, connected);
    abort.a.send(&bytes(20_000, 3));
    abort.run(40.0, |_, _| false);
    abort.a.abort();
    abort.run(120.0, both_idle);
    abort.finish();

    // a probe and a beacon
    let mut probe = Pair::new("probe-beacon", 1.0, 0.05, same, same);
    println!("== {}", probe.name);
    probe.a.probe("KK4XYZ", None).expect("probe");
    probe.run(40.0, |_, _| false);
    probe.b.beacon(None).expect("beacon");
    probe.run(30.0, |_, _| false);
    probe.finish();

    // the Test session: probe, call, message, file, ladder, disconnect
    let mut test = Pair::new("test", 1.0, 0.05, same, same);
    println!("== {}", test.name);
    test.a
        .start_test(TestPlan {
            remote: "KK4XYZ".to_owned(),
            message_bytes: 1000,
            file_bytes: 2000,
            budget_s: 400.0,
            ..TestPlan::default()
        })
        .expect("a test");
    test.run(480.0, |a, b| !a.test_running() && both_idle(a, b));
    test.finish();
}
