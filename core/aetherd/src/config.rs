//! The configuration file an operator writes.
//!
//! Everything here has a default that works, except the callsign, which nobody else can
//! guess. A file that says only `callsign = "W4ODA"` starts a receive-only station on the
//! default sound card with no keying — which is the right thing to hand somebody who wants to
//! listen before they transmit.
//!
//! Unknown keys are refused rather than ignored. A typo in a configuration file that silently
//! does nothing is how a station ends up transmitting with a watchdog the operator believed
//! they had set.

use serde::{Deserialize, Serialize};

use crate::{
    audio::AudioConfig,
    busy::BusyConfig,
    ptt::{PttError, SerialLine},
};

/// How the radio is keyed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", deny_unknown_fields)]
pub enum PttConfig {
    /// Not at all: VOX, or a receive-only station.
    #[default]
    None,
    /// A serial port's control line.
    Serial {
        /// Device path: `COM3`, `/dev/ttyUSB0`.
        port: String,
        /// Which line keys the radio.
        #[serde(default)]
        line: SerialLineConfig,
    },
    /// Hamlib's rig-control daemon, over TCP.
    Rigctld {
        /// Where it is listening; conventionally `127.0.0.1:4532`.
        #[serde(default = "default_rigctld")]
        address: String,
    },
}

fn default_rigctld() -> String {
    "127.0.0.1:4532".to_owned()
}

/// Which serial control line keys the radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SerialLineConfig {
    /// Request To Send.
    #[default]
    Rts,
    /// Data Terminal Ready.
    Dtr,
    /// Both together.
    Both,
}

impl From<SerialLineConfig> for SerialLine {
    fn from(value: SerialLineConfig) -> Self {
        match value {
            SerialLineConfig::Rts => Self::Rts,
            SerialLineConfig::Dtr => Self::Dtr,
            SerialLineConfig::Both => Self::Both,
        }
    }
}

/// Sound-card settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSection {
    /// Capture device name, or unset for the system default.
    #[serde(default)]
    pub input: Option<String>,
    /// Playback device name, or unset for the system default.
    #[serde(default)]
    pub output: Option<String>,
    /// Sample rate. The waveform is built around 48 kHz and nothing resamples.
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    /// Transmit level, as an RMS fraction of full scale.
    #[serde(default = "default_tx_level")]
    pub tx_level: f64,
}

fn default_rate() -> u32 {
    48_000
}

fn default_tx_level() -> f64 {
    0.25
}

impl Default for AudioSection {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            sample_rate: default_rate(),
            tx_level: default_tx_level(),
        }
    }
}

/// Limits that keep a station lawful and polite.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadioSection {
    /// Longest a single transmission may last, in seconds.
    #[serde(default = "default_max_key")]
    pub max_key_s: f64,
    /// Refuse to start a session while the channel is occupied.
    #[serde(default = "default_true")]
    pub wait_for_clear: bool,
    /// How far above the learned noise floor counts as occupied, in dB.
    #[serde(default = "default_busy_threshold")]
    pub busy_threshold_db: f64,
    /// Fastest mode this station will use; lower it for a rig that cannot manage the dense
    /// constellations, or a band where they never work.
    #[serde(default = "default_max_mode")]
    pub max_mode: usize,
    /// Offer payload compression in the connect handshake. Used only if the peer offers it
    /// too, so leaving it on costs nothing when talking to a station that cannot.
    #[serde(default = "default_true")]
    pub compress: bool,
}

fn default_max_key() -> f64 {
    30.0
}

fn default_true() -> bool {
    true
}

fn default_busy_threshold() -> f64 {
    6.0
}

fn default_max_mode() -> usize {
    13
}

impl Default for RadioSection {
    fn default() -> Self {
        Self {
            max_key_s: default_max_key(),
            wait_for_clear: default_true(),
            busy_threshold_db: default_busy_threshold(),
            max_mode: default_max_mode(),
            compress: default_true(),
        }
    }
}

/// The control interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSection {
    /// Whether to listen at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Address to listen on. Loopback unless the operator says otherwise, and any other
    /// address requires a token.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Bearer token, required for a non-loopback bind.
    #[serde(default)]
    pub token: Option<String>,
}

