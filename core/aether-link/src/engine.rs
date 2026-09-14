//! ARQ engine and session state machine (roadmap P3-2).
//!
//! One [`LinkEngine`] per station. It is a pure state machine driven by three inputs —
//! [`on_frame`](LinkEngine::on_frame) (a detected frame, decoded or not),
//! [`on_tx_done`](LinkEngine::on_tx_done) and [`tick`](LinkEngine::tick) (time) — and it emits
//! [`Action`]s: frames to transmit, bytes to deliver, events to report. No threads, no clock
//! of its own, no physical layer: the same engine runs under the lossy-pipe simulator and,
//! later, the real modem.
//!
//! # Protocol in one paragraph
//!
//! A session has one *information sending station* (ISS) and one *information receiving
//! station* (IRS). The ISS transmits bursts of up to 16 data frames back to back; the IRS
//! answers every burst with one short acknowledgement carrying a selective-repeat bitmap, the
//! SNR it measured and the mode it recommends. The ISS retransmits what the bitmap says is
//! missing — the same codeword under the next redundancy version, which the IRS soft-combines
//! — and fills the rest of the burst with new frames at the recommended mode. A `Turn` hands
//! the sending role to the peer (sent when the peer flagged `WANT_TX` or `BREAK`); `Poll`
//! keeps an idle link alive; `Disc` / `DiscAck` end it. Connecting is a two-way handshake in
//! data-container frames carrying both callsigns; the caller's first burst or poll confirms it.
//!
//! # HARQ without a decoded header
//!
//! A failed frame has no readable sequence number, so the IRS infers it: the ISS orders every
//! burst as *unacknowledged frames ascending, then new frames*, and the IRS knows what it has
//! acknowledged — so it can predict the burst from either of its last two acknowledgements
//! (the ISS may have missed the latest one) and map each frame's *slot*, taken from its air
//! time, to a sequence number; frames that did decode anchor the mapping. A wrong guess only
//! costs a wasted combine — the check never lets mismatched information through, and a
//! standalone decode is always tried as well.

use crate::{
    frames::{
        CONNECT_BODY_BYTES, ConnectBody, ControlFrame, ControlKind, DataHeader, DataKind,
        MAX_BURST, WINDOW, control_flags, data_capacity, decode_data, encode_data, in_window,
        seq_after, seq_distance,
    },
    phy::{Container, HarqBuffer, PhyTiming, SoftFrame, TxFrame},
    rate::RateController,
};

// ── configuration, actions, states ────────────────────────────────────

/// Tuning for a session.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkConfig {
    /// Frames per burst (at most [`MAX_BURST`]). Longer bursts amortise the turnaround.
    pub burst_frames: usize,
    /// Consecutive unanswered bursts or polls before the link is declared dead.
    pub max_retries: usize,
    /// Connection attempts before giving up.
    pub connect_retries: usize,
    /// Turn attempts before carrying on as the sender.
    pub turn_retries: usize,
    /// Disconnect attempts before closing anyway.
    pub disc_retries: usize,
    /// An idle sender polls the receiver this often.
    pub keepalive_s: f64,
    /// No valid frame from the peer for this long ends the session.
    pub link_timeout_s: f64,
    /// Slack added to every wait for a peer response.
    pub ack_margin_s: f64,
    /// Silence after a data frame that marks the end of a burst.
    pub burst_gap_s: f64,
    /// Mode a session starts on.
    pub initial_mode: usize,
    /// Fastest mode this station will use.
    pub max_mode: usize,
    /// With a peer that wants to send, hand over after this many bursts of our own.
    pub bursts_before_turn: usize,
    /// HARQ buffers are reset after this many failed combines, which bounds a wrong guess.
    pub max_combines: usize,
    /// Capability bits offered in the connect handshake. What they mean is the caller's
    /// business; the link layer carries them and reports what the peer offered.
    pub capabilities: u8,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            burst_frames: 6,
            max_retries: 8,
            connect_retries: 8,
            turn_retries: 3,
            disc_retries: 3,
            keepalive_s: 10.0,
            link_timeout_s: 45.0,
            ack_margin_s: 0.4,
            burst_gap_s: 0.2,
            initial_mode: 0,
            max_mode: 13,
            bursts_before_turn: 3,
            max_combines: 4,
            capabilities: 0,
        }
    }
}

/// Where a session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No session.
    Idle,
    /// Calling, waiting for an acceptance.
    Connecting,
    /// In session.
    Connected,
    /// Closing.
    Disconnecting,
}

/// Which half of a session this station is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Not in a session.
    None,
    /// Information sending station.
    Iss,
    /// Information receiving station.
    Irs,
}

/// Something the caller must do, or be told.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Transmit these frames back to back; they occupy `duration_s` of air time.
    Transmit {
        /// The frames, in order.
        frames: Vec<TxFrame>,
        /// Total air time.
        duration_s: f64,
    },
    /// Deliver these payload bytes to the application, in order.
    Deliver(Vec<u8>),
    /// Report something that happened.
    Event {
        /// What happened: `connected`, `disconnected` or `role`.
        name: &'static str,
        /// Detail for a human.
        detail: String,
    },
}

