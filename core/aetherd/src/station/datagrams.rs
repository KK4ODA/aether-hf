//! Datagrams for KISS clients (ADR-0019): a client's frame sent outside any session, and the
//! frames other stations sent, joined and handed on.
//!
//! A datagram waits in a queue of its own and goes out only while no session is up — the
//! session's turn-taking has no room for a stranger's burst — as bursts that each fit the key
//! (`burst_limit_s`), at the rung the client's configuration names (the tone floor's `tone-36`
//! by default, which a station of either bandwidth decodes). It reaches the air through the
//! same queue, gate and keying as everything else: `start_pending` feeds the next burst in
//! when the queue is empty, after the client's channel access — the busy detector, then
//! p-persistence, KISS's own P and SLOTTIME — says it may.

use std::collections::VecDeque;

use aether_link::datagram::{
    DatagramError, Reassembler, body, fragments, max_payload, parse_body, read_fragment,
};

use super::{Outgoing, State, Station, burst_limit_s};
use crate::ptt::Ptt;

/// The most datagrams waiting to go; a client past it is told to wait (TCP backpressure).
pub const DATAGRAM_QUEUE: usize = 16;

/// The most joined datagrams, and the most reports, held between two takes: a client that
/// never drains them loses the oldest rather than growing a heap.
const MAX_UNTAKEN: usize = 256;

/// How long a datagram's pieces are waited for once the first has arrived. Bursts of one
/// datagram follow each other within a key's length and a channel's wait; two minutes covers
/// both with room, and a piece lost for good is not waited for longer.
const REASSEMBLY_TIMEOUT_S: f64 = 120.0;

/// What a client asks to send.
#[derive(Debug, Clone, PartialEq)]
pub struct DatagramRequest {
    /// The frame type the client gave: 0 AX.25, 1 AX.25 with eight-byte addresses, 2 data.
    pub frame_type: u8,
    /// The frame, as the client handed it over.
    pub frame: Vec<u8>,
    /// The client's name for it, reported back when it has gone out (KISS ACKMODE).
    pub reference: Option<String>,
    /// The rung to send it at; the tone floor's `tone-36` when none.
    pub rung: Option<usize>,
    /// Wait for a clear channel first (VARA's KISS DCD; `IGNOREKISSDCD` turns it off).
    pub wait_for_clear: bool,
    /// p-persistence: the chance of going in any slot once the channel is clear.
    pub persistence: f64,
    /// The slot, seconds.
    pub slot_s: f64,
}

/// What the station made of a request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatagramQueued {
    /// Datagrams waiting, this one included.
    pub queued: usize,
    /// Fragments it went into.
    pub fragments: usize,
    /// Keyings it takes.
    pub bursts: usize,
    /// Seconds on the air, all bursts together.
    pub air_s: f64,
    /// The rung it goes at.
    pub rung: usize,
}

/// Why a datagram was not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatagramRefusal {
    /// The queue is full: try again when it has room.
    QueueFull,
    /// It cannot be sent: too long for the rung, an unknown type, an empty frame.
    Invalid(String),
    /// The station may not start an exchange (answer-only).
    NotAllowed(&'static str),
}

/// A datagram another station sent, joined.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedDatagram {
    /// The sending station's callsign, as its datagram carried it.
    pub source: String,
    /// The frame type it came with.
    pub frame_type: u8,
    /// The frame.
    pub frame: Vec<u8>,
    /// The SNR of its last piece.
    pub snr_db: f64,
    /// The rung it came at.
    pub rung: usize,
}

/// What became of a datagram sent with a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatagramReport {
    /// The client's reference.
    pub reference: String,
    /// Whether all of it went out.
    pub sent: bool,
    /// Why not, when it did not.
    pub reason: Option<String>,
}

/// A datagram waiting: its remaining bursts, and how it may reach the channel.
#[derive(Debug, Clone)]
pub(super) struct QueuedDatagram {
    pub(super) reference: Option<String>,
    pub(super) bursts: VecDeque<Vec<aether_link::TxFrame>>,
    pub(super) wait_for_clear: bool,
    pub(super) persistence: f64,
    pub(super) slot_s: f64,
}

/// A datagram's last burst, on the air: what to report when it has left.
#[derive(Debug, Clone)]
pub(super) struct OnAir {
    pub(super) reference: Option<String>,
}

