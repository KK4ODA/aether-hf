//! Winlink Express's reader: `RMS Express.ini` to plain [`ProgramSetup`]s, one per callsign.
//!
//! Winlink Express keeps everything in one file beside its executable: `[Properties]` lists the
//! callsigns (`Call signs`, the one in use as `Callsign`), each callsign has a section of its own
//! with its session type (`Mode`) and the bandwidth and centre frequency of each VARA session
//! type, and the radio and TNCs are shared by every callsign — `[Vara]` the radio as the VARA
//! sessions control it (the *Radio Setup* dialog), `[Vara TNC]` VARA HF's address and port,
//! `[Vara FM TNC]`, `[Ardop TNC]` and `[Packet TNC]` the others. The callsign sections also hold
//! the operator's address, telephone, e-mail and password: only `Mode` and the bandwidth and
//! frequency keys are read there.
//!
//! The PTT port's values are its help file's (*Radio Control*): `External` is VOX, a radio's
//! name or `CI-V` is a command on the control port, a serial port is its RTS or DTR line.

use super::{Files, Program, Ptt, Radio, Reader, Reading, Tnc, TncKind, something, yes};
use crate::ini::Ini;

/// How Winlink Express's installs are told and read.
pub static READER: Reader = Reader {
    program: Program::WinlinkExpress,
    exe: "RMS Express.exe",
    usual: &["RMS Express"],
    names: &["Winlink Express", "RMS Express"],
    files: &[SETTINGS],
    read,
    point,
};

/// The settings file, beside the executable.
pub const SETTINGS: &str = "RMS Express.ini";

/// The session types that run over VARA HF, as `Mode` names them.
pub const P2P: &str = "Vara HF P2P";
/// A session to a gateway.
pub const WINLINK: &str = "Vara HF Winlink";

/// A bandwidth key's value: `0` and nothing are none set.
fn bandwidth(ini: &Ini, section: &str, key: &str) -> Option<u32> {
    let value = ini.get(section, key)?;
    value.trim().parse::<u32>().ok().filter(|hz| *hz > 0)
}

/// A centre frequency as Winlink Express writes one: kilohertz (`7103.800`), or megahertz for
/// a value under a thousand (`145.530`, as its VARA FM keys hold); `0.000` is none set.
fn frequency(ini: &Ini, section: &str, key: &str) -> Option<u64> {
    let value: f64 = ini.get(section, key)?.trim().parse().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let hz = if value < 1000.0 {
        value * 1e6
    } else {
        value * 1e3
    };
    // a frequency in hertz fits easily; the cast only drops the fraction of a hertz
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some(hz.round() as u64)
}

/// The radio as the VARA sessions control it, or `None` when they control none (no control
/// port and no keying: the radio *Manual*, the PTT port *None*).
fn radio(ini: &Ini) -> Option<Radio> {
    const S: &str = "Vara";
    let rig = something(ini.get(S, "Model")).filter(|m| !m.eq_ignore_ascii_case("manual"));
    let cat_port = something(ini.get(S, "Control Port"));
    let ptt = match something(ini.get(S, "PTT Port")) {
        None => Ptt::None,
        Some(port) if port.eq_ignore_ascii_case("external") => Ptt::None,
        Some(port) if is_serial_port(&port) => Ptt::Serial {
            port,
            rts: yes(ini.get(S, "PTT RTS").as_deref()),
            dtr: yes(ini.get(S, "PTT DTR").as_deref()),
        },
        // a radio's name, or CI-V: the command on the control port
        Some(_) => Ptt::Cat,
    };
    if cat_port.is_none() && ptt == Ptt::None {
        return None;
    }
    let icom = rig.as_deref().is_some_and(|r| starts_with(r, "icom"));
    let icom_address = if icom {
        ini.get(S, "Icom Address")
            .and_then(|a| u8::from_str_radix(a.trim(), 16).ok())
    } else {
        None
    };
    Some(Radio {
        rig,
        cat_baud: cat_port
            .as_ref()
            .and_then(|_| ini.get(S, "Control Baud")?.trim().parse().ok()),
        cat_port,
        icom_address,
        ptt,
    })
}

/// Whether a name begins with another, in any case.
fn starts_with(name: &str, start: &str) -> bool {
    name.get(..start.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(start))
}

/// Whether a PTT port's value is a serial port (`COM9`, `/dev/ttyUSB0`) rather than a radio.
fn is_serial_port(value: &str) -> bool {
    let com = starts_with(value, "com") && value[3..].chars().all(|c| c.is_ascii_digit());
    com && value.len() > 3 || value.starts_with("/dev/")
}

