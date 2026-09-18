//! Keying the transmitter, and the watchdog that will not let it stay keyed.
//!
//! A [`Ptt`] is anything that can put a radio into transmit and take it out again. The
//! backends differ only in how they say it: a serial line's RTS or DTR, the radio's own CAT
//! command, a CM108-class sound-card interface's GPIO pin, a rig-control daemon over TCP,
//! or nothing at all when the operator is using VOX.
//!
//! # The watchdog is not optional
//!
//! Software that keys a transmitter can fail while it is keyed — a panic, a deadlock, a
//! dropped USB device, an operating system that stops scheduling the audio thread. What is
//! on the air then is a carrier, on a shared band, for as long as nobody notices. Amateur
//! transmitters are also not rated for continuous duty at full power, so a stuck key can
//! destroy a finals stage as well as a band.
//!
//! [`PttWatchdog`] therefore wraps every backend and enforces a maximum key time in one
//! place, and a trip **latches**: after it fires, keying is refused until something calls
//! [`unkey`](PttWatchdog::unkey). A runaway loop that keeps asking to transmit is exactly the
//! failure this exists to contain, so it must not be able to re-key by asking again. Nothing
//! here depends on a backend behaving; the latch is enforced on this side of the trait.

/// Anything that went wrong keying a radio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PttError {
    /// The backend refused or the device is gone.
    Backend(String),
    /// The watchdog has tripped and has not been cleared.
    WatchdogTripped,
}

impl core::fmt::Display for PttError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Backend(message) => write!(f, "keying: {message}"),
            Self::WatchdogTripped => {
                write!(
                    f,
                    "the key-time watchdog tripped and the transmitter was released; it stays \
                     released until the software asks again from a clean state"
                )
            }
        }
    }
}

impl core::error::Error for PttError {}

/// Something that can key a transmitter.
pub trait Ptt: Send {
    /// Put the radio into transmit.
    ///
    /// # Errors
    /// If the backend refuses or the device is gone.
    fn key(&mut self) -> Result<(), PttError>;

    /// Take the radio out of transmit.
    ///
    /// # Errors
    /// If the backend refuses or the device is gone.
    fn unkey(&mut self) -> Result<(), PttError>;

    /// What this backend is, for a log line or a status display.
    fn describe(&self) -> String;

    /// The radio's frequency in hertz, when this backend has a way to ask.
    ///
    /// A recording that says what frequency it was made on is worth far more to the field
    /// log than one that does not, and an operator who left the station running all day
    /// is not there to write it down. A keying line cannot ask; `rigctld` can.
    fn frequency_hz(&mut self) -> Option<u64> {
        None
    }

    /// Why this interface cannot key, when it cannot. `None` for one that works.
    ///
    /// A station holding a faulted interface still runs and still receives; it simply
    /// cannot transmit, and says why rather than pretending the radio is there.
    fn fault(&self) -> Option<String> {
        None
    }

    /// Whether `set_frequency_hz` has a way to ask: CAT and `rigctld` do.
    fn can_tune(&self) -> bool {
        false
    }

    /// Tune the radio to `hz`, when this backend has a way to ask.
    ///
    /// # Errors
    /// If it has none — a keying line, a GPIO pin, nothing at all — or the radio refused.
    fn set_frequency_hz(&mut self, _hz: u64) -> Result<(), PttError> {
        Err(PttError::Backend(
            "the keying interface has no way to ask; tuning takes CAT on the radio's own port, \
             or rigctld"
                .into(),
        ))
    }
}

impl<P: Ptt + ?Sized> Ptt for Box<P> {
    fn key(&mut self) -> Result<(), PttError> {
        (**self).key()
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        (**self).unkey()
    }

    fn frequency_hz(&mut self) -> Option<u64> {
        (**self).frequency_hz()
    }

    fn fault(&self) -> Option<String> {
        (**self).fault()
    }

    fn can_tune(&self) -> bool {
        (**self).can_tune()
    }

    fn set_frequency_hz(&mut self, hz: u64) -> Result<(), PttError> {
        (**self).set_frequency_hz(hz)
    }

    fn describe(&self) -> String {
        (**self).describe()
    }
}

/// No keying at all: for VOX, for a receive-only station, and for tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullPtt {
    /// Whether `key` was the last thing called.
    pub keyed: bool,
}

impl Ptt for NullPtt {
    fn key(&mut self) -> Result<(), PttError> {
        self.keyed = true;
        Ok(())
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        self.keyed = false;
        Ok(())
    }

    fn describe(&self) -> String {
        "none (vox or receive only)".to_owned()
    }
}

/// A keying interface that could not be opened, kept so the daemon can start anyway.
///
/// Refusing to start over a serial port that is not there traps the operator: the Setup
/// screen that names the port is served by this very daemon, so the one comfortable way
/// to correct the setting is behind the thing the setting stops. The daemon therefore
/// starts with this instead, and the panel comes up with the reason on it.
///
/// Nothing reaches the air. `key` fails with the original reason, and the station copies
/// no audio to the sound card until keying has succeeded, so a faulted station is deaf
/// to nothing and mute to everything.
#[derive(Debug, Clone)]
pub struct BrokenPtt {
    reason: String,
}

impl BrokenPtt {
    /// A faulted interface that will explain itself every time it is asked.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Ptt for BrokenPtt {
    fn key(&mut self) -> Result<(), PttError> {
        Err(PttError::Backend(self.reason.clone()))
    }

    /// Releasing something that was never keyed is not a failure, and must not be one:
    /// the run loop unkeys on paths where an error would be noise about nothing.
    fn unkey(&mut self) -> Result<(), PttError> {
        Ok(())
    }

    fn describe(&self) -> String {
        format!("unavailable — {}", self.reason)
    }

    fn fault(&self) -> Option<String> {
        Some(self.reason.clone())
    }
}

/// Keying through `rigctld`, Hamlib's rig-control daemon, over TCP.
///
/// The protocol is a line at a time: `T 1` to key, `T 0` to release, and a reply of
/// `RPRT <code>` where zero means it worked. Using the daemon rather than linking Hamlib
/// keeps this crate free of a C dependency and lets the operator point at a rig control
/// program they already have running and already trust with their radio.
#[derive(Debug)]
pub struct RigctldPtt {
    address: String,
    timeout: std::time::Duration,
    stream: Option<std::net::TcpStream>,
}

