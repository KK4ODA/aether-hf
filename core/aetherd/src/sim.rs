//! A simulated channel between two daemons, in place of a sound card.
//!
//! Two stations on one machine — or two machines on one network — joined by a socket, with
//! noise added at a chosen signal-to-noise ratio and the samples paced at 48 kHz by the
//! wall clock. It is what lets Pat, Winlink Express or `VarAC` be driven end to end with no
//! radio, what a demo runs on, and the bench reference a field session is compared with:
//! the same two daemons, the same host software, a channel whose SNR is a number.
//!
//! The channel is additive white Gaussian noise only. Fading, multipath and offsets live in
//! the reference model's simulator (`model/aether_model/channel.py`), which is calibrated
//! and benchmarked; this is a wire with noise on it, and it says so.
//!
//! # Timing
//!
//! A sound card delivers samples at its own rate whether or not anything is playing, and
//! the modem's clock is the audio it has heard, so this backend has to do the same: every
//! call to [`AudioIo::capture`] returns as many samples as the wall clock says have elapsed,
//! taken from what the peer sent and padded with silence (and noise) when it sent nothing.
//! What this side plays goes to the peer as soon as it is queued, which is up to a quarter
//! of a second ahead of real time — the daemon's own playback backlog — so a loop that
//! stalls for less than that leaves no gap in the peer's copy.

use std::{
    collections::VecDeque,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use crate::audio::AudioIo;

/// How the two ends find each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peer {
    /// Wait for the other end to connect here.
    Listen(String),
    /// Connect to the other end, retrying until it answers.
    Connect(String),
}

/// A simulated channel's settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// The other end.
    pub peer: Peer,
    /// Signal to noise at this receiver, in a 3 kHz noise bandwidth (the project's
    /// convention), relative to `signal_rms` — the level the other end transmits at.
    pub snr_db: f64,
    /// The RMS level the peer's waveform arrives at: its `tx_level / √2`, the waveform's
    /// RMS being 3 dB below the level a sine of that amplitude would announce.
    pub signal_rms: f64,
    /// Samples per second.
    pub sample_rate: u32,
}

/// Samples in flight between the modem thread and the socket.
#[derive(Debug, Default)]
struct Shared {
    /// What the peer sent, not yet delivered as captured audio.
    from_peer: VecDeque<f32>,
    /// Whether the peer has been found.
    connected: bool,
    /// Whether it has since gone away.
    lost: bool,
}

/// The backend.
pub struct SimLink {
    shared: Arc<Mutex<Shared>>,
    to_peer: mpsc::Sender<Vec<f32>>,
    running: Arc<AtomicBool>,
    sample_rate: u32,
    noise_sigma: f32,
    /// Where the clock was when capture last ran.
    last_capture: Instant,
    /// Fractional samples carried between captures, so the rate is exact over time.
    carry: f64,
    /// Samples played in total.
    played: u64,
    /// How many of them the wall clock has consumed, and when that was last worked out.
    /// A cell because the trait asks with `&self`, and the answer moves with the clock.
    consumed: std::cell::Cell<(Instant, u64)>,
    noise: Noise,
    /// Human-readable, for the log.
    pub description: String,
}

impl std::fmt::Debug for SimLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimLink")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

/// Twenty milliseconds: capture hands over at least this much at a time, like a sound card.
const MIN_BLOCK_S: f64 = 0.02;

