//! Stated answers (ADR-0068) over the lossy-pipe simulator: every frame says how many frames of
//! its burst follow it, and a station answers a burst at the time a frame it read stated. These
//! mirror `model/tests/test_stated.py`'s sessions, with `LinkConfig::stated_answers` on whatever
//! the default; the engine's own tests hold the single-frame rules.

use aether_link::{
    Container, LinkConfig, LinkEngine, PhyTiming, Role, State, TwoStationSim,
    rate::{
        AWGN_THRESHOLD_DB, CONTROL_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB,
        NARROW_CONTROL_THRESHOLD_DB, WIDE_FLOOR_MARGIN_DB,
    },
};
use aether_phy::{
    modes::{PREAMBLE_SYMBOLS, Rung, air_interface},
    waveform::{NARROW_500, WIDE_2300, WaveformParams},
};

/// The timing the model's harness gives an air, with its start-of-frame signals.
fn air_timing(params: WaveformParams) -> PhyTiming {
    let air = air_interface(params);
    let wide = params == WIDE_2300;
    PhyTiming {
        data_frame_s: air.long.duration_s(),
        control_frame_s: air.short.duration_s(),
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        tx_latency_s: 0.0,
        preamble_detect_s: Some((PREAMBLE_SYMBOLS + 2) as f64 * params.symbol_period_s()),
        data_capacity: air.ladder().iter().map(Rung::payload_bytes).collect(),
        mode_threshold_db: if wide {
            AWGN_THRESHOLD_DB.to_vec()
        } else {
            NARROW_AWGN_THRESHOLD_DB.to_vec()
        },
        floor_data_frame_s: Some(air.tone_data()[0].duration_s()),
        floor_control_frame_s: Some(air.tone_control().duration_s()),
        floor_modes: air.floor_modes(),
        control_threshold_db: Some(if wide {
            CONTROL_THRESHOLD_DB
        } else {
            NARROW_CONTROL_THRESHOLD_DB
        }),
        floor_margin_db: wide.then_some(WIDE_FLOOR_MARGIN_DB),
        floor_preamble_detect_s: Some(aether_phy::tone::announce_delay_s(0.1)),
    }
}

fn stated(config: &LinkConfig) -> LinkConfig {
    LinkConfig {
        stated_answers: true,
        ..config.clone()
    }
}

fn pair(timing: &PhyTiming, config: &LinkConfig) -> (LinkEngine, LinkEngine) {
    (
        LinkEngine::new("W4ODA", timing.clone(), config.clone(), 1),
        LinkEngine::new("KK4XYZ", timing.clone(), config.clone(), 2),
    )
}

#[test]
fn bursts_state_their_counts_and_a_session_both_ways_completes() {
    // every transmission is at most four frames, each stating exactly how many follow it — an
    // acknowledgement leading the other station's burst too — and a session with traffic both
    // ways completes and closes without one station keyed over the other
    for params in [WIDE_2300, NARROW_500] {
        for snr_db in [15.0, 3.0, -8.0] {
            let timing = air_timing(params);
            let (mut a, mut b) = pair(&timing, &stated(&LinkConfig::default()));
            let there: Vec<u8> = (0..1536u32).map(|i| (i % 256) as u8).collect();
            let back: Vec<u8> = (0..200u32).map(|i| (i % 256) as u8).collect();
            a.connect("KK4XYZ").expect("idle");
            a.send(&there);
            b.send(&back);
            let mut sim = TwoStationSim::new(a, b, snr_db, 7);
            let mut until = 0.0;
            while until < 1500.0 && (sim.delivered(1) != there || sim.delivered(0) != back) {
                until += 5.0;
                sim.run(until, 1e9);
            }
            assert_eq!(sim.delivered(1), there.as_slice(), "{snr_db} dB");
            assert_eq!(sim.delivered(0), back.as_slice(), "{snr_db} dB");
            sim.engine_mut(0).disconnect();
            sim.run(until + 300.0, 3.0);
            assert!(
                sim.events(0).iter().any(|e| e == "disconnected:closed"),
                "{:?}",
                sim.events(0)
            );
            assert_eq!(
                sim.collisions(0, 1) + sim.collisions(1, 0),
                0,
                "{snr_db} dB"
            );
            for who in 0..2 {
                let frames = sim.frames_sent(who);
                let mut i = 0;
                while i < frames.len() {
                    let mut j = i + 1;
                    while j < frames.len() && (frames[j].t_start - frames[j - 1].t_end).abs() < 1e-9
                    {
                        j += 1;
                    }
                    let burst = &frames[i..j];
                    assert!(burst.len() <= 4, "{burst:?}");
                    for (k, f) in burst.iter().enumerate() {
                        assert_eq!(usize::from(f.follows), burst.len() - 1 - k, "{burst:?}");
                    }
                    i = j;
                }
            }
        }
    }
}