impl RigctldPtt {
    /// Point at a running `rigctld`, conventionally `127.0.0.1:4532`.
    #[must_use]
    pub fn new(address: &str, timeout: std::time::Duration) -> Self {
        Self {
            address: address.to_owned(),
            timeout,
            stream: None,
        }
    }

    fn connect(&mut self) -> Result<&mut std::net::TcpStream, PttError> {
        if self.stream.is_none() {
            let address: std::net::SocketAddr = self
                .address
                .parse()
                .map_err(|e| PttError::Backend(format!("bad address {}: {e}", self.address)))?;
            let stream =
                std::net::TcpStream::connect_timeout(&address, self.timeout).map_err(|e| {
                    PttError::Backend(format!(
                        "cannot reach rigctld at {} ({e}). Is it running, and is [ptt] address \
                         the address it listens on?",
                        self.address
                    ))
                })?;
            stream
                .set_read_timeout(Some(self.timeout))
                .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
                .map_err(|e| PttError::Backend(format!("timeouts: {e}")))?;
            self.stream = Some(stream);
        }
        Ok(self.stream.as_mut().expect("just connected"))
    }

    /// Ask a question and take the one-line answer, or nothing.
    fn query(&mut self, line: &str) -> Option<String> {
        use std::io::{BufRead, BufReader, Write};

        let result = (|| -> Result<String, PttError> {
            let stream = self.connect()?;
            stream
                .write_all(line.as_bytes())
                .map_err(|e| PttError::Backend(format!("write: {e}")))?;
            let mut reply = String::new();
            let mut reader = BufReader::new(
                stream
                    .try_clone()
                    .map_err(|e| PttError::Backend(format!("clone: {e}")))?,
            );
            reader
                .read_line(&mut reply)
                .map_err(|e| PttError::Backend(format!("read: {e}")))?;
            if reply.trim().starts_with("RPRT") {
                return Err(PttError::Backend(format!("rigctld said {}", reply.trim())));
            }
            Ok(reply)
        })();
        if result.is_err() {
            self.stream = None;
        }
        result.ok()
    }

    fn command(&mut self, line: &str) -> Result<(), PttError> {
        use std::io::{BufRead, BufReader, Write};

        let result = (|| -> Result<(), PttError> {
            let stream = self.connect()?;
            stream
                .write_all(line.as_bytes())
                .map_err(|e| PttError::Backend(format!("write: {e}")))?;
            let mut reply = String::new();
            let mut reader = BufReader::new(
                stream
                    .try_clone()
                    .map_err(|e| PttError::Backend(format!("clone: {e}")))?,
            );
            reader
                .read_line(&mut reply)
                .map_err(|e| PttError::Backend(format!("read: {e}")))?;
            // `RPRT 0` is success; anything else is the rig or the daemon saying no
            match reply.trim().strip_prefix("RPRT ") {
                Some("0") => Ok(()),
                Some(code) => Err(PttError::Backend(format!("rigctld returned {code}"))),
                None => Err(PttError::Backend(format!(
                    "unexpected reply {:?}",
                    reply.trim()
                ))),
            }
        })();
        if result.is_err() {
            self.stream = None; // drop it so the next attempt reconnects
        }
        result
    }
}

impl Ptt for RigctldPtt {
    fn key(&mut self) -> Result<(), PttError> {
        self.command("T 1\n")
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        self.command("T 0\n")
    }

    fn describe(&self) -> String {
        format!("rigctld at {}", self.address)
    }

    fn frequency_hz(&mut self) -> Option<u64> {
        // `f` is get_freq: one line holding the frequency in hertz, or `RPRT <n>` when the
        // rig cannot say. Best effort: a recording without a frequency is still a recording.
        self.query("f\n").and_then(|line| line.trim().parse().ok())
    }

    fn can_tune(&self) -> bool {
        true
    }

    fn set_frequency_hz(&mut self, hz: u64) -> Result<(), PttError> {
        // `F` is set_freq; the daemon answers `RPRT 0` when the rig took it
        self.command(&format!("F {hz}\n"))
    }
}

/// What a watchdog poll found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogState {
    /// Not transmitting.
    Idle,
    /// Transmitting, within the limit.
    Keyed,
    /// The limit was exceeded just now and the key was released.
    Tripped,
}

/// A maximum key time, enforced over any backend.
///
/// A trip latches: keying is refused until [`unkey`](Self::unkey) is called. See the module
/// documentation for why that is the safe behaviour rather than an inconvenient one.
#[derive(Debug)]
pub struct PttWatchdog<P: Ptt> {
    inner: P,
    /// Longest a single transmission may last, in seconds.
    pub max_key_s: f64,
    keyed_since: Option<f64>,
    tripped: bool,
    /// How many times the watchdog has fired since the daemon started.
    pub trips: usize,
}

impl<P: Ptt> PttWatchdog<P> {
    /// Wrap a backend with a maximum key time.
    ///
    /// # Panics
    /// If `max_key_s` is not positive. A watchdog that can never fire is not a watchdog, and
    /// a configuration that asks for one is a mistake worth refusing loudly.
    #[must_use]
    pub fn new(inner: P, max_key_s: f64) -> Self {
        assert!(max_key_s > 0.0, "the maximum key time must be positive");
        Self {
            inner,
            max_key_s,
            keyed_since: None,
            tripped: false,
            trips: 0,
        }
    }

    /// Whether the transmitter is keyed.
    #[must_use]
    pub fn is_keyed(&self) -> bool {
        self.keyed_since.is_some()
    }

    /// Whether the watchdog has fired and not yet been cleared.
    #[must_use]
    pub fn is_tripped(&self) -> bool {
        self.tripped
    }

    /// How long the current transmission has been running.
    #[must_use]
    pub fn keyed_for(&self, now: f64) -> f64 {
        self.keyed_since.map_or(0.0, |since| (now - since).max(0.0))
    }

    /// What the backend is.
    pub fn describe(&self) -> String {
        self.inner.describe()
    }

    /// Why the backend cannot key, when it cannot.
    #[must_use]
    pub fn fault(&self) -> Option<String> {
        self.inner.fault()
    }

    /// Whether the backend can tune the radio.
    pub fn can_tune(&self) -> bool {
        self.inner.can_tune()
    }

