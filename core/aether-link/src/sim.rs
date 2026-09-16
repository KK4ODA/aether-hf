//! Two-engine discrete-event simulator over a lossy pipe (roadmap P3-2).
//!
//! This is the *fast* harness: no DSP. A single half-duplex channel connects two
//! [`LinkEngine`] stations; frames are delivered one at a time with a per-mode error model
//! and an energy-accumulation stand-in for HARQ-IR, so the protocol's timers, selective
//! repeat, HARQ inference, turnaround and handshakes can be exercised in milliseconds over
//! thousands of seeded trials. The slow harness — the same engines over the real physical
//! layer and channel simulator — belongs with the daemon.
//!
//! # Channel model
//!
//! * Half-duplex: a station hears a frame only if it was not transmitting for any part of it.
//! * Simultaneous transmissions collide and both frames are lost (this is what makes the
//!   connect race a real race).
//! * Loss: each frame draws once from a uniform; it is lost if the draw exceeds the mode's
//!   success probability at the channel SNR. A retransmission of the same block accumulates
//!   about 3 dB of "energy" per combine, and the combined frame succeeds once the summed
//!   energy clears the mode's threshold — the same qualitative behaviour as LDPC HARQ-IR.

use std::{cmp::Ordering, collections::BinaryHeap};

use crate::{
    engine::{Action, LinkEngine},
    phy::{Container, HarqBuffer, SoftFrame, TxFrame},
    rate::AWGN_THRESHOLD_DB,
};

/// Logistic steepness of frame error rate against SNR, in dB; larger is a sharper waterfall.
const STEEP: f64 = 1.2;
/// Soft-combining energy gained per retransmission of the same block, in dB.
const HARQ_GAIN_DB: f64 = 3.0;

fn success_prob(mode: usize, snr_db: f64, energy_db: f64, thresholds: &[f64; 14]) -> f64 {
    let threshold = thresholds.get(mode).copied().unwrap_or(0.0);
    1.0 / (1.0 + (-STEEP * (snr_db + energy_db - threshold)).exp())
}

/// A frame as delivered to the receiving engine.
///
/// The HARQ buffer is the accumulated combining energy in dB, as a one-element vector;
/// `decode` returns the new energy so the engine can carry it to the next retransmission.
#[derive(Debug, Clone)]
pub struct SimFrame {
    container: Container,
    mode: usize,
    rv: u8,
    snr_db: f64,
    t_start: f64,
    t_end: f64,
    payload: Vec<u8>,
    draw: f64,
    thresholds: [f64; 14],
    floor: bool,
}

impl SimFrame {
    /// A frame that decodes: a test's way of handing an engine a frame of its own making
    /// (a probe from a third station, say) at a stated SNR.
    #[must_use]
    pub fn decoded(
        container: Container,
        mode: usize,
        snr_db: f64,
        t_start: f64,
        t_end: f64,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            container,
            mode,
            rv: 0,
            snr_db,
            t_start,
            t_end,
            payload,
            draw: 0.0,
            thresholds: AWGN_THRESHOLD_DB,
            floor: false,
        }
    }
}

impl SoftFrame for SimFrame {
    fn container(&self) -> Container {
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
        self.snr_db
    }

    fn t_start(&self) -> f64 {
        self.t_start
    }

    fn t_end(&self) -> f64 {
        self.t_end
    }

    fn decode(&self, buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
        let prior = buffer.and_then(|b| b.first().copied()).unwrap_or(0.0);
        let gained = prior + if buffer.is_some() { HARQ_GAIN_DB } else { 0.0 };
        if self.draw <= success_prob(self.mode, self.snr_db, gained, &self.thresholds) {
            (Some(self.payload.clone()), vec![gained])
        } else {
            (None, vec![gained])
        }
    }
}

/// A small deterministic generator, so a seeded run is reproducible without a dependency.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(
            seed.wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407)
                | 1,
        )
    }

    fn next_unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Debug, Clone)]
enum EvKind {
    Arrive { frame: TxFrame, t0: f64, t1: f64 },
    Preamble { t0: f64 },
    TxDone,
}

#[derive(Debug)]
struct Ev {
    t: f64,
    seq: u64,
    who: usize,
    kind: EvKind,
}

