//! Profiles: a station's settings as one portable file.
//!
//! The configuration file is what the daemon runs from and stays so. A profile is the
//! *portable* part of it — everything the settings registry ([`crate::settings`]) does not
//! mark as this computer's wiring or a secret — as JSON under a name, with the dial
//! memories beside it, so an operator can keep one per radio or per place, go back to one,
//! and carry one to another computer. Nothing here is a list of fields: a profile is the
//! configuration projected through the registry, so a setting added to the daemon is in
//! the next profile saved without anyone naming it.
//!
//! Loading is transactional. The file is parsed, brought forward through the same
//! migration chain as the configuration file, and applied to a *copy* of the running
//! configuration one setting at a time: a key this version does not have is reported and
//! left out, a value that fails its rule is reported and the default kept, a device this
//! computer does not have is reported and left as named (the daemon already runs
//! receive-only or silent on a device it cannot open, and says so). The whole is validated
//! as the configuration file would be, and only then written — atomically, as the
//! configuration always is.
//!
//! The profiles live in `profiles/` beside the configuration, one `.aetherprofile` each;
//! `profiles.json` beside them says which one is active. A daemon that finds neither adopts
//! the running configuration as a profile called *Default*, so an upgrade changes nothing
//! the operator can see.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    config::{Config, ConfigError, PttConfig},
    memories::{Memories, Memory},
    settings,
};

/// What the `format` field of a profile says.
pub const FORMAT: &str = "aether-hf-profile";
/// The shape of the profile file this version writes — the envelope around the settings.
/// The settings inside carry the configuration file's own schema number and go through its
/// migrations; this number is for the envelope's fields.
pub const SCHEMA: u32 = 1;
/// The file extension.
pub const EXTENSION: &str = "aetherprofile";
/// The most characters a name may have.
pub const NAME_CHARS: usize = 60;
/// The name the running configuration is adopted under on the first start with profiles.
pub const ADOPTED_NAME: &str = "Default";

/// One step of bringing an envelope forward, from version `n` to `n + 1`.
pub type Migration = fn(&mut Map<String, Value>);

/// The envelope's migrations, in order; empty while there is one shape.
pub const MIGRATIONS: &[Migration] = &[];

/// Where a serial port or a codec came from, so that another computer's can be told apart
/// by what is behind it rather than by the number the operating system gave it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceHint {
    /// The name the configuration holds: `COM7`, a HID path.
    #[serde(default)]
    pub name: String,
    /// What the driver said was behind it when the profile was saved.
    #[serde(default)]
    pub description: String,
}

/// What the profile remembers about the hardware beyond its names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hardware {
    /// The keying port's description, for a serial or CAT port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptt_port: Option<DeviceHint>,
    /// The CM108-class interface's name, for GPIO keying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ptt_device: Option<DeviceHint>,
}

/// A profile, as the file holds it.
///
/// Parsed leniently: a field this version does not know is ignored, and one it expects but
/// does not find takes its default — a profile is meant to survive the versions on either
/// side of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// [`FORMAT`].
    pub format: String,
    /// [`SCHEMA`], as written.
    pub schema: u32,
    /// The configuration schema the settings are in, as written.
    #[serde(default = "first_settings_schema")]
    pub settings_schema: u32,
    /// The version of the daemon that wrote it.
    #[serde(default)]
    pub aether_version: String,
    /// The operator's name for it.
    #[serde(default)]
    pub name: String,
    /// When it was first saved, RFC 3339.
    #[serde(default)]
    pub created: String,
    /// When it was last saved, RFC 3339.
    #[serde(default)]
    pub modified: String,
    /// The portable settings, in the configuration's own shape.
    pub settings: Value,
    /// What is known about the hardware beyond its names.
    #[serde(default)]
    pub hardware: Hardware,
    /// The remembered dials, when the profile carries them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memories: Option<Vec<Memory>>,
}

const fn first_settings_schema() -> u32 {
    crate::config::first_schema()
}

/// Why a profile could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileError {
    /// The text is not a profile at all.
    NotAProfile(String),
    /// A newer version wrote it and this one cannot read it faithfully.
    Newer(String),
    /// The settings as a whole would not run: what the configuration refused.
    Invalid(String),
    /// A file could not be read or written.
    Io(String),
    /// No profile has that id.
    NoSuchProfile(String),
    /// A profile already has that name.
    NameTaken(String),
    /// The name is empty or too long.
    BadName(String),
    /// There is nowhere to keep profiles: the daemon runs without a configuration file.
    NoDirectory,
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAProfile(detail) => write!(f, "this is not an Aether HF profile: {detail}"),
            Self::Invalid(detail) => write!(f, "the profile cannot be applied: {detail}"),
            Self::Newer(detail) | Self::Io(detail) | Self::BadName(detail) => write!(f, "{detail}"),
            Self::NoSuchProfile(id) => write!(f, "there is no profile called {id:?}"),
            Self::NameTaken(name) => write!(f, "a profile called {name:?} already exists"),
            Self::NoDirectory => write!(
                f,
                "this daemon was started without a configuration file, so it has nowhere to \
                 keep profiles"
            ),
        }
    }
}

impl core::error::Error for ProfileError {}

/// What this machine has, for checking a profile's devices against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    /// Capture device names.
    pub inputs: Vec<String>,
    /// Playback device names.
    pub outputs: Vec<String>,
    /// Serial ports, as name and description.
    pub serial_ports: Vec<DeviceHint>,
    /// CM108-class interfaces, as path and name.
    pub gpio_interfaces: Vec<DeviceHint>,
}