    /// Key the transmitter.
    ///
    /// Keying while already keyed does not restart the clock — a burst that is transmitted as
    /// several calls is still one transmission as far as the radio is concerned, and letting
    /// each call reset the limit would defeat the whole thing.
    ///
    /// # Errors
    /// If the watchdog has tripped and not been cleared, or the backend refuses.
    pub fn key(&mut self, now: f64) -> Result<(), PttError> {
        if self.tripped {
            return Err(PttError::WatchdogTripped);
        }
        if self.keyed_since.is_some() {
            return Ok(());
        }
        self.inner.key()?;
        self.keyed_since = Some(now);
        Ok(())
    }

    /// Release the transmitter, and clear a trip.
    ///
    /// # Errors
    /// If the backend refuses. The watchdog state is cleared either way: a backend that
    /// cannot be told to stop is a reason to report loudly, not a reason to stay latched into
    /// believing the radio is still ours to key.
    pub fn unkey(&mut self, _now: f64) -> Result<(), PttError> {
        let result = self.inner.unkey();
        self.keyed_since = None;
        self.tripped = false;
        result
    }

    /// Check the clock; release the key if the limit has been passed.
    ///
    /// The run loop calls this every time round, whatever else it is doing.
    ///
    /// # Errors
    /// If the key had to be released and the backend refused.
    pub fn poll(&mut self, now: f64) -> Result<WatchdogState, PttError> {
        let Some(since) = self.keyed_since else {
            return Ok(WatchdogState::Idle);
        };
        if now - since < self.max_key_s {
            return Ok(WatchdogState::Keyed);
        }
        let result = self.inner.unkey();
        self.keyed_since = None;
        self.tripped = true;
        self.trips += 1;
        result.map(|()| WatchdogState::Tripped)
    }

    /// The backend, for a caller that needs to look at it.
    pub fn inner(&self) -> &P {
        &self.inner
    }

