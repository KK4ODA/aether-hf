//! RMS Trimode's reader: its settings file to one plain [`ProgramSetup`].
//!
//! A Trimode install is one gateway: one base callsign (the Winlink account it logs in with),
//! up to eight channels — each with a callsign of its own, a centre frequency, a bandwidth, a
//! service code and the protocols it is offered on — one radio and one VARA TNC. So it is one
//! setup, named after the base callsign; a second gateway on the same computer is a second
//! install with a folder of its own, which the finder tells apart by that folder and this name.
//!
//! Everything comes from `RMS Trimode.ini` beside the program, a plain INI file. The keys were
//! read from Trimode 1.4.4 (a scratch copy and the author's gateway, 2026-11-19). Only the keys
//! named here are read: the file also holds the sysop's address, telephone, e-mail, grid square
//! and Winlink password, which stay in it. RMS Relay's file holds nothing Aether needs (no radio,
//! no TNC), so it is not read.

use super::{Files, Program, ProgramSetup, Ptt, Radio, Reader, Reading, Tnc, TncKind};
use super::{something, yes};
use crate::ini::Ini;

/// The settings file Trimode keeps beside its program.
const SETTINGS: &str = "RMS Trimode.ini";
/// How many channels Trimode's channel table holds.
const CHANNELS: usize = 8;
/// A channel's `Mode N` is a set of bits, one for each protocol it is offered on; this one is
/// VARA HF. Read from the files: a VARA channel says `268435488` (`0x1000_0020`) and the same
/// channel with VARA unticked `268435456` (`0x1000_0000`).
const VARA_HF: u64 = 0x20;
/// Where Trimode reaches VARA when its file says nothing: VARA's own defaults.
const VARA_HOST: &str = "127.0.0.1";
const VARA_PORT: u16 = 8300;
/// The service code every Aether gateway publishes its channels under, and only that one
/// (`docs/user/winlink-gateway.md` §2.4): Winlink lists the channel as VARA HF, and the code
/// keeps it out of a VARA station's list.
const AETHER_SERVICE_CODE: &str = "AETHER";
/// Trimode's own word for the mode: its logs say `Mode: Vara` and `Mode: Vara500`.
const SESSION: &str = "Vara HF";

/// How RMS Trimode's installs are told and read.
pub static READER: Reader = Reader {
    program: Program::RmsTrimode,
    exe: "RMS Trimode.exe",
    usual: &["RMS\\RMS Trimode"],
    names: &["RMS Trimode"],
    files: &[SETTINGS],
    read,
    point,
};

/// A channel Trimode offers VARA HF on.
struct Channel {
    number: usize,
    callsign: Option<String>,
    centre_hz: u64,
    bandwidth_hz: u32,
    service_code: Option<String>,
}

fn text(ini: &Ini, section: &str, key: &str) -> Option<String> {
    something(ini.get(section, key))
}

fn number<T: std::str::FromStr>(ini: &Ini, section: &str, key: &str) -> Option<T> {
    text(ini, section, key)?.parse().ok()
}

fn callsign(ini: &Ini, section: &str, key: &str) -> Option<String> {
    text(ini, section, key).map(|c| c.to_ascii_uppercase())
}

/// The channels offering VARA HF, in Trimode's order. A channel with no frequency is an empty
/// row of the table, whatever its other columns say (a new install fills all eight).
fn channels(ini: &Ini) -> Vec<Channel> {
    // a wide channel is 2300 Hz, or 2750 with the VARA TNC's *BW2750*
    let wide = if yes(ini.get("Vara TNC", "BW2750").as_deref()) {
        2750
    } else {
        2300
    };
    (1..=CHANNELS)
        .filter_map(|n| {
            let centre_hz =
                number::<u64>(ini, "Channels", &format!("Frequency {n}")).filter(|hz| *hz > 0)?;
            let mode = number::<u64>(ini, "Channels", &format!("Mode {n}")).unwrap_or(0);
            if mode & VARA_HF == 0 {
                return None;
            }
            // unset reads as wide, Trimode's default
            let narrow = ini
                .get("Channels", &format!("Wide Bandwidth {n}"))
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("false"));
            Some(Channel {
                number: n,
                callsign: callsign(ini, "Channels", &format!("Call {n}")),
                centre_hz,
                bandwidth_hz: if narrow { 500 } else { wide },
                service_code: text(ini, "Channels", &format!("SC {n}")),
            })
        })
        .collect()
}

