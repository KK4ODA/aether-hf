//! Granted slots: the link engine of protocol 8 (ADR-0069).
//!
//! One [`GrantedEngine`] per station, beside [`LinkEngine`](crate::LinkEngine) (protocol 7).
//! Like it, it is a pure state machine — [`on_frame`](GrantedEngine::on_frame),
//! [`on_tx_done`](GrantedEngine::on_tx_done) and [`tick`](GrantedEngine::tick) in, [`Action`]s
//! out — and it keeps its data path: selective repeat over a 16-frame window, HARQ with slot
//! inference, re-encoding, the rate controller and the connect handshake's measurements. What it
//! changes is *when* a station may key, and nothing else.
//!
//! It is a port of the reference model's `aether_model.link.granted`, rule by rule and draw by
//! draw: `tools/make_granted_vectors.py` records every input and every action of the model's
//! engines through adversarial sessions, and `tests/granted_vectors.rs` replays the inputs here
//! and compares every action. A mismatch is a bug in this file, not in the vectors.
//!
//! # The protocol in one paragraph
//!
//! The caller is the **master** for the whole session and the only station that keys on its
//! own initiative; the called station is the **slave**. Every master transmission is a header
//! control frame and up to six data frames, each data frame stating exactly how many of the
//! transmission follow it. The header acknowledges the slave's data, recommends the slave's
//! rung, states its own count and *grants* the slave one slot, which starts a fixed gap after
//! the transmission's end and lasts at most the grant: an answer, an answer and `n` data
//! frames, or, when neither side has anything, an open window. The slave keys only in a slot
//! granted by a frame it decoded in time to start the slot on time — the daemon's keying gate
//! drops a transmission that would start late, whatever held it
//! ([`Action::Transmit::not_after`](Action::Transmit)) — and its answer acknowledges the
//! master's data, says what follows it and what it has queued. The master keys again only once
//! the slot can no longer be in use. Only decoded content changes state: a detection, an
//! announcement or a silence moves no deadline. `docs/research/link-architecture/07-protocol-8.md`
//! is the design, `04-formal-spec.md` the rules and their proof, ADR-0069 the decision.
//!
//! # The slot, as both stations reckon it
//!
//! After a transmission ending on the air at `E` whose last frame is of family `f` (ordinary or
//! the tone floor), at gap level `L`:
//!
//! ```text
//! start   = E + max(D_f + KEY, T_granter) + GUARD + LEVEL[L]
//! end_max = start + SLACK + grant
//! ```
//!
//! `D_f` is the decode budget, `KEY` the keying latency a station is allowed, `T_granter` the
//! turnaround the granting station declared at the connection, `GUARD` propagation and clock
//! error. They are protocol constants, the same at both ends: a station slower than they allow
//! asks for a gap level in its answers.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::{
    engine::{Action, LadderRung, ProbeResult, State},
    frames::{
        CAP_LEVEL_MASK, CONNECT_BODY_8_BYTES, ConnectBody, DataHeader, DataKind, GrantFrame,
        GrantKind, MAX_BURST, MAX_FOLLOWS, PROTOCOL_VERSION, ProbeBody, WINDOW, bandwidth_code,
        bandwidth_hz_of, data_capacity, decode_data, encode_data, in_window, level_of,
        pack_callsign, seq_after, seq_distance, with_level,
    },
    phy::{Container, HarqBuffer, PhyTiming, SoftFrame, TxFrame},
    rate::{CONTROL_THRESHOLD_DB, RateConfig, RateController},
};

// ── the protocol's constants ──────────────────────────────────────────

/// From a frame's end on the air to its decode reaching the engine, ordinary and tone floor.
/// Measured on the harness (`08-steps-0-and-1.md` §1.3): 11 895 frames of both families on both
/// airs were reported at most 0.157 s after their end, the 99th percentile 0.156 s. Hardware in
/// the loop checks it again before it freezes.
pub const DECODE_BUDGET_S: [f64; 2] = [0.30, 0.30];
/// From the engine's decision to radiation: the daemon's lead (0.1 s) and the sound card
/// (0.25 s). It leaves nothing for rendering, so the daemon renders a slot's data frames before
/// the header granting it is decoded; a station slower than this asks for a level.
pub const KEY_S: f64 = 0.35;
/// Propagation (under 20 ms one way on HF), clock error over a transmission, rounding.
pub const GUARD_S: f64 = 0.05;
/// How late a slot may start on the air and still be used; the keying gate drops past it.
pub const SLACK_S: f64 = 0.35;
/// Extra gap by level: what a slave whose computer decodes or keys late asks for, and what a
/// master that hears nothing twice in a row grants.
pub const LEVELS_S: [f64; 4] = [0.0, 0.5, 1.0, 2.0];
/// How far from its slot a frame that did not decode may begin and still count as one of the
/// transmission's: the receiver places a frame to a few milliseconds, propagation adds 20.
pub const GRID_TOLERANCE_S: f64 = 0.15;
/// Data frames in one transmission (three bits count them in the control frame).
pub const MAX_DATA: usize = 6;
/// How far into a transmission's first frame, a control frame, a cut still leaves nothing of
/// the transmission readable (10-step-3.md §3.6). A tone control frame decodes only from two of
/// its three sync blocks (ADR-0049), and the middle one begins past 45 % of the frame; an
/// ordinary control frame is coded at rate ½ at the most robust, and 60 % of its symbols lost
/// leaves nothing to decode. The daemon reports such a cut with
/// [`GrantedEngine::on_tx_cut_unread`].
pub const HEADER_UNDECODABLE_SHARE: f64 = 0.4;
/// The redundancy versions a copy decodes alone in on every rung: what R7 needs before it may
/// leave a header out, since a count is believed only from a copy that decoded alone (R2). RV 1
/// and 2 never decode alone, and RV 3 only on the rungs whose RV 3 carries every systematic bit
/// (ADR-0043): on ND1J's Tests of 2026-10-15 every headerless transmission of RV 2 and 3 copies
/// went silent, eight of the 21 silent slots (ADR-0073).
pub const READABLE_ALONE_RVS: [u8; 1] = [0];
/// An idle link's open window, after an ordinary and after a floor header.
pub const OPEN_WINDOW_S: [f64; 2] = [4.0, 10.0];

/// The reasons a session ends that leave the other station in it: its frames are answered with
/// a DISC for a link timeout after (ADR-0052).
const LEFT_UNHEARD: [&str; 4] = [
    "aborted",
    "closed (no DISC_ACK)",
    "no response",
    "link timeout",
];
/// Redundancy versions that decode on their own; a failed frame at another, not combined, is no
/// news of the path (ADR-0020).
const SELF_DECODABLE_RVS: [u8; 2] = [0, 3];
/// The redundancy version of each transmission of a block (ADR-0043).
const RV_SEQUENCE: [u8; 4] = [0, 0, 2, 3];

/// The engine's random draws (a session number, a call's jitter): `SplitMix64`, written out in
/// the model too, so that both draw the same numbers from the same seed and every action can be
/// checked against the model's.
#[derive(Debug, Clone)]
struct Draws {
    state: u64,
}

impl Draws {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1), from the top 53 bits.
    #[allow(clippy::cast_precision_loss)] // 53 bits are exact in an f64
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    fn uniform(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    /// Uniform in [0, n).
    const fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Which side of a session this station is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// No session.
    None,
    /// The caller: the only station that keys on its own initiative.
    Master,
    /// The called station: it keys only in slots the master grants.
    Slave,
}

// ── configuration and counters ────────────────────────────────────────

/// Tuning for a station; the protocol's constants are here too, so a bench can change them for
/// both stations (they must hold the same).
#[derive(Debug, Clone, PartialEq)]
pub struct GrantConfig {
    /// Data frames in one transmission, at most [`MAX_DATA`].
    pub max_data_frames: usize,
    /// The longest a transmission may be on the air, seconds: the key-time limit less the
    /// keying's lead and tail (ADR-0017). Unset, the frame count alone decides.
    pub max_burst_s: Option<f64>,
    /// Silent slots in a row before a master gives the session up ("no response").
    pub max_retries: usize,
    /// Tries of a call.
    pub connect_retries: usize,
    /// DISCs a leaving master sends, unanswered, before it leaves without the `DISC_ACK`.
    pub disc_retries: usize,
    /// A leaving master gives up on what it has not got acknowledged after this many whole
    /// exchanges with nothing new acknowledged, or [`disc_patience_s`](Self::disc_patience_s)
    /// (ADR-0039).
    pub disc_patience_exchanges: f64,
    /// The least of that patience, seconds.
    pub disc_patience_s: f64,
    /// The longest a disconnect waits for what is still queued, counted from when it was
    /// asked: past it the master sends its DISC at its next opportunity, delivered or not, and
    /// says what it left (ADR-0072). A slow path that still delivers never runs out of the
    /// patience — the soak of 2026-10-15 drew three that would have taken 30–40 minutes.
    pub disc_drain_s: f64,
    /// DISCs an aborting master says, one a slot, while nothing answers (ADR-0052).
    pub leave_repeats: usize,
    /// A call heard in the ordinary family less than this above the ordinary control frame's
    /// threshold is accepted on the tone floor (ADR-0056).
    pub floor_answer_margin_db: f64,
    /// A station asks for the other's control frames on the floor while it hears the other less
    /// than this above the ordinary control frame's threshold (swept 0–3 dB on the review's
    /// campaign: smaller margins win a steady channel at the boundary and lose on fading ones).
    pub answer_floor_margin_db: f64,
    /// How many of the other station's frames, read in a row, must all clear that before a
    /// station asks for ordinary control frames: on a fading path an ordinary frame decodes only
    /// at a peak, so one reading is biased high (three is the fewest that holds completion).
    pub family_readings: usize,
    /// The least silence that ends a session, seconds.
    pub link_timeout_s: f64,
    /// Silence ends a session after this many exchanges of the size in use.
    pub link_timeout_exchanges: f64,
    /// The rate controller's tuning.
    pub rate: RateConfig,
    /// The rung a session's data starts at before anything is measured.
    pub initial_mode: usize,
    /// The fastest rung this station uses.
    pub max_mode: usize,
    /// The fastest rung the rules allow (ADR-0018).
    pub ceiling: Option<usize>,
    /// Rungs a master steps its own data down by for every data slot heard silent (P9-7).
    pub silence_step: usize,
    /// Transmissions of a block that are combined before its buffer starts over.
    pub max_combines: usize,
    /// Transmissions after which a block the path cannot carry at its rung is re-encoded
    /// (ADR-0038).
    pub reencode_early_after: usize,
    /// How far under what `max_combines` copies make up a rung is hopeless.
    pub reencode_hopeless_db: f64,
    /// Offered in the handshake: compression (bit 0) and the bandwidth (bits 1–2).
    pub capabilities: u8,
    /// Data frames granted to the slave before it has said what it has: a gateway greets.
    pub first_grant: usize,
    /// How far an idle link's open window grows, by doubling, while it stays idle.
    pub window_doublings: usize,
    /// Leave the header out of a transmission it would change nothing in (R7).
    pub elide: bool,
    /// This station's radio: how long after it stops sending it hears again (VOX hold and
    /// receiver recovery). Declared at the connection, 0.1 s steps up to 1.5 s.
    pub turnaround_s: f64,
    /// This station's closing Morse identifier's length, 0 for none: declared at the connection,
    /// so the master grants the slave's in the `DISC_ACK`'s slot and the slave knows how long the
    /// master's lasts (ADR-0075). The length for every callsign not in
    /// [`identifiers`](Self::identifiers).
    pub identifier_s: f64,
    /// The identifier's length by the callsign a session runs under: the daemon's, since the
    /// identifier sends that callsign and whether one is sent at all can follow it (a US callsign
    /// always identifies). Set by [`set_identifiers`](GrantedEngine::set_identifiers).
    pub identifiers: BTreeMap<String, f64>,
    /// The least time between the end of the other station's frames and this station
    /// radiating: the operator's `[radio] answer_gap_ms`, or what this station learned the other
    /// needs (ADR-0036, ADR-0054). For a peer whose radio is slower to hear again than it
    /// declared — a VOX interface's hold left at 0. As the master the station keys no sooner
    /// than the gap after the slave's frames; as the slave it asks for the gap level that starts
    /// its slots that late (07-protocol-8.md §10). Set by
    /// [`set_answer_gap`](GrantedEngine::set_answer_gap).
    pub answer_gap_s: f64,
    /// [`DECODE_BUDGET_S`].
    pub decode_budget_s: [f64; 2],
    /// [`KEY_S`].
    pub key_s: f64,
    /// [`GUARD_S`].
    pub guard_s: f64,
    /// [`SLACK_S`].
    pub slack_s: f64,
    /// [`LEVELS_S`].
    pub levels_s: [f64; 4],
    /// [`OPEN_WINDOW_S`].
    pub open_window_s: [f64; 2],
    /// On an idle link the master grants an answer, not an open window, once it has heard
    /// nothing from the slave for this long: a link that is only ever silent cannot tell idle
    /// from dead.
    pub keepalive_s: f64,
    /// Silent slots in a row after which a master raises the gap level one step.
    pub silence_levels_after: usize,
    /// Answer slots looked back over: while any of the last this many went silent, a header
    /// asks for the slave's answers on the floor (10-step-3.md §3.4). The readings that
    /// otherwise decide the family come only from frames that decoded, and miss what kills a
    /// short ordinary answer and spares a floor one — a PACTOR station's bursts on a lopsided
    /// path. 0: only after two silent slots in a row. 4 by the harness (10-step-3.md §5.4).
    pub floor_answer_memory: usize,
    /// Frames whose answer went unheard are held back until an answer says what became of
    /// them, not resent blind (10-step-3.md §3.4). The next transmission carries new data, or no
    /// data at all — a header granting an answer, a poll. A second silent slot in a row sends
    /// them again: answers not getting through must not stop the data too. On the PACTOR path
    /// the slave decoded more frames than protocol 7's and still finished later: copies of
    /// frames it had.
    pub hold_unknown: bool,
    /// Slots answered in a row after which a master lowers the silence level one step.
    pub on_time_to_lower: usize,
    /// Slots answered on time after which a slave asks for one level less.
    pub need_ease_after: usize,
}

impl Default for GrantConfig {
    fn default() -> Self {
        Self {
            max_data_frames: MAX_DATA,
            max_burst_s: None,
            max_retries: 8,
            connect_retries: 8,
            disc_retries: 3,
            disc_patience_exchanges: 2.0,
            disc_patience_s: 20.0,
            disc_drain_s: 60.0,
            leave_repeats: 3,
            floor_answer_margin_db: 12.0,
            answer_floor_margin_db: 3.0,
            family_readings: 3,
            link_timeout_s: 45.0,
            link_timeout_exchanges: 4.0,
            rate: RateConfig::default(),
            initial_mode: 0,
            max_mode: 19,
            ceiling: None,
            silence_step: 2,
            max_combines: 4,
            reencode_early_after: 2,
            reencode_hopeless_db: 1.0,
            capabilities: 0,
            first_grant: 2,
            window_doublings: 1,
            elide: true,
            turnaround_s: 0.0,
            identifier_s: 0.0,
            identifiers: BTreeMap::new(),
            answer_gap_s: 0.0,
            decode_budget_s: DECODE_BUDGET_S,
            key_s: KEY_S,
            guard_s: GUARD_S,
            slack_s: SLACK_S,
            levels_s: LEVELS_S,
            open_window_s: OPEN_WINDOW_S,
            keepalive_s: 10.0,
            silence_levels_after: 2,
            floor_answer_memory: 4,
            hold_unknown: true,
            on_time_to_lower: 4,
            need_ease_after: 6,
        }
    }
}

/// Counters a caller can show an operator.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GrantStats {
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
    /// Payload bytes delivered to the application.
    pub bytes_delivered: usize,
    /// Payload bytes the other station acknowledged.
    pub bytes_acked: usize,
    /// Probes this station sent.
    pub probes_sent: usize,
    /// Probes from other stations this one answered.
    pub probes_answered: usize,
    /// Answers to this station's own probes that arrived.
    pub probe_replies: usize,
    /// Frames given another codeword at a slower rung.
    pub frames_reencoded: usize,
    /// Frames of a session this station had left, answered with a DISC (ADR-0052).
    pub left_answered: usize,
    /// Transmissions handed over.
    pub transmissions: usize,
    /// Headers left out of a transmission they changed nothing in (R7).
    pub headers_elided: usize,
    /// Slots this master granted in which it decoded nothing.
    pub silent_slots: usize,
    /// Slots this station was granted and did not use: the grant decoded too late.
    pub forfeits: usize,
    /// Transmissions the keying gate refused to start late
    /// ([`on_tx_dropped`](GrantedEngine::on_tx_dropped)).
    pub dropped: usize,
}

// ── internal records ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TxRecord {
    seq: u8,
    body: Vec<u8>,
    mode: usize,
    tx_count: usize,
    acked: bool,
    reencoded: usize,
    /// Sent in a transmission whose answer went unheard: whether it arrived is not known.
    unknown: bool,
    /// Sent first at a rung pinned to measure it (the Test's ladder). Its first copy is the
    /// measurement; after it, the frame goes on as data does, at the rung the other station asks
    /// for (ADR-0074).
    pinned: bool,
}

impl TxRecord {
    const fn sent(&self) -> bool {
        self.tx_count > 0 || self.reencoded > 0
    }
}

/// What the engine keeps of a frame it was handed: everything but the decoding.
#[derive(Debug, Clone, Copy)]
struct Heard {
    mode: usize,
    floor: bool,
    rv: u8,
    snr_db: f64,
    trusted: bool,
    follows: Option<u8>,
    t_start: f64,
    t_end: f64,
}

impl Heard {
    fn of<F: SoftFrame>(frame: &F) -> Self {
        Self {
            mode: frame.mode(),
            floor: frame.floor(),
            rv: frame.rv(),
            snr_db: frame.snr_db(),
            trusted: frame.trusted(),
            follows: frame.follows(),
            t_start: frame.t_start(),
            t_end: frame.t_end(),
        }
    }
}