    /// The backend, for a caller that needs to ask it something.
    pub fn inner_mut(&mut self) -> &mut P {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cat_commands_are_the_published_ones() {
        // TX1 and only TX1: TX2 is what the radio *reports* when its mic or data jack
        // keyed it, and as a command an FTDX10 ignores it (Hamlib and flrig both send TX1)
        let yaesu = CatProtocol::Yaesu;
        assert_eq!(yaesu.keying(true), b"TX1;");
        assert_eq!(yaesu.keying(false), b"TX0;");
        assert_ne!(yaesu.keying(true), b"TX2;", "a status code is not a command");
        assert_eq!(CatProtocol::Kenwood.keying(true), b"TX;");
        assert_eq!(CatProtocol::Kenwood.keying(false), b"RX;");
        let icom = CatProtocol::Icom { address: 0x94 };
        assert_eq!(
            icom.keying(true),
            [0xFE, 0xFE, 0x94, 0xE0, 0x1C, 0x00, 0x01, 0xFD]
        );
        assert_eq!(
            icom.keying(false),
            [0xFE, 0xFE, 0x94, 0xE0, 0x1C, 0x00, 0x00, 0xFD]
        );
        assert_eq!(icom.frequency_query(), [0xFE, 0xFE, 0x94, 0xE0, 0x03, 0xFD]);
        assert!(!icom.keying_accepted(&[0xFE, 0xFE, 0xE0, 0x94, 0xFA, 0xFD]));
        assert!(icom.keying_accepted(&[0xFE, 0xFE, 0xE0, 0x94, 0xFB, 0xFD]));
    }

    #[test]
    fn the_tuning_command_is_the_published_one_in_each_dialect() {
        let yaesu = CatProtocol::Yaesu;
        assert_eq!(yaesu.frequency_set(14_107_000), b"FA014107000;");
        assert_eq!(
            CatProtocol::Kenwood.frequency_set(7_101_000),
            b"FA00007101000;"
        );
        let icom = CatProtocol::Icom { address: 0x94 };
        let command = icom.frequency_set(14_107_000);
        assert_eq!(
            command,
            [
                0xFE, 0xFE, 0x94, 0xE0, 0x05, 0x00, 0x70, 0x10, 0x14, 0x00, 0xFD
            ]
        );
        // the same five bytes read back as the frequency: one layout both ways
        let mut answer = vec![0xFE, 0xFE, 0xE0, 0x94, 0x03];
        answer.extend_from_slice(&command[5..10]);
        answer.push(0xFD);
        assert_eq!(icom.parse_frequency(&answer), Some(14_107_000));
    }

    #[test]
    fn a_frequency_answer_is_read_in_each_dialect() {
        let yaesu = CatProtocol::Yaesu;
        assert_eq!(yaesu.parse_frequency(b"FA014107000;"), Some(14_107_000));
        assert_eq!(
            CatProtocol::Kenwood.parse_frequency(b"FA00007101000;"),
            Some(7_101_000)
        );
        assert_eq!(yaesu.parse_frequency(b"?;"), None);
        // 14.107000 MHz as CI-V BCD, least significant byte first: 00 70 10 14 00
        let icom = CatProtocol::Icom { address: 0x94 };
        assert_eq!(
            icom.parse_frequency(&[
                0xFE, 0xFE, 0xE0, 0x94, 0x03, 0x00, 0x70, 0x10, 0x14, 0x00, 0xFD
            ]),
            Some(14_107_000)
        );
        assert_eq!(
            icom.parse_frequency(&[0xFE, 0xFE, 0xE0, 0x94, 0xFA, 0xFD]),
            None
        );
    }

    /// A backend that counts what it was asked to do, and can be told to refuse.
    #[derive(Debug, Default)]
    struct FakePtt {
        keyed: bool,
        keys: usize,
        unkeys: usize,
        refuse: bool,
    }

    impl Ptt for FakePtt {
        fn key(&mut self) -> Result<(), PttError> {
            if self.refuse {
                return Err(PttError::Backend("refused".into()));
            }
            self.keys += 1;
            self.keyed = true;
            Ok(())
        }

        fn unkey(&mut self) -> Result<(), PttError> {
            if self.refuse {
                return Err(PttError::Backend("refused".into()));
            }
            self.unkeys += 1;
            self.keyed = false;
            Ok(())
        }

        fn describe(&self) -> String {
            "fake".to_owned()
        }
    }

    #[test]
    fn a_normal_transmission_is_left_alone() {
        let mut ptt = PttWatchdog::new(FakePtt::default(), 30.0);
        ptt.key(0.0).expect("key");
        assert_eq!(ptt.poll(10.0), Ok(WatchdogState::Keyed));
        ptt.unkey(12.0).expect("unkey");
        assert_eq!(ptt.poll(60.0), Ok(WatchdogState::Idle));
        assert!(!ptt.inner().keyed);
        assert_eq!(ptt.trips, 0);
    }

    #[test]
    fn the_key_is_released_when_the_limit_passes() {
        let mut ptt = PttWatchdog::new(FakePtt::default(), 30.0);
        ptt.key(0.0).expect("key");
        assert_eq!(ptt.poll(29.9), Ok(WatchdogState::Keyed));
        assert_eq!(ptt.poll(30.1), Ok(WatchdogState::Tripped));
        assert!(!ptt.inner().keyed, "the radio is still transmitting");
        assert!(!ptt.is_keyed());
        assert_eq!(ptt.trips, 1);
    }

    #[test]
    fn a_trip_latches_until_the_key_is_released() {
        // the failure this exists to contain is a loop that keeps asking to transmit, so
        // asking again must not be enough to get back on the air
        let mut ptt = PttWatchdog::new(FakePtt::default(), 10.0);
        ptt.key(0.0).expect("key");
        assert_eq!(ptt.poll(11.0), Ok(WatchdogState::Tripped));
        assert_eq!(ptt.key(11.1), Err(PttError::WatchdogTripped));
        assert_eq!(ptt.key(20.0), Err(PttError::WatchdogTripped));
        assert!(!ptt.inner().keyed);
        assert_eq!(ptt.inner().keys, 1, "it keyed again behind the latch");

        ptt.unkey(21.0).expect("release");
        ptt.key(21.1).expect("and now it may transmit");
        assert!(ptt.inner().keyed);
    }

    #[test]
    fn keying_again_does_not_restart_the_clock() {
        // a burst sent as several calls is one transmission to the radio; if each call reset
        // the limit, a steady stream of them would never trip it
        let mut ptt = PttWatchdog::new(FakePtt::default(), 10.0);
        ptt.key(0.0).expect("key");
        for t in [2.0, 4.0, 6.0, 8.0] {
            ptt.key(t).expect("still keyed");
        }
        assert_eq!(ptt.inner().keys, 1, "it keyed the radio more than once");
        assert_eq!(ptt.poll(10.5), Ok(WatchdogState::Tripped));
    }

    #[test]
    fn a_backend_that_will_not_release_still_clears_the_state() {
        // if the device is gone there is nothing more this side can do about the carrier;
        // what it must not do is keep believing the radio is still ours to key
        let mut ptt = PttWatchdog::new(FakePtt::default(), 5.0);
        ptt.key(0.0).expect("key");
        ptt.inner.refuse = true;
        let result = ptt.unkey(1.0);
        assert!(matches!(result, Err(PttError::Backend(_))));
        assert!(!ptt.is_keyed());
        assert!(!ptt.is_tripped());
    }

    #[test]
    fn a_failed_key_leaves_the_watchdog_idle() {
        let mut ptt = PttWatchdog::new(
            FakePtt {
                refuse: true,
                ..FakePtt::default()
            },
            5.0,
        );
        assert!(ptt.key(0.0).is_err());
        assert!(!ptt.is_keyed(), "a refused key must not start the clock");
        assert_eq!(ptt.poll(100.0), Ok(WatchdogState::Idle));
    }

    /// A stand-in for `rigctld`: one client, Hamlib's one-line answers, and a record of
    /// every line it was sent.
    fn fake_rigctld() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address").to_string();
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = received.clone();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut writer = stream.try_clone().expect("clone");
            let mut dial: u64 = 14_107_000;
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { break };
                seen.lock().expect("lock").push(line.clone());
                let answer = match line.as_str() {
                    "f" => format!("{dial}\n"),
                    other => {
                        if let Some(hz) = other.strip_prefix("F ") {
                            dial = hz.trim().parse().expect("hertz");
                        }
                        "RPRT 0\n".to_owned()
                    }
                };
                writer.write_all(answer.as_bytes()).expect("write");
            }
        });
        (address, received)
    }

    #[test]
    fn rigctld_is_asked_for_the_dial_and_told_a_new_one() {
        let (address, received) = fake_rigctld();
        let mut rig = RigctldPtt::new(&address, std::time::Duration::from_secs(2));
        assert!(rig.can_tune());
        assert_eq!(rig.frequency_hz(), Some(14_107_000));
        rig.set_frequency_hz(7_101_000).expect("tuned");
        assert_eq!(rig.frequency_hz(), Some(7_101_000));
        assert_eq!(
            *received.lock().expect("lock"),
            vec!["f".to_owned(), "F 7101000".to_owned(), "f".to_owned()]
        );
        let mut none = NullPtt::default();
        assert!(!none.can_tune());
        assert!(none.set_frequency_hz(7_101_000).is_err());
    }

    #[test]
    fn a_keying_interface_that_would_not_open_refuses_every_key_and_says_why() {
        let mut broken = BrokenPtt::new("cannot open the serial port COM9");
        // it explains itself rather than pretending the radio is there
        assert_eq!(
            broken.fault().as_deref(),
            Some("cannot open the serial port COM9")
        );
        assert!(broken.describe().contains("COM9"));
        // and it never keys: the station copies no audio to the card until keying succeeds,
        // so a station holding one cannot put a signal on the air
        let refused = broken.key().expect_err("it must not key");
        assert_eq!(
            refused,
            PttError::Backend("cannot open the serial port COM9".to_owned())
        );
        // releasing what was never keyed is not an error worth raising
        broken.unkey().expect("unkey is a no-op");
        // a working interface has no fault to report
        assert_eq!(NullPtt::default().fault(), None);
    }

    #[test]
    fn the_null_backend_keys_nothing() {
        let mut ptt = NullPtt::default();
        ptt.key().expect("key");
        assert!(ptt.keyed);
        ptt.unkey().expect("unkey");
        assert!(!ptt.keyed);
    }

    #[test]
    #[should_panic(expected = "the maximum key time must be positive")]
    fn a_watchdog_that_can_never_fire_is_refused() {
        let _ = PttWatchdog::new(NullPtt::default(), 0.0);
    }
}