/// Counters a caller can show an operator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkStats {
    /// Data frames put on the air.
    pub frames_sent: usize,
    /// Of those, retransmissions.
    pub frames_resent: usize,
    /// Data frames received and accepted.
    pub frames_received: usize,
    /// Data frames that did not decode.
    pub frames_failed: usize,
    /// Frames recovered only by combining with an earlier transmission.
    pub harq_rescues: usize,
    /// Acknowledgements sent.
    pub acks_sent: usize,
    /// Acknowledgements received.
    pub acks_received: usize,
    /// Waits for an acknowledgement that expired.
    pub ack_timeouts: usize,
    /// Bursts transmitted.
    pub bursts: usize,
    /// Turns taken.
    pub turns: usize,
    /// Payload bytes delivered to the application.
    pub bytes_delivered: usize,
}

#[derive(Debug, Clone)]
struct TxRecord {
    seq: u8,
    kind: DataKind,
    body: Vec<u8>,
    mode: usize,
    tx_count: usize,
    acked: bool,
}

struct RxRecord {
    slot: usize,
    mode: usize,
    snr_db: f64,
    payload: Option<Vec<u8>>,
    /// Sequence number if decoded, else the inferred guess (which may be absent).
    seq: Option<u8>,
}

#[derive(Debug, Clone)]
struct AckSnapshot {
    /// Unreceived sequence numbers between the base and the highest seen, ascending.
    missing: Vec<u8>,
    next_new: u8,
}

/// Timers the engine arms, addressed by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timer {
    Link,
    Connect,
    Ack,
    Wait,
    Keepalive,
}

/// What the receiving station has asked for in its last acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerRequest {
    /// Nothing: it is happy to keep receiving.
    None,
    /// It has data of its own; hand over when this burst run is done.
    WantsTx,
    /// It wants the channel now, before the current run finishes.
    Break,
}

/// What the station is waiting for the peer to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Ack,
    Poll,
    Turn,
    Disc,
}

/// A small deterministic generator, so backoff is reproducible in tests without a dependency.
///
/// Jitter is all it is asked for; nothing here needs to be unguessable.
#[derive(Debug, Clone)]
struct Backoff(u64);

impl Backoff {
    fn new(seed: u64) -> Self {
        // scrambled so that neighbouring seeds do not produce neighbouring streams; the low
        // bit is forced because xorshift has a fixed point at zero
        Self(
            seed.wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407)
                | 1,
        )
    }

    /// Uniform on `[0, 1)`.
    fn next_unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (value >> 11) as f64 / (1u64 << 53) as f64
    }
}

// ── the engine ────────────────────────────────────────────────────────

/// One station's ARQ engine.
pub struct LinkEngine {
    /// This station's callsign.
    pub my_call: String,
    /// The peer's callsign, once known.
    pub remote_call: String,
    /// Counters, for display.
    pub stats: LinkStats,
    timing: PhyTiming,
    config: LinkConfig,
    backoff: Backoff,
    state: State,
    role: Role,
    session: u8,
    now: f64,
    actions: Vec<Action>,
    rate: RateController,
    deadlines: Vec<(Timer, f64)>,
    tx_busy_until: f64,
    // sending side
    tx_queue: Vec<u8>,
    records: Vec<TxRecord>,
    tx_base: u8,
    tx_next: u8,
    retries: usize,
    bursts_since_turn: usize,
    peer_request: PeerRequest,
    recommended: usize,
    turn_tries: usize,
    disc_requested: bool,
    disc_tries: usize,
    connect_tries: usize,
    waiting_for: Option<Waiting>,
    // receiving side
    rx_base: u8,
    rx_buffer: Vec<(u8, Vec<u8>)>,
    max_seen: Option<u8>,
    harq: Vec<(u8, HarqBuffer, usize)>,
    burst: Vec<RxRecord>,
    burst_t0: Option<f64>,
    ack_history: Vec<AckSnapshot>,
    ack_counter: u8,
    break_requested: bool,
    peer_capabilities: u8,
}

impl std::fmt::Debug for LinkEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkEngine")
            .field("my_call", &self.my_call)
            .field("remote_call", &self.remote_call)
            .field("state", &self.state)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

impl LinkEngine {
    /// Build a station.
    #[must_use]
    pub fn new(my_call: &str, timing: PhyTiming, config: LinkConfig, seed: u64) -> Self {
        let recommended = config.initial_mode;
        Self {
            my_call: my_call.to_ascii_uppercase(),
            remote_call: String::new(),
            stats: LinkStats::default(),
            timing,
            config,
            backoff: Backoff::new(seed),
            state: State::Idle,
            role: Role::None,
            session: 0,
            now: 0.0,
            actions: Vec::new(),
            rate: RateController::default(),
            deadlines: Vec::new(),
            tx_busy_until: 0.0,
            tx_queue: Vec::new(),
            records: Vec::new(),
            tx_base: 0,
            tx_next: 0,
            retries: 0,
            bursts_since_turn: 0,
            peer_request: PeerRequest::None,
            recommended,
            turn_tries: 0,
            disc_requested: false,
            disc_tries: 0,
            connect_tries: 0,
            waiting_for: None,
            rx_base: 0,
            rx_buffer: Vec::new(),
            max_seen: None,
            harq: Vec::new(),
            burst: Vec::new(),
            burst_t0: None,
            ack_history: Vec::new(),
            ack_counter: 0,
            break_requested: false,
            peer_capabilities: 0,
        }
    }

    // ── inspection ────────────────────────────────────────────────────

    /// Where the session is.
    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// Which half of the session this station is.
    #[must_use]
    pub fn role(&self) -> Role {
        self.role
    }

    /// Session identifier.
    #[must_use]
    pub fn session(&self) -> u8 {
        self.session
    }