#[derive(Debug, Clone)]
struct RxRecord {
    frame: Heard,
    slot: usize,
    payload: Option<Vec<u8>>,
    seq: Option<u8>,
    combined: bool,
}

/// What one of this station's control frames said it had received: the frames it missed and
/// the next new one — what the sender's next transmission is laid out from.
#[derive(Debug, Clone)]
struct AckSnapshot {
    missing: Vec<u8>,
    next_new: u8,
}

/// A set of families: the ordinary and the tone floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Families {
    ordinary: bool,
    floor: bool,
}

impl Families {
    const fn of(floor: bool) -> Self {
        Self {
            ordinary: !floor,
            floor,
        }
    }

    const fn both() -> Self {
        Self {
            ordinary: true,
            floor: true,
        }
    }

    const fn insert(&mut self, floor: bool) {
        if floor {
            self.floor = true;
        } else {
            self.ordinary = true;
        }
    }

    const fn with(mut self, floor: bool) -> Self {
        self.insert(floor);
        self
    }

    /// The largest of `f` over the members.
    fn max_of(self, f: impl Fn(bool) -> f64) -> f64 {
        let mut most = f64::NEG_INFINITY;
        for (member, floor) in [(self.ordinary, false), (self.floor, true)] {
            if member {
                most = most.max(f(floor));
            }
        }
        most
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    Header,
    Disc,
    Connect,
    Probe,
    Leave,
}

/// A slot this station granted: the other station's time on the air, and this station's wait
/// for it.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
struct Slot {
    id: u64,
    kind: SlotKind,
    /// The longest the answering transmission may be, its identifier included.
    grant_s: f64,
    /// The family of the granting transmission's last frame: the gap's decode budget.
    last_floor: bool,
    /// Every family the answer, and what follows it, may be in: the wait covers the slowest.
    answer_floors: Families,
    level: usize,
    granter_turnaround_s: f64,
    open_window: bool,
    /// The slave's identifier time granted: its answer starts that much into the slot.
    ident_s: f64,
    /// The granting transmission carried data: a silent slot steps its rung down.
    data: bool,
    granted: usize,
    tx_end: f64,
    start: f64,
    end_max: f64,
    /// `tx_end` is where the granting transmission really ended (`on_tx_done`), not this
    /// engine's estimate.
    final_: bool,
    /// A frame of the other station's, begun in the slot, decoded.
    heard: bool,
    stated_end: Option<f64>,
    stated_floor: bool,
}

impl Slot {
    fn new(kind: SlotKind, grant_s: f64, last_floor: bool, answer_floors: Families) -> Self {
        Self {
            id: 0,
            kind,
            grant_s,
            last_floor,
            answer_floors,
            level: 0,
            granter_turnaround_s: 0.0,
            open_window: false,
            ident_s: 0.0,
            data: false,
            granted: 0,
            tx_end: 0.0,
            start: 0.0,
            end_max: 0.0,
            final_: false,
            heard: false,
            stated_end: None,
            stated_floor: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerKind {
    Ans,
    Window,
    ConnectAck,
    ProbeAck,
    Disc,
    DiscAck,
}

/// A transmission this station owes in a slot it was granted (or, idle, a call's or a probe's
/// or a left session's slot).
#[derive(Debug, Clone)]
struct Answer {
    what: AnswerKind,
    start: f64,
    not_after: f64,
    /// The family of its control frame.
    floor: bool,
    session: u8,
    /// The end of the transmission it answers.
    anchor: f64,
    remote: String,
    snr_db: Option<f64>,
    /// Data time granted after the answer.
    room_s: f64,
}

impl Answer {
    fn new(what: AnswerKind, start: f64, not_after: f64, floor: bool) -> Self {
        Self {
            what,
            start,
            not_after,
            floor,
            session: 0,
            anchor: 0.0,
            remote: String::new(),
            snr_db: None,
            room_s: 0.0,
        }
    }
}

/// When a transmission may radiate, and what goes after its frames.
#[derive(Debug, Clone, Copy, Default)]
struct Keying {
    /// The latest its radiation may start (the keying gate's limit).
    not_after: Option<f64>,
    /// The earliest (the start of the slot it answers in).
    not_before: Option<f64>,
    /// The closing identifier after the frames, or alone (ADR-0075).
    identifier_s: f64,
    /// Made outside every slot: the daemon listens first.
    listen_first: bool,
}

impl Keying {
    /// An answer in its slot.
    const fn answering(answer: &Answer, identifier_s: f64) -> Self {
        Self {
            not_after: Some(answer.not_after),
            not_before: Some(answer.start),
            identifier_s,
            listen_first: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timer {
    Link,
    Connect,
    Probe,
    Proceed,
    Answer,
    Leave,
    Ident,
}

impl Timer {
    const fn cause(self) -> &'static str {
        match self {
            Self::Link => "timer:link",
            Self::Connect => "timer:connect",
            Self::Probe => "timer:probe",
            Self::Proceed => "timer:proceed",
            Self::Answer => "timer:answer",
            Self::Leave => "timer:leave",
            Self::Ident => "timer:ident",
        }
    }
}

/// The armed timers, in the order they were first armed: a timer armed again keeps its place,
/// and timers due together expire in that order (the model's dictionary keeps it so).
#[derive(Debug, Clone, Default)]
struct Deadlines(Vec<(Timer, f64)>);

impl Deadlines {
    fn set(&mut self, timer: Timer, at: f64) {
        if let Some(entry) = self.0.iter_mut().find(|e| e.0 == timer) {
            entry.1 = at;
        } else {
            self.0.push((timer, at));
        }
    }

    fn get(&self, timer: Timer) -> Option<f64> {
        self.0.iter().find(|e| e.0 == timer).map(|e| e.1)
    }

    fn remove(&mut self, timer: Timer) {
        self.0.retain(|e| e.0 != timer);
    }

    fn earliest(&self) -> Option<f64> {
        self.0.iter().map(|e| e.1).reduce(f64::min)
    }
}

// ── the engine ────────────────────────────────────────────────────────

/// The link engine of protocol 8: granted slots (ADR-0069).
#[allow(clippy::struct_excessive_bools)]
pub struct GrantedEngine {
    /// The callsigns this station answers to; the first is the one it calls as.
    pub callsigns: Vec<String>,
    /// The callsign this station's session or call goes as.
    pub my_call: String,
    /// The other station of the session or call.
    pub remote_call: String,
    /// Counters.
    pub stats: GrantStats,
    timing: PhyTiming,
    config: GrantConfig,
    rng: Draws,
    state: State,
    role: Role,
    session: u8,
    now: f64,
    actions: Vec<Action>,
    deadlines: Deadlines,
    cause: &'static str,
    /// The slot this station granted last — a call's, a probe's, a header's, a DISC's.
    slot: Option<Slot>,
    next_slot_id: u64,
    /// This station's transmissions handed over and not yet done, in order, each with the slot
    /// it granted — `on_tx_done` reports where that slot's granting transmission ended — and the
    /// closing identifier it carries behind a `DISC_ACK`, owed again if it is dropped.
    in_flight: VecDeque<(Option<u64>, f64)>,
    /// Nothing this station starts on its own — a call, a probe, a leaving DISC — or answers
    /// while idle keys before this: the last slot it granted may still be in use.
    hold_until: f64,
    /// A session this station has left: its number, until when its frames are answered, and
    /// whether the other station's DISC ended it (ADR-0052).
    left: Option<(u8, f64, bool)>,
    leave_repeats: usize,
    call_jitter: f64,
    /// This station's closing identifier's length for the session under way or just ended:
    /// what its call or acceptance declared, so what the other station reckons with.
    my_ident_s: f64,
    /// The closing identifier of a session that has ended, not yet handed over: its length, or 0
    /// (ADR-0075).
    id_owed: f64,
    /// Whether the owed identifier goes on its own initiative — no slot could be had for it —
    /// rather than in time the closing exchange left this station.
    id_alone: bool,
    /// Whether the owed identifier listens first: one on its own, and a master's after a session
    /// in which it read the other station no more — that station may be sending its own
    /// (ADR-0075).
    id_listen: bool,
    probing: Option<String>,
    probe_unsent: bool,
    last_probe: Option<ProbeResult>,
    ended_peer_snr_db: Option<f64>,
    /// What each pinned transmission's acknowledgement reported (the Test's ladder).
    ladder: Vec<LadderRung>,
    pinned: Option<usize>,
    pin_body: Option<usize>,
    connect_tries: usize,
    /// What [`send`](Self::send) was given and no frame carries yet: kept across the start of a
    /// session, cleared at its end (ADR-0052).
    tx_queue: VecDeque<u8>,
    // per session
    top_acked_mode: Option<usize>,
    records: BTreeMap<u8, TxRecord>,
    tx_base: u8,
    tx_next: u8,
    /// The rung this station's data goes at: what the other station recommends.
    recommended: usize,
    peer_snr_db: Option<f64>,
    heard_db: Option<f64>,
    /// The readings of the last `family_readings` frames of the other's decoded.
    heard_recent: Vec<f64>,
    peer_capabilities: u8,
    peer_turnaround_s: f64,
    peer_identifier_s: f64,
    /// The master read the slave's DISC: the slave has left, and the master closes the session
    /// with its own so that the slave's closing identifier gets its slot (ADR-0075).
    peer_left: bool,
    disc_requested: bool,
    disc_tries: usize,
    patience_until: Option<f64>,
    /// When a disconnect was asked: by this station's operator, or — for the master — when it
    /// first read the slave's request (ADR-0072).
    leave_since: Option<f64>,
    ladder_pending: Option<(usize, Vec<u8>)>,
    rate: RateController,
    asked: Option<usize>,
    rx_base: u8,
    rx_buf: HashMap<u8, Vec<u8>>,
    max_seen: Option<u8>,
    harq: HashMap<u8, (HarqBuffer, usize, usize)>,
    /// The frames heard of the other station's transmission being received.
    rx: Vec<RxRecord>,
    rx_t0: Option<f64>,
    /// Where that transmission's data frames are, as its opening control frame stated: the
    /// first one's start, how many, their family.
    grid: Option<(f64, usize, bool)>,
    ack_history: Vec<AckSnapshot>,
    // the master
    level_asked: usize,
    silence_level: usize,
    silent: usize,
    /// Whether each of the last answer slots went silent, the latest last.
    recent_silent: VecDeque<bool>,
    answered_run: usize,
    /// Frames the slave said it has queued; `None` before its first answer.
    slave_want: Option<usize>,
    slave_leaves: bool,
    slave_id_due: bool,
    slave_floor_only: bool,
    hdr_floor: bool,
    /// Every family the slave may answer in when it has not read the header (R6).
    possible: Families,
    idle_windows: usize,
    ack_owed: bool,
    heard_at: f64,
    /// How far past the slot's slack the slave's answers have been seen to start: added to the
    /// wait for a slot nothing of which decoded, so it only ever delays the master.
    peer_late_s: f64,
    // the slave
    answer: Option<Answer>,
    /// The header of the master transmission being received, if it was read.
    grant: Option<GrantFrame>,
    grant_floor: bool,
    /// Where that transmission ends, as the frames read of it say: the slot's anchor.
    m_end: Option<f64>,
    m_last_floor: bool,
    /// The end of the last master transmission this station answered or forfeited.
    answered_end: Option<f64>,
    ans_sticky: bool,
    need_level: usize,
    on_time: usize,
    window: Option<(f64, f64, bool)>,
}

impl std::fmt::Debug for GrantedEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrantedEngine")
            .field("my_call", &self.my_call)
            .field("remote_call", &self.remote_call)
            .field("state", &self.state)
            .field("role", &self.role)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

fn rate_controller_for(timing: &PhyTiming, config: RateConfig) -> RateController {
    let controller = if timing.mode_threshold_db.is_empty() {
        RateController::new(config)
    } else {
        let frame_s: Vec<f64> = (0..timing.mode_threshold_db.len())
            .map(|m| timing.data_frame_s_for(m))
            .collect();
        RateController::for_table_timed(
            config,
            &timing.mode_threshold_db,
            &timing.data_capacity,
            &frame_s,
        )
    };
    controller.with_floor(timing.floor_modes, timing.floor_margin_db)
}

/// Whole steps of `step` in `x`, as the model's `int(x / step + 1e-9)`: truncated toward zero.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn whole(x: f64) -> usize {
    let n = (x + 1e-9).trunc();
    if n <= 0.0 { 0 } else { n as usize }
}

/// The model's `round`: to the nearest integer, ties to even.
#[allow(clippy::cast_possible_truncation)]
fn round_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

impl GrantedEngine {
    /// Build a station.
    #[must_use]
    pub fn new(my_call: &str, timing: PhyTiming, config: GrantConfig, seed: u64) -> Self {
        let call = my_call.trim().to_ascii_uppercase();
        let rate = rate_controller_for(&timing, config.rate);
        let mut engine = Self {
            callsigns: vec![call.clone()],
            my_call: call,
            remote_call: String::new(),
            stats: GrantStats::default(),
            recommended: config.initial_mode,
            timing,
            config,
            rng: Draws::new(seed),
            state: State::Idle,
            role: Role::None,
            session: 0,
            now: 0.0,
            actions: Vec::new(),
            deadlines: Deadlines::default(),
            cause: "",
            slot: None,
            next_slot_id: 0,
            in_flight: VecDeque::new(),
            hold_until: 0.0,
            left: None,
            leave_repeats: 0,
            call_jitter: 0.0,
            my_ident_s: 0.0,
            id_owed: 0.0,
            id_alone: false,
            id_listen: false,
            probing: None,
            probe_unsent: false,
            last_probe: None,
            ended_peer_snr_db: None,
            ladder: Vec::new(),
            pinned: None,
            pin_body: None,
            connect_tries: 0,
            tx_queue: VecDeque::new(),
            top_acked_mode: None,
            records: BTreeMap::new(),
            tx_base: 0,
            tx_next: 0,
            peer_snr_db: None,
            heard_db: None,
            heard_recent: Vec::new(),
            peer_capabilities: 0,
            peer_turnaround_s: 0.0,
            peer_identifier_s: 0.0,
            peer_left: false,
            disc_requested: false,
            disc_tries: 0,
            patience_until: None,
            leave_since: None,
            ladder_pending: None,
            rate,
            asked: None,
            rx_base: 0,
            rx_buf: HashMap::new(),
            max_seen: None,
            harq: HashMap::new(),
            rx: Vec::new(),
            rx_t0: None,
            grid: None,
            ack_history: Vec::new(),
            level_asked: 0,
            silence_level: 0,
            silent: 0,
            recent_silent: VecDeque::new(),
            answered_run: 0,
            slave_want: None,
            slave_leaves: false,
            slave_id_due: false,
            slave_floor_only: false,
            hdr_floor: true,
            possible: Families::of(true),
            idle_windows: 0,
            ack_owed: false,
            heard_at: 0.0,
            peer_late_s: 0.0,
            answer: None,
            grant: None,
            grant_floor: false,
            m_end: None,
            m_last_floor: false,
            answered_end: None,
            ans_sticky: true,
            need_level: 0,
            on_time: 0,
            window: None,
        };
        engine.reset_session();
        engine
    }

    // ── per-session state ─────────────────────────────────────────────

    fn reset_session(&mut self) {
        self.top_acked_mode = None;
        self.records.clear();
        self.tx_base = 0;
        self.tx_next = 0;
        self.recommended = self.config.initial_mode;
        self.peer_snr_db = None;
        self.heard_db = None;
        self.heard_recent.clear();
        self.peer_capabilities = 0;
        self.peer_turnaround_s = 0.0;
        self.peer_identifier_s = 0.0;
        self.peer_left = false;
        self.disc_requested = false;
        self.disc_tries = 0;
        self.patience_until = None;
        self.leave_since = None;
        self.ladder_pending = None;
        self.rate = rate_controller_for(&self.timing, self.config.rate);
        self.asked = None;
        self.rx_base = 0;
        self.rx_buf.clear();
        self.max_seen = None;
        self.harq.clear();
        self.rx.clear();
        self.rx_t0 = None;
        self.grid = None;
        self.ack_history.clear();
        self.slot = None;
        self.level_asked = 0;
        self.silence_level = 0;
        self.silent = 0;
        self.recent_silent.clear();
        self.answered_run = 0;
        self.slave_want = None;
        self.slave_leaves = false;
        self.slave_id_due = false;
        self.slave_floor_only = false;
        self.hdr_floor = true;
        self.possible = Families::of(true);
        self.idle_windows = 0;
        self.ack_owed = false;
        self.heard_at = self.now;
        self.peer_late_s = 0.0;
        self.answer = None;
        self.grant = None;
        self.grant_floor = false;
        self.m_end = None;
        self.m_last_floor = false;
        self.answered_end = None;
        self.ans_sticky = true;
        self.need_level = 0;
        self.on_time = 0;
        self.window = None;
        for timer in [Timer::Proceed, Timer::Answer, Timer::Link] {
            self.deadlines.remove(timer);
        }
    }

    // ── what a caller can read ────────────────────────────────────────

    /// Where a session is.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Which side of the session this station is.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// The session number, 0 outside one.
    #[must_use]
    pub const fn session(&self) -> u8 {
        self.session
    }

    /// The timing the engine runs on.
    #[must_use]
    pub const fn timing(&self) -> &PhyTiming {
        &self.timing
    }

    /// The configuration.
    #[must_use]
    pub const fn config(&self) -> &GrantConfig {
        &self.config
    }

    /// How the other station says it hears this one (every control frame carries it).
    #[must_use]
    pub const fn peer_snr_db(&self) -> Option<f64> {
        self.peer_snr_db
    }

    /// The last session's [`peer_snr_db`](Self::peer_snr_db), as it ended.
    #[must_use]
    pub const fn ended_peer_snr_db(&self) -> Option<f64> {
        self.ended_peer_snr_db
    }

    /// What the other station offered in the handshake, its level bits off.
    #[must_use]
    pub const fn peer_capabilities(&self) -> u8 {
        self.peer_capabilities
    }

    /// The rate controller's smoothed SNR and margin, for a display.
    #[must_use]
    pub fn rate_readings(&self) -> (Option<f64>, f64) {
        (self.rate.snr_db(), self.rate.margin_db())
    }

    /// What this station's last probe came back with.
    #[must_use]
    pub const fn last_probe(&self) -> Option<&ProbeResult> {
        self.last_probe.as_ref()
    }

    /// The fastest rung the other station acknowledged a frame of this session at.
    #[must_use]
    pub const fn top_acked_mode(&self) -> Option<usize> {
        self.top_acked_mode
    }

    /// The pinned transmissions' rungs reported so far, not yet taken.
    #[must_use]
    pub fn ladder(&self) -> &[LadderRung] {
        &self.ladder
    }

    /// Whether a session is up.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.state == State::Connected
    }

    /// The rung this station's data goes at.
    #[must_use]
    pub const fn current_mode(&self) -> usize {
        self.recommended
    }

    /// The gap level in use: what a master grants, what a slave asks for.
    #[must_use]
    pub fn level(&self) -> usize {
        if self.role == Role::Master {
            self.level_asked.max(self.silence_level)
        } else {
            self.asked_level()
        }
    }

    /// Whether a probe is out.
    #[must_use]
    pub const fn probing(&self) -> bool {
        self.probing.is_some()
    }

    /// Whether the station has asked to leave a session that is still up.
    #[must_use]
    pub fn disconnect_requested(&self) -> bool {
        self.disc_requested && self.state == State::Connected
    }

    /// When, at the latest, a disconnect asked for stops waiting for what is queued (engine
    /// time); the DISC goes at the master's next opportunity after it. `None` when none was
    /// asked. A slave's counts from its own request; the master's count starts when it reads
    /// it, an answer later.
    #[must_use]
    pub fn leave_by(&self) -> Option<f64> {
        if self.state != State::Connected {
            return None;
        }
        self.leave_since
            .map(|since| since + self.config.disc_drain_s)
    }

    /// Payload bytes queued or sent and not yet acknowledged.
    #[must_use]
    pub fn tx_pending_bytes(&self) -> usize {
        self.tx_queue.len()
            + self
                .records
                .values()
                .filter(|r| !r.acked)
                .map(|r| r.body.len())
                .sum::<usize>()
    }

    /// Payload bytes the other station does not hold in order yet: the queue and every frame
    /// from the lowest unacknowledged one up.
    #[must_use]
    pub fn tx_undelivered_bytes(&self) -> usize {
        self.tx_queue.len() + self.records.values().map(|r| r.body.len()).sum::<usize>()
    }

    /// Whether everything this station was given has been acknowledged.
    #[must_use]
    pub fn all_acknowledged(&self) -> bool {
        self.tx_queue.is_empty() && self.records.values().all(|r| r.acked)
    }

    /// The tries of the current (or last) call so far.
    #[must_use]
    pub const fn connect_tries(&self) -> usize {
        self.connect_tries
    }

    /// When the call's next try falls due, while it waits.
    #[must_use]
    pub fn connect_due(&self) -> Option<f64> {
        self.deadlines.get(Timer::Connect)
    }

    /// The earliest timer.
    #[must_use]
    pub fn next_deadline(&self) -> Option<f64> {
        self.deadlines.earliest()
    }

    /// The actions produced since the last call.
    pub fn drain(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// Start the counters again.
    pub fn reset_stats(&mut self) {
        self.stats = GrantStats::default();
    }

    /// The pinned transmissions' rungs reported so far, taken.
    pub fn take_ladder(&mut self) -> Vec<LadderRung> {
        std::mem::take(&mut self.ladder)
    }

    // ── public commands ───────────────────────────────────────────────

    /// The callsigns this station answers to; the first is the one it calls as.
    ///
    /// # Errors
    /// While a session is up, for an empty list, or for a callsign that cannot be packed.
    pub fn set_callsigns<S: AsRef<str>>(&mut self, calls: &[S]) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("a session is running");
        }
        let cleaned: Vec<String> = calls
            .iter()
            .map(|c| c.as_ref().trim().to_ascii_uppercase())
            .filter(|c| !c.is_empty())
            .collect();
        if cleaned.is_empty() {
            return Err("at least one callsign is needed");
        }
        if cleaned.iter().any(|c| pack_callsign(c).is_err()) {
            return Err("a callsign cannot be packed");
        }
        self.my_call.clone_from(&cleaned[0]);
        self.callsigns = cleaned;
        Ok(())
    }

    fn calling_as(&mut self, as_call: Option<&str>) -> Result<(), &'static str> {
        match as_call {
            None => {
                self.my_call.clone_from(&self.callsigns[0]);
                Ok(())
            }
            Some(call) => {
                let call = call.trim().to_ascii_uppercase();
                if self.callsigns.contains(&call) {
                    self.my_call = call;
                    Ok(())
                } else {
                    Err("not one of this station's callsigns")
                }
            }
        }
    }

    /// Call a station; the caller is the session's master once it reads the acceptance. The
    /// first try goes once the last slot this station granted can no longer be in use.
    ///
    /// # Errors
    /// In a session, for a callsign that cannot be packed, or calling as one this station
    /// does not answer to.
    pub fn connect(
        &mut self,
        remote_call: &str,
        as_call: Option<&str>,
    ) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("already in a session");
        }
        let remote = remote_call.trim().to_ascii_uppercase();
        if pack_callsign(&remote).is_err() {
            return Err("the callsign cannot be packed");
        }
        self.calling_as(as_call)?;
        if self.id_owed > 0.0 && remote == self.remote_call {
            // the station calls back the one it just lost: the communication goes on, and is
            // identified at the end of the session this call opens (ADR-0055 §4, ADR-0075)
            self.drop_identifier();
        } else if self.id_owed > 0.0 {
            // another station: the last communication's identifier goes now, ahead of the call
            self.deadlines
                .set(Timer::Ident, self.now.max(self.hold_until));
        }
        self.remote_call = remote;
        self.session = self.new_session_number();
        self.state = State::Connecting;
        self.role = Role::None;
        self.connect_tries = 0;
        self.left = None;
        self.deadlines.remove(Timer::Leave);
        self.reset_session();
        self.my_ident_s = self.ident_for(&self.my_call.clone());
        self.cause = "connect";
        // an identifier still owed to another station's session goes first: the call waits
        // for it, as for the last slot this station granted
        if self.now >= self.hold_until - 1e-9 && self.deadlines.get(Timer::Ident).is_none() {
            self.send_connect();
        } else {
            self.deadlines.set(Timer::Connect, self.hold_until);
        }
        Ok(())
    }