/// Which control line on a serial port keys the radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SerialLine {
    /// Request To Send. What most home-made and commercial interfaces use.
    #[default]
    Rts,
    /// Data Terminal Ready. The other common choice.
    Dtr,
    /// Both together, for interfaces that wire them in parallel.
    Both,
}

/// A radio's command set, as bytes on the wire: the published CAT protocols.
///
/// Pure functions, so what goes down the port is tested without a port. The commands are
/// the manufacturers' own: Yaesu's ASCII CAT (`TX1;` keys, `TX0;` releases; `FA;` asks the
/// VFO-A frequency), Kenwood's (`TX;`, `RX;`, `FA;`), and Icom's CI-V frames
/// (`FE FE <rig> <controller> 1C 00 <01|00> FD` to key, `03` to ask the frequency, which
/// comes back as little-endian BCD).
///
/// On a Yaesu the keying command is `TX1;` and only that. The protocol's `TX2` is a
/// *status* the radio reports when something other than CAT keyed it — the microphone,
/// the data jack, a key — and sending it as a command does nothing: an FTDX10 tuned
/// happily over CAT and never keyed, until it was found that Hamlib's `newcat_set_ptt` and
/// flrig's `FTdx10::set_PTT_control` both send `TX1;` for every kind of keying and never
/// `TX2;`. Which input the radio transmits is its mode's business (DATA-U transmits the
/// DATA/USB input), not the keying command's, so there is nothing to choose here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatProtocol {
    /// Yaesu ASCII CAT.
    Yaesu,
    /// Kenwood's ASCII protocol, which Elecraft speaks too.
    Kenwood,
    /// Icom CI-V, addressed to one radio.
    Icom {
        /// The radio's CI-V address.
        address: u8,
    },
}

/// The controller's own CI-V address; `E0` is what every rig-control program uses.
const CIV_CONTROLLER: u8 = 0xE0;

impl CatProtocol {
    /// The bytes that put the radio into transmit, or take it out.
    #[must_use]
    pub fn keying(self, keyed: bool) -> Vec<u8> {
        match self {
            Self::Yaesu => {
                if keyed {
                    b"TX1;".to_vec()
                } else {
                    b"TX0;".to_vec()
                }
            }
            Self::Kenwood => {
                if keyed {
                    b"TX;".to_vec()
                } else {
                    b"RX;".to_vec()
                }
            }
            Self::Icom { address } => vec![
                0xFE,
                0xFE,
                address,
                CIV_CONTROLLER,
                0x1C,
                0x00,
                u8::from(keyed),
                0xFD,
            ],
        }
    }

    /// The bytes that ask for the operating frequency.
    #[must_use]
    pub fn frequency_query(self) -> Vec<u8> {
        match self {
            Self::Yaesu | Self::Kenwood => b"FA;".to_vec(),
            Self::Icom { address } => vec![0xFE, 0xFE, address, CIV_CONTROLLER, 0x03, 0xFD],
        }
    }

    /// The command that tunes VFO A to `hz`: nine digits on a Yaesu, eleven on a
    /// Kenwood, and CI-V command 05 with the frequency as five BCD bytes, least
    /// significant first, on an Icom — the same layout its frequency answer uses.
    #[must_use]
    pub fn frequency_set(self, hz: u64) -> Vec<u8> {
        match self {
            Self::Yaesu => format!("FA{hz:09};").into_bytes(),
            Self::Kenwood => format!("FA{hz:011};").into_bytes(),
            Self::Icom { address } => {
                let mut bytes = vec![0xFE, 0xFE, address, CIV_CONTROLLER, 0x05];
                let mut rest = hz;
                for _ in 0..5 {
                    let pair = u8::try_from(rest % 100).unwrap_or(0);
                    rest /= 100;
                    bytes.push(((pair / 10) << 4) | (pair % 10));
                }
                bytes.push(0xFD);
                bytes
            }
        }
    }

    /// The frequency in hertz from the radio's answer, if the answer is one.
    #[must_use]
    pub fn parse_frequency(self, reply: &[u8]) -> Option<u64> {
        match self {
            Self::Yaesu | Self::Kenwood => {
                // `FA014107000;` on a Yaesu, `FA00014107000;` on a Kenwood: the digits
                // between the command and the terminator are the frequency in hertz
                let text = std::str::from_utf8(reply).ok()?;
                let start = text.find("FA")? + 2;
                let digits: String = text[start..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if digits.is_empty() {
                    return None;
                }
                digits.parse().ok()
            }
            Self::Icom { address } => {
                // FE FE E0 <rig> 03 <bcd × 5, least significant byte first> FD
                let start = reply
                    .windows(5)
                    .position(|w| w == [0xFE, 0xFE, CIV_CONTROLLER, address, 0x03])?;
                let data = &reply[start + 5..];
                let end = data.iter().position(|&b| b == 0xFD)?;
                let bcd = &data[..end];
                if bcd.len() < 4 {
                    return None;
                }
                let mut hz: u64 = 0;
                for &byte in bcd.iter().rev() {
                    let (high, low) = (u64::from(byte >> 4), u64::from(byte & 0x0F));
                    if high > 9 || low > 9 {
                        return None;
                    }
                    hz = hz * 100 + high * 10 + low;
                }
                Some(hz)
            }
        }
    }

    /// Whether the radio answers a keying command at all, so a missing answer means
    /// something. Yaesu and Kenwood radios say nothing on success; an Icom always answers
    /// `FB` (done) or `FA` (refused).
    #[must_use]
    pub fn acknowledges_keying(self) -> bool {
        matches!(self, Self::Icom { .. })
    }

    /// Whether a CI-V answer says the command was carried out.
    #[must_use]
    pub fn keying_accepted(self, reply: &[u8]) -> bool {
        match self {
            Self::Icom { address } => reply
                .windows(6)
                .any(|w| w == [0xFE, 0xFE, CIV_CONTROLLER, address, 0xFB, 0xFD]),
            _ => true,
        }
    }
}

/// Keying a radio with its own commands over its CAT port.
pub struct CatPtt {
    path: String,
    protocol: CatProtocol,
    port: Box<dyn serialport::SerialPort>,
}

impl std::fmt::Debug for CatPtt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatPtt")
            .field("path", &self.path)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}

