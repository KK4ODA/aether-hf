//! The settings registry: every setting the daemon has, described once.
//!
//! [`Config`] is the authoritative model of what an operator can set, and serde already
//! gives every field its name, type and default. What the struct cannot say is what a
//! setting *is* — whether it belongs to the station or to this computer, whether it names a
//! piece of hardware, what values are sensible — and that is what this module adds, as a
//! short table of rules keyed by the dotted name the control API already uses.
//!
//! The registry is derived, not maintained: [`schema`] walks a fully populated template of
//! the configuration and lists every leaf it finds, so a field added to a section appears
//! here — and in every profile — without anyone listing it. A rule is written only for a
//! setting that needs one: a bound, a set of options, a scope other than portable. The tests
//! hold the two together: every rule names a real setting, every nullable field is declared,
//! every bound is one [`Config::validate`] enforces (it runs [`check_bounds`], so a bound
//! written here is the bound).

use serde_json::Value;

use crate::config::{CatProtocol, CatSource, Config, ConfigError, PttConfig, SerialLineConfig};

/// What kind of thing a setting is, which decides whether a profile carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The station's own: what an operator would want on any computer they run it from.
    Portable,
    /// Portable, but the name of a device this computer may not have. A profile carries it
    /// and a load checks it against what the machine reports.
    Hardware,
    /// This installation's wiring — a path, a socket, the bench — and meaningless
    /// elsewhere. Never leaves the running configuration.
    Machine,
    /// A credential. Never written anywhere but the configuration file.
    Secret,
}

impl Scope {
    /// Whether a profile carries a setting of this scope.
    #[must_use]
    pub const fn is_portable(self) -> bool {
        matches!(self, Self::Portable | Self::Hardware)
    }
}

/// What the registry says about one setting beyond what its type says.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    /// The dotted key: `radio.max_key_s`. A key ending in `.*` covers a whole table.
    pub key: &'static str,
    /// Whose the setting is.
    pub scope: Scope,
    /// Whether `null` — unset — is a value it takes.
    pub nullable: bool,
    /// The smallest value allowed, inclusive.
    pub min: Option<f64>,
    /// The largest value allowed, inclusive.
    pub max: Option<f64>,
    /// The only values allowed, as the JSON renders them (`"rts"`, `"2300"`).
    pub options: &'static [&'static str],
    /// Why the bound is what it is: the half of an error message an operator can act on.
    pub why: &'static str,
}

const DEFAULT_RULE: Rule = Rule {
    key: "",
    scope: Scope::Portable,
    nullable: false,
    min: None,
    max: None,
    options: &[],
    why: "",
};

const fn rule(key: &'static str) -> Rule {
    Rule {
        key,
        ..DEFAULT_RULE
    }
}

/// The smallest positive value: the bound of a setting that must be above zero.
const POSITIVE: f64 = f64::MIN_POSITIVE;

