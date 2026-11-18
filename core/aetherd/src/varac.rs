//! `VarAC` set up for Aether in one step: a `VarAC` profile beside the operator's own that
//! starts and reaches Aether as it would VARA HF, and an Aether profile that runs the way
//! `VarAC` expects.
//!
//! `VarAC`'s manual says a profile is a copy of `VarAC.ini` named `VarAC_<name>.ini` in its
//! folder, and *Switch profile* lists it. So the `VarAC` side is a copy of one of the
//! operator's profiles with only the settings `VarAC` starts and reaches its modem by
//! changed ([`wanted`], all in `[VARAHF_CONFIG]`); every other byte is kept. Nothing of VARA's
//! own is read or written.
//!
//! Found on the bench (2026-11-18, `VarAC` 15.0.18, scratch copies and a stand-in modem):
//! `VarAC` starts the modem with no arguments, in `VarAC`'s own folder, and before it does
//! writes a small `VARA.ini` beside the modem's path (its two ports and monitor mode off).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The `VarAC` profile this writes, beside the operator's own.
pub const PROFILE_FILE: &str = "VarAC_Aether.ini";
/// `VarAC`'s own profile: the one a new profile is copied from unless another is named.
pub const MAIN_FILE: &str = "VarAC.ini";
/// The Aether profile made for `VarAC`.
pub const AETHER_PROFILE: &str = "VarAC default";
/// The section of a `VarAC` profile that says how it starts and reaches its modem.
pub const SECTION: &str = "VARAHF_CONFIG";
/// The section that says how `VarAC` keys the radio.
pub const RIG_SECTION: &str = "RIG_CONTROL";
/// The settings the Aether profile takes ([`aether_changes`]), by number: a release that
/// changes them raises it, and a station whose profile was made with a lower one is offered
/// the change.
pub const TEMPLATE: u32 = 1;
/// The file beside the configuration that remembers what was set up.
pub const STATE_FILE: &str = "varac.json";
/// Files in `VarAC`'s folder named like a profile that are not one.
const NOT_PROFILES: &[&str] = &[
    "VarAC_cat_commands.ini",
    "VarAC_templates.ini",
    "VarAC_UI_languages.ini",
];

/// An INI file's bytes, edited in place: a setting's value is rewritten and every other byte
/// is kept, so the operator's profile comes through unchanged but for what is set. Bytes, not
/// text: a profile may hold a name or a place in any encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ini {
    bytes: Vec<u8>,
}

/// The section a line heads, if it heads one: `[NAME]`, a byte-order mark allowed before it.
fn section_of(line: &[u8]) -> Option<&[u8]> {
    let line = line.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(line);
    let line = line.trim_ascii();
    if line.len() >= 2 && line.first() == Some(&b'[') && line.last() == Some(&b']') {
        Some(line[1..line.len() - 1].trim_ascii())
    } else {
        None
    }
}

/// Whether a line heads a section, by name in any case.
fn heads(line: &[u8], section: &str) -> bool {
    let Some(name) = section_of(line) else {
        return false;
    };
    name.eq_ignore_ascii_case(section.as_bytes())
}

/// Where the `=` of a line that sets `key` is, by name in any case.
fn key_end(line: &[u8], key: &str) -> Option<usize> {
    let equals = line.iter().position(|b| *b == b'=')?;
    let name = line[..equals].trim_ascii();
    name.eq_ignore_ascii_case(key.as_bytes()).then_some(equals)
}

/// Where the line after one ending at `end` starts, and whether the file ended there without
/// a line end (so a line added there needs one first).
fn after(bytes: &[u8], end: usize) -> (usize, bool) {
    if end >= bytes.len() {
        (bytes.len(), true)
    } else if bytes[end] == b'\r' {
        (end + 2, false)
    } else {
        (end + 1, false)
    }
}

impl Ini {
    /// A file's bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The bytes as they stand.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Each line's start and end, its line end left out.
    fn lines(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        for (i, byte) in self.bytes.iter().enumerate() {
            if *byte == b'\n' {
                let end = if i > start && self.bytes[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                out.push((start, end));
                start = i + 1;
            }
        }
        if start < self.bytes.len() {
            out.push((start, self.bytes.len()));
        }
        out
    }

    /// Whether the file ends its lines as Windows does: a line added ends the same way.
    fn crlf(&self) -> bool {
        self.bytes.windows(2).any(|pair| pair == b"\r\n")
    }

    /// Whether the file has a section, by name in any case.
    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        for (start, end) in self.lines() {
            if heads(&self.bytes[start..end], section) {
                return true;
            }
        }
        false
    }

