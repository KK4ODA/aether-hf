//! The session state machine over the lossy-pipe simulator (roadmap P3-2).
//!
//! These mirror `model/tests/test_link.py`. They are behavioural rather than bit-exact:
//! the lossy pipe is a stochastic model, and two implementations of it are not required to
//! draw the same random numbers. What must agree is what the protocol *does* — that a
//! transfer completes, that the rate controller follows a fade, that a break hands the
//! channel over, that HARQ rescues frames below threshold. The bit-exact comparison against
//! the model is in `model_vectors.rs`.

use aether_link::{
    Container, LinkConfig, LinkEngine, PhyTiming, RateConfig, RateController, Role, State,
    TwoStationSim,
    rate::{
        AWGN_THRESHOLD_DB, CONTROL_THRESHOLD_DB, NARROW_AWGN_THRESHOLD_DB,
        NARROW_CONTROL_THRESHOLD_DB, WIDE_FLOOR_MARGIN_DB,
    },
};
use aether_phy::{
    modes::{PREAMBLE_SYMBOLS, Rung, air_interface},
    waveform::{NARROW_500, WIDE_2300, WaveformParams},
};

/// The timing the model's harness gives an air (`phy_timing`): its ladder — the tone floor's
/// rungs (ADR-0013, ADR-0014), then its OFDM modes — optionally with a start-of-frame signal.
fn air_timing(params: WaveformParams, start_of_frame: bool) -> PhyTiming {
    let air = air_interface(params);
    let wide = params == WIDE_2300;
    PhyTiming {
        data_frame_s: air.long.duration_s(),
        control_frame_s: air.short.duration_s(),
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        tx_latency_s: 0.0,
        preamble_detect_s: start_of_frame
            .then(|| (PREAMBLE_SYMBOLS + 2) as f64 * params.symbol_period_s()),
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
        floor_preamble_detect_s: start_of_frame.then(|| aether_phy::tone::announce_delay_s(0.1)),
    }
}

/// The wide air's timing, optionally with a start-of-frame signal.
fn timing(start_of_frame: bool) -> PhyTiming {
    air_timing(WIDE_2300, start_of_frame)
}

fn pair(timing: &PhyTiming, config: &LinkConfig) -> (LinkEngine, LinkEngine) {
    (
        LinkEngine::new("W4ODA", timing.clone(), config.clone(), 1),
        LinkEngine::new("KK4XYZ", timing.clone(), config.clone(), 2),
    )
}

/// Connect, send one message, disconnect, and run to quiescence.
fn run_transfer(message: &[u8], snr_db: f64, seed: u64) -> TwoStationSim {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    a.send(message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, snr_db, seed);
    sim.run(2000.0, 3.0);
    sim
}

#[test]
fn a_session_connects_transfers_and_closes_on_a_clean_channel() {
    let message: Vec<u8> = "The quick brown fox jumps over the lazy dog. "
        .repeat(20)
        .into_bytes();
    let sim = run_transfer(&message, 15.0, 7);
    assert_eq!(sim.delivered(1), message.as_slice());
    // what the sender saw acknowledged is what the receiver delivered
    assert_eq!(sim.engine(0).stats.bytes_acked, message.len());
    assert_eq!(sim.engine(1).stats.bytes_delivered, message.len());
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.events(0).iter().any(|e| e == "connected:KK4XYZ (iss)"),
        "{:?}",
        sim.events(0)
    );
    for who in 0..2 {
        assert!(
            sim.events(who)
                .iter()
                .any(|e| e.starts_with("disconnected")),
            "station {who}: {:?}",
            sim.events(who)
        );
    }
}

#[test]
fn a_transfer_completes_across_the_usable_snr_range() {
    let message: Vec<u8> = (0..1200u32).map(|i| ((i * 37) % 256) as u8).collect();
    for snr_db in [20.0, 12.0, 6.0, 3.0] {
        let sim = run_transfer(&message, snr_db, snr_db as u64 + 3);
        assert_eq!(sim.delivered(1), message.as_slice(), "at {snr_db} dB");
    }
}

#[test]
fn throughput_rises_with_snr() {
    let message = vec![0u8; 4000];
    let mut times = Vec::new();
    for snr_db in [4.0, 18.0] {
        let t = timing(false);
        let (mut a, b) = pair(&t, &LinkConfig::default());
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        a.disconnect();
        let mut sim = TwoStationSim::new(a, b, snr_db, 11);
        times.push(sim.run(3000.0, 3.0));
        assert_eq!(sim.delivered(1), message.as_slice(), "at {snr_db} dB");
    }
    assert!(
        times[1] < 0.6 * times[0],
        "18 dB took {:.1} s, 4 dB took {:.1} s",
        times[1],
        times[0]
    );
}

#[test]
fn soft_combining_rescues_frames_below_a_mode_threshold() {
    // pinned to BPSK 1/2 (threshold about −1.8 dB) and driven at −3 dB: the transfer only
    // completes because retransmissions are combined with what came before
    let rung = air_interface(WIDE_2300)
        .rung_of(2)
        .expect("BPSK 1/2 is on the ladder");
    let config = LinkConfig {
        initial_mode: rung,
        max_mode: rung,
        max_retries: 60,
        ..LinkConfig::default()
    };
    let t = timing(false);
    let (mut a, b) = pair(&t, &config);
    let message = vec![0u8; 1500];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, -3.0, 99);
    sim.run(6000.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    assert!(
        sim.engine(1).stats.harq_rescues > 0,
        "nothing was rescued: {:?}",
        sim.engine(1).stats
    );
}

#[test]
fn a_break_hands_the_channel_over() {
    let t = timing(false);
    let (mut a, mut b) = pair(&t, &LinkConfig::default());
    let reply: Vec<u8> = "REPLY FROM KK4XYZ. ".repeat(8).into_bytes();
    let outbound = vec![0u8; 3000];
    a.connect("KK4XYZ").expect("idle");
    a.send(&outbound);
    b.send(&reply);
    b.request_break();
    let mut sim = TwoStationSim::new(a, b, 12.0, 5);
    sim.run(600.0, 3.0);
    assert_eq!(sim.delivered(1), outbound.as_slice(), "A to B");
    assert_eq!(sim.delivered(0), reply.as_slice(), "B to A after handover");
    // one handover: a TURN, or the turn offered at the end of the burst that emptied A's queue
    // and taken in B's acknowledgement (ADR-0047)
    assert_eq!(
        sim.engine(0).stats.turns + sim.engine(1).stats.turns_taken,
        1
    );
}

#[test]
fn calling_into_a_dead_channel_gives_up_rather_than_hanging() {
    // far below every mode's threshold, so nothing gets through in either direction: the
    // caller must exhaust its attempts and report why, not sit in `Connecting` for ever
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, -60.0, 1);
    sim.run(400.0, 5.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert!(
        sim.events(0).iter().any(|e| e == "disconnected:no answer"),
        "{:?}",
        sim.events(0)
    );
    assert_eq!(sim.engine(1).state(), State::Idle, "B never heard the call");
}

#[test]
fn a_simultaneous_call_resolves_to_one_session() {
    let t = timing(false);
    let (mut a, mut b) = pair(&t, &LinkConfig::default());
    let message: Vec<u8> = "data from A".repeat(3).into_bytes();
    a.connect("KK4XYZ").expect("idle");
    b.connect("W4ODA").expect("idle");
    a.send(&message);
    let mut sim = TwoStationSim::new(a, b, 15.0, 2);
    sim.run(300.0, 3.0);
    let roles = [sim.engine(0).role(), sim.engine(1).role()];
    assert!(
        roles.contains(&Role::Iss) && roles.contains(&Role::Irs),
        "roles ended up {roles:?}"
    );
    assert_eq!(sim.engine(0).session(), sim.engine(1).session());
    assert_eq!(sim.delivered(1), message.as_slice());
}

#[test]
fn a_station_answers_to_every_callsign_it_was_given() {
    // a host program owns the operator's callsign (MYCALL), and sends several when the
    // station also answers to a club or tactical call; the station answers as the one that
    // was called, so the caller sees the callsign it asked for
    let t = timing(false);
    let (mut a, mut b) = pair(&t, &LinkConfig::default());
    b.set_callsigns(&["KK4XYZ", "KK4XYZ-T"]).expect("idle");
    a.connect("KK4XYZ-T").expect("idle");
    a.send(b"to the tactical call");
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 15.0, 5);
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), b"to the tactical call");
    assert_eq!(sim.engine(1).my_call, "KK4XYZ-T");
    assert!(sim.events(1).iter().any(|e| e == "connected:W4ODA (irs)"));
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e == "connected:KK4XYZ-T (iss)")
    );
}

#[test]
fn a_message_one_byte_short_of_a_full_frame_still_crosses() {
    // the DATA container carries a full body with no length, or a partial body with two
    // length bytes; a body one byte short of full is neither, and the sender must not try
    // to build it — the modem panicked on exactly this length mid-session
    let t = timing(false);
    let capacity =
        aether_link::frames::data_capacity(t.capacity(LinkConfig::default().initial_mode));
    for length in [capacity - 1, capacity, capacity + 1, 2 * capacity - 1] {
        let message: Vec<u8> = (0..length).map(|i| (i % 256) as u8).collect();
        let sim = run_transfer(&message, 15.0, 11);
        assert_eq!(sim.delivered(1), message.as_slice(), "{length} bytes");
    }
}

#[test]
fn the_sender_learns_how_the_other_station_hears_it() {
    // every acknowledgement carries the SNR the receiver measured on the burst, and the
    // sender keeps the last one: it is the one number an operator cannot read off their
    // own receiver, and the one a panel shows as "they hear you at"
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    a.send(&[0x5a; 400]);
    let mut sim = TwoStationSim::new(a, b, 15.0, 9);
    sim.run(40.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    let heard_at = sim.engine(0).peer_snr_db().expect("an ACK carried the SNR");
    assert!((heard_at - 15.0).abs() < 3.0, "{heard_at}");
    // the receiving side has measured the same channel for its rate controller
    let (snr, margin) = sim.engine(1).rate_readings();
    assert!(snr.is_some_and(|s| (s - 15.0).abs() < 3.0), "{snr:?}");
    assert!(margin > 0.0);
    // and the report belongs to the session: it is gone when the session is
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(0).peer_snr_db(), None);
}

#[test]
fn a_call_stating_another_bandwidth_is_not_answered() {
    use aether_link::frames::{CAP_COMPRESSION, bandwidth_code, with_bandwidth};
    // the bandwidth bits are a statement of the waveform the frame was sent in; a station
    // set up for 2 300 Hz that is called by a frame claiming 500 Hz leaves it alone
    assert_eq!(bandwidth_code(with_bandwidth(0, 500)), 1);
    assert_eq!(bandwidth_code(with_bandwidth(CAP_COMPRESSION, 2300)), 0);
    assert_eq!(with_bandwidth(CAP_COMPRESSION, 500), CAP_COMPRESSION | 0x02);
    let t = timing(false);
    let narrow = LinkConfig {
        capabilities: with_bandwidth(0, 500),
        ..LinkConfig::default()
    };
    let wide = LinkConfig {
        capabilities: with_bandwidth(0, 2300),
        ..LinkConfig::default()
    };
    let mut a = LinkEngine::new("W4ODA", t.clone(), narrow.clone(), 1);
    let b = LinkEngine::new("KK4XYZ", t.clone(), wide, 2);
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.run(200.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.events(1)
            .iter()
            .any(|e| e.starts_with("ignored:W4ODA calls in another bandwidth")),
        "{:?}",
        sim.events(1)
    );
    assert!(sim.events(0).iter().any(|e| e == "disconnected:no answer"));
    // and two stations that agree connect as before
    let (mut a, b) = pair(&t, &narrow);
    a.connect("KK4XYZ").expect("idle");
    a.send(b"at five hundred hertz");
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.run(40.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    assert_eq!(bandwidth_code(sim.engine(1).peer_capabilities()), 1);
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), b"at five hundred hertz");
}

#[test]
fn a_station_moved_to_another_air_answers_calls_in_it() {
    use aether_link::frames::{bandwidth_code, with_bandwidth};
    // ADR-0026: the daemon moves the engine between sessions — to the bandwidth a host
    // program asked for, or to the narrower one a call to it came in — and the engine keeps
    // its callsigns, its counters and its session numbering across the move
    let wide_timing = timing(false);
    let narrow_timing = air_timing(NARROW_500, false);
    let narrow = LinkConfig {
        capabilities: with_bandwidth(0, 500),
        ..LinkConfig::default()
    };
    let wide = LinkConfig {
        capabilities: with_bandwidth(0, 2300),
        ..LinkConfig::default()
    };
    let a = LinkEngine::new("W4ODA", narrow_timing.clone(), narrow, 1);
    let mut b = LinkEngine::new("KK4XYZ", wide_timing.clone(), wide, 2);
    b.set_callsigns(&["KK4XYZ", "KK4XYZ-1"]).expect("idle");
    b.stats.probes_sent = 3;
    let top = narrow_timing.mode_threshold_db.len() - 1;
    b.set_air(narrow_timing.clone(), with_bandwidth(0, 500), Some(top))
        .expect("idle");
    assert_eq!(
        b.timing().mode_threshold_db,
        narrow_timing.mode_threshold_db
    );
    assert_eq!(bandwidth_code(b.config().capabilities), 1);
    assert_eq!(b.config().max_mode, top);
    assert_eq!(b.stats.probes_sent, 3);
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.engine_mut(0).connect("KK4XYZ-1").expect("idle");
    sim.engine_mut(0).send(b"a narrow call answered");
    sim.run(40.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    assert_eq!(sim.engine(1).state(), State::Connected);
    assert_eq!(bandwidth_code(sim.engine(0).peer_capabilities()), 1);
    assert_eq!(bandwidth_code(sim.engine(1).peer_capabilities()), 1);
    // nothing moves while the session runs: it ends on the air it started on
    assert!(
        sim.engine_mut(1)
            .set_air(wide_timing.clone(), with_bandwidth(0, 2300), None)
            .is_err()
    );
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), b"a narrow call answered");
    assert_eq!(sim.engine(1).state(), State::Idle);
    // and back, once it is over
    let wide_top = wide_timing.mode_threshold_db.len() - 1;
    sim.engine_mut(1)
        .set_air(wide_timing.clone(), with_bandwidth(0, 2300), Some(wide_top))
        .expect("idle");
    assert_eq!(bandwidth_code(sim.engine(1).config().capabilities), 0);
    assert_eq!(
        sim.engine(1).timing().mode_threshold_db,
        wide_timing.mode_threshold_db
    );
    // a call or a probe under way holds the air too
    let (mut c, _) = pair(&wide_timing, &LinkConfig::default());
    c.connect("KK4ABC").expect("idle");
    assert!(
        c.set_air(narrow_timing.clone(), with_bandwidth(0, 500), None)
            .is_err()
    );
    let (mut p, _) = pair(&wide_timing, &LinkConfig::default());
    p.probe("KK4ABC", None).expect("idle");
    assert!(
        p.set_air(narrow_timing, with_bandwidth(0, 500), None)
            .is_err()
    );
}

#[test]
fn a_probe_across_bandwidths_is_answered_and_names_the_mismatch() {
    use aether_link::frames::with_bandwidth;
    // ADR-0035, from the air: a 500 Hz station and a 2 300 Hz one each heard the other's
    // probes and calls and ignored them "in another bandwidth", and each one's own probes
    // went unanswered. Probes and their answers are the same tone-floor frames on both
    // airs, so a probe is answered across bandwidths and the result names both; a call
    // across them is still ignored (a session lives in one bandwidth), and says which
    let narrow = LinkConfig {
        capabilities: with_bandwidth(0, 500),
        ..LinkConfig::default()
    };
    let wide = LinkConfig {
        capabilities: with_bandwidth(0, 2300),
        ..LinkConfig::default()
    };
    let a = LinkEngine::new("W4ODA", air_timing(NARROW_500, false), narrow, 1);
    let b = LinkEngine::new("KK4XYZ", timing(false), wide, 2);
    let mut sim = TwoStationSim::new(a, b, 3.0, 35);
    sim.engine_mut(0).probe("KK4XYZ", None).expect("idle");
    sim.run(60.0, 3.0);
    assert_eq!(
        sim.engine(1).stats.probes_answered,
        1,
        "{:?}",
        sim.events(1)
    );
    assert_eq!(sim.engine(0).stats.probe_replies, 1, "{:?}", sim.events(0));
    assert_eq!(
        sim.engine(0).last_probe().and_then(|p| p.bandwidth_hz),
        Some(2300)
    );
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e.starts_with("probe:KK4XYZ hears us at")
                && e.ends_with("— runs 2300 Hz, this station 500 Hz")),
        "{:?}",
        sim.events(0)
    );
    assert!(
        sim.events(1)
            .iter()
            .any(|e| e.starts_with("probed:W4ODA at")
                && e.ends_with("— runs 500 Hz, this station 2300 Hz")),
        "{:?}",
        sim.events(1)
    );
    // and the other way round
    sim.engine_mut(1).probe("W4ODA", None).expect("idle");
    sim.run(120.0, 3.0);
    assert_eq!(
        sim.engine(1).last_probe().and_then(|p| p.bandwidth_hz),
        Some(500),
        "{:?}",
        sim.events(1)
    );
    assert_eq!(sim.engine(0).stats.probes_answered, 1);
    // a call across them is still not a session, and says which bandwidths
    sim.engine_mut(1).connect("W4ODA").expect("idle");
    sim.run(200.0, 3.0);
    assert!(!sim.engine(0).connected() && !sim.engine(1).connected());
    assert!(
        sim.events(0).iter().any(
            |e| e.starts_with("ignored:KK4XYZ calls in another bandwidth")
                && e.ends_with("— runs 2300 Hz, this station 500 Hz")
        ),
        "{:?}",
        sim.events(0)
    );
}

