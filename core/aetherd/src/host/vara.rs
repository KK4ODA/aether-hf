//! The VARA-compatible command protocol, as a state machine with no sockets in it.
//!
//! Winlink Express, Pat, `VarAC` and BPQ32 all speak the same published TCP interface: a
//! line-oriented command port and a binary data port beside it. Speaking it is what lets an
//! Aether station be used by software people already run, which — per
//! `docs/COMMUNITY-CONCERNS.md` — matters more to adoption than the waveform does.
//!
//! Only the *published* command set is implemented, from its documented behaviour. Nothing
//! here is derived from VARA's internals, and the air interface is Aether's own: two Aether
//! stations talk to each other, not to VARA.
//!
//! # Honesty about what this is
//!
//! `VERSION` answers with Aether's name, not VARA's. A client that asks what it is connected
//! to gets a true answer, and a gateway operator who lists a station as VARA when it runs
//! Aether would strand every real-VARA client that called it. The compatibility is in the
//! host interface, and saying so is the point rather than a limitation.
//!
//! Aether has one bandwidth (ADR-0002), so `BW2300` is accepted and the others are refused
//! rather than silently ignored: a client that asked for 500 Hz and got 2300 would be
//! transmitting outside what its operator chose.

use std::fmt::Write as _;

/// Default command port. The data port is conventionally the next one up.
pub const DEFAULT_COMMAND_PORT: u16 = 8300;

/// What the adapter reports for `VERSION`.
///
/// Aether's own name on purpose: see the module documentation.
#[must_use]
pub fn version_string() -> String {
    // Three words, like the published reply's shape — <product> <band> <version> — so a
    // host that takes the version from the fourth token of the line finds one. VarAC does,
    // and "Aether-HF-0.2.0" put an exception in its log. The name is still this modem's.
    format!("Aether HF {}", env!("CARGO_PKG_VERSION"))
}

/// Bandwidth this physical layer offers, in hertz (ADR-0002).
pub const BANDWIDTH_HZ: u32 = 2300;

/// What the host asked the modem to do, once a command has been understood.
#[derive(Debug, Clone, PartialEq)]
pub enum HostAction {
    /// Nothing beyond the reply.
    None,
    /// Start a session.
    Connect {
        /// The calling station.
        from: String,
        /// The station being called.
        to: String,
    },
    /// Close the session once the queue drains.
    Disconnect,
    /// Drop the session now.
    Abort,
    /// Start or stop answering calls.
    Listen(bool),
    /// Answer to these callsigns from now on, and call as the first of them.
    Callsigns(Vec<String>),
    /// Put a carrier on the air for tuning, for this many seconds.
    Tune(f64),
    /// Send an unproto identification frame.
    CqFrame,
}

/// What the adapter should do with a command: what to say, and what to act on.
#[derive(Debug, Clone, PartialEq)]
pub struct HostOutcome {
    /// Lines to send back on the command port, in order.
    pub replies: Vec<String>,
    /// What the modem should do.
    pub action: HostAction,
}

impl HostOutcome {
    fn just(reply: &str) -> Self {
        Self {
            replies: vec![reply.to_owned()],
            action: HostAction::None,
        }
    }

    fn ok() -> Self {
        Self::just("OK")
    }

    fn wrong() -> Self {
        Self::just("WRONG")
    }

    fn acting(action: HostAction) -> Self {
        Self {
            replies: vec!["OK".to_owned()],
            action,
        }
    }
}

/// How the host wants payload compressed, as the published interface words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// No compression.
    #[default]
    Off,
    /// Compress text.
    Text,
    /// Compress files.
    Files,
}

/// Which kind of session the host is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionKind {
    /// Peer to peer between two amateur stations.
    #[default]
    P2p,
    /// A Winlink radio-mail session.
    Winlink,
}

/// Everything one host connection has told the modem about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostState {
    /// Callsigns this station will answer to. The first is the station's own.
    pub callsigns: Vec<String>,
    /// Whether the host wants incoming calls answered.
    pub listening: bool,
    /// Whether the station may be listed publicly.
    pub public: bool,
    /// What the host asked for.
    pub compression: Compression,
    /// What kind of session is running.
    pub session: SessionKind,
    /// Whether to identify in Morse at the end of a transmission.
    pub cw_id: bool,
    /// Bytes the modem still has to send, as last reported.
    pub buffer: usize,
    /// The bandwidth the station runs, in hertz: the one `BW<n>` is accepted for, and
    /// the one `CONNECTED` reports. The server sets it from the modem's `capabilities`.
    pub bandwidth_hz: u32,
    /// Settings the host asked for that this modem hears and does not act on.
    pub recorded: Recorded,
}