    /// Where a setting's value is: from the byte after its `=` to its line's end.
    fn find(&self, section: &str, key: &str) -> Option<(usize, usize)> {
        let mut inside = false;
        for (start, end) in self.lines() {
            let line = &self.bytes[start..end];
            if section_of(line).is_some() {
                inside = heads(line, section);
            } else if inside && let Some(equals) = key_end(line, key) {
                return Some((start + equals + 1, end));
            }
        }
        None
    }

    /// A setting's value, its spaces trimmed; names in any case.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let (from, to) = self.find(section, key)?;
        let value = self.bytes[from..to].trim_ascii();
        Some(String::from_utf8_lossy(value).into_owned())
    }

    /// Put `with` in place of the bytes from `from` to `to`.
    fn replace(&mut self, from: usize, to: usize, with: &[u8]) {
        let mut bytes = Vec::with_capacity(self.bytes.len() + with.len());
        bytes.extend_from_slice(&self.bytes[..from]);
        bytes.extend_from_slice(with);
        bytes.extend_from_slice(&self.bytes[to..]);
        self.bytes = bytes;
    }

    /// Where a line added to a section goes: after its last line that is not blank, and
    /// whether a line end must come first.
    fn section_end(&self, section: &str) -> Option<(usize, bool)> {
        let mut inside = false;
        let mut found = None;
        for (start, end) in self.lines() {
            let line = &self.bytes[start..end];
            if let Some(name) = section_of(line) {
                if inside {
                    break;
                }
                inside = name.eq_ignore_ascii_case(section.as_bytes());
                if inside {
                    found = Some(after(&self.bytes, end));
                }
            } else if inside && !line.trim_ascii().is_empty() {
                found = Some(after(&self.bytes, end));
            }
        }
        found
    }

    /// Set a setting, and say whether that changed the file. One not there yet goes at its
    /// section's end, and a section not there yet at the file's.
    pub fn set(&mut self, section: &str, key: &str, value: &str) -> bool {
        if let Some((from, to)) = self.find(section, key) {
            if self.bytes[from..to].trim_ascii() == value.as_bytes() {
                return false;
            }
            self.replace(from, to, value.as_bytes());
            return true;
        }
        let newline: &[u8] = if self.crlf() { b"\r\n" } else { b"\n" };
        let mut line = Vec::new();
        if let Some((at, end_first)) = self.section_end(section) {
            if end_first {
                line.extend_from_slice(newline);
            }
            line.extend_from_slice(key.as_bytes());
            line.push(b'=');
            line.extend_from_slice(value.as_bytes());
            line.extend_from_slice(newline);
            self.replace(at, at, &line);
        } else {
            if !self.bytes.is_empty() && !self.bytes.ends_with(b"\n") {
                line.extend_from_slice(newline);
            }
            line.push(b'[');
            line.extend_from_slice(section.as_bytes());
            line.push(b']');
            line.extend_from_slice(newline);
            line.extend_from_slice(key.as_bytes());
            line.push(b'=');
            line.extend_from_slice(value.as_bytes());
            line.extend_from_slice(newline);
            self.bytes.extend_from_slice(&line);
        }
        true
    }
}

/// One of the settings this keeps in a `VarAC` profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Wanted {
    /// The setting, as `VarAC` names it in `[VARAHF_CONFIG]`.
    pub key: &'static str,
    /// What it must say.
    pub value: String,
    /// Set only when the profile is made: a choice the operator may change afterwards in
    /// `VarAC`, which bringing the profile up to date leaves alone.
    pub at_creation_only: bool,
}