    /// Capability bits the peer offered in the connect handshake.
    ///
    /// Zero until a session is up, which is the safe reading: a station that has not said it
    /// can do something must be assumed not to be able to.
    #[must_use]
    pub fn peer_capabilities(&self) -> u8 {
        self.peer_capabilities
    }

    /// Whether a session is up.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.state == State::Connected
    }

    /// The mode the sender is currently using.
    #[must_use]
    pub fn current_mode(&self) -> usize {
        self.recommended
    }

    /// Bytes queued, or sent but not yet acknowledged.
    #[must_use]
    pub fn tx_pending_bytes(&self) -> usize {
        self.tx_queue.len()
            + self
                .records
                .iter()
                .filter(|r| !r.acked)
                .map(|r| r.body.len())
                .sum::<usize>()
    }

    /// The timing this engine was built with.
    #[must_use]
    pub fn timing(&self) -> &PhyTiming {
        &self.timing
    }

    /// The configuration this engine was built with.
    #[must_use]
    pub fn config(&self) -> &LinkConfig {
        &self.config
    }

    /// Earliest armed timer, for an external scheduler.
    #[must_use]
    pub fn next_deadline(&self) -> Option<f64> {
        self.deadlines
            .iter()
            .map(|&(_, at)| at)
            .fold(None, |best: Option<f64>, at| {
                Some(best.map_or(at, |b| b.min(at)))
            })
    }

    /// Take the actions produced since the last call.
    pub fn drain(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    // ── commands ──────────────────────────────────────────────────────

    /// Call a station.
    ///
    /// # Errors
    /// If a session is already up.
    pub fn connect(&mut self, remote_call: &str) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("already in a session");
        }
        self.remote_call = remote_call.to_ascii_uppercase();
        self.session = (self.backoff.next_unit() * 256.0) as u8;
        self.state = State::Connecting;
        self.role = Role::None;
        self.connect_tries = 0;
        self.reset_transfer_state();
        self.send_connect(DataKind::ConnectReq);
        Ok(())
    }

    /// Orderly close: finish sending every queued byte, get it acknowledged, then exchange
    /// `Disc` / `DiscAck`.
    ///
    /// Requesting this while still connecting means "disconnect once the transfer is done";
    /// only [`abort`](Self::abort) tears down a half-open session.
    pub fn disconnect(&mut self) {
        if self.state == State::Idle {
            return;
        }
        self.disc_requested = true;
        if self.state == State::Connecting {
            return;
        }
        if self.role == Role::Iss && self.waiting_for.is_none() && !self.tx_busy() {
            self.maybe_start_burst();
        }
    }

    /// Drop the session now, with one disconnect on the way out.
    pub fn abort(&mut self) {
        if self.state == State::Idle {
            return;
        }
        if self.state != State::Connecting {
            let frame = self.control(ControlKind::Disc, 0, 0, 0, None, 0);
            self.transmit(vec![frame]);
        }
        self.end_session("aborted");
    }

    /// Queue bytes to send.
    pub fn send(&mut self, data: &[u8]) {
        self.tx_queue.extend_from_slice(data);
        if self.state == State::Connected && self.role == Role::Iss {
            self.maybe_start_burst();
        }
    }

    /// As the receiving station, demand the sending role in the next acknowledgement.
    pub fn request_break(&mut self) {
        self.break_requested = true;
    }

    // ── inputs ────────────────────────────────────────────────────────

    /// Tell the engine a transmission finished.
    pub fn on_tx_done(&mut self, now: f64) {
        self.now = self.now.max(now);
        self.tx_busy_until = self.tx_busy_until.min(now);
    }

    /// Advance the clock and let any due timers fire.
    pub fn tick(&mut self, now: f64) {
        self.now = self.now.max(now);
        // a snapshot in deadline order, so a timer rearmed by one that fires here is not
        // itself re-fired in the same tick
        let mut ordered = self.deadlines.clone();
        ordered.sort_by(|a, b| a.1.total_cmp(&b.1));
        let pending: Vec<Timer> = ordered.into_iter().map(|(timer, _)| timer).collect();
        for timer in pending {
            let due = self
                .deadlines
                .iter()
                .position(|&(t, at)| t == timer && at <= self.now);
            if let Some(index) = due {
                self.deadlines.remove(index);
                self.expire(timer);
            }
        }
    }

    /// Hand the engine a frame the physical layer detected, decoded or not.
    pub fn on_frame<F: SoftFrame>(&mut self, frame: &F, now: f64) {
        self.now = self.now.max(now);
        match frame.container() {
            Container::Control => self.on_control(frame),
            Container::Data => self.on_data(frame),
        }
    }

    /// The physical layer has detected a frame starting at `t_start` but has not decoded it.
    ///
    /// This is what lets the receiving station answer a burst promptly: it now knows the burst
    /// is still running and can hold its acknowledgement until that frame has finished,
    /// instead of taking a whole frame of silence as the end of the burst. Optional — a
    /// physical layer that cannot report preambles simply leaves
    /// [`PhyTiming::preamble_detect_s`] unset.
    pub fn on_preamble(&mut self, t_start: f64, now: f64) {
        self.now = self.now.max(now);
        if self.role != Role::Irs || !matches!(self.state, State::Connected | State::Disconnecting)
        {
            return;
        }
        let deadline = t_start + self.timing.data_frame_s + self.irs_reply_delay();
        let current = self.deadline_of(Timer::Ack).unwrap_or(0.0);
        self.set_deadline(Timer::Ack, deadline.max(current));
    }

    // ── timers ────────────────────────────────────────────────────────

    fn deadline_of(&self, timer: Timer) -> Option<f64> {
        self.deadlines
            .iter()
            .find(|&&(t, _)| t == timer)
            .map(|&(_, at)| at)
    }

    fn set_deadline(&mut self, timer: Timer, at: f64) {
        self.disarm(timer);
        self.deadlines.push((timer, at));
    }

    fn arm(&mut self, timer: Timer, delay: f64) {
        let at = self.now + delay;
        self.set_deadline(timer, at);
    }

    fn disarm(&mut self, timer: Timer) {
        self.deadlines.retain(|&(t, _)| t != timer);
    }

    /// How long the receiver waits after a burst's last frame before acknowledging.
    ///
    /// The sender carries no burst length — the header must be identical across the
    /// retransmissions the receiver soft-combines — so the end of a burst is inferred from
    /// silence. How much silence depends on what the physical layer reports: given a
    /// start-of-frame signal a contiguous next frame announces itself that quickly, so the
    /// receiver waits only that long; without one it must wait a whole data frame, which is
    /// roughly a quarter of the air time.
    fn irs_reply_delay(&self) -> f64 {
        let quiet = self
            .timing
            .preamble_detect_s
            .unwrap_or(self.timing.data_frame_s);
        quiet + self.config.burst_gap_s + self.timing.turnaround_s
    }

    /// How long to wait for a peer response of the given air time, measured from the end of
    /// our own transmission. `responder_delay` is any processing the peer does first — the
    /// receiver's end-of-burst wait before an acknowledgement.
    fn response_wait(&self, response_s: f64, responder_delay: f64) -> f64 {
        responder_delay
            + self.timing.turnaround_s
            + response_s
            + self.timing.detect_latency_s
            + self.config.ack_margin_s
    }

    fn tx_busy(&self) -> bool {
        self.now < self.tx_busy_until
    }

    fn expire(&mut self, timer: Timer) {
        match timer {
            Timer::Link => self.end_session("link timeout"),
            Timer::Connect => self.retry_connect(),
            Timer::Ack => self.send_ack(),
            Timer::Wait => self.on_response_timeout(),
            Timer::Keepalive => {
                if self.role == Role::Iss && self.waiting_for.is_none() {
                    self.send_poll();
                }
            }
        }
    }

    // ── transmit helpers ──────────────────────────────────────────────

    fn transmit(&mut self, frames: Vec<TxFrame>) {
        let duration_s: f64 = frames
            .iter()
            .map(|f| match f.container {
                Container::Data => self.timing.data_frame_s,
                Container::Control => self.timing.control_frame_s,
            })
            .sum();
        self.tx_busy_until = self.now + duration_s;
        self.actions.push(Action::Transmit { frames, duration_s });
    }

    fn control(
        &self,
        kind: ControlKind,
        flags: u8,
        base: u8,
        bitmap: u16,
        snr_db: Option<f64>,
        recommended_mode: u8,
    ) -> TxFrame {
        let frame = ControlFrame {
            kind,
            session: self.session,
            flags,
            base,
            bitmap,
            snr_db,
            recommended_mode,
            counter: self.ack_counter,
        };
        TxFrame {
            container: Container::Control,
            payload: frame.encode().to_vec(),
            mode: 0,
            rv: 0,
        }
    }

    fn data_frame(&self, record: &mut TxRecord) -> TxFrame {
        record.tx_count += 1;
        let capacity = self.timing.capacity(record.mode);
        let header = DataHeader {
            kind: record.kind,
            seq: record.seq,
            session: self.session,
        };
        let payload = encode_data(&header, &record.body, capacity).expect("body fits its mode");
        TxFrame {
            container: Container::Data,
            payload,
            mode: record.mode,
            rv: ((record.tx_count - 1) % 4) as u8,
        }
    }

    /// # Panics
    /// If the slowest mode cannot hold a connect body, which would make the protocol
    /// unusable. The waveform tables satisfy this by a wide margin.
    fn send_connect(&mut self, kind: DataKind) {
        let body = ConnectBody {
            src: self.my_call.clone(),
            dst: self.remote_call.clone(),
            caps: self.config.capabilities,
            version: 1,
        };
        let Ok(encoded) = body.encode() else { return };
        let capacity = self.timing.capacity(0);
        assert!(
            capacity >= CONNECT_BODY_BYTES + 5,
            "mode 0 is too small for a connect frame"
        );
        let header = DataHeader {
            kind,
            seq: 0,
            session: self.session,
        };
        let payload = encode_data(&header, &encoded, capacity).expect("connect body fits");
        self.transmit(vec![TxFrame {
            container: Container::Data,
            payload,
            mode: 0,
            rv: 0,
        }]);

        if kind == DataKind::ConnectReq {
            self.connect_tries += 1;
            // Backoff that widens with each retry, so two stations that called each other at
            // the same instant desynchronise instead of colliding on every attempt.
            let span = (1 + self.connect_tries) as f64 * self.timing.data_frame_s;
            let jitter = self.backoff.next_unit() * span;
            let wait = self.response_wait(self.timing.data_frame_s, 0.0) + jitter;
            let delay = self.tx_busy_until - self.now + wait;
            self.arm(Timer::Connect, delay);
        }
    }

    fn retry_connect(&mut self) {
        if self.state != State::Connecting {
            return;
        }
        if self.connect_tries >= self.config.connect_retries {
            self.end_session("no answer");
            return;
        }
        self.send_connect(DataKind::ConnectReq);
    }

    fn wait_for(&mut self, what: Waiting, response_s: f64, responder_delay: f64) {
        self.waiting_for = Some(what);
        let delay = self.tx_busy_until - self.now + self.response_wait(response_s, responder_delay);
        self.arm(Timer::Wait, delay);
    }

    fn send_poll(&mut self) {
        let frame = self.control(ControlKind::Poll, 0, 0, 0, None, 0);
        self.transmit(vec![frame]);
        self.wait_for(Waiting::Poll, self.timing.control_frame_s, 0.0);
    }

    fn send_turn(&mut self) {
        self.turn_tries += 1;
        self.stats.turns += 1;
        let frame = self.control(ControlKind::Turn, 0, 0, 0, None, 0);
        self.transmit(vec![frame]);
        self.role = Role::Irs;
        self.bursts_since_turn = 0;
        self.peer_request = PeerRequest::None;
        self.disarm(Timer::Keepalive);
        // the peer answers with its first burst (or a poll); we wait a full data frame
        self.wait_for(Waiting::Turn, self.timing.data_frame_s, 0.0);
        self.actions.push(Action::Event {
            name: "role",
            detail: "irs".into(),
        });
    }

    fn send_disc(&mut self) {
        self.disc_tries += 1;
        self.state = State::Disconnecting;
        let frame = self.control(ControlKind::Disc, 0, 0, 0, None, 0);
        self.transmit(vec![frame]);
        self.wait_for(Waiting::Disc, self.timing.control_frame_s, 0.0);
    }

    fn on_response_timeout(&mut self) {
        let what = self.waiting_for.take();
        if self.state == State::Disconnecting || what == Some(Waiting::Disc) {
            if self.disc_tries >= self.config.disc_retries {
                self.end_session("closed (no disc ack)");
            } else {
                self.send_disc();
            }
            return;
        }
        if what == Some(Waiting::Turn) {
            if self.turn_tries >= self.config.turn_retries {
                // the peer never took the turn: carry on as the sender
                self.role = Role::Iss;
                self.turn_tries = 0;
                self.actions.push(Action::Event {
                    name: "role",
                    detail: "iss".into(),
                });
                self.maybe_start_burst();
            } else {
                self.role = Role::Iss;
                self.send_turn();
            }
            return;
        }
        self.retries += 1;
        self.stats.ack_timeouts += 1;
        if self.retries > self.config.max_retries {
            self.end_session("no response");
            return;
        }
        match what {
            // same composition: nothing was acknowledged
            Some(Waiting::Ack) => self.send_burst(),
            Some(Waiting::Poll) => self.send_poll(),
            _ => {}
        }
    }

    // ── sending station: bursts ───────────────────────────────────────

    fn unacked(&self) -> Vec<u8> {
        let mut seqs: Vec<u8> = self
            .records
            .iter()
            .filter(|r| !r.acked && r.tx_count > 0)
            .map(|r| r.seq)
            .collect();
        let base = self.tx_base;
        seqs.sort_by_key(|&s| seq_distance(s, base));
        seqs
    }

    fn outstanding(&self) -> usize {
        seq_distance(self.tx_next, self.tx_base)
    }

    fn has_work(&self) -> bool {
        !self.unacked().is_empty() || !self.tx_queue.is_empty()
    }

    fn maybe_start_burst(&mut self) {
        if self.state != State::Connected
            || self.role != Role::Iss
            || self.waiting_for.is_some()
            || self.tx_busy()
        {
            return;
        }
        if self.disc_requested && self.unacked().is_empty() && self.tx_queue.is_empty() {
            self.send_disc();
            return;
        }
        let hand_over = match self.peer_request {
            PeerRequest::None => false,
            PeerRequest::Break => true,
            PeerRequest::WantsTx => {
                self.bursts_since_turn >= self.config.bursts_before_turn || !self.has_work()
            }
        };
        if hand_over {
            self.turn_tries = 0;
            self.send_turn();
            return;
        }
        if self.has_work() {
            self.send_burst();
        } else if self.deadline_of(Timer::Keepalive).is_none() {
            self.arm(Timer::Keepalive, self.config.keepalive_s);
        }
    }

    fn send_burst(&mut self) {
        let mut seqs = self.unacked();
        seqs.truncate(self.config.burst_frames);
        let mode = self.recommended.min(self.config.max_mode);
        let capacity = data_capacity(self.timing.capacity(mode));

        while seqs.len() < self.config.burst_frames.min(MAX_BURST)
            && self.outstanding() < WINDOW
            && !self.tx_queue.is_empty()
        {
            let take = capacity.min(self.tx_queue.len());
            let body: Vec<u8> = self.tx_queue.drain(..take).collect();
            let seq = self.tx_next;
            self.records.push(TxRecord {
                seq,
                kind: DataKind::Data,
                body,
                mode,
                tx_count: 0,
                acked: false,
            });
            self.tx_next = seq_after(self.tx_next, 1);
            seqs.push(seq);
        }
        if seqs.is_empty() {
            return;
        }

        let mut frames = Vec::with_capacity(seqs.len());
        for seq in &seqs {
            let Some(index) = self.records.iter().position(|r| r.seq == *seq) else {
                continue;
            };
            let mut record = self.records[index].clone();
            if record.tx_count > 0 {
                self.stats.frames_resent += 1;
            }
            frames.push(self.data_frame(&mut record));
            self.records[index] = record;
        }
        self.stats.frames_sent += frames.len();
        self.stats.bursts += 1;
        self.bursts_since_turn += 1;
        self.disarm(Timer::Keepalive);
        self.transmit(frames);
        let responder = self.irs_reply_delay();
        self.wait_for(Waiting::Ack, self.timing.control_frame_s, responder);
    }

    fn on_ack(&mut self, ack: &ControlFrame) {
        self.stats.acks_received += 1;
        self.retries = 0;
        for record in &mut self.records {
            if !record.acked && record.tx_count > 0 && ack.received(record.seq) {
                record.acked = true;
            }
        }
        while self.tx_base != self.tx_next {
            let base = self.tx_base;
            let Some(index) = self.records.iter().position(|r| r.seq == base) else {
                break;
            };
            if !self.records[index].acked {
                break;
            }
            self.records.remove(index);
            self.tx_base = seq_after(self.tx_base, 1);
        }
        self.recommended = usize::from(ack.recommended_mode).min(self.config.max_mode);
        self.peer_request = if ack.flags & control_flags::BREAK != 0 {
            PeerRequest::Break
        } else if ack.flags & control_flags::WANT_TX != 0 {
            PeerRequest::WantsTx
        } else {
            PeerRequest::None
        };
        self.waiting_for = None;
        self.disarm(Timer::Wait);
        self.maybe_start_burst();
    }

    // ── receiving station: bursts, HARQ and acknowledgements ──────────

    fn slot_of(&mut self, t_start: f64) -> usize {
        match self.burst_t0 {
            None => {
                self.burst_t0 = Some(t_start);
                0
            }
            // ties to even, so a frame landing exactly on a slot boundary lands in the same
            // slot here as it does in the reference model
            Some(t0) => ((t_start - t0) / self.timing.data_frame_s)
                .round_ties_even()
                .max(0.0) as usize,
        }
    }

    fn on_data<F: SoftFrame>(&mut self, frame: &F) {
        if matches!(self.state, State::Idle | State::Connecting) {
            let (payload, _) = frame.decode(None);
            if let Some(payload) = payload {
                self.on_connect_payload(&payload);
            }
            return;
        }
        if self.role == Role::Iss
            && matches!(self.waiting_for, Some(Waiting::Ack | Waiting::Poll) | None)
        {
            // A data frame from the peer while we hold the turn: it believes it is the sender
            // (a turn of ours it answered late, or a lost turn retry). Data wins.
            let (payload, _) = frame.decode(None);
            let Some(payload) = payload else { return };
            let Ok((header, _)) = decode_data(&payload) else {
                return;
            };
            if header.session != self.session {
                return;
            }
            if header.kind == DataKind::ConnectAck {
                return; // a repeated accept: our confirmation is on its way
            }
            self.role = Role::Irs;
            self.waiting_for = None;
            self.disarm(Timer::Wait);
            self.disarm(Timer::Keepalive);
            self.actions.push(Action::Event {
                name: "role",
                detail: "irs".into(),
            });
        }
        if self.role == Role::Irs && self.waiting_for == Some(Waiting::Turn) {
            self.waiting_for = None;
            self.turn_tries = 0;
            self.disarm(Timer::Wait);
        }
        // part of the current burst: record it, decode what decodes, acknowledge after the gap
        let slot = self.slot_of(frame.t_start());
        let mut record = RxRecord {
            slot,
            mode: frame.mode(),
            snr_db: frame.snr_db(),
            payload: None,
            seq: None,
        };
        self.decode_record(frame, &mut record);
        self.burst.push(record);
        let delay = (frame.t_end() - self.now).max(0.0) + self.irs_reply_delay();
        self.arm(Timer::Ack, delay);
    }

    fn decode_record<F: SoftFrame>(&mut self, frame: &F, record: &mut RxRecord) {
        let (payload, buffer) = frame.decode(None);
        if let Some(payload) = payload {
            self.accept(record, &payload);
            return;
        }
        // Inference: which sequence number is this slot? Then combine with any earlier
        // transmission of that block and, either way, keep this transmission's soft
        // information for the next retransmission. A wrong guess only wastes a combine — the
        // check never lets mismatched information through — and `max_combines` caps the waste.
        let guess = self.infer_seq(record.slot);
        record.seq = guess;
        let Some(guess) = guess.filter(|&g| in_window(g, self.rx_base, WINDOW)) else {
            self.stats.frames_failed += 1;
            return;
        };
        if let Some(index) = self.harq.iter().position(|(seq, _, _)| *seq == guess) {
            let previous = self.harq[index].1.clone();
            let combines = self.harq[index].2;
            let (combined, merged) = frame.decode(Some(&previous));
            if let Some(combined) = combined {
                self.stats.harq_rescues += 1;
                self.accept(record, &combined);
                return;
            }
            self.harq[index] = if combines + 1 >= self.config.max_combines {
                (guess, buffer, 0)
            } else {
                (guess, merged, combines + 1)
            };
        } else {
            self.harq.push((guess, buffer, 0));
        }
        self.stats.frames_failed += 1;
    }

    /// Map a burst slot to a sequence number. Decoded frames in this burst anchor the mapping
    /// when present; otherwise fall back to the most recent acknowledgement snapshots, since
    /// the sender puts unacknowledged frames first and new ones after, so slot 0 is the
    /// oldest gap.
    fn infer_seq(&self, slot: usize) -> Option<u8> {
        let anchors: Vec<(usize, u8)> = self
            .burst
            .iter()
            .filter_map(|r| r.payload.as_ref().and(r.seq).map(|seq| (r.slot, seq)))
            .collect();
        let fallback = [AckSnapshot {
            missing: Vec::new(),
            next_new: self.rx_base,
        }];
        let snapshots: &[AckSnapshot] = if self.ack_history.is_empty() {
            &fallback
        } else {
            &self.ack_history
        };

        for snapshot in snapshots {
            let expected = Self::expected_burst(snapshot);
            let mut offsets: Vec<isize> = Vec::new();
            let mut consistent = true;
            for &(anchor_slot, anchor_seq) in &anchors {
                let Some(position) = expected.iter().position(|&s| s == anchor_seq) else {
                    consistent = false;
                    break;
                };
                let offset = position as isize - anchor_slot as isize;
                if !offsets.contains(&offset) {
                    offsets.push(offset);
                }
            }
            if !consistent || offsets.len() > 1 {
                continue;
            }
            let index = slot as isize + offsets.first().copied().unwrap_or(0);
            if index >= 0 && (index as usize) < expected.len() {
                return Some(expected[index as usize]);
            }
        }
        None
    }

    fn expected_burst(snapshot: &AckSnapshot) -> Vec<u8> {
        let mut out = snapshot.missing.clone();
        let mut seq = snapshot.next_new;
        while out.len() < MAX_BURST {
            out.push(seq);
            seq = seq_after(seq, 1);
        }
        out
    }

    fn accept(&mut self, record: &mut RxRecord, payload: &[u8]) {
        let Ok((header, body)) = decode_data(payload) else {
            return;
        };
        if header.session != self.session {
            return;
        }
        record.payload = Some(payload.to_vec());
        record.seq = Some(header.seq);
        self.arm(Timer::Link, self.config.link_timeout_s);
        self.stats.frames_received += 1;

        match header.kind {
            DataKind::ConnectReq => {
                // our acceptance was lost: answer again
                self.send_connect(DataKind::ConnectAck);
                self.burst.clear();
                self.burst_t0 = None;
                self.disarm(Timer::Ack);
                return;
            }
            // a beacon belongs to nobody's session; it is reported by the caller and
            // never enters the sequence-numbered stream
            DataKind::ConnectAck | DataKind::Beacon => return,
            DataKind::Data => {}
        }

        if !in_window(header.seq, self.rx_base, WINDOW) {
            return; // an old duplicate (our acknowledgement was lost); the next one covers it
        }
        let advanced = self.max_seen.is_none_or(|seen| {
            seq_distance(header.seq, self.rx_base) > seq_distance(seen, self.rx_base)
        });
        if advanced {
            self.max_seen = Some(header.seq);
        }
        if !self.rx_buffer.iter().any(|(seq, _)| *seq == header.seq) {
            self.rx_buffer.push((header.seq, body));
        }
        self.harq.retain(|(seq, _, _)| *seq != header.seq);

        while let Some(index) = self
            .rx_buffer
            .iter()
            .position(|(seq, _)| *seq == self.rx_base)
        {
            let (_, data) = self.rx_buffer.remove(index);
            self.harq.retain(|(seq, _, _)| *seq != self.rx_base);
            self.rx_base = seq_after(self.rx_base, 1);
            self.stats.bytes_delivered += data.len();
            if !data.is_empty() {
                self.actions.push(Action::Deliver(data));
            }
        }
        if let Some(seen) = self.max_seen
            && seq_distance(seen, self.rx_base) >= WINDOW
        {
            self.max_seen = None;
        }
    }

    fn send_ack(&mut self) {
        if !matches!(self.state, State::Connected | State::Disconnecting) {
            return;
        }
        let ok = self.burst.iter().filter(|r| r.payload.is_some()).count();
        let failed = self.burst.len() - ok;
        let snr = if self.burst.is_empty() {
            None
        } else {
            Some(self.burst.iter().map(|r| r.snr_db).sum::<f64>() / self.burst.len() as f64)
        };
        // the mode the burst was mostly sent at, which is what the rate controller judges
        let burst_mode = self
            .burst
            .iter()
            .map(|r| r.mode)
            .max_by_key(|&mode| self.burst.iter().filter(|r| r.mode == mode).count());
        // HARQ buffers were stored per frame as the frames arrived, so only the accumulator
        // needs clearing here
        self.burst.clear();
        self.burst_t0 = None;
        self.rate.observe(snr, ok, failed, burst_mode);

        if self.disc_requested {
            self.send_disc();
            return;
        }

        let mut bitmap = 0u16;
        let mut missing = Vec::new();
        let limit = self
            .max_seen
            .map_or(0, |seen| seq_distance(seen, self.rx_base) + 1);
        for offset in 0..WINDOW {
            let seq = seq_after(self.rx_base, offset as u8);
            if self.rx_buffer.iter().any(|(s, _)| *s == seq) {
                bitmap |= 1 << offset;
            } else if offset < limit {
                missing.push(seq);
            }
        }
        let next_new = seq_after(self.rx_base, limit as u8);
        self.ack_history
            .insert(0, AckSnapshot { missing, next_new });
        self.ack_history.truncate(2);

        let mut flags = 0u8;
        if !self.tx_queue.is_empty() {
            flags |= control_flags::WANT_TX;
        }
        if self.break_requested {
            flags |= control_flags::BREAK | control_flags::WANT_TX;
        }
        self.ack_counter = (self.ack_counter + 1) % 16;
        self.stats.acks_sent += 1;
        let base = self.rx_base;
        let recommended = self.rate.recommend() as u8;
        let frame = self.control(ControlKind::Ack, flags, base, bitmap, snr, recommended);
        self.transmit(vec![frame]);
    }

    // ── control frames ────────────────────────────────────────────────

    fn on_control<F: SoftFrame>(&mut self, frame: &F) {
        let (payload, _) = frame.decode(None);
        let Some(payload) = payload else { return };
        let Ok(control) = ControlFrame::decode(&payload) else {
            return;
        };
        if matches!(self.state, State::Idle | State::Connecting) || control.session != self.session
        {
            return;
        }
        self.arm(Timer::Link, self.config.link_timeout_s);
        match control.kind {
            ControlKind::Disc => {
                let reply = self.control(ControlKind::DiscAck, 0, 0, 0, None, 0);
                self.transmit(vec![reply]);
                self.end_session("peer disconnected");
            }
            ControlKind::DiscAck => {
                if self.state == State::Disconnecting {
                    self.end_session("closed");
                }
            }
            ControlKind::Ack => {
                if self.role == Role::Iss
                    && matches!(self.waiting_for, Some(Waiting::Ack | Waiting::Poll))
                {
                    self.on_ack(&control);
                }
            }
            ControlKind::Poll => {
                if self.role == Role::Irs || self.waiting_for == Some(Waiting::Turn) {
                    self.take_irs();
                    let delay = self.timing.turnaround_s + (frame.t_end() - self.now).max(0.0);
                    self.arm(Timer::Ack, delay);
                }
            }
            ControlKind::Turn => {
                if self.role == Role::Irs || self.waiting_for == Some(Waiting::Turn) {
                    self.take_iss();
                }
            }
        }
    }

    fn take_irs(&mut self) {
        if self.role != Role::Irs {
            self.actions.push(Action::Event {
                name: "role",
                detail: "irs".into(),
            });
        }
        self.role = Role::Irs;
        self.waiting_for = None;
        self.turn_tries = 0;
        self.disarm(Timer::Wait);
        self.disarm(Timer::Keepalive);
    }

    fn take_iss(&mut self) {
        self.role = Role::Iss;
        self.waiting_for = None;
        self.disarm(Timer::Wait);
        self.disarm(Timer::Ack);
        self.burst.clear();
        self.burst_t0 = None;
        self.break_requested = false;
        self.bursts_since_turn = 0;
        self.retries = 0;
        self.actions.push(Action::Event {
            name: "role",
            detail: "iss".into(),
        });
        if self.has_work() {
            self.send_burst();
        } else {
            self.send_poll();
        }
    }

    // ── connection handling ───────────────────────────────────────────

    fn on_connect_payload(&mut self, payload: &[u8]) {
        let Ok((header, body)) = decode_data(payload) else {
            return;
        };
        match header.kind {
            DataKind::ConnectReq => self.handle_connect_req(header, &body),
            DataKind::ConnectAck => self.handle_connect_ack(header, &body),
            DataKind::Data | DataKind::Beacon => {}
        }
    }

    fn handle_connect_req(&mut self, header: DataHeader, body: &[u8]) {
        let Ok(request) = ConnectBody::decode(body) else {
            return;
        };
        if request.dst != self.my_call {
            return;
        }
        if self.state == State::Connecting && self.my_call > self.remote_call {
            return; // simultaneous call: the higher callsign keeps calling
        }
        self.remote_call = request.src;
        self.session = header.session;
        self.disarm(Timer::Connect);
        self.reset_transfer_state();
        self.peer_capabilities = request.caps;
        self.state = State::Connected;
        self.role = Role::Irs;
        self.arm(Timer::Link, self.config.link_timeout_s);
        self.send_connect(DataKind::ConnectAck);
        let detail = format!("{} (irs)", self.remote_call);
        self.actions.push(Action::Event {
            name: "connected",
            detail,
        });
    }

    fn handle_connect_ack(&mut self, header: DataHeader, body: &[u8]) {
        if self.state != State::Connecting || header.session != self.session {
            return;
        }
        let Ok(accept) = ConnectBody::decode(body) else {
            return;
        };
        if accept.dst != self.my_call {
            return;
        }
        self.disarm(Timer::Connect);
        self.peer_capabilities = accept.caps;
        self.state = State::Connected;
        self.role = Role::Iss;
        self.arm(Timer::Link, self.config.link_timeout_s);
        let detail = format!("{} (iss)", self.remote_call);
        self.actions.push(Action::Event {
            name: "connected",
            detail,
        });
        self.recommended = self.config.initial_mode;
        if self.has_work() {
            self.send_burst();
        } else {
            self.send_poll(); // confirms the handshake and fetches the first acknowledgement
        }
    }

    fn reset_transfer_state(&mut self) {
        self.records.clear();
        self.tx_base = 0;
        self.tx_next = 0;
        self.retries = 0;
        self.bursts_since_turn = 0;
        self.peer_request = PeerRequest::None;
        self.recommended = self.config.initial_mode;
        self.turn_tries = 0;
        self.disc_tries = 0;
        self.disc_requested = false;
        self.waiting_for = None;
        self.rx_base = 0;
        self.rx_buffer.clear();
        self.max_seen = None;
        self.harq.clear();
        self.burst.clear();
        self.burst_t0 = None;
        self.ack_history.clear();
        self.ack_counter = 0;
        self.break_requested = false;
        self.peer_capabilities = 0;
        self.rate = RateController::default();
        for timer in [Timer::Ack, Timer::Wait, Timer::Keepalive, Timer::Link] {
            self.disarm(timer);
        }
    }

    fn end_session(&mut self, reason: &str) {
        self.state = State::Idle;
        self.role = Role::None;
        self.reset_transfer_state();
        self.disarm(Timer::Connect);
        self.actions.push(Action::Event {
            name: "disconnected",
            detail: reason.to_owned(),
        });
    }
}
