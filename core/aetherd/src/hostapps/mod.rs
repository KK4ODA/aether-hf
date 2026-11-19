//! The host-program finder (ADR-0133): Winlink's programs found on this computer, their
//! settings read, and a setup of one turned into an Aether profile.
//!
//! An operator moving from VARA has Winlink Express, RMS Trimode or RMS Packet set up already:
//! a radio, its CAT port and keying, a TNC address and port. The finder looks for each install
//! ([`find`]), reads its settings files with that program's [`Reader`] — a pure function over
//! the files' bytes, one per program — and [`import`] makes a profile of what it read. Nothing
//! is ever written to another program's files: what the program must change to reach Aether is
//! said, not done.
//!
//! A reader reads only the keys it needs. A program's files also hold the operator's address,
//! telephone, position, e-mail and password; none of those keys is read, and a [`Reading`] and
//! its notes never carry a value of one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub mod find;
pub mod import;
pub mod packet;
pub mod state;
pub mod trimode;
pub mod winlink_express;

/// The programs the finder knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Program {
    /// Winlink Express (once RMS Express), the Winlink client.
    WinlinkExpress,
    /// RMS Trimode, the HF gateway.
    RmsTrimode,
    /// RMS Packet, the VHF/UHF packet gateway.
    RmsPacket,
}

impl Program {
    /// Every program the finder knows, in the order it lists them.
    pub const ALL: [Self; 3] = [Self::WinlinkExpress, Self::RmsTrimode, Self::RmsPacket];

    /// The program's own name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::WinlinkExpress => "Winlink Express",
            Self::RmsTrimode => "RMS Trimode",
            Self::RmsPacket => "RMS Packet",
        }
    }

    /// How its installs are told and read.
    #[must_use]
    pub fn reader(self) -> &'static Reader {
        match self {
            Self::WinlinkExpress => &winlink_express::READER,
            Self::RmsTrimode => &trimode::READER,
            Self::RmsPacket => &packet::READER,
        }
    }
}

/// A program's settings files, as the finder hands them to a reader: each of the reader's
/// [`Reader::files`] that exists, keyed exactly as the reader lists it.
pub type Files = BTreeMap<String, Vec<u8>>;

/// What the finder needs to know of a program: how to tell its folder and what to read there.
#[derive(Debug)]
pub struct Reader {
    /// The program.
    pub program: Program,
    /// Its executable's file name: a folder holding it is an install.
    pub exe: &'static str,
    /// Where its installer puts it, under the system drive (`RMS\RMS Trimode`).
    pub usual: &'static [&'static str],
    /// What an uninstall entry's or a shortcut's name contains when it is this program, in any
    /// case.
    pub names: &'static [&'static str],
    /// The files read, relative to the install's folder.
    pub files: &'static [&'static str],
    /// Read them. Pure: the same files always read the same.
    pub read: fn(&Files) -> Reading,
}

/// What a reader made of an install's files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Reading {
    /// The program's version, when its files say it; otherwise the finder reads the
    /// executable's.
    pub version: Option<String>,
    /// One for each thing an operator would make a profile of: a callsign of Winlink Express's,
    /// a gateway's setup.
    pub setups: Vec<ProgramSetup>,
    /// What could not be read, in words. Never a value.
    pub notes: Vec<String>,
}

/// One setup of a program's, in plain terms: who it runs as, its radio and its TNCs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProgramSetup {
    /// The program's own name for it: a callsign, a channel set.
    pub name: String,
    /// The callsigns it runs as; the first is the one it runs as now.
    pub callsigns: Vec<String>,
    /// The program's own words for its mode (`Vara HF P2P`, `Vara FM Wide`).
    pub session: Option<String>,
    /// The bandwidth the program asks for, its own figure (500, 2300, 2750), unmapped.
    pub bandwidth_hz: Option<u32>,
    /// The frequency it works on, when it has one: a gateway's channel, a client's last.
    pub frequency_hz: Option<u64>,
    /// An automatic-control station (a gateway).
    pub gateway: bool,
    /// The radio as the program controls it; `None` when it controls none (Manual).
    pub radio: Option<Radio>,
    /// Every modem or TNC it points at.
    pub tncs: Vec<Tnc>,
}

