//! RMS Trimode's reader: its settings files to plain `ProgramSetup`s.
//!
//! A stub until trimode-bench writes it: the finder already finds Trimode's installs by
//! [`READER`], and this reads nothing yet.

use super::{Files, Program, Reader, Reading};

/// How RMS Trimode's installs are told and read.
pub static READER: Reader = Reader {
    program: Program::RmsTrimode,
    exe: "RMS Trimode.exe",
    usual: &["RMS\\RMS Trimode"],
    names: &["RMS Trimode"],
    files: &["RMS Trimode.ini"],
    read,
};

/// Read RMS Trimode's files.
#[must_use]
pub fn read(files: &Files) -> Reading {
    let mut reading = Reading::default();
    if !files.is_empty() {
        reading
            .notes
            .push("this version does not read RMS Trimode's settings yet".to_owned());
    }
    reading
}