/// The settings a `VarAC` profile needs to start and reach Aether as it would VARA HF: the
/// application's path, Aether's two ports, and no monitor modem (a monitor path starts a
/// second modem beside the first, VARA's, on the same sound card).
#[must_use]
pub fn wanted(app: &Path, host_port: u16, kiss_port: u16) -> Vec<Wanted> {
    let always = |key, value: String| Wanted {
        key,
        value,
        at_creation_only: false,
    };
    vec![
        always("VaraModemType", "VaraHF".to_owned()),
        always("VarahfMainPath", app.display().to_string()),
        always("VarahfMainHost", "127.0.0.1".to_owned()),
        always("VarahfMainPort", host_port.to_string()),
        always("VarahfEnableKissInterface", "ON".to_owned()),
        always("VarahfMainKissPort", kiss_port.to_string()),
        always("VarahfMonitorPath", String::new()),
        Wanted {
            key: "VarahfLaunchOnModemConnect",
            value: "ON".to_owned(),
            at_creation_only: true,
        },
    ]
}

/// A port from an address such as `127.0.0.1:8300`, or the one given when it names none.
#[must_use]
pub fn port_of(bind: &str, otherwise: u16) -> u16 {
    bind.rsplit(':')
        .next()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|p| *p != 0)
        .unwrap_or(otherwise)
}

/// A setting a `VarAC` profile has that is not what this version would write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Difference {
    /// The setting.
    pub key: &'static str,
    /// What the profile says now; `None` when it does not have it.
    pub now: Option<String>,
    /// What it should say.
    pub wanted: String,
}

/// Whether a value says what is wanted: a path in any case and with either slash, as Windows
/// reads one; anything else without regard to case.
fn same(key: &str, value: &str, wanted: &str) -> bool {
    let value = value.trim();
    if key.eq_ignore_ascii_case("VarahfMainPath") {
        value.replace('/', "\\").to_lowercase() == wanted.replace('/', "\\").to_lowercase()
    } else {
        value.eq_ignore_ascii_case(wanted)
    }
}

/// What a `VarAC` profile says differently from what this version would write, leaving out
/// the choices that are the operator's once it is made.
#[must_use]
pub fn differences(ini: &Ini, wanted: &[Wanted]) -> Vec<Difference> {
    wanted
        .iter()
        .filter(|w| !w.at_creation_only)
        .filter_map(|w| {
            let now = ini.get(SECTION, w.key);
            if now.as_deref().is_some_and(|v| same(w.key, v, &w.value)) {
                None
            } else {
                Some(Difference {
                    key: w.key,
                    now,
                    wanted: w.value.clone(),
                })
            }
        })
        .collect()
}

/// Who keys the radio, as a `VarAC` profile's `RigPTTControlType` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Keying {
    /// `VarAC` does (CAT, `OmniRig`, `FLRig`, `Hamlib`, DTR/RTS): Aether asks it to, as a host
    /// program that keys the radio.
    Program,
    /// `VarAC` does not (VOX, or none): the modem keys the radio, as VARA did, and Aether
    /// keeps its own keying.
    Modem,
    /// The profile does not say.
    Unknown,
}

/// Who keys the radio in a `VarAC` profile.
#[must_use]
pub fn keying(ini: &Ini) -> Keying {
    let Some(kind) = ini.get(RIG_SECTION, "RigPTTControlType") else {
        return Keying::Unknown;
    };
    match kind.trim().to_ascii_uppercase().as_str() {
        "" => Keying::Unknown,
        "VOX" | "NONE" => Keying::Modem,
        _ => Keying::Program,
    }
}

/// What the Aether profile for `VarAC` sets over the station's own settings, as dotted keys:
/// the host interface and the KISS port on (`VarAC`'s broadcasts and its relay notifications
/// travel as KISS frames), and — when `VarAC` keys the radio and the profile does not key by
/// the host program already — keying by the host program with no rules profile, as Setup does
/// when a host program keys (Aether reads no dial then). A profile keyed by the host program
/// already keeps its own lead and rules.
#[must_use]
pub fn aether_changes(keying: Keying, host_keyed: bool) -> serde_json::Value {
    let mut changes = serde_json::json!({
        "host.enabled": true,
        "kiss.enabled": true,
    });
    if keying == Keying::Program && !host_keyed {
        changes["ptt.kind"] = serde_json::json!("host");
        changes["regulatory.profile"] = serde_json::json!("none");
    }
    changes
}

/// Where `VarAC` is usually installed: its installer's default, `C:\VarAC`, then the program
/// folders. Nothing on a system other than Windows.
#[must_use]
pub fn usual_folders() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let mut out = vec![PathBuf::from(format!("{drive}\\VarAC"))];
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(var) {
            out.push(PathBuf::from(dir).join("VarAC"));
        }
    }
    out
}