/// The radio as Trimode controls it, and how it keys it for VARA. `None` when it controls
/// none: no model, no port, and VARA keyed by neither CAT nor a serial line (VOX, or the radio
/// set to *Manual*).
fn radio(ini: &Ini) -> Option<Radio> {
    let rig = text(ini, "Radio", "Model");
    let cat_port = text(ini, "Radio", "Control Port");
    let ptt = if yes(ini.get("Vara TNC", "Use CAT PTT").as_deref()) {
        Ptt::Cat
    } else if yes(ini.get("Vara TNC", "Use Serial Port PTT").as_deref()) {
        Ptt::Serial {
            port: text(ini, "Vara TNC", "PTT Port").unwrap_or_default(),
            rts: yes(ini.get("Vara TNC", "Use RTS").as_deref()),
            dtr: yes(ini.get("Vara TNC", "Use DTR").as_deref()),
        }
    } else {
        Ptt::None
    };
    if rig.is_none() && cat_port.is_none() && ptt == Ptt::None {
        return None;
    }
    // Trimode keeps an Icom address, in hex, whatever the radio; it means something only for one
    let icom = rig
        .as_deref()
        .is_some_and(|r| r.trim_start().to_ascii_lowercase().starts_with("icom"));
    Some(Radio {
        cat_baud: cat_port
            .as_ref()
            .and_then(|_| number(ini, "Radio", "Control Baud")),
        icom_address: if icom {
            text(ini, "Radio", "Icom Address").and_then(|a| u8::from_str_radix(&a, 16).ok())
        } else {
            None
        },
        rig,
        cat_port,
        ptt,
    })
}

/// Read RMS Trimode's settings file.
#[must_use]
pub fn read(files: &Files) -> Reading {
    let mut reading = Reading::default();
    let Some(bytes) = files.get(SETTINGS) else {
        return reading;
    };
    let ini = Ini::new(bytes.clone());
    reading.version = text(&ini, "Main", "Product Version");
    let base = callsign(&ini, "Registration", "Base Callsign");
    let channels = channels(&ini);
    if channels.is_empty() {
        reading
            .notes
            .push("no channel is offered on VARA HF".to_owned());
    }
    // the callsigns on the air, the first channel's first; the base callsign only when no
    // channel names one (it is the account Trimode logs in with, not a station on the air)
    let mut callsigns: Vec<String> = Vec::new();
    for call in channels.iter().filter_map(|c| c.callsign.clone()) {
        if !callsigns.contains(&call) {
            callsigns.push(call);
        }
    }
    if callsigns.is_empty() {
        callsigns.extend(base.clone());
    }
    if callsigns.is_empty() && !channels.is_empty() {
        reading.notes.push("no callsign is set".to_owned());
    }
    for channel in &channels {
        let aether = channel.service_code.as_deref().is_some_and(|codes| {
            codes
                .split([' ', ','])
                .any(|code| code.eq_ignore_ascii_case(AETHER_SERVICE_CODE))
        });
        if !aether {
            reading.notes.push(format!(
                "channel {} is not published under the service code {AETHER_SERVICE_CODE}, \
                 which every Aether gateway uses",
                channel.number
            ));
        }
    }
    if ini
        .get("Vara TNC", "Enabled")
        .is_some_and(|v| !yes(Some(&v)))
    {
        reading
            .notes
            .push("the VARA TNC is turned off in Trimode".to_owned());
    }
    let mut tnc = Tnc::tcp(
        TncKind::VaraHf,
        &text(&ini, "Vara TNC", "TNC Address").unwrap_or_else(|| VARA_HOST.to_owned()),
        number(&ini, "Vara TNC", "TCP Port").unwrap_or(VARA_PORT),
    );
    // Trimode starts its VARA program itself, always, and restarts it when it stops answering
    tnc.program = text(&ini, "Vara TNC", "Vara Program File");
    tnc.launches = true;
    let first = channels.first();
    reading.setups.push(ProgramSetup {
        name: base
            .or_else(|| callsigns.first().cloned())
            .unwrap_or_else(|| Program::RmsTrimode.name().to_owned()),
        callsigns,
        session: Some(SESSION.to_owned()),
        // the widest channel: Trimode says `BW` for each as it scans, and Aether follows it
        bandwidth_hz: channels.iter().map(|c| c.bandwidth_hz).max(),
        frequency_hz: first.map(|c| c.centre_hz),
        channels_hz: channels.iter().map(|c| c.centre_hz).collect(),
        gateway: true,
        radio: radio(&ini),
        tncs: vec![tnc],
    });
    reading
}