impl SimLink {
    /// Open the channel. Neither end blocks: a listener waits for its peer in the
    /// background, a connector retries for thirty seconds, and until the peer is there the
    /// channel is silence — so either daemon may be started first and both come up with
    /// their control interfaces answering.
    ///
    /// # Errors
    /// If a listening address cannot be bound.
    pub fn open(config: &SimConfig) -> std::io::Result<Self> {
        let listener = match &config.peer {
            Peer::Listen(address) => Some(TcpListener::bind(address)?),
            Peer::Connect(_) => None,
        };
        let shared = Arc::new(Mutex::new(Shared::default()));
        let running = Arc::new(AtomicBool::new(true));
        let (to_peer, from_modem) = mpsc::channel::<Vec<f32>>();
        let (stream_ready, stream_for_writer) = mpsc::channel::<TcpStream>();

        // the reader: find the peer, then bytes off the socket become the peer's audio
        let reader_shared = Arc::clone(&shared);
        let reader_running = Arc::clone(&running);
        let peer = config.peer.clone();
        std::thread::Builder::new()
            .name("sim-reader".into())
            .spawn(move || {
                let Some(stream) = find_peer(&peer, listener) else {
                    if let Ok(mut shared) = reader_shared.lock() {
                        shared.lost = true;
                    }
                    return;
                };
                let _ = stream.set_nodelay(true);
                if let Ok(writer) = stream.try_clone() {
                    let _ = stream_ready.send(writer);
                }
                if let Ok(mut shared) = reader_shared.lock() {
                    shared.connected = true;
                }
                let mut reader = stream;
                let mut buffer = vec![0u8; 4 * 960];
                // a read can end mid-sample: the odd bytes wait for the next one
                let mut pending: Vec<u8> = Vec::new();
                while reader_running.load(Ordering::Relaxed) {
                    match reader.read(&mut buffer) {
                        Ok(0) | Err(_) => {
                            if let Ok(mut shared) = reader_shared.lock() {
                                shared.lost = true;
                            }
                            return;
                        }
                        Ok(n) => {
                            pending.extend_from_slice(&buffer[..n]);
                            let whole = pending.len() - pending.len() % 4;
                            let samples: Vec<f32> = pending[..whole]
                                .chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                                .collect();
                            pending.drain(..whole);
                            if let Ok(mut shared) = reader_shared.lock() {
                                shared.from_peer.extend(samples);
                            }
                        }
                    }
                }
            })?;

        // the writer: what the modem plays goes to the peer, once there is one; what it
        // played before that went out into the void, as it would have on the air
        let writer_running = Arc::clone(&running);
        std::thread::Builder::new()
            .name("sim-writer".into())
            .spawn(move || {
                let mut writer: Option<TcpStream> = None;
                while writer_running.load(Ordering::Relaxed) {
                    match from_modem.recv_timeout(Duration::from_millis(100)) {
                        Ok(samples) => {
                            // the peer may have arrived while this thread was waiting
                            if writer.is_none() {
                                writer = stream_for_writer.try_recv().ok();
                            }
                            if let Some(stream) = writer.as_mut() {
                                let bytes: Vec<u8> =
                                    samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                                if stream.write_all(&bytes).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            })?;

        let now = Instant::now();
        Ok(Self {
            shared,
            to_peer,
            running,
            sample_rate: config.sample_rate,
            noise_sigma: noise_sigma(config.snr_db, config.signal_rms) as f32,
            last_capture: now,
            carry: 0.0,
            played: 0,
            consumed: std::cell::Cell::new((now, 0)),
            noise: Noise::new(0x9E37_79B9_7F4A_7C15),
            description: format!(
                "simulated channel ({}), {:.0} dB SNR in 3 kHz",
                match &config.peer {
                    Peer::Listen(a) => format!("listening on {a}"),
                    Peer::Connect(a) => format!("connecting to {a}"),
                },
                config.snr_db
            ),
        })
    }

    /// Whether the peer has been found.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.connected)
    }

    /// Whether the peer has gone away.
    #[must_use]
    pub fn lost(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.lost)
    }
}

impl Drop for SimLink {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

impl AudioIo for SimLink {
    fn capture(&mut self) -> Vec<f32> {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_capture).as_secs_f64();
        if elapsed < MIN_BLOCK_S {
            return Vec::new();
        }
        self.last_capture = now;
        let exact = elapsed * f64::from(self.sample_rate) + self.carry;
        let count = exact.floor() as usize;
        self.carry = exact - count as f64;

        let mut out = Vec::with_capacity(count);
        if let Ok(mut shared) = self.shared.lock() {
            let have = shared.from_peer.len().min(count);
            out.extend(shared.from_peer.drain(..have));
        }
        out.resize(count, 0.0);
        if self.noise_sigma > 0.0 {
            for sample in &mut out {
                *sample += self.noise.gaussian() * self.noise_sigma;
            }
        }
        out
    }

    fn playback(&mut self, samples: &[f32]) {
        self.played += samples.len() as u64;
        let _ = self.to_peer.send(samples.to_vec());
    }

    fn queued(&self) -> usize {
        // the clock consumes what was played at the sample rate, and no faster than it
        // was played: an idle stretch does not build up a debt
        let (since, consumed) = self.consumed.get();
        let now = Instant::now();
        let due = (now.duration_since(since).as_secs_f64() * f64::from(self.sample_rate)) as u64;
        let consumed = (consumed + due).min(self.played);
        self.consumed.set((now, consumed));
        (self.played - consumed) as usize
    }

    fn dropped(&self) -> usize {
        0
    }
}

/// Wait for the other end: accept on the listener, or connect with retries.
fn find_peer(peer: &Peer, listener: Option<TcpListener>) -> Option<TcpStream> {
    match (peer, listener) {
        (Peer::Listen(_), Some(listener)) => listener.accept().ok().map(|(s, _)| s),
        (Peer::Connect(address), _) => {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                match TcpStream::connect(address) {
                    Ok(stream) => return Some(stream),
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    Err(_) => return None,
                }
            }
        }
        (Peer::Listen(_), None) => None,
    }
}

/// The noise level for an SNR referenced to 3 kHz, at the audio rate.
///
/// White noise of variance σ² at a rate `fs` has one-sided density σ²/(fs/2); in 3 kHz of
/// it there is σ² · 3000/(fs/2). With `fs` = 48 kHz that is σ²/8, so σ² = 8·S/SNR.
fn noise_sigma(snr_db: f64, signal_rms: f64) -> f64 {
    let snr = 10f64.powf(snr_db / 10.0);
    signal_rms * (8.0 / snr).sqrt()
}

/// Gaussian noise from a small deterministic generator: a failure is reproducible.
struct Noise {
    state: u64,
}

impl Noise {
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    fn uniform(&mut self) -> f64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        (self.state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn gaussian(&mut self) -> f32 {
        // Box–Muller, one of the pair
        let u1 = self.uniform().max(1e-12);
        let u2 = self.uniform();
        ((-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_level_follows_the_project_convention() {
        // 0.25 RMS signal at 20 dB: noise variance 8 · 0.0625 / 100 = 0.005 → σ = 0.0707
        let sigma = noise_sigma(20.0, 0.25);
        assert!((sigma - 0.070_71).abs() < 1e-4, "{sigma}");
        // and the generator has the variance it claims
        let mut noise = Noise::new(42);
        let n = 200_000;
        let mut sum = 0.0f64;
        let mut sum2 = 0.0f64;
        for _ in 0..n {
            let x = f64::from(noise.gaussian());
            sum += x;
            sum2 += x * x;
        }
        let mean = sum / f64::from(n);
        let var = sum2 / f64::from(n) - mean * mean;
        assert!(mean.abs() < 0.01, "{mean}");
        assert!((var - 1.0).abs() < 0.02, "{var}");
    }

    #[test]
    fn two_ends_carry_audio_both_ways_paced_by_the_clock() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr").to_string();
        drop(listener);
        let config = |peer: Peer| SimConfig {
            peer,
            snr_db: 60.0,
            signal_rms: 0.25,
            sample_rate: 48_000,
        };
        let mut a = SimLink::open(&config(Peer::Listen(address.clone()))).expect("listen");
        let mut b = SimLink::open(&config(Peer::Connect(address))).expect("connect");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(a.connected() && b.connected()) {
            assert!(Instant::now() < deadline, "the two ends never met");
            std::thread::sleep(Duration::from_millis(20));
        }

        // a plays a tone; b captures it at the wall clock's pace
        let tone: Vec<f32> = (0..9_600).map(|i| 0.25 * (i as f32 * 0.2).sin()).collect();
        a.playback(&tone);
        assert!(
            a.queued() > 0,
            "the backlog should be visible until the clock eats it"
        );
        std::thread::sleep(Duration::from_millis(250));
        let mut heard = Vec::new();
        for _ in 0..5 {
            heard.extend(b.capture());
            std::thread::sleep(Duration::from_millis(25));
        }
        // about 0.35 s of audio at 48 kHz, give or take scheduling
        assert!(
            heard.len() > 12_000 && heard.len() < 24_000,
            "{} samples",
            heard.len()
        );
        let peak = heard.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.2, "the tone did not arrive: peak {peak}");
        // the silence after it is noise at −60 dB, not more
        let tail = &heard[heard.len() - 1_000..];
        let rms = (tail.iter().map(|x| x * x).sum::<f32>() / 1_000.0).sqrt();
        assert!(rms < 0.01, "the padding is not quiet: {rms}");

        // and the other way
        b.playback(&tone);
        std::thread::sleep(Duration::from_millis(100));
        let back = a.capture();
        assert!(
            back.iter().any(|x| x.abs() > 0.2),
            "nothing came back the other way"
        );
        assert!(!a.lost() && !b.lost());
    }
}
