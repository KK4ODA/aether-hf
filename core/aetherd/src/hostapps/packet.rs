//! RMS Packet's reader: the Winlink packet gateway's settings, from `RMS Packet.ini` and the
//! KISS driver's `.aps` file it names.
//!
//! RMS Packet serves two kinds of channel at once (its help, *Vara FM Configuration*): a
//! **VARA FM** channel on a VARA-style TNC port (command port, data = +1), and **packet**
//! channels through one of three routes its *Site Properties* choose — the AGW Packet Engine
//! protocol (`Use AGWPE`), BPQ32 (`Use BPQ32`), or "direct access" to a TNC: KISS on a COM port
//! or over TCP (default 127.0.0.1:8100), *KISS ACKMODE* for software modems, or a Kantronics,
//! AEA/Timewave or SCS TNC in its host mode. Every channel is on VHF/UHF FM: Aether HF serves
//! none of them on the air its users are on, so the finder says each waits for Aether FM
//! (`frequency_hz` above 30 MHz, `TncKind::VaraFm`, `TncKind::Agw`).
//!
//! What is read, and nothing else: the program's version; the VARA FM channel's enable, port
//! callsign, frequency, width and TNC address; the packet route, TNC type, serial or TCP
//! address, the software modem it starts; each packet port's callsign, frequency, on-air baud
//! rate and link parameters; and, from the `.aps` the ini names when it is one of the two KISS
//! examples, the KISS driver's timers. The site's address, phone, e-mail, grid square, sysop
//! name and passwords are never read.

use std::collections::BTreeMap;

use super::{Files, Program, ProgramSetup, Reader, Reading, Tnc, TncKind, something, yes};
use crate::ini::Ini;

/// The ini, beside the program.
const INI: &str = "RMS Packet.ini";

/// The KISS driver's two example files, under the install's `Data` folder: RMS Packet names
/// one of them unless the sysop made a copy (its help asks for one before any edit).
const KISS_EXAMPLES: [&str; 2] = ["Data\\ExampleKiss.aps", "Data\\ExampleKissAckmode.aps"];

/// How RMS Packet's installs are told and read.
pub static READER: Reader = Reader {
    program: Program::RmsPacket,
    exe: "RMS Packet.exe",
    usual: &["RMS\\RMS Packet"],
    names: &["RMS Packet"],
    files: &[INI, KISS_EXAMPLES[0], KISS_EXAMPLES[1]],
    read,
    point,
};

/// Nothing to write: RMS Packet works on VHF and UHF through VARA FM, AGW or a serial KISS TNC,
/// all of which wait for Aether FM (ADR-0133 §4).
#[must_use]
pub fn point(_files: &Files, _target: &super::point::Target) -> Vec<super::point::Edit> {
    Vec::new()
}

/// VARA FM's command port when the ini names none (the help: "defaults to 8300").
const VARA_FM_PORT: u16 = 8300;
/// The KISS-over-TCP port when the ini names none (the help: "defaults to port 8100").
const KISS_TCP_PORT: u16 = 8100;
/// The AGW Packet Engine's port when the ini names none (the help: "default … is 8000").
const AGW_PORT: u16 = 8000;

/// The `.aps` keys the KISS driver reads, in the file's own words.
const APS_KEYS: [&str; 11] = [
    "TXDELAY",
    "PERSISTANCE",
    "SLOTTIME",
    "FULLDUPLEX",
    "MAXFRAMESIZE",
    "MAXFRAMES",
    "ACKTIMER",
    "MAXRETRY",
    "POLLTHRESH",
    "ACKMODE",
    "IPOLL",
];

/// The packet ports' link keys copied into a TNC's parameters, as the ini names them.
const PORT_KEYS: [&str; 4] = [
    "On-Air Baud Rate",
    "Packet Length",
    "Max Outstanding Frames",
    "Max Streams",
];