/// What a host asked for that changes nothing here — heard, so the host is not told the
/// modem is broken, and kept, so the control API can say what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recorded {
    /// `CHAT ON`: `VarAC` asks for it on every start, and the short frames it means are a
    /// different air interface this modem does not have.
    pub chat: bool,
    /// `LISTEN CQ`: hear only CQ frames. This station hears everything and answers calls to
    /// its own callsigns either way.
    pub cq_only: bool,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            callsigns: Vec::new(),
            listening: false,
            recorded: Recorded::default(),
            public: true,
            compression: Compression::default(),
            session: SessionKind::default(),
            cw_id: false,
            buffer: 0,
            bandwidth_hz: BANDWIDTH_HZ,
        }
    }
}

/// Whether a callsign is one the link layer can carry.
fn plausible_callsign(call: &str) -> bool {
    !call.is_empty()
        && call.len() <= 9
        && call
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'/')
}

impl HostState {
    /// This station's own callsign, if one has been set.
    #[must_use]
    pub fn my_call(&self) -> Option<&str> {
        self.callsigns.first().map(String::as_str)
    }

    /// Whether a call is addressed to this station.
    #[must_use]
    pub fn answers_to(&self, call: &str) -> bool {
        self.callsigns.iter().any(|mine| mine == call)
    }

    /// Handle one command line from the host.
    ///
    /// The published interface is case-insensitive in practice and clients differ in what
    /// they send, so commands are matched upper-cased; callsigns are upper-cased too, which
    /// is what goes on the air.
    pub fn command(&mut self, line: &str) -> HostOutcome {
        let line = line.trim();
        if line.is_empty() {
            return HostOutcome {
                replies: Vec::new(),
                action: HostAction::None,
            };
        }
        let upper = line.to_ascii_uppercase();
        let mut words = upper.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let rest: Vec<&str> = words.collect();

        match (verb, rest.as_slice()) {
            ("MYCALL", calls) if !calls.is_empty() => self.set_callsigns(calls),
            ("CONNECT", [from, to, ..]) => {
                if !plausible_callsign(from) || !plausible_callsign(to) {
                    return HostOutcome::wrong();
                }
                HostOutcome::acting(HostAction::Connect {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                })
            }
            ("DISCONNECT", []) => HostOutcome::acting(HostAction::Disconnect),
            ("ABORT", []) => HostOutcome::acting(HostAction::Abort),
            ("LISTEN", ["ON"]) => {
                self.listening = true;
                HostOutcome::acting(HostAction::Listen(true))
            }
            ("LISTEN", ["OFF"]) => {
                self.listening = false;
                HostOutcome::acting(HostAction::Listen(false))
            }
            // VarAC: hear only CQ frames. This station hears everything and answers calls
            // to its own callsigns regardless, which is what a host wanting CQs also wants.
            ("LISTEN", ["CQ"]) => {
                self.listening = true;
                self.recorded.cq_only = true;
                HostOutcome::acting(HostAction::Listen(true))
            }
            ("CHAT", ["ON"]) => {
                self.recorded.chat = true;
                HostOutcome::ok()
            }
            ("CHAT", ["OFF"]) => {
                self.recorded.chat = false;
                HostOutcome::ok()
            }

            ("PUBLIC", ["ON"]) => {
                self.public = true;
                HostOutcome::ok()
            }
            ("PUBLIC", ["OFF"]) => {
                self.public = false;
                HostOutcome::ok()
            }
            ("COMPRESSION", ["OFF"]) => {
                self.compression = Compression::Off;
                HostOutcome::ok()
            }
            // Winlink Express says `COMPRESSION ON`, which is not in the published set; it
            // means what TEXT means, and a WRONG here is the first thing in its log
            ("COMPRESSION", ["TEXT" | "ON"]) => {
                self.compression = Compression::Text;
                HostOutcome::ok()
            }
            ("COMPRESSION", ["FILES"]) => {
                self.compression = Compression::Files;
                HostOutcome::ok()
            }
            ("WINLINK", ["SESSION"]) => {
                self.session = SessionKind::Winlink;
                HostOutcome::ok()
            }
            ("P2P", ["SESSION"]) => {
                self.session = SessionKind::P2p;
                HostOutcome::ok()
            }
            ("CWID", ["ON"]) => {
                self.cw_id = true;
                HostOutcome::ok()
            }
            ("CWID", ["OFF"]) => {
                self.cw_id = false;
                HostOutcome::ok()
            }
            ("CQFRAME", _) => HostOutcome::acting(HostAction::CqFrame),
            ("VERSION", []) => HostOutcome::just(&format!("VERSION {}", version_string())),
            ("BUFFER", []) => HostOutcome::just(&format!("BUFFER {}", self.buffer)),
            ("TUNE", ["OFF"]) => HostOutcome::acting(HostAction::Tune(0.0)),
            ("TUNE", [seconds]) => match seconds.parse::<f64>() {
                Ok(value) if (0.0..=30.0).contains(&value) => {
                    HostOutcome::acting(HostAction::Tune(value))
                }
                _ => HostOutcome::wrong(),
            },
            // The station runs one bandwidth, chosen in its configuration (2300 or 500 Hz).
            // A host asking for that one is answered `OK`; one asking for another is refused,
            // because accepting and then transmitting the configured bandwidth anyway would
            // put the station outside what its operator asked for — VarAC at 500 Hz on a
            // calling frequency most of all. `BW2750` is not a waveform this version has.
            ("BW2300" | "BW500", []) if Self::bandwidth_of(verb) == Some(self.bandwidth_hz) => {
                HostOutcome::ok()
            }
            // a KISS-port detail with no KISS port behind it: heard, nothing to do
            ("IGNOREKISSDCD", ["ON" | "OFF"]) => HostOutcome::ok(),
            _ => HostOutcome::wrong(),
        }
    }

