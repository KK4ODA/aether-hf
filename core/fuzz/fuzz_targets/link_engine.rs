// SPDX-License-Identifier: MIT OR Apache-2.0
//! The link engine in a session, fed what the air can bring it. Two engines, the station called
//! (`W4ODA`) and its caller (`N0CALL`), on a small air of their own: each transmission's frames
//! reach the other engine as they end, and the input decides what happens between them — a
//! frame anyone could have sent, a frame of the caller's altered on the way, a frame lost, time
//! passing, bytes to send, an abort. The frames are ones the physical layer could hand over: a
//! mode on the air's ladder, a redundancy version of two bits, a payload of the frame's size.
//!
//! What is checked besides panics: while the air has only lost frames — nothing injected, nothing
//! altered — what each engine delivers is the start of what the other was given to send, in
//! order. The ARQ promises that whatever the losses.
#![no_main]

use aether_link::{
    Action, CONTROL_BYTES, Container, GrantConfig, GrantedEngine, HarqBuffer, PhyTiming, SoftFrame,
    TxFrame,
};
use aether_phy::waveform::{NARROW_500, WIDE_2300};
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

/// A frame as the physical layer hands it to the engine.
struct Heard {
    container: Container,
    mode: usize,
    floor: bool,
    rv: u8,
    snr_db: f64,
    trusted: bool,
    follows: Option<u8>,
    echo: Option<f64>,
    t_start: f64,
    t_end: f64,
    payload: Option<Vec<u8>>,
}

impl SoftFrame for Heard {
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
    fn trusted(&self) -> bool {
        self.trusted
    }
    fn follows(&self) -> Option<u8> {
        self.follows
    }
    fn echo(&self) -> Option<f64> {
        self.echo
    }
    fn t_start(&self) -> f64 {
        self.t_start
    }
    fn t_end(&self) -> f64 {
        self.t_end
    }
    fn decode(&self, _buffer: Option<&HarqBuffer>) -> (Option<Vec<u8>>, HarqBuffer) {
        (self.payload.clone(), vec![0.0])
    }
}

/// What happens to the caller's frames on the way to the station called.
#[derive(Clone, Copy, PartialEq)]
enum Path {
    Clear,
    Lose,
    Alter,
}

struct Air {
    now: f64,
    timing: PhyTiming,
    called: GrantedEngine,
    caller: GrantedEngine,
    /// What each was given to send, and what each delivered.
    sent: [Vec<u8>; 2],
    delivered: [Vec<u8>; 2],
    /// Nothing but losses so far: the delivered streams must be the sent ones' starts.
    honest: bool,
    /// The air loses frames and nothing else, for the whole run: half the inputs, so that the
    /// check of the delivered streams is made over long sessions, not only until the first
    /// frame the input alters.
    losses_only: bool,
    /// Transmissions asked for and not yet keyed, each with whether the caller sent it.
    queued: Vec<(bool, Transmission)>,
}

/// One transmission an engine asked for: its frames, its air time, and the window the keying
/// gate keeps it to.
struct Transmission {
    frames: Vec<TxFrame>,
    duration_s: f64,
    not_before: Option<f64>,
    not_after: Option<f64>,
}

/// The engine's own actions: its transmissions kept, its deliveries recorded.
fn transmissions(engine: &mut GrantedEngine, delivered: &mut Vec<u8>) -> Vec<Transmission> {
    let mut out = Vec::new();
    for action in engine.drain() {
        match action {
            Action::Transmit {
                frames,
                duration_s,
                not_before,
                not_after,
                ..
            } => out.push(Transmission {
                frames,
                duration_s: duration_s.max(0.0),
                not_before,
                not_after,
            }),
            Action::Deliver(bytes) => delivered.extend_from_slice(&bytes),
            Action::Event { .. } => {}
        }
    }
    out
}

impl Air {
    fn new(narrow: bool) -> Air {
        let params = if narrow { NARROW_500 } else { WIDE_2300 };
        let timing = aetherd::phy_timing(params);
        let config = GrantConfig::default();
        Air {
            now: 0.0,
            called: GrantedEngine::new("W4ODA", timing.clone(), config.clone(), 1),
            caller: GrantedEngine::new("N0CALL", timing.clone(), config, 2),
            timing,
            sent: [Vec::new(), Vec::new()],
            delivered: [Vec::new(), Vec::new()],
            honest: true,
            losses_only: false,
            queued: Vec::new(),
        }
    }