#[test]
fn a_new_fastest_rung_reaches_the_link() {
    // `max_mode` is a live setting of the station's: a change reaches the link from the
    // next burst on, under the rules' ceiling as before
    let (mut a, _) = pair(&timing(false), &LinkConfig::default());
    assert_eq!(a.config().max_mode, LinkConfig::default().max_mode);
    a.set_max_mode(8);
    assert_eq!(a.config().max_mode, 8);
}

#[test]
fn a_probe_is_answered_with_the_snr_it_arrived_at() {
    // "can you hear me, and how well?" without a session: the probed station answers
    // with the SNR the probe arrived at, and the prober reports both directions
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.probe("KK4XYZ", None).expect("idle");
    assert_eq!(a.probe("KK4XYZ", None), Err("a probe is already out"));
    let mut sim = TwoStationSim::new(a, b, 15.0, 21);
    sim.run(60.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.events(1).iter().any(|e| e == "probed:W4ODA at 15.0 dB"),
        "{:?}",
        sim.events(1)
    );
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e == "probe:KK4XYZ hears us at 15 dB, heard at 15.0 dB"),
        "{:?}",
        sim.events(0)
    );
    assert_eq!(
        sim.engine(0).last_probe(),
        Some(&aether_link::ProbeResult {
            remote: "KK4XYZ".to_owned(),
            heard_there_db: Some(15.0),
            heard_here_db: 15.0,
            bandwidth_hz: Some(2300),
        })
    );
    assert!(!sim.engine(0).probing());
    let (sent, replies, answered) = (
        sim.engine(0).stats.probes_sent,
        sim.engine(0).stats.probe_replies,
        sim.engine(1).stats.probes_answered,
    );
    assert_eq!((sent, replies, answered), (1, 1, 1));
    // the question can be asked again, and a session can follow
    sim.engine_mut(0).probe("KK4XYZ", None).expect("answered");
    sim.run(120.0, 3.0);
    assert_eq!(sim.engine(0).stats.probe_replies, 2);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(b"after the probe");
    sim.engine_mut(0).disconnect();
    sim.run(400.0, 3.0);
    assert_eq!(sim.delivered(1), b"after the probe");
}

#[test]
fn a_probe_to_nobody_reports_no_answer() {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.probe("N0BODY", None).expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 22);
    sim.run(60.0, 3.0);
    assert!(
        sim.events(0).iter().any(|e| e == "probe:N0BODY: no answer"),
        "{:?}",
        sim.events(0)
    );
    assert!(!sim.events(1).iter().any(|e| e.starts_with("probed")));
    assert_eq!(sim.engine(0).state(), State::Idle);
    // and once it has timed out, another may go
    sim.engine_mut(0).probe("KK4XYZ", None).expect("free again");
    sim.run(120.0, 3.0);
    assert_eq!(sim.engine(0).stats.probe_replies, 1);
}

#[test]
fn a_probe_is_not_answered_during_a_session_but_is_across_bandwidths() {
    use aether_link::frames::{DataHeader, DataKind, ProbeBody, encode_data, with_bandwidth};
    use aether_link::{Action, Container, SimFrame};
    let t = timing(false);
    let probe_from = |call: &str, to: &str, caps: u8| {
        let body = ProbeBody {
            src: call.to_owned(),
            dst: to.to_owned(),
            snr_db: None,
            caps,
        }
        .encode()
        .expect("body");
        let header = DataHeader {
            kind: DataKind::Probe,
            seq: 0,
            session: 0,
        };
        let payload = encode_data(&header, &body, t.capacity(0)).expect("fits");
        SimFrame::decoded(Container::Data, 0, 12.0, 0.0, 1.0, payload)
    };
    let (mut a, b) = pair(&t, &LinkConfig::default());
    // a session up: a third station's probe is left alone
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 23);
    sim.run(40.0, 3.0);
    assert!(sim.engine(1).connected());
    let now = sim.t;
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "KK4XYZ", 0), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 0);
    let unexpected = sim.engine_mut(1).drain();
    assert!(unexpected.is_empty(), "{unexpected:?}");
    // idle, and the probe states another bandwidth: answered all the same, on the tone
    // floor both airs share, and the event names both bandwidths (ADR-0035)
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.engine(1).state(), State::Idle);
    let now = sim.t;
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "KK4XYZ", with_bandwidth(0, 500)), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 1);
    let floor = sim.engine(1).robust_mode(true);
    assert!(t.is_floor(floor));
    let actions = sim.engine_mut(1).drain();
    assert!(
        !actions.iter().any(|action| matches!(
            action,
            Action::Event { name: "ignored", detail } if detail.contains("N0CALL")
        )),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Event { name: "probed", detail }
                if detail.ends_with("— runs 500 Hz, this station 2300 Hz")
        )),
        "{actions:?}"
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Transmit { frames, .. } if frames.len() == 1 && frames[0].mode == floor
        )),
        "{actions:?}"
    );
    // and one for somebody else is nobody's business
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "W1AW", 0), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 1);
    let unexpected = sim.engine_mut(1).drain();
    assert!(unexpected.is_empty(), "{unexpected:?}");
    // while one addressed to it, in its bandwidth, is answered, with nothing to add
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "KK4XYZ", 0), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 2);
    let actions = sim.engine_mut(1).drain();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Transmit { .. }))
    );
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Event { name: "probed", detail } if detail == "N0CALL at 12.0 dB"
        )),
        "{actions:?}"
    );
    // a station in a session may not probe
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    assert_eq!(
        sim.engine_mut(0).probe("KK4XYZ", None),
        Err("already in a session")
    );
}

#[test]
fn the_rate_controller_steps_the_table_the_phy_hands_it() {
    use aether_link::rate::{
        NARROW_AWGN_THRESHOLD_DB, NARROW_FRAME_S, NARROW_PAYLOAD_BYTES, usable_modes_by_rate,
    };
    // a PHY with its own mode table — the 500 Hz waveform — hands its thresholds over, and
    // the engine recommends nothing outside that table
    let mut t = timing(false);
    t.data_capacity = NARROW_PAYLOAD_BYTES.to_vec();
    t.mode_threshold_db = NARROW_AWGN_THRESHOLD_DB.to_vec();
    t.floor_modes = 4;
    t.floor_data_frame_s = Some(NARROW_FRAME_S[0]);
    t.floor_control_frame_s = Some(2.232);
    let usable = usable_modes_by_rate(
        &NARROW_AWGN_THRESHOLD_DB,
        &NARROW_PAYLOAD_BYTES,
        &NARROW_FRAME_S,
    );
    // 8-PSK 2/3 (rung 8) carries what 16-QAM 1/2 (rung 9) does and needs more
    assert!(usable.contains(&0) && usable.contains(&14) && !usable.contains(&8));
    let config = LinkConfig {
        max_mode: 14,
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    a.connect("KK4XYZ").expect("idle");
    let message: Vec<u8> = (0..1500u32).map(|i| (i * 7 % 256) as u8).collect();
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 25.0, 12);
    sim.run(600.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    let highest = sim.modes_sent().iter().copied().max().unwrap_or(0);
    assert!(
        highest < NARROW_AWGN_THRESHOLD_DB.len(),
        "a mode outside the narrow table was sent: {highest}"
    );
    assert!(
        highest >= 10,
        "at 25 dB the controller climbs the narrow table: {highest}"
    );
}

#[test]
fn a_call_to_somebody_else_is_not_answered() {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4ABC").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.run(200.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(sim.events(0).iter().any(|e| e == "disconnected:no answer"));
    assert!(!sim.events(1).iter().any(|e| e.starts_with("connected")));
}

#[test]
fn a_station_calls_as_whichever_of_its_callsigns_the_host_named() {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.set_callsigns(&["W4ODA", "W4ODA-1"]).expect("idle");
    a.connect_as("KK4XYZ", Some("w4oda-1")).expect("idle");
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 15.0, 8);
    sim.run(200.0, 3.0);
    assert!(sim.events(1).iter().any(|e| e == "connected:W4ODA-1 (irs)"));
    let a = sim.engine_mut(0);
    assert!(a.connect_as("KK4XYZ", Some("N0CALL")).is_err());
    // without a choice, the first callsign is the station's name
    a.connect("KK4XYZ").expect("idle");
    assert_eq!(a.my_call, "W4ODA");
}

#[test]
fn callsigns_cannot_change_under_a_session() {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 9);
    sim.run(30.0, 3.0);
    let a = sim.engine_mut(0);
    assert!(a.connected());
    assert_eq!(a.set_callsigns(&["W4ODA-2"]), Err("a session is running"));
    assert_eq!(a.my_call, "W4ODA");
    let mut idle = LinkEngine::new("N0CALL", t.clone(), LinkConfig::default(), 3);
    assert!(idle.set_callsigns::<&str>(&[]).is_err());
    assert!(idle.set_callsigns(&["TOOLONGCALL"]).is_err());
    assert_eq!(idle.callsigns, vec!["N0CALL"]);
}

#[test]
fn the_peer_sees_a_disconnect() {
    let sim = run_transfer(b"short message", 15.0, 4);
    assert!(
        sim.events(1).iter().any(|e| e.starts_with("disconnected")),
        "{:?}",
        sim.events(1)
    );
    assert_eq!(sim.engine(1).state(), State::Idle);
}

#[test]
fn a_transfer_survives_a_slow_fade() {
    // the channel fades from +18 dB down to +2 and back, twice over the transfer; the rate
    // controller has to follow it in both directions without losing the session or the data
    let schedule = |t: f64| {
        let phase = t % 60.0;
        if phase < 30.0 {
            0.533f64.mul_add(-phase, 18.0)
        } else {
            0.533f64.mul_add(phase - 30.0, 2.0)
        }
    };
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    let message: Vec<u8> = (0..12000u32).map(|i| ((i * 17) % 256) as u8).collect();
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 18.0, 13).with_snr_schedule(Box::new(schedule));

    // sample the sender's mode as the run proceeds, rather than instrumenting the engine,
    // until both stations are idle again — not until a step brings nothing: a call on the
    // tone floor (ADR-0016) is on the air longer than a step
    let mut track: Vec<usize> = Vec::new();
    let mut until = 0.0;
    while until < 3000.0 {
        until += 5.0;
        sim.run(until, 3.0);
        track.push(sim.engine(0).current_mode());
        if sim.engine(0).state() == State::Idle && sim.engine(1).state() == State::Idle {
            break;
        }
    }

    assert_eq!(sim.delivered(1), message.as_slice());
    let mut distinct: Vec<usize> = track.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(distinct.len() >= 4, "mode barely moved: {track:?}");
    assert!(
        track.iter().copied().max().unwrap_or(0) >= 8,
        "never exploited the good half of the fade: {track:?}"
    );

    // drop plateaus, then count direction changes: it followed the channel down and back up
    let mut steps: Vec<usize> = Vec::new();
    for &mode in &track {
        if steps.last() != Some(&mode) {
            steps.push(mode);
        }
    }
    let deltas: Vec<isize> = steps
        .windows(2)
        .map(|w| w[1] as isize - w[0] as isize)
        .collect();
    let reversals = deltas.windows(2).filter(|w| w[0] * w[1] < 0).count();
    assert!(reversals >= 2, "never reversed direction: {steps:?}");
}

#[test]
fn adaptation_pays_for_itself() {
    // on a good channel the controller must finish far sooner than a link pinned to the most
    // robust mode; otherwise all the machinery is only a way to lose frames
    let message = vec![0u8; 6000];
    let mut times = Vec::new();
    for config in [
        LinkConfig::default(),
        LinkConfig {
            max_mode: 0,
            ..LinkConfig::default()
        },
    ] {
        let t = timing(false);
        let (mut a, b) = pair(&t, &config);
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        a.disconnect();
        let mut sim = TwoStationSim::new(a, b, 16.0, 21);
        times.push(sim.run(6000.0, 3.0));
        assert_eq!(sim.delivered(1), message.as_slice());
    }
    assert!(
        times[0] < 0.25 * times[1],
        "adaptive {:.1} s vs pinned {:.1} s",
        times[0],
        times[1]
    );
}

#[test]
fn a_start_of_frame_signal_raises_throughput() {
    // P2-2a: told when a frame *starts*, the receiver no longer has to wait a whole frame of
    // silence to know a burst has ended, which is about a quarter of the air time. Measured on
    // frames without the burst countdown, which since ADR-0045 ends the wait by itself
    let message = vec![0u8; 16000];
    let mut times = Vec::new();
    for start_of_frame in [false, true] {
        let t = timing(start_of_frame);
        let (mut a, b) = pair(&t, &LinkConfig::default());
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        a.disconnect();
        let mut sim = TwoStationSim::new(a, b, 14.0, 5).without_countdown();
        times.push(sim.run(6000.0, 3.0));
        assert_eq!(sim.delivered(1), message.as_slice());
    }
    assert!(
        times[1] < 0.92 * times[0],
        "with start-of-frame {:.1} s, without {:.1} s",
        times[1],
        times[0]
    );
}

#[test]
fn a_second_connect_is_refused_while_a_session_is_up() {
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 3);
    sim.run(30.0, 3.0);
    assert!(sim.engine(0).connected());
    assert!(sim.engine_mut(0).connect("M0ABC").is_err());
}

#[test]
fn an_abort_drops_the_session_locally_and_the_peer_follows() {
    // `abort` is the impatient close: it sends one disconnect and stops, so the peer may
    // still be transmitting and miss it. The session must end at both ends either way —
    // immediately for the station that aborted, and at the link timeout for the other.
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.run(30.0, 3.0);
    assert!(sim.engine(0).connected() && sim.engine(1).connected());

    sim.engine_mut(0).abort();
    assert_eq!(sim.engine(0).state(), State::Idle);
    let timeout = sim.engine(1).config().link_timeout_s;
    sim.run(sim.t + timeout + 30.0, 3.0);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.events(1).iter().any(|e| e.starts_with("disconnected")),
        "{:?}",
        sim.events(1)
    );
}

/// A transfer with real transport latency, the sender told (or not) about its own.
fn latent_transfer(prop_s: f64, tx_latency_s: f64) -> (bool, usize) {
    let mut t = timing(false);
    t.tx_latency_s = tx_latency_s;
    let (mut a, b) = pair(&t, &LinkConfig::default());
    let message: Vec<u8> = "The quick brown fox jumps over the lazy dog. "
        .repeat(10)
        .into_bytes();
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 20.0, 3).with_propagation(prop_s);
    sim.run(30.0, 3.0);
    sim.engine_mut(0).send(&message); // once connected, as a host does
    sim.run(400.0, 3.0);
    (
        sim.delivered(1) == message.as_slice(),
        sim.engine(0).stats.ack_timeouts,
    )
}

#[test]
fn a_sender_that_knows_its_own_latency_waits_long_enough() {
    // Two real daemons over a socket stalled on their first session: each burst left the
    // sound card a quarter of a second after the engine handed it over, so every reply was
    // waited for from too early a moment, and the acknowledgement of the first poll arrived
    // just after the engine had given up on it. Told what the daemon knows about itself,
    // half a second of one-way latency costs no timeouts; blind, the same link limps.
    let (delivered, timeouts) = latent_transfer(0.45, 0.4);
    assert!(delivered && timeouts == 0, "timeouts: {timeouts}");
    let (delivered_blind, timeouts_blind) = latent_transfer(0.45, 0.0);
    assert!(delivered_blind, "the blind link did not even limp through");
    assert!(timeouts_blind > 10, "timeouts: {timeouts_blind}");
}

/// A frame handed straight from one engine to another: a perfect channel, hand-timed.
struct Wire {
    container: aether_link::Container,
    mode: usize,
    rv: u8,
    t_start: f64,
    t_end: f64,
    payload: Vec<u8>,
    floor: bool,
}

impl Wire {
    fn carry(frame: &aether_link::TxFrame, t_start: f64, t: &PhyTiming) -> Self {
        let duration = t.frame_s(frame);
        Self {
            container: frame.container,
            mode: frame.mode,
            rv: frame.rv,
            t_start,
            t_end: t_start + duration,
            payload: frame.payload.clone(),
            floor: match frame.container {
                aether_link::Container::Data => t.is_floor(frame.mode),
                aether_link::Container::Control => frame.floor,
            },
        }
    }
}

impl aether_link::SoftFrame for Wire {
    fn container(&self) -> aether_link::Container {
        self.container
    }
    fn mode(&self) -> usize {
        self.mode
    }
    fn floor(&self) -> bool {
        self.floor
    }
    fn rv(&self) -> u8 {
        self.rv
    }
    fn snr_db(&self) -> f64 {
        20.0
    }
    fn t_start(&self) -> f64 {
        self.t_start
    }
    fn t_end(&self) -> f64 {
        self.t_end
    }
    fn decode(
        &self,
        _buffer: Option<&aether_link::HarqBuffer>,
    ) -> (Option<Vec<u8>>, aether_link::HarqBuffer) {
        (
            Some(self.payload.clone()),
            aether_link::HarqBuffer::default(),
        )
    }
}