    /// The bandwidth a `BW<n>` command names.
    fn bandwidth_of(command: &str) -> Option<u32> {
        command.strip_prefix("BW")?.parse().ok()
    }

    fn set_callsigns(&mut self, calls: &[&str]) -> HostOutcome {
        // clients send them space-separated, and some comma-separated
        let parsed: Vec<String> = calls
            .iter()
            .flat_map(|entry| entry.split(','))
            .map(str::trim)
            .filter(|call| !call.is_empty())
            .map(str::to_owned)
            .collect();
        if parsed.is_empty() || !parsed.iter().all(|call| plausible_callsign(call)) {
            return HostOutcome::wrong();
        }
        self.callsigns.clone_from(&parsed);
        HostOutcome::acting(HostAction::Callsigns(parsed))
    }
}

/// An unsolicited line the modem sends the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    /// Transmit started or stopped.
    Ptt(bool),
    /// The channel became busy or free.
    Busy(bool),
    /// A session is up.
    Connected {
        /// The station that called.
        caller: String,
        /// The station that was called. A host takes a `CONNECTED` whose second callsign is
        /// not its own as somebody else's business, so the order is not decoration.
        called: String,
        /// The bandwidth in use, in hertz.
        bandwidth_hz: u32,
    },
    /// The session ended.
    Disconnected,
    /// How many payload bytes the modem still has to send.
    Buffer(usize),
    /// A keep-alive, so a host that has heard nothing knows the modem is there.
    IAmAlive,
    /// The station's callsign is usable; sent so a client does not warn about a speed limit
    /// it does not have. Aether is free software and has no registration.
    Registered(String),
}

