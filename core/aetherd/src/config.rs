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
    /// The radio's own command set — CAT — over its serial port.
    ///
    /// The port that carries CAT is usually the one a rig-control program would want too,
    /// and a radio keyed this way needs no second port and no control line wired to
    /// anything. The protocols are the manufacturers' published ones.
    Cat {
        /// Device path: `COM6`, `/dev/ttyUSB0`.
        port: String,
        /// Whose command set the radio speaks.
        protocol: CatProtocol,
        /// The port's speed, as set in the radio's menu (CAT RATE, CI-V baud rate).
        #[serde(default = "default_cat_baud")]
        baud: u32,
        /// Icom only: the radio's CI-V address (0x94 for an IC-7300, 0xA4 for an IC-705,
        /// 0xA2 for an IC-9700 — the rig's menu shows it).
        #[serde(default)]
        civ_address: Option<u8>,
        /// Yaesu only: which modulation input the radio transmits from when keyed this
        /// way — `data` (the USB or DATA jack, which is where the modem's audio is) or
        /// `mic`.
        #[serde(default)]
        source: CatSource,
    },
    /// A CM108-class sound-card interface's GPIO pin — the DRA, URI, RA-40 and most
    /// "USB radio interface" boards built on a C-Media codec. The codec that carries the
    /// audio also keys the radio, so there is no serial port at all.
    Cm108 {
        /// Which interface, when there is more than one: the path `aetherd --list-ports`
        /// prints, or part of the name it prints. Unset means the first one found.
        #[serde(default)]
        device: Option<String>,
        /// The pin wired to PTT, 1–8. The DRA and URI boards use 3.
        #[serde(default = "default_gpio")]
        gpio: u8,
    },
}

fn default_gpio() -> u8 {
    3
}

fn default_rigctld() -> String {
    "127.0.0.1:4532".to_owned()
}

fn default_cat_baud() -> u32 {
    38_400
}

/// A radio's command set, for keying and asking the frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatProtocol {
    /// Yaesu's ASCII CAT (FT-991A, FTDX10, FT-710, FTDX101 and the like): `TX2;` / `TX0;`.
    Yaesu,
    /// Kenwood's, which Elecraft also speaks: `TX;` / `RX;`.
    Kenwood,
    /// Icom's CI-V binary frames: command `1C 00`.
    Icom,
}

/// Which input a Yaesu radio transmits from when keyed over CAT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatSource {
    /// The DATA / USB input, where a modem's audio arrives.
    #[default]
    Data,
    /// The microphone.
    Mic,
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
    /// Transmit level, as a fraction of full scale: the amplitude of a sine with the same RMS
    /// as the data waveform. See `StationConfig::tx_level`.
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
///
/// Each flag is a key of the `[radio]` table as the operator writes it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
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
    /// The waveform's bandwidth in hertz: 2300, the default, or 500 — the bandwidth
    /// peer-to-peer contacts are made in and the only one 30 m allows. Both stations of a
    /// session use the same one; a call in the other bandwidth is not heard.
    #[serde(default = "default_bandwidth")]
    pub bandwidth: u32,
    /// Answer calls but never make one, and never beacon. An automatically controlled
    /// station may use 500 Hz outside the §97.221(b) sub-bands only to *respond* to a
    /// station under local or remote control (§97.221(c)), so this is how an unattended
    /// station is left on such a frequency; `docs/user/frequency-plan.md` says which.
    #[serde(default)]
    pub answer_only: bool,
    /// Fastest mode this station will use; lower it for a rig that cannot manage the dense
    /// constellations, or a band where they never work. Indexes the mode table of the
    /// bandwidth in use: fourteen modes at 2300 Hz, ten at 500 Hz.
    #[serde(default = "default_max_mode")]
    pub max_mode: usize,
    /// Offer payload compression in the connect handshake. Used only if the peer offers it
    /// too, so leaving it on costs nothing when talking to a station that cannot.
    #[serde(default = "default_true")]
    pub compress: bool,
    /// Identify in Morse at the end of a transmission. Off by default: Aether's own frames
    /// carry both callsigns, and whether that satisfies the local rules is something only the
    /// operator knows.
    #[serde(default)]
    pub cw_id: bool,
    /// Morse speed in words per minute.
    #[serde(default = "default_cw_wpm")]
    pub cw_id_wpm: f64,
    /// Longest a station may transmit without identifying, in seconds.
    #[serde(default = "default_cw_interval")]
    pub cw_id_interval_s: f64,
}

fn default_cw_wpm() -> f64 {
    20.0
}

