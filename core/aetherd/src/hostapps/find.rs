//! Finding Winlink's programs on this computer.
//!
//! An operator may have several installs of one program (a copy kept for a second radio, a
//! test copy), so the finder looks everywhere an install shows itself and keeps every folder
//! that holds the program's executable: the folders its installer uses ([`Reader::usual`]), the
//! uninstall entries in the registry, the shortcuts in the Start menu and on the desktops, the
//! programs running now, and any folder the operator added. What the system says is gathered
//! once ([`Sources::gather`]); what is made of it ([`installs`]) is a function of that and of the
//! files in each folder, so a test hands the sources over. The paths the system gives are
//! Windows', and they are taken apart by Windows' rules wherever the finder is built (`parent`,
//! `file_name`, `join`), so a test's sources mean the same on every system.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{Files, Program, ProgramSetup, Reader};

/// How an install was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundBy {
    /// In the folder its installer uses.
    Usual,
    /// Its uninstall entry.
    Registry,
    /// A shortcut to it.
    Shortcut,
    /// It is running.
    Running,
    /// The operator named its folder.
    Added,
}

/// An uninstall entry, as the registry has it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallEntry {
    /// `DisplayName`.
    pub name: String,
    /// `InstallLocation`.
    pub location: Option<String>,
    /// `DisplayIcon`: often the executable, with `,0` after it.
    pub icon: Option<String>,
    /// `UninstallString`: the uninstaller, usually in the install's folder.
    pub uninstall: Option<String>,
}

/// What the system says, gathered once for a find.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    /// The folders an installer's usual folder is under: the system drive's root and the
    /// program folders.
    pub roots: Vec<PathBuf>,
    /// The uninstall entries.
    pub uninstall: Vec<UninstallEntry>,
    /// Each shortcut's target.
    pub shortcuts: Vec<PathBuf>,
    /// The executables of the programs running now.
    pub running: Vec<PathBuf>,
    /// The folders the operator added.
    pub added: Vec<PathBuf>,
    /// How long each stage of gathering took, in milliseconds: what to look at when a find
    /// is slow.
    pub took_ms: Vec<(&'static str, u64)>,
}

/// An install found, and what its files say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Install {
    /// The program.
    pub program: Program,
    /// Its own name, for the panel.
    pub program_name: &'static str,
    /// The folder.
    pub folder: String,
    /// Its executable.
    pub exe: String,
    /// How it was found.
    pub found_by: Vec<FoundBy>,
    /// Whether it is running now.
    pub running: bool,
    /// Its version.
    pub version: Option<String>,
    /// Every callsign it is set up for.
    pub callsigns: Vec<String>,
    /// Its setups, one profile each.
    pub setups: Vec<ProgramSetup>,
    /// What could not be read.
    pub notes: Vec<String>,
}

/// A path as Windows compares one: in any case, either slash, no trailing slash.
#[must_use]
pub fn same_path(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

/// The form [`same_path`] compares.
fn key(path: &Path) -> String {
    let text = path.display().to_string().replace('/', "\\").to_lowercase();
    text.trim_end_matches('\\').to_owned()
}

/// A path as a command line or an icon reference gives one: quotes, arguments after a quoted
/// path and an icon's `,0` taken off.
#[must_use]
pub fn path_in(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let path = if let Some(rest) = text.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        // an icon's index, or arguments after an unquoted path ending in .exe
        let lower = text.to_lowercase();
        match lower.find(".exe") {
            Some(at) => &text[..at + 4],
            None => text.split(',').next()?,
        }
    };
    // a folder named with its separator after it is the folder
    let path = path.trim().trim_end_matches(['\\', '/']);
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// Windows' separators: either slash.
const SEPARATORS: [char; 2] = ['\\', '/'];

/// A path's text when it is a Windows path, one that names a drive (`C:\RMS Express`) or a
/// share (`\\server\share`). The registry, the shortcuts and the processes give such paths, and
/// `std::path` splits them only on Windows; a folder the operator added on another system is
/// that system's, and is left to it.
fn windows_text(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let bytes = text.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    (drive || text.starts_with("\\\\")).then_some(text)
}

/// The folder a file is in. A drive's root keeps its separator: `C:` alone is the drive's
/// current folder, not its root.
fn parent(path: &Path) -> Option<PathBuf> {
    let Some(text) = windows_text(path) else {
        return path.parent().map(Path::to_path_buf);
    };
    let text = text.trim_end_matches(SEPARATORS);
    let folder = text[..text.rfind(SEPARATORS)?].trim_end_matches(SEPARATORS);
    let folder = if folder.ends_with(':') {
        &text[..=folder.len()]
    } else {
        folder
    };
    (!folder.is_empty()).then(|| PathBuf::from(folder))
}

/// A path's last part: a file's name.
fn file_name(path: &Path) -> Option<Cow<'_, str>> {
    let Some(text) = windows_text(path) else {
        return path.file_name().map(|n| n.to_string_lossy());
    };
    let name = text
        .trim_end_matches(SEPARATORS)
        .rsplit(SEPARATORS)
        .next()?;
    (!name.is_empty() && !name.ends_with(':')).then_some(Cow::Borrowed(name))
}

