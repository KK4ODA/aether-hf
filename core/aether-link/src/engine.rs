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
        MAX_BURST, PROTOCOL_VERSION, ProbeBody, WINDOW, bandwidth_code, control_flags,
        data_capacity, decode_data, encode_data, in_window, pack_callsign, seq_after, seq_distance,
    },
    phy::{Container, HarqBuffer, PhyTiming, SoftFrame, TxFrame},
    rate::{RateConfig, RateController},
};

// ── configuration, actions, states ────────────────────────────────────

/// Tuning for a session.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkConfig {
    /// Frames per burst (at most [`MAX_BURST`]). Longer bursts amortise the turnaround.
    pub burst_frames: usize,
    /// The longest a burst may be on the air, seconds — the transmitter's key-time limit
    /// less the keying's lead and tail and anything appended to a burst (a Morse
    /// identifier). A burst carries at most as many frames as fit, one at least; unset,
    /// the frame count alone decides. Six tone frames (ADR-0013) are 32 s, and the
    /// daemon's 30 s key watchdog cut the last one of every full tone burst on the air: the
    /// receiver acquired it and could not decode it, its rate controller counted that as the
    /// channel's loss, and the link stepped down the tone floor, where every burst was full
    /// again (ND1J, 2026-09-25; ADR-0017).
    pub max_burst_s: Option<f64>,
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
    /// No valid frame from the peer for this long ends the session — or longer where the
    /// link runs long frames: see [`LinkConfig::link_timeout_exchanges`].
    pub link_timeout_s: f64,
    /// Silence ends a session only after at least this many whole exchanges' air time at
    /// the family the link runs in — a full burst, its acknowledgement and both turnarounds
    /// (ADR-0012). On the ordinary layouts that is well inside `link_timeout_s`; on the
    /// 500 Hz floor one exchange is nearly half a minute, and a fixed 45 s dropped two
    /// sessions in three at −4 dB on the fading bench that four exchanges carried.
    pub link_timeout_exchanges: f64,
    /// Usable modes the sending station steps its own recommendation down by for every
    /// burst that goes unanswered (ADR-0012): a burst and its acknowledgement fade together,
    /// and the recommendation otherwise moves only when an acknowledgement brings one.
    pub silence_step: usize,
    /// Slack added to every wait for a peer response.
    pub ack_margin_s: f64,
    /// Silence after a data frame that marks the end of a burst.
    pub burst_gap_s: f64,
    /// Mode a session starts on.
    pub initial_mode: usize,
    /// Fastest mode — rung of the ladder — this station will use: the top of the widest
    /// ladder (2 300 Hz, ADR-0014) by default; a recommendation never leaves the air's own
    /// table.
    pub max_mode: usize,
    /// The fastest rung the rules allow this station to send at, where it is now — the
    /// regulatory policy's answer (ADR-0018), which link adaptation never overrides: an
    /// automatically controlled station answering outside the §97.221(b) segments may not
    /// climb past the rungs that occupy 500 Hz or less, however good the path. Unlike
    /// `max_mode`, which is the operator's taste, it reaches every frame the station sends:
    /// when it admits only tone-floor rungs, control frames, connect requests and answers,
    /// and probe answers go on the floor too. `None`: only `max_mode` applies.
    pub ceiling: Option<usize>,
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
            max_burst_s: None,
            max_retries: 8,
            connect_retries: 8,
            turn_retries: 3,
            disc_retries: 3,
            keepalive_s: 10.0,
            link_timeout_s: 45.0,
            link_timeout_exchanges: 4.0,
            silence_step: 2,
            ack_margin_s: 0.4,
            burst_gap_s: 0.2,
            initial_mode: 0,
            max_mode: 19,
            ceiling: None,
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
    /// Payload bytes the peer acknowledged: what has actually crossed, seen from the sender.
    pub bytes_acked: usize,
    /// Probes this station sent.
    pub probes_sent: usize,
    /// Probes from other stations this one answered.
    pub probes_answered: usize,
    /// Answers to this station's own probes that arrived.
    pub probe_replies: usize,
    /// Frames given another codeword at a slower mode after going unacknowledged at
    /// their own for `max_combines` transmissions.
    pub frames_reencoded: usize,
}

/// What a probe of ours came back with (ADR-0006): who answered, the SNR they measured
/// on our probe, and the SNR we measured on their answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeResult {
    /// The station that answered.
    pub remote: String,
    /// The SNR it measured on our probe, when it said.
    pub heard_there_db: Option<f64>,
    /// The SNR we measured on its answer.
    pub heard_here_db: f64,
}

/// One burst sent at a pinned mode and what came back for it: a rung of the Test
/// session's mode ladder (P6-7), the frame error rate the peer saw at the SNR it measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LadderRung {
    /// The pinned mode.
    pub mode: usize,
    /// New frames the burst carried at it; retransmissions are not counted.
    pub frames: usize,
    /// Of those, the ones the acknowledgement covered.
    pub decoded: usize,
    /// The SNR the peer measured on the burst, from its acknowledgement.
    pub snr_db: Option<f64>,
}

#[derive(Debug, Clone)]
struct TxRecord {
    seq: u8,
    kind: DataKind,
    body: Vec<u8>,
    mode: usize,
    tx_count: usize,
    acked: bool,
    /// Transmissions made under an earlier codeword, before a re-encoding.
    reencoded: usize,
}

#[derive(Debug, Clone)]
struct RxRecord {
    slot: usize,
    mode: usize,
    snr_db: f64,
    payload: Option<Vec<u8>>,
    /// Sequence number if decoded, else the inferred guess (which may be absent).
    seq: Option<u8>,
    /// The redundancy version it was sent at.
    rv: u8,
    /// The PHY trusts its measurements (`SoftFrame::trusted`).
    trusted: bool,
    /// It was combined with an earlier transmission of its block: a failure after that is a
    /// failure of both.
    combined: bool,
}

/// The redundancy versions that carry the systematic bits (TS 38.212 §5.4.2.1: RV 0 starts at
/// them, RV 3 wraps round to them): a frame at RV 1 or 2 is mostly parity and, at the rates
/// this modem runs, does not decode on its own at any SNR — 6 and 10 % on ND1J's path,
/// 2026-09-25, against 75 % at RV 0 — which is why a retransmission is combined with what came
/// before (ADR-0020).
const SELF_DECODABLE_RVS: [u8; 2] = [0, 3];

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
    Probe,
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
///
/// Its flags are independent facts of a session — a disconnect asked for, a break asked for,
/// who called, which family the peer last used — not states of one machine, so they stay bools.
#[allow(clippy::struct_excessive_bools)]
pub struct LinkEngine {
    /// The callsigns this station answers to. The first is the one it calls as unless a
    /// call says otherwise; see [`set_callsigns`](Self::set_callsigns).
    pub callsigns: Vec<String>,
    /// The callsign the current (or next) session runs under: the one that was called when
    /// this station answered, the one it chose when it called.
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
    /// The SNR the peer measured on this station's last burst, carried in its ACK — or on
    /// its last frame, carried in its other control frames (ADR-0021): the one number an
    /// operator cannot get from their own receiver.
    peer_snr_db: Option<f64>,
    /// `peer_snr_db` as the session that ended last left it: a disconnect is the frame that
    /// carries it to a station that only received, and a session's account is written once
    /// the session has ended (ADR-0021).
    ended_peer_snr_db: Option<f64>,
    /// The SNR of the last frame of this session decoded from the other station: what this
    /// station's control frames other than acknowledgements say of how it hears it.
    heard_peer_db: Option<f64>,
    turn_tries: usize,
    disc_requested: bool,
    disc_tries: usize,
    /// This station placed the call: when both stations believe they hold the turn, the
    /// caller keeps it and the called station yields (ADR-0023).
    caller: bool,
    connect_tries: usize,
    /// Whether the last frame decoded from the peer came on a floor layout (ADR-0009): the
    /// family our control frames answer in, and the layout a connect answer goes back on.
    peer_floor: bool,
    /// The mode of the last DATA frame decoded from the peer — what its next frames are
    /// most likely to take on the air.
    peer_mode: Option<usize>,
    /// The station a probe of ours is out to, until it answers or the timer fires.
    probing: Option<String>,
    /// What the last probe came back with; none while one is out, or after one went
    /// unanswered.
    last_probe: Option<ProbeResult>,
    /// A mode every new burst goes out at while set, whatever the peer recommends: the
    /// Test session's mode ladder (P6-7).
    pinned: Option<usize>,
    /// While pinned, the most payload a new frame takes — small, so a frame the pinned
    /// mode cannot carry can be re-encoded at one that can.
    pin_body: Option<usize>,
    ladder_pending: Option<(usize, Vec<u8>)>,
    /// What each pinned burst reported back, in order; see [`pin_mode`](Self::pin_mode).
    ladder: Vec<LadderRung>,
    waiting_for: Option<Waiting>,
    // receiving side
    rx_base: u8,
    rx_buffer: Vec<(u8, Vec<u8>)>,
    max_seen: Option<u8>,
    /// Per sequence number: the soft information kept for combining, how many combines it
    /// has been through, and the mode it was sent at — another mode is another codeword.
    harq: Vec<(u8, HarqBuffer, usize, usize)>,
    burst: Vec<RxRecord>,
    burst_t0: Option<f64>,
    ack_history: Vec<AckSnapshot>,
    ack_counter: u8,
    /// The fastest rung this station has recommended to the sender in this session: a burst
    /// faster than any of them is the sender's choice (ADR-0020).
    asked: Option<usize>,
    break_requested: bool,
    peer_capabilities: u8,
}