impl CatPtt {
    /// Open the port at the radio's CAT rate and make sure the radio is receiving.
    ///
    /// # Errors
    /// If the port cannot be opened, or the radio refuses the first command.
    pub fn open(path: &str, baud: u32, protocol: CatProtocol) -> Result<Self, PttError> {
        let port = serialport::new(path, baud)
            .timeout(std::time::Duration::from_millis(300))
            .open()
            .map_err(|e| {
                PttError::Backend(format!(
                    "cannot open the CAT port {path} ({e}). `aetherd --list-ports` shows what \
                     is there; a port that exists but will not open is usually held by another \
                     program, such as a rig-control or logging one"
                ))
            })?;
        let mut ptt = Self {
            path: path.to_owned(),
            protocol,
            port,
        };
        ptt.set(false)?;
        Ok(ptt)
    }

    fn set(&mut self, keyed: bool) -> Result<(), PttError> {
        use std::io::{Read as _, Write as _};

        let command = self.protocol.keying(keyed);
        self.port
            .write_all(&command)
            .and_then(|()| self.port.flush())
            .map_err(|e| PttError::Backend(format!("{}: {e}", self.path)))?;
        if !self.protocol.acknowledges_keying() {
            return Ok(());
        }
        let mut reply = [0u8; 32];
        let mut got = Vec::new();
        // the answer is a few bytes; read until the terminator or the timeout
        while !got.contains(&0xFD) {
            match self.port.read(&mut reply) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&reply[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => break,
                Err(e) => return Err(PttError::Backend(format!("{}: {e}", self.path))),
            }
        }
        if self.protocol.keying_accepted(&got) {
            Ok(())
        } else {
            Err(PttError::Backend(format!(
                "{}: the radio did not accept the keying command (answer {:02X?}); check the \
                 CI-V address and rate",
                self.path, got
            )))
        }
    }

    fn ask(&mut self, query: &[u8]) -> Option<Vec<u8>> {
        use std::io::{Read as _, Write as _};

        self.port.write_all(query).ok()?;
        self.port.flush().ok()?;
        let mut reply = [0u8; 64];
        let mut got = Vec::new();
        let done = |got: &[u8]| match self.protocol {
            CatProtocol::Icom { .. } => got.contains(&0xFD),
            _ => got.contains(&b';'),
        };
        while !done(&got) {
            match self.port.read(&mut reply) {
                Ok(n) if n > 0 => got.extend_from_slice(&reply[..n]),
                // nothing more, or the timeout: what has arrived is the answer
                _ => break,
            }
        }
        (!got.is_empty()).then_some(got)
    }
}

impl Ptt for CatPtt {
    fn key(&mut self) -> Result<(), PttError> {
        self.set(true)
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        self.set(false)
    }

    fn describe(&self) -> String {
        let which = match self.protocol {
            CatProtocol::Yaesu => "Yaesu CAT",
            CatProtocol::Kenwood => "Kenwood CAT",
            CatProtocol::Icom { .. } => "Icom CI-V",
        };
        format!("{which} on {}", self.path)
    }

    fn frequency_hz(&mut self) -> Option<u64> {
        let query = self.protocol.frequency_query();
        let reply = self.ask(&query)?;
        self.protocol.parse_frequency(&reply)
    }

    fn can_tune(&self) -> bool {
        true
    }

    fn set_frequency_hz(&mut self, hz: u64) -> Result<(), PttError> {
        use std::io::Write as _;

        let command = self.protocol.frequency_set(hz);
        if self.protocol.acknowledges_keying() {
            // an Icom answers FB or FA to every command, this one included
            let reply = self.ask(&command).unwrap_or_default();
            if !self.protocol.keying_accepted(&reply) {
                return Err(PttError::Backend(format!(
                    "{}: the radio did not accept the frequency (answer {:02X?})",
                    self.path, reply
                )));
            }
            return Ok(());
        }
        self.port
            .write_all(&command)
            .and_then(|()| self.port.flush())
            .map_err(|e| PttError::Backend(format!("{}: {e}", self.path)))?;
        // a Yaesu or Kenwood says nothing back: read the dial to see that it moved
        match self.frequency_hz() {
            Some(now) if now.abs_diff(hz) <= 10 => Ok(()),
            Some(now) => Err(PttError::Backend(format!(
                "{}: the radio reads {now} Hz after being asked for {hz}; a locked VFO or a \
                 band the radio has not got",
                self.path
            ))),
            None => Err(PttError::Backend(format!(
                "{}: the radio did not answer after the frequency was set",
                self.path
            ))),
        }
    }
}

/// Keying through a serial port's RTS or DTR line.
///
/// This is the oldest and most common interface there is: a transistor across one of the
/// port's handshake lines, closing the radio's PTT. The port is held open for the life of the
/// session rather than opened per transmission, because opening a serial port asserts its
/// control lines on some drivers — which would key the radio.
pub struct SerialPtt {
    path: String,
    line: SerialLine,
    port: Box<dyn serialport::SerialPort>,
}

impl std::fmt::Debug for SerialPtt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialPtt")
            .field("path", &self.path)
            .field("line", &self.line)
            .finish_non_exhaustive()
    }
}

impl SerialPtt {
    /// Open a port and put its keying line into receive.
    ///
    /// # Errors
    /// If the port cannot be opened, or its control lines cannot be set — which on most
    /// systems means something else already has the port.
    pub fn open(path: &str, line: SerialLine) -> Result<Self, PttError> {
        // the baud rate is irrelevant: nothing is ever written, only the control lines move
        let port = serialport::new(path, 9600)
            .timeout(std::time::Duration::from_millis(100))
            .open()
            .map_err(|e| {
                PttError::Backend(format!(
                    "cannot open the serial port {path} ({e}). `aetherd --list-ports` shows \
                     what is there; a port that exists but will not open is usually held by \
                     another program, such as a rig-control one"
                ))
            })?;
        let mut ptt = Self {
            path: path.to_owned(),
            line,
            port,
        };
        // leave it in receive, whatever opening the port did to the lines
        ptt.set(false)?;
        Ok(ptt)
    }