/// The rules. Anything not named here is portable, not nullable and unbounded, which is
/// the right default for most of what gets added: a new flag or a new note wants to travel
/// with the station and takes whatever its type takes.
pub const RULES: &[Rule] = &[
    // ── who and where ──────────────────────────────────────────────────────────────
    Rule {
        nullable: true,
        min: Some(POSITIVE),
        max: Some(2000.0),
        why: "power is watts, above zero",
        ..rule("operator.power_w")
    },
    // ── the sound card ─────────────────────────────────────────────────────────────
    Rule {
        scope: Scope::Hardware,
        nullable: true,
        ..rule("audio.input")
    },
    Rule {
        scope: Scope::Hardware,
        nullable: true,
        ..rule("audio.output")
    },
    Rule {
        options: &["48000"],
        why: "the waveform is built around 48 kHz and nothing resamples",
        ..rule("audio.sample_rate")
    },
    Rule {
        min: Some(POSITIVE),
        max: Some(1.0),
        why: "the transmit level is a fraction of full scale",
        ..rule("audio.tx_level")
    },
    // ── keying ─────────────────────────────────────────────────────────────────────
    Rule {
        options: &["none", "serial", "rigctld", "cat", "cm108"],
        why: "those are the ways this version can key a radio",
        ..rule("ptt.kind")
    },
    Rule {
        scope: Scope::Hardware,
        ..rule("ptt.port")
    },
    Rule {
        scope: Scope::Hardware,
        nullable: true,
        ..rule("ptt.device")
    },
    Rule {
        options: &["rts", "dtr", "both"],
        why: "a serial port has those two control lines",
        ..rule("ptt.line")
    },
    Rule {
        options: &["yaesu", "kenwood", "icom"],
        why: "those are the command sets this version speaks",
        ..rule("ptt.protocol")
    },
    Rule {
        options: &["data", "mic"],
        why: "it is accepted and ignored, but it has to be one of the two it once meant",
        ..rule("ptt.source")
    },
    Rule {
        min: Some(1.0),
        why: "a serial rate is a positive number of bits per second",
        ..rule("ptt.baud")
    },
    Rule {
        nullable: true,
        min: Some(0.0),
        max: Some(255.0),
        why: "a CI-V address is one byte (0x94 for an IC-7300, 0xA4 for an IC-705)",
        ..rule("ptt.civ_address")
    },
    Rule {
        min: Some(1.0),
        max: Some(8.0),
        why: "the codec has eight pins, and the DRA and URI boards key on 3",
        ..rule("ptt.gpio")
    },
    // ── the radio ──────────────────────────────────────────────────────────────────
    Rule {
        min: Some(POSITIVE),
        why: "a watchdog that can never fire is not one",
        ..rule("radio.max_key_s")
    },
    Rule {
        // `2300` and `500` are the waveforms this version has; a test holds this list
        // to `RadioSection::params`
        options: &["2300", "500"],
        why: "those are the waveforms this version has",
        ..rule("radio.bandwidth")
    },
    Rule {
        // the wide ladder's last rung; a narrower ladder clamps rather than refuses, and a
        // test holds this number to the ladder
        min: Some(0.0),
        max: Some(19.0),
        why: "the ladder has twenty rungs: the tone floor's six and the fourteen OFDM modes",
        ..rule("radio.max_mode")
    },
    // ── this installation ──────────────────────────────────────────────────────────
    Rule {
        scope: Scope::Machine,
        ..rule("control.enabled")
    },
    Rule {
        scope: Scope::Machine,
        ..rule("control.bind")
    },
    Rule {
        scope: Scope::Secret,
        nullable: true,
        ..rule("control.token")
    },
    Rule {
        scope: Scope::Machine,
        nullable: true,
        ..rule("control.ui_dir")
    },
    Rule {
        options: &["text", "json"],
        why: "the log is written for a terminal or for a log shipper",
        ..rule("log.format")
    },
    Rule {
        scope: Scope::Machine,
        nullable: true,
        ..rule("log.file")
    },
    Rule {
        options: &["stable", "beta", "nightly"],
        why: "those are the release channels",
        ..rule("update.channel")
    },
    Rule {
        scope: Scope::Machine,
        nullable: true,
        ..rule("record.dir")
    },
    Rule {
        scope: Scope::Machine,
        nullable: true,
        ..rule("sim.listen")
    },
    Rule {
        scope: Scope::Machine,
        nullable: true,
        ..rule("sim.connect")
    },
    Rule {
        scope: Scope::Machine,
        ..rule("sim.*")
    },
    // ── the panel ──────────────────────────────────────────────────────────────────
    Rule {
        nullable: true,
        ..rule("panel.interface")
    },
];

/// The rule for a key: its own, the rule of the table it is in, or the default.
#[must_use]
pub fn rule_for(key: &str) -> Rule {
    RULES
        .iter()
        .find(|r| r.key == key)
        .or_else(|| RULES.iter().find(|r| covers(r.key, key)))
        .copied()
        .unwrap_or(DEFAULT_RULE)
}