    /// The next thing on the air: the transmission due first goes out, its frames reaching the
    /// other engine as they end, unless an engine's timer comes before it; then the clock moves
    /// there and both engines are ticked. One transmission at a time, as on a half-duplex channel.
    fn exchange(&mut self, path: Path, u: &mut Unstructured<'_>) -> Result<()> {
        for tx in transmissions(&mut self.called, &mut self.delivered[0]) {
            self.queued.push((false, tx));
        }
        for tx in transmissions(&mut self.caller, &mut self.delivered[1]) {
            self.queued.push((true, tx));
        }
        let start = |tx: &Transmission, now: f64| tx.not_before.map_or(now, |t| t.max(now));
        let first = (0..self.queued.len()).min_by(|&a, &b| {
            start(&self.queued[a].1, self.now).total_cmp(&start(&self.queued[b].1, self.now))
        });
        let deadline = [self.called.next_deadline(), self.caller.next_deadline()]
            .into_iter()
            .flatten()
            .fold(f64::INFINITY, f64::min);
        match first {
            Some(i) if start(&self.queued[i].1, self.now) <= deadline => {
                let (from_caller, tx) = self.queued.remove(i);
                let path = if from_caller { path } else { Path::Clear };
                self.key(&tx, from_caller, path, u)?;
            }
            _ => {
                self.now = if deadline.is_finite() {
                    deadline.max(self.now)
                } else {
                    self.now + 1.0
                };
                self.called.tick(self.now);
                self.caller.tick(self.now);
            }
        }
        Ok(())
    }

    /// A transmission through the keying gate: it waits for its `not_before`, and one that would
    /// start past its `not_after` is not sent at all, and its engine told (ADR-0069 §2.3).
    fn key(
        &mut self,
        tx: &Transmission,
        to_called: bool,
        path: Path,
        u: &mut Unstructured<'_>,
    ) -> Result<()> {
        self.now = self.now.max(tx.not_before.unwrap_or(self.now));
        let sender = if to_called {
            &mut self.caller
        } else {
            &mut self.called
        };
        if tx.not_after.is_some_and(|latest| self.now > latest) {
            sender.on_tx_dropped(self.now);
            return Ok(());
        }
        let end = self.carry(&tx.frames, tx.duration_s, to_called, path, u)?;
        let sender = if to_called {
            &mut self.caller
        } else {
            &mut self.called
        };
        sender.on_tx_done(end, Some(end));
        Ok(())
    }

    /// One transmission on the air: its frames, back to back, to the other engine.
    fn carry(
        &mut self,
        frames: &[TxFrame],
        duration_s: f64,
        to_called: bool,
        path: Path,
        u: &mut Unstructured<'_>,
    ) -> Result<f64> {
        let each = duration_s / frames.len().max(1) as f64;
        for frame in frames {
            let t_start = self.now;
            self.now += each;
            let mut payload = frame.payload.clone();
            match path {
                Path::Clear => {}
                Path::Lose => continue,
                Path::Alter => {
                    self.honest = false;
                    for byte in &mut payload {
                        *byte ^= u.arbitrary::<u8>()?;
                    }
                }
            }
            let heard = Heard {
                container: frame.container,
                mode: frame.mode,
                floor: if frame.container == Container::Data {
                    frame.mode < self.timing.floor_modes
                } else {
                    frame.floor
                },
                rv: frame.rv,
                snr_db: 10.0,
                trusted: true,
                // the tone floor's frames, and control frames, cannot say (ADR-0041)
                follows: (frame.container == Container::Data
                    && frame.mode >= self.timing.floor_modes)
                    .then_some(frame.follows),
                echo: None,
                t_start,
                t_end: self.now,
                payload: Some(payload),
            };
            if to_called {
                self.called.on_frame(&heard, self.now);
            } else {
                self.caller.on_frame(&heard, self.now);
            }
        }
        Ok(self.now)
    }