/// Read RMS Packet's files.
#[must_use]
pub fn read(files: &Files) -> Reading {
    let mut reading = Reading::default();
    let Some(bytes) = files.get(INI) else {
        reading.notes.push(format!(
            "no {INI} beside the program: it has not been set up"
        ));
        return reading;
    };
    let ini = Ini::new(bytes.clone());
    reading.version = value(&ini, "Main", "Program Version");
    if let Some(setup) = vara_fm(&ini, &mut reading.notes) {
        reading.setups.push(setup);
    }
    reading
        .setups
        .extend(packet(&ini, files, &mut reading.notes));
    if reading.setups.is_empty() {
        reading.notes.push("no channel is enabled".into());
    }
    reading
}

/// The VARA FM channel, if enabled.
fn vara_fm(ini: &Ini, notes: &mut Vec<String>) -> Option<ProgramSetup> {
    const S: &str = "Vara FM TNC";
    if !ini.has_section(S) {
        return None;
    }
    if !truth(ini, S, "Enable Vara FM") || !truth_or(ini, S, "Port Enabled", true) {
        notes.push("its VARA FM channel is set up but not enabled".into());
        return None;
    }
    let callsign = callsign(ini, S, "Callsign");
    // "Channel width — Narrow/Wide" is kept as the baud rate it replaced (revision history:
    // "Change Vara FM channel settings from 1200/9600 baud to Narrow/Wide channel width").
    let width = match value(ini, S, "RF Baud Rate").as_deref() {
        Some("9600") => "Vara FM Wide",
        Some("1200") => "Vara FM Narrow",
        _ => "Vara FM",
    };
    let port = port(ini, S, "TCP Port", VARA_FM_PORT, notes);
    let host = value(ini, S, "TNC Address").unwrap_or_else(|| "127.0.0.1".into());
    let tnc = Tnc {
        kind: TncKind::VaraFm,
        said: Some("Vara FM".into()),
        host: Some(host),
        port: Some(port),
        serial: None,
        program: value(ini, S, "Vara Program File"),
        launches: truth(ini, S, "Launch Vara FM"),
        params: BTreeMap::new(),
    };
    Some(ProgramSetup {
        name: match &callsign {
            Some(call) => format!("{call} {width}"),
            None => width.into(),
        },
        callsigns: callsign.into_iter().collect(),
        session: Some(width.into()),
        bandwidth_hz: None,
        gateway: true,
        radio: None,
        tncs: vec![tnc],
        frequency_hz: frequency(ini, S),
        channels_hz: Vec::new(),
    })
}

/// How the packet channels reach their TNC.
enum Route {
    Agw,
    Bpq,
    Direct,
}

