//! The session state machine over the lossy-pipe simulator (roadmap P3-2).
//!
//! These mirror `model/tests/test_link.py`. They are behavioural rather than bit-exact:
//! the lossy pipe is a stochastic model, and two implementations of it are not required to
//! draw the same random numbers. What must agree is what the protocol *does* — that a
//! transfer completes, that the rate controller follows a fade, that a break hands the
//! channel over, that HARQ rescues frames below threshold. The bit-exact comparison against
//! the model is in `model_vectors.rs`.

use aether_link::{
    LinkConfig, LinkEngine, PhyTiming, Role, State, TwoStationSim, rate::PAYLOAD_BYTES,
};
use aether_phy::modes::{LONG, SHORT};

/// Timing from the real waveform tables, optionally with a start-of-frame signal.
fn timing(start_of_frame: bool) -> PhyTiming {
    PhyTiming {
        data_frame_s: LONG.duration_s(),
        control_frame_s: SHORT.duration_s(),
        turnaround_s: 0.25,
        detect_latency_s: 0.15,
        tx_latency_s: 0.0,
        preamble_detect_s: start_of_frame.then(|| 4.0 * LONG.waveform.symbol_period_s()),
        data_capacity: PAYLOAD_BYTES.to_vec(),
    }
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
    // pinned to QPSK 1/2 (threshold about +1 dB) and driven at −3 dB: the transfer only
    // completes because retransmissions are combined with what came before
    let config = LinkConfig {
        initial_mode: 4,
        max_mode: 4,
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
}

impl Wire {
    fn carry(frame: &aether_link::TxFrame, t_start: f64, t: &PhyTiming) -> Self {
        let duration = match frame.container {
            aether_link::Container::Data => t.data_frame_s,
            aether_link::Container::Control => t.control_frame_s,
        };
        Self {
            container: frame.container,
            mode: frame.mode,
            rv: frame.rv,
            t_start,
            t_end: t_start + duration,
            payload: frame.payload.clone(),
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