/// The settings Trimode needs to reach Aether, of its VARA TNC only: this computer when it
/// names another (a port of its own here is followed by the profile instead), and Aether's
/// application as the program it starts and watches as its VARA — the gateway guide's first
/// way (§3.2), which since ADR-0097 starts Aether only if no modem runs. Never `Enabled` or a
/// channel's protocols: those publish the gateway's channels to Winlink.
#[must_use]
pub fn point(files: &Files, target: &super::point::Target) -> Vec<super::point::Edit> {
    use super::point::Edit;
    const S: &str = "Vara TNC";
    let Some(bytes) = files.get(SETTINGS) else {
        return Vec::new();
    };
    let ini = Ini::new(bytes.clone());
    let mut edits = Vec::new();
    if text(&ini, S, "TNC Address").is_some_and(|h| !super::is_local(&h)) {
        edits.push(Edit {
            file: SETTINGS,
            section: S,
            key: "TNC Address",
            to: VARA_HOST.to_owned(),
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
    let program = text(&ini, S, "Vara Program File");
    let is_aether = program.as_deref().is_some_and(super::import::is_aether_app);
    if let Some(app) = &target.app
        && !is_aether
    {
        edits.push(Edit {
            file: SETTINGS,
            section: S,
            key: "Vara Program File",
            to: app.clone(),
            why: "Trimode starts and watches the program named here as its VARA: Aether's \
                  application stands in, and starts Aether only if no modem runs",
        });
    }
    edits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `RMS Trimode.ini` as Trimode 1.4.4 writes one, every value made up, with the personal
    /// keys a real one has (which must never be read). Channel 1 is an Aether channel; channel 2
    /// had VARA unticked; channels 3 to 8 are the empty rows a new install writes.
    const INI: &str = "[Main]\r\nProcess ID=1234\r\nProduct Version=1.4.4.0\r\n\
        [Site Properties]\r\nGrid Square=FN31pr\r\nService Code=PUBLIC\r\nCmdPort=8510\r\n\
        Notify Email=sysop@example.com\r\n\
        [Registration]\r\nBase Callsign=n0call\r\nPassword2=secret\r\nEmail=sysop@example.com\r\n\
        Sysop Name=Pat Sysop\r\nStreet Address 1=1 Main St\r\nCity=Anytown\r\n\
        Phone Number=555-0100\r\n\
        [Channels]\r\nCall 1=N0CALL-1\r\nCall 2=N0CALL\r\nCall 3=N0CALL\r\nSC 1=AETHER\r\n\
        SC 2=PUBLIC\r\nSC 3=PUBLIC\r\nMode 1=268435488\r\nMode 2=268435456\r\nMode 3=268435488\r\n\
        Frequency 1=7102500\r\nFrequency 2=7102500\r\nFrequency 3=0\r\n\
        Wide Bandwidth 1=True\r\nWide Bandwidth 2=True\r\nWide Bandwidth 3=True\r\n\
        [Vara TNC]\r\nEnabled=True\r\nRadioControlClass=Yaesu FTDX10\r\n\
        Vara Program File=C:\\Users\\op\\AppData\\Local\\Aether HF\\aether-hf.exe\r\n\
        TCP Port=8300\r\nTNC Address=127.0.0.1\r\nUse CAT PTT=True\r\n\
        Use Serial Port PTT=False\r\nUse DTR=True\r\nUse RTS=True\r\nBW2750=False\r\nPTT Port=\r\n\
        [Radio]\r\nControl Baud=38400\r\nControl Port=COM6\r\nIcom Address=00\r\n\
        Model=Yaesu FTDX10\r\n\
        [GPS]\r\nSerial Port=COM20\r\n";

    fn files(text: &str) -> Files {
        let mut files = Files::new();
        files.insert(SETTINGS.to_owned(), text.as_bytes().to_vec());
        files
    }

    fn with(changes: &[(&str, &str)]) -> Reading {
        let mut text = INI.to_owned();
        for (from, to) in changes {
            assert!(text.contains(from), "{from}");
            text = text.replace(from, to);
        }
        read(&files(&text))
    }

    #[test]
    fn pointing_names_aethers_application_and_never_what_publishes_channels() {
        let target = crate::hostapps::point::Target {
            host_port: 8300,
            app: Some("C:\\Aether HF\\aether-hf.exe".into()),
        };
        // it runs Aether already, here: nothing to change
        assert!(point(&files(INI), &target).is_empty());
        // VARA's own program, on another computer, with the TNC turned off
        let text = INI
            .replace(
                "C:\\Users\\op\\AppData\\Local\\Aether HF\\aether-hf.exe",
                "C:\\VARA\\VARA.exe",
            )
            .replace("TNC Address=127.0.0.1", "TNC Address=192.168.1.9")
            .replace("Enabled=True", "Enabled=False");
        let edits = point(&files(&text), &target);
        let keys: Vec<&str> = edits.iter().map(|e| e.key).collect();
        assert_eq!(keys, ["TNC Address", "TCP Port", "Vara Program File"]);
        assert_eq!(edits[2].to, "C:\\Aether HF\\aether-hf.exe");
        assert!(edits.iter().all(|e| e.section == "Vara TNC"));
        let changes = crate::hostapps::point::changes(Program::RmsTrimode, &files(&text), &target);
        assert_eq!(changes.len(), 2, "the port is 8300 already: {changes:?}");
        // with no application known, the program path is left
        let none = crate::hostapps::point::Target {
            app: None,
            ..target
        };
        assert_eq!(point(&files(&text), &none).len(), 2);
        // the channels the gateway offers are read whole
        assert_eq!(read(&files(INI)).setups[0].channels_hz, [7_102_500]);
    }

    #[test]
    fn a_gateway_is_one_setup_named_after_its_base_callsign() {
        let reading = read(&files(INI));
        assert_eq!(reading.version.as_deref(), Some("1.4.4.0"));
        assert_eq!(reading.setups.len(), 1);
        let setup = &reading.setups[0];
        assert_eq!(setup.name, "N0CALL");
        // channel 2 offers no VARA and channel 3 has no frequency: neither is read
        assert_eq!(setup.callsigns, ["N0CALL-1"]);
        assert_eq!(setup.session.as_deref(), Some(SESSION));
        assert_eq!(setup.bandwidth_hz, Some(2300));
        assert_eq!(setup.frequency_hz, Some(7_102_500));
        assert!(setup.gateway);
        assert!(reading.notes.is_empty(), "{:?}", reading.notes);
    }

    #[test]
    fn the_radio_and_the_vara_tnc_are_read() {
        let setup = &read(&files(INI)).setups[0];
        let radio = setup.radio.clone().expect("a radio");
        assert_eq!(radio.rig.as_deref(), Some("Yaesu FTDX10"));
        assert_eq!(radio.cat_port.as_deref(), Some("COM6"));
        assert_eq!(radio.cat_baud, Some(38400));
        assert_eq!(radio.icom_address, None, "not an Icom");
        assert_eq!(radio.ptt, Ptt::Cat);
        assert_eq!(setup.tncs.len(), 1);
        let vara = &setup.tncs[0];
        assert_eq!(vara.kind, TncKind::VaraHf);
        assert_eq!(vara.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(vara.port, Some(8300));
        assert!(
            vara.program
                .as_deref()
                .is_some_and(|p| p.ends_with("aether-hf.exe"))
        );
        assert!(vara.launches);
    }

    #[test]
    fn every_vara_channel_lends_its_callsign_and_its_bandwidth() {
        let reading = with(&[
            ("Mode 2=268435456", "Mode 2=268435488"),
            ("Call 2=N0CALL", "Call 2=N0CALL-2"),
            ("Frequency 2=7102500", "Frequency 2=3592000"),
            ("Wide Bandwidth 1=True", "Wide Bandwidth 1=False"),
            ("SC 2=PUBLIC", "SC 2=AETHER"),
        ]);
        let setup = &reading.setups[0];
        assert_eq!(setup.callsigns, ["N0CALL-1", "N0CALL-2"]);
        // the widest channel's; the first channel's frequency
        assert_eq!(setup.bandwidth_hz, Some(2300));
        assert_eq!(setup.frequency_hz, Some(7_102_500));
        // all of them narrow, and 2750 when Trimode uses it for a wide one
        let narrow = with(&[("Wide Bandwidth 1=True", "Wide Bandwidth 1=False")]);
        assert_eq!(narrow.setups[0].bandwidth_hz, Some(500));
        let wider = with(&[("BW2750=False", "BW2750=True")]);
        assert_eq!(wider.setups[0].bandwidth_hz, Some(2750));
    }

    #[test]
    fn the_keying_is_read_as_trimode_sets_it() {
        // a serial line: the port and the lines Trimode raises
        let serial = with(&[
            ("Use CAT PTT=True", "Use CAT PTT=False"),
            ("Use Serial Port PTT=False", "Use Serial Port PTT=True"),
            ("PTT Port=\r\n", "PTT Port=COM9\r\n"),
            ("Use DTR=True", "Use DTR=False"),
        ]);
        assert_eq!(
            serial.setups[0].radio.clone().expect("a radio").ptt,
            Ptt::Serial {
                port: "COM9".to_owned(),
                rts: true,
                dtr: false,
            }
        );
        // no radio, no keying: *Manual*, VOX or Aether keying the radio
        let manual = with(&[
            ("Use CAT PTT=True", "Use CAT PTT=False"),
            ("Control Port=COM6", "Control Port="),
            ("Model=Yaesu FTDX10", "Model=None"),
        ]);
        assert_eq!(manual.setups[0].radio, None);
        // an Icom's CI-V address, kept in hex
        let icom = with(&[
            ("Model=Yaesu FTDX10", "Model=Icom IC-7300"),
            ("Icom Address=00", "Icom Address=94"),
            ("Control Baud=38400", "Control Baud=19200"),
        ]);
        let radio = icom.setups[0].radio.clone().expect("a radio");
        assert_eq!(radio.icom_address, Some(0x94));
        assert_eq!(radio.cat_baud, Some(19200));
    }

    #[test]
    fn what_keeps_it_from_an_aether_gateway_is_said() {
        let public = with(&[("SC 1=AETHER", "SC 1=PUBLIC")]);
        assert!(
            public
                .notes
                .iter()
                .any(|n| n.contains("channel 1") && n.contains(AETHER_SERVICE_CODE)),
            "{:?}",
            public.notes
        );
        // a code among others still counts
        let both = with(&[("SC 1=AETHER", "SC 1=PUBLIC AETHER")]);
        assert!(both.notes.is_empty(), "{:?}", both.notes);
        let off = with(&[("Enabled=True", "Enabled=False")]);
        assert!(
            off.notes
                .iter()
                .any(|n| n.contains("VARA TNC is turned off"))
        );
        let none = with(&[("Mode 1=268435488", "Mode 1=268435456")]);
        assert!(none.notes.iter().any(|n| n.contains("no channel")));
        assert!(
            none.setups[0].callsigns == ["N0CALL"],
            "the base callsign then"
        );
    }

    #[test]
    fn a_second_gateway_is_told_by_its_callsigns() {
        let other = with(&[
            ("Base Callsign=n0call", "Base Callsign=N1CALL"),
            ("Call 1=N0CALL-1", "Call 1=N1CALL-10"),
            ("TCP Port=8300", "TCP Port=8310"),
        ]);
        let first = &read(&files(INI)).setups[0];
        let second = &other.setups[0];
        assert_ne!(first.name, second.name);
        assert_eq!(second.callsigns, ["N1CALL-10"]);
        assert_eq!(second.tncs[0].port, Some(8310));
    }

    #[test]
    fn no_personal_value_is_read() {
        let json = serde_json::to_string(&read(&files(INI))).expect("json");
        for secret in [
            "secret",
            "555-0100",
            "Main St",
            "Anytown",
            "sysop@example.com",
            "Pat Sysop",
            "FN31",
            "COM20",
        ] {
            assert!(!json.contains(secret), "{secret} in {json}");
        }
    }

    #[test]
    fn a_file_with_nothing_set_is_still_read() {
        let reading = read(&files("[Main]\r\nProduct Version=1.4.4.0\r\n"));
        assert_eq!(reading.setups.len(), 1);
        let setup = &reading.setups[0];
        assert_eq!(setup.name, "RMS Trimode");
        assert!(setup.callsigns.is_empty());
        assert_eq!(setup.tncs[0].port, Some(VARA_PORT));
        assert_eq!(setup.radio, None);
        assert!(reading.notes.iter().any(|n| n.contains("no channel")));
    }
}