/// The packet channels: one setup per enabled port.
fn packet(ini: &Ini, files: &Files, notes: &mut Vec<String>) -> Vec<ProgramSetup> {
    const SITE: &str = "Site Properties";
    const TNC: &str = "TNC Properties";
    let route = if truth(ini, SITE, "Use AGWPE") {
        Route::Agw
    } else if truth(ini, SITE, "Use BPQ32") {
        Route::Bpq
    } else {
        Route::Direct
    };
    let tnc_type = value(ini, TNC, "TNC Type").unwrap_or_default();
    if matches!(route, Route::Direct) && (tnc_type.is_empty() || tnc_type.starts_with("(None")) {
        return Vec::new();
    }
    let base = match route {
        Route::Agw => Tnc {
            kind: TncKind::Agw,
            said: Some("AGW Packet Engine".into()),
            host: Some(value(ini, TNC, "AGW IP Address").unwrap_or_else(|| "localhost".into())),
            port: Some(port(ini, TNC, "AGW IP Port", AGW_PORT, notes)),
            serial: None,
            program: value(ini, TNC, "AGW Path").filter(|_| !truth(ini, TNC, "AGW Remote")),
            launches: !truth(ini, TNC, "AGW Remote") && value(ini, TNC, "AGW Path").is_some(),
            params: BTreeMap::new(),
        },
        Route::Bpq => Tnc {
            kind: TncKind::Other,
            said: Some("BPQ32".into()),
            host: None,
            port: None,
            serial: None,
            program: None,
            launches: false,
            params: BTreeMap::new(),
        },
        Route::Direct => direct(ini, files, &tnc_type, notes),
    };
    // AGW asks one callsign for every port ("AGWPE Requires that only one callsign per TCP
    // connection be used"); the others name each port's own.
    let agw_call = matches!(route, Route::Agw)
        .then(|| callsign(ini, TNC, "AGW Call sign"))
        .flatten();
    let ports = ini
        .get(TNC, "Number of Ports")
        .and_then(|n| n.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 16);
    let mut setups = Vec::new();
    for n in 1..=ports {
        let section = format!("Port {n} Properties");
        if !ini.has_section(&section) {
            continue;
        }
        if !truth_or(ini, &section, "Port Enabled", true) {
            notes.push(format!("packet port {n} is set up but not enabled"));
            continue;
        }
        let call = agw_call
            .clone()
            .or_else(|| callsign(ini, &section, "Callsign"));
        let baud = value(ini, &section, "On-Air Baud Rate");
        let session = match &baud {
            Some(b) => format!("Packet {b} baud"),
            None => "Packet".into(),
        };
        let mut tnc = base.clone();
        for key in PORT_KEYS {
            if let Some(v) = value(ini, &section, key) {
                tnc.params.insert(key.into(), v);
            }
        }
        setups.push(ProgramSetup {
            name: match &call {
                Some(c) if ports > 1 => format!("{c} packet port {n}"),
                Some(c) => format!("{c} packet"),
                None => format!("packet port {n}"),
            },
            callsigns: call.into_iter().collect(),
            session: Some(session),
            bandwidth_hz: None,
            gateway: true,
            radio: None,
            tncs: vec![tnc],
            frequency_hz: frequency(ini, &section),
            channels_hz: Vec::new(),
        });
    }
    setups
}

/// A TNC reached directly: KISS (serial or TCP) or a TNC in its host mode.
fn direct(ini: &Ini, files: &Files, tnc_type: &str, notes: &mut Vec<String>) -> Tnc {
    const TNC: &str = "TNC Properties";
    const KISS: &str = "KISS TNC";
    let kiss = tnc_type.to_ascii_uppercase().contains("KISS");
    let serial = value(ini, TNC, "Serial Port");
    let over_tcp = serial
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("TCP"));
    let mut tnc = Tnc {
        kind: if kiss { TncKind::Kiss } else { TncKind::Other },
        said: Some(tnc_type.to_string()),
        host: None,
        port: None,
        serial: None,
        program: None,
        launches: false,
        params: BTreeMap::new(),
    };
    if over_tcp {
        tnc.host = Some(value(ini, TNC, "TCP Host").unwrap_or_else(|| "127.0.0.1".into()));
        tnc.port = Some(port(ini, TNC, "TCP Port", KISS_TCP_PORT, notes));
    } else {
        tnc.serial = serial;
        if let Some(baud) = value(ini, TNC, "Serial Baud Rate") {
            tnc.params.insert("Serial Baud Rate".into(), baud);
        }
    }
    if kiss {
        // A software modem RMS Packet starts itself (Soundmodem, Dire Wolf …).
        tnc.program = value(ini, KISS, "KISS Program File");
        tnc.launches = tnc.program.is_some() && truth(ini, KISS, "KISS Program Enable");
    }
    if let Some(aps) = value(ini, TNC, "TNC Config File") {
        aps_params(&aps, files, &mut tnc.params, notes);
    }
    tnc
}