fn default_cw_interval() -> f64 {
    600.0
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

fn default_bandwidth() -> u32 {
    2300
}

impl RadioSection {
    /// The fastest mode the station will use, within the table of its bandwidth: the
    /// configured `max_mode`, clamped to the table's last mode. The default, 13, is the
    /// wide table's last; at 500 Hz it means the narrow table's last, mode 9, so a station
    /// switched to 500 Hz with nothing else touched runs every mode it has.
    #[must_use]
    pub fn fastest_mode(&self) -> usize {
        let modes = self
            .params()
            .map_or(aether_link::AWGN_THRESHOLD_DB.len(), |p| {
                aether_phy::modes::air_interface(p).n_modes()
            });
        self.max_mode.min(modes - 1)
    }

    /// The waveform the bandwidth names, if this version has it.
    #[must_use]
    pub fn params(&self) -> Option<aether_phy::waveform::WaveformParams> {
        match self.bandwidth {
            2300 => Some(aether_phy::waveform::WIDE_2300),
            500 => Some(aether_phy::waveform::NARROW_500),
            _ => None,
        }
    }
}

impl Default for RadioSection {
    fn default() -> Self {
        Self {
            max_key_s: default_max_key(),
            wait_for_clear: default_true(),
            busy_threshold_db: default_busy_threshold(),
            bandwidth: default_bandwidth(),
            answer_only: false,
            max_mode: default_max_mode(),
            compress: default_true(),
            cw_id: false,
            cw_id_wpm: default_cw_wpm(),
            cw_id_interval_s: default_cw_interval(),
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
    /// Directory holding the station panel, served at `/`. A gateway is headless, and a
    /// browser over an SSH tunnel is the only practical way to look at one.
    #[serde(default)]
    pub ui_dir: Option<std::path::PathBuf>,
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
            ui_dir: None,
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
    /// Print every line of the conversation with the host, for the first run against a
    /// client nobody has tried yet.
    #[serde(default)]
    pub trace: bool,
}

fn default_host_bind() -> String {
    "127.0.0.1:8300".to_owned()
}

/// Which releases the desktop application offers to install.
///
/// Read by the shell, not the daemon: it lives here because this file is the one place a
/// station's settings are, and the panel edits it like any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Tagged releases only. Never offered a beta or a nightly.
    #[default]
    Stable,
    /// Betas, and any stable release newer than the beta in hand.
    Beta,
    /// The rolling nightly, and anything newer on the other channels.
    Nightly,
}

/// Automatic updates of the desktop application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSection {
    /// Which channel to follow.
    #[serde(default)]
    pub channel: UpdateChannel,
    /// Whether to look for a newer version when the desktop application starts. Nothing is
    /// installed without asking; this only decides whether the question is asked.
    #[serde(default = "default_true")]
    pub check: bool,
}

impl Default for UpdateSection {
    fn default() -> Self {
        Self {
            channel: UpdateChannel::Stable,
            check: true,
        }
    }
}

/// A simulated channel in place of a sound card.
///
/// Two daemons joined by a socket, with noise at a chosen SNR: how Pat or Winlink Express is
/// driven end to end with no radio, and the bench reference a field session is compared
/// with. With either address set the daemon keys nothing, whatever `[ptt]` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimSection {
    /// Wait for the other daemon here.
    #[serde(default)]
    pub listen: Option<String>,
    /// Connect to the other daemon there.
    #[serde(default)]
    pub connect: Option<String>,
    /// Signal to noise at this receiver, 3 kHz reference, relative to the level the peer
    /// transmits at (its `tx_level`, taken to be the same as this station's).
    #[serde(default = "default_sim_snr")]
    pub snr_db: f64,
}

impl Default for SimSection {
    fn default() -> Self {
        Self {
            listen: None,
            connect: None,
            snr_db: default_sim_snr(),
        }
    }
}

fn default_sim_snr() -> f64 {
    30.0
}

/// Session recordings, for field validation and for finding out what happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecordSection {
    /// Where recordings go. Unset means `recordings/` beside the configuration file.
    #[serde(default)]
    pub dir: Option<std::path::PathBuf>,
    /// Record every session without being asked: from connect to disconnect, one file each.
    #[serde(default)]
    pub auto: bool,
    /// What every automatic recording says about the station — band, antenna, whatever the
    /// operator would have written down had they been at the radio when the call came.
    #[serde(default)]
    pub notes: String,
}

/// Where the daemon's log goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSection {
    /// `text` for a terminal, `json` (one object per line) for a journal or a log shipper.
    #[serde(default)]
    pub format: crate::log::Format,
    /// Also append to this file. Standard output is always written; a packaged desktop
    /// daemon has no terminal, so without a file nothing it says survives the session.
    #[serde(default)]
    pub file: Option<std::path::PathBuf>,
    /// How many recent entries to keep in memory for the `diagnostics` bundle.
    #[serde(default = "default_log_keep")]
    pub keep: usize,
}

impl Default for LogSection {
    fn default() -> Self {
        Self {
            format: crate::log::Format::Text,
            file: None,
            keep: default_log_keep(),
        }
    }
}

fn default_log_keep() -> usize {
    500
}

impl Default for HostSection {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_host_bind(),
            trace: false,
        }
    }
}

/// A whole configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Which shape of file this is. See [`SCHEMA_VERSION`].
    #[serde(default = "first_schema")]
    pub schema_version: u32,
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
    /// The log.
    #[serde(default)]
    pub log: LogSection,
    /// Automatic updates of the desktop application.
    #[serde(default)]
    pub update: UpdateSection,
    /// Session recordings.
    #[serde(default)]
    pub record: RecordSection,
    /// A simulated channel instead of a sound card.
    #[serde(default)]
    pub sim: SimSection,
}