    fn set(&mut self, keyed: bool) -> Result<(), PttError> {
        let mut apply = |which: SerialLine| -> Result<(), PttError> {
            let result = match which {
                SerialLine::Rts => self.port.write_request_to_send(keyed),
                SerialLine::Dtr => self.port.write_data_terminal_ready(keyed),
                SerialLine::Both => unreachable!("handled by the caller"),
            };
            result.map_err(|e| PttError::Backend(format!("{}: {e}", self.path)))
        };
        match self.line {
            SerialLine::Both => {
                let rts = apply(SerialLine::Rts);
                let dtr = apply(SerialLine::Dtr);
                rts.and(dtr)
            }
            other => apply(other),
        }
    }
}

impl Ptt for SerialPtt {
    fn key(&mut self) -> Result<(), PttError> {
        self.set(true)
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        self.set(false)
    }

    fn describe(&self) -> String {
        let line = match self.line {
            SerialLine::Rts => "RTS",
            SerialLine::Dtr => "DTR",
            SerialLine::Both => "RTS and DTR",
        };
        format!("{line} on {}", self.path)
    }
}

/// Serial ports the system offers, for an operator choosing one.
#[must_use]
pub fn list_serial_ports() -> Vec<SerialPortInfo> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(SerialPortInfo::from).collect())
        .unwrap_or_default()
}

/// A serial port as the operator should see it: its name, and what is behind it.
///
/// A radio's USB port often presents two serial ports and only one of them keys. The
/// FTDX10's Silicon Labs bridge calls them the *Enhanced* port (CAT) and the *Standard* port
/// (PTT and CW on its RTS/DTR), and Windows numbers them in whichever order it met them —
/// so "COM6, the one I always use" keys nothing while COM7 does. A name alone cannot say
/// which is which; the description can.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SerialPortInfo {
    /// The name the configuration takes: `COM7`, `/dev/ttyUSB1`.
    pub name: String,
    /// What the driver says is behind it, or empty when it says nothing.
    pub description: String,
}

impl From<serialport::SerialPortInfo> for SerialPortInfo {
    fn from(port: serialport::SerialPortInfo) -> Self {
        let description = match port.port_type {
            serialport::SerialPortType::UsbPort(usb) => {
                let product = usb.product.unwrap_or_default();
                // Windows appends the port's own name to the friendly name; it is the name
                // column already
                let product = product
                    .strip_suffix(&format!(" ({})", port.port_name))
                    .unwrap_or(&product)
                    .to_owned();
                match usb.manufacturer {
                    Some(maker) if !product.starts_with(&maker) && !product.is_empty() => {
                        format!("{maker} {product}")
                    }
                    Some(maker) if product.is_empty() => maker,
                    _ => product,
                }
            }
            serialport::SerialPortType::BluetoothPort => "Bluetooth".to_owned(),
            serialport::SerialPortType::PciPort => "built in".to_owned(),
            serialport::SerialPortType::Unknown => String::new(),
        };
        Self {
            name: port.port_name,
            description,
        }
    }
}

// ── CM108-class interfaces: keying through the codec's own GPIO pin ──────────────────

/// Whether a USB identity is one of the sound-card codecs whose general-purpose pins the
/// DRA, URI, RA-40 and similar interfaces bring out to a PTT transistor: C-Media's CM108,
/// CM108AH, CM108B, CM119, CM119A and CM119B, Solid State System's SSS1621/1623, and the
/// AIOC cable that emulates one. The ids are the ones the parts report; a maker's EEPROM
/// can override them, which is what `[ptt] device` is for.
#[must_use]
pub fn is_gpio_codec(vendor: u16, product: u16) -> bool {
    match vendor {
        0x0D8C => matches!(
            product,
            0x0008..=0x000F | 0x0012 | 0x0013 | 0x0139 | 0x013A | 0x013C
        ),
        0x0C76 => matches!(product, 0x1605 | 0x1607 | 0x160B),
        0x1209 => product == 0x7388,
        _ => false,
    }
}

/// The HID output report that drives one pin, as the CM108 data sheet lays the four bytes
/// out — register bits left alone, the pins' output data, the pins' direction (a set bit
/// makes that pin an output), a spare — behind the report-id byte of zero that a device
/// with unnumbered reports takes. One pin in the mask, so the others are not touched;
/// bit 0 is GPIO1.
#[must_use]
pub fn gpio_report(pin: u8, high: bool) -> [u8; 5] {
    let mask = 1u8 << (pin.clamp(1, 8) - 1);
    [0x00, 0x00, if high { mask } else { 0x00 }, mask, 0x00]
}

/// A CM108-class interface as the operator should see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GpioInterfaceInfo {
    /// The path the configuration takes to name this one among several.
    pub path: String,
    /// What the interface calls itself.
    pub name: String,
}

/// The CM108-class interfaces on this machine, for an operator choosing one.
#[must_use]
pub fn list_gpio_interfaces() -> Vec<GpioInterfaceInfo> {
    hidapi::HidApi::new()
        .map(|api| gpio_interfaces_in(&api))
        .unwrap_or_default()
}

fn gpio_interfaces_in(api: &hidapi::HidApi) -> Vec<GpioInterfaceInfo> {
    let mut out: Vec<GpioInterfaceInfo> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for device in api.device_list() {
        if !is_gpio_codec(device.vendor_id(), device.product_id()) {
            continue;
        }
        // Windows lists a codec once per HID collection: one entry per device is enough
        let path = device.path().to_string_lossy().into_owned();
        let key = instance_key(&path);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(GpioInterfaceInfo {
            path,
            name: interface_name(device),
        });
    }
    out
}

fn interface_name(device: &hidapi::DeviceInfo) -> String {
    let product = device.product_string().unwrap_or("").trim();
    let maker = device.manufacturer_string().unwrap_or("").trim();
    let mut name = if maker.is_empty() || product.starts_with(maker) {
        product.to_owned()
    } else {
        format!("{maker} {product}")
    };
    if name.is_empty() {
        name = format!(
            "CM108-class codec {:04x}:{:04x}",
            device.vendor_id(),
            device.product_id()
        );
    }
    if let Some(serial) = device.serial_number().filter(|s| !s.trim().is_empty()) {
        name = format!("{name} ({})", serial.trim());
    }
    name
}