    /// A frame anyone on the frequency could have sent, to the station called.
    fn inject(&mut self, u: &mut Unstructured<'_>) -> Result<()> {
        self.honest = false;
        let control = u.arbitrary::<bool>()?;
        let modes = self.timing.data_capacity.len().max(1);
        let mode = if control { 0 } else { u.choose_index(modes)? };
        let size = if control {
            CONTROL_BYTES
        } else {
            self.timing.data_capacity.get(mode).copied().unwrap_or(0)
        };
        let mut payload = vec![0u8; size];
        u.fill_buffer(&mut payload)?;
        let length_s = f64::from(u.int_in_range(1u16..=3000)?) / 100.0;
        let t_start = self.now;
        self.now += length_s;
        let heard = Heard {
            container: if control {
                Container::Control
            } else {
                Container::Data
            },
            mode,
            floor: u.arbitrary()?,
            rv: u.int_in_range(0..=3)?,
            snr_db: f64::from(u.int_in_range(-30i8..=40)?),
            trusted: u.arbitrary()?,
            follows: if u.arbitrary()? {
                Some(u.int_in_range(0..=3)?)
            } else {
                None
            },
            echo: if u.arbitrary()? {
                Some(f64::from(u.int_in_range(0u8..=100)?) / 100.0)
            } else {
                None
            },
            t_start,
            t_end: self.now,
            payload: if u.ratio(9, 10)? { Some(payload) } else { None },
        };
        self.called.on_frame(&heard, self.now);
        Ok(())
    }

    fn send(&mut self, to_caller: bool, u: &mut Unstructured<'_>) -> Result<()> {
        let n = u.int_in_range(1usize..=600)?;
        let bytes: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
        if to_caller {
            self.caller.send(&bytes);
            self.sent[1].extend_from_slice(&bytes);
        } else {
            self.called.send(&bytes);
            self.sent[0].extend_from_slice(&bytes);
        }
        Ok(())
    }

    /// One step of the run. Endings (a disconnect, an abort) come only in its last quarter, so
    /// that a session runs long enough to carry what was sent: the caller does not call again.
    fn step(&mut self, u: &mut Unstructured<'_>, late: bool) -> Result<()> {
        let last = if late { 29 } else { 28 };
        match u.int_in_range(0u8..=last)? {
            0..=17 => self.exchange(Path::Clear, u)?,
            18..=19 => self.exchange(Path::Lose, u)?,
            20..=22 if self.losses_only => self.exchange(Path::Lose, u)?,
            20 | 21 => self.exchange(Path::Alter, u)?,
            22 => self.inject(u)?,
            23..=25 => self.send(u.arbitrary()?, u)?,
            26..=27 => {
                self.now += f64::from(u.int_in_range(1u16..=1200)?) / 10.0;
                self.called.tick(self.now);
                self.caller.tick(self.now);
            }
            28 => {
                let frame_s = self.timing.data_frame_s;
                self.called.on_preamble(self.now, self.now, Some(frame_s));
            }
            _ => match u.int_in_range(0u8..=2)? {
                0 => self.called.disconnect(),
                1 => self.called.abort(),
                _ => self.caller.disconnect(),
            },
        }
        Ok(())
    }

    /// While nothing but losses has happened on the air, each engine delivered the start of what
    /// the other was given to send.
    fn check(&self) {
        if !self.honest {
            return;
        }
        for (delivered, sent) in [
            (&self.delivered[0], &self.sent[1]),
            (&self.delivered[1], &self.sent[0]),
        ] {
            assert!(
                sent.starts_with(delivered),
                "delivered {} bytes that are not the start of the {} sent",
                delivered.len(),
                sent.len()
            );
        }
    }
}

/// Steps in a run, after the call.
const STEPS: usize = 400;

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(narrow) = u.arbitrary::<bool>() else {
        return;
    };
    let mut air = Air::new(narrow);
    air.losses_only = u.arbitrary::<bool>().unwrap_or(false);
    if air.caller.connect("W4ODA", None).is_err() {
        return;
    }
    // the call and its acceptance on a clear air, unless the input says otherwise from the start
    for _ in 0..40 {
        if air.exchange(Path::Clear, &mut u).is_err() {
            break;
        }
    }
    for n in 0..STEPS {
        if u.is_empty() || air.step(&mut u, n >= STEPS * 3 / 4).is_err() {
            break;
        }
        air.check();
    }
    air.check();
});