/// The KISS driver's timers and limits from the `.aps` the ini names, when it is one of the
/// examples beside the program; a copy elsewhere is named in a note, not read.
fn aps_params(
    aps: &str,
    files: &Files,
    params: &mut BTreeMap<String, String>,
    notes: &mut Vec<String>,
) {
    let name = aps.rsplit(['\\', '/']).next().unwrap_or(aps);
    if name.is_empty() {
        return;
    }
    let Some(bytes) = KISS_EXAMPLES
        .iter()
        .find(|example| {
            example
                .rsplit('\\')
                .next()
                .is_some_and(|e| e.eq_ignore_ascii_case(name))
        })
        .and_then(|example| files.get(*example))
    else {
        notes.push(format!(
            "its TNC settings are in {name}, which was not read"
        ));
        return;
    };
    for line in String::from_utf8_lossy(bytes).lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        if let (Some(key), Some(v)) = (words.next(), words.next()) {
            let key = key.to_ascii_uppercase();
            if APS_KEYS.contains(&key.as_str()) {
                params.insert(key, v.to_string());
            }
        }
    }
}

/// A value, trimmed; an empty one (or `None`) is none.
fn value(ini: &Ini, section: &str, key: &str) -> Option<String> {
    something(ini.get(section, key))
}

/// A `True`/`False` value; missing or anything else is false.
fn truth(ini: &Ini, section: &str, key: &str) -> bool {
    yes(ini.get(section, key).as_deref())
}

/// A `True`/`False` value that is `otherwise` when missing (`Port Enabled` is written only
/// once a port has been set up).
fn truth_or(ini: &Ini, section: &str, key: &str, otherwise: bool) -> bool {
    match value(ini, section, key) {
        Some(v) => yes(Some(&v)),
        None => otherwise,
    }
}

/// A callsign, upper-cased.
fn callsign(ini: &Ini, section: &str, key: &str) -> Option<String> {
    value(ini, section, key).map(|c| c.to_ascii_uppercase())
}

/// A TCP port, or the program's default with a note when the value is not one.
fn port(ini: &Ini, section: &str, key: &str, otherwise: u16, notes: &mut Vec<String>) -> u16 {
    match value(ini, section, key) {
        None => otherwise,
        Some(v) => match v.parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                notes.push(format!(
                    "{section}: {key} is not a port number; {otherwise} assumed"
                ));
                otherwise
            }
        },
    }
}