impl Notification {
    /// The line to write on the command port, without its terminator.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out = String::new();
        match self {
            Self::Ptt(true) => out.push_str("PTT ON"),
            Self::Ptt(false) => out.push_str("PTT OFF"),
            Self::Busy(true) => out.push_str("BUSY ON"),
            Self::Busy(false) => out.push_str("BUSY OFF"),
            Self::Connected {
                caller,
                called,
                bandwidth_hz,
            } => {
                let _ = write!(out, "CONNECTED {caller} {called} {bandwidth_hz}");
            }
            Self::Disconnected => out.push_str("DISCONNECTED"),
            Self::Buffer(bytes) => {
                let _ = write!(out, "BUFFER {bytes}");
            }
            Self::IAmAlive => out.push_str("IAMALIVE"),
            Self::Registered(call) => {
                let _ = write!(out, "REGISTERED {call}");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HostState {
        HostState::default()
    }

    #[test]
    fn a_callsign_is_accepted_and_remembered() {
        let mut host = state();
        let outcome = host.command("MYCALL W4ODA");
        assert_eq!(outcome.replies, vec!["OK"]);
        assert_eq!(host.my_call(), Some("W4ODA"));
        assert!(host.answers_to("W4ODA"));
        assert!(!host.answers_to("KK4XYZ"));
        // the modem answers to it too: the host owns the operator's callsign
        assert_eq!(
            outcome.action,
            HostAction::Callsigns(vec!["W4ODA".to_owned()])
        );
    }

    #[test]
    fn several_callsigns_are_accepted_however_the_client_separates_them() {
        // clients differ; a station that answers to its club call as well should not depend
        // on which of them it happens to be talking to
        for line in ["MYCALL W4ODA KK4XYZ", "MYCALL W4ODA,KK4XYZ"] {
            let mut host = state();
            assert_eq!(host.command(line).replies, vec!["OK"], "{line}");
            assert!(
                host.answers_to("W4ODA") && host.answers_to("KK4XYZ"),
                "{line}"
            );
        }
    }

    #[test]
    fn a_callsign_the_air_interface_cannot_carry_is_refused() {
        // the link layer packs callsigns six bits per character into seven bytes; accepting
        // one it cannot carry would fail later, on the air, for no visible reason
        let mut host = state();
        for line in ["MYCALL", "MYCALL TOOLONGCALL", "MYCALL W4ODA!"] {
            assert_eq!(host.command(line).replies, vec!["WRONG"], "{line}");
        }
        assert_eq!(host.my_call(), None);
    }

    #[test]
    fn connect_carries_both_callsigns_through() {
        let mut host = state();
        let outcome = host.command("CONNECT W4ODA KK4XYZ");
        assert_eq!(outcome.replies, vec!["OK"]);
        assert_eq!(
            outcome.action,
            HostAction::Connect {
                from: "W4ODA".into(),
                to: "KK4XYZ".into()
            }
        );
    }

    #[test]
    fn commands_are_case_insensitive_and_callsigns_come_back_upper_case() {
        // what goes on the air is upper case, and clients are inconsistent about what they
        // send, so a lower-case command must not silently produce a different callsign
        let mut host = state();
        let outcome = host.command("connect w4oda kk4xyz");
        assert_eq!(
            outcome.action,
            HostAction::Connect {
                from: "W4ODA".into(),
                to: "KK4XYZ".into()
            }
        );
    }

    #[test]
    fn disconnect_and_abort_are_different_actions() {
        let mut host = state();
        assert_eq!(host.command("DISCONNECT").action, HostAction::Disconnect);
        assert_eq!(host.command("ABORT").action, HostAction::Abort);
    }

    #[test]
    fn listening_can_be_turned_on_and_off() {
        let mut host = state();
        assert!(
            !host.listening,
            "a station must not answer calls until told to"
        );
        assert_eq!(host.command("LISTEN ON").action, HostAction::Listen(true));
        assert!(host.listening);
        assert_eq!(host.command("LISTEN OFF").action, HostAction::Listen(false));
        assert!(!host.listening);
    }

    #[test]
    fn what_varac_says_on_every_start_is_heard_rather_than_refused() {
        // verbatim from VarAC 15.0.18's first conversation with this modem: three of these
        // came back WRONG, which is what a modem says to a command it has never heard of
        let mut host = HostState::default();
        assert_eq!(host.command("CHAT ON").replies, vec!["OK"]);
        assert!(host.recorded.chat);
        assert_eq!(host.command("CHAT OFF").replies, vec!["OK"]);
        assert_eq!(host.command("IGNOREKISSDCD ON").replies, vec!["OK"]);
        let listen = host.command("LISTEN CQ");
        assert_eq!(listen.replies, vec!["OK"]);
        assert_eq!(listen.action, HostAction::Listen(true));
        assert!(host.listening && host.recorded.cq_only);
        // and the two it says that this modem genuinely cannot do stay refused
        assert_eq!(host.command("BW500").replies, vec!["WRONG"]);
        // Winlink Express 1.8.5's own opening line, verbatim: PUBLIC ON, CWID ON,
        // COMPRESSION ON, BW<max>, MYCALL, LISTEN ON
        assert_eq!(host.command("COMPRESSION ON").replies, vec!["OK"]);
        assert_eq!(host.compression, Compression::Text);
        assert_eq!(
            host.command("MYCALL KK4ODA-1 KK4ODA-T").replies,
            vec!["OK"],
            "a tactical suffix is a callsign like any other"
        );
    }

    #[test]
    fn the_one_bandwidth_this_phy_has_is_accepted_and_the_others_refused() {
        // silently substituting 2300 for a client that asked for 500 would put the station
        // outside the bandwidth its operator chose
        let mut host = state();
        assert_eq!(host.command("BW2300").replies, vec!["OK"]);
        assert_eq!(host.command("BW500").replies, vec!["WRONG"]);
        assert_eq!(host.command("BW2750").replies, vec!["WRONG"]);
        // a station running the narrow waveform accepts BW500 and refuses BW2300
        let mut narrow = HostState {
            bandwidth_hz: 500,
            ..HostState::default()
        };
        assert_eq!(narrow.command("BW500").replies, vec!["OK"]);
        assert_eq!(narrow.command("BW2300").replies, vec!["WRONG"]);
        assert_eq!(narrow.command("BW2750").replies, vec!["WRONG"]);
    }

    #[test]
    fn version_says_what_this_actually_is() {
        // a gateway listed as VARA that runs Aether would strand every real VARA client that
        // called it, so the modem never claims to be one
        let mut host = state();
        let reply = host.command("VERSION").replies.join("");
        assert!(reply.starts_with("VERSION "), "{reply}");
        assert!(reply.contains("Aether"), "{reply}");
        assert!(
            !reply.to_ascii_uppercase().contains("VARA"),
            "the modem claimed to be VARA: {reply}"
        );
    }

    #[test]
    fn session_kind_and_compression_are_remembered() {
        let mut host = state();
        assert_eq!(host.command("WINLINK SESSION").replies, vec!["OK"]);
        assert_eq!(host.session, SessionKind::Winlink);
        assert_eq!(host.command("P2P SESSION").replies, vec!["OK"]);
        assert_eq!(host.session, SessionKind::P2p);

        for (line, want) in [
            ("COMPRESSION TEXT", Compression::Text),
            ("COMPRESSION FILES", Compression::Files),
            ("COMPRESSION OFF", Compression::Off),
        ] {
            assert_eq!(host.command(line).replies, vec!["OK"], "{line}");
            assert_eq!(host.compression, want, "{line}");
        }
    }

    #[test]
    fn buffer_reports_what_is_left_to_send() {
        let mut host = state();
        host.buffer = 1234;
        assert_eq!(host.command("BUFFER").replies, vec!["BUFFER 1234"]);
    }

    #[test]
    fn tune_takes_a_bounded_duration() {
        let mut host = state();
        assert_eq!(host.command("TUNE 5").action, HostAction::Tune(5.0));
        assert_eq!(host.command("TUNE OFF").action, HostAction::Tune(0.0));
        // an unbounded tune is a stuck carrier by another name
        assert_eq!(host.command("TUNE 600").replies, vec!["WRONG"]);
        assert_eq!(host.command("TUNE forever").replies, vec!["WRONG"]);
    }

    #[test]
    fn an_unknown_command_is_refused_rather_than_ignored() {
        // a client that gets silence cannot tell a missing feature from a hung modem
        let mut host = state();
        for line in ["TELEPORT", "CONNECT", "LISTEN MAYBE", "MYCALL "] {
            assert_eq!(host.command(line).replies, vec!["WRONG"], "{line}");
        }
    }

    #[test]
    fn an_empty_line_is_not_an_error() {
        let mut host = state();
        assert!(host.command("").replies.is_empty());
        assert!(host.command("   ").replies.is_empty());
    }

    #[test]
    fn notifications_are_the_lines_the_published_interface_uses() {
        assert_eq!(Notification::Ptt(true).line(), "PTT ON");
        assert_eq!(Notification::Ptt(false).line(), "PTT OFF");
        assert_eq!(Notification::Busy(true).line(), "BUSY ON");
        assert_eq!(Notification::Disconnected.line(), "DISCONNECTED");
        assert_eq!(Notification::Buffer(42).line(), "BUFFER 42");
        assert_eq!(Notification::IAmAlive.line(), "IAMALIVE");
        assert_eq!(
            Notification::Connected {
                caller: "W4ODA".into(),
                called: "KK4XYZ".into(),
                bandwidth_hz: BANDWIDTH_HZ,
            }
            .line(),
            "CONNECTED W4ODA KK4XYZ 2300"
        );
        assert_eq!(
            Notification::Registered("W4ODA".into()).line(),
            "REGISTERED W4ODA"
        );
    }
}