impl Inventory {
    /// From the object `devices.list` answers with. An inventory that could not be taken
    /// (the object says `error`) is empty, and an empty inventory checks nothing.
    #[must_use]
    pub fn from_json(value: &Value) -> Self {
        let names = |key: &str, flag: &str| -> Vec<String> {
            value[key]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter(|d| d[flag].as_bool().unwrap_or(false))
                        .filter_map(|d| d["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let hints = |key: &str, name: &str, description: &str| -> Vec<DeviceHint> {
            value[key]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|d| {
                            Some(DeviceHint {
                                name: d[name].as_str()?.to_owned(),
                                description: d[description].as_str().unwrap_or("").to_owned(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Self {
            inputs: names("devices", "input"),
            outputs: names("devices", "output"),
            serial_ports: hints("serial_ports", "name", "description"),
            gpio_interfaces: hints("gpio_interfaces", "path", "name"),
        }
    }

    /// Whether anything at all was reported: an empty inventory is a machine that could
    /// not be asked, and a check against it would call every device missing.
    fn is_empty(&self) -> bool {
        self.inputs.is_empty()
            && self.outputs.is_empty()
            && self.serial_ports.is_empty()
            && self.gpio_interfaces.is_empty()
    }
}

/// A device the profile names and this computer does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingHardware {
    /// Which setting: `audio.input`, `ptt.port`.
    pub key: String,
    /// What the profile named.
    pub name: String,
    /// A device of this computer with the same description, when there is exactly one —
    /// offered, never chosen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// A value the profile carried that was not taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Rejected {
    /// Which setting.
    pub key: String,
    /// What the profile said.
    pub value: Value,
    /// Why it was not taken.
    pub reason: String,
}

/// What applying a profile to a configuration produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The configuration to run: the base's machine-local settings, the profile's for the
    /// rest, validated.
    pub config: Config,
    /// The dial memories to keep, when the profile carried a usable list.
    pub memories: Option<Vec<Memory>>,
    /// The dotted keys whose value the profile changed, sorted.
    pub changed: Vec<String>,
    /// Settings this version does not have — from a newer version, or a typo.
    pub unknown: Vec<String>,
    /// Settings the file carried that a profile does not: this computer's were kept.
    pub ignored: Vec<String>,
    /// Values that failed their rule; the default was kept for each.
    pub invalid: Vec<Rejected>,
    /// Devices this computer does not have.
    pub missing_hardware: Vec<MissingHardware>,
}

impl Applied {
    /// The changed keys that take effect only after a restart.
    #[must_use]
    pub fn restart_required(&self) -> Vec<String> {
        self.changed
            .iter()
            .filter(|key| !Config::is_live(key))
            .cloned()
            .collect()
    }
}

/// The portable part of a configuration, in its own shape.
#[must_use]
pub fn portable(config: &Config) -> Value {
    let document = serde_json::to_value(config).unwrap_or(Value::Null);
    let mut all = Vec::new();
    settings::leaves("", &document, &mut all);
    let mut out = Value::Object(Map::new());
    for (key, value) in all {
        if settings::is_portable(&key) {
            settings::set_leaf(&mut out, &key, value);
        }
    }
    out
}

/// Milliseconds since the Unix epoch, now.
fn unix_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Now, RFC 3339.
#[must_use]
pub fn now() -> String {
    crate::log::rfc3339(unix_ms_now())
}

/// Take every `null` out of a JSON tree: TOML has no null, and a null setting is an unset
/// one, which is the same as an absent one.
fn strip_nulls(value: &mut Value) {
    if let Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
        for inner in map.values_mut() {
            strip_nulls(inner);
        }
    }
}

impl Profile {
    /// A profile of a configuration as it stands.
    ///
    /// The inventory, when given, is where the keying port's description comes from — what
    /// the driver says is behind it — so a load on another computer can suggest the port
    /// with the same thing behind it.
    #[must_use]
    pub fn capture(
        name: &str,
        config: &Config,
        memories: &[Memory],
        inventory: Option<&Inventory>,
        now: &str,
    ) -> Self {
        let mut hardware = Hardware::default();
        match &config.ptt {
            PttConfig::Serial { port, .. } | PttConfig::Cat { port, .. } => {
                let description = inventory
                    .and_then(|i| i.serial_ports.iter().find(|p| &p.name == port))
                    .map(|p| p.description.clone())
                    .unwrap_or_default();
                hardware.ptt_port = Some(DeviceHint {
                    name: port.clone(),
                    description,
                });
            }
            PttConfig::Cm108 {
                device: Some(path), ..
            } => {
                let description = inventory
                    .and_then(|i| i.gpio_interfaces.iter().find(|p| &p.name == path))
                    .map(|p| p.description.clone())
                    .unwrap_or_default();
                hardware.ptt_device = Some(DeviceHint {
                    name: path.clone(),
                    description,
                });
            }
            PttConfig::None | PttConfig::Rigctld { .. } | PttConfig::Cm108 { device: None, .. } => {
            }
        }
        Self {
            format: FORMAT.to_owned(),
            schema: SCHEMA,
            settings_schema: crate::config::SCHEMA_VERSION,
            aether_version: env!("CARGO_PKG_VERSION").to_owned(),
            name: name.trim().to_owned(),
            created: now.to_owned(),
            modified: now.to_owned(),
            settings: portable(config),
            hardware,
            memories: Some(memories.to_vec()),
        }
    }

    /// A profile of the defaults, keeping only who the station is: its callsign and the
    /// operator's details. What *New profile* makes.
    #[must_use]
    pub fn fresh(name: &str, identity: &Config, now: &str) -> Self {
        let mut config = Config::with_callsign(&identity.callsign);
        config.operator.clone_from(&identity.operator);
        Self::capture(name, &config, &crate::memories::defaults(), None, now)
    }

    /// The file's text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Read a profile from its text, bringing an older one forward.
    ///
    /// # Errors
    /// If the text is not JSON, not a profile, from a newer version, or has no settings.
    pub fn parse(text: &str) -> Result<Self, ProfileError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| ProfileError::NotAProfile(format!("not valid JSON ({e})")))?;
        let Value::Object(mut map) = value else {
            return Err(ProfileError::NotAProfile(
                "the file is not a JSON object".into(),
            ));
        };
        match map.get("format").and_then(Value::as_str) {
            Some(FORMAT) => {}
            Some(other) => {
                return Err(ProfileError::NotAProfile(format!(
                    "its format is {other:?}, not {FORMAT:?}"
                )));
            }
            None => {
                return Err(ProfileError::NotAProfile(
                    "it does not say it is one (no \"format\" field)".into(),
                ));
            }
        }
        let schema = match map.get("schema") {
            None => 1,
            Some(Value::Number(n)) => n
                .as_u64()
                .map_or(u32::MAX, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            Some(other) => {
                return Err(ProfileError::NotAProfile(format!(
                    "its schema is {other}, not a number"
                )));
            }
        };
        if schema > SCHEMA {
            return Err(ProfileError::Newer(format!(
                "this profile was written by a newer Aether HF (profile schema {schema}; this \
                 version reads up to {SCHEMA}). Install that version to use it."
            )));
        }
        let mut version = schema;
        while let Some(step) = usize::try_from(version.saturating_sub(1))
            .ok()
            .and_then(|index| MIGRATIONS.get(index))
        {
            step(&mut map);
            version += 1;
            map.insert("schema".into(), Value::from(version));
        }
        let mut profile: Self = serde_json::from_value(Value::Object(map))
            .map_err(|e| ProfileError::NotAProfile(format!("{e}")))?;
        if !profile.settings.is_object() {
            return Err(ProfileError::NotAProfile(
                "its \"settings\" are not an object".into(),
            ));
        }
        if profile.settings_schema > crate::config::SCHEMA_VERSION {
            return Err(ProfileError::Newer(format!(
                "this profile's settings were written by a newer Aether HF (configuration \
                 schema {}; this version reads up to {}). Install that version to use it.",
                profile.settings_schema,
                crate::config::SCHEMA_VERSION
            )));
        }
        if profile.settings_schema < crate::config::SCHEMA_VERSION {
            profile.migrate_settings()?;
        }
        profile.name = profile.name.trim().to_owned();
        Ok(profile)
    }

    /// Bring the settings forward through the configuration file's own chain.
    fn migrate_settings(&mut self) -> Result<(), ProfileError> {
        let mut json = self.settings.clone();
        strip_nulls(&mut json);
        let mut table: toml::Table = serde_json::from_value(json)
            .map_err(|e| ProfileError::NotAProfile(format!("its settings cannot be read: {e}")))?;
        let reached = crate::config::migrate_with(
            &mut table,
            self.settings_schema,
            crate::config::MIGRATIONS,
        );
        table.remove("schema_version");
        self.settings = serde_json::to_value(&table)
            .map_err(|e| ProfileError::NotAProfile(format!("its settings cannot be read: {e}")))?;
        self.settings_schema = reached;
        Ok(())
    }

    /// Apply this profile to a configuration.
    ///
    /// The result keeps `base`'s machine-local and secret settings and takes the profile's
    /// for everything portable; a portable setting the profile does not name gets its
    /// default, so a profile means the same station wherever it is loaded. Nothing is
    /// changed in `base`: the caller commits the result, or not.
    ///
    /// # Errors
    /// Only when the settings will not work *together* — a rule between two settings that
    /// the configuration file would refuse too. A single bad value is reported in the
    /// result, not refused.
    pub fn apply(
        &self,
        base: &Config,
        inventory: Option<&Inventory>,
    ) -> Result<Applied, ProfileError> {
        let before = serde_json::to_value(base)
            .map_err(|e| ProfileError::Invalid(format!("cannot read the running settings: {e}")))?;
        let mut document = before.clone();
        let mut sorted = self.sort_settings(&mut document);

        let config: Config = serde_json::from_value(document.clone())
            .map_err(|e| ProfileError::Invalid(e.to_string()))?;
        config.validate().map_err(|e| match e {
            ConfigError::Invalid(detail)
            | ConfigError::Parse(detail)
            | ConfigError::Read(detail) => ProfileError::Invalid(detail),
        })?;

        let memories = match &self.memories {
            None => None,
            Some(list) => {
                let mut check = Memories::open(None);
                match check.replace(list.clone()) {
                    Ok(()) => Some(check.entries().to_vec()),
                    Err(reason) => {
                        sorted.invalid.push(Rejected {
                            key: "memories".into(),
                            value: Value::Null,
                            reason: format!("the dial list was left alone: {reason}"),
                        });
                        None
                    }
                }
            }
        };

        Ok(Applied {
            missing_hardware: self.missing_hardware(&config, inventory),
            config,
            memories,
            changed: changed_keys(&before, &document),
            unknown: sorted.unknown,
            ignored: sorted.ignored,
            invalid: sorted.invalid,
        })
    }

    /// Put the profile's settings into a document that starts as the running
    /// configuration: every portable setting reset to its default, then each of the
    /// profile's values taken or set aside with the reason.
    fn sort_settings(&self, document: &mut Value) -> Sorted {
        let schema = settings::schema();
        // every portable setting starts at its default; the keying section is one kind
        // at a time, so it is rebuilt from what the profile says below
        for setting in &schema {
            if setting.scope.is_portable() && !setting.key.starts_with("ptt.") {
                settings::set_leaf(document, &setting.key, setting.default.clone());
            }
        }
        let mut ptt: Map<String, Value> = Map::new();
        let mut sorted = Sorted::default();
        let mut all = Vec::new();
        settings::leaves("", &self.settings, &mut all);
        for (key, value) in all {
            let Some(setting) = schema.iter().find(|s| s.key == key) else {
                sorted.unknown.push(key);
                continue;
            };
            if !setting.scope.is_portable() {
                sorted.ignored.push(key);
                continue;
            }
            if !settings::fits(&key, &value, setting.type_) {
                sorted.invalid.push(Rejected {
                    reason: format!("{key} must be a {}, not {value}", type_word(setting.type_)),
                    key,
                    value,
                });
                continue;
            }
            if let Some(reason) = settings::violation(&key, &value) {
                sorted.invalid.push(Rejected { key, value, reason });
                continue;
            }
            if let Some(field) = key.strip_prefix("ptt.") {
                ptt.insert(field.to_owned(), value);
            } else {
                settings::set_leaf(document, &key, value);
            }
        }
        // the keying section: the kind decides which fields there are, so without a kind
        // the fields have nothing to hang on, and the station keys nothing until told
        if ptt.contains_key("kind") {
            document["ptt"] = Value::Object(ptt);
        } else {
            for (field, value) in ptt {
                sorted.invalid.push(Rejected {
                    key: format!("ptt.{field}"),
                    value,
                    reason: "the profile names no keying kind for it to belong to".into(),
                });
            }
            document["ptt"] = serde_json::json!({ "kind": "none" });
        }
        sorted
    }

    /// The devices the applied configuration names that this computer does not report.
    fn missing_hardware(
        &self,
        config: &Config,
        inventory: Option<&Inventory>,
    ) -> Vec<MissingHardware> {
        let Some(inventory) = inventory.filter(|i| !i.is_empty()) else {
            return Vec::new();
        };
        let mut missing = Vec::new();
        if let Some(name) = &config.audio.input
            && !inventory.inputs.contains(name)
        {
            missing.push(MissingHardware {
                key: "audio.input".into(),
                name: name.clone(),
                suggestion: None,
            });
        }
        if let Some(name) = &config.audio.output
            && !inventory.outputs.contains(name)
        {
            missing.push(MissingHardware {
                key: "audio.output".into(),
                name: name.clone(),
                suggestion: None,
            });
        }
        // a port with the same thing behind it is offered, when there is exactly one:
        // a radio's two ports share a description, and picking one of two would be a guess
        let only_match = |list: &[DeviceHint], hint: Option<&DeviceHint>| -> Option<String> {
            let description = hint.map_or("", |h| h.description.as_str());
            if description.is_empty() {
                return None;
            }
            let mut found = list.iter().filter(|d| d.description == description);
            match (found.next(), found.next()) {
                (Some(one), None) => Some(one.name.clone()),
                _ => None,
            }
        };
        match &config.ptt {
            PttConfig::Serial { port, .. } | PttConfig::Cat { port, .. }
                if !inventory.serial_ports.iter().any(|p| &p.name == port) =>
            {
                missing.push(MissingHardware {
                    key: "ptt.port".into(),
                    name: port.clone(),
                    suggestion: only_match(
                        &inventory.serial_ports,
                        self.hardware.ptt_port.as_ref(),
                    ),
                });
            }
            PttConfig::Cm108 {
                device: Some(path), ..
            } if !inventory.gpio_interfaces.iter().any(|i| &i.name == path) => {
                missing.push(MissingHardware {
                    key: "ptt.device".into(),
                    name: path.clone(),
                    suggestion: only_match(
                        &inventory.gpio_interfaces,
                        self.hardware.ptt_device.as_ref(),
                    ),
                });
            }
            _ => {}
        }
        missing
    }
}

/// What a profile's settings were sorted into, beyond the ones that were taken.
#[derive(Debug, Default)]
struct Sorted {
    unknown: Vec<String>,
    ignored: Vec<String>,
    invalid: Vec<Rejected>,
}

/// The dotted keys whose value differs between two serialised configurations, sorted:
/// what the result has that the base did not have that way. A key the old keying kind
/// had and the new one does not is not listed — `ptt.kind` says it all, as it does for
/// `config.set`.
fn changed_keys(before: &Value, after: &Value) -> Vec<String> {
    let mut was = Vec::new();
    settings::leaves("", before, &mut was);
    let mut now = Vec::new();
    settings::leaves("", after, &mut now);
    let mut changed: Vec<String> = now
        .iter()
        .filter(|(key, value)| {
            !was.iter()
                .find(|(k, _)| k == key)
                .is_some_and(|(_, old)| crate::config::same_value(old, value))
        })
        .map(|(key, _)| key.clone())
        .collect();
    changed.sort();
    changed
}

/// A type name with its article, for a sentence.
fn type_word(type_: &str) -> &str {
    match type_ {
        "boolean" => "true or false",
        "integer" => "whole number",
        "number" => "number",
        _ => "text",
    }
}

/// The name a file's stem is, and the file a name gets: the name with the characters no
/// file system takes replaced, so `Home FTDX10` is `Home FTDX10.aetherprofile`.
#[must_use]
pub fn id_for(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c == ' ' || c == '-');
    if trimmed.is_empty() {
        "profile".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A profile as the list describes it, without its settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// The file's stem, which is what the methods take.
    pub id: String,
    /// The operator's name for it.
    pub name: String,
    /// When it was first saved.
    pub created: String,
    /// When it was last saved.
    pub modified: String,
    /// Which version wrote it.
    pub aether_version: String,
    /// Where the file is.
    pub path: String,
    /// Why the file could not be read, when it could not: it is listed so the operator
    /// can see it is there, and cannot be loaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What `profiles.json` holds.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    schema: u32,
    #[serde(default)]
    active: Option<String>,
}

/// The profiles beside a configuration file, and which one is active.
#[derive(Debug)]
pub struct Store {
    dir: Option<PathBuf>,
    state_path: Option<PathBuf>,
    active: Option<String>,
}

impl Store {
    /// The store beside a configuration file: `profiles/` and `profiles.json` in its
    /// directory. With no file there is no store, and every write says so.
    #[must_use]
    pub fn open(config_path: Option<&Path>) -> Self {
        let Some(path) = config_path else {
            return Self {
                dir: None,
                state_path: None,
                active: None,
            };
        };
        let dir = path.with_file_name("profiles");
        let state_path = path.with_file_name("profiles.json");
        let active = std::fs::read_to_string(&state_path)
            .ok()
            .and_then(|text| serde_json::from_str::<State>(&text).ok())
            .filter(|state| state.schema == 1)
            .and_then(|state| state.active);
        Self {
            dir: Some(dir),
            state_path: Some(state_path),
            active,
        }
    }

    /// Where the profiles are.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The active profile's id.
    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Whether the state file has been written: a store that has never been touched is a
    /// station from before profiles, whose configuration is adopted on the first start.
    fn has_state(&self) -> bool {
        self.state_path.as_deref().is_some_and(Path::exists)
    }

    /// Make a profile active, and remember it.
    ///
    /// # Errors
    /// If the state file cannot be written.
    pub fn set_active(&mut self, id: Option<String>) -> Result<(), ProfileError> {
        let Some(path) = &self.state_path else {
            return Err(ProfileError::NoDirectory);
        };
        let state = State {
            schema: 1,
            active: id.clone(),
        };
        let text =
            serde_json::to_string_pretty(&state).map_err(|e| ProfileError::Io(e.to_string()))?;
        write_atomically(path, &text)?;
        self.active = id;
        Ok(())
    }

    /// Where a profile's file is, by id.
    #[must_use]
    pub fn path_of(&self, id: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|dir| dir.join(format!("{}.{EXTENSION}", id_for(id))))
    }

    /// Every profile, by name.
    #[must_use]
    pub fn list(&self) -> Vec<Entry> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<Entry> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
            .map(|path| {
                let id = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                match std::fs::read_to_string(&path)
                    .map_err(|e| ProfileError::Io(e.to_string()))
                    .and_then(|text| Self::parse_named(&text, &id))
                {
                    Ok(profile) => Entry {
                        id,
                        name: profile.name,
                        created: profile.created,
                        modified: profile.modified,
                        aether_version: profile.aether_version,
                        path: path.display().to_string(),
                        error: None,
                    },
                    Err(error) => Entry {
                        name: id.clone(),
                        id,
                        created: String::new(),
                        modified: String::new(),
                        aether_version: String::new(),
                        path: path.display().to_string(),
                        error: Some(error.to_string()),
                    },
                }
            })
            .collect();
        out.sort_by_key(|e| e.name.to_lowercase());
        out
    }

    /// A profile parsed, with the file's stem for a name when it has none.
    fn parse_named(text: &str, id: &str) -> Result<Profile, ProfileError> {
        let mut profile = Profile::parse(text)?;
        if profile.name.is_empty() {
            id.clone_into(&mut profile.name);
        }
        Ok(profile)
    }

    /// The id of the profile with this name, if one exists — names are compared without
    /// regard to case, as a file system would.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<String> {
        let wanted = name.trim().to_lowercase();
        self.list()
            .into_iter()
            .find(|e| e.name.to_lowercase() == wanted || e.id.to_lowercase() == wanted)
            .map(|e| e.id)
    }