/// A radio as a program controls it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Radio {
    /// The program's name for it: `Yaesu FTDX10`, `Icom IC-7300`.
    pub rig: Option<String>,
    /// The serial port its CAT commands go to; `None` when there is none.
    pub cat_port: Option<String>,
    /// That port's speed.
    pub cat_baud: Option<u32>,
    /// An Icom's CI-V address.
    pub icom_address: Option<u8>,
    /// How the program keys it.
    pub ptt: Ptt,
}

/// How a program keys the radio.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ptt {
    /// It does not: VOX (Winlink Express's *External*), or nothing set.
    #[default]
    None,
    /// A command on the CAT port.
    Cat,
    /// A serial port's RTS or DTR line, or both.
    Serial {
        /// The port.
        port: String,
        /// RTS keys.
        rts: bool,
        /// DTR keys.
        dtr: bool,
    },
    /// Anything else, in the program's words.
    Other {
        /// What the program says.
        said: String,
    },
}

/// What kind of modem or TNC a program points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TncKind {
    /// VARA HF's host interface: a command port and the data port after it.
    VaraHf,
    /// VARA FM's.
    VaraFm,
    /// ARDOP's.
    Ardop,
    /// A KISS TNC, over TCP or a serial port.
    Kiss,
    /// AGWPE's interface.
    Agw,
    /// Anything else.
    Other,
}

/// A modem or TNC a program points at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tnc {
    /// What kind.
    pub kind: TncKind,
    /// The program's own name for the kind, when it says more than `kind` (`KISS ACKMODE`,
    /// `KPC3+`).
    pub said: Option<String>,
    /// The address it connects to, for a TNC over TCP.
    pub host: Option<String>,
    /// The port there: VARA's command port (its data port is the next), KISS's or AGW's.
    pub port: Option<u16>,
    /// The serial port, for a TNC on one.
    pub serial: Option<String>,
    /// What the program starts as the TNC, if anything.
    pub program: Option<String>,
    /// Whether it starts that itself.
    pub launches: bool,
    /// The program's own link parameters, as it names them (`PACLEN` 128, `MAXFRAMES` 4).
    pub params: BTreeMap<String, String>,
}

impl Tnc {
    /// A TNC over TCP of a kind, at an address and port, with nothing else said.
    #[must_use]
    pub fn tcp(kind: TncKind, host: &str, port: u16) -> Self {
        Self {
            kind,
            said: None,
            host: Some(host.to_owned()),
            port: Some(port),
            serial: None,
            program: None,
            launches: false,
            params: BTreeMap::new(),
        }
    }
}

/// Whether an address is this computer: a TNC there can be this Aether.
#[must_use]
pub fn is_local(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.")
}

/// A setting that is a yes or a no, as Winlink's programs write one (`True`, `False`).
#[must_use]
pub fn yes(value: Option<&str>) -> bool {
    value.is_some_and(|v| {
        let v = v.trim();
        v.eq_ignore_ascii_case("true") || v == "1" || v.eq_ignore_ascii_case("yes")
    })
}

/// A setting with something in it: spaces trimmed, an empty one and `None` as nothing.
#[must_use]
pub fn something(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("none"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_program_has_its_own_reader() {
        for program in Program::ALL {
            let reader = program.reader();
            assert_eq!(reader.program, program);
            let exe = std::path::Path::new(reader.exe);
            let is_exe = exe
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
            assert!(is_exe, "{}", reader.exe);
            assert!(!reader.files.is_empty(), "{}", program.name());
            // no files, nothing read, and no panic
            let nothing = (reader.read)(&Files::new());
            assert!(nothing.setups.is_empty(), "{nothing:?}");
        }
    }

    #[test]
    fn this_computer_is_told_from_another() {
        assert!(is_local("127.0.0.1"));
        assert!(is_local("localhost"));
        assert!(is_local("[::1]"));
        assert!(!is_local("192.168.1.20"));
        assert!(yes(Some(" True")));
        assert!(!yes(Some("False")));
        assert!(!yes(None));
        assert_eq!(something(Some(" COM6 ".into())).as_deref(), Some("COM6"));
        assert_eq!(something(Some("None".into())), None);
        assert_eq!(something(Some(String::new())), None);
    }
}
