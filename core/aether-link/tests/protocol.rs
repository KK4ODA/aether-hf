//! The session state machine over the lossy-pipe simulator (roadmap P3-2).
//!
//! These mirror `model/tests/test_link.py`. They are behavioural rather than bit-exact:
//! the lossy pipe is a stochastic model, and two implementations of it are not required to
//! draw the same random numbers. What must agree is what the protocol *does* — that a
//! transfer completes, that the rate controller follows a fade, that a break hands the
//! channel over, that HARQ rescues frames below threshold. The bit-exact comparison against
//! the model is in `model_vectors.rs`.

use aether_link::{
    LinkConfig, LinkEngine, PhyTiming, Role, State, TwoStationSim,
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
    assert_eq!(sim.engine(0).stats.turns, 1);
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
fn a_probe_is_not_answered_during_a_session_or_in_another_bandwidth() {
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
    assert!(sim.engine_mut(1).drain().is_empty());
    // idle, but the probe claims another bandwidth: ignored, and said so
    sim.engine_mut(0).disconnect();
    sim.run(300.0, 3.0);
    assert_eq!(sim.engine(1).state(), State::Idle);
    let now = sim.t;
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "KK4XYZ", with_bandwidth(0, 500)), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 0);
    let actions = sim.engine_mut(1).drain();
    assert!(
        actions.iter().any(|action| matches!(
            action,
            Action::Event { name: "ignored", detail } if detail.contains("N0CALL")
        )),
        "{actions:?}"
    );
    // and one for somebody else is nobody's business
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "W1AW", 0), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 0);
    assert!(sim.engine_mut(1).drain().is_empty());
    // while one addressed to it, in its bandwidth, is answered
    sim.engine_mut(1)
        .on_frame(&probe_from("N0CALL", "KK4XYZ", 0), now);
    assert_eq!(sim.engine(1).stats.probes_answered, 1);
    let actions = sim.engine_mut(1).drain();
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, Action::Transmit { .. }))
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

    // sample the sender's mode as the run proceeds, rather than instrumenting the engine
    let mut track: Vec<usize> = Vec::new();
    while sim.t < 3000.0 {
        let before = sim.t;
        sim.run(before + 5.0, 3.0);
        track.push(sim.engine(0).current_mode());
        if (sim.t - before).abs() < 1e-9 {
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
    // silence to know a burst has ended, which is about a quarter of the air time
    let message = vec![0u8; 16000];
    let mut times = Vec::new();
    for start_of_frame in [false, true] {
        let t = timing(start_of_frame);
        let (mut a, b) = pair(&t, &LinkConfig::default());
        a.connect("KK4XYZ").expect("idle");
        a.send(&message);
        a.disconnect();
        let mut sim = TwoStationSim::new(a, b, 14.0, 5);
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
    assert!(sim.engine_mut(0).take_ladder().is_empty());
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
    // version 4 of the link protocol numbers the 500 Hz ladder's rungs with its middle kinds
    // (ADR-0015), version 3 the 2 300 Hz one's with the fast kinds (ADR-0014), version 2 the
    // ladders before them (ADR-0013), version 1 OFDM modes: a station of another version means
    // other frames by the same numbers, so a call from one is not a session to start — it is
    // ignored, with an event saying why
    assert_eq!(PROTOCOL_VERSION, 4);
    for version in [1u8, 2, 3] {
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