/// Whether a folder is `VarAC`'s: its own profile is there.
#[must_use]
pub fn is_varac_folder(dir: &Path) -> bool {
    dir.join(MAIN_FILE).is_file()
}

/// Whether a file's name is that of a profile of the operator's: `VarAC.ini`, or
/// `VarAC_<name>.ini` other than `VarAC`'s own data files and the one this writes.
fn is_profile_name(name: &str) -> bool {
    if name.eq_ignore_ascii_case(MAIN_FILE) {
        return true;
    }
    let is_ini = Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ini"));
    let prefix = name.get(..6).unwrap_or_default();
    is_ini
        && prefix.eq_ignore_ascii_case("VarAC_")
        && !name.eq_ignore_ascii_case(PROFILE_FILE)
        && !NOT_PROFILES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// Whether a file is a `VarAC` profile: it has the modem's section.
fn has_modem_section(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    Ini::new(bytes).has_section(SECTION)
}

/// The profiles in `VarAC`'s folder a new one can be copied from: `VarAC.ini` first (in
/// lower case its `.` sorts before every `_`), then each `VarAC_<name>.ini` with a modem
/// section, never the one this writes.
#[must_use]
pub fn profiles_in(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_profile_name(name))
        .filter(|name| has_modem_section(&dir.join(name)))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// Read a profile in `VarAC`'s folder.
///
/// # Errors
/// If the name is not one of its profiles' or the file cannot be read.
pub fn read_profile(dir: &Path, name: &str) -> Result<Ini, String> {
    if name.contains(['/', '\\']) || !is_profile_name(name) {
        return Err(format!("{name} is not a VarAC profile"));
    }
    let path = dir.join(name);
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(Ini::new(bytes))
}

/// What making or bringing up to date the `VarAC` profile did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Written {
    /// Where the profile is.
    pub path: String,
    /// Whether it was made now, rather than brought up to date.
    pub created: bool,
    /// The settings that changed: none when it was already up to date.
    pub changed: Vec<String>,
    /// The copy of what it said before, when it was changed.
    pub backup: Option<String>,
}

/// Why a file could not be written, with what to do when Windows refused.
fn write_error(path: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "Windows did not let Aether write {} ({error}). A folder under Program Files \
             needs an administrator to write in; VarAC's own default, C:\\VarAC, does not.",
            path.display()
        )
    } else {
        format!("cannot write {}: {error}", path.display())
    }
}

/// Write a file whole or not at all: to a new file first, then over the old one.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, bytes).map_err(|e| write_error(path, &e))?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        write_error(path, &e)
    })
}

/// Make the `VarAC` profile in `VarAC`'s folder from one of the operator's (`source`), or bring
/// the one there up to date. Only the settings in `wanted` change; every other byte is kept,
/// and the profile as it was before a change is kept beside it as `VarAC_Aether.ini.bak`.
///
/// # Errors
/// If `source` is not one of the operator's profiles, or a file cannot be read or written.
pub fn write_profile(dir: &Path, source: &str, wanted: &[Wanted]) -> Result<Written, String> {
    let path = dir.join(PROFILE_FILE);
    let before = match std::fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let created = before.is_none();
    let mut ini = match &before {
        Some(bytes) => Ini::new(bytes.clone()),
        None => read_profile(dir, source)?,
    };
    let differ = differences(&ini, wanted);
    let mut changed = Vec::new();
    for w in wanted {
        let due = created || differ.iter().any(|d| d.key == w.key);
        if due && ini.set(SECTION, w.key, &w.value) {
            changed.push(w.key.to_owned());
        }
    }
    if !created && changed.is_empty() {
        return Ok(Written {
            path: path.display().to_string(),
            created,
            changed,
            backup: None,
        });
    }
    let backup = match &before {
        Some(bytes) => {
            let kept = dir.join(format!("{PROFILE_FILE}.bak"));
            std::fs::write(&kept, bytes).map_err(|e| write_error(&kept, &e))?;
            Some(kept.display().to_string())
        }
        None => None,
    };
    write_atomically(&path, ini.bytes())?;
    Ok(Written {
        path: path.display().to_string(),
        created,
        changed,
        backup,
    })
}

