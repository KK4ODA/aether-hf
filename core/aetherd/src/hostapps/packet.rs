//! RMS Packet's reader: its settings files to plain `ProgramSetup`s.
//!
//! A stub until packet-bench writes it: the finder already finds RMS Packet's installs by
//! [`READER`], and this reads nothing yet.

use super::{Files, Program, Reader, Reading};

/// How RMS Packet's installs are told and read.
pub static READER: Reader = Reader {
    program: Program::RmsPacket,
    exe: "RMS Packet.exe",
    usual: &["RMS\\RMS Packet"],
    names: &["RMS Packet"],
    files: &["RMS Packet.ini"],
    read,
};

/// Read RMS Packet's files.
#[must_use]
pub fn read(files: &Files) -> Reading {
    let mut reading = Reading::default();
    if !files.is_empty() {
        reading
            .notes
            .push("this version does not read RMS Packet's settings yet".to_owned());
    }
    reading
}