/// The frames an engine wants sent, from its actions.
fn transmitted(engine: &mut LinkEngine) -> Vec<aether_link::TxFrame> {
    engine
        .drain()
        .into_iter()
        .filter_map(|action| match action {
            aether_link::Action::Transmit { frames, .. } => Some(frames),
            aether_link::Action::Event { .. } | aether_link::Action::Deliver(_) => None,
        })
        .flatten()
        .collect()
}

#[test]
fn a_burst_held_back_by_a_busy_channel_moves_the_timers_with_it() {
    // on the air, connect requests went out in pairs inside one keying: the busy detector
    // held the first back, the retry timer fired meanwhile, and both left together
    let t = timing(false);
    let mut a = LinkEngine::new("W4ODA", t.clone(), LinkConfig::default(), 1);
    a.connect("KK4XYZ").expect("idle");
    assert_eq!(transmitted(&mut a).len(), 1);
    let mut now = 0.0;
    while now < 8.0 {
        now += 0.02;
        a.on_tx_delayed(0.02);
        a.tick(now);
    }
    assert!(
        transmitted(&mut a).is_empty(),
        "a retry was queued while the first was held back"
    );
    assert_eq!(a.state(), State::Connecting);
    a.on_tx_done(now + t.tx_latency_s + t.data_frame_s);
    let mut retry_at = None;
    while now < 30.0 {
        now += 0.02;
        a.tick(now);
        if !transmitted(&mut a).is_empty() {
            retry_at = Some(now);
            break;
        }
    }
    let retry_at = retry_at.expect("no retry ever came");
    assert!(
        retry_at - 8.0 > 2.0 * t.data_frame_s,
        "the retry came too soon after departure: {:.2} s",
        retry_at - 8.0
    );
}

#[test]
fn an_ack_that_arrives_during_a_repoll_is_acted_on_when_the_poll_ends() {
    // The other half of the same stall, timed by hand: the acknowledgement of the
    // post-connect poll arrives just after the sender gave up on it and re-polled. It is
    // accepted while the re-poll is on the air, and the burst it should start is refused
    // because the transmitter is busy. Before the fix nothing tried again, and the *next*
    // acknowledgement was ignored because nothing was being waited for: a dead session
    // with data queued. `on_tx_done` now tries.
    let t = timing(false);
    let (mut a, mut b) = pair(&t, &LinkConfig::default());

    // the handshake, over a perfect wire
    a.connect("KK4XYZ").expect("idle");
    let request = transmitted(&mut a);
    assert_eq!(request.len(), 1);
    b.on_frame(&Wire::carry(&request[0], 0.0, &t), t.data_frame_s);
    let accept = transmitted(&mut b);
    assert_eq!(accept.len(), 1);
    let now = 2.0 * t.data_frame_s;
    a.on_tx_done(now);
    a.on_frame(&Wire::carry(&accept[0], t.data_frame_s, &t), now);
    assert!(a.connected());
    let poll = transmitted(&mut a);
    assert_eq!(
        poll.len(),
        1,
        "the sender confirms the handshake with a poll"
    );
    a.on_tx_done(now + t.control_frame_s);

    // data arrives from the host, as it does the moment a host sees "connected"
    a.send(b"queued while the poll was unanswered");
    assert!(
        transmitted(&mut a).is_empty(),
        "nothing goes out while a reply is awaited"
    );

    // the reply is late: the sender gives up and polls again
    let mut late = now + t.control_frame_s;
    let repoll = loop {
        late += 0.1;
        a.tick(late);
        let frames = transmitted(&mut a);
        if !frames.is_empty() {
            break frames;
        }
        assert!(late < now + 10.0, "the sender never re-polled");
    };
    assert_eq!(repoll[0].container, aether_link::Container::Control);

    // ...and while the re-poll is on the air, the first poll's acknowledgement lands
    b.on_frame(&Wire::carry(&poll[0], now, &t), late);
    // the receiver answers after its turnaround, on its own clock
    b.tick(late + t.control_frame_s + t.turnaround_s + 0.1);
    let acks = transmitted(&mut b);
    assert_eq!(acks.len(), 1, "the receiver acknowledges the poll");
    a.on_frame(&Wire::carry(&acks[0], late, &t), late + 0.05);
    assert!(
        transmitted(&mut a).is_empty(),
        "a burst cannot start while the re-poll is on the air"
    );

    // the re-poll finishes: the queued data has to go out now, not never
    a.on_tx_done(late + t.control_frame_s);
    let burst = transmitted(&mut a);
    assert!(
        burst
            .iter()
            .any(|f| f.container == aether_link::Container::Data),
        "the queued data never went out: {burst:?}"
    );
}

#[test]
fn a_pinned_mode_goes_out_whatever_the_peer_recommends() {
    // P6-7's ladder: while a mode is pinned every new frame goes out at it, the peer's
    // recommendation notwithstanding, and each pinned burst leaves a rung saying how
    // many of its frames the peer acknowledged at what SNR
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    assert_eq!(a.pin_mode(Some(99), None), Err("not a mode of the table"));
    assert_eq!(
        a.pin_mode(Some(2), Some(0)),
        Err("body_bytes must be at least 1")
    );
    a.pin_mode(Some(2), Some(16)).expect("a mode of the table");
    let message: Vec<u8> = (0..200u8).collect();
    let mut sim = TwoStationSim::new(a, b, 15.0, 31);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(&message);
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), &message[..]);
    // connect frames go at a robust mode — the floor's, or the ordinary family's (rung 6,
    // BPSK 1/5); every data frame went at the pin, a fast tone kind (ADR-0014)
    let robust = air_interface(WIDE_2300).control_rung();
    let modes = sim.modes_sent();
    assert!(
        modes.iter().all(|&m| m == 0 || m == robust || m == 2),
        "{modes:?}"
    );
    assert!(modes.iter().filter(|&&m| m == 2).count() >= 13, "{modes:?}");
    // 16-byte bodies: 200 bytes are 13 frames, six to a burst
    let rungs = sim.engine_mut(0).take_ladder();
    let frames: Vec<usize> = rungs.iter().map(|r| r.frames).collect();
    assert_eq!(frames, vec![6, 6, 1], "{rungs:?}");
    assert!(
        rungs.iter().all(|r| r.mode == 2 && r.decoded == r.frames),
        "{rungs:?}"
    );
    assert!(
        rungs
            .iter()
            .all(|r| r.snr_db.is_some_and(|s| (s - 15.0).abs() < 1.0)),
        "{rungs:?}"
    );
    let unexpected = sim.engine_mut(0).take_ladder();
    assert!(unexpected.is_empty(), "{unexpected:?}");
    assert!(sim.engine(0).all_acknowledged());
}

#[test]
fn a_stranded_frame_is_re_encoded_at_a_mode_that_carries_it() {
    // a frame that has gone max_combines transmissions unacknowledged at a mode the
    // channel cannot carry is given another codeword at the slowest mode down to the
    // recommendation that fits its body — so a ladder rung above the channel, or a link
    // that drops into the floor, does not strand the session
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    let top = t.data_capacity.len() - 1;
    a.pin_mode(Some(top), Some(16)).expect("the top mode");
    let message: Vec<u8> = (0..48u8).collect();
    let mut sim = TwoStationSim::new(a, b, 0.0, 7);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(&message);
    sim.engine_mut(0).disconnect();
    let ended = sim.run(600.0, 3.0);
    assert_eq!(sim.delivered(1), &message[..]);
    assert!(ended < 600.0, "the session did not end: {ended}");
    let rungs = sim.engine_mut(0).take_ladder();
    assert!(
        rungs
            .first()
            .is_some_and(|r| r.mode == top && r.decoded == 0),
        "{rungs:?}"
    );
    assert!(sim.engine(0).stats.frames_reencoded >= 1);
}