/// Where Aether's application is, for `VarAC` to start: what the desktop application said
/// when it started this daemon (`AETHER_APP`), or else the application beside the daemon,
/// where the installer puts both.
#[must_use]
pub fn app_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AETHER_APP").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let name = if cfg!(windows) {
        "aether-hf.exe"
    } else {
        "aether-hf"
    };
    let beside = std::env::current_exe().ok()?.parent()?.join(name);
    beside.is_file().then_some(beside)
}

/// What the station remembers of `VarAC`, in `varac.json` beside its configuration — not a
/// setting: nothing here changes how the modem runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    /// The folder `VarAC` was found or set up in.
    #[serde(default)]
    pub folder: Option<String>,
    /// The offer the operator dismissed, by its key: that offer is not made again; another
    /// (a later release's change) is.
    #[serde(default)]
    pub dismissed: Option<String>,
    /// The [`TEMPLATE`] the Aether profile was last made with; 0 before it was.
    #[serde(default)]
    pub template: u32,
}

/// `VarAC` as the daemon knows it: what it remembers and where, where Aether's application is
/// and where `VarAC` is looked for.
#[derive(Debug, Clone, Default)]
pub struct Setup {
    /// The state file; `None` for a daemon without a configuration file.
    path: Option<PathBuf>,
    /// What it holds.
    pub remembered: Remembered,
    /// Aether's application, for `VarAC` to start; `None` when it cannot be found (a
    /// development build run on its own).
    pub app: Option<PathBuf>,
    /// Where `VarAC` is looked for when no folder is remembered or asked for.
    pub usual: Vec<PathBuf>,
}