/// A VARA-style TNC section: its address, port, the program it starts and whether it does.
fn vara_tnc(ini: &Ini, section: &str, kind: TncKind, launch_key: &str) -> Option<Tnc> {
    if !ini.has_section(section) {
        return None;
    }
    let port = ini.get(section, "TCP Port")?.trim().parse::<u16>().ok()?;
    let host = something(ini.get(section, "TNC Address")).unwrap_or_else(|| "127.0.0.1".into());
    let mut tnc = Tnc::tcp(kind, &host, port);
    tnc.program = something(ini.get(section, "Vara Program File"));
    tnc.launches = yes(ini.get(section, launch_key).as_deref());
    Some(tnc)
}

/// The packet TNC, when it is KISS over TCP or on a serial port.
fn packet_tnc(ini: &Ini) -> Option<Tnc> {
    const S: &str = "Packet TNC";
    let said = something(ini.get(S, "Packet TNC"))?;
    let kind = if said.to_ascii_uppercase().contains("KISS") {
        TncKind::Kiss
    } else {
        TncKind::Other
    };
    let serial = something(ini.get(S, "Packet Serial Port"));
    let mut tnc = if serial
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("tcp"))
    {
        let host = something(ini.get(S, "TCPHost")).unwrap_or_else(|| "127.0.0.1".into());
        let port = ini.get(S, "TCPPort")?.trim().parse::<u16>().ok()?;
        Tnc::tcp(kind, &host, port)
    } else {
        Tnc {
            kind,
            said: None,
            host: None,
            port: None,
            serial,
            program: None,
            launches: false,
            params: std::collections::BTreeMap::new(),
        }
    };
    tnc.said = Some(said);
    tnc.program = something(ini.get(S, "KISS modem"));
    tnc.launches = tnc.program.is_some() && yes(ini.get(S, "KISS modem enable").as_deref());
    Some(tnc)
}