fn default_bind() -> String {
    "127.0.0.1:8515".to_owned()
}

impl Default for ControlSection {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            bind: default_bind(),
            token: None,
        }
    }
}

/// The VARA-compatible host interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostSection {
    /// Whether to listen at all. Off by default: a station that has not been asked to accept
    /// commands from other software should not be accepting them.
    #[serde(default)]
    pub enabled: bool,
    /// Command-port address. The data port is the next one up, which every client assumes.
    #[serde(default = "default_host_bind")]
    pub bind: String,
}

fn default_host_bind() -> String {
    "127.0.0.1:8300".to_owned()
}

impl Default for HostSection {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_host_bind(),
        }
    }
}

/// A whole configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// This station's callsign. There is no default; nobody else can supply it.
    pub callsign: String,
    /// Sound card.
    #[serde(default)]
    pub audio: AudioSection,
    /// Keying.
    #[serde(default)]
    pub ptt: PttConfig,
    /// Limits.
    #[serde(default)]
    pub radio: RadioSection,
    /// The control interface.
    #[serde(default)]
    pub control: ControlSection,
    /// The VARA-compatible host interface.
    #[serde(default)]
    pub host: HostSection,
}

/// Why a configuration was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The file could not be read.
    Read(String),
    /// The file is not valid TOML, or has a key this version does not define.
    Parse(String),
    /// A value is outside what the modem can do.
    Invalid(String),
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Read(detail) => write!(f, "cannot read the configuration: {detail}"),
            Self::Parse(detail) => write!(f, "configuration is not valid: {detail}"),
            Self::Invalid(detail) => write!(f, "configuration will not work: {detail}"),
        }
    }
}

impl core::error::Error for ConfigError {}

impl From<PttError> for ConfigError {
    fn from(error: PttError) -> Self {
        Self::Invalid(error.to_string())
    }
}

impl Config {
    /// Parse a configuration from TOML text.
    ///
    /// # Errors
    /// If the text is not valid TOML, carries a key this version does not define, or holds a
    /// value the modem cannot work with.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self =
            toml::from_str(text).map_err(|e| ConfigError::Parse(e.message().to_owned()))?;
        config.validate()?;
        Ok(config)
    }

    /// Read a configuration file.
    ///
    /// # Errors
    /// If the file cannot be read, or its contents are refused.
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Read(format!("{}: {e}", path.display())))?;
        Self::parse(&text)
    }

    /// Check the values against what the modem can actually do.
    ///
    /// # Errors
    /// If a value is outside that.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.callsign.trim().is_empty() {
            return Err(ConfigError::Invalid("a callsign is required".into()));
        }
        crate::ptt::validate_callsign(&self.callsign)?;
        if self.audio.sample_rate != default_rate() {
            return Err(ConfigError::Invalid(format!(
                "the waveform is built around {} Hz and nothing resamples; {} Hz will not work",
                default_rate(),
                self.audio.sample_rate
            )));
        }
        if !(0.0..=1.0).contains(&self.audio.tx_level) || self.audio.tx_level <= 0.0 {
            return Err(ConfigError::Invalid(format!(
                "tx_level is a fraction of full scale, so it must be above 0 and at most 1; \
                 got {}",
                self.audio.tx_level
            )));
        }
        if self.radio.max_key_s <= 0.0 {
            return Err(ConfigError::Invalid(
                "max_key_s must be positive: a watchdog that can never fire is not one".into(),
            ));
        }
        if self.radio.max_mode >= aether_link::AWGN_THRESHOLD_DB.len() {
            return Err(ConfigError::Invalid(format!(
                "max_mode must be below {}, the number of modes this version defines",
                aether_link::AWGN_THRESHOLD_DB.len()
            )));
        }
        // The control interface can key a transmitter, so an address the network can reach
        // is refused here rather than started open and warned about.
        if self.control.enabled
            && !crate::control::server::is_loopback(&self.control.bind)
            && self.control.token.is_none()
        {
            return Err(ConfigError::Invalid(format!(
                "control.bind is {}, which is reachable from the network, and this interface \
                 can key a transmitter. Set control.token, or bind to 127.0.0.1.",
                self.control.bind
            )));
        }
        Ok(())
    }

    /// Sound-card settings in the form the audio layer wants.
    #[must_use]
    pub fn audio_config(&self) -> AudioConfig {
        AudioConfig {
            input: self.audio.input.clone(),
            output: self.audio.output.clone(),
            sample_rate: self.audio.sample_rate,
            ..AudioConfig::default()
        }
    }

    /// Control-interface settings in the form the server wants.
    #[must_use]
    pub fn control_config(&self) -> crate::control::ControlConfig {
        crate::control::ControlConfig {
            bind: self.control.bind.clone(),
            token: self.control.token.clone(),
        }
    }

    /// Host-interface settings in the form the adapter wants.
    #[must_use]
    pub fn host_config(&self) -> crate::host::HostConfig {
        crate::host::HostConfig {
            enabled: self.host.enabled,
            bind: self.host.bind.clone(),
        }
    }

    /// Busy-detector settings in the form the detector wants.
    #[must_use]
    pub fn busy_config(&self) -> BusyConfig {
        BusyConfig {
            threshold_db: self.radio.busy_threshold_db,
            ..BusyConfig::default()
        }
    }
}