/// The station's datagram state.
#[derive(Debug)]
pub(super) struct Datagrams {
    pub(super) queue: VecDeque<QueuedDatagram>,
    number: u8,
    reassembler: Reassembler,
    received: Vec<ReceivedDatagram>,
    reports: Vec<DatagramReport>,
    /// The datagram whose last burst is on the air.
    pub(super) on_air: Option<OnAir>,
    /// When the channel may next be tried (p-persistence's slot).
    next_try_s: f64,
    rng: u64,
    /// Datagrams sent, whole.
    pub(super) sent: usize,
    /// Datagrams heard, whole.
    pub(super) heard: usize,
}

impl Datagrams {
    pub(super) fn new(seed: u64) -> Self {
        Self {
            queue: VecDeque::new(),
            number: 0,
            reassembler: Reassembler::new(REASSEMBLY_TIMEOUT_S, 8),
            received: Vec::new(),
            reports: Vec::new(),
            on_air: None,
            next_try_s: f64::NEG_INFINITY,
            rng: seed ^ 0x9E37_79B9_7F4A_7C15 | 1,
            sent: 0,
            heard: 0,
        }
    }

    /// The next datagram number, 1–255.
    fn next_number(&mut self) -> u8 {
        self.number = if self.number == 255 {
            1
        } else {
            self.number + 1
        };
        self.number
    }

    /// A draw in [0, 1): xorshift, for the persistence coin.
    fn draw(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    pub(super) fn report(&mut self, reference: Option<String>, sent: bool, reason: Option<String>) {
        let Some(reference) = reference else { return };
        if self.reports.len() >= MAX_UNTAKEN {
            self.reports.remove(0);
        }
        self.reports.push(DatagramReport {
            reference,
            sent,
            reason,
        });
    }

    /// Drop every datagram waiting, reporting each: a station stopping, or told to.
    pub(super) fn clear(&mut self, reason: &str) {
        for datagram in std::mem::take(&mut self.queue) {
            self.report(datagram.reference, false, Some(reason.to_owned()));
        }
    }

    /// The datagrams waiting and the ones heard, for a status display.
    pub(super) fn waiting(&self) -> usize {
        self.queue.len()
    }

    pub(super) fn take_received(&mut self) -> Vec<ReceivedDatagram> {
        std::mem::take(&mut self.received)
    }

    pub(super) fn take_reports(&mut self) -> Vec<DatagramReport> {
        std::mem::take(&mut self.reports)
    }
}

impl<P: Ptt> Station<P> {
    /// Queue a KISS client's frame to go out as a datagram (ADR-0019).
    ///
    /// # Errors
    /// The queue is full ([`DatagramRefusal::QueueFull`], try again), the frame cannot go at
    /// the rung asked for, or the station may not start an exchange.
    pub fn send_datagram(
        &mut self,
        request: DatagramRequest,
    ) -> Result<DatagramQueued, DatagramRefusal> {
        if self.config.answer_only {
            return Err(DatagramRefusal::NotAllowed(
                "this station is answer-only: it starts no exchange, and a datagram is one",
            ));
        }
        if self.datagrams.queue.len() >= DATAGRAM_QUEUE {
            return Err(DatagramRefusal::QueueFull);
        }
        let timing = self.engine.timing().clone();
        let rungs = timing.data_capacity.len();
        let rung = request.rung.unwrap_or(1).min(rungs.saturating_sub(1));
        let capacity = timing.capacity(rung);
        let invalid = |e: DatagramError| DatagramRefusal::Invalid(e.to_string());
        let carried =
            body(&self.engine.my_call, request.frame_type, &request.frame).map_err(invalid)?;
        if carried.len() > max_payload(capacity) {
            return Err(DatagramRefusal::Invalid(format!(
                "{} bytes do not fit the {} a datagram carries at rung {rung}",
                request.frame.len(),
                max_payload(capacity).saturating_sub(aether_link::datagram::HEADER_BYTES)
            )));
        }
        let number = self.datagrams.next_number();
        let pieces = fragments(&carried, number, capacity).map_err(invalid)?;
        let frame_s = timing.data_frame_s_for(rung);
        let per_burst = ((burst_limit_s(&self.config, false) / frame_s).floor() as usize).max(1);
        let frames: Vec<aether_link::TxFrame> = pieces
            .into_iter()
            .map(|payload| aether_link::TxFrame {
                container: aether_link::Container::Data,
                payload,
                mode: rung,
                rv: 0,
                floor: false,
            })
            .collect();
        let count = frames.len();
        let bursts: VecDeque<Vec<aether_link::TxFrame>> =
            frames.chunks(per_burst).map(<[_]>::to_vec).collect();
        let queued = DatagramQueued {
            queued: self.datagrams.queue.len() + 1,
            fragments: count,
            bursts: bursts.len(),
            air_s: count as f64 * frame_s,
            rung,
        };
        self.datagrams.queue.push_back(QueuedDatagram {
            reference: request.reference,
            bursts,
            wait_for_clear: request.wait_for_clear,
            persistence: request.persistence.clamp(0.05, 1.0),
            slot_s: request.slot_s.clamp(0.02, 2.0),
        });
        Ok(queued)
    }