#[test]
fn a_receiver_hearing_frames_it_cannot_read_keeps_the_link() {
    // ADR-0033's case with stated answers: the path falls below the ordinary rungs while the
    // called station holds the turn; its bursts are not answered, and the receiving station,
    // which no longer transmits, keeps the session for the frames it hears arriving
    let t = air_timing(NARROW_500);
    let config = stated(&LinkConfig {
        max_mode: 4,
        max_burst_s: Some(28.85),
        ..LinkConfig::default()
    });
    let (mut a, mut b) = pair(&t, &config);
    let fade = 25.0;
    let message: Vec<u8> = (0..480u32).map(|i| (i % 240) as u8).collect();
    a.connect("KK4XYZ").expect("idle");
    a.send(b"hello");
    b.send(&message);
    let mut sim = TwoStationSim::new(a, b, 10.0, 5)
        .with_snr_schedule(Box::new(move |t| if t >= fade { -16.0 } else { 10.0 }));
    sim.run(fade, 1e9);
    assert_eq!(sim.engine(1).role(), Role::Iss, "not the case");
    assert!(!sim.delivered(0).is_empty() && sim.delivered(0).len() < message.len());
    let mut until = fade;
    while until < 1500.0 && sim.delivered(0) != message {
        until += 10.0;
        sim.run(until, 1e9);
    }
    assert_eq!(sim.delivered(0), message.as_slice());
    for who in 0..2 {
        assert!(
            !sim.events(who).iter().any(|e| e.contains("link timeout")),
            "{:?}",
            sim.events(who)
        );
    }
}

#[test]
fn a_disconnect_leaves_once_the_floor_went_unanswered() {
    // ADR-0039 with stated answers: a path that carries none of the queue sends nothing back,
    // so the sender asked to leave goes once nothing has been heard for its patience and every
    // frame left has gone on the floor unanswered — before the link timeout, its DISC answered
    let t = air_timing(NARROW_500);
    let (mut a, b) = pair(&t, &stated(&LinkConfig::default()));
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 6.0, 3);
    sim.run(60.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Connected);
    sim.set_thresholds(Some(vec![99.0; NARROW_AWGN_THRESHOLD_DB.len()]));
    sim.engine_mut(0).send(&[0u8; 400]);
    sim.engine_mut(0).disconnect();
    let asked = 60.0;
    sim.run(asked + 600.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e.starts_with("disconnect:") && e.contains("not acknowledged")),
        "{:?}",
        sim.events(0)
    );
    assert!(
        sim.events(0).iter().any(|e| e == "disconnected:closed"),
        "{:?}",
        sim.events(0)
    );
}

#[test]
fn every_data_frame_goes_in_a_burst_of_at_most_four() {
    let t = air_timing(WIDE_2300);
    let (mut a, b) = pair(&t, &stated(&LinkConfig::default()));
    let message = vec![0x5Au8; 8000];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 12.0, 11);
    sim.run(2000.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    let data = sim
        .frames_sent(0)
        .iter()
        .filter(|f| f.container == Container::Data)
        .count();
    assert!(data > 4);
    assert!(sim.frames_sent(0).iter().all(|f| f.follows <= 3));
}