/// A file or folder in a folder: after a backslash, unless the folder ends in a separator or is
/// a drive alone (`C:` + `x` is `C:x`), as Windows joins them.
fn join(dir: &Path, name: &str) -> PathBuf {
    let Some(text) = windows_text(dir) else {
        return dir.join(name);
    };
    let glue = if text.ends_with(SEPARATORS) || text.len() == 2 {
        ""
    } else {
        "\\"
    };
    PathBuf::from(format!("{text}{glue}{name}"))
}

/// The folders a source names for a program: each is checked for the executable afterwards.
fn named_folders(reader: &Reader, sources: &Sources) -> Vec<(PathBuf, FoundBy)> {
    let mut out = Vec::new();
    for root in &sources.roots {
        for usual in reader.usual {
            out.push((join(root, usual), FoundBy::Usual));
        }
    }
    let ours = |name: &str| {
        let name = name.to_lowercase();
        reader
            .names
            .iter()
            .any(|n| name.contains(&n.to_lowercase()))
    };
    for entry in sources.uninstall.iter().filter(|e| ours(&e.name)) {
        if let Some(location) = entry.location.as_deref().and_then(path_in) {
            out.push((location, FoundBy::Registry));
        }
        for file in [&entry.icon, &entry.uninstall] {
            if let Some(dir) = file.as_deref().and_then(path_in).and_then(|p| parent(&p)) {
                out.push((dir, FoundBy::Registry));
            }
        }
    }
    let is_exe = |path: &Path| file_name(path).is_some_and(|n| n.eq_ignore_ascii_case(reader.exe));
    for target in sources.shortcuts.iter().filter(|t| is_exe(t)) {
        if let Some(dir) = parent(target) {
            out.push((dir, FoundBy::Shortcut));
        }
    }
    for exe in sources.running.iter().filter(|t| is_exe(t)) {
        if let Some(dir) = parent(exe) {
            out.push((dir, FoundBy::Running));
        }
    }
    for dir in &sources.added {
        out.push((dir.clone(), FoundBy::Added));
    }
    out
}