/// The shape of configuration file this version writes.
///
/// A file names its shape so a later version can bring it forward. Adding a key with a
/// default is not a new shape — `serde(default)` handles it. Renaming or moving one is, and
/// gets a migration in [`MIGRATIONS`]: a function from the file as one version wrote it to
/// the file as the next expects it, applied in order on the way in. The operator's file is
/// backed up before it is rewritten, and a file from a *newer* version is refused rather
/// than read with its unknown keys dropped — a downgrade that silently loses settings is
/// worse than one that says so.
pub const SCHEMA_VERSION: u32 = 1;

/// The version a file is when it does not say: the first one shipped.
const fn first_schema() -> u32 {
    1
}

/// One step of bringing a file forward, from version `n` to `n + 1`.
pub type Migration = fn(&mut toml::Table);

/// The steps from the first schema to the current one. `MIGRATIONS[i]` takes a file at
/// version `i + 1` to version `i + 2`; the table is empty while there is only one schema.
pub const MIGRATIONS: &[Migration] = &[];

/// Bring a parsed file forward through `migrations`, starting at `from`.
///
/// Returns the version it ended at. Separated from the file handling so the machinery can
/// be tested with a made-up chain of steps while the real chain is still empty.
fn migrate_with(table: &mut toml::Table, from: u32, migrations: &[Migration]) -> u32 {
    let mut version = from;
    while let Some(step) = usize::try_from(version.saturating_sub(1))
        .ok()
        .and_then(|index| migrations.get(index))
    {
        step(table);
        version += 1;
        table.insert(
            "schema_version".into(),
            toml::Value::Integer(i64::from(version)),
        );
    }
    version
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
        Self::parse_migrating(text).map(|(config, _)| config)
    }

    /// Parse, bringing an older file forward; also says which version the text was.
    fn parse_migrating(text: &str) -> Result<(Self, u32), ConfigError> {
        let mut table: toml::Table =
            toml::from_str(text).map_err(|e| ConfigError::Parse(e.message().to_owned()))?;
        let written_at = match table.get("schema_version") {
            None => first_schema(),
            Some(toml::Value::Integer(n)) => u32::try_from(*n).unwrap_or(u32::MAX),
            Some(other) => {
                return Err(ConfigError::Parse(format!(
                    "schema_version must be a whole number, not {other}"
                )));
            }
        };
        if written_at > SCHEMA_VERSION {
            return Err(ConfigError::Parse(format!(
                "this file was written by a newer aetherd (schema {written_at}; this one \
                 reads up to {SCHEMA_VERSION}). Install that version, or start again from \
                 `aetherd --example-config`"
            )));
        }
        let reached = migrate_with(&mut table, written_at, MIGRATIONS);
        debug_assert_eq!(reached, SCHEMA_VERSION, "the migration chain is incomplete");
        let config: Self = toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| ConfigError::Parse(e.message().to_owned()))?;
        config.validate()?;
        Ok((config, written_at))
    }

    /// Read a configuration file, bringing it forward if an older version wrote it.
    ///
    /// An older file is backed up beside itself as `<name>.bak-v<n>` and rewritten at the
    /// current schema, so the next start reads it without a migration and the operator can
    /// see what changed — and go back, with the backup and the previous version, if they
    /// have to.
    ///
    /// # Errors
    /// If the file cannot be read, or its contents are refused.
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Read(format!("{}: {e}", path.display())))?;
        let (config, written_at) = Self::parse_migrating(&text)?;
        if written_at < SCHEMA_VERSION {
            let backup = backup_path(path, written_at);
            std::fs::write(&backup, &text)
                .map_err(|e| ConfigError::Read(format!("{}: {e}", backup.display())))?;
            config.save(path)?;
        }
        Ok(config)
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
        if let PttConfig::Cat {
            protocol,
            civ_address,
            baud,
            ..
        } = &self.ptt
        {
            if *protocol == CatProtocol::Icom && civ_address.is_none() {
                return Err(ConfigError::Invalid(
                    "[ptt] protocol = \"icom\" needs civ_address, the radio's CI-V address \
                     (0x94 for an IC-7300, 0xA4 for an IC-705, 0xA2 for an IC-9700)"
                        .into(),
                ));
            }
            if *baud == 0 {
                return Err(ConfigError::Invalid(
                    "[ptt] baud must be a serial rate".into(),
                ));
            }
        }
        if let PttConfig::Cm108 { gpio, .. } = &self.ptt
            && !(1..=8).contains(gpio)
        {
            return Err(ConfigError::Invalid(format!(
                "[ptt] gpio must be 1–8, not {gpio}; the DRA and URI boards key on 3"
            )));
        }
        if self.sim.listen.is_some() && self.sim.connect.is_some() {
            return Err(ConfigError::Invalid(
                "[sim] listen and connect are alternatives; set one of them".into(),
            ));
        }
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
        if self.radio.cw_id && !(5.0..=40.0).contains(&self.radio.cw_id_wpm) {
            return Err(ConfigError::Invalid(format!(
                "cw_id_wpm is {}; below 5 an identifier takes longer than the transmission \
                 and above 40 very few people can copy it",
                self.radio.cw_id_wpm
            )));
        }
        if self.radio.max_key_s <= 0.0 {
            return Err(ConfigError::Invalid(
                "max_key_s must be positive: a watchdog that can never fire is not one".into(),
            ));
        }
        if self.radio.params().is_none() {
            return Err(ConfigError::Invalid(format!(
                "bandwidth must be 2300 or 500, not {}: those are the waveforms this version \
                 has",
                self.radio.bandwidth
            )));
        }
        // the widest table's size; a narrower table clamps (`RadioSection::fastest_mode`)
        // rather than refuses, so `bandwidth = 500` with everything else left alone works
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
    ///
    /// The panel directory comes from the file if it says; otherwise from `AETHER_UI_DIR`,
    /// which is how the desktop shell points a daemon it started at the panel it ships with;
    /// otherwise a `ui` directory beside the binary, which is how a package lays it out. A
    /// daemon that cannot find one still runs — it just answers `/` with a 404.
    #[must_use]
    pub fn control_config(&self) -> crate::control::ControlConfig {
        let ui_dir = self
            .control
            .ui_dir
            .clone()
            .or_else(|| std::env::var_os("AETHER_UI_DIR").map(std::path::PathBuf::from))
            .or_else(|| {
                let beside = std::env::current_exe().ok()?.parent()?.join("ui");
                beside.is_dir().then_some(beside)
            });
        crate::control::ControlConfig {
            bind: self.control.bind.clone(),
            token: self.control.token.clone(),
            ui_dir,
        }
    }

    /// Host-interface settings in the form the adapter wants.
    #[must_use]
    pub fn host_config(&self) -> crate::host::HostConfig {
        crate::host::HostConfig {
            enabled: self.host.enabled,
            bind: self.host.bind.clone(),
            trace: self.host.trace,
        }
    }

    /// The simulated channel, if the file asks for one.
    #[must_use]
    pub fn sim_config(&self) -> Option<crate::sim::SimConfig> {
        let peer = match (&self.sim.listen, &self.sim.connect) {
            (Some(address), _) => crate::sim::Peer::Listen(address.clone()),
            (None, Some(address)) => crate::sim::Peer::Connect(address.clone()),
            (None, None) => return None,
        };
        Some(crate::sim::SimConfig {
            peer,
            snr_db: self.sim.snr_db,
            // the waveform's RMS is the level over root two: see `StationConfig::tx_level`
            signal_rms: self.audio.tx_level / std::f64::consts::SQRT_2,
            sample_rate: self.audio.sample_rate,
        })
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

/// Settings that take effect without restarting the daemon.
///
/// Everything else needs one: a sound card is opened once, and a control socket is bound
/// once. Saying which is which is the difference between a setting that appears to work and
/// one that does.
pub const LIVE_KEYS: &[&str] = &[
    "audio.tx_level",
    "radio.max_key_s",
    "radio.wait_for_clear",
    "radio.max_mode",
    "radio.answer_only",
    "radio.busy_threshold_db",
    "record.auto",
    "record.notes",
];

/// Whether two JSON values say the same thing, with `20` and `20.0` counting as the same:
/// the panel sends whole numbers as integers and the file round-trips them as floats, and a
/// "change" between the two once restarted the modem on every save.
fn same_value(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        // exact on purpose: the question is whether the same number was written twice
        (Some(x), Some(y)) => x.to_bits() == y.to_bits() || (x - y).abs() < f64::EPSILON,
        _ => a == b,
    }
}

