//! A connection read a while at a time, without a read timeout on its socket.
//!
//! Each server here serves a connection from one thread that waits a short while for input and
//! then does its other work: the control API's WebSocket pushes events, the host program's
//! command port says what the station did, its data port moves payload to the program, the
//! KISS port checks whether the daemon is stopping. That wait was a read timeout on the socket
//! (`SO_RCVTIMEO`), and on Windows it is not safe. Rust's sockets there are overlapped, and a
//! read whose time runs out can fail with `ERROR_IO_PENDING` — os error 997, "Overlapped I/O
//! operation is in progress" — rather than as a timeout, which every loop here took for a broken
//! connection. Microsoft's documentation of `SO_RCVTIMEO` says a socket whose receive timed out
//! is in an indeterminate state and may lose data, so reading that error as a timeout would be
//! no fix. The gateway KK4ODA-1 lost five control connections to it on 2026-11-12, and `RadioPost`
//! Gateway's own adapter, which reads the same way, lost four. A program that loses its
//! connection in a session ends the session (ADR-0107).
//!
//! So a thread of the connection's own reads the socket with plain blocking reads, which no
//! platform times out, and hands what it reads over a channel. [`PolledStream`] reads from that
//! channel as the socket read with its timeout: what has come in at once, else
//! [`std::io::ErrorKind::TimedOut`] after the wait, `Ok(0)` once the other end has closed. It
//! writes straight to the socket, as before. Dropped, it shuts the socket down, which ends the
//! reading thread's read.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::time::Duration;

/// The most the reading thread takes off the socket at a time.
const CHUNK: usize = 8192;
/// Chunks read ahead before the reading thread waits for the serving one: past that the bytes
/// stay in the socket's own buffer, and a client that sends faster than it is served is slowed
/// by TCP, as it was.
const READ_AHEAD: usize = 16;

/// What the reading thread hands over.
enum Input {
    Bytes(Vec<u8>),
    /// The other end closed the connection.
    Closed,
    Failed(io::Error),
}

/// How the connection ended, for every read after the first that said so.
enum Ended {
    Closed,
    Failed(io::ErrorKind, String),
}

/// A TCP connection whose reads wait at most a set time, read by a thread of its own.
pub struct PolledStream {
    /// The socket, for writes and for the shutdown that ends the reading thread.
    stream: TcpStream,
    input: Receiver<Input>,
    /// Bytes handed over and not read yet, from `at`.
    held: Vec<u8>,
    at: usize,
    wait: Duration,
    ended: Option<Ended>,
}

impl PolledStream {
    /// Read `stream` from a thread named `name`, each read waiting at most `wait`.
    ///
    /// # Errors
    /// If the socket cannot be cloned, its read timeout cleared, or the thread started.
    pub fn new(stream: &TcpStream, wait: Duration, name: &str) -> io::Result<Self> {
        let mut reading = stream.try_clone()?;
        // the reading thread waits for as long as the connection is quiet; a timeout left on
        // the socket (it is the clones' too) would bring back what this is here to remove
        reading.set_read_timeout(None)?;
        let writing = stream.try_clone()?;
        let (hand, input) = mpsc::sync_channel(READ_AHEAD);
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || read_into(&mut reading, &hand))?;
        Ok(Self {
            stream: writing,
            input,
            held: Vec::new(),
            at: 0,
            wait,
            ended: None,
        })
    }
}

/// The reading thread: the socket's bytes into the channel until the connection ends, or the
/// serving side has gone.
fn read_into(stream: &mut TcpStream, hand: &SyncSender<Input>) {
    let mut buffer = vec![0u8; CHUNK];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => {
                let _ = hand.send(Input::Closed);
                return;
            }
            Ok(count) => {
                if hand.send(Input::Bytes(buffer[..count].to_vec())).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                let _ = hand.send(Input::Failed(error));
                return;
            }
        }
    }
}

impl Read for PolledStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.at >= self.held.len() {
            match &self.ended {
                Some(Ended::Closed) => return Ok(0),
                Some(Ended::Failed(kind, said)) => return Err(io::Error::new(*kind, said.clone())),
                None => {}
            }
            match self.input.recv_timeout(self.wait) {
                Ok(Input::Bytes(bytes)) => {
                    self.held = bytes;
                    self.at = 0;
                }
                Ok(Input::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    self.ended = Some(Ended::Closed);
                    return Ok(0);
                }
                Ok(Input::Failed(error)) => {
                    self.ended = Some(Ended::Failed(error.kind(), error.to_string()));
                    return Err(error);
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "nothing came in the wait",
                    ));
                }
            }
        }
        let count = (self.held.len() - self.at).min(buf.len());
        buf[..count].copy_from_slice(&self.held[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

impl Write for PolledStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

impl Drop for PolledStream {
    fn drop(&mut self) {
        // the connection is over: the shutdown ends the reading thread's read, and its next
        // hand-over finds nobody to take it
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    /// A connected pair: the server's end read through a [`PolledStream`], the client's plain.
    fn pair(wait: Duration) -> (PolledStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
        let (served, _) = listener.accept().expect("accept");
        let polled = PolledStream::new(&served, wait, "test-polled").expect("polled");
        (polled, client)
    }

    #[test]
    fn a_quiet_connection_waits_and_loses_nothing() {
        // the property the socket's read timeout could not give on Windows: a connection idle
        // for many waits is still there, and nothing sent after them is lost
        let (mut polled, mut client) = pair(Duration::from_millis(10));
        let mut buf = [0u8; 64];
        for _ in 0..20 {
            let started = Instant::now();
            let error = polled.read(&mut buf).expect_err("nothing was sent");
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "a read waited too long"
            );
        }
        let message: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
        client.write_all(&message).expect("send");
        let mut got = Vec::new();
        while got.len() < message.len() {
            match polled.read(&mut buf) {
                Ok(0) => panic!("the connection ended after {} bytes", got.len()),
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
                Err(error) => panic!("a read failed: {error}"),
            }
        }
        assert_eq!(got, message, "the bytes arrived as sent");
    }

    #[test]
    fn writes_reach_the_other_end_and_a_close_reads_as_the_end() {
        let (mut polled, mut client) = pair(Duration::from_millis(10));
        polled.write_all(b"OK\r").expect("write");
        let mut said = [0u8; 3];
        client.read_exact(&mut said).expect("read");
        assert_eq!(&said, b"OK\r");
        client.write_all(b"BYE").expect("send");
        drop(client);
        let mut buf = [0u8; 8];
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match polled.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
                Err(error) => panic!("a read failed: {error}"),
            }
            assert!(Instant::now() < deadline, "the close was never read");
        }
        assert_eq!(got, b"BYE");
        // and the end again, at once
        assert_eq!(polled.read(&mut buf).expect("ended"), 0);
    }

    #[test]
    fn dropped_it_closes_the_connection() {
        let (polled, mut client) = pair(Duration::from_millis(10));
        drop(polled);
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        let mut buf = [0u8; 8];
        // the shutdown reads as the end at the other station; never as a wait run out
        match client.read(&mut buf) {
            Ok(n) => assert_eq!(n, 0, "nothing was sent"),
            Err(error) => assert!(
                !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ),
                "the connection was not closed: {error}"
            ),
        }
    }
}