impl std::fmt::Debug for LinkEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkEngine")
            .field("callsigns", &self.callsigns)
            .field("my_call", &self.my_call)
            .field("remote_call", &self.remote_call)
            .field("state", &self.state)
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// A fresh rate controller for the PHY's mode table: its thresholds when the timing carries
/// them, the wide waveform's otherwise — with the air's floor: how many rungs are the floor's
/// and how far the first OFDM rung's margin is capped against it (ADR-0013).
fn rate_controller_for(timing: &PhyTiming) -> RateController {
    let controller = if timing.mode_threshold_db.is_empty() {
        RateController::default()
    } else {
        let frame_s: Vec<f64> = (0..timing.mode_threshold_db.len())
            .map(|m| timing.data_frame_s_for(m))
            .collect();
        RateController::for_table_timed(
            RateConfig::default(),
            &timing.mode_threshold_db,
            &timing.data_capacity,
            &frame_s,
        )
    };
    controller.with_floor(timing.floor_modes, timing.floor_margin_db)
}

impl LinkEngine {
    /// Build a station.
    #[must_use]
    pub fn new(my_call: &str, timing: PhyTiming, config: LinkConfig, seed: u64) -> Self {
        let recommended = config.initial_mode;
        let rate = rate_controller_for(&timing);
        Self {
            callsigns: vec![my_call.to_ascii_uppercase()],
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
            rate,
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
            peer_snr_db: None,
            ended_peer_snr_db: None,
            heard_peer_db: None,
            turn_tries: 0,
            disc_requested: false,
            caller: false,
            disc_tries: 0,
            connect_tries: 0,
            peer_floor: false,
            peer_mode: None,
            probing: None,
            last_probe: None,
            pinned: None,
            pin_body: None,
            ladder_pending: None,
            ladder: Vec::new(),
            waiting_for: None,
            rx_base: 0,
            rx_buffer: Vec::new(),
            max_seen: None,
            harq: Vec::new(),
            burst: Vec::new(),
            burst_t0: None,
            ack_history: Vec::new(),
            ack_counter: 0,
            asked: None,
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

    /// The SNR the peer last reported hearing this station at, in dB (3 kHz reference),
    /// once an acknowledgement — or another control frame (ADR-0021) — has carried one this
    /// session.
    #[must_use]
    pub fn peer_snr_db(&self) -> Option<f64> {
        self.peer_snr_db
    }

    /// [`peer_snr_db`](Self::peer_snr_db) as the session that ended last left it: what the
    /// other station's disconnect said, for a station that only received (ADR-0021).
    #[must_use]
    pub fn ended_peer_snr_db(&self) -> Option<f64> {
        self.ended_peer_snr_db
    }

    /// What the rate controller has measured of the other station's signal: the
    /// smoothed SNR of the bursts received this session, and the margin it is keeping
    /// over a mode's threshold. For a diagnostics display; the mode itself is
    /// [`current_mode`](Self::current_mode).
    #[must_use]
    pub fn rate_readings(&self) -> (Option<f64>, f64) {
        (self.rate.snr_db(), self.rate.margin_db())
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

    /// Send every new burst at `mode` until unpinned (`None`), whatever the peer
    /// recommends — the Test session's mode ladder (P6-7): a burst at each mode from the
    /// floor up, and what its acknowledgement said kept as a [`LadderRung`] (the frame
    /// error rate the peer saw at the SNR it measured). `body_bytes` caps the payload of
    /// each new frame while pinned, so that a frame a mode cannot carry can be re-encoded
    /// at one that can; the operator's `max_mode` still applies.
    ///
    /// # Errors
    /// For a mode the table has not, or an empty body cap.
    pub fn pin_mode(
        &mut self,
        mode: Option<usize>,
        body_bytes: Option<usize>,
    ) -> Result<(), &'static str> {
        if mode.is_some_and(|m| m >= self.timing.data_capacity.len()) {
            return Err("not a mode of the table");
        }
        if body_bytes == Some(0) {
            return Err("body_bytes must be at least 1");
        }
        self.pinned = mode;
        self.pin_body = if mode.is_some() { body_bytes } else { None };
        Ok(())
    }

    /// The rungs recorded since the last call, oldest first.
    pub fn take_ladder(&mut self) -> Vec<LadderRung> {
        std::mem::take(&mut self.ladder)
    }

    /// The rungs recorded so far.
    #[must_use]
    pub fn ladder(&self) -> &[LadderRung] {
        &self.ladder
    }

    /// What the last probe came back with.
    #[must_use]
    pub fn last_probe(&self) -> Option<&ProbeResult> {
        self.last_probe.as_ref()
    }

    /// Whether a probe of ours is out, unanswered and not yet given up on.
    #[must_use]
    pub fn probing(&self) -> bool {
        self.probing.is_some()
    }

    /// Whether everything handed to [`send`](Self::send) has left and been acknowledged.
    #[must_use]
    pub fn all_acknowledged(&self) -> bool {
        self.tx_queue.is_empty() && self.records.iter().all(|r| r.acked)
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

    /// Bytes handed to [`send`](Self::send) that have not yet reached the other station's
    /// application: the queue not yet framed, and every frame from the lowest unacknowledged
    /// one up. A frame acknowledged past a hole still counts — the receiving station hands
    /// the stream on in order, so its bytes wait with the hole — which is where this differs
    /// from [`tx_pending_bytes`](Self::tx_pending_bytes). What a session's `send` calls were
    /// given, less this, has arrived: a panel marks a message delivered from it.
    #[must_use]
    pub fn tx_undelivered_bytes(&self) -> usize {
        self.tx_queue.len() + self.records.iter().map(|r| r.body.len()).sum::<usize>()
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

    /// Change the callsigns this station answers to; the first is the one it calls as.
    ///
    /// The operator's callsign belongs to the host program in practice — VARA's published
    /// interface has no callsign of its own, `MYCALL` is the only place one is ever set, and
    /// clients send several when a station also answers to a club or tactical call — so the
    /// engine takes a list at run time rather than one name at construction.
    ///
    /// # Errors
    /// While a session is up (the callsign is in every frame's addressing, and changing it
    /// would orphan the peer), when the list is empty, or when a callsign is one the air
    /// interface cannot carry.
    pub fn set_callsigns<S: AsRef<str>>(&mut self, calls: &[S]) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("a session is running");
        }
        let cleaned: Vec<String> = calls
            .iter()
            .map(|call| call.as_ref().trim().to_ascii_uppercase())
            .filter(|call| !call.is_empty())
            .collect();
        if cleaned.is_empty() {
            return Err("at least one callsign is needed");
        }
        if cleaned.iter().any(|call| pack_callsign(call).is_err()) {
            return Err("a callsign the air interface cannot carry");
        }
        self.my_call.clone_from(&cleaned[0]);
        self.callsigns = cleaned;
        Ok(())
    }

    /// Call a station, as this station's first callsign.
    ///
    /// # Errors
    /// If a session is already up.
    pub fn connect(&mut self, remote_call: &str) -> Result<(), &'static str> {
        self.connect_as(remote_call, None)
    }

    /// Call a station as whichever of this station's callsigns the caller names, or the
    /// first of them.
    ///
    /// # Errors
    /// If a session is already up, or `as_call` is not one of this station's callsigns.
    pub fn connect_as(
        &mut self,
        remote_call: &str,
        as_call: Option<&str>,
    ) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("already in a session");
        }
        let mine = match as_call {
            None => self.callsigns[0].clone(),
            Some(call) => {
                let call = call.to_ascii_uppercase();
                if !self.callsigns.contains(&call) {
                    return Err("not one of this station's callsigns");
                }
                call
            }
        };
        self.my_call = mine;
        self.remote_call = remote_call.to_ascii_uppercase();
        // never 0: with kind DATA and sequence 0 an all-zero body would make an all-zero
        // frame, which the PHY refuses (the all-zero codeword passes any CRC)
        self.session = 1 + (self.backoff.next_unit() * 255.0) as u8;
        self.state = State::Connecting;
        self.role = Role::None;
        self.connect_tries = 0;
        self.reset_transfer_state();
        self.send_connect(DataKind::ConnectReq);
        Ok(())
    }

    /// Ask a station whether it hears this one, and how well, without a session.
    ///
    /// One PROBE frame, on the tone floor — the most robust frame there is, which a probe
    /// exists to measure a weak path with (ADR-0016) — as whichever of this station's
    /// callsigns the caller names (or the first of them); the answer, if it comes, arrives as a
    /// `probe` event naming both directions of the path — the SNR the other station
    /// measured on our probe, and the SNR we measured on its answer. A probe that goes
    /// unanswered within one frame's turnaround is reported as such; the operator asks again
    /// if they want, so there are no retries to fill a channel with.
    ///
    /// # Errors
    /// If a session is up, a probe is already out, or `as_call` is not one of this
    /// station's callsigns.
    pub fn probe(&mut self, remote_call: &str, as_call: Option<&str>) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("already in a session");
        }
        if self.probing.is_some() {
            return Err("a probe is already out");
        }
        let mine = match as_call {
            None => self.callsigns[0].clone(),
            Some(call) => {
                let call = call.to_ascii_uppercase();
                if !self.callsigns.contains(&call) {
                    return Err("not one of this station's callsigns");
                }
                call
            }
        };
        self.my_call = mine;
        let remote = remote_call.to_ascii_uppercase();
        self.stats.probes_sent += 1;
        self.last_probe = None;
        self.send_probe(DataKind::Probe, &remote, None, true);
        self.probing = Some(remote);
        let wait = self.response_wait(self.timing.data_frame_s_for(self.robust_mode(true)), 0.0);
        let delay = self.tx_busy_until - self.now + wait;
        self.arm(Timer::Probe, delay);
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
        } else if self.role == Role::Irs
            && self.state == State::Connected
            && self.burst.is_empty()
            && self.deadline_of(Timer::Ack).is_none()
            && !self.tx_busy()
        {
            // A receiving station has nothing of its own to finish, and leaves between the
            // other station's bursts: waiting to put the DISC in place of the next
            // acknowledgement waited for ever on a path where nothing decodable came — five
            // sessions with KE4QCM on 2026-09-25 ended with Abort (ADR-0023).
            self.send_disc();
        }
    }

    /// A disconnect was asked for and the DISC has not gone yet: the sender is finishing
    /// what it has queued, or the receiver is waiting for a burst to end.
    #[must_use]
    pub fn disconnect_requested(&self) -> bool {
        self.disc_requested && self.state == State::Connected
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

    /// Tell the engine a transmission finished at `now` — the last sample on the air.
    ///
    /// Work that arrived while the transmitter was busy — an acknowledgement that came in
    /// during a re-poll, data queued mid-burst — could not start a burst then, and nothing
    /// else would start it later: this is the moment to try.
    pub fn on_tx_done(&mut self, now: f64) {
        self.now = self.now.max(now);
        self.tx_busy_until = self.tx_busy_until.min(now);
        if self.state == State::Connected && self.role == Role::Iss && self.waiting_for.is_none() {
            self.maybe_start_burst();
        }
    }

    /// The physical layer is holding the last transmission back — the channel is busy —
    /// and has now held it for another `seconds`.
    ///
    /// Every deadline moves with it. A timer set when the burst was handed over expects a
    /// reply to a burst that has not left yet; left alone it fires against nothing, a retry
    /// of the same frame is queued behind the one still waiting, and the two go out back to
    /// back when the channel clears. Seen on the air on the first attempt: pairs of connect
    /// requests in one keying, at a cadence set by the busy detector rather than by the
    /// backoff.
    pub fn on_tx_delayed(&mut self, seconds: f64) {
        if seconds <= 0.0 {
            return;
        }
        self.tx_busy_until += seconds;
        for (_, at) in &mut self.deadlines {
            *at += seconds;
        }
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
            // A sender waiting for the answer to its DISC has no burst to acknowledge. What it
            // hears is the answer's company — the other station's Morse identifier, which a
            // detector can take for a data frame — or a burst from a peer that missed the
            // DISC and will hear the next one. Taken as a burst, it armed an acknowledgement,
            // and a leaving station's acknowledgement is another DISC: ND1J, 2026-09-25, two
            // in one keying, the second over his identifier.
            Container::Data if self.state == State::Disconnecting && self.role == Role::Iss => {}
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
    ///
    /// `frame_s` is the announced frame's own air time, which its preamble names (the layout,
    /// and with it the family: a floor frame is four times an ordinary one). Without it the
    /// receiving station assumes the longest frame the peer may send next — a guess that
    /// went wrong when a session's first burst dropped into the floor after ordinary connect
    /// frames: the acknowledgement fired inside every floor frame and trampled it
    /// (ADR-0012).
    ///
    /// A caller does not call over a frame it hears arriving, and a prober does not give up
    /// on one: it may be the answer, and if it is another station's the channel is busy. The
    /// next try (or the probe's deadline) waits for its end. A called station that has
    /// accepted a call whose acceptance was lost is connected, and acknowledges the
    /// undecodable preamble of the caller's next try on the floor — which the try after that
    /// ran into, again and again, until the caller gave up (ADR-0016).
    pub fn on_preamble(&mut self, t_start: f64, now: f64, frame_s: Option<f64>) {
        self.now = self.now.max(now);
        if self.state == State::Connecting || self.probing.is_some() {
            // the longest frame there is when the physical layer cannot say
            let length = frame_s.unwrap_or_else(|| self.timing.data_frame_s_for(0));
            let clear = t_start + length + self.timing.turnaround_s;
            for timer in [Timer::Connect, Timer::Probe] {
                if let Some(due) = self.deadline_of(timer) {
                    self.set_deadline(timer, due.max(clear));
                }
            }
            return;
        }
        if (self.state == State::Disconnecting || self.waiting_for == Some(Waiting::Turn))
            && let Some(due) = self.deadline_of(Timer::Wait)
        {
            // Nor does a leaving station repeat its DISC, or a station that handed over the
            // turn its TURN, over a frame it hears arriving: it may be the answer, late, and a
            // repeat keyed over it is heard by nobody (ADR-0022, ADR-0023).
            let length = frame_s.unwrap_or_else(|| self.timing.data_frame_s_for(0));
            let clear = t_start + length + self.response_wait(0.0, 0.0);
            self.set_deadline(Timer::Wait, due.max(clear));
        }
        if self.role != Role::Irs || !matches!(self.state, State::Connected | State::Disconnecting)
        {
            return;
        }
        let length = frame_s.unwrap_or_else(|| self.peer_data_frame_s());
        let deadline = t_start + length + self.irs_reply_delay(None, None);
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
    /// roughly a quarter of the air time. The floor's frames announce themselves later than
    /// the ordinary ones (the tone floor's first sync block is eight symbols of 40 ms), so a
    /// burst on the floor gets the floor's wait (ADR-0013).
    ///
    /// The receiver asks with its own view — the family and the frames it has been hearing:
    /// `None`, `None`. The sender asks too, to size its wait for the acknowledgement, and has
    /// to ask about the burst it has just *sent*: `floor` its family and `frame_s` the
    /// longest data frame the receiver will expect. Asked with its own view instead, it
    /// answered with the family it last *heard* — an ordinary acceptance before a session's
    /// first floor burst — and waited too little by the difference, the whole margin.
    fn irs_reply_delay(&self, floor: Option<bool>, frame_s: Option<f64>) -> f64 {
        let quiet = self
            .timing
            .preamble_detect_s_for(floor.unwrap_or(self.peer_floor))
            .unwrap_or_else(|| frame_s.unwrap_or_else(|| self.peer_data_frame_s()));
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
            Timer::Probe => {
                if let Some(remote) = self.probing.take() {
                    self.actions.push(Action::Event {
                        name: "probe",
                        detail: format!("{remote}: no answer"),
                    });
                }
            }
        }
    }

    // ── transmit helpers ──────────────────────────────────────────────

    fn transmit(&mut self, frames: Vec<TxFrame>) {
        let duration_s: f64 = frames.iter().map(|f| self.timing.frame_s(f)).sum();
        self.tx_busy_until = self.now + self.timing.tx_latency_s + duration_s;
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
        // every control frame says how its sender hears the other station (ADR-0021): an
        // acknowledgement says it of the burst it answers; a poll, a turn, a disconnect and
        // its answer of the last frame heard — so a station that only received, and was
        // never acknowledged, still learns how it was heard
        let snr_db = if kind == ControlKind::Ack {
            snr_db
        } else {
            snr_db.or(self.heard_peer_db)
        };
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
            floor: self.control_floor(),
        }
    }

    /// The family our control frames go out in (ADR-0009): the sending station answers in
    /// the family of the bursts it sends, the receiving one in the family of what it last
    /// decoded — so an acknowledgement comes back the way the burst went out, and either
    /// side can tell how long to wait for it.
    fn control_floor(&self) -> bool {
        if self.floor_only() {
            true
        } else if self.role == Role::Iss && self.state == State::Connected {
            self.timing.is_floor(self.burst_mode())
        } else {
            self.peer_floor
        }
    }

    /// The mode the next burst's new frames go out at: the pin while one is set, the
    /// peer's recommendation otherwise, never past the operator's ceiling or the rules'.
    fn burst_mode(&self) -> usize {
        self.pinned.unwrap_or(self.recommended).min(self.cap())
    }

    /// The fastest rung this station may send at: the operator's ceiling, and the rules'
    /// when they set one.
    fn cap(&self) -> usize {
        self.config.ceiling.map_or(self.config.max_mode, |ceiling| {
            ceiling.min(self.config.max_mode)
        })
    }

    /// Whether the rules admit only the tone floor: every frame this station sends —
    /// control frames and connect and probe answers included — goes out in that family.
    fn floor_only(&self) -> bool {
        self.config
            .ceiling
            .is_some_and(|ceiling| self.timing.is_floor(ceiling))
    }

    /// How long the control frame that answers ours may take: ours goes out in our family,
    /// and the answer comes back in it — or on the floor, from a station whose regulatory
    /// ceiling admits only the floor (ADR-0018), which is the family it last sent in.
    /// Waiting for ours alone, a poll was repeated every second and a half into a floor
    /// acknowledgement three seconds long, and the link timed out with both ends up. The
    /// longer of the two, as a burst's acknowledgement is waited for (ADR-0016).
    fn reply_control_s(&self) -> f64 {
        self.timing
            .control_frame_s_for(self.control_floor())
            .max(self.timing.control_frame_s_for(self.peer_floor))
    }

    /// Whether a body can be encoded at `mode`: within its capacity, and not the one
    /// length short of full that the container cannot carry.
    fn fits(&self, body_len: usize, mode: usize) -> bool {
        let capacity = data_capacity(self.timing.capacity(mode));
        body_len == capacity || body_len + 2 <= capacity
    }

    /// The slowest mode from `to_mode` up to (not including) `from_mode` that carries a
    /// body of `body_len` bytes, or none when none does — a full frame has nowhere
    /// slower to go.
    fn reencode_target(&self, body_len: usize, from_mode: usize, to_mode: usize) -> Option<usize> {
        (to_mode..from_mode.min(self.timing.data_capacity.len()))
            .find(|&mode| self.fits(body_len, mode))
    }

    /// An unanswered burst is evidence too: step the recommendation down
    /// [`LinkConfig::silence_step`] usable modes, never below the table's first. The frames
    /// stranded above are re-encoded on the way, after `max_combines` transmissions, and the
    /// next acknowledgement puts the peer's own recommendation back.
    fn back_off(&mut self) {
        let modes = self.rate.modes();
        let current = self.recommended.min(self.cap());
        let index = modes.iter().rposition(|&m| m <= current).unwrap_or(0);
        self.recommended = modes[index.saturating_sub(self.config.silence_step)];
    }

    /// How long the peer may stay silent before the session ends: the configured time, or
    /// [`LinkConfig::link_timeout_exchanges`] whole exchanges at the family the link runs in
    /// — the longest data frame either side sends or may send next, and that family's
    /// control frame — whichever is longer.
    fn link_timeout(&self) -> f64 {
        let frame = self
            .peer_data_frame_s()
            .max(self.timing.data_frame_s_for(self.burst_mode()));
        let floor = self.peer_floor || self.timing.is_floor(self.burst_mode());
        let exchange = self.burst_capacity(frame) as f64 * frame
            + self.timing.control_frame_s_for(floor)
            + 2.0 * self.timing.turnaround_s
            + self.config.burst_gap_s;
        self.config
            .link_timeout_s
            .max(self.config.link_timeout_exchanges * exchange)
    }

    /// Frames of `frame_s` seconds one burst may carry: the configured count, and no more
    /// than fit in [`LinkConfig::max_burst_s`] — one at least.
    #[must_use]
    pub fn burst_capacity(&self, frame_s: f64) -> usize {
        let count = self.config.burst_frames.min(MAX_BURST);
        match self.config.max_burst_s {
            Some(limit) if frame_s > 0.0 => {
                // truncation to whole frames is the point: a partial frame is a lost one
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let fit = (limit / frame_s + 1e-9).floor().max(1.0) as usize;
                count.min(fit)
            }
            _ => count,
        }
    }

    /// A new limit on a burst's air time, from the next burst on: the key-time limit is a
    /// live setting of the station's.
    pub fn set_max_burst_s(&mut self, seconds: Option<f64>) {
        self.config.max_burst_s = seconds;
    }

    /// A new regulatory ceiling (`LinkConfig::ceiling`), from the next frame on: the dial,
    /// the station's control or the session's direction changed what the rules allow.
    pub fn set_ceiling(&mut self, rung: Option<usize>) {
        self.config.ceiling = rung;
    }

    /// The regulatory ceiling in force, if any.
    #[must_use]
    pub fn ceiling(&self) -> Option<usize> {
        self.config.ceiling
    }

    /// The longest DATA frame the peer may send next: the family of what we recommended
    /// or of what it last sent, whichever is longer.
    fn peer_data_frame_s(&self) -> f64 {
        let recommended = self.timing.data_frame_s_for(self.rate.recommend());
        match self.peer_mode {
            Some(mode) => recommended.max(self.timing.data_frame_s_for(mode)),
            None => recommended,
        }
    }

    /// The slowest mode of a family whose frame carries a connect body (with a DATA header
    /// and length): what connect requests, answers, probes and beacons go out at — on the
    /// tone floor first (ADR-0016). Falls back to the ordinary family when the floor has no
    /// such mode.
    ///
    /// # Panics
    /// If no mode carries a connect frame, which would make the protocol unusable.
    #[must_use]
    pub fn robust_mode(&self, floor: bool) -> usize {
        let need = CONNECT_BODY_BYTES + 5;
        let found = (0..self.timing.data_capacity.len())
            .find(|&m| self.timing.is_floor(m) == floor && self.timing.capacity(m) >= need);
        match found {
            Some(mode) => mode,
            None if floor => self.robust_mode(false),
            None => panic!("no mode carries a connect frame"),
        }
    }

    /// Whether the next connect request goes out on the tone floor: the first try does, and
    /// every other one after it (ADR-0016). A call is made before anything is known of the
    /// path, so it goes where the path most likely carries it — the floor reaches 14 dB lower
    /// than the ordinary family's control rung — and the ordinary tries between keep a path
    /// the floor does not carry (a narrowband interferer on the floor's tones) from failing
    /// every one. ADR-0009 had the first two tries ordinary, from when the floor was an OFDM
    /// frame of its own that reached a few decibels lower; with the tone floor a weak path's
    /// first two tries were wasted.
    fn connect_floor(&self) -> bool {
        self.timing.floor_modes > 0 && (self.floor_only() || self.connect_tries % 2 == 0)
    }

    /// Learn the family and mode the peer sends data in — from a frame that decoded, so a
    /// false detection cannot switch our control frames to the wrong layout.
    fn note_peer_data(&mut self, mode: usize) {
        self.peer_floor = self.timing.is_floor(mode);
        self.peer_mode = Some(mode);
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
            floor: false,
        }
    }

    /// # Panics
    /// If the slowest mode cannot hold a connect body, which would make the protocol
    /// unusable. The waveform tables satisfy this by a wide margin.
    fn send_connect(&mut self, kind: DataKind) {
        self.send_connect_with(kind, None);
    }

    /// A connect frame; an acceptance carries the SNR the request arrived at.
    fn send_connect_with(&mut self, kind: DataKind, snr_db: Option<f64>) {
        let body = ConnectBody {
            src: self.my_call.clone(),
            dst: self.remote_call.clone(),
            caps: self.config.capabilities,
            version: PROTOCOL_VERSION,
            snr_db,
        };
        let Ok(encoded) = body.encode() else { return };
        // a request starts on the tone floor and alternates families (ADR-0016); an answer
        // goes back on the layout the request arrived on (ADR-0009)
        let floor = if kind == DataKind::ConnectReq {
            self.connect_floor()
        } else {
            self.peer_floor || self.floor_only()
        };
        let mode = self.robust_mode(floor);
        let capacity = self.timing.capacity(mode);
        let header = DataHeader {
            kind,
            seq: 0,
            session: self.session,
        };
        let payload = encode_data(&header, &encoded, capacity).expect("connect body fits");
        self.transmit(vec![TxFrame {
            container: Container::Data,
            payload,
            mode,
            rv: 0,
            floor: false,
        }]);

        if kind == DataKind::ConnectReq {
            self.connect_tries += 1;
            // Backoff that widens with each retry, so two stations that called each other at
            // the same instant desynchronise instead of colliding on every attempt.
            let frame_s = self.timing.data_frame_s_for(mode);
            let span = (1 + self.connect_tries) as f64 * frame_s;
            let jitter = self.backoff.next_unit() * span;
            let wait = self.response_wait(frame_s, 0.0) + jitter;
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

    /// A probe (`snr_db` absent) or its answer (the SNR the probe arrived at), outside
    /// any session: session 0, sequence 0, at the robust mode of `floor`'s family — a probe
    /// on the tone floor, an answer in the family the probe arrived in, as a connect answer
    /// goes (ADR-0016).
    fn send_probe(&mut self, kind: DataKind, remote: &str, snr_db: Option<f64>, floor: bool) {
        let body = ProbeBody {
            src: self.my_call.clone(),
            dst: remote.to_owned(),
            snr_db,
            caps: self.config.capabilities,
        };
        let Ok(encoded) = body.encode() else { return };
        let mode = self.robust_mode(floor);
        let capacity = self.timing.capacity(mode);
        let header = DataHeader {
            kind,
            seq: 0,
            session: 0,
        };
        let payload = encode_data(&header, &encoded, capacity).expect("probe body fits");
        self.transmit(vec![TxFrame {
            container: Container::Data,
            payload,
            mode,
            rv: 0,
            floor: false,
        }]);
    }

    fn wait_for(&mut self, what: Waiting, response_s: f64, responder_delay: f64) {
        self.waiting_for = Some(what);
        let delay = self.tx_busy_until - self.now + self.response_wait(response_s, responder_delay);
        self.arm(Timer::Wait, delay);
    }

    fn send_poll(&mut self) {
        let frame = self.control(ControlKind::Poll, 0, 0, 0, None, 0);
        self.transmit(vec![frame]);
        self.wait_for(Waiting::Poll, self.reply_control_s(), 0.0);
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
        // The peer answers with its first burst (or a poll), at a rung of its own choosing —
        // after a fade the tone floor's, a frame five seconds long. Waiting one frame of the
        // rung this station recommends, a second of OFDM, repeated the TURN over the answer
        // until the tries ran out and both stations held the turn (KE4QCM, 2026-09-25,
        // ADR-0023): the wait covers the longest first frame there is.
        let frame_s = self
            .timing
            .data_frame_s_for(0)
            .max(self.timing.data_frame_s_for(self.rate.recommend()));
        self.wait_for(Waiting::Turn, frame_s, 0.0);
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
        self.wait_for(Waiting::Disc, self.reply_control_s(), 0.0);
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
            // same composition: nothing was acknowledged — at a mode no higher than the
            // silence says the path can carry
            Some(Waiting::Ack) => {
                self.back_off();
                self.send_burst();
            }
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
        let mode = self.burst_mode();
        let recommendation = self.recommended.min(self.cap());
        let unacked = self.unacked();
        self.reencode_stranded(&unacked, recommendation);
        let mut family = self.timing.is_floor(mode);
        // One family per burst (ADR-0009): the receiver infers a frame's slot from its air
        // time, which needs every frame of the burst to be the same length. A frame keeps
        // its codeword — and so its mode — across retransmissions, so when the oldest
        // unacknowledged frame is of the other family the burst carries that family's
        // retransmissions alone and new frames wait for the next one.
        if let Some(&oldest) = unacked.first() {
            family = self.timing.is_floor(self.mode_of(oldest));
        }
        let mut seqs: Vec<u8> = unacked
            .into_iter()
            .filter(|&s| self.timing.is_floor(self.mode_of(s)) == family)
            .collect();
        // every frame of a burst is one family, and so one length: as many as fit
        let frame_s = self
            .timing
            .data_frame_s_for(seqs.first().map_or(mode, |&s| self.mode_of(s)));
        let room = self.burst_capacity(frame_s);
        seqs.truncate(room);
        let new_frames = family == self.timing.is_floor(mode);
        let capacity = data_capacity(self.timing.capacity(mode));

        while new_frames
            && seqs.len() < room
            && self.outstanding() < WINDOW
            && !self.tx_queue.is_empty()
        {
            let mut take = capacity.min(self.tx_queue.len());
            if let Some(cap) = self.pin_body {
                take = take.min(cap);
            }
            // A body one byte short of full is the one length the container cannot carry:
            // it is partial, so it needs its two length bytes, and then it no longer fits.
            // Leave one more byte for the next frame instead of failing on the air.
            if take + 1 == capacity {
                take -= 1;
            }
            let body: Vec<u8> = self.tx_queue.drain(..take).collect();
            let seq = self.tx_next;
            self.records.push(TxRecord {
                seq,
                kind: DataKind::Data,
                body,
                mode,
                tx_count: 0,
                acked: false,
                reencoded: 0,
            });
            self.tx_next = seq_after(self.tx_next, 1);
            seqs.push(seq);
        }
        if seqs.is_empty() {
            return;
        }
        if self.pinned.is_some() {
            let fresh: Vec<u8> = seqs
                .iter()
                .copied()
                .filter(|&s| {
                    self.records
                        .iter()
                        .any(|r| r.seq == s && r.tx_count == 0 && r.reencoded == 0)
                })
                .collect();
            if !fresh.is_empty() {
                self.ladder_pending = Some((mode, fresh));
            }
        }

        let mut frames = Vec::with_capacity(seqs.len());
        for seq in &seqs {
            let Some(index) = self.records.iter().position(|r| r.seq == *seq) else {
                continue;
            };
            let mut record = self.records[index].clone();
            if record.tx_count > 0 || record.reencoded > 0 {
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
        // the receiver's quiet after this burst: its family, and the longest frame it will
        // expect — this burst's, or the mode it recommended, as its own peer_data_frame_s
        // has it
        let expected = seqs
            .first()
            .map_or(0.0, |&seq| self.timing.data_frame_s_for(self.mode_of(seq)))
            .max(self.timing.data_frame_s_for(recommendation));
        // The receiving station answers in the family it last heard from us: this burst's,
        // if it decodes any of it, and the one its last answer came in (`peer_floor`) if it
        // decodes none — the acceptance of a call on the floor before a first OFDM burst, or
        // the floor bursts before a climb. Wait for whichever of the two is longer: waiting
        // for this burst's alone gave up on a floor acknowledgement a second into it, and the
        // recommendation it carried was lost with it (ADR-0016).
        let families = [family, self.peer_floor];
        let control = families
            .iter()
            .map(|&f| self.timing.control_frame_s_for(f))
            .fold(0.0, f64::max);
        let responder = families
            .iter()
            .map(|&f| self.irs_reply_delay(Some(f), Some(expected)))
            .fold(0.0, f64::max);
        self.wait_for(Waiting::Ack, control, responder);
    }

    /// A frame sent `max_combines` times at its mode without an acknowledgement is stranded
    /// there: the peer has reset its buffer for it, and another round at the same mode has
    /// the odds the last one had. When the recommendation has moved below that mode, the
    /// frame is re-encoded at the slowest mode down to it that carries the body — another
    /// codeword, which the peer starts fresh on. A full frame has nowhere slower to go and
    /// keeps trying; the ladder (P6-7) sends small bodies for that reason, and so should
    /// anything that expects to fall far.
    fn reencode_stranded(&mut self, unacked: &[u8], recommendation: usize) {
        for &seq in unacked {
            let Some(index) = self.records.iter().position(|r| r.seq == seq) else {
                continue;
            };
            let record = &self.records[index];
            if record.tx_count >= self.config.max_combines && recommendation < record.mode {
                if let Some(target) =
                    self.reencode_target(record.body.len(), record.mode, recommendation)
                {
                    let record = &mut self.records[index];
                    record.mode = target;
                    record.reencoded += record.tx_count;
                    record.tx_count = 0;
                    self.stats.frames_reencoded += 1;
                }
            }
        }
    }

    /// The mode a transmit record was encoded at.
    fn mode_of(&self, seq: u8) -> usize {
        self.records
            .iter()
            .find(|r| r.seq == seq)
            .map_or(0, |r| r.mode)
    }

    fn on_ack(&mut self, ack: &ControlFrame) {
        self.stats.acks_received += 1;
        self.retries = 0;
        if let Some((mode, fresh)) = self.ladder_pending.take() {
            let decoded = fresh.iter().filter(|&&s| ack.received(s)).count();
            self.ladder.push(LadderRung {
                mode,
                frames: fresh.len(),
                decoded,
                snr_db: ack.snr_db,
            });
        }
        for record in &mut self.records {
            if !record.acked && record.tx_count > 0 && ack.received(record.seq) {
                record.acked = true;
                self.stats.bytes_acked += record.body.len();
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
        self.recommended = usize::from(ack.recommended_mode).min(self.cap());
        if ack.snr_db.is_some() {
            self.peer_snr_db = ack.snr_db;
        }
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

    fn slot_of(&mut self, t_start: f64, mode: usize) -> usize {
        match self.burst_t0 {
            None => {
                self.burst_t0 = Some(t_start);
                0
            }
            // ties to even, so a frame landing exactly on a slot boundary lands in the same
            // slot here as it does in the reference model
            Some(t0) => ((t_start - t0) / self.timing.data_frame_s_for(mode))
                .round_ties_even()
                .max(0.0) as usize,
        }
    }

    fn on_data<F: SoftFrame>(&mut self, frame: &F) {
        if matches!(self.state, State::Idle | State::Connecting) {
            let (payload, _) = frame.decode(None);
            if let Some(payload) = payload {
                self.note_peer_data(frame.mode());
                self.on_connect_payload(&payload, frame.snr_db());
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
        if frame.floor() != self.timing.is_floor(frame.mode()) {
            // a floor frame whose chips name an ordinary mode, or the reverse: the chips are
            // noise — a false detection, most likely — and so are its SNR and its soft bits;
            // it is not part of any burst
            self.stats.frames_failed += 1;
            return;
        }
        // part of the current burst: record it, decode what decodes, acknowledge after the gap
        let slot = self.slot_of(frame.t_start(), frame.mode());
        let mut record = RxRecord {
            slot,
            mode: frame.mode(),
            snr_db: frame.snr_db(),
            payload: None,
            seq: None,
            rv: frame.rv(),
            trusted: frame.trusted(),
            combined: false,
        };
        self.decode_record(frame, &mut record);
        self.burst.push(record);
        let delay = (frame.t_end() - self.now).max(0.0) + self.irs_reply_delay(None, None);
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
        // re-encoded at another mode: another codeword, start over
        self.harq
            .retain(|(seq, _, _, mode)| *seq != guess || *mode == frame.mode());
        if let Some(index) = self.harq.iter().position(|(seq, _, _, _)| *seq == guess) {
            let previous = self.harq[index].1.clone();
            let combines = self.harq[index].2;
            record.combined = true;
            let (combined, merged) = frame.decode(Some(&previous));
            if let Some(combined) = combined {
                self.stats.harq_rescues += 1;
                self.accept(record, &combined);
                return;
            }
            self.harq[index] = if combines + 1 >= self.config.max_combines {
                (guess, buffer, 0, frame.mode())
            } else {
                (guess, merged, combines + 1, frame.mode())
            };
        } else {
            self.harq.push((guess, buffer, 0, frame.mode()));
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
        // a frame of nobody's session — a beacon, a probe or its answer, a datagram — is never
        // session data, whatever its session byte says: a datagram's holds its own number
        // (ADR-0019), which can equal this session's
        if header.kind.outside_sessions() || header.session != self.session {
            return;
        }
        record.payload = Some(payload.to_vec());
        record.seq = Some(header.seq);
        self.note_peer_data(record.mode);
        self.heard_peer_db = Some(record.snr_db);
        self.arm(Timer::Link, self.link_timeout());
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
            // an acceptance repeated: ours is on its way; nobody's frames never get this far
            DataKind::ConnectAck
            | DataKind::Beacon
            | DataKind::Probe
            | DataKind::ProbeAck
            | DataKind::Datagram => {
                return;
            }
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
        self.harq.retain(|(seq, _, _, _)| *seq != header.seq);

        while let Some(index) = self
            .rx_buffer
            .iter()
            .position(|(seq, _)| *seq == self.rx_base)
        {
            let (_, data) = self.rx_buffer.remove(index);
            self.harq.retain(|(seq, _, _, _)| *seq != self.rx_base);
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
        // A failure is news of the path when the frame could have decoded (ADR-0020): a real
        // frame (the PHY trusts it), at a redundancy version that decodes on its own or
        // combined with an earlier transmission of its block. A retransmission at RV 1 or 2
        // with nothing to combine with does not decode at any SNR — and that is how the
        // retransmission of a frame this station already has arrives, after an
        // acknowledgement the sender missed: each lost one had taught the margin 3 dB.
        let failed = self
            .burst
            .iter()
            .filter(|r| {
                r.payload.is_none()
                    && r.trusted
                    && (SELF_DECODABLE_RVS.contains(&r.rv) || r.combined)
            })
            .count();
        // The SNR of the frames that were really there: what decoded, and what the PHY
        // trusts — a real frame that failed says how the path was. A detection just over its
        // threshold that did not decode is as likely noise: on ND1J's 40 m path one read
        // -11 dB between frames decoding at +5 to +8, and it took the recommendation from
        // rung 4 to rung 1 (2026-09-25). A burst with none reports no SNR.
        let measured: Vec<f64> = self
            .burst
            .iter()
            .filter(|r| r.payload.is_some() || r.trusted)
            .map(|r| r.snr_db)
            .collect();
        let snr =
            (!measured.is_empty()).then(|| measured.iter().sum::<f64>() / measured.len() as f64);
        // the mode the burst was mostly sent at, which is what the rate controller judges
        let burst_mode = self
            .burst
            .iter()
            .map(|r| r.mode)
            .max_by_key(|&mode| self.burst.iter().filter(|r| r.mode == mode).count());
        let slowest = self.burst.iter().map(|r| r.mode).min();
        // HARQ buffers were stored per frame as the frames arrived, so only the accumulator
        // needs clearing here
        self.burst.clear();
        self.burst_t0 = None;
        if failed > 0
            && let (Some(slowest), Some(asked)) = (slowest, self.asked)
            && slowest > asked
        {
            // every frame faster than any rung this station has asked for: the sender's
            // choice, and its failure no news — the Test's ladder, pinned past what the path
            // carries, held the margin at its ceiling for the file that followed it. Anything
            // slower is judged as usual: a retransmission keeps the rung it was first sent
            // at, so after a step down the sender still sends rungs this station asked for
            // once, and their failures are the path's.
            self.rate.observe_snr(snr);
        } else {
            self.rate.observe(snr, ok, failed, burst_mode);
        }

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
        // frames sent and not yet acknowledged are work as much as the queue is: a station
        // that gave up the turn with some in flight — a BREAK, or yielding to a poll — asked
        // for nothing back, and they waited for the other station to run out of its own
        if self.has_work() {
            flags |= control_flags::WANT_TX;
        }
        if self.break_requested {
            flags |= control_flags::BREAK | control_flags::WANT_TX;
        }
        self.ack_counter = (self.ack_counter + 1) % 8;
        self.stats.acks_sent += 1;
        let base = self.rx_base;
        let recommended = self.rate.recommend();
        self.asked = Some(
            self.asked
                .map_or(recommended, |asked| asked.max(recommended)),
        );
        let frame = self.control(
            ControlKind::Ack,
            flags,
            base,
            bitmap,
            snr,
            recommended as u8,
        );
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
        self.peer_floor = frame.floor();
        self.heard_peer_db = Some(frame.snr_db());
        if control.kind != ControlKind::Ack
            && let Some(heard) = control.snr_db
        {
            // how the other station hears this one, from a frame other than an
            // acknowledgement (ADR-0021): a disconnect carries it to a station that only
            // received
            self.peer_snr_db = Some(heard);
        }
        self.arm(Timer::Link, self.link_timeout());
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
                // Both stations hold the turn when a sender hears a poll: the other missed
                // this one's answer to its TURN, took the turn back when its tries ran out,
                // and polls — and a sender that ignores a poll leaves both sending into a
                // path the other cannot hear until the session dies (KE4QCM, 2026-09-25:
                // "no response"). The called station yields: it answers the poll and asks
                // for the turn back (WANT_TX), and the caller keeps the turn, so the two can
                // never both yield (ADR-0023).
                if self.role == Role::Irs
                    || self.waiting_for == Some(Waiting::Turn)
                    || (self.role == Role::Iss && !self.caller)
                {
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
        // the first burst of a turn goes out where this station's own measurements of the
        // peer put it — HF is reciprocal — as a caller's goes out where the acceptance puts
        // it (P9-2), not at the slowest rung of the ladder: that is the tone floor
        // (ADR-0013), five times slower than the first OFDM mode
        if let Some(snr) = self.rate.snr_db() {
            let first = self.rate.first_mode(snr);
            self.recommended = self.config.initial_mode.max(first.min(self.cap()));
        }
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

    /// A decoded DATA container outside a session: a call, an answer, or a probe and its
    /// answer; `snr_db` is what the frame arrived at, which a probe is answered with.
    fn on_connect_payload(&mut self, payload: &[u8], snr_db: f64) {
        let Ok((header, body)) = decode_data(payload) else {
            return;
        };
        match header.kind {
            DataKind::ConnectReq => self.handle_connect_req(header, &body, snr_db),
            DataKind::ConnectAck => self.handle_connect_ack(header, &body, snr_db),
            DataKind::Probe => self.handle_probe(&body, snr_db),
            DataKind::ProbeAck => self.handle_probe_ack(&body, snr_db),
            DataKind::Data | DataKind::Beacon | DataKind::Datagram => {}
        }
    }

    fn handle_probe(&mut self, body: &[u8], snr_db: f64) {
        let Ok(request) = ProbeBody::decode(body) else {
            return;
        };
        if !self.callsigns.contains(&request.dst) {
            return;
        }
        if bandwidth_code(request.caps) != bandwidth_code(self.config.capabilities) {
            self.actions.push(Action::Event {
                name: "ignored",
                detail: format!("{} probes in another bandwidth", request.src),
            });
            return;
        }
        if self.state != State::Idle {
            return; // a session's frames matter more than a question from outside it
        }
        // answer as the callsign that was probed, with the SNR the probe arrived at — the
        // one number the prober cannot measure for itself
        self.my_call = request.dst;
        self.stats.probes_answered += 1;
        self.actions.push(Action::Event {
            name: "probed",
            detail: format!("{} at {snr_db:.1} dB", request.src),
        });
        // back in the family the probe came in (noted from it), as an acceptance goes back on
        // the layout its request arrived on
        let floor = self.peer_floor || self.floor_only();
        self.send_probe(DataKind::ProbeAck, &request.src, Some(snr_db), floor);
    }

    fn handle_probe_ack(&mut self, body: &[u8], snr_db: f64) {
        let Ok(answer) = ProbeBody::decode(body) else {
            return;
        };
        if answer.dst != self.my_call || self.probing.as_deref() != Some(answer.src.as_str()) {
            return;
        }
        self.disarm(Timer::Probe);
        self.probing = None;
        self.stats.probe_replies += 1;
        self.last_probe = Some(ProbeResult {
            remote: answer.src.clone(),
            heard_there_db: answer.snr_db,
            heard_here_db: snr_db,
        });
        let theirs = answer
            .snr_db
            .map_or_else(|| "?".to_owned(), |value| format!("{value:.0}"));
        self.actions.push(Action::Event {
            name: "probe",
            detail: format!(
                "{} hears us at {theirs} dB, heard at {snr_db:.1} dB",
                answer.src
            ),
        });
    }

    fn handle_connect_req(&mut self, header: DataHeader, body: &[u8], snr_db: f64) {
        let Ok(request) = ConnectBody::decode(body) else {
            return;
        };
        if !self.callsigns.contains(&request.dst) {
            return;
        }
        if bandwidth_code(request.caps) != bandwidth_code(self.config.capabilities) {
            // a call that says it was made in another bandwidth than this station's: the
            // frame decoded, so the claim is wrong, or the station is not set up for the
            // bandwidth it was called in — either way not a session to start
            self.actions.push(Action::Event {
                name: "ignored",
                detail: format!("{} calls in another bandwidth", request.src),
            });
            return;
        }
        if request.version != PROTOCOL_VERSION {
            // a mode number is a rung of the ladder since ADR-0013, which an earlier version
            // numbers differently: a session would run on numbers meaning other frames
            self.actions.push(Action::Event {
                name: "ignored",
                detail: format!(
                    "{} calls with link protocol {}",
                    request.src, request.version
                ),
            });
            return;
        }
        if self.state == State::Connecting && self.my_call > self.remote_call {
            return; // simultaneous call: the higher callsign keeps calling
        }
        self.my_call = request.dst; // answer as the callsign that was called
        self.remote_call = request.src;
        self.session = header.session;
        self.disarm(Timer::Connect);
        self.reset_transfer_state();
        self.peer_capabilities = request.caps;
        self.state = State::Connected;
        self.role = Role::Irs;
        self.heard_peer_db = Some(snr_db);
        self.arm(Timer::Link, self.link_timeout());
        // the request is the first measurement of how the caller is heard: the
        // controller starts from it, and the acceptance carries it back so the caller's
        // first burst can too (P9-2) — a lower bound if it came on the tone floor (ADR-0016)
        self.rate.seed(snr_db, self.peer_floor);
        self.send_connect_with(DataKind::ConnectAck, Some(snr_db));
        let detail = format!("{} (irs)", self.remote_call);
        self.actions.push(Action::Event {
            name: "connected",
            detail,
        });
    }

    fn handle_connect_ack(&mut self, header: DataHeader, body: &[u8], snr_db: f64) {
        if self.state != State::Connecting || header.session != self.session {
            return;
        }
        let Ok(accept) = ConnectBody::decode(body) else {
            return;
        };
        if accept.dst != self.my_call {
            return;
        }
        if bandwidth_code(accept.caps) != bandwidth_code(self.config.capabilities) {
            self.actions.push(Action::Event {
                name: "ignored",
                detail: format!("{} answers in another bandwidth", accept.src),
            });
            return;
        }
        if accept.version != PROTOCOL_VERSION {
            self.actions.push(Action::Event {
                name: "ignored",
                detail: format!(
                    "{} answers with link protocol {}",
                    accept.src, accept.version
                ),
            });
            return;
        }
        self.disarm(Timer::Connect);
        self.peer_capabilities = accept.caps;
        self.state = State::Connected;
        self.role = Role::Iss;
        self.caller = true;
        self.heard_peer_db = Some(snr_db);
        self.arm(Timer::Link, self.link_timeout());
        let detail = format!("{} (iss)", self.remote_call);
        self.actions.push(Action::Event {
            name: "connected",
            detail,
        });
        // the acceptance says how the request was heard: the first burst starts at what
        // that supports, less a step, instead of at the slowest mode; the acceptance's
        // own SNR is how the other station is heard here, which this station's
        // controller starts from for the day it receives (P9-2)
        self.rate.seed(snr_db, self.peer_floor);
        self.recommended = match accept.snr_db {
            Some(heard) => self.config.initial_mode.max(self.rate.first_mode(heard)),
            None => self.config.initial_mode,
        };
        if self.has_work() {
            self.send_burst();
        } else {
            self.send_poll(); // confirms the handshake and fetches the first acknowledgement
        }
    }

    /// Zero the running tallies. These are counted for display only — nothing in the
    /// protocol reads them — so clearing them mid-session changes no behaviour.
    pub fn reset_stats(&mut self) {
        self.stats = LinkStats::default();
    }

    fn reset_transfer_state(&mut self) {
        self.records.clear();
        self.tx_base = 0;
        self.tx_next = 0;
        self.retries = 0;
        self.bursts_since_turn = 0;
        self.peer_request = PeerRequest::None;
        self.recommended = self.config.initial_mode;
        self.peer_snr_db = None;
        self.heard_peer_db = None;
        self.turn_tries = 0;
        self.disc_tries = 0;
        self.disc_requested = false;
        self.caller = false;
        self.waiting_for = None;
        self.rx_base = 0;
        self.rx_buffer.clear();
        self.max_seen = None;
        self.harq.clear();
        self.burst.clear();
        self.burst_t0 = None;
        self.ack_history.clear();
        self.ack_counter = 0;
        self.asked = None;
        self.break_requested = false;
        self.peer_capabilities = 0;
        self.rate = rate_controller_for(&self.timing);
        for timer in [Timer::Ack, Timer::Wait, Timer::Keepalive, Timer::Link] {
            self.disarm(timer);
        }
    }

    fn end_session(&mut self, reason: &str) {
        self.ended_peer_snr_db = self.peer_snr_db;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate::{
        AWGN_THRESHOLD_DB, CONTROL_THRESHOLD_DB, FRAME_S, NARROW_AWGN_THRESHOLD_DB,
        NARROW_CONTROL_THRESHOLD_DB, NARROW_FRAME_S, NARROW_PAYLOAD_BYTES, PAYLOAD_BYTES,
    };

    /// The tone floor's control frame, in seconds: 80 symbols of 40 ms (ADR-0013).
    const TONE_CONTROL_S: f64 = 3.2;

    /// The 500 Hz air's timing, with the tone floor under it (ADR-0013) and its middle kinds
    /// (ADR-0015): the durations the model's harness hands the engine.
    fn narrow() -> PhyTiming {
        PhyTiming {
            data_frame_s: NARROW_FRAME_S[4],
            control_frame_s: 0.434,
            turnaround_s: 0.25,
            detect_latency_s: 0.15,
            tx_latency_s: 0.0,
            preamble_detect_s: Some(0.31),
            data_capacity: NARROW_PAYLOAD_BYTES.to_vec(),
            mode_threshold_db: NARROW_AWGN_THRESHOLD_DB.to_vec(),
            floor_data_frame_s: Some(NARROW_FRAME_S[0]),
            floor_control_frame_s: Some(TONE_CONTROL_S),
            floor_modes: 4,
            control_threshold_db: Some(NARROW_CONTROL_THRESHOLD_DB),
            floor_margin_db: None,
            floor_preamble_detect_s: Some(0.54),
        }
    }

    /// The 2 300 Hz air's, the same way.
    fn wide() -> PhyTiming {
        PhyTiming {
            data_frame_s: FRAME_S[6],
            control_frame_s: 0.434,
            data_capacity: PAYLOAD_BYTES.to_vec(),
            mode_threshold_db: AWGN_THRESHOLD_DB.to_vec(),
            floor_data_frame_s: Some(FRAME_S[0]),
            floor_modes: 6,
            control_threshold_db: Some(CONTROL_THRESHOLD_DB),
            floor_margin_db: Some(1.0),
            ..narrow()
        }
    }

    fn engine(timing: PhyTiming) -> LinkEngine {
        LinkEngine::new(
            "W4ODA",
            timing,
            LinkConfig {
                max_mode: 12,
                ..LinkConfig::default()
            },
            1,
        )
    }

    /// An engine receiving in a session, its controller seeded at `seed_db`.
    fn receiving(seed_db: f64) -> LinkEngine {
        let mut e = engine(wide());
        e.state = State::Connected;
        e.role = Role::Irs;
        e.session = 7;
        e.rate.seed(seed_db, false);
        e
    }

    fn heard(mode: usize, snr_db: f64, decoded: bool, trusted: bool, rv: u8) -> RxRecord {
        RxRecord {
            slot: 0,
            mode,
            snr_db,
            payload: decoded.then(|| vec![0x55]),
            seq: None,
            rv,
            trusted,
            combined: false,
        }
    }

    /// The SNR the last acknowledgement the engine sent carried.
    fn acknowledged_snr(e: &mut LinkEngine) -> Option<f64> {
        let frames = e
            .actions
            .iter()
            .rev()
            .find_map(|a| match a {
                Action::Transmit { frames, .. } => Some(frames.clone()),
                _ => None,
            })
            .expect("an acknowledgement");
        ControlFrame::decode(&frames[0].payload)
            .expect("a control frame")
            .snr_db
    }

    #[test]
    fn a_burst_reports_the_snr_of_the_frames_that_were_there() {
        // ND1J's 40 m path, 2026-09-25 (ADR-0020): failed frames barely over their threshold
        // read -11 dB between frames decoding at +5 to +8, and the burst's mean took the
        // recommendation from rung 4 to rung 1
        let mut e = receiving(6.0);
        e.burst = vec![
            heard(8, 6.0, true, true, 0),
            heard(8, -12.0, false, false, 0),
            heard(8, 2.0, false, true, 0),
        ];
        e.send_ack();
        assert_eq!(acknowledged_snr(&mut e), Some(4.0));
        // nothing that was really there: no SNR at all, and the reading stands
        let reading = e.rate.snr_db();
        e.burst = vec![heard(8, -12.0, false, false, 0); 3];
        e.send_ack();
        assert_eq!(acknowledged_snr(&mut e), None);
        assert_eq!(e.rate.snr_db(), reading);
    }

    #[test]
    fn a_burst_faster_than_this_station_ever_asked_for_is_the_senders_choice() {
        // the Test's ladder pins rungs past what the path carries: their failures are no news
        // to a controller that never asked for them
        let mut e = receiving(0.0);
        let asked = e.rate.recommend();
        e.asked = Some(asked);
        let before = (e.rate.margin_db(), e.rate.recommend());
        e.burst = vec![heard(asked + 3, 1.0, false, true, 0); 4];
        e.send_ack();
        assert_eq!((e.rate.margin_db(), e.rate.recommend()), before);
        let reading = e.rate.snr_db().expect("a reading");
        assert!(
            (reading - 0.3).abs() < 1e-9,
            "what it measured still counts: {reading}"
        );
        // a rung it asked for, failing: the path's news
        e.burst = vec![heard(asked, 1.0, false, true, 0); 4];
        e.send_ack();
        assert!(e.rate.margin_db() > before.0 || e.rate.recommend() < before.1);
    }

    #[test]
    fn a_retransmission_that_could_not_decode_alone_is_no_news() {
        // RV 1 and 2 are mostly parity (6 and 10 % alone on ND1J's path, 75 % at RV 0), and
        // alone is how the retransmission of a frame this station already has arrives, after
        // an acknowledgement the sender missed
        let mut e = receiving(6.0);
        let asked = e.rate.recommend();
        e.asked = Some(asked);
        let before = (e.rate.margin_db(), e.rate.recommend());
        e.burst = [1u8, 2, 1, 2]
            .iter()
            .map(|&rv| heard(asked, 6.0, false, true, rv))
            .collect();
        e.send_ack();
        assert_eq!((e.rate.margin_db(), e.rate.recommend()), before);
        let mut combined = heard(asked, 6.0, false, true, 1);
        combined.combined = true;
        for telling in [
            combined,
            heard(asked, 6.0, false, true, 3),
            heard(asked, 6.0, false, true, 0),
        ] {
            let mut e = receiving(6.0);
            e.asked = Some(asked);
            e.burst = vec![telling.clone()];
            e.send_ack();
            assert_ne!(
                (e.rate.margin_db(), e.rate.recommend()),
                before,
                "{telling:?}"
            );
        }
    }

    #[test]
    fn the_link_timeout_spans_whole_exchanges_at_the_floor() {
        // ADR-0012: 45 s on the ordinary layouts, four whole exchanges on the floor, where one
        // exchange is over half a minute — the tone floor now (ADR-0013), the same frames on
        // both airs
        let exchange = 6.0 * NARROW_FRAME_S[0] + TONE_CONTROL_S + 2.0 * 0.25 + 0.2;
        for timing in [wide(), narrow()] {
            let mut e = LinkEngine::new("W4ODA", timing, LinkConfig::default(), 1);
            e.recommended = 0; // a floor mode
            assert!(
                (e.link_timeout() - 4.0 * exchange).abs() < 0.01,
                "{}",
                e.link_timeout()
            );
            // the ordinary layouts on both sides: what the peer recommends and what it may send
            e.recommended = 6;
            e.rate.seed(15.0, false);
            assert!(
                (e.link_timeout() - 45.0).abs() < 1e-9,
                "{}",
                e.link_timeout()
            );
        }
    }

    #[test]
    fn an_unanswered_burst_steps_the_recommendation_down() {
        let mut e = engine(narrow());
        let modes = e.rate.modes().to_vec();
        e.recommended = modes[8];
        e.back_off();
        assert_eq!(e.recommended, modes[6]);
        e.recommended = modes[1];
        e.back_off();
        assert_eq!(e.recommended, modes[0]);
        e.back_off();
        assert_eq!(e.recommended, modes[0]);
    }

    #[test]
    fn the_iss_waits_for_an_answer_in_the_family_the_irs_last_heard() {
        // ADR-0016: the IRS answers in the family it last heard from the ISS, so a first
        // OFDM burst after a call on the floor that it decodes none of is answered on the
        // floor; the ISS waits for that acknowledgement, not only for the OFDM one its burst
        // would bring
        let wait_after_burst = |peer_floor: bool| {
            let mut e = engine(wide());
            e.state = State::Connected;
            e.role = Role::Iss;
            e.session = 7;
            e.peer_floor = peer_floor;
            e.recommended = 7; // BPSK 1/3, an OFDM rung
            e.send(&[0u8; 100]);
            assert!(matches!(e.waiting_for, Some(Waiting::Ack)));
            e.deadline_of(Timer::Wait).expect("armed") - e.tx_busy_until
        };
        let (ordinary, after_floor) = (wait_after_burst(false), wait_after_burst(true));
        let t = wide();
        let longer = t.control_frame_s_for(true) - t.control_frame_s_for(false);
        assert!(
            after_floor >= ordinary + longer - 1e-9,
            "{ordinary} {after_floor}"
        );
    }

    #[test]
    fn a_caller_does_not_call_over_a_frame_it_hears_arriving() {
        // ADR-0016: a called station whose acceptance was lost is connected, and answers the
        // undecodable preamble of the caller's next try with an acknowledgement on the floor;
        // the caller's try after that ran into it until the caller gave up. A caller waits
        // out a frame it hears arriving, and a prober its answer
        let t = wide();
        let floor_control = t.control_frame_s_for(true);
        let mut e = engine(t.clone());
        e.connect("KK4XYZ").expect("idle");
        let due = e.deadline_of(Timer::Connect).expect("armed");
        e.on_preamble(due - 1.0, due - 0.5, Some(floor_control));
        let moved = e.deadline_of(Timer::Connect).expect("armed");
        assert!(
            moved >= due - 1.0 + floor_control + t.turnaround_s,
            "{moved}"
        );
        // a frame that ends before the next try was due changes nothing
        let mut e = engine(t.clone());
        e.connect("KK4XYZ").expect("idle");
        let due = e.deadline_of(Timer::Connect).expect("armed");
        e.on_preamble(due - 10.0, due - 9.5, Some(1.0));
        let kept = e.deadline_of(Timer::Connect).expect("armed");
        assert!((kept - due).abs() < 1e-12, "{kept} {due}");
        // a probe's answer heard arriving is waited for
        let mut e = engine(t.clone());
        e.probe("KK4XYZ", None).expect("idle");
        let due = e.deadline_of(Timer::Probe).expect("armed");
        e.on_preamble(due - 1.0, due - 0.5, Some(floor_control));
        let waited = e.deadline_of(Timer::Probe).expect("armed");
        assert!(waited >= due - 1.0 + floor_control, "{waited}");
    }

    #[test]
    fn the_ack_waits_for_the_frame_the_preamble_announced() {
        // an IRS expecting ordinary frames (the connect frames were) must not answer inside a
        // floor frame five times as long: the frame names itself and the ACK waits for its end
        let mut e = engine(narrow());
        e.role = Role::Irs;
        e.state = State::Connected;
        e.peer_mode = Some(5); // the narrow air's control rung, where the connect frames went
        e.rate.seed(10.0, false);
        assert!(e.peer_data_frame_s() < 2.0);
        let floor_frame = NARROW_FRAME_S[0];
        e.on_preamble(100.0, 100.3, Some(floor_frame));
        let deadline = e.deadline_of(Timer::Ack).expect("armed");
        assert!(deadline >= 100.0 + floor_frame, "{deadline}");
        e.deadlines.clear();
        e.on_preamble(100.0, 100.3, None);
        let guessed = e.deadline_of(Timer::Ack).expect("armed");
        assert!(guessed < 100.0 + floor_frame, "{guessed}");
    }
    /// A detection that decodes to nothing: noise, or a Morse identifier, whose chips named
    /// a data mode.
    struct Junk {
        t_start: f64,
        t_end: f64,
    }

    impl SoftFrame for Junk {
        fn container(&self) -> Container {
            Container::Data
        }
        fn mode(&self) -> usize {
            12
        }
        fn floor(&self) -> bool {
            false
        }
        fn rv(&self) -> u8 {
            0
        }
        fn snr_db(&self) -> f64 {
            -11.0
        }
        fn trusted(&self) -> bool {
            false
        }
        fn t_start(&self) -> f64 {
            self.t_start
        }
        fn t_end(&self) -> f64 {
            self.t_end
        }
        fn decode(&self, _buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
            (None, vec![0.0])
        }
    }

    /// A sender that has just keyed its DISC and is waiting for the answer.
    fn disconnecting() -> LinkEngine {
        let mut e = engine(wide());
        e.role = Role::Iss;
        e.state = State::Connected;
        e.now = 100.0;
        e.disconnect();
        assert_eq!(e.state, State::Disconnecting);
        e.actions.clear();
        e.on_tx_done(101.0);
        e
    }

    fn transmissions(e: &LinkEngine) -> usize {
        e.actions
            .iter()
            .filter(|a| matches!(a, Action::Transmit { .. }))
            .count()
    }

    #[test]
    fn a_turn_waits_for_the_longest_first_frame() {
        // KE4QCM, 2026-09-25 (23:30:58): the caller waited for the answer to its TURN as
        // long as one OFDM frame, and the called station's first burst was a tone-floor frame
        // five seconds long — the TURN went out again over it, three times, and the caller
        // took the turn back. The wait covers the longest first frame there is, and a frame
        // heard arriving (ADR-0023)
        let mut e = engine(wide());
        e.role = Role::Iss;
        e.state = State::Connected;
        e.now = 100.0;
        e.rate.seed(24.0, false);
        let longest = e.timing.data_frame_s_for(0);
        assert!(e.timing.data_frame_s_for(e.rate.recommend()) < longest);
        e.peer_request = PeerRequest::WantsTx;
        e.maybe_start_burst();
        assert_eq!(e.waiting_for, Some(Waiting::Turn));
        let due = e.deadline_of(Timer::Wait).expect("armed");
        assert!(due >= 100.0 + longest, "{due}");
        let t_start = due - 0.5;
        e.on_preamble(t_start, t_start + 0.2, Some(longest));
        assert!(e.deadline_of(Timer::Wait).expect("armed") >= t_start + longest);
    }

    #[test]
    fn a_station_waiting_for_the_answer_to_its_disc_acknowledges_nothing() {
        // ND1J, 2026-09-25: waiting for the answer to its DISC, KK4ODA-1 took the other
        // station's Morse identifier — a detection whose chips named data mode 12 — for a
        // burst and armed an acknowledgement; a leaving station's acknowledgement is another
        // DISC, and two went out in one keying, over the identifier
        let mut e = disconnecting();
        let retry_at = e.deadline_of(Timer::Wait).expect("armed");
        e.on_frame(
            &Junk {
                t_start: 100.9,
                t_end: 101.9,
            },
            101.95,
        );
        assert!(
            e.deadline_of(Timer::Ack).is_none(),
            "an acknowledgement was armed"
        );
        e.tick(retry_at + 5.0);
        assert_eq!(transmissions(&e), 1, "one retry, and only one");
        assert_eq!(e.disc_tries, 2);
    }

    #[test]
    fn a_disc_is_not_repeated_over_a_frame_heard_arriving() {
        // the answer to a DISC can be late — the other station's own queue, a busy hold —
        // and a repeat keyed over it is heard by neither station
        let mut e = disconnecting();
        let retry_at = e.deadline_of(Timer::Wait).expect("armed");
        let t_start = retry_at - 0.2;
        let control = e.timing.control_frame_s;
        e.on_preamble(t_start, t_start + 0.2, Some(control));
        assert!(e.deadline_of(Timer::Wait).expect("armed") >= t_start + control);
        e.tick(t_start + control);
        assert_eq!(transmissions(&e), 0, "repeated over the frame");
        // a physical layer that cannot name the frame: the longest there is
        let mut e = disconnecting();
        e.on_preamble(t_start, t_start + 0.2, None);
        let longest = e.timing.data_frame_s_for(0);
        assert!(e.deadline_of(Timer::Wait).expect("armed") >= t_start + longest);
    }
}