    /// Put the next datagram burst on the transmit queue, when nothing else is on it, no
    /// session is up, and the channel access allows. Called from `start_pending`.
    pub(super) fn feed_datagram(&mut self, now: f64) {
        if !self.pending.is_empty()
            || self.transmitting
            || self.engine.state() != State::Idle
            || self.engine.probing()
        {
            return;
        }
        let Some(front) = self.datagrams.queue.front() else {
            return;
        };
        if front.wait_for_clear && (!self.busy.settled() || self.busy.busy(now)) {
            return;
        }
        if now < self.datagrams.next_try_s {
            return;
        }
        let (persistence, slot_s) = (front.persistence, front.slot_s);
        if self.datagrams.draw() >= persistence {
            self.datagrams.next_try_s = now + slot_s;
            return;
        }
        let Some(front) = self.datagrams.queue.front_mut() else {
            return;
        };
        let Some(burst) = front.bursts.pop_front() else {
            self.datagrams.queue.pop_front();
            return;
        };
        let last = front.bursts.is_empty();
        let reference = front.reference.clone();
        if last {
            self.datagrams.queue.pop_front();
        }
        self.pending.push_back(Outgoing::Datagram {
            frames: burst,
            reference,
            last,
        });
    }

    /// A datagram burst the gate refused: the rest of that datagram goes too, reported.
    pub(super) fn refuse_datagram(&mut self, reference: Option<String>, last: bool, why: &str) {
        if !last {
            // its remaining bursts are the front of the queue
            if let Some(front) = self.datagrams.queue.pop_front() {
                self.datagrams
                    .report(front.reference, false, Some(why.to_owned()));
                return;
            }
        }
        self.datagrams
            .report(reference, false, Some(why.to_owned()));
    }

    /// A datagram's last burst left the transmitter — whole, or cut short.
    pub(super) fn datagram_transmitted(&mut self, cut: bool) {
        let Some(OnAir { reference }) = self.datagrams.on_air.take() else {
            return;
        };
        if cut {
            self.datagrams.report(
                reference,
                false,
                Some("the transmission was cut short".to_owned()),
            );
        } else {
            self.datagrams.sent += 1;
            self.datagrams.report(reference, true, None);
        }
    }

    /// A decoded DATAGRAM frame: join it with its datagram's other pieces, and when it
    /// completes one, keep it for the KISS clients. Returns the sender when this piece named it
    /// (the first piece carries the callsign), for the frame's report.
    pub(super) fn heard_datagram_piece(
        &mut self,
        payload: &[u8],
        snr_db: f64,
        rung: usize,
        now: f64,
    ) -> Option<String> {
        let fragment = read_fragment(payload)?;
        let first = fragment.index == 0;
        let named = if first {
            aether_link::frames::unpack_callsign(&fragment.body).ok()
        } else {
            None
        };
        if let Some(joined) = self.datagrams.reassembler.add(fragment, now)
            && let Some((source, frame_type, frame)) = parse_body(&joined)
        {
            self.datagrams.heard += 1;
            if self.datagrams.received.len() >= MAX_UNTAKEN {
                self.datagrams.received.remove(0);
            }
            self.datagrams.received.push(ReceivedDatagram {
                source,
                frame_type,
                frame,
                snr_db,
                rung,
            });
        }
        named.filter(|call| !call.is_empty())
    }

    /// Datagrams heard since the last call, for the KISS clients.
    pub fn take_received_datagrams(&mut self) -> Vec<ReceivedDatagram> {
        self.datagrams.take_received()
    }

    /// What became of datagrams sent with a reference, since the last call.
    pub fn take_datagram_reports(&mut self) -> Vec<DatagramReport> {
        self.datagrams.take_reports()
    }

    /// Datagrams waiting to go, and the counters.
    #[must_use]
    pub fn datagram_status(&self) -> serde_json::Value {
        serde_json::json!({
            "queued": self.datagrams.waiting(),
            "limit": DATAGRAM_QUEUE,
            "sent": self.datagrams.sent,
            "heard": self.datagrams.heard,
            "incomplete_dropped": self.datagrams.reassembler.dropped,
        })
    }

    /// Drop every datagram waiting — the KISS server was switched off, or told to.
    pub fn clear_datagrams(&mut self, reason: &str) {
        self.datagrams.clear(reason);
    }
}
