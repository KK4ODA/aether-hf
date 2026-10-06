//! The daemon's audio taken from a channel process, in lockstep (ADR-0042).
//!
//! A scenario harness runs two daemons through a simulated HF channel — fading, noise,
//! interference, static crashes, the radios' own switching — and wants a whole session to
//! take minutes, not the half hour it takes on the air. The station's clock is the audio it
//! has heard (`Station::now`), so nothing in a session depends on the wall clock: given its
//! audio a block at a time, as fast as it can take it, a daemon runs a session as fast as
//! its CPU allows. This backend is that: it connects to a channel server (`--channel`), and
//! every capture is one block the server sent, which the server sends only once both
//! stations have asked for their next one. What the station plays goes to the server as the
//! station hands it over, and the server plays each station's queue on the shared sample
//! clock, so the playback clock here is what the server says it consumed.
//!
//! The wire, little-endian:
//!
//! * station → server, once: `AEC1`, `u32` sample rate, `u16` length, the station's name;
//! * station → server: `2` `u32 n` then `n` `f32` samples to play; `3` ready for the next
//!   block (the last one has been processed); `4` drop what is queued and not yet played;
//! * server → station: `1`, `u64` the playback clock — samples the station's device has
//!   played through this block, silence included, as a sound card counts them —, `u64` the
//!   samples of its queue played so far, `u32 n`, then `n` `f32` samples heard.
//!
//! Nothing here is used on the air: `--channel` is for the harness, and a daemon on it keys
//! no transmitter.

use std::{
    io::{Read as _, Write as _},
    net::TcpStream,
    time::Duration,
};

use crate::audio::AudioIo;

/// How long a station waits for its next block before it decides the server has gone: long
/// enough for the other daemon's slowest pass, short enough that a dead harness ends.
const BLOCK_WAIT: Duration = Duration::from_secs(120);

/// Audio from a channel server, a block at a time.
pub struct ChannelLink {
    stream: Option<TcpStream>,
    /// Samples handed over to play, in all.
    handed: u64,
    /// Samples of them the server has played, as its last block said.
    consumed: u64,
    /// The playback clock as the last block said: silence counts, as on a sound card.
    clock: u64,
    /// Human-readable, for the log.
    pub description: String,
}

impl std::fmt::Debug for ChannelLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelLink")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

impl ChannelLink {
    /// Connect to the channel server at `address` as `name`, retrying for thirty seconds:
    /// the harness may start the daemons before the server listens.
    ///
    /// # Errors
    /// If the server cannot be reached, or the greeting cannot be sent.
    pub fn open(address: &str, name: &str, sample_rate: u32) -> std::io::Result<Self> {
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut stream = loop {
            match TcpStream::connect(address) {
                Ok(stream) => break stream,
                Err(error) if std::time::Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(BLOCK_WAIT))?;
        let name = name.as_bytes();
        let mut hello = b"AEC1".to_vec();
        hello.extend_from_slice(&sample_rate.to_le_bytes());
        hello.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
        hello.extend_from_slice(&name[..name.len().min(usize::from(u16::MAX))]);
        stream.write_all(&hello)?;
        Ok(Self {
            stream: Some(stream),
            handed: 0,
            consumed: 0,
            clock: 0,
            description: format!("channel server at {address}, in lockstep"),
        })
    }

    /// Whether the server is still there.
    #[must_use]
    pub const fn connected(&self) -> bool {
        self.stream.is_some()
    }

    fn send(&mut self, bytes: &[u8]) {
        if let Some(stream) = self.stream.as_mut()
            && stream.write_all(bytes).is_err()
        {
            self.stream = None;
        }
    }

    /// The next block: `(clock, played of the queue so far, samples heard)`.
    fn read_block(&mut self) -> std::io::Result<(u64, u64, Vec<f32>)> {
        let Some(stream) = self.stream.as_mut() else {
            return Err(std::io::ErrorKind::NotConnected.into());
        };
        let mut head = [0u8; 21];
        stream.read_exact(&mut head)?;
        if head[0] != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the channel server sent message {}", head[0]),
            ));
        }
        let clock = u64::from_le_bytes(head[1..9].try_into().unwrap_or_default());
        let consumed = u64::from_le_bytes(head[9..17].try_into().unwrap_or_default());
        let n = u32::from_le_bytes(head[17..21].try_into().unwrap_or_default()) as usize;
        let mut body = vec![0u8; 4 * n];
        stream.read_exact(&mut body)?;
        let samples = body
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok((clock, consumed, samples))
    }
}