impl Config {
    /// Merge a JSON object of dotted keys into this configuration.
    ///
    /// Returns the keys that were changed. The result is validated before it is returned, so
    /// a caller never receives a configuration it cannot run.
    ///
    /// # Errors
    /// If a key is not one this version defines, a value is the wrong type, or the merged
    /// configuration would not work.
    pub fn merge(&mut self, changes: &serde_json::Value) -> Result<Vec<String>, ConfigError> {
        let Some(object) = changes.as_object() else {
            return Err(ConfigError::Invalid(
                "changes must be an object of dotted keys, like {\"radio.max_mode\": 8}".into(),
            ));
        };

        // Round-trip through the serialised form so the merge works on exactly the shape the
        // file has, and so an unknown key is refused by the same `deny_unknown_fields` that
        // refuses one in the file.
        let mut document = serde_json::to_value(&*self)
            .map_err(|e| ConfigError::Invalid(format!("cannot read the current settings: {e}")))?;

        let mut changed = Vec::new();
        for (key, value) in object {
            let parts: Vec<&str> = key.split('.').collect();
            // `split` always yields at least one part, even for an empty key, so this
            // cannot fail; the empty name is then refused by `deny_unknown_fields` below.
            let Some((last, path)) = parts.split_last() else {
                return Err(ConfigError::Invalid(format!("{key} is not a setting")));
            };
            let mut cursor = &mut document;
            for part in path {
                let Some(map) = cursor.as_object_mut() else {
                    return Err(ConfigError::Invalid(format!("{key} is not a setting")));
                };
                cursor = map
                    .entry((*part).to_owned())
                    .or_insert_with(|| serde_json::json!({}));
            }
            let Some(map) = cursor.as_object_mut() else {
                return Err(ConfigError::Invalid(format!("{key} is not a setting")));
            };
            // Only a value that differs is a change. The panel sends its whole form, and a
            // save that merely repeated the sound card's name used to count as a change to
            // it — and, once a change to it meant a restart, restarted the modem for
            // nothing every time a level or a note was saved.
            let before = map.insert((*last).to_owned(), value.clone());
            if !before.as_ref().is_some_and(|old| same_value(old, value)) {
                changed.push(key.clone());
            }
        }

        // A change of keying kind takes the old kind's fields with it: `port` and `line`
        // belong to a serial port and `address` to rigctld, the tagged enum refuses a mix,
        // and a client cannot remove a key — it can only say which kind it wants now.
        if object.contains_key("ptt.kind")
            && let Some(ptt) = document
                .get_mut("ptt")
                .and_then(serde_json::Value::as_object_mut)
        {
            ptt.retain(|field, _| field == "kind" || object.contains_key(&format!("ptt.{field}")));
        }

        let merged: Self =
            serde_json::from_value(document).map_err(|e| ConfigError::Parse(format!("{e}")))?;
        merged.validate()?;
        *self = merged;
        changed.sort();
        Ok(changed)
    }