impl Setup {
    /// What is remembered in a state file, with the application and the usual folders found
    /// on this computer.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let text = path.as_deref().map(std::fs::read_to_string);
        let remembered: Remembered = if let Some(Ok(text)) = text {
            serde_json::from_str(&text).unwrap_or_default()
        } else {
            Remembered::default()
        };
        Self {
            path,
            remembered,
            app: app_path(),
            usual: usual_folders(),
        }
    }

    /// Write what is remembered.
    ///
    /// # Errors
    /// If there is no state file or it cannot be written.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("this modem keeps no files".to_owned());
        };
        let text = serde_json::to_string_pretty(&self.remembered).unwrap_or_default();
        write_atomically(path, text.as_bytes())
    }

    /// `VarAC`'s folder: the one asked for, else the one remembered, else the first usual one
    /// that holds `VarAC`; `None` when none does.
    #[must_use]
    pub fn folder(&self, asked: Option<&str>) -> Option<PathBuf> {
        if let Some(asked) = asked {
            let dir = PathBuf::from(asked.trim());
            return is_varac_folder(&dir).then_some(dir);
        }
        let remembered = self.remembered.folder.as_deref().map(PathBuf::from);
        let mut candidates = remembered.into_iter().chain(self.usual.iter().cloned());
        candidates.find(|dir| is_varac_folder(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile as `VarAC` writes one: LF line ends, made-up values.
    const PROFILE: &str = "[DEBUG]\nDebugMode=OFF\n\n[RIG_CONTROL]\nRigPTTControlType=CAT\n\
        RigFreqControlType=CAT\n\n[VARAHF_CONFIG]\nVaraModemType=VaraHF\n\
        VarahfMainPath=C:\\VARA\\VARA.exe\nVarahfMainPort=8300\nVarahfMainHost=127.0.0.1\n\
        VarahfEnableKissInterface=OFF\nVarahfMainKissPort=8100\n\
        VarahfMonitorPath=C:\\VARA Monitor\\VARA.exe\nVarahfLaunchOnModemConnect=OFF\n\
        LogVARACommands=OFF\n\n\n[MY_INFO]\nMyCall=N0CALL\nMyQTH=Somewhere ñ\n";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-varac-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn app() -> PathBuf {
        PathBuf::from("C:\\Users\\op\\AppData\\Local\\Aether HF\\aether-hf.exe")
    }

    fn ini() -> Ini {
        Ini::new(PROFILE.as_bytes().to_vec())
    }

    #[test]
    fn a_setting_is_changed_in_place_and_nothing_else_moves() {
        let mut ini = ini();
        assert!(ini.set(SECTION, "VarahfMainPort", "8400"));
        let expected = PROFILE.replace("VarahfMainPort=8300", "VarahfMainPort=8400");
        assert_eq!(ini.bytes(), expected.as_bytes());
        // the same value again, its name in another case, is no change
        assert!(!ini.set(SECTION, "varahfmainport", "8400"));
        assert_eq!(ini.get(SECTION, "VARAHFMAINPORT").as_deref(), Some("8400"));
        assert_eq!(ini.get("my_info", "MyQTH").as_deref(), Some("Somewhere ñ"));
        assert_eq!(ini.get(SECTION, "MyCall"), None, "another section's");
    }

    #[test]
    fn a_missing_setting_goes_at_its_sections_end_and_a_missing_section_at_the_files() {
        let mut ini = ini();
        assert!(ini.set(SECTION, "VarahfMonitorPort", "8350"));
        let expected = PROFILE.replace(
            "LogVARACommands=OFF\n",
            "LogVARACommands=OFF\nVarahfMonitorPort=8350\n",
        );
        assert_eq!(ini.bytes(), expected.as_bytes());
        let mut bare = Ini::new(b"[A]\nx=1".to_vec());
        assert!(bare.set("A", "y", "2"));
        assert_eq!(bare.bytes(), b"[A]\nx=1\ny=2\n");
        assert!(bare.set("B", "z", "3"));
        assert_eq!(bare.bytes(), b"[A]\nx=1\ny=2\n[B]\nz=3\n");
    }

    #[test]
    fn windows_line_ends_are_kept_and_used() {
        let text = "[VARAHF_CONFIG]\r\nVarahfMainPort=8300\r\n\r\n[OTHER]\r\nA=1\r\n";
        let mut ini = Ini::new(text.as_bytes().to_vec());
        assert!(ini.set(SECTION, "VarahfMainPort", "8301"));
        assert!(ini.set(SECTION, "VarahfMainKissPort", "8100"));
        let expected = text.replace(
            "VarahfMainPort=8300\r\n",
            "VarahfMainPort=8301\r\nVarahfMainKissPort=8100\r\n",
        );
        assert_eq!(ini.bytes(), expected.as_bytes());
    }

    #[test]
    fn the_profile_is_compared_with_what_this_version_would_write() {
        let ini = ini();
        let wanted = wanted(&app(), 8300, 8100);
        let keys: Vec<&str> = differences(&ini, &wanted).iter().map(|d| d.key).collect();
        // the ports agree, and starting the modem is the operator's choice once made
        let expected = [
            "VarahfMainPath",
            "VarahfEnableKissInterface",
            "VarahfMonitorPath",
        ];
        assert_eq!(keys, expected);
        let mut done = ini.clone();
        for w in &wanted {
            done.set(SECTION, w.key, &w.value);
        }
        let none = differences(&done, &wanted);
        assert!(none.is_empty(), "{none:?}");
        // a path in another case and with the other slash is the same path
        let other = "c:/users/OP/appdata/local/aether hf/AETHER-HF.EXE";
        done.set(SECTION, "VarahfMainPath", other);
        let same = differences(&done, &wanted);
        assert!(same.is_empty(), "{same:?}");
    }

    #[test]
    fn who_keys_the_radio_is_read_from_the_profile() {
        assert_eq!(keying(&ini()), Keying::Program);
        let vox = Ini::new(b"[RIG_CONTROL]\nRigPTTControlType=VOX\n".to_vec());
        assert_eq!(keying(&vox), Keying::Modem);
        assert_eq!(keying(&Ini::new(Vec::new())), Keying::Unknown);
        let program = aether_changes(Keying::Program, false);
        assert_eq!(program["ptt.kind"], "host");
        assert_eq!(program["regulatory.profile"], "none");
        let kept = aether_changes(Keying::Program, true);
        assert!(kept.get("ptt.kind").is_none(), "{kept}");
        let modem = aether_changes(Keying::Modem, false);
        assert!(modem.get("ptt.kind").is_none(), "{modem}");
        assert_eq!(modem["kiss.enabled"], true);
    }

    #[test]
    fn the_profiles_listed_are_the_operators_own() {
        let dir = temp_dir("list");
        let names = [
            "VarAC.ini",
            "VarAC_Kenwood.ini",
            "VarAC_Aether.ini",
            "VarAC_cat_commands.ini",
        ];
        for name in names {
            std::fs::write(dir.join(name), PROFILE).expect("write");
        }
        std::fs::write(dir.join("VarAC_notes.ini"), "[OTHER]\nA=1\n").expect("write");
        assert_eq!(profiles_in(&dir), ["VarAC.ini", "VarAC_Kenwood.ini"]);
        assert!(is_varac_folder(&dir));
        assert!(read_profile(&dir, "..\\VarAC.ini").is_err());
        assert!(read_profile(&dir, "VarAC_cat_commands.ini").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_profile_is_made_from_the_operators_and_then_kept_up_to_date() {
        let dir = temp_dir("write");
        std::fs::write(dir.join(MAIN_FILE), PROFILE).expect("write");
        let wanted = wanted(&app(), 8300, 8100);
        let made = write_profile(&dir, MAIN_FILE, &wanted).expect("made");
        assert!(made.created);
        assert_eq!(made.backup, None);
        let ini = Ini::new(std::fs::read(dir.join(PROFILE_FILE)).expect("read"));
        let path = app().display().to_string();
        assert_eq!(ini.get(SECTION, "VarahfMainPath"), Some(path));
        assert_eq!(ini.get(SECTION, "VarahfMonitorPath").as_deref(), Some(""));
        let launch = ini.get(SECTION, "VarahfLaunchOnModemConnect");
        assert_eq!(launch.as_deref(), Some("ON"));
        assert_eq!(ini.get("MY_INFO", "MyQTH").as_deref(), Some("Somewhere ñ"));
        // the operator's own profile is not touched
        let own = std::fs::read(dir.join(MAIN_FILE)).expect("read");
        assert_eq!(own, PROFILE.as_bytes());

        // the operator turns starting the modem off: an update leaves that alone
        let mut edited = ini.clone();
        edited.set(SECTION, "VarahfLaunchOnModemConnect", "OFF");
        std::fs::write(dir.join(PROFILE_FILE), edited.bytes()).expect("write");
        let same = write_profile(&dir, MAIN_FILE, &wanted).expect("up to date");
        assert!(!same.created);
        assert!(same.changed.is_empty(), "{:?}", same.changed);
        assert_eq!(same.backup, None);

        // Aether moved and its port changed: those two, and a copy of what was there
        let moved = PathBuf::from("D:\\Aether HF\\aether-hf.exe");
        let later = super::wanted(&moved, 8310, 8100);
        let updated = write_profile(&dir, MAIN_FILE, &later).expect("updated");
        assert_eq!(updated.changed, ["VarahfMainPath", "VarahfMainPort"]);
        assert!(updated.backup.is_some());
        let now = Ini::new(std::fs::read(dir.join(PROFILE_FILE)).expect("read"));
        assert_eq!(now.get(SECTION, "VarahfMainPort").as_deref(), Some("8310"));
        let launch = now.get(SECTION, "VarahfLaunchOnModemConnect");
        assert_eq!(launch.as_deref(), Some("OFF"));
        let kept = std::fs::read(dir.join(format!("{PROFILE_FILE}.bak"))).expect("backup");
        assert_eq!(kept, edited.bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_remembered_comes_back_and_the_folder_is_found() {
        let dir = temp_dir("remember");
        let varac = dir.join("VarAC");
        std::fs::create_dir_all(&varac).expect("dir");
        std::fs::write(varac.join(MAIN_FILE), PROFILE).expect("write");
        let mut setup = Setup::open(Some(dir.join(STATE_FILE)));
        assert_eq!(setup.remembered, Remembered::default());
        setup.usual = vec![dir.join("nowhere"), varac.clone()];
        assert_eq!(setup.folder(None), Some(varac.clone()));
        let not_it = dir.display().to_string();
        assert_eq!(setup.folder(Some(not_it.as_str())), None);
        setup.remembered.folder = Some(varac.display().to_string());
        setup.remembered.template = TEMPLATE;
        setup.save().expect("save");
        let again = Setup::open(Some(dir.join(STATE_FILE)));
        assert_eq!(again.remembered, setup.remembered);
        assert_eq!(port_of("127.0.0.1:8310", 8300), 8310);
        assert_eq!(port_of("nonsense", 8300), 8300);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