#[test]
fn a_station_that_only_received_learns_how_it_was_heard() {
    // ADR-0021: a station says how it hears the other in its acknowledgements, and the one
    // that only receives is never acknowledged — ND1J called and sent, and KK4ODA-1's history
    // could not say how it was heard. Every control frame now carries it, and the disconnect
    // brings it to the station that only received; kept, once the session has ended, for the
    // account written after it
    let t = timing(false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let mut sim = TwoStationSim::new(a, b, 15.0, 9);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(&[0x5A; 400]);
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert_eq!(
        sim.engine(1).peer_snr_db(),
        None,
        "the session's own report goes with it"
    );
    let heard = sim
        .engine(1)
        .ended_peer_snr_db()
        .expect("the disconnect said how it heard this station");
    assert!((heard - 15.0).abs() < 3.0, "{heard}");
}

#[test]
fn the_tests_ladder_does_not_teach_the_receiver_a_margin() {
    // ADR-0020: a session settles, the sender pins the fastest rung for a while — far past
    // the path, every frame failing until it is re-encoded — and unpins; the receiving
    // station's learned margin is no wider for it (on ND1J's test it had been held at its
    // ceiling for the whole file that followed)
    let t = timing(false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let top = t.data_capacity.len() - 1;
    let mut sim = TwoStationSim::new(a, b, 6.0, 11);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(&[0u8; 400]);
    sim.run(120.0, 1e9);
    assert_eq!(sim.engine(1).state(), State::Connected);
    let (_, margin) = sim.engine(1).rate_readings();
    sim.engine_mut(0)
        .pin_mode(Some(top), Some(16))
        .expect("the top mode");
    sim.engine_mut(0).send(&[0u8; 64]);
    sim.run(360.0, 1e9);
    sim.engine_mut(0).pin_mode(None, None).expect("unpinned");
    let rungs = sim.engine_mut(0).take_ladder();
    assert!(
        rungs.iter().any(|r| r.mode == top && r.decoded == 0),
        "{rungs:?}"
    );
    let (_, after) = sim.engine(1).rate_readings();
    assert!(after <= margin + 1e-9, "{margin} -> {after}");
}

#[test]
fn a_narrow_session_at_the_floor_completes_through_the_pipe() {
    use aether_link::sim::control_thresholds_for;
    // at −8 dB on AWGN only the floor decodes — the tone floor since ADR-0013: its data
    // rungs and its control frame. The pipe delivers each frame's family and judges a
    // control frame at its own family's threshold — at data mode 0's, as it did before P9-6,
    // an ordinary acknowledgement went through eight decibels below where the modem decodes
    // it
    let t = air_timing(NARROW_500, true);
    let [ordinary, floor] = control_thresholds_for(&t);
    assert!(ordinary > -8.0 && -8.0 > floor, "{ordinary} {floor}");
    let config = LinkConfig {
        max_mode: 12,
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    a.connect("KK4XYZ").expect("idle");
    let message: Vec<u8> = (0..60u8).collect();
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, -8.0, 3);
    sim.run(900.0, 3.0);
    assert_eq!(
        sim.delivered(1),
        message.as_slice(),
        "{:?} {:?}",
        sim.events(0),
        sim.events(1)
    );
    assert!(sim.engine(1).stats.frames_received > 0);
}

#[test]
fn a_call_starts_on_the_tone_floor_and_alternates() {
    // ADR-0016: a call is made before anything is known of the path, so it goes where the
    // path most likely carries it — the tone floor, 14 dB below the ordinary family's control
    // rung — on its first try and every other one after, the ordinary family between
    let t = timing(false);
    let config = LinkConfig {
        connect_retries: 4,
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    a.connect("N0BODY").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 24);
    sim.run(900.0, 3.0);
    assert!(
        sim.events(0).iter().any(|e| e == "disconnected:no answer"),
        "{:?}",
        sim.events(0)
    );
    let robust = air_interface(WIDE_2300).control_rung();
    assert_eq!(sim.modes_sent(), &[0, robust, 0, robust]);
}

#[test]
fn a_probe_goes_out_on_the_floor_and_is_answered_in_its_family() {
    use aether_link::frames::{DataHeader, DataKind, ProbeBody, encode_data};
    // ADR-0016: a probe exists to measure a weak path, so it goes out on the tone floor, and
    // it is answered in the family it arrived in — a station of an earlier version probes in
    // the ordinary family and hears its answer there
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.probe("KK4XYZ", None).expect("idle");
    let mut sim = TwoStationSim::new(a, b, -14.0, 26);
    sim.run(120.0, 3.0);
    assert_eq!(sim.engine(0).stats.probe_replies, 1, "{:?}", sim.events(0));
    assert_eq!(sim.modes_sent(), &[0, 0]);
    let robust = air_interface(WIDE_2300).control_rung();
    let mut b = LinkEngine::new("KK4XYZ", t.clone(), LinkConfig::default(), 2);
    let body = ProbeBody {
        src: "N0CALL".into(),
        dst: "KK4XYZ".into(),
        snr_db: None,
        caps: 0,
    }
    .encode()
    .expect("body");
    let header = DataHeader {
        kind: DataKind::Probe,
        seq: 0,
        session: 0,
    };
    let payload = encode_data(&header, &body, t.capacity(robust)).expect("fits");
    b.on_frame(
        &Handed {
            payload,
            mode: robust,
        },
        1.0,
    );
    let modes: Vec<usize> = b
        .drain()
        .into_iter()
        .filter_map(|action| match action {
            aether_link::Action::Transmit { frames, .. } => Some(frames[0].mode),
            _ => None,
        })
        .collect();
    assert_eq!(modes, vec![robust]);
}

#[test]
fn a_strong_path_called_on_the_floor_climbs_from_its_first_ordinary_burst() {
    // ADR-0016: the tone floor's SNR estimate reads a few decibels on a strong dispersive
    // path whatever the SNR. The session's first burst goes out where that reading puts it;
    // the called station's controller, seeded from it, starts again from the first clean
    // burst it measures on an ordinary frame — so the second burst goes out where an ordinary
    // connect frame would have started the session, not two rungs a burst up
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    // a controller as the engine builds one for this air, to say where a measurement puts it
    let frame_s: Vec<f64> = (0..t.mode_threshold_db.len())
        .map(|m| t.data_frame_s_for(m))
        .collect();
    let fresh = RateController::for_table_timed(
        RateConfig::default(),
        &t.mode_threshold_db,
        &t.data_capacity,
        &frame_s,
    )
    .with_floor(t.floor_modes, t.floor_margin_db);
    let message = vec![0u8; 20000];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 20.0, 41).with_floor_reading_cap(4.0);
    sim.run(900.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    // the caller's bursts: past the connect frames, on the floor's first rung, every frame
    // of a burst goes at the burst's mode
    let bursts: Vec<usize> = sim
        .modes_sent()
        .iter()
        .copied()
        .skip_while(|&m| m == 0)
        .collect::<Vec<_>>()
        .chunk_by(|x, y| x == y)
        .map(|run| run[0])
        .collect();
    assert_eq!(bursts[0], fresh.first_mode(4.0), "{bursts:?}");
    assert_eq!(bursts[1], fresh.first_mode(20.0), "{bursts:?}");
    assert!(
        bursts[1] > bursts[0] + RateConfig::default().max_up_step,
        "{bursts:?}"
    );
}

/// A pipe's `unheard`: the caller (station 0) never detects the first DATA frame sent to it —
/// the called station's first acceptance.
fn first_acceptance_unheard() -> aether_link::sim::Unheard {
    let lost = std::cell::Cell::new(None::<f64>);
    Box::new(move |rx, container, t0| {
        if rx != 0 || container != Container::Data {
            return false;
        }
        let first = lost.get().unwrap_or(t0);
        lost.set(Some(first));
        (t0 - first).abs() < 1e-9
    })
}

#[test]
fn a_repeated_acceptance_says_how_its_request_arrived() {
    // A called station whose acceptance was lost answers the caller's next try with the
    // acceptance again, carrying the SNR that try arrived at, as the first carried the first's:
    // a caller that hears only the repeat starts its first burst where that measurement puts
    // it (P9-2). The repeat said "not measured", and the session started on the ladder's first
    // rung, the tone floor, whatever the path
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    let frame_s: Vec<f64> = (0..t.mode_threshold_db.len())
        .map(|m| t.data_frame_s_for(m))
        .collect();
    let fresh = RateController::for_table_timed(
        RateConfig::default(),
        &t.mode_threshold_db,
        &t.data_capacity,
        &frame_s,
    )
    .with_floor(t.floor_modes, t.floor_margin_db);
    assert!(fresh.first_mode(18.0) > fresh.first_mode(3.0));
    let message = vec![0u8; 600];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    // the first try arrives in a fade, the second on a strong path
    let mut sim = TwoStationSim::new(a, b, 18.0, 31)
        .with_snr_schedule(Box::new(|t| if t < 10.0 { 3.0 } else { 18.0 }))
        .with_unheard(first_acceptance_unheard());
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    // a call on the floor and its acceptance, lost; the ordinary try and the acceptance again;
    // then the caller's first burst
    let robust = air_interface(WIDE_2300).control_rung();
    let modes = sim.modes_sent();
    assert_eq!(&modes[..4], &[0, 0, robust, robust], "{modes:?}");
    assert_eq!(modes[4], fresh.first_mode(18.0), "{modes:?}");
}

/// WC4Y's call of 2026-10-05 (ADR-0048): the caller hears none of the called station's DATA
/// frames — its acceptances — and every try after the first arrives unreadable, so the called
/// station, connected, answers them with acknowledgements of the session.
fn acceptances_lost(capabilities: u8, message: &[u8]) -> TwoStationSim {
    use aether_link::frames::decode_data;
    let t = timing(false);
    let config = LinkConfig {
        capabilities,
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    a.connect("KK4XYZ").expect("idle");
    a.send(message);
    a.disconnect();
    let requests = std::cell::Cell::new(0usize);
    TwoStationSim::new(a, b, 15.0, 31)
        .with_unheard(Box::new(|rx, container, _| {
            rx == 0 && container == Container::Data
        }))
        .with_frame_snr_offset(Box::new(move |frame| {
            let request = frame.container == Container::Data
                && decode_data(&frame.payload)
                    .is_ok_and(|(header, _)| header.kind == aether_link::DataKind::ConnectReq);
            if !request {
                return 0.0;
            }
            requests.set(requests.get() + 1);
            if requests.get() == 1 { 0.0 } else { -40.0 }
        }))
}

#[test]
fn an_acknowledgement_of_the_callers_session_is_its_acceptance() {
    // A caller that reads an acknowledgement of its own session has been accepted: only a
    // station that accepted the request takes its session number. WC4Y read one while all
    // four of KK4ODA-1's acceptances were lost, kept calling, and gave up (ADR-0048). It
    // offered nothing beyond its bandwidth, so the acceptance it missed held nothing it did
    // not know
    use aether_link::frames::with_bandwidth;
    let message: Vec<u8> = (0..=255u8).chain(0..=255u8).collect();
    let mut sim = acceptances_lost(with_bandwidth(0, 2300), &message);
    sim.run(400.0, 3.0);
    let caller = sim.engine(0);
    assert_eq!(caller.stats.acceptances_inferred, 1, "{:?}", caller.stats);
    assert_eq!(caller.peer_capabilities(), with_bandwidth(0, 2300));
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e.starts_with("accepted:KK4XYZ")),
        "{:?}",
        sim.events(0)
    );
    assert_eq!(sim.delivered(1), message.as_slice());
}

#[test]
fn a_caller_that_offered_compression_waits_for_the_acceptance() {
    // Compression is used only when both offer it, and only the acceptance says whether the
    // other did: a caller that offered it does not take an acknowledgement for the acceptance
    use aether_link::frames::{CAP_COMPRESSION, with_bandwidth};
    let mut sim = acceptances_lost(with_bandwidth(CAP_COMPRESSION, 2300), &[0u8; 100]);
    sim.run(400.0, 3.0);
    let caller = sim.engine(0);
    assert_eq!(caller.stats.acceptances_inferred, 0, "{:?}", caller.stats);
    assert_eq!(sim.delivered(1), b"");
}

#[test]
fn a_request_heard_again_is_answered_and_nothing_else() {
    // The acceptance again is the whole answer to a request heard again. The request had also
    // armed an acknowledgement, which went out a burst's quiet after it — over the caller's
    // first burst, which follows the acceptance at once — and here the request's own record,
    // left in the burst, made that acknowledgement a clean burst to the rate controller
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    let message = vec![0u8; 600];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 15.0, 31).with_unheard(first_acceptance_unheard());
    sim.run(300.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    // one burst, one acknowledgement, and nothing of the burst sent twice
    let (caller, called) = (&sim.engine(0).stats, &sim.engine(1).stats);
    assert_eq!(called.acks_sent, 1, "{called:?}");
    assert_eq!(caller.frames_resent, 0, "{caller:?}");
}

#[test]
fn a_burst_fits_the_transmitters_key_time() {
    // ADR-0017: six tone frames are 32 s, and the daemon's 30 s key watchdog cut the last one
    // of every full tone burst on the air; the receiver acquired the cut frame and could not
    // decode it, and the link stepped down the tone floor where every burst was cut again
    let t = timing(false);
    let tone = t.data_frame_s_for(0);
    let ofdm = t.data_frame_s_for(t.floor_modes);
    assert!(
        6.0 * tone > 30.0 && 30.0 > 5.0 * tone,
        "the case this exists for"
    );
    let capped = LinkEngine::new(
        "W4ODA",
        t.clone(),
        LinkConfig {
            max_burst_s: Some(29.0),
            ..LinkConfig::default()
        },
        1,
    );
    assert_eq!(capped.burst_capacity(tone), 5);
    assert_eq!(
        capped.burst_capacity(ofdm),
        6,
        "a second a frame: six fit either way"
    );
    let session = |max_burst_s: Option<f64>| {
        let config = LinkConfig {
            max_burst_s,
            ..LinkConfig::default()
        };
        let (mut a, b) = pair(&t, &config);
        let message = vec![0u8; 2000];
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        a.disconnect();
        // -4 dB: the fast tones' range, and a transmitter that unkeys at 30 s
        let mut sim = TwoStationSim::new(a, b, -4.0, 3).with_key_limit(30.0);
        let took = sim.run(4000.0, 3.0);
        assert_eq!(sim.delivered(1), message.as_slice());
        (took, sim.engine(0).stats.frames_resent)
    };
    let (cut_took, cut_resent) = session(None);
    let (fit_took, fit_resent) = session(Some(29.0));
    assert_eq!(
        fit_resent, 0,
        "a channel that carries the rung resends nothing"
    );
    assert!(
        cut_resent >= 10,
        "every full burst lost a frame: {cut_resent}"
    );
    assert!(fit_took < 0.5 * cut_took, "{fit_took} {cut_took}");
}

#[test]
fn a_frame_re_encoded_and_left_out_of_its_burst_goes_in_the_next() {
    // Six frames stranded at an OFDM rung are re-encoded on the tone floor together, and a
    // burst held to the key time carries five tone frames (ADR-0017). The sixth, its count of
    // transmissions reset with its new codeword, fell out of the unacknowledged frames: never
    // sent again, never acknowledged — the other station waited for it for ever, the window
    // filled behind it, and the sender went silent with data queued until the link timed out.
    // Every session the chat bench dropped at 0 dB had left a frame so (ADR-0031). Here a path
    // that never carries the rung: the message still arrives whole
    let t = air_timing(NARROW_500, true);
    let rung = 4; // QPSK 1/3 on the ordinary frame, the first OFDM rung at 500 Hz
    assert!(
        t.is_floor(rung - 1) && !t.is_floor(rung),
        "not the case measured"
    );
    let tone = t.data_frame_s_for(0);
    assert!(5.0 * tone <= 28.85 && 28.85 < 6.0 * tone);
    let config = LinkConfig {
        max_mode: rung,
        max_burst_s: Some(28.85), // the daemon's limit at 30 s of key
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    let mut thresholds = NARROW_AWGN_THRESHOLD_DB;
    thresholds[rung] = 99.0;
    let length = 6 * aether_link::frames::data_capacity(t.data_capacity[rung]);
    let message: Vec<u8> = (0..=u8::MAX).take(length).collect();
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    a.disconnect();
    let mut sim = TwoStationSim::new(a, b, 0.0, 4).with_thresholds(&thresholds);
    sim.run(900.0, 3.0);
    let at_rung = sim
        .frames_sent(0)
        .iter()
        .filter(|f| f.container == Container::Data && f.mode == rung)
        .count();
    let stranded = LinkConfig::default().max_combines;
    assert_eq!(at_rung, 6 * stranded, "not the case measured");
    assert_eq!(sim.engine(0).stats.frames_reencoded, 6);
    assert_eq!(
        sim.delivered(1),
        message.as_slice(),
        "{:?} {:?}",
        sim.events(0),
        sim.events(1)
    );
    assert!(sim.engine(0).all_acknowledged());
}

#[test]
fn what_has_reached_the_other_station_is_counted_in_order() {
    // the panel's check mark on a sent message: every byte of it and every byte before it
    // acknowledged. A frame acknowledged past a hole still waits for the hole, which is
    // where this differs from the bytes still pending
    let t = timing(false);
    let message = vec![0u8; 3000];
    let mut holes = 0;
    for seed in 1..=6 {
        let (mut a, b) = pair(&t, &LinkConfig::default());
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        assert_eq!(a.tx_undelivered_bytes(), 3000);
        // 0 dB: lossy enough that frames are acknowledged past holes
        let mut sim = TwoStationSim::new(a, b, 0.0, seed);
        let mut arrived_before = 0;
        let mut now = 0.0;
        while now < 3000.0 {
            now += 0.5;
            sim.run(now, 1e9);
            let a = sim.engine(0);
            let (pending, undelivered) = (a.tx_pending_bytes(), a.tx_undelivered_bytes());
            let arrived = message.len() - undelivered;
            assert!(arrived >= arrived_before, "seed {seed}: it only grows");
            assert!(
                arrived <= sim.delivered(1).len(),
                "seed {seed}: never ahead of what the other station has"
            );
            assert!(undelivered >= pending, "seed {seed}");
            holes += usize::from(undelivered > pending);
            arrived_before = arrived;
            if undelivered == 0 {
                break;
            }
        }
        assert_eq!(sim.delivered(1), message.as_slice(), "seed {seed}");
        assert_eq!(sim.engine(0).tx_undelivered_bytes(), 0, "seed {seed}");
    }
    assert!(holes > 0, "a frame acknowledged past a hole waits for it");
}

#[test]
fn a_regulatory_ceiling_holds_every_frame_the_station_sends() {
    use aether_link::Container;
    // ADR-0018: the rules outrank link adaptation. A station whose ceiling admits only the
    // tone floor sends every data frame at a floor rung and every control frame, answer and
    // probe answer on the floor, however good the path; the other station climbs as before
    let t = timing(false);
    let floor_top = t.floor_modes - 1;
    let ceiling = 1;
    assert!(t.is_floor(ceiling) && ceiling < floor_top);
    let (mut a, mut b) = pair(&t, &LinkConfig::default());
    b.set_ceiling(Some(ceiling));
    let message = vec![0u8; 3000];
    a.connect("KK4XYZ").expect("idle");
    a.send(&message);
    b.send(&message);
    let mut sim = TwoStationSim::new(a, b, 24.0, 31);
    sim.run(3000.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    assert_eq!(sim.delivered(0), message.as_slice());
    let data = |who: usize| -> Vec<usize> {
        sim.frames_sent(who)
            .iter()
            .filter(|f| f.container == Container::Data)
            .map(|f| f.mode)
            .collect()
    };
    let b_data = data(1);
    assert!(
        !b_data.is_empty() && b_data.iter().all(|&m| m <= ceiling),
        "{b_data:?}"
    );
    let b_control: Vec<bool> = sim
        .frames_sent(1)
        .iter()
        .filter(|f| f.container == Container::Control)
        .map(|f| f.floor)
        .collect();
    assert!(
        !b_control.is_empty() && b_control.iter().all(|&f| f),
        "all on the floor"
    );
    assert!(
        data(0).iter().any(|&m| m > floor_top),
        "the path was good: the other station climbed"
    );

    // a ceiling set mid-session holds from the next frame on
    let before = sim.frames_sent(0).len();
    sim.engine_mut(0).set_ceiling(Some(ceiling));
    sim.engine_mut(0).send(&[1u8; 1500]);
    sim.run(6000.0, 3.0);
    let later = &sim.frames_sent(0)[before..];
    assert!(!later.is_empty(), "later is empty");
    assert!(
        later
            .iter()
            .all(|f| (f.container == Container::Data && f.mode <= ceiling)
                || (f.container == Container::Control && f.floor))
    );
}

/// A frame handed straight to an engine: a payload that always decodes.
struct Handed {
    payload: Vec<u8>,
    mode: usize,
}

impl aether_link::SoftFrame for Handed {
    fn container(&self) -> aether_link::Container {
        aether_link::Container::Data
    }
    fn mode(&self) -> usize {
        self.mode
    }
    fn floor(&self) -> bool {
        false
    }
    fn rv(&self) -> u8 {
        0
    }
    fn snr_db(&self) -> f64 {
        10.0
    }
    fn t_start(&self) -> f64 {
        0.0
    }
    fn t_end(&self) -> f64 {
        1.0
    }
    fn decode(
        &self,
        _buffer: Option<&aether_link::HarqBuffer>,
    ) -> (Option<Vec<u8>>, aether_link::HarqBuffer) {
        (Some(self.payload.clone()), Vec::new())
    }
}

#[test]
fn a_call_in_another_link_protocol_is_ignored_and_said_so() {
    use aether_link::frames::{ConnectBody, DataHeader, DataKind, PROTOCOL_VERSION, encode_data};
    // version 7 offers the turn at the end of a burst (ADR-0047), which a version 6 station
    // would take for a TURN to answer; version 6 counts a burst's following frames in pairs
    // (ADR-0046), which a version 5
    // receiver reads as the frames themselves and answers a frame early; version 5 turns an
    // ordinary data frame's mode chips by the frames of its burst that
    // follow it (ADR-0041), which a version 4 receiver takes for noise; version 4 of the link
    // protocol numbers the 500 Hz ladder's rungs with its middle kinds
    // (ADR-0015), version 3 the 2 300 Hz one's with the fast kinds (ADR-0014), version 2 the
    // ladders before them (ADR-0013), version 1 OFDM modes: a station of another version means
    // other frames by the same numbers, so a call from one is not a session to start — it is
    // ignored, with an event saying why
    assert_eq!(PROTOCOL_VERSION, 7);
    for version in [1u8, 2, 3, 4, 5, 6] {
        let t = timing(false);
        let mut b = LinkEngine::new("KK4XYZ", t.clone(), LinkConfig::default(), 2);
        let body = ConnectBody {
            src: "W4ODA".into(),
            dst: "KK4XYZ".into(),
            caps: 0,
            version,
            snr_db: None,
        }
        .encode()
        .expect("body");
        let header = DataHeader {
            kind: DataKind::ConnectReq,
            seq: 0,
            session: 7,
        };
        let payload = encode_data(&header, &body, t.capacity(2)).expect("frame");
        b.on_frame(&Handed { payload, mode: 2 }, 1.0);
        assert_eq!(b.state(), State::Idle);
        let events: Vec<String> = b
            .drain()
            .into_iter()
            .filter_map(|action| match action {
                aether_link::Action::Event { name, detail } => Some(format!("{name}:{detail}")),
                _ => None,
            })
            .collect();
        assert!(
            events
                .iter()
                .any(|e| e.contains(&format!("link protocol {version}"))),
            "{events:?}"
        );
    }
}

#[test]
fn the_iss_waits_out_the_irs_quiet_after_a_floor_burst() {
    // the ISS sizes its wait for an ACK by the quiet the IRS keeps after a burst. It has to
    // ask about the burst it sent — a floor burst straight after an ordinary acceptance — not
    // the family it last heard: asked the other way it under-waited by the difference, a
    // whole tone frame without preamble reports, and a floor session pinned at 16 dB died of
    // ACK timeouts (ADR-0013)
    for reports in [false, true] {
        let t = timing(reports);
        let config = LinkConfig {
            max_mode: 0,
            ..LinkConfig::default()
        };
        let (mut a, b) = pair(&t, &config);
        a.connect("KK4XYZ").expect("idle");
        let message = vec![0u8; 600];
        a.send(&message);
        a.disconnect();
        let mut sim = TwoStationSim::new(a, b, 16.0, 21);
        sim.run(1200.0, 3.0);
        assert_eq!(sim.delivered(1), message.as_slice(), "reports {reports}");
        assert_eq!(
            (
                sim.engine(0).stats.ack_timeouts,
                sim.engine(1).stats.ack_timeouts
            ),
            (0, 0),
            "reports {reports}"
        );
    }
}

/// A pipe's frame SNR offset: every POLL arrives far below any threshold — its preamble is
/// heard, the frame never decodes; everything else as the channel gives.
fn polls_unreadable() -> aether_link::sim::FrameSnrOffset {
    use aether_link::{ControlFrame, ControlKind, TxFrame};
    Box::new(|frame: &TxFrame| {
        let poll = frame.container == Container::Control
            && ControlFrame::decode(&frame.payload).is_ok_and(|c| c.kind == ControlKind::Poll);
        if poll { -80.0 } else { 0.0 }
    })
}

#[test]
fn a_session_whose_polls_never_decode_lives_on_their_late_answers() {
    // The chat bench's trace (500 Hz, ITU Good, −6 dB, ADR-0027 §7 (1), ADR-0028), made
    // certain: the call and its acceptance go on the tone floor, the confirming poll on the
    // ordinary layouts the acceptance's SNR puts the sender on, and the called station, which
    // still hears the floor and cannot read the poll, answers it after the floor's quiet with
    // a floor acknowledgement. Waited for as if it began a turnaround after the poll, the
    // answer was run into by the repeat, every repeat by the next answer, and the sender gave
    // up — "no response" — on a path that carried everything else. Now the answer is heard,
    // the keepalive polls live the same way, and the message sent later arrives
    for params in [WIDE_2300, NARROW_500] {
        let t = air_timing(params, true);
        let config = LinkConfig {
            max_mode: air_interface(params).n_rungs() - 1,
            ..LinkConfig::default()
        };
        let (mut a, b) = pair(&t, &config);
        a.connect("KK4XYZ").expect("idle");
        let mut sim = TwoStationSim::new(a, b, 10.0, 4).with_frame_snr_offset(polls_unreadable());
        sim.run(90.0, 3.0);
        assert_eq!(
            sim.engine(0).state(),
            State::Connected,
            "{:?}",
            sim.events(0)
        );
        assert_eq!(
            sim.engine(1).state(),
            State::Connected,
            "{:?}",
            sim.events(1)
        );
        // the called station answered every poll on the floor, and the caller heard it
        let answers = sim
            .frames_sent(1)
            .iter()
            .filter(|f| f.container == Container::Control);
        assert!(answers.clone().count() > 1 && answers.clone().all(|f| f.floor));
        assert_eq!(sim.engine(0).stats.ack_timeouts, 0);
        let message = b"sent after a minute of polls nobody could read";
        sim.engine_mut(0).send(message);
        sim.engine_mut(0).disconnect();
        let until = sim.t + 120.0;
        sim.run(until, 3.0);
        assert_eq!(sim.delivered(1), message.as_slice(), "{:?}", sim.events(0));
        assert!(
            sim.events(0).iter().any(|e| e == "disconnected:closed"),
            "{:?}",
            sim.events(0)
        );
    }
}

#[test]
fn a_receiving_station_leaves_at_once_when_asked() {
    // KE4QCM, 2026-09-25: five sessions ended with Abort. On the receiving side Disconnect
    // only put the DISC in place of the next acknowledgement, and on a path where nothing
    // decodable came there never was one. A receiving station leaves between bursts now
    // (ADR-0023); the sender here stays quiet, polling nobody for five minutes
    let t = timing(false);
    let config = LinkConfig {
        keepalive_s: 300.0,
        ..LinkConfig::default()
    };
    let (a, b) = pair(&t, &config);
    let mut sim = TwoStationSim::new(a, b, 15.0, 6);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.run(30.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    assert_eq!(sim.engine(1).state(), State::Connected);
    assert_eq!(sim.engine(1).role(), Role::Irs);
    let start = sim.t;
    sim.engine_mut(1).disconnect();
    sim.run(start + 30.0, 3.0);
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert!(
        sim.events(1).iter().any(|e| e == "disconnected:closed"),
        "{:?}",
        sim.events(1)
    );
    assert!(
        sim.events(0)
            .iter()
            .any(|e| e == "disconnected:peer disconnected")
    );
}

#[test]
fn a_called_sender_yields_to_the_callers_poll() {
    // KE4QCM, 2026-09-25 (23:30:58): the caller handed the turn over, heard none of the
    // called station's data, took the turn back when its TURN tries ran out and polled; the
    // called station, sure it held the turn, ignored the polls and gave up: "no response".
    // It yields to the caller's poll now and asks for the turn back, so the session outlives
    // a fade that lets control frames through and no data (ADR-0023)
    let t = timing(false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let fade = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
    let until = std::rc::Rc::clone(&fade);
    // the caller detects nothing of the called station's data, not a preamble, not a frame;
    // no data flows the other way here, so every DATA frame after the connect is the
    // called one's
    let mut sim =
        TwoStationSim::new(a, b, 15.0, 5).with_unheard(Box::new(move |rx, container, t0| {
            rx == 0 && container == Container::Data && t0 < until.get()
        }));
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.run(30.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    assert_eq!(sim.engine(1).state(), State::Connected);
    let fade_until = sim.t + 150.0;
    fade.set(fade_until);
    let message = b"sent while the path would carry no data";
    sim.engine_mut(1).send(message);
    sim.run(fade_until + 300.0, 3.0);
    assert_eq!(sim.delivered(0), message.as_slice(), "{:?}", sim.events(1));
    assert!(!sim.events(1).iter().any(|e| e.starts_with("disconnected")));
}

#[test]
fn a_climbed_session_outlives_a_fade_that_carries_no_data() {
    // ADR-0023's fade after the link has climbed: both stations' frames are OFDM, so silence
    // ends the session after 45 s, not after four floor exchanges. The caller hears none of the
    // called station's data for 90 s while control frames get through; what keeps the link
    // alive is the poll of a caller that took the turn back after its TURNs went unanswered,
    // which the called station answers by yielding. A caller that offered the turn for longer
    // on silence — ADR-0029's rejected rule — heard nothing it could read for as long, and the
    // session timed out, ten seeds in ten
    let t = timing(false);
    for seed in 0..3u64 {
        let a = LinkEngine::new("W4ODA", t.clone(), LinkConfig::default(), seed);
        let b = LinkEngine::new("KK4XYZ", t.clone(), LinkConfig::default(), seed + 1);
        let fade = std::rc::Rc::new(std::cell::Cell::new(0.0f64));
        let until = std::rc::Rc::clone(&fade);
        let mut sim = TwoStationSim::new(a, b, 15.0, seed).with_unheard(Box::new(
            move |rx, container, t0| rx == 0 && container == Container::Data && t0 < until.get(),
        ));
        sim.engine_mut(0).connect("KK4XYZ").expect("idle");
        sim.run(30.0, 3.0);
        // an exchange on a clean path first: the link climbs to OFDM both ways
        sim.engine_mut(1)
            .send(&b"a first line, before the fade ".repeat(4));
        let now = sim.t;
        sim.run(now + 60.0, 3.0);
        sim.engine_mut(0).send(&b"and its reply ".repeat(6));
        let now = sim.t;
        sim.run(now + 60.0, 3.0);
        assert!(
            sim.modes_sent().iter().any(|&m| !t.is_floor(m)),
            "seed {seed}: the link never climbed"
        );
        fade.set(sim.t + 90.0);
        let message = b"sent while the path would carry no data";
        sim.engine_mut(1).send(message);
        sim.run(fade.get() + 300.0, 3.0);
        assert!(sim.delivered(0).ends_with(message), "seed {seed}");
        for who in 0..2 {
            assert!(
                !sim.events(who)
                    .iter()
                    .any(|e| e.starts_with("disconnected")),
                "seed {seed}: {:?}",
                sim.events(who)
            );
        }
    }
}

// ── chat: asking for the turn (ADR-0027) ──────────────────────────────

/// A session up on a clean path with preambles reported, as the daemon runs it: the caller
/// holding the turn with nothing to send, each station in chat or not.
fn chat_session(chat: (bool, bool), seed: u64) -> TwoStationSim {
    let t = timing(true);
    let config = |on: bool| LinkConfig {
        chat: on,
        ..LinkConfig::default()
    };
    let mut a = LinkEngine::new("W4ODA", t.clone(), config(chat.0), 1);
    let b = LinkEngine::new("KK4XYZ", t, config(chat.1), 2);
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, seed);
    sim.run(30.0, f64::INFINITY);
    assert_eq!(sim.engine(0).state(), State::Connected);
    assert_eq!(sim.engine(0).role(), Role::Iss);
    assert_eq!(sim.engine(1).role(), Role::Irs);
    sim
}

/// A quiet moment on an idle link: two seconds after the sender `iss`'s last poll was
/// answered, its next one still six or more away.
fn between_polls(sim: &mut TwoStationSim, iss: usize) -> f64 {
    let keepalive = sim.engine(iss).config().keepalive_s;
    let start = sim.t;
    let mut t = start;
    while t < start + 120.0 {
        t += 0.25;
        sim.run(t, f64::INFINITY);
        if sim
            .engine(iss)
            .next_deadline()
            .is_some_and(|due| due - t >= keepalive - 2.0)
        {
            return t + 2.0;
        }
    }
    panic!("the sender never went idle");
}

/// When each station of `expected` — `(who, bytes)` — has that many bytes delivered, to a
/// tenth of a second, running the simulation on from `since`.
fn arrivals(sim: &mut TwoStationSim, expected: &[(usize, usize)], since: f64) -> Vec<Option<f64>> {
    let mut got = vec![None; expected.len()];
    let mut t = since;
    while got.iter().any(Option::is_none) && t < since + 300.0 {
        t += 0.1;
        sim.run(t, f64::INFINITY);
        for (slot, &(who, length)) in got.iter_mut().zip(expected) {
            if slot.is_none() && sim.delivered(who).len() >= length {
                *slot = Some(t);
            }
        }
    }
    got
}

#[test]
fn in_a_chat_the_receiving_station_asks_for_the_turn() {
    // a line typed at the station that does not hold the turn waited for the sender's next
    // poll; in a chat the station asks at once, and the idle sender hands over
    let line = b"QSL on the report, 5 by 7 here in Atlanta";
    let mut took = [0.0; 2];
    for chat in [false, true] {
        let mut sim = chat_session((chat, chat), 5);
        let t = between_polls(&mut sim, 0);
        sim.send_at(1, line, t);
        let arrived = arrivals(&mut sim, &[(0, line.len())], t)[0].expect("the line arrives");
        took[usize::from(chat)] = arrived - t;
        assert_eq!(sim.engine(1).stats.turn_requests, usize::from(chat));
        assert_eq!(sim.engine(0).stats.turns, 1, "one handover either way");
    }
    assert!(took[0] > 6.0, "{took:?}"); // it waited for the poll
    assert!(took[1] < 4.0, "{took:?}");
}

#[test]
fn chat_lines_typed_at_both_stations_at_once_both_arrive() {
    // the one risk in speaking unasked: the other station keys at the same moment; each
    // side's usual recovery handles it
    let mut sim = chat_session((true, true), 5);
    let t = between_polls(&mut sim, 0);
    let (mine, theirs) = (b"going QRT for dinner shortly", b"same here, 73");
    sim.send_at(0, mine, t);
    sim.send_at(1, theirs, t);
    assert_eq!(sim.engine(1).stats.turn_requests, 1);
    sim.run(t + 120.0, f64::INFINITY);
    assert_eq!(sim.delivered(1), mine.as_slice());
    assert_eq!(sim.delivered(0), theirs.as_slice());
    for who in 0..2 {
        assert!(
            !sim.events(who)
                .iter()
                .any(|e| e.starts_with("disconnected")),
            "{:?}",
            sim.events(who)
        );
    }
}

#[test]
fn a_chat_station_and_one_without_chat_still_talk() {
    // chat is each station's own setting: a request nobody takes costs one control frame,
    // and the line goes when the sender polls, as it always did
    for chat in [(true, false), (false, true)] {
        let mut sim = chat_session(chat, 5);
        // the called station first, while the caller holds the turn; then the caller, while
        // the called station does — it took the turn to send its line
        let lines: [(usize, &[u8], usize); 2] = [
            (1, b"first from the called station", 0),
            (0, b"and an answer", 1),
        ];
        for (who, line, iss) in lines {
            let t = between_polls(&mut sim, iss);
            sim.send_at(who, line, t);
            let arrived = arrivals(&mut sim, &[(1 - who, line.len())], t)[0];
            assert!(arrived.is_some(), "{chat:?}, station {who}");
            sim.run(sim.t + 1.0, f64::INFINITY);
        }
        for who in 0..2 {
            assert!(
                !sim.events(who)
                    .iter()
                    .any(|e| e.starts_with("disconnected"))
            );
        }
    }
}

#[test]
fn chat_leaves_a_transfer_alone() {
    // a file inside a chat session, the receiving station typing a line while it arrives:
    // its acknowledgements ask for the turn as they always did — a request is never sent
    // into a transfer — so the file and the line arrive exactly as without chat
    let document: Vec<u8> = (0..3000).map(|i| (i * 37 % 256) as u8).collect();
    let line = b"got the first part, looks good";
    let mut times = Vec::new();
    for chat in [false, true] {
        let mut sim = chat_session((chat, chat), 5);
        let t = between_polls(&mut sim, 0);
        sim.send_at(0, &document, t);
        sim.send_at(1, line, t + 4.0);
        let got = arrivals(&mut sim, &[(1, document.len()), (0, line.len())], t + 4.0);
        assert_eq!(sim.delivered(1), document.as_slice());
        assert_eq!(sim.delivered(0), line.as_slice());
        assert_eq!(sim.engine(1).stats.turn_requests, 0);
        times.push(got);
    }
    assert_eq!(times[0], times[1]);
}

#[test]
fn a_poll_is_not_repeated_over_its_answer_arriving() {
    // ADR-0027 §7, found by the chat bench: a receiving station that detects a poll and cannot
    // decode it answers the preamble once its quiet after a frame has passed — 0.99 s when the
    // last frame it decoded was a floor one, and the answer goes on the floor, 3.2 s — while
    // the sender waited for an answer starting within a turnaround. The re-poll went out 0.2 s
    // before the answer ended, the sender heard none of it, the next answer met the next
    // re-poll, and the sender gave up with the link up: "no response". A sender waits out a
    // frame it hears arriving, as a caller, a leaving station and one that handed over the
    // turn already did
    for (params, floor_control) in [
        (WIDE_2300, CONTROL_THRESHOLD_DB[1]),
        (NARROW_500, NARROW_CONTROL_THRESHOLD_DB[1]),
    ] {
        // preamble reports, as the daemon gives them: the receiving station answers what it
        // detected and could not decode
        let t = air_timing(params, true);
        let (a, b) = pair(&t, &LinkConfig::default());
        // a path the tone floor's frames cross and the ordinary control frame does not: the
        // call goes on the floor and measures a strong path, so the sender polls in the
        // ordinary family of the rung it would send at, and the receiving station, which
        // decoded only the floor's call, answers every poll it detects on the floor — and
        // since that answer did not read its poll, the sender's next poll goes on the floor,
        // and the one after it, answered in its own family, is ordinary again (ADR-0034)
        let mut sim =
            TwoStationSim::new(a, b, 20.0, 3).with_control_thresholds([99.0, floor_control]);
        sim.engine_mut(0).connect("KK4XYZ").expect("idle");
        sim.run(100.0, 3.0);
        let a = sim.engine(0);
        assert_eq!(
            a.state(),
            State::Connected,
            "{:?}: {:?}",
            params.bandwidth,
            sim.events(0)
        );
        let controls = |who: usize| {
            sim.frames_sent(who)
                .iter()
                .filter(|f| f.container == Container::Control)
                .map(|f| f.floor)
                .collect::<Vec<_>>()
        };
        let polls = controls(0);
        assert!(
            polls.len() >= 4
                && !polls[0]
                && polls.windows(2).all(|w| w[0] != w[1])
                && controls(1).iter().all(|&floor| floor),
            "not the case measured: {polls:?}"
        );
        assert_eq!(a.stats.ack_timeouts, 0, "{:?}", params.bandwidth);
        assert!(
            a.stats.acks_received >= 5,
            "{:?}: every poll's answer is heard",
            params.bandwidth
        );
    }
}

#[test]
fn the_polls_step_down_to_the_floor_through_a_fade() {
    // ADR-0030 §3, found on the chat bench: an ordinary poll and its ordinary answer lost
    // together in a slow ITU Good fade, polled again every 2.3 s until the retries ran out —
    // "no response" — while the tone floor, 14 dB lower, was never tried: an unanswered poll
    // stepped nothing down. Here a minute and a half below the ordinary control frame and above
    // the floor's, after a strong start: the polls step down to the floor, the session stays up
    // through the fade, and what is sent after it arrives (ADR-0032)
    const FADE: (f64, f64) = (60.0, 150.0);
    const DEEP: f64 = -12.0;
    for params in [WIDE_2300, NARROW_500] {
        let t = air_timing(params, true);
        let controls = t.control_threshold_db.expect("the air's control frames");
        assert!(
            controls[1] + 6.0 < DEEP && DEEP < controls[0] - 6.0,
            "not the case measured"
        );
        let config = LinkConfig {
            max_mode: air_interface(params).n_rungs() - 1,
            ..LinkConfig::default()
        };
        let (a, b) = pair(&t, &config);
        let mut sim = TwoStationSim::new(a, b, 15.0, 3).with_snr_schedule(Box::new(|at| {
            if (FADE.0..FADE.1).contains(&at) {
                DEEP
            } else {
                15.0
            }
        }));
        let first: Vec<u8> = (0..200u8).collect();
        let second: Vec<u8> = (56..=255u8).chain(56..=255u8).collect();
        sim.engine_mut(0).connect("KK4XYZ").expect("idle");
        sim.engine_mut(0).send(&first);
        sim.run(FADE.0, 3.0);
        let bandwidth = params.bandwidth;
        assert_eq!(
            sim.delivered(1),
            first.as_slice(),
            "{bandwidth:?}: {:?} {:?}",
            sim.events(0),
            sim.events(1)
        );
        assert_eq!(sim.engine(0).role(), Role::Iss, "not the case measured");
        let before = sim.frames_sent(0).len();
        let heard = sim.engine(0).stats.acks_received;
        sim.run(FADE.1, 3.0);
        assert!(
            sim.engine(0).connected() && sim.engine(1).connected(),
            "{bandwidth:?}: {:?} {:?}",
            sim.events(0),
            sim.events(1)
        );
        let during: Vec<bool> = sim.frames_sent(0)[before..]
            .iter()
            .filter(|f| f.container == Container::Control)
            .map(|f| f.floor)
            .collect();
        assert!(
            during.first() == Some(&false) && during.contains(&true),
            "{bandwidth:?}: {during:?}"
        );
        assert!(
            sim.engine(0).stats.acks_received > heard,
            "{bandwidth:?}: no poll was answered in the fade"
        );
        sim.send_at(0, &second, FADE.1 + 1.0);
        sim.run(FADE.1 + 300.0, 3.0);
        let both: Vec<u8> = first.iter().chain(&second).copied().collect();
        assert_eq!(
            sim.delivered(1),
            both.as_slice(),
            "{bandwidth:?}: {:?} {:?}",
            sim.events(0),
            sim.events(1)
        );
        assert!(
            !sim.events(0)
                .iter()
                .chain(sim.events(1))
                .any(|e| e.starts_with("disconnected")),
            "{bandwidth:?}"
        );
    }
}

#[test]
fn a_session_whose_link_falls_to_the_floor_is_not_cut_off_on_the_way() {
    // ADR-0033 end to end, as the chat bench found it at 500 Hz: the called station holds the
    // turn, sending at the first OFDM rung, when the path falls below that rung and below the
    // ordinary control frame. Its bursts and the answers to them are lost; it steps down, and
    // its frames go to the floor after their four tries, half a minute after the last answer
    // it heard — whose 45 s ran out 12 s into the first tone burst. Before, it ended the
    // session there, with the answer to that burst on its way
    const FADE: f64 = 25.0;
    let t = air_timing(NARROW_500, true);
    let config = LinkConfig {
        max_mode: 4,
        max_burst_s: Some(28.85),
        ..LinkConfig::default()
    };
    let (mut a, mut b) = pair(&t, &config);
    let message: Vec<u8> = (0..240u8).chain(0..240u8).collect();
    a.connect("KK4XYZ").expect("idle");
    a.send(b"hello");
    b.send(&message);
    let mut sim = TwoStationSim::new(a, b, 10.0, 5)
        .with_snr_schedule(Box::new(|at| if at >= FADE { -16.0 } else { 10.0 }));
    sim.run(FADE, 3.0);
    let sent = sim.delivered(0).len();
    assert!(
        sim.engine(1).role() == Role::Iss && sent > 0 && sent < message.len(),
        "not the case measured"
    );
    let mut until = FADE;
    while until < 1500.0 && sim.delivered(0).len() < message.len() {
        until += 10.0;
        sim.run(until, 3.0);
    }
    assert_eq!(
        sim.delivered(0),
        message.as_slice(),
        "{:?} {:?}",
        sim.events(0),
        sim.events(1)
    );
    assert!(
        !sim.events(0)
            .iter()
            .chain(sim.events(1))
            .any(|e| e.starts_with("disconnected")),
        "{:?} {:?}",
        sim.events(0),
        sim.events(1)
    );
}

#[test]
fn an_answer_that_did_not_read_its_frame_sends_the_next_on_the_floor() {
    // ADR-0034 end to end, the chat bench's trial 184 (500 Hz, ITU Moderate, 0 dB) made
    // certain: the path falls from 15 dB to −12 dB — below the ordinary control frame, above
    // the floor's — while the called station holds the turn with nothing to send. Its ordinary
    // polls and their answers are lost, and from the second silence its polls step down to the
    // floor (ADR-0032); the other station reads one, and from then on answers on the floor
    // every ordinary frame it hears and cannot read. The poller took each such answer for the
    // answer and went back to ordinary polls — and to ordinary TURNs for the line waiting at
    // the other station — and the other station, reading nothing, ended the session 45 s after
    // the floor poll, idle or not. Now an answer that did not read its frame sends the next one
    // on the floor: the line arrives, and an idle link stays up
    use std::{cell::Cell, rc::Rc};
    const DEEP: f64 = -12.0;
    for params in [WIDE_2300, NARROW_500] {
        let t = air_timing(params, true);
        let controls = t.control_threshold_db.expect("the air's control frames");
        assert!(
            controls[1] + 6.0 < DEEP && DEEP < controls[0] - 6.0,
            "not the case measured"
        );
        let config = LinkConfig {
            max_mode: air_interface(params).n_rungs() - 1,
            ..LinkConfig::default()
        };
        let bandwidth = params.bandwidth;
        for waiting in [true, false] {
            let (a, b) = pair(&t, &config);
            let fade = Rc::new(Cell::new(f64::INFINITY));
            let from = Rc::clone(&fade);
            let mut sim =
                TwoStationSim::new(a, b, 15.0, 3).with_snr_schedule(Box::new(move |at| {
                    if at >= from.get() { DEEP } else { 15.0 }
                }));
            let reply: Vec<u8> = (0..60u8).collect();
            sim.engine_mut(0).connect("KK4XYZ").expect("idle");
            sim.engine_mut(0).send(b"hello");
            sim.engine_mut(1).send(&reply);
            sim.run(60.0, 3.0);
            assert!(
                sim.delivered(0) == reply.as_slice()
                    && sim.engine(1).role() == Role::Iss
                    && !t.is_floor(sim.engine(1).current_mode()),
                "{bandwidth:?}: not the case measured"
            );
            fade.set(sim.t);
            let line = b"typed at the station that does not hold the turn";
            if waiting {
                sim.send_at(0, line, fade.get() + 20.0);
            }
            sim.run(fade.get() + 300.0, 3.0);
            assert!(
                !sim.events(0)
                    .iter()
                    .chain(sim.events(1))
                    .any(|e| e.starts_with("disconnected")),
                "{bandwidth:?} waiting {waiting}: {:?} {:?}",
                sim.events(0),
                sim.events(1)
            );
            if waiting {
                let expected: Vec<u8> = b"hello".iter().chain(line).copied().collect();
                assert_eq!(
                    sim.delivered(1),
                    expected.as_slice(),
                    "{bandwidth:?}: {:?} {:?}",
                    sim.events(0),
                    sim.events(1)
                );
            }
        }
    }
}

#[test]
fn a_burst_is_not_repeated_over_its_acknowledgement_arriving() {
    // A burst's acknowledgement can start late too — the receiving station's quiet stretched
    // by a frame it heard arriving, a receiver running behind — and a burst sent again over it
    // loses the acknowledgement and costs the burst's air time. The retry waits for the
    // frame's end, and the acknowledgement is taken when it arrives. The burst alone: without
    // the turn on offer after it (ADR-0047)
    let t = timing(false);
    let config = LinkConfig {
        offer_turn: false,
        ..LinkConfig::default()
    };
    let (mut a, mut b) = pair(&t, &config);

    // the handshake over a perfect wire, with a line waiting to go
    a.connect("KK4XYZ").expect("idle");
    a.send(b"a line typed at the keyboard");
    let request = transmitted(&mut a);
    assert_eq!(request.len(), 1);
    let mut now = t.frame_s(&request[0]);
    a.on_tx_done(now);
    b.on_frame(&Wire::carry(&request[0], 0.0, &t), now);
    let accept = transmitted(&mut b);
    assert_eq!(accept.len(), 1);
    let answered = now;
    now += t.frame_s(&accept[0]);
    a.on_frame(&Wire::carry(&accept[0], answered, &t), now);
    assert!(a.connected());
    let burst = transmitted(&mut a);
    assert!(
        !burst.is_empty() && burst.iter().all(|f| f.container == Container::Data),
        "{burst:?}"
    );
    for frame in &burst {
        b.on_frame(&Wire::carry(frame, now, &t), now + t.frame_s(frame));
        now += t.frame_s(frame);
    }
    a.on_tx_done(now);
    b.tick(now + 30.0);
    let acks = transmitted(&mut b);
    assert_eq!(
        acks.len(),
        1,
        "the receiving station acknowledges the burst"
    );

    // the acknowledgement is heard arriving just before the sender would try again
    let due = a.next_deadline().expect("waiting for the acknowledgement");
    let length = t.frame_s(&acks[0]);
    let t_start = due - 0.2;
    a.on_preamble(t_start, t_start + 0.1, Some(length));
    assert!(a.next_deadline().expect("armed") >= t_start + length);
    a.tick(t_start + length);
    assert!(transmitted(&mut a).is_empty(), "repeated over the frame");
    a.on_frame(&Wire::carry(&acks[0], t_start, &t), t_start + length + 0.01);
    assert!(a.all_acknowledged());
    assert_eq!(a.stats.ack_timeouts, 0);
}

// ── ADR-0029: a TURN whose answer is lost ─────────────────────────────

/// Whether both stations of a session hold the turn.
fn both_sending(sim: &TwoStationSim) -> bool {
    let (a, b) = (sim.engine(0), sim.engine(1));
    a.connected() && b.connected() && a.role() == Role::Iss && b.role() == Role::Iss
}

/// The kind of each control frame among `frames`.
fn control_kinds(frames: &[aether_link::TxFrame]) -> Vec<aether_link::ControlKind> {
    frames
        .iter()
        .filter(|f| f.container == Container::Control)
        .map(|f| {
            aether_link::ControlFrame::decode(&f.payload)
                .expect("a control frame")
                .kind
        })
        .collect()
}

#[test]
fn a_turn_whose_answer_was_lost_is_answered_again() {
    // ADR-0029 (found by the chat bench, ADR-0027 §7): the caller hands the turn over and hears
    // nothing of the called station until its TURN has gone out three times — not the burst
    // that answered it, not what followed. The called station, holding the turn, ignored the
    // TURN repeated, and the caller took the turn back when its tries ran out: two senders,
    // each sending into the other. The called station answers a TURN heard again now, and the
    // caller hears the answer to its third
    let t = timing(false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let offered = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let seen = std::rc::Rc::clone(&offered);
    // the caller hears nothing of the called station, preamble or frame, from its first TURN
    // until its third has gone out: the answer to the TURN and everything after it
    let mut sim = TwoStationSim::new(a, b, 15.0, 0).with_unheard(Box::new(move |rx, _, _| {
        rx == 0 && (1..3).contains(&seen.get())
    }));
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.run(30.0, 3.0);
    assert!(sim.engine(0).connected() && sim.engine(1).connected());
    let before = sim.engine(0).stats.turns;
    let message = b"typed at the station that does not hold the turn";
    sim.engine_mut(1).send(message);
    let mut clock = sim.t;
    let end = clock + 300.0;
    let mut both = None;
    while clock < end {
        clock += 0.01;
        sim.run(clock, 3.0);
        offered.set(sim.engine(0).stats.turns - before);
        if both.is_none() && both_sending(&sim) {
            both = Some(clock);
        }
    }
    assert_eq!(sim.delivered(0), message.as_slice(), "{:?}", sim.events(0));
    assert!(offered.get() >= 3, "{} TURNs", offered.get());
    assert_eq!(both, None, "both held the turn");
}

#[test]
fn a_turn_answered_by_a_poll_the_sender_cannot_read() {
    // ADR-0029, the chat bench's trace: a station takes the turn with nothing of its own to
    // send and polls; the station that handed it over cannot read the poll, answers its
    // preamble with an acknowledgement — which tells the new sender all is well — and offers
    // the turn again. The new sender ignored the TURN, the old one could not read its next poll
    // either, and took the turn back after three TURNs: two senders. The new sender answers
    // each TURN now, and the old one, having heard frames it could not read, offers until it
    // reads an answer — here the answer to its fourth TURN
    let t = timing(true);
    let (a, b) = pair(&t, &LinkConfig::default());
    // the first three polls after the TURN arrive far too weak to read — each the answer to a
    // TURN, or the one after it; their preambles are still heard, a preamble being far below
    // any frame's threshold
    let spoiled = std::rc::Rc::new(std::cell::RefCell::new(Vec::<(f64, f64)>::new()));
    let windows = std::rc::Rc::clone(&spoiled);
    let mut sim = TwoStationSim::new(a, b, 15.0, 0).with_snr_schedule(Box::new(move |at| {
        if windows
            .borrow()
            .iter()
            .any(|&(t0, t1)| t0 <= at && at <= t1)
        {
            -60.0
        } else {
            15.0
        }
    }));
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.run(30.0, 3.0);
    assert!(sim.engine(0).connected() && sim.engine(1).connected());
    let turns = sim.engine(0).stats.turns;
    // the turn, with nothing to send yet: it is answered with a poll
    sim.engine_mut(1).request_break();
    let mut watched: Option<usize> = None;
    let mut clock = sim.t;
    let end = clock + 60.0;
    let mut both = None;
    while clock < end {
        let from = clock;
        clock += 0.01;
        sim.run(clock, 3.0);
        let sent = sim.frames_sent(1).len();
        match watched {
            None if sim.engine(0).stats.turns > turns => watched = Some(sent),
            Some(seen) if sent > seen => {
                // a control frame the called station keyed after the TURN: a poll
                let polled = sim.frames_sent(1)[seen..]
                    .iter()
                    .any(|f| f.container == Container::Control);
                if polled && spoiled.borrow().len() < 3 {
                    let window = (from - 0.01, clock + t.control_frame_s + 0.05);
                    spoiled.borrow_mut().push(window);
                }
                watched = Some(sent);
            }
            _ => {}
        }
        if both.is_none() && both_sending(&sim) {
            both = Some(clock);
        }
    }
    assert_eq!(spoiled.borrow().len(), 3);
    assert_eq!(sim.engine(1).role(), Role::Iss);
    assert_eq!(sim.engine(0).role(), Role::Irs);
    assert_eq!(both, None, "both held the turn");
    // and the session carries on the other way
    let message = b"sent by the station that took the turn";
    sim.engine_mut(1).send(message);
    sim.run(clock + 60.0, 3.0);
    assert_eq!(sim.delivered(0), message.as_slice());
}

/// A frame whose preamble was heard and whose bits could not be read.
struct Unreadable {
    t_start: f64,
    t_end: f64,
}

impl aether_link::SoftFrame for Unreadable {
    fn container(&self) -> Container {
        Container::Control
    }
    fn mode(&self) -> usize {
        0
    }
    fn floor(&self) -> bool {
        false
    }
    fn rv(&self) -> u8 {
        0
    }
    fn snr_db(&self) -> f64 {
        -20.0
    }
    fn t_start(&self) -> f64 {
        self.t_start
    }
    fn t_end(&self) -> f64 {
        self.t_end
    }
    fn decode(
        &self,
        _buffer: Option<&aether_link::HarqBuffer>,
    ) -> (Option<Vec<u8>>, aether_link::HarqBuffer) {
        (None, aether_link::HarqBuffer::default())
    }
}

/// A readable control frame of the session, as the other station would send it.
fn control_frame(kind: aether_link::ControlKind, session: u8, t_end: f64, t: &PhyTiming) -> Wire {
    let payload = aether_link::ControlFrame {
        kind,
        session,
        flags: 0,
        base: 0,
        bitmap: 0,
        snr_db: None,
        recommended_mode: 0,
        counter: 0,
    }
    .encode()
    .to_vec();
    Wire {
        container: Container::Control,
        mode: 0,
        rv: 0,
        t_start: t_end - t.control_frame_s,
        t_end,
        payload,
        floor: false,
    }
}

/// Carry a transmission from `from` to `to` over a perfect wire, keyed at `at`: the receiver
/// reads each frame as it ends, and the transmission ends with the last. Returns when.
fn relay(
    from: &mut LinkEngine,
    to: &mut LinkEngine,
    frames: &[aether_link::TxFrame],
    at: f64,
) -> f64 {
    let t = from.timing().clone();
    let mut end = at;
    for frame in frames {
        let wire = Wire::carry(frame, end, &t);
        end = wire.t_end;
        to.on_frame(&wire, end);
    }
    from.on_tx_done(end);
    end
}

/// Let a transmission go out and nobody hear it. Returns when it ended.
fn lose(from: &mut LinkEngine, frames: &[aether_link::TxFrame], at: f64) -> f64 {
    let t = from.timing().clone();
    let end = at + frames.iter().map(|f| t.frame_s(f)).sum::<f64>();
    from.on_tx_done(end);
    end
}

/// Run a station's clock to its next timer. Returns when, and what it sends then.
fn at_next_deadline(engine: &mut LinkEngine) -> (f64, Vec<aether_link::TxFrame>) {
    let at = engine.next_deadline().expect("a timer is armed");
    engine.tick(at);
    (at, transmitted(engine))
}

/// Two stations in a session over a perfect wire, by hand, the called one with something to
/// send (`work`) or asking for the turn with nothing (a break): the caller's poll has fetched
/// the wish, and the caller has just keyed its TURN, which nobody has heard yet. Returns them,
/// the TURN and when it was keyed. The wish goes by the poll, as it does when the called
/// station's own request for the turn (ADR-0044) was not heard: these tests are about the TURN
/// that follows, so the request is left out of them.
fn handing_over(
    t: &PhyTiming,
    work: bool,
) -> (LinkEngine, LinkEngine, Vec<aether_link::TxFrame>, f64) {
    let config = LinkConfig {
        chat: false,
        // the TURN these tests are about, not the turn offered at the end of a burst (ADR-0047)
        offer_turn: false,
        ..LinkConfig::default()
    };
    let (mut a, mut b) = pair(t, &config);
    a.connect("KK4XYZ").expect("idle");
    let request = transmitted(&mut a);
    let now = relay(&mut a, &mut b, &request, 0.0);
    let accept = transmitted(&mut b);
    let mut now = relay(&mut b, &mut a, &accept, now);
    assert!(a.connected() && b.connected());
    // the caller confirms the call with a poll, and polls again when the link is idle
    let mut poll = transmitted(&mut a);
    for asked in [false, true] {
        if asked {
            if work {
                b.send(b"a line for the other station");
            } else {
                b.request_break();
            }
            let (at, next) = at_next_deadline(&mut a);
            now = at.max(now);
            poll = next;
        }
        assert_eq!(control_kinds(&poll), [aether_link::ControlKind::Poll]);
        now = relay(&mut a, &mut b, &poll, now);
        let (at, ack) = at_next_deadline(&mut b);
        now = relay(&mut b, &mut a, &ack, at.max(now));
    }
    let turn = transmitted(&mut a);
    assert_eq!(control_kinds(&turn), [aether_link::ControlKind::Turn]);
    (a, b, turn, now)
}

#[test]
fn the_station_holding_the_turn_answers_a_turn_heard_again() {
    // ADR-0029: a TURN heard by the station that already holds the turn says the other station
    // heard nothing of it taking it. The answer goes again — the burst, which the other station
    // heard none of, or the poll — and not while the station's own transmission is going out
    let t = timing(false);
    for work in [false, true] {
        let (mut a, mut b, turn, now) = handing_over(&t, work);
        let now = relay(&mut a, &mut b, &turn, now);
        assert_eq!(b.role(), Role::Iss);
        let answer = transmitted(&mut b);
        let now = lose(&mut b, &answer, now);
        let (at, again) = at_next_deadline(&mut a);
        assert!(at >= now);
        assert_eq!(control_kinds(&again), [aether_link::ControlKind::Turn]);
        let end = relay(&mut a, &mut b, &again, at);
        let second = transmitted(&mut b);
        if work {
            assert!(
                !second.is_empty() && second.iter().all(|f| f.container == Container::Data),
                "work {work}: {second:?}"
            );
        } else {
            assert_eq!(
                control_kinds(&second),
                [aether_link::ControlKind::Poll],
                "work {work}"
            );
        }
        assert_eq!(b.role(), Role::Iss);
        // one heard while the answer is still on the air needs none, and one of another
        // session is nobody's business here
        let session = b.session();
        for other in [session, session.wrapping_add(1)] {
            let turn = control_frame(aether_link::ControlKind::Turn, other, end + 0.2, &t);
            b.on_frame(&turn, end + 0.2);
        }
        assert!(transmitted(&mut b).is_empty(), "work {work}");
    }
}

#[test]
fn a_station_offering_the_turn_does_not_take_it_back_over_what_it_cannot_read() {
    // ADR-0029: a frame heard after a TURN and not read may be the other station's answer from
    // the turn it now holds — a poll too weak to read — and while the last frame heard was such
    // a frame, the offering station offers again, up to max_retries TURNs, rather than take the
    // turn back after turn_retries. An acknowledgement it can read says the other station is
    // still receiving, and silence says nothing either way: after those it takes the turn back
    // as before — a path that carries control frames and no data is kept alive by the poll
    // that follows (ADR-0023)
    let t = timing(false);
    let config = LinkConfig::default();
    let offering = || {
        let (mut a, _, turn, now) = handing_over(&t, true);
        lose(&mut a, &turn, now);
        a
    };
    let unreadable = |a: &mut LinkEngine| {
        let at = a.next_deadline().expect("the TURN's wait") - 0.5;
        let frame = Unreadable {
            t_start: at - t.control_frame_s,
            t_end: at,
        };
        a.on_frame(&frame, at);
    };
    // whether the station offers again: a TURN goes out when the wait runs out
    let offers_again = |a: &mut LinkEngine| {
        let (at, sent) = at_next_deadline(a);
        let turn = control_kinds(&sent) == [aether_link::ControlKind::Turn];
        if turn {
            lose(a, &sent, at);
        }
        turn
    };

    // silence: the turn is taken back after turn_retries TURNs, as it always was
    let mut a = offering();
    for _ in 1..config.turn_retries {
        assert!(offers_again(&mut a));
    }
    assert!(!offers_again(&mut a));
    assert_eq!(a.role(), Role::Iss);

    // a frame it could not read: it offers on, to max_retries, and then takes the turn back
    let mut a = offering();
    unreadable(&mut a);
    for tries in 1..config.max_retries {
        assert!(offers_again(&mut a), "try {}", tries + 1);
    }
    assert!(!offers_again(&mut a));
    assert_eq!(a.role(), Role::Iss);

    // its preamble alone counts: the frame may never arrive whole
    let mut a = offering();
    for _ in 1..config.turn_retries {
        assert!(offers_again(&mut a));
    }
    let at = a.next_deadline().expect("the TURN's wait") - 0.5;
    a.on_preamble(at, at + 0.2, Some(t.control_frame_s));
    assert!(offers_again(&mut a));
    assert_eq!(a.role(), Role::Irs);

    // an acknowledgement read after it: the other station is receiving still
    let mut a = offering();
    unreadable(&mut a);
    for _ in 1..config.turn_retries {
        assert!(offers_again(&mut a));
    }
    let at = a.next_deadline().expect("the TURN's wait") - 0.5;
    let session = a.session();
    a.on_frame(
        &control_frame(aether_link::ControlKind::Ack, session, at, &t),
        at,
    );
    let unexpected = transmitted(&mut a);
    assert!(unexpected.is_empty(), "{unexpected:?}");
    assert!(!offers_again(&mut a));
    assert_eq!(a.role(), Role::Iss);
}

#[test]
fn an_acknowledgement_is_not_acknowledged() {
    // ADR-0029: a receiving station answers a frame it hears arriving as the end of a burst —
    // it may be the last frame of one, or a poll it will not be able to read. When the frame
    // turns out to be an acknowledgement, the other station is receiving too and waits for
    // nothing: no acknowledgement goes back. One did, and was answered in turn; keyed by a
    // station that had just offered the turn, it went out over the answer to its TURN
    let t = timing(false);
    let (_, mut b, _, now) = handing_over(&t, true);
    assert_eq!(b.role(), Role::Irs);
    let start = now + 1.0;
    b.on_preamble(start, start + 0.2, Some(t.control_frame_s));
    let end = start + t.control_frame_s;
    let session = b.session();
    b.on_frame(
        &control_frame(aether_link::ControlKind::Ack, session, end, &t),
        end,
    );
    b.tick(end + 20.0);
    assert!(
        transmitted(&mut b).is_empty(),
        "an acknowledgement answered"
    );
    // a frame it cannot read is still answered: it may be a poll
    let start = end + 25.0;
    b.on_preamble(start, start + 0.2, Some(t.control_frame_s));
    let end = start + t.control_frame_s;
    let frame = Unreadable {
        t_start: start,
        t_end: end,
    };
    b.on_frame(&frame, end);
    let (_, sent) = at_next_deadline(&mut b);
    assert_eq!(control_kinds(&sent), [aether_link::ControlKind::Ack]);
}

#[test]
fn a_disconnect_leaves_without_what_the_path_will_not_carry() {
    // ND1J, 2026-10-05: "the disconnect button does not work". A sender's Disconnect delivers
    // what is queued first, and on a path that carried none of it that never finished: the
    // session ended only when the link timed out. Now the sender waits two whole exchanges with
    // nothing new acknowledged and leaves without the rest, saying how much (ADR-0039).
    let t = air_timing(NARROW_500, false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let mut sim = TwoStationSim::new(a, b, 6.0, 3);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    let connected_by = sim.run(60.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Connected, "{connected_by}");
    // from now on no data frame decodes, whatever its rung; control frames still do
    sim.set_thresholds(Some(vec![99.0; t.mode_threshold_db.len()]));
    sim.engine_mut(0).send(&[0u8; 400]);
    sim.engine_mut(0).disconnect();
    let asked = connected_by;
    let ended = sim.run(asked + 600.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle);
    let events = sim.events(0);
    let last = events
        .iter()
        .rev()
        .find(|e| e.starts_with("disconnected:"))
        .expect("the session ended");
    assert!(!last.contains("link timeout"), "{events:?}");
    assert!(
        events
            .iter()
            .any(|e| e.starts_with("disconnect:") && e.contains("not acknowledged")),
        "{events:?}"
    );
    assert!(ended - asked < 120.0, "{:.0} s to leave", ended - asked);
}

fn fading_session(countdown: bool) -> TwoStationSim {
    fading_session_with(countdown, false)
}

fn fading_session_with(countdown: bool, garbled: bool) -> TwoStationSim {
    fading_session_full(countdown, garbled, false)
}

fn fading_session_full(countdown: bool, garbled: bool, misread: bool) -> TwoStationSim {
    let t = air_timing(NARROW_500, true);
    let (a, b) = pair(&t, &LinkConfig::default());
    // three data frames in ten fade out of the receiver's hearing altogether: not even their
    // preambles are detected (the same frame is judged the same each time it is asked)
    let faded: aether_link::sim::Unheard = Box::new(|rx, container, t0| {
        rx == 1 && container == Container::Data && ((t0 * 1000.0) as i64).rem_euclid(10) < 3
    });
    let mut sim = TwoStationSim::new(a, b, 14.0, 3).with_unheard(faded);
    if !countdown {
        sim = sim.without_countdown();
    }
    if garbled {
        // three in ten more arrive too faint to read
        sim = sim.with_garbled(Box::new(|rx, container, t0| {
            rx == 1 && container == Container::Data && ((t0 * 1000.0) as i64).rem_euclid(10) >= 7
        }));
    }
    if misread {
        // three in ten more arrive undecodable with an acquisition the receiver trusts and
        // chips read wrong: the fastest rung of the ladder, and "none follow"
        let fastest = t.data_capacity.len() - 1;
        sim = sim.with_misread(
            fastest,
            Box::new(|rx, container, t0| {
                rx == 1
                    && container == Container::Data
                    && ((t0 * 1000.0) as i64).rem_euclid(10) >= 7
            }),
        );
    }
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    let connected_by = sim.run(60.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Connected, "{connected_by}");
    let message: Vec<u8> = (0..12).flat_map(|_| 0..=255u8).collect();
    sim.engine_mut(0).send(&message);
    sim.run(connected_by + 900.0, 3.0);
    sim
}

#[test]
fn a_receiver_does_not_answer_over_a_frame_it_lost_in_a_fade() {
    // ND1J, 2026-10-06: the last frame of his bursts arrived faded, KK4ODA-1 took the silence
    // for the end of the burst and acknowledged over it 21 times in eight minutes — the frame
    // lost, the acknowledgement unheard, the burst sent again. Each frame now says how many of
    // its burst follow it (ADR-0041), and a receiver that loses one still waits for it.
    let blind = fading_session(false);
    let told = fading_session(true);
    let message: Vec<u8> = (0..12).flat_map(|_| 0..=255u8).collect();
    assert_eq!(told.delivered(1), message.as_slice());
    let (before, after) = (blind.collisions(0, 1), told.collisions(0, 1));
    assert!(
        before >= 5,
        "the fades caused only {before} collisions without the countdown"
    );
    assert!(
        after <= before / 4,
        "{after} collisions with the countdown, {before} without"
    );
}

#[test]
fn a_frame_read_too_faintly_to_believe_does_not_cut_the_burst_short() {
    // the scenario harness on an 80 m Poor path (ADR-0042): a frame said three more followed,
    // the next arrived too faint to believe its count, and the acknowledgement was set for that
    // frame's end — over the frames the earlier one had announced, eight times in a session
    let blind = fading_session_with(false, true);
    let told = fading_session_with(true, true);
    let message: Vec<u8> = (0..12).flat_map(|_| 0..=255u8).collect();
    assert_eq!(told.delivered(1), message.as_slice());
    let (before, after) = (blind.collisions(0, 1), told.collisions(0, 1));
    assert!(
        after <= before / 4,
        "{after} collisions with the countdown, {before} without"
    );
}

#[test]
fn a_misread_countdown_does_not_cut_the_burst_short() {
    // the scenario harness, 80 m at 500 Hz (ADR-0047): a frame that did not decode read its
    // chips as rung 13 in a burst at rung 7, and its countdown as "none follow"; its acquisition
    // was trusted, and the acknowledgement went at the turnaround over the rest of the burst. A
    // countdown is believed from a frame that did not decode only when the rung it names is one
    // the receiver has asked for or below.
    let sim = fading_session_full(true, false, true);
    let message: Vec<u8> = (0..12).flat_map(|_| 0..=255u8).collect();
    assert_eq!(sim.delivered(1), message.as_slice());
    assert_eq!(sim.collisions(0, 1), 0);
}

#[test]
fn a_frame_whose_copies_go_unheard_is_sent_again_at_rv_0_first() {
    // ADR-0043: a retransmission is as often of a frame the receiver never detected as of one
    // it could not decode, so the second copy is RV 0 again — it decodes on its own and
    // combines with a failed first copy as well as any — then RV 2 and RV 3. RV 1 decodes
    // alone at no SNR, and was the second copy
    let t = timing(false);
    let (a, b) = pair(&t, &LinkConfig::default());
    let connected_at = std::rc::Rc::new(std::cell::Cell::new(f64::INFINITY));
    let after = std::rc::Rc::clone(&connected_at);
    let lost = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let mut sim =
        TwoStationSim::new(a, b, 15.0, 3).with_unheard(Box::new(move |rx, container, t0| {
            if rx == 1 && container == Container::Data && t0 >= after.get() {
                lost.set(lost.get() + 1);
                return lost.get() <= 3;
            }
            false
        }));
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.run(60.0, 3.0);
    assert_eq!(sim.engine(0).state(), State::Connected);
    connected_at.set(sim.t);
    let before = sim.frames_sent(0).len();
    sim.engine_mut(0).send(&[0u8; 10]);
    sim.run(sim.t + 300.0, 3.0);
    assert_eq!(sim.delivered(1), [0u8; 10].as_slice());
    let rvs: Vec<u8> = sim.frames_sent(0)[before..]
        .iter()
        .filter(|f| f.container == Container::Data)
        .map(|f| f.rv)
        .collect();
    assert_eq!(rvs[..4], [0, 0, 2, 3], "{rvs:?}");
}

#[test]
fn an_acknowledgement_goes_a_turnaround_after_a_burst_said_to_be_over() {
    // ADR-0045: a burst whose last frame said none follow it (ADR-0041's countdown at 0) is over
    // when that frame ends, so the acknowledgement waits only the turnaround — not the silence a
    // receiver otherwise needs to be sure no next frame is starting. Without the countdown it
    // waits that, as before
    let t = timing(true);
    let message: Vec<u8> = (0..24).flat_map(|_| 0..=255u8).collect();
    // the countdown alone: without the turn on offer, which closes a burst by itself (ADR-0047)
    let config = LinkConfig {
        offer_turn: false,
        ..LinkConfig::default()
    };
    for countdown in [true, false] {
        let (a, b) = pair(&t, &config);
        let mut sim = TwoStationSim::new(a, b, 20.0, 3);
        if !countdown {
            sim = sim.without_countdown();
        }
        sim.engine_mut(0).connect("KK4XYZ").expect("idle");
        sim.run(60.0, 3.0);
        let (from_a, from_b) = (sim.frames_sent(0).len(), sim.frames_sent(1).len());
        sim.engine_mut(0).send(&message);
        sim.run(sim.t + 300.0, 3.0);
        assert_eq!(sim.delivered(1), message.as_slice());
        // each acknowledgement against the end of the data frame before it
        let data: Vec<f64> = sim.frames_sent(0)[from_a..]
            .iter()
            .filter(|f| f.container == Container::Data)
            .map(|f| f.t_end)
            .collect();
        let gaps: Vec<f64> = sim.frames_sent(1)[from_b..]
            .iter()
            .filter_map(|ack| {
                let last = data
                    .iter()
                    .copied()
                    .filter(|&end| end <= ack.t_start)
                    .fold(f64::NAN, f64::max);
                (ack.t_start - last < 2.0).then_some(ack.t_start - last)
            })
            .collect();
        // without it: a preamble's detection, the gap between bursts, the turnaround
        let silence = t.preamble_detect_s_for(false).expect("reported")
            + LinkConfig::default().burst_gap_s
            + t.turnaround_s;
        let expected = if countdown { t.turnaround_s } else { silence };
        assert!(
            !gaps.is_empty() && gaps.iter().all(|g| (g - 0.01 - expected).abs() < 0.01),
            "countdown {countdown}: expected {expected} after each burst, {gaps:?}"
        );
    }
}

#[test]
fn the_countdown_counts_in_pairs_and_never_short() {
    // ADR-0046: half the frames that follow, rounded up — exact at the last frame, never fewer
    // than follow, one more at most
    use aether_link::frames::{countdown_of, most_following};
    let counts: Vec<u8> = (0..7).map(countdown_of).collect();
    assert_eq!(counts, [0, 1, 1, 2, 2, 3, 3]);
    for n in 0..7usize {
        let most = usize::from(most_following(countdown_of(n)));
        assert!(
            n <= most && most <= n + 1,
            "{n} following, said at most {most}"
        );
    }
}

#[test]
fn a_fade_over_the_end_of_a_burst_is_not_answered_over() {
    // the scenario harness, 80 m at 500 Hz (ADR-0045): a receiver that heard only the first
    // frames of a burst of six read "three or more" as three and answered over the rest.
    // Counted in pairs the countdown never says fewer than follow (ADR-0046): a fade of 3.5 s
    // every 11 s over a 3 kB transfer, which took the ends of bursts, collides with nothing
    let narrow = air_timing(NARROW_500, true);
    let message: Vec<u8> = (0..12).flat_map(|_| 0..=255u8).collect();
    for offset in [2.74f64, 5.48, 13.7] {
        let a = LinkEngine::new("ND1J", narrow.clone(), LinkConfig::default(), 1);
        let b = LinkEngine::new("KK4ODA", narrow.clone(), LinkConfig::default(), 2);
        let mut sim =
            TwoStationSim::new(a, b, 14.0, 3).with_unheard(Box::new(move |rx, container, t0| {
                rx == 1 && container == Container::Data && (t0 + offset) % 11.0 < 3.5
            }));
        sim.engine_mut(0).connect("KK4ODA").expect("idle");
        sim.run(60.0, 3.0);
        sim.engine_mut(0).send(&message);
        sim.run(sim.t + 1200.0, 3.0);
        assert_eq!(sim.delivered(1), message.as_slice(), "offset {offset}");
        assert_eq!(sim.collisions(0, 1), 0, "offset {offset}");
    }
}

/// ADR-0047 (proposed, off by default): a burst that empties the sender's queue ends with a
/// TURN carrying OFFER, and a receiving station with data answers with an acknowledgement
/// carrying TAKEN and its own burst in the same transmission — one keying for the change of
/// direction where the request and the TURN take two. Off, the handover is a TURN as before.
#[test]
fn the_turn_on_offer_is_taken_in_the_acknowledgement() {
    for offer in [false, true] {
        let t = timing(false);
        let config = LinkConfig {
            offer_turn: offer,
            ..LinkConfig::default()
        };
        let (mut a, mut b) = pair(&t, &config);
        let reply: Vec<u8> = "REPLY FROM KK4XYZ. ".repeat(8).into_bytes();
        let outbound = vec![0u8; 300];
        a.connect("KK4XYZ").expect("idle");
        a.send(&outbound);
        b.send(&reply);
        let mut sim = TwoStationSim::new(a, b, 12.0, 5);
        sim.run(300.0, 3.0);
        assert_eq!(
            sim.delivered(1),
            outbound.as_slice(),
            "A to B, offer {offer}"
        );
        assert_eq!(sim.delivered(0), reply.as_slice(), "B to A, offer {offer}");
        let (a, b) = (&sim.engine(0).stats, &sim.engine(1).stats);
        if offer {
            assert!(a.turn_offers >= 1, "offers {}", a.turn_offers);
            assert_eq!(b.turns_taken, 1);
            assert_eq!(a.turns, 0);
        } else {
            assert_eq!((a.turn_offers, b.turns_taken), (0, 0));
            assert_eq!(a.turns, 1);
        }
    }
}

/// ADR-0053: a caller with nothing to send polls to confirm the session, and the poll offers
/// the turn; a called station with something to say — a gateway's greeting — takes it in its
/// acknowledgement and sends at once. Without the offer the acknowledgement says `WANT_TX` and a
/// TURN follows: two more frames, on a weak path two more floor frames (the three-clients
/// scenario: a 120-byte greeting in 23 s).
#[test]
fn a_callers_first_poll_offers_the_turn() {
    let greeting: Vec<u8> = "[RMS Trimode-1.3.3.0-B2FHM$]\r".repeat(4).into_bytes();
    for snr in [-4.0, 12.0] {
        let mut when = [0.0; 2];
        for offer in [false, true] {
            let t = timing(false);
            let config = LinkConfig {
                offer_turn: offer,
                ..LinkConfig::default()
            };
            let (mut a, mut b) = pair(&t, &config);
            b.send(&greeting);
            a.connect("KK4XYZ").expect("idle");
            let mut sim = TwoStationSim::new(a, b, snr, 3);
            let mut at = 0.0;
            while at < 200.0 && sim.delivered(0) != greeting.as_slice() {
                at += 0.5;
                sim.run(at, 1e9);
            }
            assert!(
                at < 200.0,
                "the greeting never arrived, offer {offer} at {snr} dB"
            );
            when[usize::from(offer)] = at;
            let (a, b) = (&sim.engine(0).stats, &sim.engine(1).stats);
            if offer {
                assert_eq!((b.turns_taken, a.turns), (1, 0), "at {snr} dB");
            } else {
                assert_eq!((b.turns_taken, a.turns), (0, 1), "at {snr} dB");
            }
        }
        assert!(when[1] < when[0], "no time saved at {snr} dB: {when:?}");
    }
}

/// ADR-0047: the acknowledgement that takes the turn offered goes unread, and so do the first
/// frames of the burst behind it. The sender must not wait out its acknowledgement and send its
/// burst again over the other station's: a trusted data frame of the session's family arriving
/// after an offer is that burst, and the sender becomes the receiving station (found by the
/// scenario harness, 80 m at 500 Hz).
#[test]
fn a_lost_acceptance_of_the_turn_is_read_from_the_burst_after_it() {
    use aether_link::{ControlFrame, ControlKind, TxFrame, frames::control_flags};
    use std::cell::Cell;
    use std::rc::Rc;
    for taken_heard in [false, true] {
        let t = timing(false);
        let config = LinkConfig {
            offer_turn: true,
            ..LinkConfig::default()
        };
        let (mut a, mut b) = pair(&t, &config);
        let after_taken = Rc::new(Cell::new(0u8));
        let left = Rc::clone(&after_taken);
        let offset = move |frame: &TxFrame| -> f64 {
            if frame.container == Container::Control {
                if ControlFrame::decode(&frame.payload).is_ok_and(|c| {
                    c.kind == ControlKind::Ack && c.flags & control_flags::TAKEN != 0
                }) {
                    left.set(2);
                    return if taken_heard { 0.0 } else { -60.0 };
                }
            } else if left.get() > 0 {
                left.set(left.get() - 1);
                return -60.0;
            }
            0.0
        };
        let reply: Vec<u8> = "REPLY FROM KK4XYZ. ".repeat(20).into_bytes();
        a.connect("KK4XYZ").expect("idle");
        a.send(&[0u8; 300]);
        b.send(&reply);
        let mut sim = TwoStationSim::new(a, b, 12.0, 5).with_frame_snr_offset(Box::new(offset));
        sim.run(300.0, 3.0);
        assert_eq!(
            sim.delivered(1),
            [0u8; 300].as_slice(),
            "heard {taken_heard}"
        );
        assert_eq!(sim.delivered(0), reply.as_slice(), "heard {taken_heard}");
        assert_eq!(sim.engine(1).stats.turns_taken, 1, "heard {taken_heard}");
        assert_eq!(sim.collisions(0, 1), 0, "heard {taken_heard}");
        assert_eq!(after_taken.get(), 0);
    }
}

/// The scenario harness, 40 m with a receiver that takes 280 ms to come back after transmitting
/// (ADR-0047): the other station answered each acknowledgement a turnaround later, its one data
/// frame fell in the deafness, and only the offer after it was heard — which drew an
/// acknowledgement at once, and the frame went again into the same deafness, for the rest of the
/// session. An acknowledgement of an offer that measured none of the burst's data takes the offer
/// off the next burst, so nothing answers it at once and its retry is heard.
#[test]
fn an_offer_is_not_answered_into_a_receiver_still_coming_back() {
    for size in [100usize, 1500] {
        let t = timing(true);
        let (a, b) = pair(&t, &LinkConfig::default());
        let mut sim = TwoStationSim::new(a, b, 12.0, 1).with_rx_recovery(1, 0.3);
        sim.engine_mut(0).connect("KK4XYZ").expect("idle");
        let connected_by = sim.run(60.0, 1e9);
        assert_eq!(sim.engine(0).state(), State::Connected, "{connected_by}");
        let message = vec![0u8; size];
        sim.engine_mut(0).send(&message);
        sim.run(connected_by + 120.0, 1e9);
        assert_eq!(sim.delivered(1), message.as_slice(), "{size} bytes");
    }
}

#[test]
fn what_a_session_did_not_carry_does_not_go_in_the_next() {
    // the scenario harness (2026-10-08): a client vanished mid-transfer, both stations timed
    // out, and its next session opened by sending the 18 kB the dead one had left queued. A
    // call that fails, and a session that ends, take what they did not carry with them.
    let t = timing(false);
    let (mut a, b) = pair(&t, &LinkConfig::default());
    a.connect("N0BODY").expect("idle");
    a.send(b"for N0BODY only");
    let mut sim = TwoStationSim::new(a, b, 15.0, 31);
    sim.run(200.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(0).tx_undelivered_bytes(), 0);
    // a session cut short in the middle of a transfer
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    let now = sim.t;
    sim.run(now + 60.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Connected);
    let block: Vec<u8> = (0..=255u8).cycle().take(256 * 40).collect();
    sim.engine_mut(0).send(&block);
    let now = sim.t;
    sim.run(now + 8.0, 1e9);
    sim.engine_mut(0).abort();
    let now = sim.t;
    sim.run(now + 120.0, 1e9);
    assert_eq!(sim.engine(0).state(), State::Idle);
    assert_eq!(sim.engine(1).state(), State::Idle, "{:?}", sim.events(1));
    assert_eq!(sim.engine(0).tx_undelivered_bytes(), 0);
    let carried = sim.delivered(1).len();
    // the next session carries what it was given, and nothing else
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    let now = sim.t;
    sim.run(now + 60.0, 1e9);
    sim.engine_mut(0).send(b"the next message");
    sim.engine_mut(0).disconnect();
    let now = sim.t;
    sim.run(now + 200.0, 1e9);
    assert_eq!(&sim.delivered(1)[carried..], b"the next message");
}

#[test]
fn a_station_that_left_answers_its_old_session_with_a_disc() {
    // ADR-0052: a receiving station aborts in the middle of the other's burst, its DISC goes
    // while the sender transmits and is never heard. With no repeats of it, the left station
    // answering the sender's next burst with a DISC ends the session long before the
    // sender's link timeout.
    let t = timing(false);
    let config = LinkConfig {
        leave_repeats: 0,
        ..LinkConfig::default()
    };
    let (mut a, b) = pair(&t, &config);
    a.connect("KK4XYZ").expect("idle");
    let mut sim = TwoStationSim::new(a, b, 15.0, 32);
    sim.run(40.0, 1e9);
    assert_eq!(sim.engine(1).state(), State::Connected);
    let block: Vec<u8> = (0..=255u8).cycle().take(256 * 60).collect();
    sim.engine_mut(1).send(&block);
    for step in 0..2000 {
        sim.run(40.0 + 0.1 * f64::from(step + 1), 1e9);
        if sim.engine(1).role() == Role::Iss && sim.engine(1).transmitting() {
            break;
        }
    }
    assert!(sim.engine(1).role() == Role::Iss && sim.engine(1).transmitting());
    sim.engine_mut(0).abort();
    let started = sim.t;
    sim.run(started + 90.0, 1e9);
    assert_eq!(sim.engine(1).state(), State::Idle, "{:?}", sim.events(1));
    assert!(
        sim.events(1)
            .iter()
            .any(|e| e == "disconnected:peer disconnected"),
        "{:?}",
        sim.events(1)
    );
    assert!(sim.engine(0).stats.left_answered >= 1);
    assert!(sim.t - started < sim.engine(1).link_timeout_s());
}

/// A connect request handed straight to an engine, heard at `snr_db` at `t`.
struct Called {
    payload: Vec<u8>,
    mode: usize,
    snr_db: f64,
    t: f64,
}

impl aether_link::SoftFrame for Called {
    fn container(&self) -> aether_link::Container {
        aether_link::Container::Data
    }
    fn mode(&self) -> usize {
        self.mode
    }
    fn floor(&self) -> bool {
        false
    }
    fn rv(&self) -> u8 {
        0
    }
    fn snr_db(&self) -> f64 {
        self.snr_db
    }
    fn t_start(&self) -> f64 {
        self.t
    }
    fn t_end(&self) -> f64 {
        self.t + 0.5
    }
    fn decode(
        &self,
        _buffer: Option<&aether_link::HarqBuffer>,
    ) -> (Option<Vec<u8>>, aether_link::HarqBuffer) {
        (Some(self.payload.clone()), Vec::new())
    }
}

/// A call from `caller` to `called` in the ordinary family under `session`.
fn call_frame(
    t: &PhyTiming,
    caller: &str,
    called: &str,
    session: u8,
    snr_db: f64,
    at: f64,
) -> Called {
    use aether_link::frames::{ConnectBody, DataHeader, DataKind, PROTOCOL_VERSION, encode_data};
    let mode = air_interface(WIDE_2300).control_rung();
    let body = ConnectBody {
        src: caller.into(),
        dst: called.into(),
        caps: LinkConfig::default().capabilities,
        version: PROTOCOL_VERSION,
        snr_db: None,
    }
    .encode()
    .expect("body");
    let header = DataHeader {
        kind: DataKind::ConnectReq,
        seq: 0,
        session,
    };
    let payload = encode_data(&header, &body, t.capacity(mode)).expect("frame");
    Called {
        payload,
        mode,
        snr_db,
        t: at,
    }
}

fn modes_transmitted(engine: &mut LinkEngine) -> Vec<usize> {
    engine
        .drain()
        .into_iter()
        .filter_map(|action| match action {
            aether_link::Action::Transmit { frames, .. } => Some(frames),
            _ => None,
        })
        .flatten()
        .map(|frame| frame.mode)
        .collect()
}

#[test]
fn a_weak_call_is_accepted_on_the_floor() {
    // a call heard in the ordinary family near its threshold is accepted on the tone floor:
    // the acceptance is the frame a session cannot do without, and KE4QCM's ordinary ones,
    // answering calls heard at +1…+6.5 dB, never reached him (ADR-0056); a strong call is
    // answered in its own family, as before
    let t = timing(false);
    for (snr_db, floor) in [(2.0, true), (6.5, true), (20.0, false)] {
        let mut b = LinkEngine::new("KK4XYZ", t.clone(), LinkConfig::default(), 2);
        b.on_frame(&call_frame(&t, "W4ODA", "KK4XYZ", 7, snr_db, 1.0), 2.0);
        assert_eq!(b.state(), State::Connected);
        let sent = modes_transmitted(&mut b);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(t.is_floor(sent[0]), floor, "{snr_db} dB: {sent:?}");
    }
}

#[test]
fn a_new_call_from_the_peer_replaces_its_dead_session() {
    // a station that calls again under a new session number has given up the one this station
    // accepted — one station is in one session — so the session ends and the new call is
    // answered; taken for another session's frame, KE4QCM's second call went unanswered while
    // this station waited out the first session's link timeout (ADR-0056)
    let t = timing(false);
    let mut b = LinkEngine::new("KK4XYZ", t.clone(), LinkConfig::default(), 2);
    b.on_frame(&call_frame(&t, "W4ODA", "KK4XYZ", 7, 20.0, 1.0), 2.0);
    assert_eq!((b.state(), b.session()), (State::Connected, 7));
    b.drain();
    b.on_frame(&call_frame(&t, "W4ODA", "KK4XYZ", 9, 20.0, 20.0), 21.0);
    assert_eq!((b.state(), b.session()), (State::Connected, 9));
    let actions = b.drain();
    assert!(
        actions.iter().any(|action| matches!(action,
            aether_link::Action::Event { name, detail } if *name == "disconnected" && detail == "W4ODA called again")),
        "{actions:?}"
    );
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, aether_link::Action::Transmit { .. }))
            .count(),
        1
    );
    // a call from anybody else under another session number is no business of this one
    b.on_frame(&call_frame(&t, "N0CALL", "KK4XYZ", 11, 20.0, 40.0), 41.0);
    assert_eq!((b.state(), b.session()), (State::Connected, 9));
    assert_eq!(b.remote_call, "W4ODA");
}

#[test]
fn a_call_moved_to_the_narrow_air_is_answered_by_a_narrow_station() {
    // a 2 300 Hz call is ignored by a 500 Hz station; the caller that moves its call to the
    // narrow air after unanswered tries is answered, and the session runs there (ADR-0058)
    let wide = LinkConfig {
        capabilities: aether_link::frames::with_bandwidth(0, 2300),
        ..LinkConfig::default()
    };
    let narrow = LinkConfig {
        capabilities: aether_link::frames::with_bandwidth(0, 500),
        ..LinkConfig::default()
    };
    let a = LinkEngine::new("W4ODA", timing(false), wide.clone(), 1);
    let b = LinkEngine::new("KK4XYZ", air_timing(NARROW_500, false), narrow, 2);
    let mut sim = TwoStationSim::new(a, b, 6.0, 41);
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    let mut t = 0.0;
    while sim.engine(0).connect_tries() < 2 && t < 120.0 {
        t += 0.5;
        sim.run(t, 1000.0);
    }
    assert_eq!(sim.engine(1).state(), State::Idle);
    assert!(
        sim.engine_mut(1)
            .move_call(timing(false), wide.capabilities, None)
            .is_err()
    );
    sim.engine_mut(0)
        .move_call(
            air_timing(NARROW_500, false),
            aether_link::frames::with_bandwidth(0, 500),
            None,
        )
        .expect("calling");
    sim.engine_mut(0).send(b"moved");
    sim.run(600.0, 1000.0);
    assert_eq!(sim.delivered(1), b"moved", "{:?}", sim.events(1));
}

#[test]
fn acknowledgements_go_on_the_floor_once_one_was_lost() {
    // a burst of nothing but blocks already acknowledged says the acknowledgement was lost;
    // the receiver's acknowledgements go on the tone floor from then on (ADR-0059). 500 Hz at
    // 0 dB with the receiver's short ordinary control frames 8 dB under the channel, as
    // KK4ODA-1's were at WC4Y on 2026-10-10
    let t = air_timing(NARROW_500, false);
    let config = LinkConfig::default();
    let a = LinkEngine::new("W4ODA", t.clone(), config.clone(), 1);
    let b = LinkEngine::new("KK4XYZ", t, config, 2);
    let mut sim = TwoStationSim::new(a, b, 0.0, 59).with_frame_snr_offset(Box::new(|frame| {
        if frame.container == aether_link::Container::Control && !frame.floor {
            -8.0
        } else {
            0.0
        }
    }));
    let message: Vec<u8> = (0..2048u32).map(|i| (i % 251) as u8).collect();
    sim.engine_mut(0).connect("KK4XYZ").expect("idle");
    sim.engine_mut(0).send(&message);
    sim.engine_mut(0).disconnect();
    sim.run(3000.0, 3.0);
    assert_eq!(sim.delivered(1), message.as_slice());
    assert!(
        sim.events(1)
            .iter()
            .any(|e| e == "acks:on the floor: W4ODA missed an acknowledgement"),
        "{:?}",
        sim.events(1)
    );
}