// `BinaryHeap` is a max-heap, so the ordering is reversed to pop the earliest event first.
// Ties break on insertion order, which keeps a run reproducible.
impl Ord for Ev {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .t
            .total_cmp(&self.t)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Ev {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Ev {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ev {}

/// One side of the simulated link.
struct Station {
    engine: LinkEngine,
    delivered: Vec<u8>,
    events: Vec<String>,
    tx_end: f64,
    busy: Vec<(f64, f64)>,
}

impl Station {
    fn new(engine: LinkEngine) -> Self {
        Self {
            engine,
            delivered: Vec::new(),
            events: Vec::new(),
            tx_end: 0.0,
            busy: Vec::new(),
        }
    }
}

/// Two engines driven to quiescence over a lossy half-duplex pipe.
pub struct TwoStationSim {
    stations: [Station; 2],
    snr_db: f64,
    prop_s: f64,
    rng: Rng,
    queue: BinaryHeap<Ev>,
    seq: u64,
    /// Current simulation time.
    pub t: f64,
    thresholds: [f64; 14],
    snr_schedule: Option<Box<dyn Fn(f64) -> f64>>,
    /// The mode of every DATA frame put on the pipe, in order: what the rate control did.
    modes_sent: Vec<usize>,
}

impl TwoStationSim {
    /// Connect two stations over a channel at a fixed SNR.
    #[must_use]
    pub fn new(a: LinkEngine, b: LinkEngine, snr_db: f64, seed: u64) -> Self {
        Self {
            stations: [Station::new(a), Station::new(b)],
            snr_db,
            prop_s: 0.01,
            rng: Rng::new(seed),
            queue: BinaryHeap::new(),
            seq: 0,
            t: 0.0,
            thresholds: AWGN_THRESHOLD_DB,
            modes_sent: Vec::new(),
            snr_schedule: None,
        }
    }

    /// Use per-mode thresholds other than the AWGN table — a fading channel, say.
    #[must_use]
    pub fn with_thresholds(mut self, thresholds: [f64; 14]) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Make the SNR a function of time, for ramps. Overrides the fixed SNR.
    #[must_use]
    pub fn with_snr_schedule(mut self, schedule: Box<dyn Fn(f64) -> f64>) -> Self {
        self.snr_schedule = Some(schedule);
        self
    }

    /// One-way propagation and audio delay.
    #[must_use]
    pub fn with_propagation(mut self, prop_s: f64) -> Self {
        self.prop_s = prop_s;
        self
    }

    /// Change the channel SNR mid-run.
    pub fn set_snr(&mut self, snr_db: f64) {
        self.snr_db = snr_db;
    }

    /// The channel SNR at a given time.
    #[must_use]
    pub fn snr_at(&self, t: f64) -> f64 {
        self.snr_schedule
            .as_ref()
            .map_or(self.snr_db, |schedule| schedule(t))
    }

    /// Bytes a station has delivered to its application.
    #[must_use]
    pub fn delivered(&self, who: usize) -> &[u8] {
        &self.stations[who].delivered
    }

    /// The mode of every DATA frame sent, in order.
    #[must_use]
    pub fn modes_sent(&self) -> &[usize] {
        &self.modes_sent
    }

    /// Events a station reported, as `name:detail`.
    #[must_use]
    pub fn events(&self, who: usize) -> &[String] {
        &self.stations[who].events
    }

    /// A station's engine, for inspection or commands mid-run.
    #[must_use]
    pub fn engine(&self, who: usize) -> &LinkEngine {
        &self.stations[who].engine
    }

    /// A station's engine, mutably.
    pub fn engine_mut(&mut self, who: usize) -> &mut LinkEngine {
        &mut self.stations[who].engine
    }

    // ── scheduling ────────────────────────────────────────────────────

    fn push(&mut self, t: f64, who: usize, kind: EvKind) {
        let seq = self.seq;
        self.seq += 1;
        self.queue.push(Ev { t, seq, who, kind });
    }

    fn pump(&mut self, who: usize, at: f64) {
        for action in self.stations[who].engine.drain() {
            match action {
                Action::Transmit { frames, duration_s } => {
                    self.launch(who, at, frames, duration_s);
                }
                Action::Deliver(data) => self.stations[who].delivered.extend_from_slice(&data),
                Action::Event { name, detail } => {
                    self.stations[who].events.push(format!("{name}:{detail}"));
                }
            }
        }
    }

    fn launch(&mut self, who: usize, at: f64, frames: Vec<TxFrame>, duration_s: f64) {
        let mut t = at.max(self.stations[who].tx_end);
        self.stations[who].busy.push((t, t + duration_s));
        let timing = self.stations[who].engine.timing();
        let sof = timing.preamble_detect_s;
        let timing = timing.clone();
        let peer = 1 - who;
        for frame in frames {
            let duration = timing.frame_s(&frame);
            if let Some(sof) = sof
                && frame.container == Container::Data
            {
                // acquisition succeeds far below every mode's decode threshold (P2-3 measured
                // 100 % at −5 dB), so a listening receiver is assumed to see every preamble
                self.push(t + sof, peer, EvKind::Preamble { t0: t });
            }
            self.push(
                t + duration,
                peer,
                EvKind::Arrive {
                    frame,
                    t0: t,
                    t1: t + duration,
                },
            );
            t += duration;
        }
        self.stations[who].tx_end = t;
        self.push(t, who, EvKind::TxDone);
    }

    fn busy(&self, who: usize, t0: f64, t1: f64) -> bool {
        self.stations[who]
            .busy
            .iter()
            .any(|&(a, b)| a < t1 - 1e-9 && t0 + 1e-9 < b)
    }

    fn deliver(&mut self, rx: usize, frame: &TxFrame, t0: f64, t1: f64) {
        if frame.container == Container::Data {
            self.modes_sent.push(frame.mode);
        }
        if self.busy(rx, t0, t1) {
            return; // half-duplex, or a collision: the receiver was transmitting
        }
        let arrival = t1 + self.prop_s;
        // a data frame's family is its mode's; a control frame says which it went out on
        let floor = match frame.container {
            Container::Data => self.stations[rx].engine.timing().is_floor(frame.mode),
            Container::Control => frame.floor,
        };
        let sim = SimFrame {
            container: frame.container,
            mode: frame.mode,
            rv: frame.rv,
            snr_db: self.snr_at(f64::midpoint(t0, t1)),
            t_start: t0 + self.prop_s,
            t_end: arrival,
            payload: frame.payload.clone(),
            draw: self.rng.next_unit(),
            thresholds: self.thresholds,
            floor,
        };
        self.stations[rx].engine.tick(arrival);
        self.stations[rx].engine.on_frame(&sim, arrival);
        self.pump(rx, arrival);
    }

    /// Tell a listening receiver a frame's preamble was detected.
    fn announce(&mut self, rx: usize, t_start: f64, at: f64) {
        if self.busy(rx, t_start, at) {
            return;
        }
        self.stations[rx].engine.tick(at);
        self.stations[rx]
            .engine
            .on_preamble(t_start + self.prop_s, at);
        self.pump(rx, at);
    }

    // ── run loop ──────────────────────────────────────────────────────

    /// Advance until both engines have been idle for `idle_gap` seconds, or `until` is
    /// reached. Returns the final simulation time.
    pub fn run(&mut self, until: f64, idle_gap: f64) -> f64 {
        for who in 0..2 {
            self.pump(who, self.t);
        }
        let mut last = 0.0;
        while self.t < until {
            let next_queued = self.queue.peek().map_or(f64::INFINITY, |ev| ev.t);
            let next = (0..2)
                .filter_map(|w| self.stations[w].engine.next_deadline())
                .fold(next_queued, f64::min);
            if !next.is_finite() || next > until {
                break;
            }
            self.t = next;
            let mut progressed = false;
            for who in 0..2 {
                let due = self.stations[who]
                    .engine
                    .next_deadline()
                    .is_some_and(|dl| dl <= next + 1e-9);
                if due {
                    self.stations[who].engine.tick(next);
                    self.pump(who, next);
                    progressed = true;
                }
            }
            while let Some(ev) = self.queue.pop() {
                if ev.t > next + 1e-9 {
                    self.queue.push(ev); // not due yet: put it back and stop
                    break;
                }
                match ev.kind {
                    EvKind::Arrive { frame, t0, t1 } => self.deliver(ev.who, &frame, t0, t1),
                    EvKind::Preamble { t0 } => self.announce(ev.who, t0, ev.t),
                    EvKind::TxDone => {
                        self.stations[ev.who].engine.on_tx_done(next);
                        self.pump(ev.who, next);
                    }
                }
                progressed = true;
            }
            if progressed {
                last = self.t;
            } else if self.t - last > idle_gap {
                break;
            }
        }
        self.t
    }
}

impl std::fmt::Debug for TwoStationSim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TwoStationSim")
            .field("t", &self.t)
            .field("snr_db", &self.snr_db)
            .field("queued", &self.queue.len())
            .finish_non_exhaustive()
    }
}