/// Read Winlink Express's `RMS Express.ini`.
#[must_use]
pub fn read(files: &Files) -> Reading {
    let mut reading = Reading::default();
    let Some(bytes) = files.get(SETTINGS) else {
        return reading;
    };
    let ini = Ini::new(bytes.clone());
    reading.version = something(ini.get("Main", "Product Version"));
    let current = something(ini.get("Properties", "Callsign"));
    let listed = ini.get("Properties", "Call signs").unwrap_or_default();
    let mut callsigns: Vec<String> = current.into_iter().collect();
    for call in listed.split([' ', ',', ';']).filter(|c| !c.is_empty()) {
        if !callsigns.iter().any(|c| c.eq_ignore_ascii_case(call)) {
            callsigns.push(call.to_owned());
        }
    }
    if callsigns.is_empty() {
        reading
            .notes
            .push("no callsign is set up in Winlink Express yet".to_owned());
        return reading;
    }
    let radio = radio(&ini);
    let tncs: Vec<Tnc> = [
        vara_tnc(&ini, "Vara TNC", TncKind::VaraHf, "Auto Launch Vara"),
        vara_tnc(&ini, "Vara FM TNC", TncKind::VaraFm, "Auto Launch Vara FM"),
        vara_tnc(&ini, "Ardop TNC", TncKind::Ardop, "Auto Launch Ardop"),
        packet_tnc(&ini),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !tncs.iter().any(|t| t.kind == TncKind::VaraHf) {
        reading
            .notes
            .push("no VARA HF TNC is set up in Winlink Express yet".to_owned());
    }
    for call in callsigns {
        let mode = something(ini.get(&call, "Mode"));
        let p2p = (
            bandwidth(&ini, &call, "Vara P2P Channel Bandwidth"),
            frequency(&ini, &call, "Vara P2P Center Frequency"),
        );
        let winlink = (
            bandwidth(&ini, &call, "Vara WL2K Channel Bandwidth"),
            frequency(&ini, &call, "Vara WL2K Center Frequency"),
        );
        // the session type the callsign last ran; for another (Telnet, packet), a gateway
        // session's if one was set up, else a P2P one's
        let (bandwidth_hz, frequency_hz) = match mode.as_deref() {
            Some(m) if m.eq_ignore_ascii_case(P2P) => p2p,
            Some(m) if m.eq_ignore_ascii_case(WINLINK) => winlink,
            _ if winlink.0.is_some() => winlink,
            _ => p2p,
        };
        reading.setups.push(super::ProgramSetup {
            name: call.clone(),
            callsigns: vec![call],
            session: mode,
            bandwidth_hz,
            frequency_hz,
            channels_hz: Vec::new(),
            gateway: false,
            radio: radio.clone(),
            tncs: tncs.clone(),
        });
    }
    reading
}

/// The settings Winlink Express needs to reach Aether, of its VARA HF TNC only: this computer
/// when it names another (a port of its own here is followed by the profile instead, and needs
/// no change), and its launch off — Winlink Express ends the TNC it started when it closes.
/// Nothing when it has no VARA HF TNC yet.
#[must_use]
pub fn point(files: &Files, target: &super::point::Target) -> Vec<super::point::Edit> {
    use super::point::Edit;
    const S: &str = "Vara TNC";
    let Some(bytes) = files.get(SETTINGS) else {
        return Vec::new();
    };
    let ini = Ini::new(bytes.clone());
    if !ini.has_section(S) {
        return Vec::new();
    }
    let mut edits = Vec::new();
    let host = something(ini.get(S, "TNC Address"));
    if host.as_deref().is_some_and(|h| !super::is_local(h)) {
        edits.push(Edit {
            file: SETTINGS,
            section: S,
            key: "TNC Address",
            to: "127.0.0.1".to_owned(),
            why: "Aether runs on this computer",
        });
        edits.push(Edit {
            file: SETTINGS,
            section: S,
            key: "TCP Port",
            to: target.host_port.to_string(),
            why: "Aether's host interface is on this port",
        });
    }
    if yes(ini.get(S, "Auto Launch Vara").as_deref()) {
        edits.push(Edit {
            file: SETTINGS,
            section: S,
            key: "Auto Launch Vara",
            to: "False".to_owned(),
            why: "Winlink Express ends the TNC it started when it closes: Aether starts on its \
                  own and stays",
        });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `RMS Express.ini` as Winlink Express writes one, every value made up, with the
    /// personal keys a real one has (which must never be read).
    const INI: &str = "\r\n[Main]\r\nTest Autoupdate=False\r\nProduct Version=1.8.6.0\r\n\
        [Properties]\r\nCall signs=N0CALL N1CALL\r\nCallsign=N1CALL\r\n\
        [N0CALL]\r\nPassword2=secret\r\nPhone Number=555-0100\r\nStreet Address 1=1 Main St\r\n\
        Mode=Vara HF P2P\r\nVara WL2K Channel Bandwidth=2300\r\nVara P2P Channel Bandwidth=500\r\n\
        Vara P2P Center Frequency=7083.5\r\nVara WL2K Center Frequency=7103.800\r\n\
        [N1CALL]\r\nPassword2=secret\r\nMode=Telnet Winlink\r\nVara WL2K Channel Bandwidth=2750\r\n\
        Vara WL2K Center Frequency=3591.500\r\nVara P2P Channel Bandwidth=0\r\n\
        [Vara TNC]\r\nProcess Id=-1\r\nVara Program File=C:\\VARA\\VARA.exe\r\nTCP Port=8300\r\n\
        TNC Address=127.0.0.1\r\nAuto Launch Vara=True\r\n\
        [Vara FM TNC]\r\nVara Program File=C:\\VARA FM\\VaraFM.exe\r\nTCP Port=8250\r\n\
        TNC Address=127.0.0.1\r\nAuto Launch Vara FM=False\r\n\
        [Packet TNC]\r\nPacket TNC=KISS\r\nPacket Serial Port=TCP\r\nTCPHost=127.0.0.1\r\n\
        TCPPort=8101\r\nKISS modem=C:\\soundmodem\\soundmodem.exe\r\nKISS modem enable=True\r\n\
        [Vara]\r\nPTT Baud=38400\r\nControl Baud=38400\r\nControl Port=COM6\r\n\
        Icom Address=94\r\nModel=Yaesu FTDX10\r\nPTT Port=FTDX10\r\nPTT DTR=True\r\nPTT RTS=True\r\n\
        Codan password=secret\r\n";

    fn files(text: &str) -> Files {
        let mut files = Files::new();
        files.insert(SETTINGS.to_owned(), text.as_bytes().to_vec());
        files
    }

    #[test]
    fn each_callsign_is_a_setup_and_the_one_in_use_comes_first() {
        let reading = read(&files(INI));
        assert_eq!(reading.version.as_deref(), Some("1.8.6.0"));
        let names: Vec<&str> = reading.setups.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["N1CALL", "N0CALL"]);
        let p2p = &reading.setups[1];
        assert_eq!(p2p.callsigns, ["N0CALL"]);
        assert_eq!(p2p.session.as_deref(), Some(P2P));
        assert_eq!(p2p.bandwidth_hz, Some(500));
        assert_eq!(p2p.frequency_hz, Some(7_083_500));
        assert!(!p2p.gateway);
        // a callsign last used for Telnet: its gateway session's figures
        let telnet = &reading.setups[0];
        assert_eq!(telnet.bandwidth_hz, Some(2750));
        assert_eq!(telnet.frequency_hz, Some(3_591_500));
        assert!(reading.notes.is_empty(), "{:?}", reading.notes);
    }

    #[test]
    fn the_radio_and_the_tncs_are_shared_by_every_callsign() {
        let reading = read(&files(INI));
        let radio = reading.setups[0].radio.clone().expect("a radio");
        assert_eq!(radio.rig.as_deref(), Some("Yaesu FTDX10"));
        assert_eq!(radio.cat_port.as_deref(), Some("COM6"));
        assert_eq!(radio.cat_baud, Some(38400));
        assert_eq!(radio.icom_address, None, "not an Icom");
        assert_eq!(radio.ptt, Ptt::Cat, "the radio's name is a command PTT");
        let tncs = &reading.setups[0].tncs;
        assert_eq!(tncs.len(), 3, "{tncs:?}");
        let vara = &tncs[0];
        assert_eq!(vara.kind, TncKind::VaraHf);
        assert_eq!(vara.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(vara.port, Some(8300));
        assert_eq!(vara.program.as_deref(), Some("C:\\VARA\\VARA.exe"));
        assert!(vara.launches);
        assert_eq!(tncs[1].kind, TncKind::VaraFm);
        assert!(!tncs[1].launches);
        let kiss = &tncs[2];
        assert_eq!(kiss.kind, TncKind::Kiss);
        assert_eq!(kiss.port, Some(8101));
        assert!(kiss.launches);
        assert_eq!(reading.setups[1].radio, reading.setups[0].radio);
    }

    #[test]
    fn no_personal_value_is_read() {
        let reading = read(&files(INI));
        let json = serde_json::to_string(&reading).expect("json");
        for secret in ["secret", "555-0100", "Main St"] {
            assert!(!json.contains(secret), "{secret} in {json}");
        }
    }

    #[test]
    fn the_keying_is_read_as_the_help_file_names_it() {
        let with = |ptt: &str, model: &str, port: &str| {
            let text = format!(
                "[Properties]\nCallsign=N0CALL\n[Vara]\nModel={model}\nControl Port={port}\n\
                 Control Baud=9600\nIcom Address=A4\nPTT Port={ptt}\nPTT RTS=True\nPTT DTR=False\n"
            );
            read(&files(&text)).setups[0].radio.clone()
        };
        let rts = with("COM9", "Kenwood TS-480", "COM9").expect("radio");
        let serial = Ptt::Serial {
            port: "COM9".into(),
            rts: true,
            dtr: false,
        };
        assert_eq!(rts.ptt, serial);
        let vox = with("External", "Icom IC-705", "COM4").expect("radio");
        assert_eq!(vox.ptt, Ptt::None);
        assert_eq!(vox.icom_address, Some(0xA4));
        let civ = with("CI-V", "Icom IC-705", "COM4").expect("radio");
        assert_eq!(civ.ptt, Ptt::Cat);
        // the radio Manual and the PTT port None: the program controls nothing
        assert_eq!(with("None", "Manual", "None"), None);
        // nothing at all set up: no radio, and a note that there is no VARA HF TNC
        let bare = read(&files("[Properties]\nCallsign=N0CALL\n"));
        assert_eq!(bare.setups[0].radio, None);
        assert!(bare.setups[0].tncs.is_empty());
        assert_eq!(bare.notes.len(), 1, "{:?}", bare.notes);
    }

    #[test]
    fn a_file_with_no_callsign_has_no_setup() {
        let reading = read(&files("[Main]\nProduct Version=1.8.5.0\n"));
        assert!(reading.setups.is_empty());
        assert_eq!(reading.version.as_deref(), Some("1.8.5.0"));
        assert_eq!(reading.notes.len(), 1);
        assert_eq!(read(&Files::new()), Reading::default());
    }
}