impl AudioIo for ChannelLink {
    fn capture(&mut self) -> Vec<f32> {
        // the last block has been processed, and whatever it set going has been handed over:
        // the server may move the clock on
        self.send(&[3]);
        let Ok((clock, consumed, samples)) = self.read_block() else {
            // gone, or stuck: the station hears nothing from now on, and the run loop's idle
            // pacing takes over
            self.stream = None;
            return Vec::new();
        };
        self.clock = clock;
        self.consumed = consumed.min(self.handed);
        samples
    }

    fn playback(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(5 + 4 * samples.len());
        bytes.push(2);
        bytes.extend_from_slice(&u32::try_from(samples.len()).unwrap_or(0).to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        self.send(&bytes);
        self.handed += samples.len() as u64;
    }

    fn queued(&self) -> usize {
        usize::try_from(self.handed - self.consumed).unwrap_or(usize::MAX)
    }

    fn dropped(&self) -> usize {
        0
    }

    fn played(&self) -> u64 {
        self.clock
    }

    fn drains_at(&self) -> u64 {
        // the server plays a queue without a break from the next block on: it runs out what is
        // queued after the clock as it stands
        self.clock + self.handed - self.consumed
    }

    fn set_playing(&mut self, _playing: bool) {}

    fn starved(&self) -> usize {
        0
    }

    fn clear(&mut self) {
        self.send(&[4]);
        self.handed = self.consumed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A server that plays the station's queue back to it a block later, as a mirror.
    fn mirror(listener: &TcpListener, blocks: usize, block: usize) -> Vec<f32> {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut hello = [0u8; 10];
        stream.read_exact(&mut hello).expect("hello");
        assert_eq!(&hello[..4], b"AEC1");
        let name_len = u16::from_le_bytes([hello[8], hello[9]]) as usize;
        let mut name = vec![0u8; name_len];
        stream.read_exact(&mut name).expect("name");
        assert_eq!(name, b"W4ODA");
        let mut queue: std::collections::VecDeque<f32> = std::collections::VecDeque::new();
        let mut consumed = 0u64;
        let mut clock = 0u64;
        let mut heard_back = Vec::new();
        let mut last = vec![0.0f32; block];
        for _ in 0..blocks {
            // read until ready
            loop {
                let mut kind = [0u8; 1];
                stream.read_exact(&mut kind).expect("message");
                match kind[0] {
                    2 => {
                        let mut n = [0u8; 4];
                        stream.read_exact(&mut n).expect("n");
                        let mut body = vec![0u8; 4 * u32::from_le_bytes(n) as usize];
                        stream.read_exact(&mut body).expect("body");
                        queue.extend(
                            body.chunks_exact(4)
                                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                        );
                    }
                    3 => break,
                    4 => queue.clear(),
                    other => panic!("message {other}"),
                }
            }
            let mut message = vec![1u8];
            message.extend_from_slice(&clock.to_le_bytes());
            message.extend_from_slice(&consumed.to_le_bytes());
            message.extend_from_slice(&(block as u32).to_le_bytes());
            for &sample in &last {
                message.extend_from_slice(&sample.to_le_bytes());
            }
            heard_back.extend_from_slice(&last);
            stream.write_all(&message).expect("block");
            last = (0..block)
                .map(|_| queue.pop_front().inspect(|_| consumed += 1).unwrap_or(0.0))
                .collect();
            clock += block as u64;
        }
        heard_back
    }

    #[test]
    fn the_clock_is_the_servers_and_a_block_comes_only_when_asked_for() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr").to_string();
        let server = std::thread::spawn(move || mirror(&listener, 6, 100));
        let mut link = ChannelLink::open(&address, "W4ODA", 48_000).expect("open");
        assert!(link.capture().iter().all(|&s| s == 0.0), "silence first");
        let burst: Vec<f32> = (1..=150).map(|i| i as f32).collect();
        link.playback(&burst);
        assert_eq!(link.queued(), 150);
        assert_eq!(link.drains_at(), 150);
        let mut heard = Vec::new();
        for _ in 0..5 {
            heard.extend(link.capture());
        }
        // the server played it on its clock, silence and all: four blocks on, all of it gone
        assert_eq!(link.played(), 500);
        assert_eq!(link.queued(), 0);
        assert_eq!(link.drains_at(), 500);
        let echoed: Vec<f32> = heard.into_iter().filter(|&s| s != 0.0).collect();
        assert_eq!(echoed, burst, "played in order, without a break");
        drop(link);
        let _ = server.join();
    }
}