    /// Whether a change to this key takes effect without a restart.
    #[must_use]
    pub fn is_live(key: &str) -> bool {
        LIVE_KEYS.contains(&key)
    }

    /// Write this configuration to a file, atomically.
    ///
    /// Written beside the target and then renamed over it: a configuration half-written by a
    /// machine that lost power is a station that will not start, and the operator would have
    /// no way to know what it used to say.
    ///
    /// # Errors
    /// If the configuration cannot be serialised or the file cannot be replaced.
    pub fn save(&self, path: &std::path::Path) -> Result<(), ConfigError> {
        self.validate()?;
        let text = toml::to_string_pretty(self)
            .map_err(|e| ConfigError::Invalid(format!("cannot write these settings: {e}")))?;
        let temporary = path.with_extension("toml.new");
        std::fs::write(&temporary, text)
            .map_err(|e| ConfigError::Read(format!("{}: {e}", temporary.display())))?;
        std::fs::rename(&temporary, path)
            .map_err(|e| ConfigError::Read(format!("{}: {e}", path.display())))?;
        Ok(())
    }
}

/// Where the copy of an older file goes before it is rewritten.
fn backup_path(path: &std::path::Path, written_at: u32) -> std::path::PathBuf {
    let name = path.file_name().map_or_else(
        || "station.toml".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    path.with_file_name(format!("{name}.bak-v{written_at}"))
}

/// An example file, for `aetherd --example-config`.
pub const EXAMPLE: &str = r#"# Aether HF station configuration.
#
# Only `callsign` has no default. A file with nothing else in it starts a receive-only
# station on the default sound card, which is a good way to listen before transmitting.

# The shape of this file. Leave it: a newer aetherd uses it to bring the file forward.
schema_version = 1

callsign = "N0CALL"

[audio]
# Names as `aetherd --list-devices` prints them. Unset means the system default.
# input = "USB Audio CODEC"
# output = "USB Audio CODEC"
sample_rate = 48000
# Transmit level as a fraction of full scale: the amplitude of a sine with the same RMS as
# the data waveform (which is what Tune plays). The audio's RMS is this over root two —
# 0.25 is -15 dBFS RMS, peaks around -6 dBFS. Leave headroom: the waveform has peaks.
tx_level = 0.25

[ptt]
# kind = "none"                       # VOX, or receive only
# kind = "serial"                     # a serial control line
# port = "COM3"                       # `aetherd --list-ports` shows what is there
# line = "rts"                        # rts | dtr | both
# kind = "cat"                        # the radio's own commands over its CAT port
# port = "COM6"
# protocol = "yaesu"                  # yaesu | kenwood | icom
# baud = 38400                        # the rate set in the radio's menu
# civ_address = 148                   # icom only: the CI-V address (0x94 = 148)
# source = "data"                     # yaesu only: transmit from data | mic
# kind = "cm108"                      # a DRA, URI or other CM108-class interface's GPIO pin
# device = "..."                      # which one, when there are several: its path or name
# gpio = 3                            # the pin wired to PTT (3 on the DRA and URI boards)
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
# The waveform: 2300 Hz, or 500 Hz — the bandwidth peer-to-peer contacts are made in and
# the only one 30 m allows. Both stations of a session use the same one.
bandwidth = 2300
# Answer calls but never make one, and never beacon: how an unattended station is left on
# a 500 Hz frequency outside the automatic sub-bands (§97.221(c)).
answer_only = false
# The fastest mode this station will use: 0 to 13 at 2300 Hz; at 500 Hz the table has ten
# modes and anything past 9 means 9.
max_mode = 13
# Offer payload compression. Used only if the other station offers it too, so leaving this on
# costs nothing when talking to one that cannot.
compress = true
# Identify in Morse at the end of a transmission. Off by default: Aether's frames carry both
# callsigns, and whether that satisfies your licence conditions is your call, not the modem's.
cw_id = false
cw_id_wpm = 20.0
cw_id_interval_s = 600.0

[control]
# The modem's own interface: JSON over WebSocket at ws://<bind>/v1, or POST /v1/<method>.
enabled = true
# Loopback needs no token. Any other address does, and the daemon refuses to start without
# one rather than leaving a transmitter open to the network.
bind = "127.0.0.1:8515"
# token = "a long random string"
# The station panel, served at http://<bind>/ so a headless gateway can be watched from a
# browser over an SSH tunnel. Point it at the `app/ui` directory of a checkout, or at wherever
# your package installed it.
# ui_dir = "/usr/share/aetherd/ui"

[host]
# The VARA-compatible host interface, so Winlink Express, Pat, VarAC and BPQ32 can use this
# station. Off unless asked for. The data port is the command port plus one, and both have to
# be free. `VERSION` answers with Aether's name, not VARA's — see docs/spec/host-interfaces.md.
enabled = false
bind = "127.0.0.1:8300"
# Print every line exchanged with the host program, for the first run against a new one.
trace = false

[log]
# `text` for a terminal; `json` writes one object per line for a journal or a log shipper.
format = "text"
# Also append to a file. A desktop shell has no terminal, so this is where its story goes.
# file = "aetherd.log"
# How many recent entries the `diagnostics` bundle carries.
keep = 500

[update]
# The desktop application looks for a newer version when it starts and asks before
# installing one. `stable` is tagged releases only; `beta` adds the betas; `nightly` adds
# the nightly build. A headless gateway ignores this section.
channel = "stable"
check = true

[record]
# Session recordings: a 48 kHz WAV of what the radio delivered and a JSON file saying what
# the modem made of it — every frame, its SNR, whether it decoded. `record.start` on the
# control API (or the panel's Record button) starts one; `auto = true` records every session
# on its own, which is what a gateway and field validation want.
# dir = "recordings"                  # default: recordings/ beside this file
auto = false
# What every automatic recording says about the station: the band, the antenna, the
# frequency when there is no rig control to ask (with [ptt] kind = "rigctld" the frequency
# is asked for and recorded on its own).
notes = ""

[sim]
# A simulated channel instead of a sound card: two daemons joined by a socket, with noise
# at this SNR (3 kHz reference, relative to the other end's tx_level). One end listens and
# the other connects; a daemon with either set keys nothing. This is how Pat or Winlink
# Express is tried end to end with no radio.
# listen = "127.0.0.1:8600"
# connect = "127.0.0.1:8600"
snr_db = 30.0
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

        let gpio = Config::parse("callsign = \"W4ODA\"\n[ptt]\nkind = \"cm108\"\n").expect("cm108");
        assert_eq!(
            gpio.ptt,
            PttConfig::Cm108 {
                device: None,
                gpio: 3
            }
        );
        let named = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"cm108\"\ndevice = \"DRA-36\"\ngpio = 4\n",
        )
        .expect("cm108 named");
        assert!(matches!(named.ptt, PttConfig::Cm108 { gpio: 4, .. }));
        assert!(
            Config::parse("callsign = \"W4ODA\"\n[ptt]\nkind = \"cm108\"\ngpio = 9\n").is_err(),
            "a pin the codecs do not have"
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
            // 2750 Hz is not a waveform yet
            "callsign = \"W4ODA\"\n[radio]\nbandwidth = 2750\n",
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
    fn morse_identification_is_off_by_default_and_its_speed_is_checked() {
        let plain = Config::parse("callsign = \"W4ODA\"").expect("parse");
        assert!(!plain.radio.cw_id, "it identified without being asked to");

        let on = Config::parse("callsign = \"W4ODA\"\n[radio]\ncw_id = true\n").expect("parse");
        assert!(on.radio.cw_id);

        for bad in ["cw_id_wpm = 1.0", "cw_id_wpm = 200.0"] {
            let text = format!("callsign = \"W4ODA\"\n[radio]\ncw_id = true\n{bad}\n");
            assert!(Config::parse(&text).is_err(), "accepted {bad}");
        }
        // the speed is only checked when it will actually be used
        let unused = "callsign = \"W4ODA\"\n[radio]\ncw_id_wpm = 200.0\n";
        assert!(Config::parse(unused).is_ok());
    }

    #[test]
    fn a_setting_can_be_changed_by_its_dotted_name() {
        let mut config = Config::parse("callsign = \"W4ODA\"").expect("parse");
        let changed = config
            .merge(&serde_json::json!({"radio.max_mode": 8, "audio.tx_level": 0.3}))
            .expect("merge");
        assert_eq!(changed, vec!["audio.tx_level", "radio.max_mode"]);
        assert_eq!(config.radio.max_mode, 8);
        assert!((config.audio.tx_level - 0.3).abs() < 1e-12);
        // everything not named is left alone
        assert_eq!(config.callsign, "W4ODA");
        assert!(config.radio.wait_for_clear);
    }

    #[test]
    fn changing_the_keying_kind_leaves_the_old_kind_behind() {
        // the panel says `ptt.kind = "rigctld"` with an address; the serial port and line
        // of the kind before must not survive to make the file invalid
        let mut config = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"serial\"\nport = \"COM7\"\nline = \"dtr\"\n",
        )
        .expect("parse");
        config
            .merge(&serde_json::json!({"ptt.kind": "rigctld", "ptt.address": "127.0.0.1:4532"}))
            .expect("rigctld");
        assert_eq!(
            config.ptt,
            PttConfig::Rigctld {
                address: "127.0.0.1:4532".into()
            }
        );
        config
            .merge(&serde_json::json!({"ptt.kind": "serial", "ptt.port": "COM3"}))
            .expect("serial again");
        assert_eq!(
            config.ptt,
            PttConfig::Serial {
                port: "COM3".into(),
                line: SerialLineConfig::Rts
            }
        );
        config
            .merge(&serde_json::json!({"ptt.kind": "none"}))
            .expect("none");
        assert_eq!(config.ptt, PttConfig::None);
    }

    #[test]
    fn cat_keying_is_read_and_checked() {
        let config = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"cat\"\nport = \"COM6\"\nprotocol = \"yaesu\"\n",
        )
        .expect("yaesu");
        assert_eq!(
            config.ptt,
            PttConfig::Cat {
                port: "COM6".into(),
                protocol: CatProtocol::Yaesu,
                baud: 38_400,
                civ_address: None,
                source: CatSource::Data,
            }
        );
        // an Icom without its address cannot be spoken to
        let error = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"cat\"\nport = \"COM4\"\nprotocol = \"icom\"\n",
        )
        .expect_err("icom without an address");
        assert!(error.to_string().contains("civ_address"), "{error}");
        let config = Config::parse(
            "callsign = \"W4ODA\"\n[ptt]\nkind = \"cat\"\nport = \"COM4\"\nprotocol = \"icom\"\nciv_address = 148\nbaud = 19200\n",
        )
        .expect("icom");
        assert!(matches!(
            config.ptt,
            PttConfig::Cat {
                protocol: CatProtocol::Icom,
                civ_address: Some(0x94),
                baud: 19_200,
                ..
            }
        ));
    }

    #[test]
    fn repeating_a_setting_is_not_a_change() {
        // the panel sends its whole form on every save; only what differs is reported, or
        // a restart-needing key that did not move would restart the modem for nothing
        let mut config = Config::parse(EXAMPLE).expect("example");
        let same = config.audio.input.clone();
        let changed = config
            .merge(&serde_json::json!({
                "audio.input": same,
                "callsign": config.callsign.clone(),
                "radio.max_mode": 8,
            }))
            .expect("merge");
        assert_eq!(changed, vec!["radio.max_mode"]);
        // a whole number sent as an integer is the float the file holds
        let changed = config
            .merge(&serde_json::json!({"radio.cw_id_wpm": 20, "radio.max_key_s": 30}))
            .expect("merge");
        assert!(changed.is_empty(), "{changed:?}");
    }

    #[test]
    fn a_change_that_would_not_work_is_refused_and_changes_nothing() {
        // the station has to be left running on what it had; a half-applied configuration is
        // worse than a refused one
        let mut config = Config::parse("callsign = \"W4ODA\"").expect("parse");
        let before = config.clone();
        for bad in [
            serde_json::json!({"radio.max_mode": 99}),
            serde_json::json!({"audio.sample_rate": 44100}),
            serde_json::json!({"callsign": "NOT A CALL!"}),
            serde_json::json!({"radio.max_key_s": 0}),
            serde_json::json!({"radio.no_such_setting": 1}),
            serde_json::json!({"radio.max_mode": "eight"}),
            serde_json::json!("not an object"),
        ] {
            assert!(config.merge(&bad).is_err(), "accepted {bad}");
            assert_eq!(config, before, "a refused change was partly applied: {bad}");
        }
    }

    #[test]
    fn which_settings_need_a_restart_is_stated_rather_than_guessed() {
        // a sound card is opened once and a socket is bound once; a setting that silently
        // does nothing until the next restart is worse than one that says so
        assert!(Config::is_live("radio.max_mode"));
        assert!(Config::is_live("radio.wait_for_clear"));
        assert!(Config::is_live("radio.answer_only"));
        // the waveform is the modem: a change to it is a restart
        assert!(!Config::is_live("radio.bandwidth"));
        // a station switched to 500 Hz with nothing else touched runs every narrow mode:
        // the wide default of 13 clamps to the narrow table's last, 12
        let narrow = Config::parse("callsign = \"W4ODA\"\n[radio]\nbandwidth = 500\n")
            .expect("a narrow station");
        assert_eq!(
            narrow.radio.params(),
            Some(aether_phy::waveform::NARROW_500)
        );
        assert_eq!(narrow.radio.max_mode, 13);
        assert_eq!(narrow.radio.fastest_mode(), 12);
        let wide =
            Config::parse("callsign = \"W4ODA\"\n[radio]\nmax_mode = 8\n").expect("a wide station");
        assert_eq!(wide.radio.fastest_mode(), 8);
        assert!(!Config::is_live("audio.input"));
        assert!(!Config::is_live("ptt.port"));
        assert!(!Config::is_live("control.bind"));
    }

    #[test]
    fn saving_and_reloading_gives_back_the_same_configuration() {
        let mut config = Config::parse(EXAMPLE).expect("parse");
        config
            .merge(&serde_json::json!({"callsign": "W4ODA", "radio.max_mode": 9}))
            .expect("merge");

        let dir = std::env::temp_dir().join(format!("aether-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("station.toml");
        config.save(&path).expect("save");

        let reloaded = Config::load(&path).expect("reload");
        assert_eq!(reloaded, config);
        assert!(
            !path.with_extension("toml.new").exists(),
            "the temporary file was left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_does_not_say_its_schema_is_the_first_one() {
        let config = Config::parse("callsign = \"W4ODA\"").expect("parse");
        assert_eq!(config.schema_version, 1);
        // and it is written out saying so, so the next version can tell
        let text = toml::to_string_pretty(&config).expect("serialise");
        assert!(text.starts_with("schema_version = 1\n"), "{text}");
    }

    #[test]
    fn a_file_from_a_newer_version_is_refused_not_misread() {
        let error = Config::parse("schema_version = 99\ncallsign = \"W4ODA\"").unwrap_err();
        let message = error.to_string();
        assert!(message.contains("newer aetherd"), "{message}");
        assert!(message.contains("schema 99"), "{message}");
    }

    #[test]
    fn migrations_run_in_order_from_where_the_file_is() {
        // a made-up chain: v1 -> v2 renames a key, v2 -> v3 moves it into a table
        fn one_to_two(table: &mut toml::Table) {
            if let Some(value) = table.remove("call") {
                table.insert("callsign".into(), value);
            }
        }
        fn two_to_three(table: &mut toml::Table) {
            if let Some(level) = table.remove("tx_level") {
                let mut audio = toml::Table::new();
                audio.insert("tx_level".into(), level);
                table.insert("audio".into(), toml::Value::Table(audio));
            }
        }
        let chain: &[Migration] = &[one_to_two, two_to_three];

        let mut table: toml::Table =
            toml::from_str("call = \"W4ODA\"\ntx_level = 0.3\n").expect("toml");
        assert_eq!(migrate_with(&mut table, 1, chain), 3);
        assert_eq!(table["schema_version"], toml::Value::Integer(3));
        let config: Config = toml::Value::Table(table)
            .try_into()
            .expect("a current file");
        assert_eq!(config.callsign, "W4ODA");
        assert!((config.audio.tx_level - 0.3).abs() < 1e-9);

        // a file already at v2 only takes the second step
        let mut table: toml::Table =
            toml::from_str("schema_version = 2\ncallsign = \"W4ODA\"\ntx_level = 0.3\n")
                .expect("toml");
        assert_eq!(migrate_with(&mut table, 2, chain), 3);
        assert!(table.get("tx_level").is_none());

        // and the real chain, applied to a current file, changes nothing
        let mut table: toml::Table = toml::from_str(EXAMPLE).expect("toml");
        let before = table.clone();
        assert_eq!(migrate_with(&mut table, 1, MIGRATIONS), SCHEMA_VERSION);
        assert_eq!(table, before);
    }

    #[test]
    fn every_file_a_released_version_wrote_still_loads_with_nothing_lost() {
        fn leaves(prefix: &str, table: &toml::Table, out: &mut Vec<(String, toml::Value)>) {
            for (key, value) in table {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match value {
                    toml::Value::Table(inner) => leaves(&name, inner, out),
                    other => out.push((name, other.clone())),
                }
            }
        }
        // `tests/data/config/` holds a file as each released version wrote it. A release
        // adds its own; this test is the promise that an upgrade keeps the operator's
        // settings. The check is that loading and saving keeps every key and value the
        // file had — a migration may move a key, but it may not drop one.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/config");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("fixture directory") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            seen += 1;
            let text = std::fs::read_to_string(&path).expect("read fixture");
            let config = Config::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let written: toml::Table = toml::from_str(&text).expect("fixture toml");
            let saved: toml::Table =
                toml::from_str(&toml::to_string_pretty(&config).expect("write"))
                    .expect("saved toml");
            // every leaf the operator wrote is still there, with the same value, unless a
            // migration moved it — and the real chain has no moves yet
            let mut wrote = Vec::new();
            leaves("", &written, &mut wrote);
            let mut kept = Vec::new();
            leaves("", &saved, &mut kept);
            for (name, value) in &wrote {
                let found = kept.iter().find(|(n, _)| n == name);
                assert!(
                    found.is_some_and(|(_, v)| v == value),
                    "{}: {name} = {value} was lost or changed on load",
                    path.display()
                );
            }
        }
        assert!(seen > 0, "no fixtures in {}", dir.display());
    }

    #[test]
    fn an_older_file_is_backed_up_and_rewritten_when_loaded() {
        // with one schema there is no older file to load; what can be checked is that a
        // current file is loaded without being touched, and where a backup would go
        let dir = std::env::temp_dir().join(format!("aether-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("station.toml");
        std::fs::write(&path, "callsign = \"W4ODA\"\n").expect("write");
        let config = Config::load(&path).expect("load");
        assert_eq!(config.callsign, "W4ODA");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "callsign = \"W4ODA\"\n",
            "a current file must not be rewritten on load"
        );
        assert_eq!(
            backup_path(&path, 1).file_name().and_then(|n| n.to_str()),
            Some("station.toml.bak-v1")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_configuration_survives_a_round_trip_through_toml() {
        let config = Config::parse(EXAMPLE).expect("parse");
        let text = toml::to_string(&config).expect("serialise");
        assert_eq!(Config::parse(&text).expect("reparse"), config);
    }
}