/// The installs of one program the sources show: each folder that holds its executable, once,
/// with every way it was found. `exists` says whether a file is there (the file system, or a
/// test's).
#[must_use]
pub fn folders_of(
    reader: &Reader,
    sources: &Sources,
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<(PathBuf, Vec<FoundBy>)> {
    let mut out: Vec<(PathBuf, Vec<FoundBy>)> = Vec::new();
    for (dir, by) in named_folders(reader, sources) {
        if !exists(&join(&dir, reader.exe)) {
            continue;
        }
        if let Some((_, found)) = out.iter_mut().find(|(d, _)| same_path(d, &dir)) {
            if !found.contains(&by) {
                found.push(by);
                found.sort();
            }
        } else {
            out.push((dir, vec![by]));
        }
    }
    out
}

/// A Windows executable's file version, from its version resource's fixed part
/// (`VS_FIXEDFILEINFO`, whose signature is `0xFEEF04BD`): `1.8.6.0`.
#[must_use]
pub fn exe_version(bytes: &[u8]) -> Option<String> {
    const SIGNATURE: [u8; 4] = [0xBD, 0x04, 0xEF, 0xFE];
    let at = bytes.windows(4).position(|w| w == SIGNATURE)?;
    let word = |offset: usize| -> Option<u32> {
        let b = bytes.get(at + offset..at + offset + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let ms = word(8)?;
    let ls = word(12)?;
    Some(format!(
        "{}.{}.{}.{}",
        ms >> 16,
        ms & 0xFFFF,
        ls >> 16,
        ls & 0xFFFF
    ))
}

/// Read an install's files: each of the reader's that is there.
fn read_files(reader: &Reader, dir: &Path) -> Files {
    let mut files = Files::new();
    for name in reader.files {
        if let Ok(bytes) = std::fs::read(join(dir, name)) {
            files.insert((*name).to_owned(), bytes);
        }
    }
    files
}

/// Every install the sources show, of every program, its files read.
#[must_use]
pub fn installs(sources: &Sources) -> Vec<Install> {
    let mut out = Vec::new();
    for program in Program::ALL {
        let reader = program.reader();
        for (dir, found_by) in folders_of(reader, sources, &|p| p.is_file()) {
            let exe = join(&dir, reader.exe);
            let reading = (reader.read)(&read_files(reader, &dir));
            let version = reading
                .version
                .clone()
                .or_else(|| std::fs::read(&exe).ok().and_then(|b| exe_version(&b)));
            out.push(install(program, &dir, found_by, reading, version, sources));
        }
    }
    out
}

/// An install read again, as a find would report it now: after its program's settings were
/// written, so that a plan sees them. `running` is what [`running_now`] said.
#[must_use]
pub fn reread(old: &Install, running: &[PathBuf]) -> Install {
    let reader = old.program.reader();
    let dir = PathBuf::from(&old.folder);
    let reading = (reader.read)(&read_files(reader, &dir));
    let version = reading.version.clone().or_else(|| old.version.clone());
    let sources = Sources {
        running: running.to_vec(),
        ..Sources::default()
    };
    install(
        old.program,
        &dir,
        old.found_by.clone(),
        reading,
        version,
        &sources,
    )
}

/// The executables of the programs running now: asked just before a program's settings are
/// written, since a find's answer may be minutes old.
#[must_use]
pub fn running_now() -> Vec<PathBuf> {
    system::running_executables()
}

/// An install as the finder reports it.
fn install(
    program: Program,
    dir: &Path,
    found_by: Vec<FoundBy>,
    reading: super::Reading,
    version: Option<String>,
    sources: &Sources,
) -> Install {
    let reader = program.reader();
    let exe = join(dir, reader.exe);
    let running = sources.running.iter().any(|r| same_path(r, &exe));
    let mut callsigns: Vec<String> = Vec::new();
    for call in reading.setups.iter().flat_map(|s| &s.callsigns) {
        if !callsigns.iter().any(|c| c.eq_ignore_ascii_case(call)) {
            callsigns.push(call.clone());
        }
    }
    Install {
        program,
        program_name: program.name(),
        folder: dir.display().to_string(),
        exe: exe.display().to_string(),
        found_by,
        running,
        version,
        callsigns,
        setups: reading.setups,
        notes: reading.notes,
    }
}

/// The target of a Windows shortcut (`.lnk`, the Shell Link format, MS-SHLLINK §2): the
/// local path its link information gives, or else its working folder and relative path joined.
#[must_use]
pub fn shortcut_target(bytes: &[u8]) -> Option<PathBuf> {
    const HAS_ID_LIST: u32 = 0x1;
    const HAS_LINK_INFO: u32 = 0x2;
    const HAS_NAME: u32 = 0x4;
    const HAS_RELATIVE_PATH: u32 = 0x8;
    const HAS_WORKING_DIR: u32 = 0x10;
    const IS_UNICODE: u32 = 0x80;
    let u16_at = |at: usize| -> Option<usize> {
        let b = bytes.get(at..at + 2)?;
        Some(usize::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let u32_at = |at: usize| -> Option<u32> {
        let b = bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if u32_at(0)? != 0x4C {
        return None;
    }
    let flags = u32_at(0x14)?;
    let mut at = 0x4C;
    if flags & HAS_ID_LIST != 0 {
        at += 2 + u16_at(at)?;
    }
    let mut local = None;
    if flags & HAS_LINK_INFO != 0 {
        let info = at;
        let size = usize::try_from(u32_at(info)?).ok()?;
        let header = u32_at(info + 4)?;
        let info_flags = u32_at(info + 8)?;
        if info_flags & 1 != 0 {
            let suffix_offset = usize::try_from(u32_at(info + 24)?).ok()?;
            local = if header >= 0x24 {
                let base = usize::try_from(u32_at(info + 28)?).ok()?;
                let suffix = usize::try_from(u32_at(info + 32)?).ok()?;
                let mut path = wide_string(bytes, info + base)?;
                path.push_str(&wide_string(bytes, info + suffix).unwrap_or_default());
                Some(path)
            } else {
                let base = usize::try_from(u32_at(info + 16)?).ok()?;
                let mut path = narrow_string(bytes, info + base)?;
                path.push_str(&narrow_string(bytes, info + suffix_offset).unwrap_or_default());
                Some(path)
            };
        }
        at = info + size;
    }
    if let Some(path) = local.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    // the string data that follows, in its fixed order
    let unicode = flags & IS_UNICODE != 0;
    let mut read = |present: bool| -> Option<Option<String>> {
        if !present {
            return Some(None);
        }
        let count = u16_at(at)?;
        let start = at + 2;
        let (text, len) = if unicode {
            let units: Vec<u16> = bytes
                .get(start..start + 2 * count)?
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            (String::from_utf16_lossy(&units), 2 * count)
        } else {
            let raw = bytes.get(start..start + count)?;
            (raw.iter().map(|b| char::from(*b)).collect(), count)
        };
        at = start + len;
        Some(Some(text))
    };
    let _name = read(flags & HAS_NAME != 0)?;
    let relative = read(flags & HAS_RELATIVE_PATH != 0)?;
    let working = read(flags & HAS_WORKING_DIR != 0)?;
    // a shortcut's relative path is Windows' whatever its form, so it is split by hand
    let file = relative
        .as_deref()
        .and_then(|r| r.rsplit(SEPARATORS).next())
        .filter(|f| !f.is_empty());
    match (working, file) {
        (Some(dir), Some(file)) if !dir.is_empty() => Some(join(Path::new(&dir), file)),
        _ => None,
    }
}

/// A NUL-ended string of single bytes at an offset (the system's code page; a path in it is
/// ASCII in practice, and anything else is kept as Latin-1).
fn narrow_string(bytes: &[u8], at: usize) -> Option<String> {
    let rest = bytes.get(at..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    Some(rest[..end].iter().map(|b| char::from(*b)).collect())
}

/// A NUL-ended UTF-16 string at an offset.
fn wide_string(bytes: &[u8], at: usize) -> Option<String> {
    let rest = bytes.get(at..)?;
    let units: Vec<u16> = rest
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// The `.lnk` files under a folder, a few levels down, a bounded number of them.
fn shortcuts_under(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    const MOST: usize = 4000;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MOST {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() && depth > 0 {
            shortcuts_under(&path, depth - 1, out);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            out.push(path);
        }
    }
}

impl Sources {
    /// Gather what this computer says, with the folders the operator added. On a system other
    /// than Windows only the added folders are looked in: Winlink's programs are Windows'.
    #[must_use]
    pub fn gather(added: &[PathBuf]) -> Self {
        let mut sources = Self {
            added: added.to_vec(),
            ..Self::default()
        };
        if !cfg!(windows) {
            return sources;
        }
        let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
        sources.roots.push(PathBuf::from(format!("{drive}\\")));
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = env(var) {
                sources.roots.push(dir);
            }
        }
        let mut clock = Stopwatch::start();
        let mut links = Vec::new();
        let start_menu = Path::new("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        for (var, below) in [
            ("APPDATA", Some(start_menu.clone())),
            ("ProgramData", Some(start_menu)),
            ("USERPROFILE", Some(PathBuf::from("Desktop"))),
            ("OneDrive", Some(PathBuf::from("Desktop"))),
            ("PUBLIC", Some(PathBuf::from("Desktop"))),
        ] {
            if let (Some(dir), Some(below)) = (env(var), below) {
                shortcuts_under(&dir.join(below), 3, &mut links);
            }
        }
        sources.shortcuts = links
            .iter()
            .filter_map(|l| std::fs::read(l).ok())
            .filter_map(|b| shortcut_target(&b))
            .collect();
        sources.took_ms.push(("shortcuts", clock.lap()));
        sources.uninstall = system::uninstall_entries();
        sources.took_ms.push(("registry", clock.lap()));
        sources.running = system::running_executables();
        sources.took_ms.push(("processes", clock.lap()));
        sources
    }
}

/// Milliseconds between laps.
struct Stopwatch(std::time::Instant);

impl Stopwatch {
    fn start() -> Self {
        Self(std::time::Instant::now())
    }

    /// The milliseconds since the last lap, or the start.
    fn lap(&mut self) -> u64 {
        let now = std::time::Instant::now();
        let ms = now.duration_since(self.0).as_millis();
        self.0 = now;
        u64::try_from(ms).unwrap_or(u64::MAX)
    }
}

/// A find: the installs, and how long each stage took.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The installs.
    pub installs: Vec<Install>,
    /// Each stage's milliseconds, gathering and reading.
    pub took_ms: Vec<(&'static str, u64)>,
}

/// Gather what this computer says, with the folders added, and read every install found.
#[must_use]
pub fn found(added: &[PathBuf]) -> Found {
    let sources = Sources::gather(added);
    let mut clock = Stopwatch::start();
    let installs = installs(&sources);
    let mut took_ms = sources.took_ms.clone();
    took_ms.push(("reading", clock.lap()));
    Found { installs, took_ms }
}

/// What only the system can say: the registry and the processes.
#[cfg(windows)]
mod system {
    use std::path::PathBuf;

    use super::UninstallEntry;

    /// The uninstall entries, the machine's (both views) and the user's.
    pub fn uninstall_entries() -> Vec<UninstallEntry> {
        use winreg::{
            RegKey,
            enums::{
                HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
            },
        };
        const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
        let mut out = Vec::new();
        for (hive, view) in [
            (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
            (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
            (HKEY_CURRENT_USER, 0),
        ] {
            let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(UNINSTALL, KEY_READ | view)
            else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                let Ok(key) = root.open_subkey_with_flags(&name, KEY_READ | view) else {
                    continue;
                };
                let Ok(display) = key.get_value::<String, _>("DisplayName") else {
                    continue;
                };
                let text = |v: &str| key.get_value::<String, _>(v).ok().filter(|s| !s.is_empty());
                let entry = UninstallEntry {
                    name: display,
                    location: text("InstallLocation"),
                    icon: text("DisplayIcon"),
                    uninstall: text("UninstallString"),
                };
                if !out.contains(&entry) {
                    out.push(entry);
                }
            }
        }
        out
    }

    /// The executables of the processes running now that this user can see.
    pub fn running_executables() -> Vec<PathBuf> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        system
            .processes()
            .values()
            .filter_map(|p| p.exe().map(std::path::Path::to_path_buf))
            .collect()
    }
}

/// Elsewhere: no registry, and Winlink's programs do not run.
#[cfg(not(windows))]
mod system {
    use std::path::PathBuf;

    use super::UninstallEntry;

    pub fn uninstall_entries() -> Vec<UninstallEntry> {
        Vec::new()
    }

    pub fn running_executables() -> Vec<PathBuf> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists_in(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| files.iter().any(|f| same_path(p, Path::new(f)))
    }

    #[test]
    fn every_way_of_finding_an_install_comes_to_one_entry() {
        let reader = Program::WinlinkExpress.reader();
        let sources = Sources {
            roots: vec![PathBuf::from("C:\\"), PathBuf::from("C:\\Program Files")],
            uninstall: vec![
                UninstallEntry {
                    name: "Winlink Express".into(),
                    location: Some("C:\\RMS Express\\".into()),
                    icon: Some("\"C:\\RMS Express\\RMS Express.exe\",0".into()),
                    uninstall: Some("\"C:\\RMS Express\\unins000.exe\"".into()),
                },
                UninstallEntry {
                    name: "Something else".into(),
                    location: Some("D:\\Other".into()),
                    ..UninstallEntry::default()
                },
            ],
            shortcuts: vec![
                PathBuf::from("c:\\rms express\\rms express.exe"),
                PathBuf::from("D:\\Radio\\Express 2\\RMS Express.exe"),
                PathBuf::from("C:\\VarAC\\VarAC.exe"),
            ],
            running: vec![PathBuf::from("D:\\Radio\\Express 2\\RMS Express.exe")],
            added: vec![PathBuf::from("E:\\Not An Install")],
            took_ms: Vec::new(),
        };
        let exists = exists_in(&[
            "C:\\RMS Express\\RMS Express.exe",
            "D:\\Radio\\Express 2\\RMS Express.exe",
            "D:\\Other\\RMS Express.exe",
        ]);
        let found = folders_of(reader, &sources, &exists);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].0, PathBuf::from("C:\\RMS Express"));
        let all = vec![FoundBy::Usual, FoundBy::Registry, FoundBy::Shortcut];
        assert_eq!(found[0].1, all);
        let second = vec![FoundBy::Shortcut, FoundBy::Running];
        assert_eq!(found[1].1, second);
        // another program's entry names nothing of this one's
        let trimode = folders_of(Program::RmsTrimode.reader(), &sources, &exists);
        assert!(trimode.is_empty(), "{trimode:?}");
    }

    #[test]
    fn a_path_is_taken_out_of_a_command_or_an_icon() {
        let exe = PathBuf::from("C:\\RMS Express\\RMS Express.exe");
        assert_eq!(
            path_in("\"C:\\RMS Express\\RMS Express.exe\",0"),
            Some(exe.clone())
        );
        assert_eq!(
            path_in("C:\\RMS Express\\RMS Express.exe,0"),
            Some(exe.clone())
        );
        assert_eq!(path_in("C:\\RMS Express\\RMS Express.exe /S"), Some(exe));
        assert_eq!(path_in("  "), None);
        assert!(same_path(
            Path::new("C:/RMS Express/"),
            Path::new("c:\\rms express")
        ));
    }

    #[test]
    fn a_windows_path_is_taken_apart_alike_on_every_system() {
        let p = PathBuf::from;
        let exe = p("C:\\RMS Express\\RMS Express.exe");
        assert_eq!(parent(&exe), Some(p("C:\\RMS Express")));
        assert_eq!(
            parent(&p("c:/rms express/rms express.exe")),
            Some(p("c:/rms express"))
        );
        assert_eq!(parent(&p("C:\\RMS Express.exe")), Some(p("C:\\")));
        assert_eq!(parent(&p("C:\\\\RMS Express.exe")), Some(p("C:\\")));
        let shared = p("\\\\radio\\share\\RMS Express.exe");
        assert_eq!(parent(&shared), Some(p("\\\\radio\\share")));
        assert_eq!(parent(&p("C:\\")), None);
        assert_eq!(file_name(&exe).as_deref(), Some("RMS Express.exe"));
        assert_eq!(file_name(&shared).as_deref(), Some("RMS Express.exe"));
        assert_eq!(file_name(&p("C:\\")), None);
        assert_eq!(join(&p("C:\\"), "RMS Express"), p("C:\\RMS Express"));
        assert_eq!(
            join(&p("C:\\Program Files"), "RMS\\RMS Trimode"),
            p("C:\\Program Files\\RMS\\RMS Trimode")
        );
        assert_eq!(join(&p("C:\\RMS\\"), "x"), p("C:\\RMS\\x"));
        assert_eq!(join(&p("C:"), "x"), p("C:x"));
        // a folder the operator added on another system is that system's
        let wine = Path::new("/home/op/.wine/drive_c/RMS Express");
        assert_eq!(join(wine, "RMS Express.exe"), wine.join("RMS Express.exe"));
        assert_eq!(parent(&wine.join("x")), Some(wine.to_path_buf()));
        // and on Windows, the same as the system's own
        if cfg!(windows) {
            for path in [
                "C:\\RMS Express\\RMS Express.exe",
                "C:/RMS Express/RMS Express.exe",
                "C:\\RMS Express.exe",
                "D:\\Radio\\Express 2\\RMS Express.exe",
                "\\\\?\\C:\\RMS Express\\RMS Express.exe",
            ] {
                let path = Path::new(path);
                assert_eq!(parent(path).as_deref(), path.parent(), "{path:?}");
                let name = path.file_name().map(|n| n.to_string_lossy());
                assert_eq!(file_name(path), name, "{path:?}");
            }
            for dir in [
                "C:\\",
                "C:\\Program Files",
                "C:\\RMS\\",
                "C:/RMS",
                "C:",
                "\\\\radio\\share",
            ] {
                let dir = Path::new(dir);
                assert_eq!(join(dir, "RMS Express"), dir.join("RMS Express"), "{dir:?}");
            }
        }
    }

    #[test]
    fn an_executables_version_is_read_from_its_version_resource() {
        let mut bytes = vec![0u8; 40];
        bytes.extend_from_slice(&[0xBD, 0x04, 0xEF, 0xFE, 0, 0, 1, 0]);
        bytes.extend_from_slice(&((1u32 << 16) | 8).to_le_bytes());
        bytes.extend_from_slice(&(6u32 << 16).to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        assert_eq!(exe_version(&bytes).as_deref(), Some("1.8.6.0"));
        assert_eq!(exe_version(&[0; 64]), None);
    }

    /// A shortcut as Windows writes one: the header, no item list, link information with a
    /// local path, and the relative path and working folder.
    fn shortcut(
        local: &str,
        unicode_info: bool,
        relative: Option<&str>,
        working: Option<&str>,
    ) -> Vec<u8> {
        let mut flags: u32 = 0x2 | 0x80;
        if relative.is_some() {
            flags |= 0x8;
        }
        if working.is_some() {
            flags |= 0x10;
        }
        let mut bytes = vec![0u8; 0x4C];
        bytes[0] = 0x4C;
        bytes[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        // link information
        let header: u32 = if unicode_info { 0x24 } else { 0x1C };
        let mut info = vec![0u8; header as usize];
        let base = info.len();
        let mut strings = Vec::new();
        if unicode_info {
            for unit in local.encode_utf16().chain([0]) {
                strings.extend_from_slice(&unit.to_le_bytes());
            }
            strings.extend_from_slice(&[0, 0]);
        } else {
            strings.extend_from_slice(local.as_bytes());
            strings.extend_from_slice(&[0, 0]);
        }
        let suffix = base + strings.len() - if unicode_info { 2 } else { 1 };
        info.extend_from_slice(&strings);
        let size = u32::try_from(info.len()).expect("size");
        info[0..4].copy_from_slice(&size.to_le_bytes());
        info[4..8].copy_from_slice(&header.to_le_bytes());
        info[8..12].copy_from_slice(&1u32.to_le_bytes());
        let base32 = u32::try_from(base).expect("base");
        let suffix32 = u32::try_from(suffix).expect("suffix");
        if unicode_info {
            info[28..32].copy_from_slice(&base32.to_le_bytes());
            info[32..36].copy_from_slice(&suffix32.to_le_bytes());
            info[24..28].copy_from_slice(&suffix32.to_le_bytes());
        } else {
            info[16..20].copy_from_slice(&base32.to_le_bytes());
            info[24..28].copy_from_slice(&suffix32.to_le_bytes());
        }
        bytes.extend_from_slice(&info);
        for text in [relative, working].into_iter().flatten() {
            let units: Vec<u16> = text.encode_utf16().collect();
            bytes.extend_from_slice(&u16::try_from(units.len()).expect("len").to_le_bytes());
            for unit in units {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }

    #[test]
    fn a_shortcuts_target_is_read() {
        let path = "C:\\RMS\\RMS Trimode\\RMS Trimode.exe";
        let narrow = shortcut(path, false, None, Some("C:\\RMS\\RMS Trimode"));
        assert_eq!(shortcut_target(&narrow), Some(PathBuf::from(path)));
        let wide = shortcut("D:\\Ünïcode\\RMS Express.exe", true, None, None);
        let expected = PathBuf::from("D:\\Ünïcode\\RMS Express.exe");
        assert_eq!(shortcut_target(&wide), Some(expected));
        // no local path: the relative path's file in the working folder
        let relative = "..\\..\\..\\RMS Express\\RMS Express.exe";
        let bare = shortcut("", false, Some(relative), Some("C:\\RMS Express"));
        let expected = PathBuf::from("C:\\RMS Express\\RMS Express.exe");
        assert_eq!(shortcut_target(&bare), Some(expected));
        assert_eq!(shortcut_target(b"not a shortcut"), None);
        assert_eq!(shortcut_target(&[]), None);
    }
}