/// A channel's frequency in hertz: the ini keeps hertz (`145770000`); a value with a point is
/// megahertz, as the dialog shows it.
fn frequency(ini: &Ini, section: &str) -> Option<u64> {
    let v = value(ini, section, "Frequency")?;
    if v.contains('.') {
        let mhz: f64 = v.parse().ok()?;
        (mhz > 0.0).then(|| (mhz * 1e6).round() as u64)
    } else {
        v.parse::<u64>().ok().filter(|&hz| hz > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic `RMS Packet.ini` in the program's own shape (sections and key names as RMS
    /// Packet 2.2.4.0 writes them, CRLF), with N0CALL and made-up values only. The site's
    /// personal keys are there, filled with placeholders, to prove they are never read.
    fn ini(site: &str, vara: &str, tnc: &str, ports: &str) -> Vec<u8> {
        let text = format!(
            "[Main]\nProgram Version=2.2.4.0\nInhibit Autoupdate=False\n\n\
             [Site Properties]\nBase Callsign=N0CALL\nDefault Grid Square=AA00aa\n\
             Password2=SECRET-PLACEHOLDER\nSysop Email=nobody@example.invalid\n\
             Phone Number=555-0100\nStreet Address 1=1 Placeholder Rd\n{site}\n\
             {vara}\n[TNC Properties]\n{tnc}\n{ports}\n\
             [KISS TNC]\nKISS Program File=C:\\Soundmodem\\soundmodem.exe\nKISS Program Enable=False\n"
        );
        text.replace('\n', "\r\n").into_bytes()
    }

    const VARA_ON: &str = "[Vara FM TNC]\nEnable Vara FM=True\nVara Program File=C:\\VARA FM\\VARAFM.exe\n\
        TCP Port=8300\nTNC Address=127.0.0.1\nRF Baud Rate=9600\nLaunch Vara FM=True\n\
        Callsign=n0call-10\nFrequency=145770000\nPort Enabled=True\n";

    fn files(ini: Vec<u8>) -> Files {
        let mut files = Files::new();
        files.insert(INI.into(), ini);
        files
    }

    fn read_one(site: &str, vara: &str, tnc: &str, ports: &str) -> Reading {
        read(&files(ini(site, vara, tnc, ports)))
    }

    const PORT1: &str = "[Port 1 Properties]\nCallsign=N0CALL-10\nFrequency=145050000\nOn-Air Baud Rate=1200\n\
        Max Streams=3\nPacket Length=128\nMax Outstanding Frames=2\nPort Enabled=True\n";

    #[test]
    fn a_vara_fm_only_gateway_is_one_setup_on_its_tnc_port() {
        let r = read_one(
            "Use AGWPE=False\nUse BPQ32=False",
            VARA_ON,
            "TNC Type=(None - No packet channels, Vara FM only)\nSerial Port=TCP",
            "",
        );
        assert_eq!(r.version.as_deref(), Some("2.2.4.0"));
        assert_eq!(r.setups.len(), 1, "{:?}", r.setups);
        let s = &r.setups[0];
        assert_eq!(s.callsigns, ["N0CALL-10"]);
        assert_eq!(s.session.as_deref(), Some("Vara FM Wide"));
        assert_eq!(s.frequency_hz, Some(145_770_000));
        assert!(s.gateway);
        assert!(s.radio.is_none());
        let t = &s.tncs[0];
        assert_eq!(t.kind, TncKind::VaraFm);
        assert_eq!((t.host.as_deref(), t.port), (Some("127.0.0.1"), Some(8300)));
        assert!(t.launches);
        assert_eq!(t.program.as_deref(), Some("C:\\VARA FM\\VARAFM.exe"));
    }

    #[test]
    fn agw_ports_all_answer_to_the_engines_one_callsign() {
        let r = read_one(
            "Use AGWPE=True\nUse BPQ32=False",
            "",
            "TNC Type=AGWPE\nAGW IP Address=localhost\nAGW Call sign=N0CALL-10\nNumber of Ports=2\n\
             AGW IP Port=8000\nAGW Remote=True\nAGW Path=",
            &format!(
                "{PORT1}\n[Port 2 Properties]\nCallsign=N0CALL-7\nFrequency=440.975\nOn-Air Baud Rate=9600\nPort Enabled=True\n"
            ),
        );
        assert_eq!(r.setups.len(), 2);
        for s in &r.setups {
            assert_eq!(
                s.callsigns,
                ["N0CALL-10"],
                "AGW asks one callsign for every port"
            );
            let t = &s.tncs[0];
            assert_eq!(t.kind, TncKind::Agw);
            assert_eq!((t.host.as_deref(), t.port), (Some("localhost"), Some(8000)));
            assert!(!t.launches, "a remote engine is not started by RMS Packet");
        }
        assert_eq!(r.setups[0].session.as_deref(), Some("Packet 1200 baud"));
        assert_eq!(
            r.setups[0].tncs[0]
                .params
                .get("Packet Length")
                .map(String::as_str),
            Some("128")
        );
        assert_eq!(
            r.setups[1].frequency_hz,
            Some(440_975_000),
            "megahertz with a point"
        );
        assert_eq!(r.setups[1].session.as_deref(), Some("Packet 9600 baud"));
    }

    #[test]
    fn kiss_over_tcp_reads_the_address_and_the_examples_timers() {
        let mut f = files(ini(
            "Use AGWPE=False\nUse BPQ32=False",
            "",
            "TNC Type=KISS ACKMODE\nSerial Port=TCP\nTCP Host=127.0.0.1\nTCP Port=8105\n\
             TNC Config File=C:\\RMS\\RMS Packet\\Data\\ExampleKissAckmode.aps",
            PORT1,
        ));
        f.insert(
            KISS_EXAMPLES[1].into(),
            b"; comment\r\nTXDELAY 50  ; x\r\nPERSISTANCE 128\r\nMAXFRAMES 4 ; y\r\nACKTIMER 10\r\nACKMODE TRUE\r\nNOTAKEY 1\r\n".to_vec(),
        );
        let r = read(&f);
        assert_eq!(r.setups.len(), 1);
        let t = &r.setups[0].tncs[0];
        assert_eq!(t.kind, TncKind::Kiss);
        assert_eq!(t.said.as_deref(), Some("KISS ACKMODE"));
        assert_eq!(
            (t.host.as_deref(), t.port, t.serial.as_deref()),
            (Some("127.0.0.1"), Some(8105), None)
        );
        assert_eq!(t.params.get("ACKTIMER").map(String::as_str), Some("10"));
        assert_eq!(t.params.get("ACKMODE").map(String::as_str), Some("TRUE"));
        assert_eq!(t.params.get("MAXFRAMES").map(String::as_str), Some("4"));
        assert!(!t.params.contains_key("NOTAKEY"));
        assert_eq!(t.program.as_deref(), Some("C:\\Soundmodem\\soundmodem.exe"));
        assert!(!t.launches);
    }

    #[test]
    fn a_serial_tnc_names_its_port_and_a_custom_aps_is_named_not_read() {
        let r = read_one(
            "Use AGWPE=False\nUse BPQ32=False",
            "",
            "TNC Type=Kantronics\nSerial Port=COM7\nSerial Baud Rate=9600\nTNC Config File=C:\\RMS\\RMS Packet\\Data\\MyKPC3.aps",
            PORT1,
        );
        let t = &r.setups[0].tncs[0];
        assert_eq!(t.kind, TncKind::Other);
        assert_eq!(t.said.as_deref(), Some("Kantronics"));
        assert_eq!(
            (t.host.as_deref(), t.port, t.serial.as_deref()),
            (None, None, Some("COM7"))
        );
        assert_eq!(
            t.params.get("Serial Baud Rate").map(String::as_str),
            Some("9600")
        );
        assert!(
            r.notes.iter().any(|n| n.contains("MyKPC3.aps")),
            "{:?}",
            r.notes
        );
    }

    #[test]
    fn vara_fm_and_packet_together_are_two_setups_and_disabled_ones_are_said() {
        let r = read_one(
            "Use AGWPE=False\nUse BPQ32=True",
            &VARA_ON.replace("RF Baud Rate=9600", "RF Baud Rate=1200"),
            "TNC Type=AGWPE",
            &format!("{PORT1}\n[Port 2 Properties]\nCallsign=N0CALL-11\nPort Enabled=False\n"),
        );
        // BPQ32 with one port (Number of Ports absent → 1)
        assert_eq!(r.setups.len(), 2, "{:?}", r.setups);
        assert_eq!(r.setups[0].session.as_deref(), Some("Vara FM Narrow"));
        assert_eq!(r.setups[1].tncs[0].said.as_deref(), Some("BPQ32"));
        assert_eq!(r.setups[1].tncs[0].port, None);
    }

    #[test]
    fn nothing_personal_is_read_and_a_missing_ini_is_said() {
        let r = read_one(
            "Use AGWPE=False",
            VARA_ON,
            "TNC Type=KISS\nSerial Port=COM3",
            PORT1,
        );
        let all = serde_json::to_string(&r).unwrap();
        for secret in [
            "SECRET",
            "example.invalid",
            "555-0100",
            "Placeholder",
            "AA00aa",
        ] {
            assert!(!all.contains(secret), "{secret} leaked into {all}");
        }
        let empty = read(&Files::new());
        assert!(empty.setups.is_empty());
        assert_eq!(empty.notes.len(), 1);
    }

    #[test]
    fn a_disabled_vara_fm_channel_is_not_a_setup() {
        let r = read_one(
            "Use AGWPE=False",
            &VARA_ON.replace("Enable Vara FM=True", "Enable Vara FM=False"),
            "TNC Type=(None - No packet channels, Vara FM only)",
            "",
        );
        assert!(r.setups.is_empty());
        assert!(r.notes.iter().any(|n| n.contains("not enabled")));
    }
}