/// An example file, for `aetherd --example-config`.
pub const EXAMPLE: &str = r#"# Aether HF station configuration.
#
# Only `callsign` has no default. A file with nothing else in it starts a receive-only
# station on the default sound card, which is a good way to listen before transmitting.

callsign = "N0CALL"

[audio]
# Names as `aetherd --list-devices` prints them. Unset means the system default.
# input = "USB Audio CODEC"
# output = "USB Audio CODEC"
sample_rate = 48000
# Transmit level as a fraction of full scale, RMS. Leave headroom: the waveform has peaks.
tx_level = 0.25

[ptt]
# kind = "none"                       # VOX, or receive only
# kind = "serial"                     # a serial control line
# port = "COM3"                       # `aetherd --list-ports` shows what is there
# line = "rts"                        # rts | dtr | both
kind = "rigctld"                      # Hamlib's rig control daemon
address = "127.0.0.1:4532"

[radio]
# The longest a single transmission may last. If this fires the key is released and stays
# released until the software asks again from a clean state.
max_key_s = 30.0
# Wait for a clear channel before starting a session. A session already under way always
# answers: the peer is waiting, and silence only makes it retransmit.
wait_for_clear = true
busy_threshold_db = 6.0
# The fastest mode this station will use, 0 to 13.
max_mode = 13
# Offer payload compression. Used only if the other station offers it too, so leaving this on
# costs nothing when talking to one that cannot.
compress = true

[control]
# The modem's own interface: JSON over WebSocket at ws://<bind>/v1, or POST /v1/<method>.
enabled = true
# Loopback needs no token. Any other address does, and the daemon refuses to start without
# one rather than leaving a transmitter open to the network.
bind = "127.0.0.1:8515"
# token = "a long random string"