    /// Read a profile by id.
    ///
    /// # Errors
    /// If there is none, or its file cannot be read or is not a profile.
    pub fn read(&self, id: &str) -> Result<Profile, ProfileError> {
        let path = self.path_of(id).ok_or(ProfileError::NoDirectory)?;
        let text = std::fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProfileError::NoSuchProfile(id.to_owned())
            } else {
                ProfileError::Io(format!("{}: {e}", path.display()))
            }
        })?;
        Self::parse_named(&text, id)
    }

    /// The name a profile must have.
    fn check_name(name: &str) -> Result<String, ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::BadName("a profile needs a name".into()));
        }
        if name.chars().count() > NAME_CHARS {
            return Err(ProfileError::BadName(format!(
                "a name is at most {NAME_CHARS} characters"
            )));
        }
        Ok(name.to_owned())
    }

    /// Write a profile, atomically, under its name.
    ///
    /// # Errors
    /// If the name is not one, another profile has it (unless `replace` names that one's
    /// id), or the file cannot be written.
    pub fn write(&self, profile: &Profile, replace: Option<&str>) -> Result<Entry, ProfileError> {
        let name = Self::check_name(&profile.name)?;
        let id = id_for(&name);
        if let Some(existing) = self.find(&name)
            && replace.is_none_or(|r| !r.eq_ignore_ascii_case(&existing))
        {
            return Err(ProfileError::NameTaken(name));
        }
        let path = self.path_of(&id).ok_or(ProfileError::NoDirectory)?;
        write_atomically(&path, &profile.to_text())?;
        Ok(Entry {
            id,
            name,
            created: profile.created.clone(),
            modified: profile.modified.clone(),
            aether_version: profile.aether_version.clone(),
            path: path.display().to_string(),
            error: None,
        })
    }

    /// Delete a profile. The active one cannot be: switch first.
    ///
    /// # Errors
    /// If it is active, does not exist, or cannot be removed.
    pub fn delete(&mut self, id: &str) -> Result<(), ProfileError> {
        if self
            .active
            .as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(id))
        {
            return Err(ProfileError::Invalid(
                "the active profile cannot be deleted: switch to another one first".into(),
            ));
        }
        let path = self.path_of(id).ok_or(ProfileError::NoDirectory)?;
        if !path.exists() {
            return Err(ProfileError::NoSuchProfile(id.to_owned()));
        }
        std::fs::remove_file(&path)
            .map_err(|e| ProfileError::Io(format!("{}: {e}", path.display())))
    }

    /// Give a profile a new name: the file moves with it, and so does the active mark.
    ///
    /// # Errors
    /// If there is no such profile, the name is taken or not one, or the files cannot be
    /// written.
    pub fn rename(&mut self, id: &str, name: &str, now: &str) -> Result<Entry, ProfileError> {
        let mut profile = self.read(id)?;
        let name = Self::check_name(name)?;
        let new_id = id_for(&name);
        if let Some(existing) = self.find(&name)
            && !existing.eq_ignore_ascii_case(id)
        {
            return Err(ProfileError::NameTaken(name));
        }
        profile.name.clone_from(&name);
        now.clone_into(&mut profile.modified);
        let entry = self.write(&profile, Some(id))?;
        if !new_id.eq_ignore_ascii_case(id) {
            let old = self.path_of(id).ok_or(ProfileError::NoDirectory)?;
            std::fs::remove_file(&old)
                .map_err(|e| ProfileError::Io(format!("{}: {e}", old.display())))?;
            if self
                .active
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(id))
            {
                self.set_active(Some(new_id))?;
            }
        }
        Ok(entry)
    }

    /// The first start with profiles: the running configuration becomes *Default*, so the
    /// operator sees their own settings under a name and nothing else changes. Done only
    /// when the store has never been touched; a store with a state file, even one that
    /// names no active profile, is left as it is.
    ///
    /// # Errors
    /// If the files cannot be written.
    pub fn adopt(
        &mut self,
        config: &Config,
        memories: &[Memory],
        inventory: Option<&Inventory>,
        now: &str,
    ) -> Result<Option<Entry>, ProfileError> {
        if self.dir.is_none() || self.has_state() {
            return Ok(None);
        }
        let mut name = ADOPTED_NAME.to_owned();
        // a directory somebody filled by hand before the first start: adopt beside it
        if self.find(&name).is_some() {
            name = format!("{ADOPTED_NAME} ({})", config.callsign);
        }
        if self.find(&name).is_some() {
            self.set_active(None)?;
            return Ok(None);
        }
        let profile = Profile::capture(&name, config, memories, inventory, now);
        let entry = self.write(&profile, None)?;
        self.set_active(Some(entry.id.clone()))?;
        Ok(Some(entry))
    }

    /// Whether the running configuration has moved from what the active profile says:
    /// whether loading the profile again would change anything. `None` without an active
    /// profile, or when its file cannot be read.
    #[must_use]
    pub fn dirty(&self, config: &Config, memories: &[Memory]) -> Option<bool> {
        let profile = self.read(self.active()?).ok()?;
        let applied = profile.apply(config, None).ok()?;
        if portable(&applied.config) != portable(config) {
            return Some(true);
        }
        // the profile's list comes back tidy — sorted, trimmed — so the running one is
        // compared in the same state
        let mut running = Memories::open(None);
        if running.replace(memories.to_vec()).is_err() {
            return Some(true);
        }
        Some(
            applied
                .memories
                .is_some_and(|list| list != running.entries()),
        )
    }
}