/// What identifies the device behind a HID path. Windows gives each of a device's HID
/// collections a path of its own, differing only in a `&colNN` field; the GPIO report is
/// accepted by one of them, so the device is the unit and the collections are tried.
fn instance_key(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    match lower.find("&col") {
        Some(at) if lower.len() >= at + 6 => format!("{}{}", &lower[..at], &lower[at + 6..]),
        _ => lower,
    }
}

/// Keying through the GPIO pin of a CM108-class sound-card interface — the DRA, URI, RA-40
/// and most "USB radio interface" boards built on a C-Media codec. The codec that carries
/// the audio also holds the PTT transistor, so there is no serial port at all: one USB
/// cable for both.
pub struct GpioPtt {
    name: String,
    pin: u8,
    device: hidapi::HidDevice,
}

impl std::fmt::Debug for GpioPtt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpioPtt")
            .field("name", &self.name)
            .field("pin", &self.pin)
            .finish_non_exhaustive()
    }
}

impl GpioPtt {
    /// Open the interface — the one named by path or by part of its name, or the first
    /// one on the machine — and put its pin into receive.
    ///
    /// # Errors
    /// If there is no such interface, or none of its HID collections takes the report —
    /// which on Linux is usually a `/dev/hidraw` the operator may not write to.
    pub fn open(which: Option<&str>, pin: u8) -> Result<Self, PttError> {
        if !(1..=8).contains(&pin) {
            return Err(PttError::Backend(format!(
                "[ptt] gpio must be 1–8, not {pin}; the DRA and URI boards key on 3"
            )));
        }
        let api = hidapi::HidApi::new()
            .map_err(|e| PttError::Backend(format!("HID devices cannot be listed ({e})")))?;
        let listed = gpio_interfaces_in(&api);
        let chosen = match which {
            Some(wanted) => listed.iter().find(|d| {
                d.path == wanted || d.name.to_lowercase().contains(&wanted.to_lowercase())
            }),
            None => listed.first(),
        };
        let Some(chosen) = chosen else {
            return Err(PttError::Backend(match which {
                Some(wanted) => format!(
                    "no CM108-class interface matches {wanted:?}; `aetherd --list-ports` shows \
                     what is there"
                ),
                None => "no CM108-class interface is plugged in; `aetherd --list-ports` shows \
                         what is there"
                    .to_owned(),
            }));
        };
        let key = instance_key(&chosen.path);
        let mut last_error = String::new();
        let collections: Vec<_> = api
            .device_list()
            .filter(|d| instance_key(&d.path().to_string_lossy()) == key)
            .collect();
        for collection in collections {
            match collection.open_device(&api) {
                Ok(device) => {
                    let mut ptt = Self {
                        name: chosen.name.clone(),
                        pin,
                        device,
                    };
                    match ptt.set(false) {
                        Ok(()) => return Ok(ptt),
                        Err(PttError::Backend(message)) => last_error = message,
                        Err(other) => return Err(other),
                    }
                }
                Err(e) => last_error = e.to_string(),
            }
        }
        Err(PttError::Backend(format!(
            "{}: cannot drive its GPIO ({last_error}). On Linux the hidraw device needs a \
             udev rule; docs/user/gateway-kit.md has it",
            chosen.name
        )))
    }

    fn set(&mut self, high: bool) -> Result<(), PttError> {
        self.device
            .write(&gpio_report(self.pin, high))
            .map(|_| ())
            .map_err(|e| PttError::Backend(format!("{}: {e}", self.name)))
    }
}

impl Ptt for GpioPtt {
    fn key(&mut self) -> Result<(), PttError> {
        self.set(true)
    }

    fn unkey(&mut self) -> Result<(), PttError> {
        self.set(false)
    }

    fn describe(&self) -> String {
        format!("GPIO{} of {}", self.pin, self.name)
    }
}

/// Check that a callsign is one the protocol can carry.
///
/// The link layer packs callsigns six bits per character into seven bytes, so anything
/// outside `A–Z 0–9 - /` or longer than nine characters cannot go on the air at all. Better
/// to say so when the configuration is read than when the first connection is attempted.
///
/// # Errors
/// If the callsign is empty, too long, or holds a character the format has no room for.
pub fn validate_callsign(call: &str) -> Result<(), PttError> {
    aether_link::frames::pack_callsign(call)
        .map(|_| ())
        .map_err(|e| PttError::Backend(format!("callsign {call:?}: {e}")))
}

#[cfg(test)]
mod gpio_tests {
    use super::*;

    #[test]
    fn the_report_drives_one_pin_and_leaves_the_others_alone() {
        assert_eq!(gpio_report(3, true), [0, 0, 0b0100, 0b0100, 0]);
        assert_eq!(gpio_report(3, false), [0, 0, 0, 0b0100, 0]);
        assert_eq!(gpio_report(1, true), [0, 0, 1, 1, 0]);
        assert_eq!(gpio_report(8, true), [0, 0, 0x80, 0x80, 0]);
    }

    #[test]
    fn the_codecs_with_pins_are_known_by_their_ids() {
        assert!(is_gpio_codec(0x0D8C, 0x0008)); // CM108
        assert!(is_gpio_codec(0x0D8C, 0x000F)); // CM119
        assert!(is_gpio_codec(0x0D8C, 0x0012)); // CM108B
        assert!(is_gpio_codec(0x0D8C, 0x013A)); // CM119A
        assert!(is_gpio_codec(0x0C76, 0x1607)); // SSS1623
        assert!(is_gpio_codec(0x1209, 0x7388)); // AIOC
        assert!(!is_gpio_codec(0x0D8C, 0x0100));
        assert!(!is_gpio_codec(0x046D, 0x0008)); // another maker's product 8
    }

    #[test]
    fn a_windows_path_names_the_device_and_not_one_of_its_collections() {
        let one = r"\\?\hid#vid_0d8c&pid_0008&mi_03&col01#7&1a2b&0&0000#{4d1e55b2}";
        let two = r"\\?\hid#vid_0d8c&pid_0008&mi_03&col02#7&1a2b&0&0000#{4d1e55b2}";
        assert_eq!(instance_key(one), instance_key(two));
        assert_ne!(instance_key("/dev/hidraw0"), instance_key("/dev/hidraw1"));
    }

    #[test]
    fn a_pin_the_codecs_do_not_have_is_refused() {
        let error = GpioPtt::open(None, 9).expect_err("no such pin");
        assert!(error.to_string().contains("1–8"), "{error}");
    }
}