[host]
# The VARA-compatible host interface, so Winlink Express, Pat, VarAC and BPQ32 can use this
# station. Off unless asked for. The data port is the command port plus one, and both have to
# be free. `VERSION` answers with Aether's name, not VARA's — see docs/spec/host-interfaces.md.
enabled = false
bind = "127.0.0.1:8300"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_useful_file_is_a_callsign() {
        let config = Config::parse("callsign = \"W4ODA\"").expect("parse");
        assert_eq!(config.callsign, "W4ODA");
        assert_eq!(
            config.ptt,
            PttConfig::None,
            "a station must not key by default"
        );
        assert!(config.radio.wait_for_clear, "and must be polite by default");
        assert_eq!(config.audio.input, None);
    }

    #[test]
    fn the_example_file_is_valid() {
        // it is the first thing an operator edits, so it has to parse as shipped
        let config = Config::parse(EXAMPLE).expect("the example does not parse");
        assert_eq!(config.callsign, "N0CALL");
        assert!(matches!(config.ptt, PttConfig::Rigctld { .. }));
    }

    #[test]
    fn every_ptt_backend_can_be_asked_for() {
        let serial = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"serial\"\nport = \"COM3\"\nline = \"dtr\"\n",
        )
        .expect("serial");
        assert_eq!(
            serial.ptt,
            PttConfig::Serial {
                port: "COM3".into(),
                line: SerialLineConfig::Dtr
            }
        );

        let rig = Config::parse("callsign = \"W4ODA\"\n[ptt]\nkind = \"rigctld\"\n").expect("rig");
        assert_eq!(
            rig.ptt,
            PttConfig::Rigctld {
                address: "127.0.0.1:4532".into()
            }
        );
    }

    #[test]
    fn a_typo_is_refused_rather_than_ignored() {
        // the failure this prevents: an operator who believes they set a watchdog and did not
        let error = Config::parse("callsign = \"W4ODA\"\n[radio]\nmax_key_sec = 10.0\n")
            .expect_err("a misspelt key was accepted");
        assert!(matches!(error, ConfigError::Parse(_)), "{error}");
    }

    #[test]
    fn a_callsign_is_required_and_has_to_be_one() {
        assert!(Config::parse("[radio]\nmax_key_s = 10.0\n").is_err());
        assert!(Config::parse("callsign = \"\"").is_err());
        assert!(
            Config::parse("callsign = \"NOT A CALLSIGN!\"").is_err(),
            "a callsign the protocol cannot carry was accepted"
        );
    }

    #[test]
    fn a_sample_rate_the_waveform_cannot_use_is_refused() {
        // nothing resamples, so a device at another rate puts every timing estimate out by
        // the ratio — a failure that would look like a bad band rather than a typo
        let error = Config::parse("callsign = \"W4ODA\"\n[audio]\nsample_rate = 44100\n")
            .expect_err("44.1 kHz was accepted");
        assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
    }

    #[test]
    fn limits_that_would_do_nothing_are_refused() {
        for text in [
            "callsign = \"W4ODA\"\n[radio]\nmax_key_s = 0.0\n",
            "callsign = \"W4ODA\"\n[radio]\nmax_mode = 14\n",
            "callsign = \"W4ODA\"\n[audio]\ntx_level = 0.0\n",
            "callsign = \"W4ODA\"\n[audio]\ntx_level = 2.0\n",
        ] {
            assert!(
                Config::parse(text).is_err(),
                "accepted a setting that cannot work: {text}"
            );
        }
    }

    #[test]
    fn a_control_bind_the_network_can_reach_needs_a_token() {
        let open = "callsign = \"W4ODA\"\n[control]\nbind = \"0.0.0.0:8515\"\n";
        let error = Config::parse(open).expect_err("an open control bind was accepted");
        assert!(matches!(error, ConfigError::Invalid(_)), "{error}");
        assert!(
            error.to_string().contains("127.0.0.1"),
            "the message does not say what to do instead: {error}"
        );

        let with_token =
            "callsign = \"W4ODA\"\n[control]\nbind = \"0.0.0.0:8515\"\ntoken = \"secret\"\n";
        assert!(Config::parse(with_token).is_ok(), "a token should allow it");

        let disabled =
            "callsign = \"W4ODA\"\n[control]\nenabled = false\nbind = \"0.0.0.0:8515\"\n";
        assert!(
            Config::parse(disabled).is_ok(),
            "a bind that is never listened on is not a risk"
        );
    }

    #[test]
    fn the_host_interface_is_off_until_it_is_asked_for() {
        // it lets other software key this radio; that is not a default
        let config = Config::parse("callsign = \"W4ODA\"").expect("parse");
        assert!(!config.host.enabled);
        assert_eq!(config.host.bind, "127.0.0.1:8300");

        let on = Config::parse("callsign = \"W4ODA\"\n[host]\nenabled = true\n").expect("parse");
        assert!(on.host.enabled);
    }

    #[test]
    fn a_configuration_survives_a_round_trip_through_toml() {
        let config = Config::parse(EXAMPLE).expect("parse");
        let text = toml::to_string(&config).expect("serialise");
        assert_eq!(Config::parse(&text).expect("reparse"), config);
    }
}