/// Write a file beside its target and rename it over: a profile half-written by a machine
/// that lost power is not a profile the operator could load, and the one before it is
/// what they would want back.
fn write_atomically(path: &Path, text: &str) -> Result<(), ProfileError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| ProfileError::Io(format!("cannot create {}: {e}", dir.display())))?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)
        .map_err(|e| ProfileError::Io(format!("{}: {e}", temporary.display())))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        ProfileError::Io(format!("{}: {e}", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Config {
        let mut config = Config::parse(crate::config::EXAMPLE).expect("example");
        config
            .merge(&serde_json::json!({
                "callsign": "KK4ODA",
                "operator.grid": "EM73tv",
                "operator.rig": "FTDX10",
                "operator.power_w": 30,
                "audio.input": "FTDX10 (USB AUDIO  CODEC)",
                "audio.output": "FTDX10 (USB AUDIO  CODEC)",
                "audio.tx_level": 0.2,
                "ptt.kind": "cat",
                "ptt.port": "COM6",
                "ptt.protocol": "yaesu",
                "ptt.baud": 38400,
                "radio.max_mode": 11,
                "radio.busy_threshold_db": 7.5,
                "control.token": "hunter2",
                "log.file": "aetherd.log",
                "record.dir": "C:/somewhere/recordings",
                "panel.interface": "yaesu-usb",
                "panel.waterfall.gain_db": 50,
                "panel.waterfall.palette": "blue",
            }))
            .expect("merge");
        config
    }

    fn inventory() -> Inventory {
        Inventory::from_json(&serde_json::json!({
            "devices": [
                {"name": "FTDX10 (USB AUDIO  CODEC)", "input": true, "output": true},
                {"name": "Speakers", "input": false, "output": true},
            ],
            "serial_ports": [
                {"name": "COM6", "description": "Silicon Labs CP210x USB to UART Bridge Enhanced COM Port"},
                {"name": "COM7", "description": "Silicon Labs CP210x USB to UART Bridge Standard COM Port"},
            ],
            "gpio_interfaces": [{"path": "hid#vid_0d8c", "name": "C-Media USB Audio Device"}],
        }))
    }

    fn memories() -> Vec<Memory> {
        vec![
            Memory {
                hz: 7_101_000,
                name: "40 m".into(),
            },
            Memory {
                hz: 14_107_000,
                name: "20 m".into(),
            },
        ]
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-profile-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_profile_holds_the_portable_settings_and_nothing_else() {
        let config = station();
        let profile = Profile::capture(
            "Home FTDX10",
            &config,
            &memories(),
            Some(&inventory()),
            "2026-09-19T10:00:00Z",
        );
        let text = profile.to_text();
        assert!(!text.contains("hunter2"), "the token leaked: {text}");
        assert!(!text.contains("aetherd.log"), "a path leaked: {text}");
        assert!(
            !text.contains("somewhere/recordings"),
            "a path leaked: {text}"
        );
        assert!(!text.contains("\"sim\""), "the bench leaked: {text}");
        assert!(
            !text.contains("\"bind\": \"127.0.0.1:8515\""),
            "the control port leaked"
        );
        assert!(!text.contains("schema_version"));
        assert_eq!(profile.settings["callsign"], "KK4ODA");
        assert_eq!(profile.settings["ptt"]["kind"], "cat");
        assert_eq!(profile.settings["ptt"]["port"], "COM6");
        assert_eq!(
            profile.settings["host"]["bind"], "127.0.0.1:8300",
            "a preference travels"
        );
        assert_eq!(profile.settings["panel"]["waterfall"]["palette"], "blue");
        assert_eq!(profile.settings["radio"]["max_mode"], 11);
        assert_eq!(
            profile.hardware.ptt_port,
            Some(DeviceHint {
                name: "COM6".into(),
                description: "Silicon Labs CP210x USB to UART Bridge Enhanced COM Port".into(),
            })
        );
        assert_eq!(profile.memories.as_deref(), Some(memories().as_slice()));
        assert_eq!(profile.format, FORMAT);
        assert_eq!(profile.schema, SCHEMA);
        assert_eq!(profile.settings_schema, crate::config::SCHEMA_VERSION);
        assert_eq!(profile.name, "Home FTDX10");
        assert_eq!(profile.created, "2026-09-19T10:00:00Z");
    }

    #[test]
    fn saving_and_loading_gives_back_the_same_station() {
        let config = station();
        let profile = Profile::capture("Home", &config, &memories(), Some(&inventory()), &now());
        let reread = Profile::parse(&profile.to_text()).expect("parse");
        assert_eq!(reread, profile);
        let applied = reread.apply(&config, Some(&inventory())).expect("apply");
        assert_eq!(
            applied.config, config,
            "a round trip changed the configuration"
        );
        assert!(applied.changed.is_empty(), "{:?}", applied.changed);
        assert!(applied.unknown.is_empty());
        assert!(applied.invalid.is_empty());
        assert!(applied.missing_hardware.is_empty());
        assert_eq!(applied.memories.as_deref(), Some(memories().as_slice()));
    }

    #[test]
    fn loading_keeps_this_computers_own_settings() {
        // a profile from the home station, loaded on a laptop with its own token, log and
        // recordings directory: those stay, the station's settings arrive
        let home = station();
        let profile = Profile::capture("Home", &home, &[], None, &now());
        let mut laptop = Config::parse("callsign = \"N0CALL\"").expect("parse");
        laptop.control.token = Some("laptop-secret".into());
        laptop.control.bind = "127.0.0.1:9000".into();
        laptop.log.file = Some("laptop.log".into());
        laptop.record.dir = Some("D:/rec".into());
        laptop.sim.listen = Some("127.0.0.1:8600".into());
        let applied = profile.apply(&laptop, None).expect("apply");
        assert_eq!(applied.config.callsign, "KK4ODA");
        assert_eq!(applied.config.radio.max_mode, 11);
        assert_eq!(
            applied.config.control.token.as_deref(),
            Some("laptop-secret")
        );
        assert_eq!(applied.config.control.bind, "127.0.0.1:9000");
        assert_eq!(
            applied.config.log.file.as_deref(),
            Some(Path::new("laptop.log"))
        );
        assert_eq!(
            applied.config.record.dir.as_deref(),
            Some(Path::new("D:/rec"))
        );
        assert_eq!(applied.config.sim.listen.as_deref(), Some("127.0.0.1:8600"));
        assert!(applied.changed.contains(&"callsign".to_owned()));
        assert!(applied.changed.contains(&"ptt.kind".to_owned()));
        assert!(
            applied
                .restart_required()
                .contains(&"audio.input".to_owned())
        );
        assert!(
            !applied
                .restart_required()
                .contains(&"radio.max_mode".to_owned())
        );
    }

    #[test]
    fn a_setting_the_profile_does_not_name_gets_its_default_not_the_running_value() {
        // a profile means the same station wherever it is loaded; what it does not say is
        // the default, or switching profiles would leak settings from one to the next
        let mut running = station();
        running.radio.max_mode = 5;
        running.radio.cw_id = true;
        let mut profile = Profile::capture("Sparse", &running, &[], None, &now());
        profile.settings["radio"]
            .as_object_mut()
            .expect("radio")
            .remove("max_mode");
        profile.settings["radio"]
            .as_object_mut()
            .expect("radio")
            .remove("cw_id");
        profile
            .settings
            .as_object_mut()
            .expect("settings")
            .remove("update");
        let applied = profile.apply(&running, None).expect("apply");
        assert_eq!(applied.config.radio.max_mode, 15);
        assert!(!applied.config.radio.cw_id);
        assert_eq!(
            applied.config.update,
            crate::config::UpdateSection::default()
        );
        assert!(applied.unknown.is_empty());
        assert!(applied.invalid.is_empty());
    }

    #[test]
    fn a_setting_this_version_does_not_have_is_reported_and_left_out() {
        let config = station();
        let mut profile = Profile::capture("Future", &config, &[], None, &now());
        profile.settings["radio"]["time_diversity"] = serde_json::json!(true);
        profile.settings["antenna"] = serde_json::json!({ "rotator": "yes" });
        profile.aether_version = "0.9.0".into();
        let text = profile.to_text();
        let reread = Profile::parse(&text).expect("a newer field is not a refusal");
        let applied = reread.apply(&config, None).expect("apply");
        assert_eq!(
            applied.unknown,
            vec![
                "antenna.rotator".to_owned(),
                "radio.time_diversity".to_owned()
            ]
        );
        assert_eq!(applied.config, config);
    }

    #[test]
    fn a_secret_or_a_path_in_a_profile_is_ignored_not_applied() {
        let config = station();
        let mut profile = Profile::capture("Tampered", &config, &[], None, &now());
        profile.settings["control"] =
            serde_json::json!({ "token": "injected", "bind": "0.0.0.0:8515" });
        profile.settings["log"]["file"] = serde_json::json!("/etc/passwd");
        let applied = profile.apply(&config, None).expect("apply");
        assert_eq!(applied.config.control.token.as_deref(), Some("hunter2"));
        assert_eq!(applied.config.control.bind, "127.0.0.1:8515");
        assert_eq!(
            applied.config.log.file.as_deref(),
            Some(Path::new("aetherd.log"))
        );
        assert!(applied.ignored.contains(&"control.token".to_owned()));
        assert!(applied.ignored.contains(&"log.file".to_owned()));
    }

    #[test]
    fn a_bad_value_is_reported_and_the_default_kept_while_the_rest_loads() {
        let config = station();
        let mut profile = Profile::capture("Odd", &config, &[], None, &now());
        profile.settings["radio"]["max_key_s"] = serde_json::json!(-5);
        profile.settings["radio"]["bandwidth"] = serde_json::json!(2750);
        profile.settings["radio"]["wait_for_clear"] = serde_json::json!("yes");
        profile.settings["audio"]["tx_level"] = serde_json::json!(4.0);
        profile.settings["update"]["channel"] = serde_json::json!("alpha");
        profile.settings["radio"]["max_mode"] = serde_json::json!(3.7);
        let applied = profile.apply(&config, None).expect("the rest loads");
        let keys: Vec<&str> = applied.invalid.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "audio.tx_level",
                "radio.bandwidth",
                "radio.max_key_s",
                "radio.max_mode",
                "radio.wait_for_clear",
                "update.channel"
            ]
        );
        assert!(
            applied.invalid[2].reason.contains("watchdog"),
            "{}",
            applied.invalid[2].reason
        );
        assert!(
            (applied.config.radio.max_key_s - 30.0).abs() < 1e-9,
            "the default"
        );
        assert_eq!(applied.config.radio.bandwidth, 2300);
        assert!(applied.config.radio.wait_for_clear);
        assert!((applied.config.audio.tx_level - 0.25).abs() < 1e-9);
        assert_eq!(applied.config.radio.max_mode, 15);
        // and what was fine arrived
        assert_eq!(applied.config.callsign, "KK4ODA");
        assert!((applied.config.radio.busy_threshold_db - 7.5).abs() < 1e-9);
    }

    #[test]
    fn settings_that_will_not_work_together_refuse_the_whole_load() {
        // an Icom with no address is a rule between two settings; nothing is half applied
        let config = station();
        let mut profile = Profile::capture("Icom", &config, &[], None, &now());
        profile.settings["ptt"] =
            serde_json::json!({ "kind": "cat", "port": "COM4", "protocol": "icom" });
        let error = profile.apply(&config, None).expect_err("refused");
        assert!(matches!(error, ProfileError::Invalid(_)), "{error}");
        assert!(error.to_string().contains("civ_address"), "{error}");
    }

    #[test]
    fn keying_fields_without_a_kind_key_nothing() {
        let config = station();
        let mut profile = Profile::capture("No kind", &config, &[], None, &now());
        profile.settings["ptt"] = serde_json::json!({ "port": "COM4", "line": "rts" });
        let applied = profile.apply(&config, None).expect("apply");
        assert_eq!(applied.config.ptt, PttConfig::None);
        assert_eq!(applied.invalid.len(), 2);
        // and a profile with no keying section at all is a station that keys nothing
        profile
            .settings
            .as_object_mut()
            .expect("settings")
            .remove("ptt");
        let applied = profile.apply(&config, None).expect("apply");
        assert_eq!(applied.config.ptt, PttConfig::None);
        assert!(applied.invalid.is_empty());
    }

    #[test]
    fn a_missing_sound_card_is_reported_and_left_as_named() {
        let config = station();
        let profile = Profile::capture("Home", &config, &[], Some(&inventory()), &now());
        let elsewhere = Inventory::from_json(&serde_json::json!({
            "devices": [{"name": "Realtek Audio", "input": true, "output": true}],
            "serial_ports": [{"name": "COM6", "description": "whatever"}],
        }));
        let applied = profile.apply(&config, Some(&elsewhere)).expect("apply");
        let keys: Vec<&str> = applied
            .missing_hardware
            .iter()
            .map(|m| m.key.as_str())
            .collect();
        assert_eq!(keys, vec!["audio.input", "audio.output"]);
        assert_eq!(
            applied.missing_hardware[0].name,
            "FTDX10 (USB AUDIO  CODEC)"
        );
        // kept as named, never swapped for whatever is there: the daemon runs without
        // audio and says so until the operator chooses
        assert_eq!(
            applied.config.audio.input.as_deref(),
            Some("FTDX10 (USB AUDIO  CODEC)")
        );
    }

    #[test]
    fn a_missing_serial_port_is_reported_with_the_port_that_has_the_same_thing_behind_it() {
        let config = station();
        let profile = Profile::capture("Home", &config, &[], Some(&inventory()), &now());
        // the other computer numbered the same bridge differently: COM6 became COM11
        let elsewhere = Inventory::from_json(&serde_json::json!({
            "devices": [{"name": "FTDX10 (USB AUDIO  CODEC)", "input": true, "output": true}],
            "serial_ports": [
                {"name": "COM10", "description": "Silicon Labs CP210x USB to UART Bridge Standard COM Port"},
                {"name": "COM11", "description": "Silicon Labs CP210x USB to UART Bridge Enhanced COM Port"},
            ],
        }));
        let applied = profile.apply(&config, Some(&elsewhere)).expect("apply");
        assert_eq!(applied.missing_hardware.len(), 1);
        let missing = &applied.missing_hardware[0];
        assert_eq!(missing.key, "ptt.port");
        assert_eq!(missing.name, "COM6");
        assert_eq!(missing.suggestion.as_deref(), Some("COM11"));
        assert!(
            matches!(&applied.config.ptt, PttConfig::Cat { port, .. } if port == "COM6"),
            "the suggestion was taken without asking"
        );
        // two ports with the same description is a guess, and a guess is not offered
        let ambiguous = Inventory::from_json(&serde_json::json!({
            "serial_ports": [
                {"name": "COM10", "description": "Silicon Labs CP210x USB to UART Bridge Enhanced COM Port"},
                {"name": "COM11", "description": "Silicon Labs CP210x USB to UART Bridge Enhanced COM Port"},
            ],
        }));
        let applied = profile.apply(&config, Some(&ambiguous)).expect("apply");
        assert_eq!(applied.missing_hardware[0].suggestion, None);
        // a computer that could not be asked reports nothing rather than everything
        let applied = profile
            .apply(&config, Some(&Inventory::default()))
            .expect("apply");
        assert!(applied.missing_hardware.is_empty());
    }

    #[test]
    fn a_missing_gpio_interface_is_reported_too() {
        let mut config = station();
        config
            .merge(&serde_json::json!({ "ptt.kind": "cm108", "ptt.device": "hid#vid_0d8c", "ptt.gpio": 3 }))
            .expect("merge");
        let profile = Profile::capture("DRA", &config, &[], Some(&inventory()), &now());
        assert_eq!(
            profile
                .hardware
                .ptt_device
                .as_ref()
                .map(|h| h.description.as_str()),
            Some("C-Media USB Audio Device")
        );
        let elsewhere = Inventory::from_json(&serde_json::json!({
            "devices": [{"name": "FTDX10 (USB AUDIO  CODEC)", "input": true, "output": true}],
            "gpio_interfaces": [{"path": "hid#other", "name": "C-Media USB Audio Device"}],
        }));
        let applied = profile.apply(&config, Some(&elsewhere)).expect("apply");
        assert_eq!(applied.missing_hardware.len(), 1);
        assert_eq!(applied.missing_hardware[0].key, "ptt.device");
        assert_eq!(
            applied.missing_hardware[0].suggestion.as_deref(),
            Some("hid#other")
        );
    }

    #[test]
    fn corrupt_files_are_refused_with_a_reason() {
        for (text, fragment) in [
            ("", "JSON"),
            ("{", "JSON"),
            ("[1, 2]", "object"),
            ("{\"callsign\": \"W4ODA\"}", "format"),
            ("{\"format\": \"something-else\", \"settings\": {}}", "not"),
            (
                "{\"format\": \"aether-hf-profile\", \"schema\": 1}",
                "settings",
            ),
            (
                "{\"format\": \"aether-hf-profile\", \"schema\": 1, \"settings\": 7}",
                "settings",
            ),
            (
                "{\"format\": \"aether-hf-profile\", \"schema\": \"one\", \"settings\": {}}",
                "schema",
            ),
        ] {
            let error = Profile::parse(text).expect_err(text);
            assert!(
                matches!(error, ProfileError::NotAProfile(_)),
                "{text}: {error:?}"
            );
            assert!(error.to_string().contains(fragment), "{text}: {error}");
        }
    }

    #[test]
    fn a_profile_from_a_newer_version_is_refused_not_misread() {
        let newer = "{\"format\": \"aether-hf-profile\", \"schema\": 99, \"settings\": {}}";
        let error = Profile::parse(newer).expect_err("newer");
        assert!(matches!(error, ProfileError::Newer(_)), "{error}");
        assert!(error.to_string().contains("newer"), "{error}");
        let newer_settings = format!(
            "{{\"format\": \"aether-hf-profile\", \"schema\": 1, \"settings_schema\": {}, \"settings\": {{}}}}",
            crate::config::SCHEMA_VERSION + 1
        );
        let error = Profile::parse(&newer_settings).expect_err("newer settings");
        assert!(matches!(error, ProfileError::Newer(_)), "{error}");
    }

    #[test]
    fn an_older_profile_is_read_with_what_it_has_and_defaults_for_the_rest() {
        // the first shape, with the envelope fields a later version added missing
        let old = r#"{
          "format": "aether-hf-profile",
          "schema": 1,
          "name": "Old",
          "settings": { "callsign": "W4ODA", "radio": { "max_mode": 6 } }
        }"#;
        let profile = Profile::parse(old).expect("parse");
        // its settings were the first schema's, and come forward through the file's chain
        assert_eq!(profile.settings_schema, crate::config::SCHEMA_VERSION);
        assert_eq!(profile.aether_version, "");
        assert_eq!(profile.memories, None);
        let applied = profile.apply(&station(), None).expect("apply");
        assert_eq!(applied.config.callsign, "W4ODA");
        // the OFDM mode 6 it named is rung 8 of the wide ladder (ADR-0013)
        assert_eq!(applied.config.radio.max_mode, 8);
        assert_eq!(applied.config.ptt, PttConfig::None);
        assert_eq!(
            applied.memories, None,
            "a profile without dials leaves the list alone"
        );
    }

    #[test]
    fn the_settings_go_through_the_configuration_files_own_migrations() {
        // a profile of the first schema, with a null leaf, which TOML cannot hold and the
        // conversion must strip: its max_mode comes forward as the file's would
        let mut profile = Profile::capture("Now", &station(), &[], None, &now());
        profile.settings["audio"]["input"] = Value::Null;
        profile.settings["radio"]["max_mode"] = serde_json::json!(13);
        profile.settings_schema = crate::config::first_schema();
        profile.migrate_settings().expect("migrates");
        assert_eq!(profile.settings_schema, crate::config::SCHEMA_VERSION);
        assert_eq!(profile.settings["radio"]["max_mode"], 15);
        assert!(
            profile.settings["audio"].get("input").is_none(),
            "a null is an absent setting"
        );
        assert_eq!(profile.settings["callsign"], "KK4ODA");
        assert!(profile.settings.get("schema_version").is_none());
    }

    #[test]
    fn every_profile_a_released_version_wrote_still_loads() {
        // `tests/data/profiles/` holds a file as each released version wrote it; a
        // release adds its own, and this is the promise that an old profile stays usable
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/profiles");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("fixture directory") {
            let path = entry.expect("entry").path();
            if path.extension().is_none_or(|e| e != EXTENSION) {
                continue;
            }
            seen += 1;
            let text = std::fs::read_to_string(&path).expect("read fixture");
            let profile =
                Profile::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let applied = profile
                .apply(
                    &Config::parse("callsign = \"N0CALL\"").expect("parse"),
                    None,
                )
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                applied.unknown.is_empty(),
                "{}: {:?}",
                path.display(),
                applied.unknown
            );
            assert!(
                applied.invalid.is_empty(),
                "{}: {:?}",
                path.display(),
                applied.invalid
            );
            // every leaf the file has arrived, with its value
            let mut wrote = Vec::new();
            settings::leaves("", &profile.settings, &mut wrote);
            let after = portable(&applied.config);
            for (key, value) in &wrote {
                let kept = key
                    .split('.')
                    .try_fold(&after, |cursor, part| cursor.get(part))
                    .cloned()
                    .unwrap_or(Value::Null);
                assert!(
                    crate::config::same_value(&kept, value),
                    "{}: {key} = {value} was lost or changed on load (now {kept})",
                    path.display()
                );
            }
        }
        assert!(seen > 0, "no fixtures in {}", dir.display());
    }

    #[test]
    fn the_store_lists_reads_writes_renames_and_deletes() {
        let dir = temp_dir("store");
        let config_path = dir.join("station.toml");
        let mut store = Store::open(Some(&config_path));
        assert_eq!(store.dir(), Some(dir.join("profiles").as_path()));
        assert!(store.list().is_empty());
        assert_eq!(store.active(), None);

        let home = Profile::capture(
            "Home FTDX10",
            &station(),
            &memories(),
            None,
            "2026-09-19T10:00:00Z",
        );
        let entry = store.write(&home, None).expect("write");
        assert_eq!(entry.id, "Home FTDX10");
        assert!(Path::new(&entry.path).is_file());
        assert!(
            !dir.join("profiles")
                .join("Home FTDX10.aetherprofile.new")
                .exists()
        );
        assert_eq!(
            store.write(&home, None).expect_err("twice"),
            ProfileError::NameTaken("Home FTDX10".into())
        );
        let mut truck = Profile::fresh("Truck / IC-705", &station(), "2026-09-19T11:00:00Z");
        assert_eq!(truck.settings["callsign"], "KK4ODA", "the identity stays");
        assert_eq!(truck.settings["operator"]["grid"], "EM73tv");
        assert_eq!(
            truck.settings["ptt"]["kind"], "none",
            "everything else is the default"
        );
        assert_eq!(truck.settings["radio"]["max_mode"], 15);
        let entry = store.write(&truck, None).expect("write");
        assert_eq!(
            entry.id, "Truck - IC-705",
            "the slash is not a file's to have"
        );
        truck.name = "home ftdx10".into();
        assert!(
            matches!(store.write(&truck, None), Err(ProfileError::NameTaken(_))),
            "names differ in case only"
        );

        let listed = store.list();
        assert_eq!(
            listed.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["Home FTDX10", "Truck / IC-705"]
        );
        assert_eq!(store.find("home ftdx10").as_deref(), Some("Home FTDX10"));
        assert_eq!(store.read("Home FTDX10").expect("read"), home);
        assert!(matches!(
            store.read("Nope"),
            Err(ProfileError::NoSuchProfile(_))
        ));

        store
            .set_active(Some("Home FTDX10".into()))
            .expect("active");
        let again = Store::open(Some(&config_path));
        assert_eq!(
            again.active(),
            Some("Home FTDX10"),
            "remembered across a restart"
        );
        assert!(
            matches!(store.delete("Home FTDX10"), Err(ProfileError::Invalid(_))),
            "the active one stays"
        );

        let renamed = store
            .rename("Home FTDX10", "Home", "2026-09-19T12:00:00Z")
            .expect("rename");
        assert_eq!(renamed.id, "Home");
        assert_eq!(store.active(), Some("Home"), "the mark moved with it");
        assert!(
            !dir.join("profiles")
                .join("Home FTDX10.aetherprofile")
                .exists()
        );
        assert_eq!(
            store.read("Home").expect("read").modified,
            "2026-09-19T12:00:00Z"
        );
        assert!(matches!(
            store.rename("Home", "Truck / IC-705", "x"),
            Err(ProfileError::NameTaken(_))
        ));
        assert!(matches!(
            store.rename("Home", "  ", "x"),
            Err(ProfileError::BadName(_))
        ));

        store.delete("Truck - IC-705").expect("delete");
        assert_eq!(store.list().len(), 1);
        assert!(matches!(
            store.delete("Truck - IC-705"),
            Err(ProfileError::NoSuchProfile(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_in_the_directory_is_listed_with_its_error_and_the_rest_still_work() {
        let dir = temp_dir("corrupt");
        let store = Store::open(Some(&dir.join("station.toml")));
        store
            .write(
                &Profile::capture("Good", &station(), &[], None, &now()),
                None,
            )
            .expect("write");
        std::fs::write(
            dir.join("profiles").join("Broken.aetherprofile"),
            "{ not json",
        )
        .expect("write");
        let listed = store.list();
        assert_eq!(listed.len(), 2);
        let broken = listed.iter().find(|e| e.id == "Broken").expect("listed");
        assert!(
            broken.error.as_deref().is_some_and(|e| e.contains("JSON")),
            "{broken:?}"
        );
        assert!(listed.iter().any(|e| e.id == "Good" && e.error.is_none()));
        assert!(matches!(
            store.read("Broken"),
            Err(ProfileError::NotAProfile(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_the_profile_that_was_there() {
        let dir = temp_dir("atomic");
        let store = Store::open(Some(&dir.join("station.toml")));
        let profile = Profile::capture("Kept", &station(), &[], None, &now());
        let entry = store.write(&profile, None).expect("write");
        let before = std::fs::read_to_string(&entry.path).expect("read");
        // the temporary file cannot be written: a directory sits where it would go, so
        // nothing reaches the rename and nothing that was there is touched
        let path = store.path_of("Blocked").expect("path");
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".new");
        std::fs::create_dir_all(&temporary).expect("a directory in the way");
        let mut other = profile.clone();
        other.name = "Blocked".into();
        assert!(matches!(
            store.write(&other, None),
            Err(ProfileError::Io(_))
        ));
        assert!(
            !path.exists(),
            "a profile appeared from a write that failed"
        );
        assert_eq!(std::fs::read_to_string(&entry.path).expect("read"), before);
        // an overwrite of a profile that exists goes through the same temporary file, and
        // the one being replaced is whole until the rename
        let mut again = profile.clone();
        again.modified = "later".into();
        store.write(&again, Some("Kept")).expect("overwrite");
        assert!(
            std::fs::read_to_string(&entry.path)
                .expect("read")
                .contains("\"later\"")
        );
        assert!(
            !Path::new(&format!("{}.new", entry.path)).exists(),
            "the temporary file was left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_start_adopts_the_running_configuration_and_is_not_dirty() {
        let dir = temp_dir("adopt");
        let config_path = dir.join("station.toml");
        let config = station();
        config.save(&config_path).expect("save");
        let before = std::fs::read_to_string(&config_path).expect("read");
        let mut store = Store::open(Some(&config_path));
        let entry = store
            .adopt(&config, &memories(), Some(&inventory()), &now())
            .expect("adopt")
            .expect("adopted");
        assert_eq!(entry.name, ADOPTED_NAME);
        assert_eq!(store.active(), Some("Default"));
        assert_eq!(store.dirty(&config, &memories()), Some(false));
        assert_eq!(
            std::fs::read_to_string(&config_path).expect("read"),
            before,
            "the configuration file was touched"
        );
        // the next start finds the state and adopts nothing, even with no active profile
        store.set_active(None).expect("cleared");
        let mut again = Store::open(Some(&config_path));
        assert_eq!(
            again.adopt(&config, &[], None, &now()).expect("adopt"),
            None
        );
        assert_eq!(again.list().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_to_the_running_configuration_makes_the_profile_dirty_and_saving_cleans_it() {
        let dir = temp_dir("dirty");
        let config_path = dir.join("station.toml");
        let mut config = station();
        let mut store = Store::open(Some(&config_path));
        store
            .adopt(&config, &memories(), None, &now())
            .expect("adopt");
        assert_eq!(store.dirty(&config, &memories()), Some(false));
        config
            .merge(&serde_json::json!({ "radio.max_mode": 4 }))
            .expect("merge");
        assert_eq!(store.dirty(&config, &memories()), Some(true));
        // a machine-local change is not the profile's business
        config
            .merge(&serde_json::json!({ "radio.max_mode": 11, "log.file": "elsewhere.log" }))
            .expect("merge");
        assert_eq!(store.dirty(&config, &memories()), Some(false));
        // the dials are the profile's
        let mut dials = memories();
        dials.push(Memory {
            hz: 3_590_000,
            name: "80 m".into(),
        });
        assert_eq!(store.dirty(&config, &dials), Some(true));
        let saved = Profile::capture("Default", &config, &dials, None, &now());
        store.write(&saved, Some("Default")).expect("overwrite");
        assert_eq!(store.dirty(&config, &dials), Some(false));
        // no active profile: nothing to be dirty against
        store.set_active(None).expect("cleared");
        assert_eq!(store.dirty(&config, &dials), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_daemon_without_a_configuration_file_has_no_store() {
        let mut store = Store::open(None);
        assert!(store.list().is_empty());
        assert_eq!(store.dir(), None);
        let profile = Profile::capture("X", &station(), &[], None, &now());
        assert_eq!(
            store.write(&profile, None).expect_err("no dir"),
            ProfileError::NoDirectory
        );
        assert_eq!(
            store.adopt(&station(), &[], None, &now()).expect("nothing"),
            None
        );
        assert_eq!(store.dirty(&station(), &[]), None);
    }

    #[test]
    fn ids_are_names_a_file_system_takes() {
        assert_eq!(id_for("Home FTDX10"), "Home FTDX10");
        assert_eq!(id_for("  Truck / IC-705  "), "Truck - IC-705");
        assert_eq!(id_for("a:b*c?d\"e<f>g|h\\i"), "a-b-c-d-e-f-g-h-i");
        assert_eq!(id_for("..."), "profile");
        assert_eq!(id_for(""), "profile");
        assert_eq!(id_for("trailing. "), "trailing");
    }
}