/// Whether a `table.*` rule key covers a dotted key.
fn covers(pattern: &str, key: &str) -> bool {
    pattern.strip_suffix(".*").is_some_and(|prefix| {
        key.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Whether a profile carries this setting.
#[must_use]
pub fn is_portable(key: &str) -> bool {
    key != "schema_version" && rule_for(key).scope.is_portable()
}

/// One setting, as `config.schema` describes it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Setting {
    /// The dotted key.
    pub key: String,
    /// `boolean`, `integer`, `number` or `string`.
    #[serde(rename = "type")]
    pub type_: &'static str,
    /// Whether unset is a value.
    pub nullable: bool,
    /// What a file that does not name it gets.
    pub default: Value,
    /// Whose it is.
    pub scope: Scope,
    /// Whether a change takes effect without a restart.
    pub live: bool,
    /// The smallest value allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// The largest value allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The only values allowed.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub options: &'static [&'static str],
    /// Why the bound is what it is.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub why: &'static str,
}

/// A configuration with every optional field set, so that every leaf has a type.
///
/// The keying section is an enum and a file holds one kind at a time, so the template
/// alone cannot show every keying field; [`shapes`] adds one instance of each kind.
const TEMPLATE: &str = r#"
schema_version = 1
callsign = "N0CALL"

[operator]
grid = "EM73tv"
rig = "FTDX10"
power_w = 30.0
antenna = "EFHW at 10 m"

[audio]
input = "USB Audio CODEC"
output = "USB Audio CODEC"
sample_rate = 48000
tx_level = 0.25

[ptt]
kind = "cat"
port = "COM6"
protocol = "icom"
baud = 19200
civ_address = 148
source = "data"

[radio]
max_key_s = 30.0
wait_for_clear = true
busy_threshold_db = 6.0
bandwidth = 2300
answer_only = false
max_mode = 19
compress = false
cw_id = false
cw_id_wpm = 20.0
cw_id_interval_s = 600.0

[control]
enabled = true
bind = "127.0.0.1:8515"
token = "a long random string"
ui_dir = "ui"

[host]
enabled = false
bind = "127.0.0.1:8300"
trace = false

[log]
format = "text"
file = "aetherd.log"
keep = 500

[update]
channel = "stable"
check = true

[record]
dir = "recordings"
auto = false
notes = ""
tx_audio = false

[sim]
listen = "127.0.0.1:8600"
connect = "127.0.0.1:8601"
snr_db = 30.0

[panel]
interface = "manual"

[panel.waterfall]
auto = true
floor_db = -90.0
gain_db = 45.0
speed = 8
palette = "aether"
"#;

/// The template, parsed but not validated: it sets both `[sim]` addresses, which a running
/// station may not, and a token on a loopback bind, which is harmless.
fn template() -> Config {
    let table: toml::Table = toml::from_str(TEMPLATE).expect("the settings template parses");
    toml::Value::Table(table)
        .try_into()
        .expect("the settings template is a configuration")
}

/// One keying section of each kind, every field set, for the union of keying fields.
///
/// A new variant of [`PttConfig`] is added here too, or the test that asks for every kind
/// by name fails.
fn keying_kinds() -> Vec<PttConfig> {
    vec![
        PttConfig::None,
        PttConfig::Serial {
            port: "COM7".into(),
            line: SerialLineConfig::Rts,
        },
        PttConfig::Rigctld {
            address: "127.0.0.1:4532".into(),
        },
        PttConfig::Cat {
            port: "COM6".into(),
            protocol: CatProtocol::Icom,
            baud: 19_200,
            civ_address: Some(0x94),
            source: CatSource::Data,
        },
        PttConfig::Cm108 {
            device: Some("hid#vid_0d8c&pid_000c".into()),
            gpio: 3,
        },
    ]
}

/// Every leaf of a JSON object, as `(dotted key, value)`, tables recursed into.
pub fn leaves(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                let name = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(&name, inner, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

/// The leaves of a value, with those of one keying section of each kind added after.
fn with_every_keying_kind(value: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    leaves("", value, &mut out);
    for kind in keying_kinds() {
        let mut ptt = Vec::new();
        leaves(
            "ptt",
            &serde_json::to_value(&kind).expect("serialise"),
            &mut ptt,
        );
        for (key, value) in ptt {
            if !out.iter().any(|(k, _)| *k == key) {
                out.push((key, value));
            }
        }
    }
    out
}

/// Every setting there is, with the value the template gives it.
fn shapes() -> Vec<(String, Value)> {
    with_every_keying_kind(&serde_json::to_value(template()).expect("serialise"))
}

/// Every key the configuration has, dotted, in the file's order with the keying kinds'
/// fields after the template's.
#[must_use]
pub fn keys() -> Vec<String> {
    shapes().into_iter().map(|(key, _)| key).collect()
}

/// The JSON type name of a value.
fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Null => "null",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Whether a value is of the type a setting has — the shape's, with null where the rule
/// allows it and an integer where a number is wanted.
#[must_use]
pub fn fits(key: &str, value: &Value, type_: &str) -> bool {
    match (value, type_) {
        (Value::Null, _) => rule_for(key).nullable,
        (Value::Bool(_), "boolean")
        | (Value::String(_), "string")
        | (Value::Number(_), "number") => true,
        (Value::Number(n), "integer") => n.is_i64() || n.is_u64(),
        _ => false,
    }
}

/// The defaults, as the smallest file gives them: a callsign and nothing else, with one
/// keying section of each kind for the fields the default kind does not have (a kind's
/// fields with no default — a port — show the template's value).
fn defaults() -> Vec<(String, Value)> {
    let plain = Config::with_callsign("N0CALL");
    with_every_keying_kind(&serde_json::to_value(&plain).expect("serialise"))
}

/// The registry: every setting, described.
#[must_use]
pub fn schema() -> Vec<Setting> {
    let defaults = defaults();
    shapes()
        .into_iter()
        .filter(|(key, _)| key != "schema_version")
        .map(|(key, value)| {
            let rule = rule_for(&key);
            let default = defaults
                .iter()
                .find(|(k, _)| *k == key)
                .map_or(Value::Null, |(_, v)| v.clone());
            Setting {
                type_: type_name(&value),
                nullable: rule.nullable,
                default,
                scope: rule.scope,
                live: Config::is_live(&key),
                min: rule.min,
                max: rule.max,
                options: rule.options,
                why: rule.why,
                key,
            }
        })
        .collect()
}

/// How a value fails its rule, in a sentence, or nothing when it does not.
///
/// `null` passes: whether the setting takes it is the type's business, and serde refuses
/// it where it does not. A bound is checked on any number; an option list on the value as
/// JSON renders it, so `2300` matches `"2300"`.
#[must_use]
pub fn violation(key: &str, value: &Value) -> Option<String> {
    let rule = rule_for(key);
    if value.is_null() {
        return None;
    }
    if !rule.options.is_empty() {
        let text = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if !rule.options.contains(&text.as_str()) {
            return Some(format!(
                "{key} must be one of {}, not {value}: {}",
                rule.options.join(", "),
                rule.why
            ));
        }
    }
    if let Some(number) = value.as_f64() {
        let low = rule.min.is_some_and(|min| number < min);
        let high = rule.max.is_some_and(|max| number > max);
        if low || high {
            let bound = match (rule.min, rule.max) {
                (Some(min), Some(max)) if min == POSITIVE => format!("above 0 and at most {max}"),
                (Some(min), Some(max)) => format!("between {min} and {max}"),
                (Some(min), None) if min == POSITIVE => "above 0".to_owned(),
                (Some(min), None) => format!("at least {min}"),
                (None, Some(max)) => format!("at most {max}"),
                (None, None) => return None,
            };
            return Some(format!("{key} must be {bound}, not {value}: {}", rule.why));
        }
    }
    None
}

/// Every leaf of a serialised configuration, held to its rule.
///
/// # Errors
/// The first violation, as [`ConfigError::Invalid`].
pub fn check_bounds(document: &Value) -> Result<(), ConfigError> {
    let mut all = Vec::new();
    leaves("", document, &mut all);
    for (key, value) in &all {
        if let Some(reason) = violation(key, value) {
            return Err(ConfigError::Invalid(reason));
        }
    }
    Ok(())
}

/// The table at a value, making one there if there is none.
fn table_at(cursor: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !cursor.is_object() {
        *cursor = Value::Object(serde_json::Map::new());
    }
    match cursor {
        Value::Object(map) => map,
        _ => unreachable!("an object was just put there"),
    }
}

/// Set one dotted key in a JSON document, making the tables on the way.
pub fn set_leaf(document: &mut Value, key: &str, value: Value) {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((last, path)) = parts.split_last() else {
        return;
    };
    let mut cursor = document;
    for part in path {
        cursor = table_at(cursor)
            .entry((*part).to_owned())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    table_at(cursor).insert((*last).to_owned(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_names_a_setting_that_exists() {
        let keys = keys();
        for rule in RULES {
            let found = if rule.key.ends_with(".*") {
                keys.iter().any(|k| covers(rule.key, k))
            } else {
                keys.contains(&rule.key.to_owned())
            };
            assert!(
                found,
                "the rule for {} names a setting that does not exist",
                rule.key
            );
        }
    }

    #[test]
    fn the_template_sets_every_optional_field() {
        // a null leaf is an `Option` field nobody added to the template, and a setting the
        // registry cannot type
        for (key, value) in shapes() {
            assert!(
                !value.is_null(),
                "{key} is unset in the settings template: add it"
            );
        }
    }

    #[test]
    fn nullable_is_declared_for_exactly_the_fields_that_take_null() {
        // set each leaf to null in turn and ask serde: the registry's word must match
        let document = serde_json::to_value(template()).expect("serialise");
        let mut all = Vec::new();
        leaves("", &document, &mut all);
        for (key, _) in &all {
            if key == "schema_version" {
                continue;
            }
            let mut probe = document.clone();
            set_leaf(&mut probe, key, Value::Null);
            let takes_null = serde_json::from_value::<Config>(probe).is_ok();
            assert_eq!(
                takes_null,
                rule_for(key).nullable,
                "{key}: serde says nullable = {takes_null}, the registry says otherwise"
            );
        }
    }

    #[test]
    fn every_keying_kind_is_in_the_shapes() {
        // the list in `keying_kinds` is by hand; hold it to the kinds the file accepts
        for kind in ["none", "serial", "rigctld", "cat", "cm108"] {
            assert!(
                keying_kinds()
                    .iter()
                    .any(|k| serde_json::to_value(k).expect("serialise")["kind"] == kind),
                "keying kind {kind} is missing from the registry's shapes"
            );
        }
        assert_eq!(rule_for("ptt.kind").options.len(), keying_kinds().len());
        let keys = keys();
        for field in [
            "ptt.port",
            "ptt.line",
            "ptt.address",
            "ptt.device",
            "ptt.gpio",
        ] {
            assert!(
                keys.contains(&field.to_owned()),
                "{field} is not in the shapes"
            );
        }
    }

    #[test]
    fn the_bounds_written_here_are_the_ones_the_configuration_enforces() {
        // a value just outside a rule must be refused by `validate`, which runs
        // `check_bounds`; a rule that validation did not hold would be a promise the
        // profile loader keeps and the file does not
        let base = serde_json::to_value(template()).expect("serialise");
        let mut all = Vec::new();
        leaves("", &base, &mut all);
        let mut checked = 0;
        for setting in schema() {
            let Some((_, present)) = all.iter().find(|(k, _)| *k == setting.key) else {
                continue; // a keying field of another kind
            };
            let mut probes: Vec<Value> = Vec::new();
            if let Some(min) = setting.min {
                probes.push(serde_json::json!(if min == POSITIVE {
                    -1.0
                } else {
                    min - 1.0
                }));
            }
            if let Some(max) = setting.max {
                probes.push(serde_json::json!(max + 1.0));
            }
            if !setting.options.is_empty() {
                probes.push(match present {
                    Value::Number(_) => serde_json::json!(999_999),
                    _ => Value::String("not-an-option".into()),
                });
            }
            for probe in probes {
                let mut document = base.clone();
                set_leaf(&mut document, &setting.key, probe.clone());
                let refused = match serde_json::from_value::<Config>(document) {
                    Ok(config) => config.validate().is_err(),
                    Err(_) => true,
                };
                assert!(refused, "{} = {probe} was accepted", setting.key);
                checked += 1;
            }
        }
        assert!(checked > 10, "the bounds were not exercised");
    }

    #[test]
    fn the_bandwidth_and_mode_rules_follow_the_physical_layer() {
        let bandwidths = rule_for("radio.bandwidth").options;
        for text in bandwidths {
            let radio = crate::config::RadioSection {
                bandwidth: text.parse().expect("a number"),
                ..Default::default()
            };
            assert!(
                radio.params().is_some(),
                "{text} Hz is listed but not a waveform"
            );
        }
        let radio = crate::config::RadioSection {
            bandwidth: 2750,
            ..Default::default()
        };
        assert!(
            radio.params().is_none(),
            "the list is shorter than the waveforms"
        );
        assert_eq!(
            rule_for("radio.max_mode").max,
            Some((aether_link::AWGN_THRESHOLD_DB.len() - 1) as f64),
            "the mode bound is not the ladder's last rung"
        );
    }

    #[test]
    fn the_scopes_are_what_a_profile_needs() {
        assert!(is_portable("callsign"));
        assert!(is_portable("radio.max_key_s"));
        assert!(is_portable("audio.input"), "a device name travels, checked");
        assert!(is_portable("ptt.port"));
        assert!(is_portable("host.bind"));
        assert!(is_portable("panel.waterfall.gain_db"));
        assert!(!is_portable("control.token"), "a secret never leaves");
        assert!(!is_portable("control.bind"), "this installation's port");
        assert!(!is_portable("log.file"), "a path on this machine");
        assert!(!is_portable("record.dir"));
        assert!(!is_portable("sim.listen"), "the bench's wiring");
        assert!(!is_portable("sim.snr_db"));
        assert!(
            !is_portable("schema_version"),
            "the file's shape is not a setting"
        );
        assert_eq!(rule_for("audio.input").scope, Scope::Hardware);
    }

    #[test]
    fn the_schema_describes_every_setting_with_a_type_and_a_default() {
        let schema = schema();
        let key = schema
            .iter()
            .find(|s| s.key == "radio.max_key_s")
            .expect("max_key_s");
        assert_eq!(key.type_, "number");
        assert_eq!(key.default, serde_json::json!(30.0));
        assert!(key.live);
        assert_eq!(key.min, Some(POSITIVE));
        let modes = schema
            .iter()
            .find(|s| s.key == "radio.max_mode")
            .expect("max_mode");
        assert_eq!(modes.type_, "integer");
        let port = schema
            .iter()
            .find(|s| s.key == "ptt.port")
            .expect("a keying field");
        assert_eq!(port.type_, "string");
        assert_eq!(port.scope, Scope::Hardware);
        let civ = schema
            .iter()
            .find(|s| s.key == "ptt.civ_address")
            .expect("civ");
        assert!(civ.nullable);
        assert!(!schema.iter().any(|s| s.key == "schema_version"));
        for setting in &schema {
            assert!(
                matches!(setting.type_, "boolean" | "integer" | "number" | "string"),
                "{} has type {}",
                setting.key,
                setting.type_
            );
        }
    }

    #[test]
    fn a_violation_says_what_the_bound_is_and_why() {
        let text = violation("ptt.gpio", &serde_json::json!(9)).expect("out of range");
        assert!(text.contains("between 1 and 8"), "{text}");
        assert!(text.contains("DRA"), "{text}");
        let text = violation("radio.bandwidth", &serde_json::json!(2750)).expect("not an option");
        assert!(text.contains("2300, 500"), "{text}");
        assert!(violation("radio.bandwidth", &serde_json::json!(500)).is_none());
        assert!(violation("ptt.line", &serde_json::json!("dtr")).is_none());
        assert!(violation("ptt.line", &serde_json::json!("cts")).is_some());
        assert!(violation("audio.tx_level", &serde_json::json!(0.0)).is_some());
        assert!(violation("audio.tx_level", &serde_json::json!(1.0)).is_none());
        assert!(
            violation("operator.power_w", &Value::Null).is_none(),
            "unset is fine"
        );
        assert!(violation("radio.nobody_bounded_this", &serde_json::json!("x")).is_none());
    }

    #[test]
    fn a_value_fits_its_type_or_does_not() {
        assert!(fits("radio.max_mode", &serde_json::json!(8), "integer"));
        assert!(!fits("radio.max_mode", &serde_json::json!(8.5), "integer"));
        assert!(
            fits("radio.max_key_s", &serde_json::json!(8), "number"),
            "a whole number is a number"
        );
        assert!(
            fits("audio.input", &Value::Null, "string"),
            "declared nullable"
        );
        assert!(!fits("callsign", &Value::Null, "string"));
        assert!(!fits("radio.cw_id", &serde_json::json!("yes"), "boolean"));
    }
}