    fn new_session_number(&mut self) -> u8 {
        // 1–255, and never 0 in the control frame's seven bits: a header with no counts, at
        // level 0, and an SNR of 0 dB would be all zeros, which the PHY refuses
        loop {
            let session = u8::try_from(1 + self.rng.below(255)).unwrap_or(1);
            if session & 0x7F != 0 {
                return session;
            }
        }
    }

    /// Ask a station whether it hears this one, outside a session (ADR-0006). The probe grants
    /// its answer a slot, at the level this station's own turnaround needs (a probe carries no
    /// turnaround); the answer comes in it or not at all.
    ///
    /// # Errors
    /// In a session, with a probe out, for a callsign that cannot be packed, or asking as one
    /// this station does not answer to.
    pub fn probe(&mut self, remote_call: &str, as_call: Option<&str>) -> Result<(), &'static str> {
        if self.state != State::Idle {
            return Err("already in a session");
        }
        if self.probing.is_some() {
            return Err("a probe is already out");
        }
        let remote = remote_call.trim().to_ascii_uppercase();
        if pack_callsign(&remote).is_err() {
            return Err("the callsign cannot be packed");
        }
        self.calling_as(as_call)?;
        self.probing = Some(remote);
        if self.id_owed > 0.0 {
            // it goes first
            self.deadlines
                .set(Timer::Ident, self.now.max(self.hold_until));
        }
        self.last_probe = None;
        self.stats.probes_sent += 1;
        self.cause = "probe";
        self.probe_unsent = true;
        if self.now >= self.hold_until - 1e-9 && self.deadlines.get(Timer::Ident).is_none() {
            self.send_probe();
        } else {
            self.deadlines.set(Timer::Probe, self.hold_until);
        }
        Ok(())
    }

    /// Orderly close. The master finishes what it has queued, and what the slave has, for no
    /// longer than `disc_drain_s` from now (ADR-0072) or its patience with nothing moving
    /// (ADR-0039), then sends DISC; the slave asks to leave in its next answer and the master
    /// closes the session.
    pub fn disconnect(&mut self) {
        if self.state == State::Idle {
            return;
        }
        self.disc_requested = true;
        if self.leave_since.is_none() {
            self.leave_since = Some(self.now);
        }
        if self.role == Role::Slave && self.window.is_some() {
            self.cause = "disconnect";
            self.try_window();
        }
    }

    /// Drop the session now. A master says DISC from idle, on the floor, at its next
    /// opportunities, until one is answered or `leave_repeats` have gone; a slave, which never
    /// keys outside a slot, answers the old session's next header with a DISC in its slot
    /// (ADR-0052).
    pub fn abort(&mut self) {
        if self.state == State::Idle {
            return;
        }
        let was_master = self.role == Role::Master;
        self.cause = "abort";
        self.end_session("aborted", false);
        if was_master && self.left.is_some() {
            self.leave_repeats = self.config.leave_repeats;
            self.deadlines
                .set(Timer::Leave, self.now.max(self.hold_until));
        } else if was_master {
            self.identify_after_leaving();
        }
    }

    /// Queue bytes for the other station.
    pub fn send(&mut self, data: &[u8]) {
        self.tx_queue.extend(data.iter().copied());
        if self.role == Role::Slave && self.window.is_some() {
            self.cause = "send";
            self.try_window();
        }
    }

    /// The least time between the other station's frames and this station radiating
    /// ([`GrantConfig::answer_gap_s`]): the daemon's, for the station it is calling or in a
    /// session with. From the next key as the master, the next answer as the slave.
    pub const fn set_answer_gap(&mut self, seconds: f64) {
        self.config.answer_gap_s = seconds.max(0.0);
    }

    /// This station's own new data goes at `mode` until unpinned — the Test's ladder, at either
    /// end of a session.
    ///
    /// # Errors
    /// For a mode outside the table, or a body of no bytes.
    pub fn pin_mode(
        &mut self,
        mode: Option<usize>,
        body_bytes: Option<usize>,
    ) -> Result<(), &'static str> {
        if let Some(m) = mode
            && m >= self.timing.data_capacity.len()
        {
            return Err("mode is not in the table");
        }
        if body_bytes == Some(0) {
            return Err("body_bytes must be at least 1");
        }
        self.pinned = mode;
        self.pin_body = if mode.is_some() { body_bytes } else { None };
        Ok(())
    }

    /// Kept for the daemon's interface. Every answer states the slave's queue and the next
    /// header grants it, so the slave's data never waits for the master's to run out: there is
    /// no turn to break into.
    pub const fn request_break(&mut self) {}

    /// The slowest mode of a family that carries a connect body: what calls, acceptances,
    /// probes and their answers go at.
    ///
    /// # Panics
    /// If no mode of either family carries one.
    #[must_use]
    pub fn robust_mode(&self, floor: bool) -> usize {
        let need = CONNECT_BODY_8_BYTES + 5;
        for (mode, &bytes) in self.timing.data_capacity.iter().enumerate() {
            if self.timing.is_floor(mode) == floor && bytes >= need {
                return mode;
            }
        }
        assert!(floor, "no mode carries a connect frame");
        self.robust_mode(false)
    }

    /// Data frames of `frame_s` seconds one transmission may carry, one at least.
    #[must_use]
    pub fn burst_capacity(&self, frame_s: f64) -> usize {
        let mut count = self.config.max_data_frames.min(MAX_DATA);
        if let Some(limit) = self.config.max_burst_s
            && frame_s > 0.0
        {
            count = count.min(whole(limit / frame_s).max(1));
        }
        count
    }

    /// The longest a transmission may be on the air.
    pub const fn set_max_burst_s(&mut self, seconds: Option<f64>) {
        self.config.max_burst_s = seconds;
    }

    /// The fastest rung the rules allow.
    pub const fn set_ceiling(&mut self, rung: Option<usize>) {
        self.config.ceiling = rung;
    }

    /// [`set_ceiling`](Self::set_ceiling)'s rung.
    #[must_use]
    pub const fn ceiling(&self) -> Option<usize> {
        self.config.ceiling
    }

    /// The fastest rung this station uses, live.
    pub const fn set_max_mode(&mut self, rung: usize) {
        self.config.max_mode = rung;
    }

    /// How long after this station keys its radiation starts: the daemon's lead and the sound
    /// card's output latency, as the card reports it once it runs. What the engine keys an
    /// answer that much ahead of its slot by, and measures its own lateness against `KEY` with.
    pub const fn set_tx_latency_s(&mut self, seconds: f64) {
        self.timing.tx_latency_s = seconds;
    }

    /// This station's closing identifier's length for every callsign not given its own
    /// ([`GrantConfig::identifier_s`]): from the next call or acceptance on.
    pub const fn set_identifier_s(&mut self, seconds: f64) {
        self.config.identifier_s = seconds.max(0.0);
    }

    /// The closing identifier's length by callsign ([`GrantConfig::identifiers`]), 0 for a
    /// callsign that sends none: from the next call or acceptance on. A session keeps the length
    /// its call or acceptance declared, which is what the other station reckons with.
    pub fn set_identifiers<S: AsRef<str>>(&mut self, lengths: &[(S, f64)]) {
        self.config.identifiers = lengths
            .iter()
            .map(|(call, seconds)| (call.as_ref().trim().to_ascii_uppercase(), seconds.max(0.0)))
            .collect();
    }

    fn ident_for(&self, call: &str) -> f64 {
        self.config
            .identifiers
            .get(&call.to_ascii_uppercase())
            .copied()
            .unwrap_or(self.config.identifier_s)
    }

    /// Whether a session's closing identifier is owed and not yet handed over: the daemon tells
    /// a host program the session is over only once it has gone out (ADR-0075).
    #[must_use]
    pub fn identifier_owed(&self) -> bool {
        self.id_owed > 0.0
    }

    /// This station's radio: how long after it stops sending it hears again — what the next
    /// call or acceptance declares.
    pub const fn set_turnaround_s(&mut self, seconds: f64) {
        self.config.turnaround_s = seconds;
    }

    /// Another air from the next session on (ADR-0026).
    ///
    /// # Errors
    /// While a session, a call or a probe is running.
    pub fn set_air(
        &mut self,
        timing: PhyTiming,
        capabilities: u8,
        max_mode: Option<usize>,
    ) -> Result<(), &'static str> {
        if self.state != State::Idle || self.probing.is_some() {
            return Err("a session, a call or a probe is running");
        }
        self.move_to(timing, capabilities, max_mode);
        Ok(())
    }

    /// An unanswered call's next tries go out on another air, its tries and timers standing
    /// (ADR-0058).
    ///
    /// # Errors
    /// When no call is going out.
    pub fn move_call(
        &mut self,
        timing: PhyTiming,
        capabilities: u8,
        max_mode: Option<usize>,
    ) -> Result<(), &'static str> {
        if self.state != State::Connecting {
            return Err("no call is going out");
        }
        self.move_to(timing, capabilities, max_mode);
        Ok(())
    }

    fn move_to(&mut self, timing: PhyTiming, capabilities: u8, max_mode: Option<usize>) {
        self.timing = timing;
        self.config.capabilities = capabilities;
        if let Some(rung) = max_mode {
            self.config.max_mode = rung;
        }
        self.rate = rate_controller_for(&self.timing, self.config.rate);
        self.recommended = self.config.initial_mode;
    }

    // ── inputs ────────────────────────────────────────────────────────

    /// A transmission finished. `frames_end` is where its last frame ended on the air — the
    /// card's clock less the keying tail; `now` when not given. The slot a transmission granted
    /// is measured from it, as the other station measures it from the frames it hears end.
    pub fn on_tx_done(&mut self, now: f64, frames_end: Option<f64>) {
        self.now = self.now.max(now);
        let Some((granted, _)) = self.in_flight.pop_front() else {
            return;
        };
        let current = self.slot.as_ref().map(|s| (s.id, s.final_));
        if let (Some(id), Some((slot_id, false))) = (granted, current)
            && id == slot_id
        {
            if let Some(slot) = self.slot.as_mut() {
                slot.final_ = true;
            }
            self.open_slot(frames_end.unwrap_or(now));
        }
    }

    /// A transmission was cut — an abort, the key watchdog — before any frame of it could be
    /// read: its first frame a control frame not [`HEADER_UNDECODABLE_SHARE`] of the way out
    /// (10-step-3.md §3.6). The other station learned nothing from it, so it granted nothing.
    /// No slot waits on it, and what waited for this station's own transmission to end — the
    /// master's next key, an abort's DISC — waits only for the cut. Reported as ending where
    /// its frames would have ([`Self::on_tx_done`]), it held a Test's leaving DISC 29 s for the
    /// slot of a header nobody had read.
    pub fn on_tx_cut_unread(&mut self, now: f64) {
        self.now = self.now.max(now);
        let Some((granted, ident)) = self.in_flight.pop_front() else {
            return;
        };
        self.owe_again(ident);
        let current = self.slot.as_ref().map(|s| (s.id, s.final_));
        if let (Some(id), Some((slot_id, false))) = (granted, current)
            && id == slot_id
        {
            self.slot = None;
            self.hold_until = self.now;
            if self.role == Role::Master
                && matches!(self.state, State::Connected | State::Disconnecting)
            {
                self.deadlines.set(Timer::Proceed, self.now);
            }
        }
    }

    /// The keying gate did not start a transmission: it would have begun past its `not_after`,
    /// and nothing of it was sent.
    pub fn on_tx_dropped(&mut self, now: f64) {
        self.now = self.now.max(now);
        if let Some((_, ident)) = self.in_flight.pop_front() {
            self.owe_again(ident);
        }
        self.stats.dropped += 1;
    }

    /// The daemon is holding this station's last transmission. A slot is absolute and a late
    /// answer is dropped by the gate, so what moves is only what waits for this station's own
    /// transmission to end: the master's next key, a call's next try, a probe's wait, a leaving
    /// DISC's repeat.
    pub fn on_tx_delayed(&mut self, seconds: f64) {
        if seconds <= 0.0 {
            return;
        }
        for entry in &mut self.deadlines.0 {
            if !matches!(entry.0, Timer::Link | Timer::Answer) {
                entry.1 += seconds;
            }
        }
        self.hold_until += seconds;
    }

    /// Time passes.
    pub fn tick(&mut self, now: f64) {
        self.now = self.now.max(now);
        let mut order = self.deadlines.0.clone();
        // stable, so timers due together keep the order they were first armed in
        order.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (timer, _) in order {
            if self.deadlines.get(timer).unwrap_or(f64::INFINITY) <= self.now + 1e-9 {
                self.deadlines.remove(timer);
                self.cause = timer.cause();
                self.expire(timer);
            }
        }
    }

    /// A frame announced and not yet decoded. Inside a session it changes nothing: only decoded
    /// content does (ADR-0069 §2.5). A caller or a prober takes it as carrier — its next try,
    /// or its giving up, waits past the frame, which may be the answer.
    pub fn on_preamble(&mut self, t_start: f64, now: f64, frame_s: Option<f64>) {
        self.now = self.now.max(now);
        if self.state != State::Connecting && self.probing.is_none() {
            return;
        }
        let length = frame_s.unwrap_or_else(|| self.timing.data_frame_s_for(0));
        let clear = t_start + length + self.budget(true) + self.config.guard_s;
        if let Some(due) = self.deadlines.get(Timer::Connect)
            && self.connect_tries > 0
        {
            self.deadlines.set(Timer::Connect, due.max(clear));
        }
        if let Some(due) = self.deadlines.get(Timer::Probe)
            && !self.probe_unsent
        {
            self.deadlines.set(Timer::Probe, due.max(clear));
        }
    }

    /// A frame detected, decoded or not.
    pub fn on_frame<F: SoftFrame>(&mut self, frame: &F, now: f64) {
        self.now = self.now.max(now);
        self.cause = "frame";
        let heard = Heard::of(frame);
        let (payload, _) = frame.decode(None);
        if frame.container() == Container::Control {
            // a control frame not read changes nothing
            if let Some(payload) = payload
                && let Ok(ctl) = GrantFrame::decode(&payload)
            {
                self.on_control(&ctl, &heard);
            }
            return;
        }
        if let Some(payload) = &payload
            && !self.outside(payload, &heard)
        {
            return;
        }
        if !matches!(self.state, State::Connected | State::Disconnecting) {
            return;
        }
        if payload.is_none() && !heard.trusted {
            return; // untrusted and unread: nothing at all
        }
        if heard.floor != self.timing.is_floor(heard.mode) {
            self.stats.frames_failed += 1; // the chips are noise
        } else if self.role == Role::Master {
            self.master_data(frame, &heard, payload.is_some());
        } else if self.role == Role::Slave {
            self.slave_data(frame, &heard, payload.is_some());
        }
    }

    /// Handle a decoded DATA frame that is no session data — a call, an acceptance, a probe or
    /// its answer — and `true` only for this session's data.
    fn outside(&mut self, payload: &[u8], frame: &Heard) -> bool {
        let Ok((header, body)) = decode_data(payload) else {
            return false;
        };
        match header.kind {
            DataKind::ConnectReq | DataKind::ConnectAck => {
                self.on_connect(header, &body, frame);
                false
            }
            DataKind::Probe | DataKind::ProbeAck => {
                self.on_probe(header.kind, &body, frame);
                false
            }
            // a beacon or a datagram is nobody's session's, whatever its session byte says
            kind => !kind.outside_sessions() && header.session == self.session,
        }
    }

    // ── timers ────────────────────────────────────────────────────────

    fn expire(&mut self, timer: Timer) {
        match timer {
            Timer::Link => {
                // whole exchanges of the size in use now: the answer that last armed the
                // timer may have moved the link to the floor, whose exchanges are three times
                // as long
                let due = self.heard_at + self.link_timeout();
                if due > self.now {
                    self.deadlines.set(Timer::Link, due);
                } else {
                    self.end_session("link timeout", false);
                }
            }
            Timer::Connect => {
                if self.now < self.hold_until - 1e-9 {
                    // a closing identifier went first
                    self.deadlines.set(Timer::Connect, self.hold_until);
                } else {
                    self.retry_connect();
                }
            }
            Timer::Probe => {
                if self.probe_unsent && self.now < self.hold_until - 1e-9 {
                    self.deadlines.set(Timer::Probe, self.hold_until);
                } else if self.probe_unsent {
                    self.send_probe();
                } else if let Some(remote) = self.probing.take() {
                    self.event("probe", format!("{remote}: no answer"));
                }
            }
            Timer::Proceed => self.proceed(),
            Timer::Answer => self.key_answer(),
            Timer::Leave => self.repeat_leave(),
            Timer::Ident => self.send_identifier(),
        }
    }

    fn event(&mut self, name: &'static str, detail: String) {
        self.actions.push(Action::Event { name, detail });
    }

    // ── timing ────────────────────────────────────────────────────────

    fn budget(&self, floor: bool) -> f64 {
        self.config.decode_budget_s[usize::from(floor)]
    }

    const fn own_latency(&self) -> f64 {
        self.timing.tx_latency_s
    }

    /// Where the slot granted by a transmission ending at `end` starts, as both stations
    /// reckon it (07-protocol-8.md §3.1).
    fn slot_start(&self, end: f64, floor: bool, level: usize, granter_turnaround: f64) -> f64 {
        let cfg = &self.config;
        let gap = (self.budget(floor) + cfg.key_s).max(granter_turnaround) + cfg.guard_s;
        end + gap + cfg.levels_s[level.min(cfg.levels_s.len() - 1)]
    }

    /// The earliest this station keys after the other's transmission ends at `end`: with its
    /// last frame decoded, and radiating no sooner than the other hears again (§3.2). The
    /// keying latency counted is this station's own, never more than the protocol's.
    fn after(&self, end: f64, floor: bool, other_turnaround: f64) -> f64 {
        let lead = self.config.key_s.min(self.own_latency());
        end + self.budget(floor).max(other_turnaround - lead) + self.config.guard_s
    }

    /// How long after the other station's frames this station's radiation waits: the other's
    /// declared turnaround, or the answer gap when that is longer.
    const fn peer_wait_s(&self) -> f64 {
        self.peer_turnaround_s.max(self.config.answer_gap_s)
    }

    /// The gap level this station asks for as the slave: what its own lateness needs, and at
    /// least what starts its slots the answer gap after the master's frames.
    fn asked_level(&self) -> usize {
        let gap = self.config.answer_gap_s;
        if gap <= 0.0 {
            return self.need_level;
        }
        let base = [false, true]
            .into_iter()
            .map(|floor| (self.budget(floor) + self.config.key_s).max(self.peer_turnaround_s))
            .fold(f64::INFINITY, f64::min);
        self.need_level
            .max(self.level_for(gap - base - self.config.guard_s))
    }

    /// The smallest gap level at least `extra_s` long.
    fn level_for(&self, extra_s: f64) -> usize {
        self.config
            .levels_s
            .iter()
            .position(|&step| extra_s <= step + 1e-9)
            .unwrap_or(self.config.levels_s.len() - 1)
    }

    fn ctl_s(&self, floor: bool) -> f64 {
        self.timing.control_frame_s_for(floor)
    }

    fn frame_s(&self, floor: bool) -> f64 {
        match self.timing.floor_data_frame_s {
            Some(long) if floor => long,
            _ => self.timing.data_frame_s,
        }
    }

    fn robust_frame_s(&self, floor: bool) -> f64 {
        self.timing.data_frame_s_for(self.robust_mode(floor))
    }

    fn window_s(&self, floor: bool, index: usize) -> f64 {
        self.config.open_window_s[usize::from(floor)] * f64::from(1u32 << index.min(16))
    }

    /// One whole exchange of the size in use: two full transmissions at the rungs the link runs
    /// at with their control frames, the gaps at the highest level and the slack — or an idle
    /// exchange's longest open window, whichever is longer.
    #[allow(clippy::cast_precision_loss)]
    fn exchange_s(&self) -> f64 {
        let burst_floor = self.timing.is_floor(self.burst_mode());
        let recommended_floor = self.timing.is_floor(self.rate.recommend());
        let floor = burst_floor || recommended_floor || self.hdr_floor;
        let frame = self
            .frame_s(burst_floor)
            .max(self.frame_s(recommended_floor));
        let gap = self.slot_start(
            0.0,
            true,
            self.config.levels_s.len() - 1,
            self.peer_turnaround_s,
        ) + self.config.slack_s;
        let busy = 2.0 * (self.ctl_s(floor) + self.burst_capacity(frame) as f64 * frame + gap);
        let idle = self.ctl_s(true) + self.window_s(true, self.config.window_doublings) + 2.0 * gap;
        busy.max(idle)
    }

    fn link_timeout(&self) -> f64 {
        self.config
            .link_timeout_s
            .max(self.config.link_timeout_exchanges * self.exchange_s())
    }

    fn disc_patience(&self) -> f64 {
        self.config
            .disc_patience_s
            .max(self.config.disc_patience_exchanges * self.exchange_s())
    }

    /// A decoded frame of the session: the link is alive.
    fn heard(&mut self, frame: &Heard) {
        self.heard_db = Some(frame.snr_db);
        self.heard_recent.push(frame.snr_db);
        let keep = self.config.family_readings;
        if self.heard_recent.len() > keep {
            self.heard_recent.drain(..self.heard_recent.len() - keep);
        }
        self.heard_at = self.now;
        self.deadlines
            .set(Timer::Link, self.now + self.link_timeout());
    }

    // ── rate and family ───────────────────────────────────────────────

    fn cap(&self) -> usize {
        self.config
            .ceiling
            .map_or(self.config.max_mode, |c| self.config.max_mode.min(c))
    }

    fn floor_only(&self) -> bool {
        self.config.ceiling.is_some_and(|c| self.timing.is_floor(c))
    }

    fn burst_mode(&self) -> usize {
        self.pinned.unwrap_or(self.recommended).min(self.cap())
    }

    fn ctl_threshold(&self) -> f64 {
        self.timing
            .control_threshold_db
            .unwrap_or(CONTROL_THRESHOLD_DB)[0]
    }

    /// Whether this station needs the other's control frames on the floor: it hears the other
    /// under the ordinary control frame's threshold plus a margin, or not yet at all.
    fn wants_floor(&self) -> bool {
        if self.timing.floor_modes == 0 {
            return false;
        }
        if self.floor_only() || self.heard_recent.is_empty() {
            return true;
        }
        let weakest = self
            .heard_recent
            .iter()
            .copied()
            .reduce(f64::min)
            .unwrap_or(f64::NEG_INFINITY);
        weakest < self.ctl_threshold() + self.config.answer_floor_margin_db
    }

    /// The rung this station's control frame recommends for the other's data, remembered
    /// (ADR-0020). `floor_only`: the other station has said its data can only go on the tone
    /// floor, and a header's rung is also what its grant is sized by, so it stays there.
    fn recommend(&mut self, floor_only: bool) -> usize {
        let mut rung = self.rate.recommend();
        if floor_only && self.timing.floor_modes > 0 {
            rung = rung.min(self.timing.floor_modes - 1);
        }
        self.asked = Some(self.asked.map_or(rung, |a| a.max(rung)));
        rung
    }

    fn back_off(&mut self) {
        let modes = self.rate.modes().to_vec();
        let current = self.recommended.min(self.cap());
        let index = modes.iter().rposition(|&m| m <= current).unwrap_or(0);
        self.recommended = modes[index.saturating_sub(self.config.silence_step)];
    }

    #[allow(clippy::cast_precision_loss)]
    fn hopeless(&self, mode: usize) -> bool {
        let (Some(peer), Some(threshold)) = (self.peer_snr_db, self.rate.threshold_db(mode)) else {
            return false;
        };
        let reach = threshold - 10.0 * (self.config.max_combines.max(1) as f64).log10();
        peer < reach - self.config.reencode_hopeless_db
    }

    fn fits(&self, body_len: usize, mode: usize) -> bool {
        let cap = data_capacity(self.timing.capacity(mode));
        body_len == cap || body_len + 2 <= cap
    }

    fn reencode_target(&self, body_len: usize, from_mode: usize, to_mode: usize) -> Option<usize> {
        (0..self.timing.data_capacity.len())
            .find(|&mode| to_mode <= mode && mode < from_mode && self.fits(body_len, mode))
    }

    // ── keying ────────────────────────────────────────────────────────

    /// Hand a transmission over; returns where its frames end on the air, as far as this engine
    /// knows.
    /// Hand a transmission over; returns where it ends on the air, as far as this engine knows.
    /// An identifier goes after the frames (ADR-0075).
    fn transmit(&mut self, frames: Vec<TxFrame>, keying: Keying, granting: Option<u64>) -> f64 {
        // the frames' air time first, then the identifier: the model's association
        let frames_s: f64 = frames.iter().map(|f| self.timing.frame_s(f)).sum();
        let dur = keying.identifier_s + frames_s;
        let carried = if frames.is_empty() {
            0.0
        } else {
            keying.identifier_s
        };
        self.actions.push(Action::Transmit {
            frames,
            duration_s: dur,
            cause: self.cause,
            not_after: keying.not_after,
            not_before: keying.not_before,
            identifier_s: keying.identifier_s,
            listen_first: keying.listen_first,
        });
        self.in_flight.push_back((granting, carried));
        self.stats.transmissions += 1;
        let mut on_air = self.now + self.own_latency();
        if let Some(earliest) = keying.not_before {
            on_air = on_air.max(earliest);
        }
        on_air + dur
    }

    fn control(ctl: &GrantFrame, floor: bool) -> TxFrame {
        TxFrame {
            container: Container::Control,
            payload: ctl.encode().to_vec(),
            mode: 0,
            rv: 0,
            floor,
            follows: 0,
        }
    }

    // ── slots this station grants ─────────────────────────────────────

    /// Key a transmission that grants a slot, and wait for the slot — placed from this engine's
    /// estimate of the transmission's end until `on_tx_done` says where it was.
    fn key_granting(&mut self, mut slot: Slot, frames: Vec<TxFrame>) {
        self.next_slot_id += 1;
        slot.id = self.next_slot_id;
        let id = slot.id;
        self.slot = Some(slot);
        let end = self.transmit(frames, Keying::default(), Some(id));
        self.open_slot(end);
    }

    fn open_slot(&mut self, end: f64) {
        let Some(slot) = self.slot.as_ref() else {
            return;
        };
        let start = self.slot_start(end, slot.last_floor, slot.level, slot.granter_turnaround_s);
        let end_max = start
            + slot.grant_s
            + if slot.open_window {
                0.0
            } else {
                self.config.slack_s
            };
        if let Some(slot) = self.slot.as_mut() {
            slot.tx_end = end;
            slot.start = start;
            slot.end_max = end_max;
        }
        self.arm_wait();
    }

    /// When this station may key again: a gap after the end a decoded frame of the slot stated,
    /// or after the slot's latest end when nothing of it was decoded (§3.2).
    fn slot_close(&self, slot: &Slot) -> f64 {
        if let Some(end) = slot.stated_end {
            return self.after(end, slot.stated_floor, self.peer_wait_s());
        }
        let end = slot.end_max + self.peer_late_s;
        self.after(end, slot.answer_floors.floor, self.peer_wait_s())
    }

    /// Arm what ends this station's wait for the slot it granted last.
    fn arm_wait(&mut self) {
        let Some(slot) = self.slot.as_ref() else {
            return;
        };
        let due = self.slot_close(slot);
        let kind = slot.kind;
        self.hold_until = due;
        if kind == SlotKind::Connect && self.state == State::Connecting {
            self.deadlines.set(Timer::Connect, due + self.call_jitter);
        } else if kind == SlotKind::Probe {
            if self.probing.is_some() && !self.probe_unsent {
                self.deadlines.set(Timer::Probe, due);
            }
        } else if self.role == Role::Master
            && matches!(self.state, State::Connected | State::Disconnecting)
        {
            self.deadlines.set(Timer::Proceed, self.now.max(due));
        }
    }

    /// A frame of the other station's is of the slot this station granted last if it began
    /// after the granting transmission ended: the other station keys only in a slot.
    fn in_slot(&self, frame: &Heard) -> bool {
        self.slot
            .as_ref()
            .is_some_and(|s| s.final_ && frame.t_start >= s.tx_end - 1e-9)
    }

    /// A decoded frame of the slot says where the other station's transmission ends.
    fn state_end(&mut self, end: f64, floor: bool) {
        let Some(slot) = self.slot.as_mut() else {
            return;
        };
        slot.heard = true;
        if slot.stated_end.is_none_or(|stated| end >= stated - 1e-9) {
            slot.stated_end = Some(end);
            slot.stated_floor = floor;
        }
        self.arm_wait();
    }

    // ── the call and the probe ────────────────────────────────────────

    fn connect_floor(&self) -> bool {
        if self.timing.floor_modes > 0 && self.floor_only() {
            return true;
        }
        self.timing.floor_modes > 0 && self.connect_tries % 2 == 0
    }

    /// The gap level a call grants its acceptance: up one every second try, so a called station
    /// whose computer decodes late is reached (capability bits 3–4).
    fn call_level(&self) -> usize {
        (self.config.levels_s.len() - 1).min(self.connect_tries / 2)
    }

    fn families(&self) -> Families {
        if self.timing.floor_modes > 0 {
            Families::both()
        } else {
            Families::of(false)
        }
    }

    fn data_frame(payload: Vec<u8>, mode: usize) -> TxFrame {
        TxFrame {
            container: Container::Data,
            payload,
            mode,
            rv: 0,
            floor: false,
            follows: 0,
        }
    }

    /// A try of the call: on the tone floor first, the families alternating (ADR-0016); the
    /// acceptance's slot at the try's level; the next try once that slot can no longer be in
    /// use, and a random part of a floor frame per try later, so that two stations calling each
    /// other fall apart.
    fn send_connect(&mut self) {
        let floor = self.connect_floor();
        let level = self.call_level();
        let body = ConnectBody {
            src: self.my_call.clone(),
            dst: self.remote_call.clone(),
            caps: with_level(self.config.capabilities, u8::try_from(level).unwrap_or(3)),
            version: PROTOCOL_VERSION,
            snr_db: None,
            turnaround_s: self.config.turnaround_s,
            identifier_s: self.my_ident_s,
        };
        let mode = self.robust_mode(floor);
        let header = DataHeader {
            kind: DataKind::ConnectReq,
            seq: 0,
            session: self.session,
        };
        let Ok(body) = body.encode() else { return };
        let Ok(payload) = encode_data(&header, &body, self.timing.capacity(mode)) else {
            return;
        };
        self.connect_tries += 1;
        #[allow(clippy::cast_precision_loss)]
        let spread = (1 + self.connect_tries) as f64 * self.robust_frame_s(true);
        self.call_jitter = self.rng.uniform(0.0, spread);
        // the acceptance comes in the request's family, or on the floor when the call arrived
        // weak (ADR-0056) or the rules allow only the floor
        let families = self.families();
        let mut slot = Slot::new(
            SlotKind::Connect,
            families.max_of(|f| self.robust_frame_s(f)),
            self.timing.is_floor(mode),
            families,
        );
        slot.level = level;
        slot.granter_turnaround_s = self.config.turnaround_s;
        self.key_granting(slot, vec![Self::data_frame(payload, mode)]);
    }

    fn retry_connect(&mut self) {
        if self.state != State::Connecting {
            return;
        }
        if self.connect_tries >= self.config.connect_retries {
            self.end_session("no answer", false);
            return;
        }
        self.send_connect();
    }

    /// The probe goes on the tone floor (ADR-0016). It carries no turnaround: its level covers
    /// this station's own.
    fn send_probe(&mut self) {
        self.probe_unsent = false;
        let Some(remote) = self.probing.clone() else {
            return;
        };
        let level =
            self.level_for(self.config.turnaround_s - self.budget(true) - self.config.key_s);
        let body = ProbeBody {
            src: self.my_call.clone(),
            dst: remote,
            snr_db: None,
            caps: with_level(self.config.capabilities, u8::try_from(level).unwrap_or(3)),
        };
        let mode = self.robust_mode(true);
        let header = DataHeader {
            kind: DataKind::Probe,
            seq: 0,
            session: 0,
        };
        let Ok(body) = body.encode() else { return };
        let Ok(payload) = encode_data(&header, &body, self.timing.capacity(mode)) else {
            return;
        };
        let families = self.families();
        let mut slot = Slot::new(
            SlotKind::Probe,
            families.max_of(|f| self.robust_frame_s(f)),
            self.timing.is_floor(mode),
            families,
        );
        slot.level = level;
        self.key_granting(slot, vec![Self::data_frame(payload, mode)]);
    }

    fn on_connect(&mut self, header: DataHeader, body: &[u8], frame: &Heard) {
        let Ok(cb) = ConnectBody::decode(body) else {
            return;
        };
        if header.kind == DataKind::ConnectReq {
            self.on_request(header, &cb, frame, body.len());
        } else {
            self.on_acceptance(header, &cb, frame, body.len());
        }
    }

    fn version_ok(&mut self, cb: &ConnectBody, size: usize, verb: &str) -> bool {
        if cb.version == PROTOCOL_VERSION && size >= CONNECT_BODY_8_BYTES {
            return true;
        }
        self.event(
            "ignored",
            format!("{} {verb} with link protocol {}", cb.src, cb.version),
        );
        false
    }

    fn on_request(&mut self, header: DataHeader, req: &ConnectBody, frame: &Heard, size: usize) {
        if !self.callsigns.contains(&req.dst) {
            return;
        }
        if bandwidth_code(req.caps) != bandwidth_code(self.config.capabilities) {
            let mismatch = self.mismatch(req.caps);
            self.event(
                "ignored",
                format!("{} calls in another bandwidth{mismatch}", req.src),
            );
            return;
        }
        if !self.version_ok(req, size, "calls") {
            return;
        }
        let start = self.slot_start(
            frame.t_end,
            frame.floor,
            usize::from(level_of(req.caps)),
            req.turnaround_s,
        );
        if matches!(self.state, State::Connected | State::Disconnecting)
            && !self.called_again(header, req, frame, start)
        {
            return;
        }
        if start < self.hold_until - 1e-9 || self.probing.is_some() {
            return; // its slot would fall in one this station granted, still open
        }
        if self.state == State::Connecting {
            if req.src == self.remote_call && self.my_call > self.remote_call {
                return; // simultaneous calls: the higher callsign keeps calling
            }
            self.deadlines.remove(Timer::Connect);
        }
        self.accept_call(header, req, frame, start);
    }

    /// A call heard in a session. The peer's under a new session number has given the session
    /// up (ADR-0056): the session ends, `true`, and the call is answered as from idle. The
    /// peer's under this session's number is a try whose acceptance was not read: answered
    /// again, in its own slot, with the SNR it arrived at (P9-2).
    fn called_again(
        &mut self,
        header: DataHeader,
        req: &ConnectBody,
        frame: &Heard,
        start: f64,
    ) -> bool {
        if req.src != self.remote_call {
            return false; // another station's call: this one is in a session
        }
        if header.session != self.session {
            self.end_session(&format!("{} called again", req.src), false);
            return true;
        }
        if self.role == Role::Slave && self.m_end.is_none() && self.grant.is_none() {
            self.heard(frame);
            self.measure(frame);
            self.schedule_acceptance(req, frame, start);
        }
        false
    }

    fn accept_call(&mut self, header: DataHeader, req: &ConnectBody, frame: &Heard, start: f64) {
        self.my_call.clone_from(&req.dst);
        // an identifier still owed for an earlier session is not sent now: an acceptance cannot
        // wait for it, and this session ends with one (ADR-0075)
        self.drop_identifier();
        self.remote_call.clone_from(&req.src);
        self.session = header.session;
        self.left = None;
        self.deadlines.remove(Timer::Leave);
        self.reset_session();
        self.my_ident_s = self.ident_for(&req.dst);
        self.peer_capabilities = req.caps & !CAP_LEVEL_MASK;
        self.peer_turnaround_s = req.turnaround_s;
        self.peer_identifier_s = req.identifier_s;
        self.state = State::Connected;
        self.role = Role::Slave;
        self.heard(frame);
        self.measure(frame);
        // the request is the first measurement of how the caller is heard; a lower bound if it
        // came on the tone floor (ADR-0016)
        self.rate.seed(frame.snr_db, frame.floor);
        self.ans_sticky = self.acceptance_floor(frame);
        let remote = self.remote_call.clone();
        self.event("connected", format!("{remote} (slave)"));
        self.event("role", "slave".to_owned());
        self.schedule_acceptance(req, frame, start);
    }

    /// The acceptance goes back in the request's family — or on the floor when an ordinary call
    /// arrived too weak to bet the session on an ordinary acceptance making it back (ADR-0056),
    /// or when the rules allow only the floor.
    fn acceptance_floor(&self, frame: &Heard) -> bool {
        if frame.floor || self.floor_only() {
            return self.timing.floor_modes > 0;
        }
        if self.timing.floor_modes == 0 {
            return false;
        }
        self.rate
            .threshold_db(self.robust_mode(false))
            .is_some_and(|t| frame.snr_db < t + self.config.floor_answer_margin_db)
    }

    fn schedule_acceptance(&mut self, req: &ConnectBody, frame: &Heard, start: f64) {
        let mut answer = Answer::new(
            AnswerKind::ConnectAck,
            start,
            start + self.config.slack_s,
            self.acceptance_floor(frame),
        );
        answer.session = self.session;
        answer.anchor = frame.t_end;
        answer.remote.clone_from(&req.src);
        answer.snr_db = Some(frame.snr_db);
        self.schedule(answer);
    }

    fn on_acceptance(&mut self, header: DataHeader, ack: &ConnectBody, frame: &Heard, size: usize) {
        if self.state != State::Connecting || header.session != self.session {
            return;
        }
        if ack.dst != self.my_call {
            return;
        }
        if bandwidth_code(ack.caps) != bandwidth_code(self.config.capabilities) {
            self.event(
                "ignored",
                format!("{} answers in another bandwidth", ack.src),
            );
            return;
        }
        if !self.version_ok(ack, size, "answers") {
            return;
        }
        self.deadlines.remove(Timer::Connect);
        self.peer_capabilities = ack.caps & !CAP_LEVEL_MASK;
        self.peer_turnaround_s = ack.turnaround_s;
        self.peer_identifier_s = ack.identifier_s;
        // an acceptance's level bits say the gap level the slave needs, measured on the call
        self.level_asked = usize::from(level_of(ack.caps));
        self.state = State::Connected;
        self.role = Role::Master;
        self.heard(frame);
        self.rate.seed(frame.snr_db, frame.floor);
        self.recommended = self.config.initial_mode;
        if let Some(snr) = ack.snr_db {
            let first = self.rate.first_mode(snr);
            self.recommended = self.config.initial_mode.max(self.cap().min(first));
            self.peer_snr_db = Some(snr);
            // the first header goes in the family the slave's reading of the call asks for: a
            // floor reading is a lower bound, never too fast
            if self.timing.floor_modes > 0 {
                self.hdr_floor = self.floor_only()
                    || snr < self.ctl_threshold() + self.config.answer_floor_margin_db;
            }
        }
        self.possible = Families::of(frame.floor);
        let remote = self.remote_call.clone();
        self.event("connected", format!("{remote} (master)"));
        self.event("role", "master".to_owned());
        if self.slot.is_some() && self.in_slot(frame) {
            self.state_end(frame.t_end, frame.floor);
        } else {
            // an earlier try's acceptance, read late: the latest try's slot may be in use
            self.arm_wait();
        }
    }

    fn on_probe(&mut self, kind: DataKind, body: &[u8], frame: &Heard) {
        let Ok(pb) = ProbeBody::decode(body) else {
            return;
        };
        if kind == DataKind::Probe {
            if !self.callsigns.contains(&pb.dst) || self.state != State::Idle {
                return; // a session's frames matter more than a question from outside it
            }
            if self.probing.is_some() {
                return;
            }
            let start = self.slot_start(
                frame.t_end,
                frame.floor,
                usize::from(level_of(pb.caps)),
                0.0,
            );
            if start < self.hold_until - 1e-9 {
                return;
            }
            // answered across bandwidths too, on the floor both airs share (ADR-0035)
            let across = bandwidth_code(pb.caps) != bandwidth_code(self.config.capabilities);
            self.my_call.clone_from(&pb.dst);
            self.stats.probes_answered += 1;
            let mismatch = self.mismatch(pb.caps);
            self.event(
                "probed",
                format!("{} at {:.1} dB{mismatch}", pb.src, frame.snr_db),
            );
            let mut answer = Answer::new(
                AnswerKind::ProbeAck,
                start,
                start + self.config.slack_s,
                self.timing.floor_modes > 0 && (across || frame.floor || self.floor_only()),
            );
            answer.remote = pb.src;
            answer.snr_db = Some(frame.snr_db);
            answer.anchor = frame.t_end;
            self.schedule(answer);
            return;
        }
        if self.probing.as_deref() != Some(pb.src.as_str()) || pb.dst != self.my_call {
            return;
        }
        self.deadlines.remove(Timer::Probe);
        self.probing = None;
        self.stats.probe_replies += 1;
        self.last_probe = Some(ProbeResult {
            remote: pb.src.clone(),
            heard_there_db: pb.snr_db,
            heard_here_db: frame.snr_db,
            bandwidth_hz: bandwidth_hz_of(pb.caps),
        });
        let theirs = pb
            .snr_db
            .map_or_else(|| "?".to_owned(), |snr| format!("{snr:.0}"));
        let mismatch = self.mismatch(pb.caps);
        self.event(
            "probe",
            format!(
                "{} hears us at {theirs} dB, heard at {:.1} dB{mismatch}",
                pb.src, frame.snr_db
            ),
        );
        if self
            .slot
            .as_ref()
            .is_some_and(|s| s.kind == SlotKind::Probe)
            && self.in_slot(frame)
        {
            self.state_end(frame.t_end, frame.floor);
        }
    }

    fn mismatch(&self, caps: u8) -> String {
        if bandwidth_code(caps) == bandwidth_code(self.config.capabilities) {
            return String::new();
        }
        let named = |hz: Option<usize>| {
            hz.map_or_else(
                || "an unknown bandwidth".to_owned(),
                |hz| format!("{hz} Hz"),
            )
        };
        format!(
            " — runs {}, this station {}",
            named(bandwidth_hz_of(caps)),
            named(bandwidth_hz_of(self.config.capabilities))
        )
    }

    // ── control frames ────────────────────────────────────────────────

    fn on_control(&mut self, ctl: &GrantFrame, frame: &Heard) {
        if matches!(self.state, State::Idle | State::Connecting) {
            self.answer_left(ctl, frame);
            return;
        }
        if ctl.session != self.session & 0x7F {
            return;
        }
        if self.role == Role::Master {
            self.master_control(ctl, frame);
        } else {
            self.slave_control(ctl, frame);
        }
    }

    // ── the master ────────────────────────────────────────────────────

    fn master_control(&mut self, ctl: &GrantFrame, frame: &Heard) {
        if !self.in_slot(frame) {
            return; // a frame of a slot already closed: its timing is nothing's now
        }
        self.heard(frame);
        if ctl.snr_db.is_some() {
            self.peer_snr_db = ctl.snr_db;
        }
        match ctl.kind {
            GrantKind::DiscAck => {
                if self.state == State::Disconnecting {
                    // the slave's closing identifier follows its DISC_ACK, as long as it
                    // declared (ADR-0075): its transmission ends there, and the master's
                    // identifier goes after it, whatever the slot's latest end
                    self.state_end(frame.t_end + self.peer_identifier_s, frame.floor);
                    let reason = if self.peer_left {
                        "peer disconnected"
                    } else {
                        "closed"
                    };
                    self.end_session(reason, false);
                }
                return;
            }
            GrantKind::Disc => {
                // the slave has left (an abort, a left session's answer). The master closes
                // with a DISC of its own, whose slot carries the slave's closing identifier
                // (ADR-0075); ended here, neither station's identifier had a place both could
                // count on
                self.peer_left = true;
                self.state = State::Disconnecting;
                self.state_end(frame.t_end, frame.floor);
                return;
            }
            GrantKind::Header => return,
            GrantKind::Answer => {}
        }
        if let Some(slot) = self.slot.as_ref()
            && !slot.open_window
        {
            let late = frame.t_start - slot.ident_s - slot.start - self.config.slack_s;
            if late > self.peer_late_s {
                self.peer_late_s = late;
            } else {
                self.peer_late_s = 0.0f64.max(late).max(self.peer_late_s - 0.1);
            }
        }
        self.take_ack(ctl, true);
        self.slave_want = Some(usize::from(ctl.n));
        self.level_asked = usize::from(ctl.level);
        self.hdr_floor = ctl.fam || self.floor_only();
        self.possible = Families::of(frame.floor);
        self.slave_leaves = ctl.flag;
        if ctl.flag && self.leave_since.is_none() {
            self.leave_since = Some(self.now);
        }
        if ctl.count == 0 {
            self.slave_floor_only = ctl.data_floor;
        }
        self.slave_id_due = ctl.ident;
        self.open_rx(frame, usize::from(ctl.count), ctl.data_floor);
        if ctl.count > 0 {
            self.ack_owed = true;
        }
        let last_floor = if ctl.count > 0 {
            ctl.data_floor
        } else {
            frame.floor
        };
        let end = frame.t_end + f64::from(ctl.count) * self.frame_s(ctl.data_floor);
        self.state_end(end, last_floor);
    }

    fn master_data<F: SoftFrame>(&mut self, soft: &F, frame: &Heard, decoded: bool) {
        if !self.in_slot(frame) {
            if decoded {
                // of an earlier slot, decoded late: what it carries is kept (sequence numbers
                // make that safe), its timing is not
                let (payload, _) = soft.decode(None);
                if let Some(payload) = payload {
                    self.accept(&payload);
                }
            }
            return;
        }
        let index = self.take_data(soft, frame);
        if index.is_none_or(|i| self.rx[i].payload.is_none()) {
            return;
        }
        let rescued = index.is_some_and(|i| self.rx[i].combined);
        self.heard(frame);
        self.ack_owed = true;
        if let Some(follows) = frame.follows
            && follows < MAX_FOLLOWS
            && !rescued
        {
            // a count rescued with its block is not believed (the note at `take_data`): the
            // master waits for the slot's latest end
            let floor = self.timing.is_floor(frame.mode);
            let end = frame.t_end + f64::from(follows) * (frame.t_end - frame.t_start);
            self.state_end(end, floor);
        } else if let Some(slot) = self.slot.as_mut() {
            slot.heard = true;
        }
    }

    /// The slot is over: the master keys again.
    fn proceed(&mut self) {
        if self.role != Role::Master
            || !matches!(self.state, State::Connected | State::Disconnecting)
        {
            return;
        }
        self.close_slot();
        if self.state == State::Idle {
            return;
        }
        if self.state == State::Disconnecting {
            if self.disc_tries >= self.config.disc_retries {
                let reason = if self.peer_left {
                    "peer disconnected"
                } else {
                    "closed (no DISC_ACK)"
                };
                self.end_session_unread(reason);
                return;
            }
            self.send_disc();
            return;
        }
        if (self.disc_requested || self.slave_leaves) && self.can_leave() {
            self.state = State::Disconnecting;
            self.send_disc();
            return;
        }
        self.send_header();
    }

    /// Whether the master may send its DISC now. What the slave still has goes first, and the
    /// master's own queue when the disconnect is its own; when the slave asked, its operator
    /// has stopped receiving, and what the master has not delivered stays undelivered
    /// (ADR-0023's receiving station; `80m-leave-together-500`). Either waits no longer than
    /// the drain's limit from when the disconnect was asked (ADR-0072) or the patience with
    /// nothing moving (ADR-0039), and says what it leaves.
    fn can_leave(&mut self) -> bool {
        let own_done = self.all_acknowledged() || self.slave_leaves;
        if own_done && self.slave_want.unwrap_or(0) == 0 && self.max_seen.is_none() {
            return true;
        }
        if self.patience_until.is_none() {
            self.patience_until = Some(self.now + self.disc_patience());
        }
        let since = *self.leave_since.get_or_insert(self.now);
        if self.now >= since + self.config.disc_drain_s {
            let why = format!("{:.0} s after the disconnect was asked", self.now - since);
            self.say_left(&why);
            return true;
        }
        if self.patience_until.is_some_and(|until| self.now >= until) {
            let why = format!("nothing acknowledged for {:.0} s", self.disc_patience());
            self.say_left(&why);
            return true;
        }
        false
    }

    /// What a closing master leaves behind, said once as it decides to close: its own queue
    /// when the disconnect is its own, and what the slave still said it had.
    fn say_left(&mut self, why: &str) {
        let mut left = Vec::new();
        let own = if self.slave_leaves {
            0
        } else {
            self.tx_undelivered_bytes()
        };
        if own > 0 {
            left.push(format!("{own} bytes not delivered"));
        }
        let theirs = self.slave_want.unwrap_or(0);
        if theirs > 0 {
            left.push(format!(
                "the other station's last {theirs} frames not received"
            ));
        }
        if !left.is_empty() {
            self.event("disconnect", format!("{}; closing {why}", left.join("; ")));
        }
    }

    /// What the slave sent in the slot goes to the rate controller; silence is counted and
    /// stepped down from (ADR-0069 §2.5).
    fn close_slot(&mut self) {
        let Some(slot) = self.slot.take() else {
            return;
        };
        if !self.rx.is_empty() {
            let records = std::mem::take(&mut self.rx);
            self.observe(&records);
        }
        self.restart_rx();
        if !slot.open_window {
            self.recent_silent.push_back(!slot.heard);
            while self.recent_silent.len() > self.config.floor_answer_memory {
                self.recent_silent.pop_front();
            }
        }
        if slot.heard {
            self.silent = 0;
            self.answered_run += 1;
            if self.answered_run >= self.config.on_time_to_lower && self.silence_level > 0 {
                self.silence_level -= 1;
                self.answered_run = 0;
            }
            self.idle_windows = if slot.open_window {
                self.idle_windows + 1
            } else {
                0
            };
            return;
        }
        if slot.open_window {
            self.idle_windows += 1; // an open window nobody needed is no silence
            return;
        }
        self.stats.silent_slots += 1;
        self.silent += 1;
        self.answered_run = 0;
        if slot.data && self.config.hold_unknown {
            for s in self.unacked() {
                if let Some(rec) = self.records.get_mut(&s) {
                    rec.unknown = true;
                }
            }
        }
        if self.silent >= self.config.silence_levels_after
            && self.silence_level < self.config.levels_s.len() - 1
        {
            self.silence_level += 1;
        }
        if slot.data {
            self.back_off();
        }
        if slot.granted > 0 && self.silent >= 2 {
            self.rate.observe(None, 0, 1, None);
        }
        if self.silent > self.config.max_retries {
            self.end_session("no response", false);
        }
    }

    fn has_work(&self) -> bool {
        !self.tx_queue.is_empty() || !self.unacked().is_empty()
    }

    /// (data frames granted, open window, window index): the master's policy (§5.2).
    fn grant_size(&self, rung: usize) -> (usize, bool, usize) {
        let frame = self.frame_s(self.timing.is_floor(rung));
        let mut fit = self.config.max_data_frames.min(MAX_DATA);
        if let Some(limit) = self.config.max_burst_s {
            let room = limit - self.ctl_s(true) - self.peer_identifier_s;
            fit = fit.min(whole(room / frame));
        }
        match self.slave_want {
            None => (self.config.first_grant.min(fit), false, 0),
            Some(want) if want > 0 => (want.min(fit), false, 0),
            Some(_) => {
                let idle = !self.has_work() && !self.disc_requested && !self.slave_id_due;
                if idle && self.now - self.heard_at < self.config.keepalive_s {
                    (0, true, self.config.window_doublings.min(self.idle_windows))
                } else {
                    (0, false, 0)
                }
            }
        }
    }

    /// Whether this transmission may go without its header (R7): data only, nothing granted
    /// beyond an answer, the gap at level 0, nothing to acknowledge or recommend anew, the last
    /// slot heard, no identifier time asked for, and the answer's family one the slave surely
    /// holds.
    fn elide(&self, n: usize, open_window: bool, wanted: bool) -> bool {
        self.config.elide
            && !open_window
            && n == 0
            && self.slave_want == Some(0)
            && self.silent == 0
            && self.level() == 0
            && self.possible == Families::of(wanted)
            && !self.ack_owed
            && !self.slave_id_due
            && self.has_work()
    }

    #[allow(clippy::too_many_lines)]
    fn send_header(&mut self) {
        let rung = self.recommend(self.slave_floor_only);
        let (n, open_window, index) = self.grant_size(rung);
        let wanted =
            self.wants_floor() || self.silent >= 2 || self.recent_silent.iter().any(|&s| s);
        let mut elide = self.elide(n, open_window, wanted);
        let hdr_floor = self.hdr_floor || self.silent >= 2 || self.floor_only();
        let room = self.config.max_burst_s.unwrap_or(f64::INFINITY);
        let data = if open_window {
            Vec::new()
        } else {
            self.data_frames(room - self.ctl_s(hdr_floor))
        };
        // the header is left out only of a transmission that carries no fewer data frames for
        // it: capped at the four whose counts can anchor it, the 15-byte rung of the 500 Hz air
        // sent 136 transmissions where protocol 7 sent 82 (the link bench, AWGN -3 dB). And only
        // where a count can anchor it at all: the slave believes a count only from a copy that
        // decoded alone (R2), and only an RV 0 copy can on every rung (ADR-0073)
        elide = elide
            && !data.is_empty()
            && data.len() <= usize::from(MAX_FOLLOWS) + 1
            && data.iter().any(|f| READABLE_ALONE_RVS.contains(&f.rv));
        let level = if elide { 0 } else { self.level() };
        // a slave of an earlier build asks for its ten-minute identifier's time, and is given it:
        // this build sends none inside a session (ADR-0075), and asks for none
        let slave_ident = self.slave_id_due && !elide;
        if !elide {
            self.possible.insert(wanted);
        }
        let mut answer_floors = self.possible.with(wanted);
        let ans_s = answer_floors.max_of(|f| self.ctl_s(f));
        let slave_floor = self.timing.is_floor(rung);
        #[allow(clippy::cast_precision_loss)]
        let grant_s = if open_window {
            self.window_s(hdr_floor, index)
        } else {
            let mut grant = ans_s + n as f64 * self.frame_s(slave_floor);
            if n > 0 {
                answer_floors.insert(slave_floor);
            }
            if slave_ident {
                grant += self.peer_identifier_s;
            }
            grant
        };
        let data_floor = data.first().is_some_and(|f| self.timing.is_floor(f.mode));
        let carried = !data.is_empty();
        let mut frames = data;
        if elide {
            self.stats.headers_elided += 1;
        } else {
            let (base, bitmap) = self.ack_fields();
            let ctl = GrantFrame {
                kind: GrantKind::Header,
                session: self.session & 0x7F,
                count: u8::try_from(frames.len()).unwrap_or(7),
                data_floor,
                level: u8::try_from(level).unwrap_or(3),
                fam: wanted,
                flag: open_window,
                ident: slave_ident,
                base,
                bitmap,
                snr_db: self.heard_db,
                rung: u8::try_from(rung).unwrap_or(31),
                n: u8::try_from(if open_window { index } else { n }).unwrap_or(7),
            };
            frames.insert(0, Self::control(&ctl, hdr_floor));
            self.ack_owed = false;
            self.slave_id_due = false;
        }
        let mut slot = Slot::new(
            SlotKind::Header,
            grant_s,
            if carried { data_floor } else { hdr_floor },
            answer_floors,
        );
        slot.level = level;
        slot.granter_turnaround_s = self.config.turnaround_s;
        slot.open_window = open_window;
        slot.ident_s = if slave_ident {
            self.peer_identifier_s
        } else {
            0.0
        };
        slot.data = carried;
        slot.granted = n;
        self.key_granting(slot, frames);
    }

    /// DISC grants the `DISC_ACK`'s slot with the slave's identifier time: its closing identifier
    /// goes right behind the `DISC_ACK`. The master's goes on its own once that slot is over
    /// (ADR-0075), the last thing either station sends for the session.
    fn send_disc(&mut self) {
        self.disc_tries += 1;
        let wanted = self.wants_floor() || self.silent >= 1;
        let floor = self.hdr_floor || self.silent >= 1 || self.floor_only();
        let level = self.level();
        let mut ctl = GrantFrame::new(GrantKind::Disc, self.session & 0x7F);
        ctl.level = u8::try_from(level).unwrap_or(3);
        ctl.fam = wanted;
        ctl.ident = self.peer_identifier_s > 0.0;
        ctl.snr_db = self.heard_db;
        self.possible.insert(wanted);
        let answer_floors = self.possible.with(wanted);
        let mut slot = Slot::new(
            SlotKind::Disc,
            answer_floors.max_of(|f| self.ctl_s(f)) + self.peer_identifier_s,
            floor,
            answer_floors,
        );
        slot.level = level;
        slot.granter_turnaround_s = self.config.turnaround_s;
        self.key_granting(slot, vec![Self::control(&ctl, floor)]);
    }

    // ── the slave ─────────────────────────────────────────────────────

    /// Place a frame of the master's among its transmissions: `false` for one of a transmission
    /// this station has answered already, decoded late, and for one that cannot be the master's.
    /// A frame that begins at or after the end of the transmission being received starts the
    /// next one — once this station has answered that transmission, or when the frame decoded:
    /// the master keys only after the slot, so until then a frame nothing vouches for is no
    /// transmission of its, and changes nothing (ADR-0069 §2.5).
    fn placed(&mut self, frame: &Heard, decoded: bool) -> bool {
        if let Some(answered) = self.answered_end
            && frame.t_end <= answered + 1e-6
        {
            return false;
        }
        if let Some(m_end) = self.m_end
            && frame.t_start >= m_end - 1e-3
        {
            let owed = self
                .answered_end
                .is_none_or(|answered| (m_end - answered).abs() > 1e-6);
            if owed && !decoded {
                return false;
            }
            self.next_transmission();
        }
        true
    }

    /// The master transmission being received is over and another begins. What is left of the
    /// last was heard after its answer, or with no anchor to answer it by: nothing the rate
    /// controller can take as a transmission.
    fn next_transmission(&mut self) {
        self.restart_rx();
        self.grant = None;
        self.m_end = None;
        self.window = None;
        if self
            .answer
            .as_ref()
            .is_some_and(|a| matches!(a.what, AnswerKind::Ans | AnswerKind::Window))
        {
            self.answer = None;
            self.deadlines.remove(Timer::Answer);
        }
    }

    fn slave_control(&mut self, ctl: &GrantFrame, frame: &Heard) {
        if !self.placed(frame, true) {
            if ctl.kind == GrantKind::Header {
                self.take_ack(ctl, false);
            }
            return;
        }
        self.heard(frame);
        self.measure(frame);
        if ctl.snr_db.is_some() {
            self.peer_snr_db = ctl.snr_db;
        }
        if ctl.kind == GrantKind::Disc {
            self.end_session("peer disconnected", true);
            self.answer_disc(ctl, frame, GrantKind::DiscAck);
            return;
        }
        if ctl.kind != GrantKind::Header {
            return;
        }
        self.open_rx(frame, usize::from(ctl.count), ctl.data_floor);
        self.take_ack(ctl, true);
        self.ans_sticky = ctl.fam || self.floor_only();
        self.grant = Some(*ctl);
        self.grant_floor = frame.floor;
        self.m_end = Some(frame.t_end + f64::from(ctl.count) * self.frame_s(ctl.data_floor));
        self.m_last_floor = if ctl.count > 0 {
            ctl.data_floor
        } else {
            frame.floor
        };
        self.schedule_slave();
    }

    fn slave_data<F: SoftFrame>(&mut self, soft: &F, frame: &Heard, decoded: bool) {
        if !self.placed(frame, decoded) {
            if decoded {
                let (payload, _) = soft.decode(None);
                if let Some(payload) = payload {
                    self.accept(&payload);
                }
            }
            return;
        }
        let index = self.take_data(soft, frame);
        if index.is_some_and(|i| self.rx[i].payload.is_some()) {
            let rescued = index.is_some_and(|i| self.rx[i].combined);
            self.heard(frame);
            self.measure(frame);
            if self.m_end.is_none()
                && !rescued
                && let Some(follows) = frame.follows
                && follows < MAX_FOLLOWS
            {
                // the header was not read: the transmission's end is the one this frame states —
                // if its own copy decoded (the note at `take_data`)
                self.m_end = Some(frame.t_end + f64::from(follows) * (frame.t_end - frame.t_start));
                self.m_last_floor = self.timing.is_floor(frame.mode);
                self.schedule_slave();
            }
        }
        self.decide_if_final();
    }

    /// How late this station's receiver reported a frame of the other's, and how late its
    /// keying is, against what the slot allows: the level its answers ask for. Up at once, down
    /// one level after `need_ease_after` frames on time.
    fn measure(&mut self, frame: &Heard) {
        let mut late = self.now - frame.t_end - self.budget(frame.floor);
        late += 0.0f64.max(self.own_latency() - self.config.key_s);
        let need = self.level_for(late);
        if need > self.need_level {
            self.need_level = need;
            self.on_time = 0;
        } else if need < self.need_level {
            self.on_time += 1;
            if self.on_time >= self.config.need_ease_after {
                self.need_level -= 1;
                self.on_time = 0;
            }
        }
    }

    /// The slot the master transmission being received grants. A value the master can change is
    /// taken only from that transmission's own header; without it, the one that fits every slot
    /// the master may have granted — level 0, an answer alone, the family last asked for
    /// (ADR-0069 §2.3).
    fn schedule_slave(&mut self) {
        let Some(m_end) = self.m_end else {
            return;
        };
        if self.state != State::Connected {
            return;
        }
        if self
            .answered_end
            .is_some_and(|answered| (m_end - answered).abs() < 1e-6)
        {
            return;
        }
        let grant = self.grant;
        let level = grant.map_or(0, |g| usize::from(g.level));
        let start = self.slot_start(m_end, self.m_last_floor, level, self.peer_turnaround_s);
        let floor = self.ans_sticky;
        if let Some(g) = grant
            && g.flag
        {
            let window = self.window_s(self.grant_floor, usize::from(g.n));
            self.window = Some((start, start + window, floor));
            self.answer = None;
            self.deadlines.remove(Timer::Answer);
            if self.has_work() || self.disc_requested {
                self.try_window();
            }
            return;
        }
        self.window = None;
        let mut room = 0.0;
        if let Some(g) = grant {
            room = f64::from(g.n) * self.frame_s(self.timing.is_floor(usize::from(g.rung)));
        }
        let mut answer = Answer::new(AnswerKind::Ans, start, start + self.config.slack_s, floor);
        answer.session = self.session;
        answer.anchor = m_end;
        answer.room_s = room;
        self.schedule(answer);
    }

    /// Whether `now` is too late to start an answer due at `start` on time.
    fn too_late(&self, start: f64) -> bool {
        self.now > start - self.own_latency() + self.config.slack_s + 1e-9
    }

    /// Owe an answer in a slot — or forfeit it, decoded too late to start on time. Nothing is
    /// ever sent late.
    fn schedule(&mut self, answer: Answer) {
        if self.too_late(answer.start) {
            self.stats.forfeits += 1;
            if answer.what == AnswerKind::Ans {
                self.forfeit(answer.anchor);
            }
            return;
        }
        let at = self.now.max(answer.start - self.own_latency());
        self.answer = Some(answer);
        self.deadlines.set(Timer::Answer, at);
        self.decide_if_final();
    }

    /// Decide an owed answer as soon as what it says is final, not at the last moment: it
    /// radiates at its slot's start all the same (`not_before`), and the daemon renders it
    /// meanwhile — four tone frames take up to 0.18 s to render, more under load, which the
    /// keying latency the slot allows has no room for (07-protocol-8.md §10). An answer to a
    /// call, a probe or a DISC is final at once; a slot's answer once every data frame its
    /// header stated has been reported, decoded or not. Without the header, or with a frame
    /// never reported, it is decided at the last moment, as it always was.
    fn decide_if_final(&mut self) {
        let Some(what) = self.answer.as_ref().map(|a| a.what) else {
            return;
        };
        if what == AnswerKind::Window || (what == AnswerKind::Ans && !self.heard_all()) {
            return;
        }
        self.deadlines.remove(Timer::Answer);
        self.key_answer();
    }

    /// Every data frame the header being answered stated has been reported.
    fn heard_all(&self) -> bool {
        let (Some((_, count, _)), Some(_)) = (self.grid, self.grant) else {
            return false;
        };
        (0..count).all(|k| self.rx.iter().any(|r| r.slot == k))
    }

    /// A transmission of the master's this station will not answer: what it heard of it is
    /// still news of the path.
    fn forfeit(&mut self, anchor: f64) {
        self.answered_end = Some(anchor);
        if !self.rx.is_empty() {
            let records = std::mem::take(&mut self.rx);
            self.observe(&records);
        }
        self.restart_rx();
    }

    fn key_answer(&mut self) {
        let Some(answer) = self.answer.take() else {
            return;
        };
        if answer.what == AnswerKind::Window {
            self.try_window();
            return;
        }
        if self.too_late(answer.start) {
            self.stats.forfeits += 1; // the tick came too late
            if answer.what == AnswerKind::Ans {
                self.forfeit(answer.anchor);
            }
            return;
        }
        match answer.what {
            AnswerKind::ConnectAck => self.send_acceptance(&answer),
            AnswerKind::ProbeAck => self.send_probe_answer(&answer),
            AnswerKind::Disc | AnswerKind::DiscAck => self.send_disc_answer(&answer),
            AnswerKind::Ans if self.state == State::Connected && self.role == Role::Slave => {
                self.answered_end = Some(answer.anchor);
                let keying = Keying {
                    not_after: Some(answer.not_after),
                    not_before: Some(answer.start),
                    ..Keying::default()
                };
                self.send_answer(answer.floor, answer.room_s, keying, None);
            }
            _ => {}
        }
    }

    /// Inside an open window: speak now if there is something to say, ending inside it.
    fn try_window(&mut self) {
        let Some((start, end, floor)) = self.window else {
            return;
        };
        if self.state != State::Connected {
            return;
        }
        let on_air = self.now + self.own_latency();
        if on_air < start - 1e-9 {
            self.answer = Some(Answer::new(AnswerKind::Window, start, end, floor));
            self.deadlines
                .set(Timer::Answer, start - self.own_latency());
            return;
        }
        if !(self.has_work() || self.disc_requested) {
            return;
        }
        let room = end - on_air - self.ctl_s(floor);
        if room < 0.0 {
            return;
        }
        self.window = None;
        self.answered_end = self.m_end;
        self.send_answer(floor, room, Keying::default(), Some(end));
    }

    fn send_answer(&mut self, floor: bool, room_s: f64, keying: Keying, window_end: Option<f64>) {
        let snr = if self.rx.is_empty() {
            None
        } else {
            let records = std::mem::take(&mut self.rx);
            self.observe(&records)
        };
        self.restart_rx();
        if snr.is_some() {
            self.heard_db = snr;
        }
        let mut room_s = room_s;
        if let Some(limit) = self.config.max_burst_s {
            room_s = room_s.min(limit - self.ctl_s(floor));
        }
        let data = self.data_frames(room_s);
        // with no data after it, the bit says where this station's data can go: only on the
        // floor, when its ceiling or its pinned rung is there
        let data_floor = data
            .first()
            .map_or_else(|| self.data_floor_only(), |f| self.timing.is_floor(f.mode));
        let fam = self.wants_floor();
        let rung = self.recommend(false);
        let queued = self.queued_frames().min(7);
        let (base, bitmap) = self.ack_fields();
        let ctl = GrantFrame {
            kind: GrantKind::Answer,
            session: self.session & 0x7F,
            count: u8::try_from(data.len()).unwrap_or(7),
            data_floor,
            level: u8::try_from(self.asked_level()).unwrap_or(3),
            fam,
            flag: self.disc_requested,
            ident: false,
            base,
            bitmap,
            snr_db: self.heard_db,
            rung: u8::try_from(rung).unwrap_or(31),
            n: u8::try_from(queued).unwrap_or(7),
        };
        let mut frames = vec![Self::control(&ctl, floor)];
        frames.extend(data);
        let mut keying = keying;
        if let Some(window_end) = window_end {
            let frames_s: f64 = frames.iter().map(|f| self.timing.frame_s(f)).sum();
            keying.not_after = Some(window_end - frames_s);
        }
        self.transmit(frames, keying, None);
    }

    /// Whether this station's data can go only on the tone floor: its ceiling, or the rung it is
    /// pinned at, is a floor rung.
    fn data_floor_only(&self) -> bool {
        let limit = self
            .pinned
            .map_or_else(|| self.cap(), |pinned| pinned.min(self.cap()));
        self.timing.floor_modes > 0 && self.timing.is_floor(limit)
    }

    fn queued_frames(&self) -> usize {
        let cap = data_capacity(self.timing.capacity(self.burst_mode()));
        self.unacked().len() + self.tx_queue.len().div_ceil(cap.max(1))
    }

    fn send_acceptance(&mut self, answer: &Answer) {
        if self.session != answer.session || self.role != Role::Slave {
            return;
        }
        let body = ConnectBody {
            src: self.my_call.clone(),
            dst: answer.remote.clone(),
            caps: with_level(
                self.config.capabilities,
                u8::try_from(self.asked_level()).unwrap_or(3),
            ),
            version: PROTOCOL_VERSION,
            snr_db: answer.snr_db,
            turnaround_s: self.config.turnaround_s,
            identifier_s: self.my_ident_s,
        };
        let mode = self.robust_mode(answer.floor);
        let header = DataHeader {
            kind: DataKind::ConnectAck,
            seq: 0,
            session: self.session,
        };
        let Ok(body) = body.encode() else { return };
        let Ok(payload) = encode_data(&header, &body, self.timing.capacity(mode)) else {
            return;
        };
        self.transmit(
            vec![Self::data_frame(payload, mode)],
            Keying::answering(answer, 0.0),
            None,
        );
    }

    fn send_probe_answer(&mut self, answer: &Answer) {
        if self.state != State::Idle {
            return;
        }
        let body = ProbeBody {
            src: self.my_call.clone(),
            dst: answer.remote.clone(),
            snr_db: answer.snr_db,
            caps: self.config.capabilities,
        };
        let mode = self.robust_mode(answer.floor);
        let header = DataHeader {
            kind: DataKind::ProbeAck,
            seq: 0,
            session: 0,
        };
        let Ok(body) = body.encode() else { return };
        let Ok(payload) = encode_data(&header, &body, self.timing.capacity(mode)) else {
            return;
        };
        self.transmit(
            vec![Self::data_frame(payload, mode)],
            Keying::answering(answer, 0.0),
            None,
        );
    }

    /// A `DISC_ACK`, and right behind it this station's closing identifier — the last thing it
    /// sends for the session, in the time every DISC grants it (07-protocol-8.md §5.4: the
    /// `DISC_ACK`'s slot always includes the identifier the slave declared). A DISC heard again is
    /// answered again, identifier and all, so the identifier stays last (ADR-0075). Or a DISC, to
    /// a left session's header, after which the other station closes.
    fn send_disc_answer(&mut self, answer: &Answer) {
        let kind = if answer.what == AnswerKind::DiscAck {
            GrantKind::DiscAck
        } else {
            GrantKind::Disc
        };
        let mut ctl = GrantFrame::new(kind, answer.session & 0x7F);
        ctl.snr_db = answer.snr_db;
        let mut ident_s = 0.0;
        if kind == GrantKind::DiscAck && self.my_ident_s > 0.0 {
            ident_s = self.my_ident_s;
            self.id_owed = 0.0;
            self.deadlines.remove(Timer::Ident);
        }
        if kind == GrantKind::Disc {
            self.stats.left_answered += 1;
        }
        self.transmit(
            vec![Self::control(&ctl, answer.floor)],
            Keying::answering(answer, ident_s),
            None,
        );
        if ident_s != 0.0 {
            self.hold_until = self.hold_until.max(self.closing_hold(answer.start));
        }
    }

    /// The latest the other station's closing identifier can be on the air, after a `DISC_ACK`
    /// of this station's that answered in a slot starting at `start`: the slot's latest end as
    /// that station may reckon it — its slack, a floor `DISC_ACK`, this station's identifier — the
    /// wait that follows, at the top gap level with room for a slow keying, and the other's
    /// declared identifier (ADR-0075). Nothing this station starts on its own keys before it: a
    /// call answered or made into the other's identifier would go over it.
    fn closing_hold(&self, start: f64) -> f64 {
        let cfg = &self.config;
        let latest = start + cfg.slack_s + self.ctl_s(true) + self.my_ident_s;
        let wait = self.budget(true).max(cfg.turnaround_s) + cfg.guard_s + cfg.levels_s[3];
        latest + wait + cfg.key_s + cfg.slack_s + self.peer_identifier_s
    }

    /// A `DISC_ACK` to a DISC, or a DISC to a left session's header: in the slot the frame
    /// grants, or not at all.
    fn answer_disc(&mut self, ctl: &GrantFrame, frame: &Heard, kind: GrantKind) {
        let end = frame.t_end + f64::from(ctl.count) * self.frame_s(ctl.data_floor);
        let last_floor = if ctl.count > 0 {
            ctl.data_floor
        } else {
            frame.floor
        };
        let start = self.slot_start(
            end,
            last_floor,
            usize::from(ctl.level),
            self.peer_turnaround_s,
        );
        let what = if kind == GrantKind::DiscAck {
            AnswerKind::DiscAck
        } else {
            AnswerKind::Disc
        };
        let mut answer = Answer::new(
            what,
            start,
            start + self.config.slack_s,
            ctl.fam || self.floor_only(),
        );
        answer.session = ctl.session;
        answer.anchor = end;
        answer.snr_db = Some(frame.snr_db);
        self.schedule(answer);
    }

    // ── sending data ──────────────────────────────────────────────────

    fn unacked(&self) -> Vec<u8> {
        let mut seqs: Vec<u8> = self
            .records
            .iter()
            .filter(|(_, r)| !r.acked && r.sent())
            .map(|(&s, _)| s)
            .collect();
        seqs.sort_by_key(|&s| seq_distance(s, self.tx_base));
        seqs
    }

    /// This station's data for a transmission, laid out as `LinkEngine` lays out a burst: the
    /// unacknowledged frames first — re-encoded when stranded (ADR-0038) — then new ones, one
    /// family a transmission, as many as `room_s` and [`MAX_DATA`] allow. Each frame states how
    /// many follow it, three meaning three or more.
    #[allow(clippy::too_many_lines)]
    fn data_frames(&mut self, room_s: f64) -> Vec<TxFrame> {
        if room_s <= 0.0 {
            return Vec::new();
        }
        let mode = self.burst_mode();
        let recommendation = self.recommended.min(self.cap());
        // what an unheard answer would have said is not resent blind: it waits for an answer —
        // through one silent slot. Two in a row and the answers are not getting through, and
        // the frames go again rather than not at all
        let unacked: Vec<u8> = self
            .unacked()
            .into_iter()
            .filter(|s| self.silent >= 2 || self.records.get(s).is_some_and(|r| !r.unknown))
            .collect();
        for &s in &unacked {
            let Some((tx_count, rec_mode, body_len, pinned)) = self
                .records
                .get(&s)
                .map(|r| (r.tx_count, r.mode, r.body.len(), r.pinned))
            else {
                continue;
            };
            // a frame pinned for a measurement has made it with its first copy: from then on it
            // goes as data goes, at the rung asked for, never again at the rung that failed — the
            // Test's ladder had sent such frames again at RV 0, 2 and 3 before re-encoding them,
            // 13 of the 21 silent slots of ND1J's Tests of 2026-10-15 (ADR-0074)
            let due = if pinned {
                1
            } else if self.hopeless(rec_mode) {
                self.config.reencode_early_after
            } else {
                self.config.max_combines
            };
            if tx_count >= due && recommendation < rec_mode {
                let target = self.reencode_target(body_len, rec_mode, recommendation);
                if let (Some(target), Some(rec)) = (target, self.records.get_mut(&s)) {
                    rec.mode = target;
                    rec.reencoded += rec.tx_count;
                    rec.tx_count = 0;
                    rec.pinned = false;
                    self.stats.frames_reencoded += 1;
                }
            }
        }
        let mut family = self.timing.is_floor(mode);
        if let Some(first) = unacked.first().and_then(|s| self.records.get(s)) {
            family = self.timing.is_floor(first.mode);
        }
        let mut seqs: Vec<u8> = unacked
            .iter()
            .copied()
            .filter(|s| {
                self.records
                    .get(s)
                    .is_some_and(|r| self.timing.is_floor(r.mode) == family)
            })
            .collect();
        let frame_s = self.frame_s(family);
        let mut room = self.burst_capacity(frame_s);
        if room_s.is_finite() {
            room = room.min(whole(room_s / frame_s));
        }
        if room == 0 {
            return Vec::new();
        }
        seqs.truncate(room);
        let new_frames = family == self.timing.is_floor(mode);
        let cap = data_capacity(self.timing.capacity(mode));
        while new_frames
            && seqs.len() < room
            && seq_distance(self.tx_next, self.tx_base) < WINDOW
            && !self.tx_queue.is_empty()
        {
            let mut take = cap.min(self.tx_queue.len());
            if let Some(body) = self.pin_body {
                take = take.min(body);
            }
            if take + 1 == cap {
                take -= 1; // the one length the container cannot carry
            }
            let body: Vec<u8> = self.tx_queue.drain(..take).collect();
            let seq = self.tx_next;
            self.records.insert(
                seq,
                TxRecord {
                    seq,
                    body,
                    mode,
                    tx_count: 0,
                    acked: false,
                    reencoded: 0,
                    unknown: false,
                    pinned: self.pinned.is_some(),
                },
            );
            self.tx_next = seq_after(self.tx_next, 1);
            seqs.push(seq);
        }
        if self.pinned.is_some() {
            let fresh: Vec<u8> = seqs
                .iter()
                .copied()
                .filter(|s| self.records.get(s).is_some_and(|r| !r.sent()))
                .collect();
            if !fresh.is_empty() {
                self.ladder_pending = Some((mode, fresh));
            }
        }
        let mut frames = Vec::with_capacity(seqs.len());
        let count = seqs.len();
        for (i, s) in seqs.iter().enumerate() {
            let Some(rec) = self.records.get_mut(s) else {
                continue;
            };
            if rec.sent() {
                self.stats.frames_resent += 1;
            }
            rec.tx_count += 1;
            let header = DataHeader {
                kind: DataKind::Data,
                seq: rec.seq,
                session: self.session,
            };
            let Ok(payload) = encode_data(&header, &rec.body, self.timing.capacity(rec.mode))
            else {
                continue;
            };
            let rv = RV_SEQUENCE[(rec.tx_count - 1) % RV_SEQUENCE.len()];
            let follows =
                u8::try_from((count - 1 - i).min(usize::from(MAX_FOLLOWS))).unwrap_or(MAX_FOLLOWS);
            frames.push(TxFrame {
                container: Container::Data,
                payload,
                mode: rec.mode,
                rv,
                floor: false,
                follows,
            });
        }
        self.stats.frames_sent += frames.len();
        frames
    }

    /// The other station's acknowledgement of this one's data, and its recommendation.
    fn take_ack(&mut self, ctl: &GrantFrame, recommendation: bool) {
        if recommendation && let Some((pinned_mode, fresh)) = self.ladder_pending.take() {
            let decoded = fresh.iter().filter(|&&s| ctl.received(s)).count();
            self.ladder.push(LadderRung {
                mode: pinned_mode,
                frames: fresh.len(),
                decoded,
                snr_db: ctl.snr_db,
            });
        }
        let mut progress = false;
        for rec in self.records.values_mut() {
            rec.unknown = false; // the answer says what arrived
        }
        for (&s, rec) in &mut self.records {
            if !rec.acked && rec.sent() && ctl.received(s) {
                rec.acked = true;
                progress = true;
                self.top_acked_mode = Some(self.top_acked_mode.unwrap_or(0).max(rec.mode));
                self.stats.bytes_acked += rec.body.len();
            }
        }
        while self.tx_base != self.tx_next
            && self.records.get(&self.tx_base).is_some_and(|r| r.acked)
        {
            self.records.remove(&self.tx_base);
            self.tx_base = seq_after(self.tx_base, 1);
        }
        if progress && self.patience_until.is_some() {
            self.patience_until = Some(self.now + self.disc_patience());
        }
        if recommendation {
            self.recommended = self.cap().min(usize::from(ctl.rung));
        }
    }

    // ── receiving data ────────────────────────────────────────────────

    // A frame's count rides in its chips, outside the codeword, so a CRC that passes vouches for
    // the payload and not for the count. A copy that decodes alone was read well enough for its
    // count too; a copy HARQ rescued was not — it is the earlier copies that decoded — and on the
    // scenario harness one read three frames to follow as none: the slave answered over them
    // (10-step-3.md §6). Such a count places no end: the slave keeps waiting for one it can
    // believe (a silent slot at worst), the master for the slot's latest end.

    /// A data frame of the transmission being received: decoded alone, or combined with an
    /// earlier copy of its block, its sequence number inferred from its slot (as `LinkEngine`).
    /// Returns where its record is kept, if it is.
    fn take_data<F: SoftFrame>(&mut self, soft: &F, frame: &Heard) -> Option<usize> {
        let (payload, buffer) = soft.decode(None);
        if payload.is_none() && !self.on_grid(frame) {
            self.stats.frames_failed += 1; // where no frame of the transmission can be
            return None;
        }
        let t0 = *self.rx_t0.get_or_insert(frame.t_start);
        let frame_s = (frame.t_end - frame.t_start).max(1e-6);
        let slot = usize::try_from(round_even((frame.t_start - t0) / frame_s).max(0)).unwrap_or(0);
        self.rx.push(RxRecord {
            frame: *frame,
            slot,
            payload: None,
            seq: None,
            combined: false,
        });
        let index = self.rx.len() - 1;
        if let Some(payload) = payload {
            return self.keep(index, payload);
        }
        let guess = self.infer_seq(slot);
        self.rx[index].seq = guess;
        let Some(guess) = guess.filter(|&g| in_window(g, self.rx_base, WINDOW)) else {
            self.stats.frames_failed += 1;
            return Some(index);
        };
        if self.harq.get(&guess).is_some_and(|h| h.2 != frame.mode) {
            self.harq.remove(&guess); // re-encoded at another mode: start over
        }
        if let Some((prev, combines, _)) = self.harq.get(&guess).cloned() {
            self.rx[index].combined = true;
            let (combined, merged) = soft.decode(Some(&prev));
            if let Some(combined) = combined {
                self.stats.harq_rescues += 1;
                return self.keep(index, combined);
            }
            let entry = if combines + 1 >= self.config.max_combines {
                (buffer, 0, frame.mode)
            } else {
                (merged, combines + 1, frame.mode)
            };
            self.harq.insert(guess, entry);
        } else {
            self.harq.insert(guess, (buffer, 0, frame.mode));
        }
        self.stats.frames_failed += 1;
        Some(index)
    }

    /// The decoded payload of the record at `index`: kept if it is this session's data, the
    /// record dropped otherwise.
    fn keep(&mut self, index: usize, payload: Vec<u8>) -> Option<usize> {
        if let Some(seq) = self.accept(&payload) {
            self.rx[index].payload = Some(payload);
            self.rx[index].seq = Some(seq);
            Some(index)
        } else {
            self.rx.remove(index);
            None
        }
    }

    fn restart_rx(&mut self) {
        self.rx.clear();
        self.rx_t0 = None;
        self.grid = None;
    }

    /// A control frame read opens the other station's transmission and states it: `count` data
    /// frames of `floor`'s family, back to back from its end. Nothing heard before is of it, and
    /// from now on a frame that does not decode counts only where one of those frames can be.
    fn open_rx(&mut self, opener: &Heard, count: usize, floor: bool) {
        self.rx_t0 = Some(opener.t_end);
        self.grid = Some((opener.t_end, count, floor));
        let records = std::mem::take(&mut self.rx);
        self.rx = records
            .into_iter()
            .filter(|r| self.on_grid(&r.frame))
            .collect();
    }

    /// Whether a frame sits where a data frame of the transmission being received can: in its
    /// family, on its slots, within its count — as its opening control frame stated, or, without
    /// it, in line with a frame of it that decoded. A false detection between the slots is a
    /// failure of nothing: counted in, it stepped the rate down the floor.
    #[allow(clippy::cast_precision_loss)]
    fn on_grid(&self, frame: &Heard) -> bool {
        let floor = self.timing.is_floor(frame.mode);
        if let Some((t0, count, grid_floor)) = self.grid {
            let slot_s = self.frame_s(grid_floor);
            let k = round_even((frame.t_start - t0) / slot_s);
            return floor == grid_floor
                && k >= 0
                && usize::try_from(k).is_ok_and(|k| k < count)
                && (frame.t_start - (t0 + k as f64 * slot_s)).abs() <= GRID_TOLERANCE_S;
        }
        let Some(reference) = self
            .rx
            .iter()
            .find(|r| r.payload.is_some())
            .map(|r| r.frame)
        else {
            return true;
        };
        let slot_s = (reference.t_end - reference.t_start).max(1e-6);
        let k = round_even((frame.t_start - reference.t_start) / slot_s);
        let offset = (frame.t_start - (reference.t_start + k as f64 * slot_s)).abs();
        floor == self.timing.is_floor(reference.mode) && offset <= GRID_TOLERANCE_S
    }

    fn infer_seq(&self, slot: usize) -> Option<u8> {
        let anchors: Vec<(usize, u8)> = self
            .rx
            .iter()
            .filter(|r| r.payload.is_some())
            .filter_map(|r| r.seq.map(|seq| (r.slot, seq)))
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
        for snap in snapshots {
            let mut expected = snap.missing.clone();
            let mut s = snap.next_new;
            while expected.len() < MAX_BURST {
                expected.push(s);
                s = seq_after(s, 1);
            }
            let mut offset: Option<i64> = None;
            let mut consistent = true;
            for &(a_slot, a_seq) in &anchors {
                let Some(position) = expected.iter().position(|&e| e == a_seq) else {
                    consistent = false;
                    break;
                };
                let this =
                    i64::try_from(position).unwrap_or(0) - i64::try_from(a_slot).unwrap_or(0);
                match offset {
                    None => offset = Some(this),
                    Some(o) if o != this => {
                        consistent = false;
                        break;
                    }
                    Some(_) => {}
                }
            }
            if !consistent {
                continue;
            }
            let idx = i64::try_from(slot).unwrap_or(i64::MAX) + offset.unwrap_or(0);
            if let Ok(idx) = usize::try_from(idx)
                && idx < expected.len()
            {
                return Some(expected[idx]);
            }
        }
        None
    }

    /// Take a decoded data frame of this session; its sequence number, or `None` for anything
    /// else.
    fn accept(&mut self, payload: &[u8]) -> Option<u8> {
        let Ok((header, body)) = decode_data(payload) else {
            return None;
        };
        if header.kind != DataKind::Data || header.session != self.session {
            return None;
        }
        self.stats.frames_received += 1;
        if in_window(header.seq, self.rx_base, WINDOW) {
            if self.max_seen.is_none_or(|seen| {
                seq_distance(header.seq, self.rx_base) > seq_distance(seen, self.rx_base)
            }) {
                self.max_seen = Some(header.seq);
            }
            self.rx_buf.entry(header.seq).or_insert(body);
            self.harq.remove(&header.seq);
            while let Some(data) = self.rx_buf.remove(&self.rx_base) {
                self.harq.remove(&self.rx_base);
                self.rx_base = seq_after(self.rx_base, 1);
                self.stats.bytes_delivered += data.len();
                if !data.is_empty() {
                    self.actions.push(Action::Deliver(data));
                }
            }
            if self
                .max_seen
                .is_some_and(|seen| seq_distance(seen, self.rx_base) >= WINDOW)
            {
                self.max_seen = None;
            }
        }
        Some(header.seq)
    }

    /// What a transmission's data said of the path, to the rate controller (ADR-0020's rules).
    /// Returns the SNR measured on it.
    #[allow(clippy::cast_precision_loss)]
    fn observe(&mut self, records: &[RxRecord]) -> Option<f64> {
        let ok = records.iter().filter(|r| r.payload.is_some()).count();
        let failed = records
            .iter()
            .filter(|r| {
                r.payload.is_none()
                    && r.frame.trusted
                    && (SELF_DECODABLE_RVS.contains(&r.frame.rv) || r.combined)
            })
            .count();
        let snrs: Vec<f64> = records
            .iter()
            .filter(|r| r.payload.is_some() || r.frame.trusted)
            .map(|r| r.frame.snr_db)
            .collect();
        let snr = (!snrs.is_empty()).then(|| snrs.iter().sum::<f64>() / snrs.len() as f64);
        let Some(slowest) = records.iter().map(|r| r.frame.mode).min() else {
            return snr;
        };
        if failed > 0 && self.asked.is_some_and(|asked| slowest > asked) {
            self.rate.observe_snr(snr); // the sender's choice, not news of the path
        } else {
            // the rung most of it went at; of two as common, the slower
            let mut modes: Vec<usize> = records.iter().map(|r| r.frame.mode).collect();
            modes.sort_unstable();
            let mut best = modes[0];
            let mut best_count = 0;
            let mut i = 0;
            while i < modes.len() {
                let mut j = i;
                while j < modes.len() && modes[j] == modes[i] {
                    j += 1;
                }
                if j - i > best_count {
                    best = modes[i];
                    best_count = j - i;
                }
                i = j;
            }
            self.rate.observe(snr, ok, failed, Some(best));
        }
        snr
    }

    fn receive_window(&self) -> (u16, Vec<u8>, u8) {
        let mut bitmap = 0u16;
        let mut missing = Vec::new();
        let limit = self
            .max_seen
            .map_or(0, |seen| seq_distance(seen, self.rx_base) + 1);
        for i in 0..WINDOW {
            let s = seq_after(self.rx_base, u8::try_from(i).unwrap_or(0));
            if self.rx_buf.contains_key(&s) {
                bitmap |= 1 << i;
            } else if i < limit {
                missing.push(s);
            }
        }
        (
            bitmap,
            missing,
            seq_after(self.rx_base, u8::try_from(limit).unwrap_or(0)),
        )
    }

    /// What a control frame says this station has received — remembered, for the slot
    /// inference of the copies sent after it.
    fn ack_fields(&mut self) -> (u8, u16) {
        let (bitmap, missing, next_new) = self.receive_window();
        let snap = AckSnapshot { missing, next_new };
        self.ack_history.insert(0, snap);
        self.ack_history.truncate(2);
        (self.rx_base, bitmap)
    }

    // ── the end ───────────────────────────────────────────────────────

    /// An idle station hears a control frame of a session it left: a DISC is answered with its
    /// `DISC_ACK`, a header (the other station still in the session) with a DISC — in the
    /// frame's slot, or not at all (ADR-0052). A `DISC_ACK` ends the leaving.
    fn answer_left(&mut self, ctl: &GrantFrame, frame: &Heard) {
        let Some((session, until, disc_heard)) = self.left else {
            return;
        };
        if self.now > until {
            self.left = None;
            return;
        }
        if ctl.session != session & 0x7F {
            return;
        }
        if ctl.kind == GrantKind::DiscAck {
            self.left = None;
            self.deadlines.remove(Timer::Leave);
            let leaving = self
                .slot
                .as_ref()
                .is_some_and(|slot| slot.kind == SlotKind::Leave);
            if leaving && self.in_slot(frame) {
                // an aborting master's DISC answered: the slave's identifier follows the
                // DISC_ACK, and this station's own goes after it (ADR-0075)
                self.state_end(frame.t_end + self.peer_identifier_s, frame.floor);
            }
            self.identify_after_leaving();
            return;
        }
        if self.state == State::Connecting {
            return; // a call of this station's is out: its slot is the one that matters
        }
        if self.id_owed > 0.0 && self.id_alone {
            // the other station is still there: its next words may close the session properly,
            // and this station's identifier is not sent over it on its own (ADR-0075)
            self.deadlines
                .set(Timer::Ident, self.now + self.exchange_s());
        }
        if ctl.kind == GrantKind::Disc {
            self.answer_disc(ctl, frame, GrantKind::DiscAck);
        } else if ctl.kind == GrantKind::Header && !disc_heard {
            self.answer_disc(ctl, frame, GrantKind::Disc);
        }
    }

    /// An aborting master's DISC, from idle: on the floor, granting the `DISC_ACK`'s slot, up
    /// to `leave_repeats` times, each once the last one's slot is over.
    fn repeat_leave(&mut self) {
        let Some((session, until, _)) = self.left else {
            self.identify_after_leaving();
            return;
        };
        if self.state != State::Idle || self.leave_repeats == 0 {
            self.identify_after_leaving();
            return;
        }
        if self.now < self.hold_until - 1e-9 {
            self.deadlines.set(Timer::Leave, self.hold_until);
            return;
        }
        if self.now > until {
            self.left = None;
            self.identify_after_leaving();
            return;
        }
        self.leave_repeats -= 1;
        let mut ctl = GrantFrame::new(GrantKind::Disc, session & 0x7F);
        ctl.fam = true;
        ctl.level = u8::try_from(self.silence_level).unwrap_or(3);
        ctl.ident = self.peer_identifier_s > 0.0;
        let mut slot = Slot::new(
            SlotKind::Leave,
            self.ctl_s(true) + self.peer_identifier_s,
            true,
            Families::of(true),
        );
        slot.level = self.silence_level;
        slot.granter_turnaround_s = self.config.turnaround_s;
        self.key_granting(slot, vec![Self::control(&ctl, true)]);
        if self.leave_repeats > 0 {
            self.deadlines.set(Timer::Leave, self.hold_until);
        } else {
            self.identify_after_leaving();
        }
    }

    // ── the closing identifier (ADR-0075) ─────────────────────────────

    /// An aborting master's leaving is over — its DISC answered, or the last one's slot past:
    /// its closing identifier goes once that slot can no longer be in use.
    fn identify_after_leaving(&mut self) {
        if self.id_owed > 0.0 && !self.id_alone && self.state == State::Idle {
            self.deadlines
                .set(Timer::Ident, self.now.max(self.hold_until));
        }
    }

    /// The closing identifier on its own: a master's, once the last slot it granted is over, or
    /// a station's that no slot could be had for, listening first. Nothing of the session
    /// follows it.
    fn send_identifier(&mut self) {
        let length = self.id_owed;
        if length <= 0.0 {
            return;
        }
        if self.state != State::Idle
            && !(self.state == State::Connecting && self.connect_tries == 0)
        {
            return; // a session is under way: it ends with an identifier of its own
        }
        if self.now < self.hold_until - 1e-9 {
            self.deadlines.set(Timer::Ident, self.hold_until);
            return;
        }
        self.id_owed = 0.0;
        let keying = Keying {
            identifier_s: length,
            listen_first: self.id_listen,
            ..Keying::default()
        };
        let end = self.transmit(Vec::new(), keying, None);
        // a call or a probe of this station's, and an answer to another's, waits for it
        self.hold_until = self.hold_until.max(end);
    }

    /// No identifier for the session just ended: the communication goes on in a new one.
    fn drop_identifier(&mut self) {
        self.id_owed = 0.0;
        self.deadlines.remove(Timer::Ident);
    }

    /// A `DISC_ACK` that carried the closing identifier did not go out: the identifier is owed
    /// again — behind the next `DISC_ACK`, or on its own if no DISC comes (ADR-0075).
    fn owe_again(&mut self, ident: f64) {
        if ident <= 0.0 || self.state != State::Idle || self.id_owed > 0.0 {
            return;
        }
        self.id_owed = ident;
        self.id_alone = true;
        self.id_listen = true;
        self.deadlines
            .set(Timer::Ident, self.now + self.exchange_s());
    }

    fn end_session(&mut self, reason: &str, disc_heard: bool) {
        self.end_session_with(reason, disc_heard, false);
    }

    /// A master's session that ends with the other station unread: every DISC unanswered.
    fn end_session_unread(&mut self, reason: &str) {
        self.end_session_with(reason, false, true);
    }

    fn end_session_with(&mut self, reason: &str, disc_heard: bool, unread: bool) {
        let left = (self.state != State::Connecting
            && (LEFT_UNHEARD.contains(&reason) || disc_heard))
            .then(|| (self.session, self.now + self.link_timeout(), disc_heard));
        // a session that came up ends with this station's closing identifier — unless the other
        // station ended it by calling again, which goes on in the session its call opens
        // (ADR-0055 §4); a call never answered is the daemon's to identify (ADR-0055)
        let owes = self.state != State::Connecting
            && self.my_ident_s > 0.0
            && !reason.ends_with(" called again");
        let was_master = self.role == Role::Master;
        self.ended_peer_snr_db = self.peer_snr_db;
        let kept = (
            self.peer_turnaround_s,
            self.peer_identifier_s,
            self.silence_level,
        );
        self.state = State::Idle;
        self.role = Role::None;
        self.deadlines.remove(Timer::Connect);
        self.deadlines.remove(Timer::Ident);
        self.reset_session();
        self.tx_queue.clear();
        // what answering or leaving the old session in its slots still needs
        (
            self.peer_turnaround_s,
            self.peer_identifier_s,
            self.silence_level,
        ) = kept;
        self.left = left;
        self.id_owed = if owes { self.my_ident_s } else { 0.0 };
        self.event("disconnected", reason.to_owned());
        if !owes {
            return;
        }
        // where the closing identifier goes (ADR-0075): the master's once the last slot it
        // granted can no longer be in use — an aborting master's once its leaving is over; a
        // slave's behind its DISC_ACK, or on its own, listening first, when nothing of the
        // session is heard for an exchange — a live master keys at least that often
        self.id_alone = !was_master;
        // a master that read nothing of the other station at the end listens before its own: the
        // other may have given the session up first and be sending its identifier alone
        self.id_listen =
            self.id_alone || unread || matches!(reason, "no response" | "link timeout");
        if was_master && reason != "aborted" {
            self.deadlines
                .set(Timer::Ident, self.now.max(self.hold_until));
        } else if !was_master && reason == "link timeout" {
            // a slave that timed out cannot hear the master, so listening first cannot keep
            // their identifiers apart: it waits until the master has given up and identified.
            // The master gives up no later than its own link timeout after the last answer it
            // can have read — this station's last slot, inside the exchange after the frame that
            // last armed this station's timer — and identifies then (I1; the band-closes
            // scenario's two identifiers met 0.6 s apart)
            let at = self.now + self.exchange_s() + self.peer_identifier_s;
            self.deadlines.set(Timer::Ident, at);
        } else if !was_master {
            let at = if left.is_none() {
                self.now
            } else {
                self.now + self.exchange_s()
            };
            self.deadlines.set(Timer::Ident, at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rate::{AWGN_THRESHOLD_DB, FRAME_S, PAYLOAD_BYTES};

    /// The 2 300 Hz air's timing, as the model's harness hands it to the engine.
    fn wide() -> PhyTiming {
        PhyTiming {
            data_frame_s: FRAME_S[6],
            control_frame_s: 0.434,
            turnaround_s: 0.25,
            detect_latency_s: 0.15,
            tx_latency_s: 0.0,
            preamble_detect_s: Some(0.31),
            data_capacity: PAYLOAD_BYTES.to_vec(),
            mode_threshold_db: AWGN_THRESHOLD_DB.to_vec(),
            floor_data_frame_s: Some(FRAME_S[0]),
            floor_control_frame_s: Some(3.2),
            floor_modes: 6,
            control_threshold_db: Some(CONTROL_THRESHOLD_DB),
            floor_margin_db: Some(1.0),
            floor_preamble_detect_s: Some(0.54),
        }
    }

    fn record(mode: usize, decoded: bool) -> RxRecord {
        RxRecord {
            frame: Heard {
                mode,
                floor: false,
                rv: 0,
                snr_db: 8.5,
                trusted: true,
                follows: None,
                t_start: 0.0,
                t_end: 1.0,
            },
            slot: 0,
            payload: decoded.then(Vec::new),
            seq: None,
            combined: false,
        }
    }

    #[test]
    fn a_transmission_split_evenly_between_two_rungs_is_judged_at_the_slower() {
        // the model breaks the tie the same way (test_granted.py); the trace vectors never
        // happen to carry a tie with a failure in it
        let mut engine = GrantedEngine::new("W4ODA", wide(), GrantConfig::default(), 1);
        engine.observe(&[
            record(12, true),
            record(12, false),
            record(9, true),
            record(9, false),
        ]);
        let judged_at = |mode| {
            let mut rate = rate_controller_for(&wide(), RateConfig::default());
            rate.observe(Some(8.5), 2, 2, Some(mode));
            rate.margin_db()
        };
        assert!(
            (judged_at(9) - judged_at(12)).abs() > 0.1,
            "the rung must change the controller for this to mean anything"
        );
        assert!((engine.rate.margin_db() - judged_at(9)).abs() < 1e-12);
    }

    #[test]
    fn the_draws_are_the_models() {
        // SplitMix64 from seed 0, as `granted.Draws(0)` draws it
        let mut draws = Draws::new(0);
        assert_eq!(draws.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(draws.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }
}
