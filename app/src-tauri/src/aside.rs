//! What an installer set aside.
//!
//! Before the installer replaces the program, it makes sure nothing holds its files
//! (`windows/hooks.nsh`). A file something still holds after the shell and the modem were
//! asked to close — an antivirus, Explorer, a copy started as administrator, a process stuck
//! on its way out — is renamed beside itself as `aether-hf.exe.old` (`.old2` to `.old9`
//! while an earlier one is still held), which Windows allows for a program in use where it
//! refuses overwriting or deleting it, and the new file is written in its place. The next
//! start deletes them; one still in use stays for the start after.

use std::path::Path;

/// The names the installer gives what it sets aside, before the number.
const SET_ASIDE: [&str; 2] = ["aether-hf.exe.old", "aetherd.exe.old"];

/// Delete what the installer set aside beside this program. Only Windows installers set
/// anything aside.
pub fn clear_beside_program() {
    if cfg!(windows)
        && let Ok(own) = std::env::current_exe()
        && let Some(dir) = own.parent()
    {
        clear(dir);
    }
}

/// Delete what the installer set aside in `dir`; how many went.
fn clear(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| is_set_aside(&entry.file_name().to_string_lossy()))
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

/// Whether `name` is one the installer set aside: a name of [`SET_ASIDE`], then nothing or
/// one digit from 2 to 9. Windows file names ignore case.
fn is_set_aside(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SET_ASIDE.iter().any(|stem| {
        name.strip_prefix(stem)
            .is_some_and(|rest| matches!(rest.as_bytes(), [] | [b'2'..=b'9']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_installers_names_are_set_aside() {
        for name in [
            "aether-hf.exe.old",
            "aether-hf.exe.old2",
            "aether-hf.exe.old9",
            "AETHERD.EXE.OLD",
            "aetherd.exe.old5",
        ] {
            assert!(is_set_aside(name), "{name} is the installer's");
        }
        for name in [
            "aether-hf.exe",
            "aetherd.exe",
            "aether-hf.exe.old1",
            "aether-hf.exe.old10",
            "aether-hf.exe.older",
            "aetherd.exe.old.txt",
            "uninstall.exe.old",
            "station.toml.old",
        ] {
            assert!(!is_set_aside(name), "{name} is not the installer's");
        }
    }

    #[test]
    fn a_start_deletes_what_was_set_aside_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("aether-aside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let set_aside = ["aether-hf.exe.old", "aetherd.exe.old", "aetherd.exe.old2"];
        let kept = ["aether-hf.exe", "aetherd.exe", "uninstall.exe", "notes.old"];
        for name in set_aside.iter().chain(&kept) {
            std::fs::write(dir.join(name), b"x").expect("write");
        }
        assert_eq!(clear(&dir), set_aside.len());
        for name in set_aside {
            assert!(!dir.join(name).exists(), "{name} was set aside and goes");
        }
        for name in kept {
            assert!(dir.join(name).exists(), "{name} stays");
        }
        // nothing left to clear, and a folder that is not there clears nothing
        assert_eq!(clear(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(clear(&dir), 0);
    }
}
