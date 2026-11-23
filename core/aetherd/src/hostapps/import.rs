//! A program's setup turned into an Aether profile (ADR-0133).
//!
//! What a profile made from a setup sets, over the station's own settings: the program's
//! callsign, its bandwidth, Aether's host interface on (or its KISS port, for a program that
//! speaks KISS), the host port the program points at, and the keying. Who keys the radio is
//! the one choice:
//!
//! * **The program keys** ([`Keying::Program`], the default whenever the program keys the radio
//!   itself): Aether's keying is *Host program*, as with `VarAC`. The program's radio setup is
//!   left as it is — it keeps keying on Aether's `PTT ON` and tuning the radio (a client's
//!   channel, a gateway's scan) — so nothing in the program changes. Aether reads no dial then,
//!   so its rules check judges at the program's own channel, declared as the dial when the
//!   profile is switched to ([`Plan::dial_hz`]; the author's decision of 2026-11-19). A program
//!   that scans several channels leaves the rules check off: Aether cannot tell which it is on.
//! * **Aether keys** ([`Keying::Aether`]): Aether keys the radio the way the program did — by
//!   CAT on the program's port and speed, or by the program's RTS/DTR line — and its rules
//!   check stays on; the program's radio must be set to *Manual* and its PTT port to *None*,
//!   since only one program can hold the radio's CAT port.
//!
//! A program that keys nothing (VOX, or no radio set up) leaves Aether's keying as it is.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Program, ProgramSetup, Ptt, Tnc, TncKind, find::Install, is_local};

/// Who keys the radio in a profile made from a program's setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keying {
    /// The program, on Aether's `PTT ON`: Aether's keying is *Host program*.
    Program,
    /// Aether, the way the program did; the program's radio set to *Manual*.
    Aether,
}

/// Aether as it stands, for comparing a program's TNC with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aether {
    /// The host interface's address (`127.0.0.1:8300`).
    pub host_bind: String,
    /// The KISS port's.
    pub kiss_bind: String,
}

impl Aether {
    /// The host interface's port.
    #[must_use]
    pub fn host_port(&self) -> u16 {
        crate::varac::port_of(&self.host_bind, 8300)
    }

    /// The KISS port's port.
    #[must_use]
    pub fn kiss_port(&self) -> u16 {
        crate::varac::port_of(&self.kiss_bind, 8100)
    }
}

/// Where a program's TNC points, against this Aether.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pointing {
    /// At this Aether's port for its kind.
    Aether,
    /// At this computer, on another port.
    OtherPort,
    /// At another computer.
    OtherHost,
    /// At a serial port.
    Serial,
    /// At something Aether does not serve (VARA FM, ARDOP, AGW).
    NotServed,
}

/// Where a TNC points, against this Aether.
#[must_use]
pub fn pointing(tnc: &Tnc, aether: &Aether) -> Pointing {
    let ours = match tnc.kind {
        TncKind::VaraHf => aether.host_port(),
        TncKind::Kiss => aether.kiss_port(),
        _ => return Pointing::NotServed,
    };
    match (&tnc.host, tnc.port) {
        (Some(host), Some(port)) if is_local(host) => {
            if port == ours {
                Pointing::Aether
            } else {
                Pointing::OtherPort
            }
        }
        (Some(_), _) => Pointing::OtherHost,
        _ => Pointing::Serial,
    }
}

/// The TNC a profile is made for: the program's VARA HF, else its KISS TNC.
#[must_use]
pub fn served_tnc(setup: &ProgramSetup) -> Option<&Tnc> {
    let of = |kind| setup.tncs.iter().find(|t| t.kind == kind);
    of(TncKind::VaraHf).or_else(|| of(TncKind::Kiss))
}

/// A CAT command set Aether speaks, from the program's name for the radio.
#[must_use]
pub fn cat_protocol(rig: Option<&str>) -> Option<&'static str> {
    let rig = rig?.trim().to_lowercase();
    if rig.starts_with("yaesu") {
        Some("yaesu")
    } else if rig.starts_with("kenwood") || rig.starts_with("elecraft") || rig.starts_with("flex") {
        Some("kenwood")
    } else if rig.starts_with("icom") {
        Some("icom")
    } else {
        None
    }
}

/// What a profile made from one of a program's setups will be: its name, the settings it sets
/// over the station's, what the program must say to match it, or why none can be made.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    /// The program.
    pub program: Program,
    /// The install's folder.
    pub folder: String,
    /// The setup's name in the program.
    pub setup: String,
    /// The profile's name: the program's and the callsign.
    pub profile: String,
    /// Who keys the radio in it.
    pub keying: Option<Keying>,
    /// The keyings this setup allows, the default first.
    pub keyings: Vec<Keying>,
    /// The settings it sets, as dotted keys, over the station's own.
    pub changes: Value,
    /// What the program must say for the profile to work.
    pub program_must: Vec<Advice>,
    /// What else the operator should know.
    pub notes: Vec<Advice>,
    /// Where the program's TNC points.
    pub pointing: Option<Pointing>,
    /// The dial declared for the rules check when the station is switched to the profile: the
    /// program's channel, for a program that keys the radio and tunes it to one. A dial is this
    /// computer's setting, not a profile's, so it is set at the switch, not kept in the profile.
    pub dial_hz: Option<u64>,
    /// Why no profile can be made from it, when none can.
    pub refused: Option<String>,
}

/// Where the dial sits for a channel's centre: the audio centre is 1.5 kHz above the
/// suppressed carrier on the upper sideband, as every VARA program tunes (Winlink Express's
/// *USB Dial* is its centre less 1.5 kHz).
pub const CENTRE_ABOVE_DIAL_HZ: u64 = 1500;

/// A thing to say to the operator: a few words for the screen (a value: `Radio Manual, PTT
/// port None`), and why, for its tooltip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Advice {
    /// The few words.
    pub what: String,
    /// Why.
    pub why: String,
    /// What Aether can set in the program itself ([`super::point`]): the panel leaves it out
    /// while the operator has it written.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub settable: bool,
}

/// Advice, from its two parts.
fn advice(what: impl Into<String>, why: impl Into<String>) -> Advice {
    Advice {
        what: what.into(),
        why: why.into(),
        settable: false,
    }
}

/// Whether a program a TNC starts is Aether's own application: a program that starts it needs
/// nothing said.
#[must_use]
pub fn is_aether_app(program: &str) -> bool {
    let name = program.rsplit(['\\', '/']).next().unwrap_or(program);
    name.eq_ignore_ascii_case("aether-hf.exe")
}

/// Advice the program's reader can carry out itself.
fn settable(what: impl Into<String>, why: impl Into<String>) -> Advice {
    Advice {
        settable: true,
        ..advice(what, why)
    }
}

/// The keyings a setup allows, the default first: the program keys whenever it keys the radio
/// itself; Aether keys whenever it can do so as the program did.
#[must_use]
pub fn keyings(setup: &ProgramSetup) -> Vec<Keying> {
    let Some(radio) = &setup.radio else {
        return Vec::new();
    };
    let aether_can = match &radio.ptt {
        Ptt::Cat => radio.cat_port.is_some() && cat_protocol(radio.rig.as_deref()).is_some(),
        Ptt::Serial { rts, dtr, .. } => *rts || *dtr,
        Ptt::None | Ptt::Other { .. } => false,
    };
    let program_keys = !matches!(radio.ptt, Ptt::None);
    let mut out = Vec::new();
    if program_keys {
        out.push(Keying::Program);
    }
    if aether_can {
        out.push(Keying::Aether);
    }
    out
}

/// The bandwidth Aether runs for the program's own figure: 2300 Hz for VARA's 2300 and 2750.
#[must_use]
pub fn aether_bandwidth(program_hz: Option<u32>) -> Option<u32> {
    match program_hz? {
        500 => Some(500),
        2300 | 2750 => Some(2300),
        _ => None,
    }
}

/// The plan for one setup of an install, with the keying asked for (the default if `None`, or
/// if the setup does not allow the one asked for).
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn plan(
    install: &Install,
    setup: &ProgramSetup,
    asked: Option<Keying>,
    aether: &Aether,
) -> Plan {
    let call = setup
        .callsigns
        .first()
        .cloned()
        .unwrap_or_else(|| setup.name.clone());
    let mut plan = Plan {
        program: install.program,
        folder: install.folder.clone(),
        setup: setup.name.clone(),
        profile: format!("{} {}", install.program.name(), call.to_uppercase()),
        keying: None,
        keyings: keyings(setup),
        changes: json!({}),
        program_must: Vec::new(),
        notes: Vec::new(),
        pointing: None,
        dial_hz: None,
        refused: None,
    };
    if setup.frequency_hz.is_some_and(|hz| hz > 30_000_000) {
        plan.refused = Some("works above 30 MHz: it waits for Aether FM".to_owned());
        return plan;
    }
    let Some(tnc) = served_tnc(setup) else {
        let fm = setup
            .tncs
            .iter()
            .any(|t| matches!(t.kind, TncKind::VaraFm | TncKind::Agw));
        plan.refused = Some(if fm {
            "its TNC is VARA FM or AGW: it waits for Aether FM".to_owned()
        } else {
            "it points at no VARA HF or KISS TNC, which are what Aether answers as".to_owned()
        });
        return plan;
    };
    let pointing = pointing(tnc, aether);
    plan.pointing = Some(pointing);
    let mut changes = serde_json::Map::new();
    if !call.is_empty() {
        changes.insert("callsign".into(), json!(call.to_uppercase()));
    }
    let what = if tnc.kind == TncKind::VaraHf {
        changes.insert("host.enabled".into(), json!(true));
        "VARA HF TNC"
    } else {
        changes.insert("kiss.enabled".into(), json!(true));
        "KISS TNC"
    };
    if let Some(hz) = aether_bandwidth(setup.bandwidth_hz) {
        changes.insert("radio.bandwidth".into(), json!(hz));
    }
    if setup.gateway {
        // a gateway is an automatically controlled station: only a program answers its calls
        // (ADR-0107), and nothing it starts goes over another station
        changes.insert("regulatory.control".into(), json!("automatic"));
        changes.insert("radio.wait_for_clear".into(), json!(true));
    }
    match pointing {
        Pointing::Aether | Pointing::NotServed => {}
        Pointing::OtherPort if tnc.kind == TncKind::VaraHf => {
            // the host interface's address is a profile's: Aether listens where the program
            // already points, and the program needs no change
            let port = tnc.port.unwrap_or_default();
            let address = aether
                .host_bind
                .rsplit_once(':')
                .map_or("127.0.0.1", |(a, _)| a);
            changes.insert("host.bind".into(), json!(format!("{address}:{port}")));
            plan.notes.push(advice(
                format!("Aether on port {port}"),
                format!(
                    "Aether listens for it on port {port}, where it points, while this profile \
                     is in use (now {}): the program needs no change",
                    aether.host_port()
                ),
            ));
        }
        Pointing::OtherPort => plan.program_must.push(advice(
            format!("{what} 127.0.0.1:{}", aether.kiss_port()),
            format!(
                "it points at port {}: Aether's KISS port belongs to this computer, not to a \
                 profile, so the program changes",
                tnc.port.unwrap_or_default()
            ),
        )),
        Pointing::OtherHost => {
            let mut said = advice(
                format!("{what} 127.0.0.1:{}", aether.host_port()),
                format!(
                    "it points at another computer, {}: Aether is on this one",
                    tnc.host.as_deref().unwrap_or_default()
                ),
            );
            // a VARA HF TNC's address and port are what the readers write; a KISS one's are not
            said.settable = tnc.kind == TncKind::VaraHf;
            plan.program_must.push(said);
        }
        Pointing::Serial => plan.program_must.push(advice(
            format!("{what} on TCP 127.0.0.1:{}", aether.kiss_port()),
            "it points at a serial port: Aether's KISS port is a TCP port",
        )),
    }
    if tnc.kind == TncKind::VaraHf && tnc.launches {
        if install.program == Program::WinlinkExpress {
            plan.program_must.push(settable(
                "Auto Launch Vara off",
                "Winlink Express ends the TNC it started when it closes: start Aether first, \
                 and leave the launch off",
            ));
        } else if let Some(program) = tnc.program.as_ref().filter(|p| !is_aether_app(p)) {
            plan.notes.push(settable(
                "Starts its own TNC",
                format!(
                    "it starts {program} as its TNC: name Aether's application there, or turn \
                     its launch off and start Aether first"
                ),
            ));
        }
    }
    let keying = asked
        .filter(|k| plan.keyings.contains(k))
        .or_else(|| plan.keyings.first().copied());
    plan.keying = keying;
    let radio = setup.radio.as_ref();
    match (keying, radio) {
        (Some(Keying::Program), _) => {
            changes.insert("ptt.kind".into(), json!("host"));
            plan.notes.push(advice(
                "Keeps its radio setup",
                "it keys the radio on Aether's PTT ON and tunes it, as it did with VARA: \
                 nothing in its radio setup changes",
            ));
            program_dial(&mut plan, &mut changes, setup, install.program);
        }
        (Some(Keying::Aether), Some(radio)) => {
            if let Ptt::Serial { port, rts, dtr } = &radio.ptt {
                let line = match (rts, dtr) {
                    (true, true) => "both",
                    (false, true) => "dtr",
                    _ => "rts",
                };
                changes.insert("ptt.kind".into(), json!("serial"));
                changes.insert("ptt.port".into(), json!(port));
                changes.insert("ptt.line".into(), json!(line));
                let shared = radio
                    .cat_port
                    .as_deref()
                    .is_some_and(|c| c.eq_ignore_ascii_case(port));
                if shared {
                    hand_over(
                        &mut plan,
                        install.program,
                        true,
                        format!(
                            "Aether keys on {port}, which is also the radio's CAT port, and \
                             only one program can hold it"
                        ),
                    );
                } else {
                    hand_over(
                        &mut plan,
                        install.program,
                        false,
                        format!(
                            "Aether keys on {port}; the program can keep its frequency control \
                             on {}",
                            radio.cat_port.as_deref().unwrap_or("the CAT port")
                        ),
                    );
                }
            } else {
                let port = radio.cat_port.clone().unwrap_or_default();
                changes.insert("ptt.kind".into(), json!("cat"));
                changes.insert("ptt.port".into(), json!(port));
                let protocol = cat_protocol(radio.rig.as_deref()).unwrap_or("yaesu");
                changes.insert("ptt.protocol".into(), json!(protocol));
                if let Some(baud) = radio.cat_baud {
                    changes.insert("ptt.baud".into(), json!(baud));
                }
                if protocol == "icom"
                    && let Some(address) = radio.icom_address
                {
                    changes.insert("ptt.civ_address".into(), json!(address));
                }
                hand_over(
                    &mut plan,
                    install.program,
                    true,
                    format!(
                        "Aether keys the radio by CAT on {port}, and only one program can hold \
                         that port"
                    ),
                );
            }
        }
        _ => {
            let why = if radio.is_some_and(|r| r.ptt == Ptt::None) {
                "it keys the radio by VOX: Aether's keying is left as it is"
            } else {
                "it controls no radio: Aether's keying is left as it is"
            };
            plan.notes.push(advice("Keying as it is", why));
        }
    }
    plan.changes = Value::Object(changes);
    plan
}

/// What the program must change for Aether to key the radio: its keying off, and with `radio`
/// (Aether keys on the port its radio control holds) its radio *Manual*. The readers write both,
/// in the words each program writes itself (read from copies set by hand, 2026-11-23).
fn hand_over(plan: &mut Plan, program: Program, radio: bool, why: String) {
    let what = match (program, radio) {
        (Program::WinlinkExpress, true) => "Radio Manual, PTT port External",
        (Program::WinlinkExpress, false) => "PTT port External",
        (_, true) => "Radio Manual, PTT off",
        (_, false) => "PTT off",
    };
    plan.program_must.push(settable(what, why));
}

/// A frequency in megahertz as the panel writes one: at least three decimals, no trailing
/// zeros past them (`3.5905`, `14.105`).
#[must_use]
pub fn mhz(hz: u64) -> String {
    let mut text = format!("{}.{:06}", hz / 1_000_000, hz % 1_000_000);
    while text.ends_with('0') && text.len() - text.find('.').unwrap_or(0) > 4 {
        text.pop();
    }
    text
}

/// The rules check for a program that keys the radio and tunes it: kept, at the dial of the
/// program's one channel; kept with the dial left to declare when the program names none; off
/// when it scans several, since Aether cannot tell which it is on.
fn program_dial(
    plan: &mut Plan,
    changes: &mut serde_json::Map<String, Value>,
    setup: &ProgramSetup,
    program: Program,
) {
    let mut centres: Vec<u64> = if setup.channels_hz.is_empty() {
        setup.frequency_hz.into_iter().collect()
    } else {
        setup.channels_hz.clone()
    };
    centres.sort_unstable();
    centres.dedup();
    match centres.as_slice() {
        [] => plan.notes.push(advice(
            "Dial to declare",
            "the program names no frequency, so Aether cannot place its signal yet: say where \
             the dial is on the Session tab (Dial is at) before the first transmission. The \
             rules check stays on",
        )),
        [centre] => {
            // said once, beside the switch that declares it: the panel's *Switch to it now and
            // set the dial to …*
            let dial = centre.saturating_sub(CENTRE_ABOVE_DIAL_HZ);
            plan.dial_hz = Some(dial);
            changes.insert("regulatory.sideband".into(), json!("usb"));
        }
        several => {
            changes.insert("regulatory.profile".into(), json!("none"));
            plan.notes.push(advice(
                "Rules check off",
                format!(
                    "{} scans {} channels and Aether reads no dial, so it cannot tell which \
                     one a transmission goes out on: its rules check is off, and you check \
                     every channel yourself, as with VARA",
                    program.name(),
                    several.len()
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostapps::{Radio, find::FoundBy};

    fn aether() -> Aether {
        Aether {
            host_bind: "127.0.0.1:8300".into(),
            kiss_bind: "127.0.0.1:8100".into(),
        }
    }

    fn setup(ptt: Ptt, port: u16) -> ProgramSetup {
        ProgramSetup {
            name: "N0CALL".into(),
            callsigns: vec!["n0call".into()],
            session: Some("Vara HF P2P".into()),
            bandwidth_hz: Some(2750),
            frequency_hz: Some(7_083_500),
            channels_hz: Vec::new(),
            gateway: false,
            radio: Some(Radio {
                rig: Some("Icom IC-7300".into()),
                cat_port: Some("COM4".into()),
                cat_baud: Some(19200),
                icom_address: Some(0x94),
                ptt,
            }),
            tncs: vec![Tnc::tcp(TncKind::VaraHf, "127.0.0.1", port)],
        }
    }

    fn install(setup: &ProgramSetup) -> Install {
        Install {
            program: Program::WinlinkExpress,
            program_name: "Winlink Express",
            folder: "C:\\RMS Express".into(),
            exe: "C:\\RMS Express\\RMS Express.exe".into(),
            found_by: vec![FoundBy::Usual],
            running: false,
            version: Some("1.8.6.0".into()),
            callsigns: setup.callsigns.clone(),
            setups: vec![setup.clone()],
            notes: Vec::new(),
        }
    }

    #[test]
    fn a_program_that_keys_the_radio_keeps_it_by_default() {
        let s = setup(Ptt::Cat, 8300);
        let plan = plan(&install(&s), &s, None, &aether());
        assert_eq!(plan.profile, "Winlink Express N0CALL");
        assert_eq!(plan.keyings, [Keying::Program, Keying::Aether]);
        assert_eq!(plan.keying, Some(Keying::Program));
        assert_eq!(plan.pointing, Some(Pointing::Aether));
        let expected = json!({
            "callsign": "N0CALL",
            "host.enabled": true,
            "radio.bandwidth": 2300,
            "ptt.kind": "host",
            "regulatory.sideband": "usb",
        });
        assert_eq!(plan.changes, expected, "the rules check stays on");
        assert_eq!(
            plan.dial_hz,
            Some(7_082_000),
            "the channel's centre less 1.5 kHz"
        );
        assert!(
            plan.notes.iter().any(|n| n.what == "Keeps its radio setup"),
            "{:?}",
            plan.notes
        );
        assert!(plan.program_must.is_empty(), "{:?}", plan.program_must);
        assert!(plan.refused.is_none());
    }

    #[test]
    fn a_program_that_scans_leaves_the_rules_off_and_one_with_no_channel_declares_later() {
        let mut gate = setup(Ptt::Cat, 8300);
        gate.gateway = true;
        gate.channels_hz = vec![3_592_000, 7_103_500, 3_592_000];
        let p = plan(&install(&gate), &gate, None, &aether());
        assert_eq!(p.changes["regulatory.profile"], "none");
        assert_eq!(p.dial_hz, None);
        assert!(
            p.notes.iter().any(|n| n.why.contains("scans 2 channels")),
            "{:?}",
            p.notes
        );
        // one channel listed twice is one channel
        gate.channels_hz = vec![3_592_000, 3_592_000];
        let p = plan(&install(&gate), &gate, None, &aether());
        assert_eq!(p.dial_hz, Some(3_590_500));
        assert!(p.changes.get("regulatory.profile").is_none());
        let mut nowhere = setup(Ptt::Cat, 8300);
        nowhere.frequency_hz = None;
        let p = plan(&install(&nowhere), &nowhere, None, &aether());
        assert_eq!(p.dial_hz, None);
        assert!(
            p.changes.get("regulatory.profile").is_none(),
            "{}",
            p.changes
        );
        assert!(p.notes.iter().any(|n| n.what == "Dial to declare"));
        assert_eq!(mhz(14_105_000), "14.105");
        assert_eq!(mhz(3_590_500), "3.5905");
        assert_eq!(mhz(7_101_250), "7.10125");
    }

    #[test]
    fn aether_keys_by_cat_as_the_program_did() {
        let s = setup(Ptt::Cat, 8300);
        let plan = plan(&install(&s), &s, Some(Keying::Aether), &aether());
        assert_eq!(plan.keying, Some(Keying::Aether));
        assert_eq!(plan.changes["ptt.kind"], "cat");
        assert_eq!(plan.changes["ptt.port"], "COM4");
        assert_eq!(plan.changes["ptt.protocol"], "icom");
        assert_eq!(plan.changes["ptt.baud"], 19200);
        assert_eq!(plan.changes["ptt.civ_address"], 0x94);
        assert!(
            plan.changes.get("regulatory.profile").is_none(),
            "the rules stay on"
        );
        assert!(
            plan.program_must[0].what.contains("Manual"),
            "{:?}",
            plan.program_must
        );
    }

    #[test]
    fn a_line_on_the_cat_port_and_one_beside_it() {
        let shared = Ptt::Serial {
            port: "COM4".into(),
            rts: true,
            dtr: false,
        };
        let s = setup(shared, 8300);
        let p = plan(&install(&s), &s, Some(Keying::Aether), &aether());
        assert_eq!(p.changes["ptt.kind"], "serial");
        assert_eq!(p.changes["ptt.line"], "rts");
        assert!(
            p.program_must[0].what.contains("Manual"),
            "{:?}",
            p.program_must
        );
        let beside = Ptt::Serial {
            port: "COM9".into(),
            rts: true,
            dtr: true,
        };
        let s = setup(beside, 8300);
        let p = plan(&install(&s), &s, Some(Keying::Aether), &aether());
        assert_eq!(p.changes["ptt.line"], "both");
        assert!(
            p.program_must[0].why.contains("frequency control"),
            "{:?}",
            p.program_must
        );
    }

    #[test]
    fn another_port_is_followed_and_another_computer_is_said() {
        let s = setup(Ptt::Cat, 8370);
        let p = plan(&install(&s), &s, None, &aether());
        assert_eq!(p.pointing, Some(Pointing::OtherPort));
        assert_eq!(p.changes["host.bind"], "127.0.0.1:8370");
        let mut far = setup(Ptt::Cat, 8300);
        far.tncs = vec![Tnc::tcp(TncKind::VaraHf, "192.168.1.9", 8300)];
        let p = plan(&install(&far), &far, None, &aether());
        assert_eq!(p.pointing, Some(Pointing::OtherHost));
        assert!(p.changes.get("host.bind").is_none());
        assert!(
            p.program_must.iter().any(|m| m.why.contains("192.168.1.9")),
            "{:?}",
            p.program_must
        );
    }

    #[test]
    fn a_gateway_is_under_automatic_control_and_waits_for_a_clear_channel() {
        let mut gate = setup(Ptt::Cat, 8700);
        gate.gateway = true;
        gate.bandwidth_hz = Some(2750);
        let p = plan(&install(&gate), &gate, None, &aether());
        assert_eq!(p.changes["regulatory.control"], "automatic");
        assert_eq!(p.changes["radio.wait_for_clear"], true);
        assert_eq!(p.changes["radio.bandwidth"], 2300);
        assert_eq!(p.changes["host.bind"], "127.0.0.1:8700");
        assert_eq!(p.changes["ptt.kind"], "host", "the gateway program keys");
        let client = setup(Ptt::Cat, 8300);
        let p = plan(&install(&client), &client, None, &aether());
        assert!(
            p.changes.get("regulatory.control").is_none(),
            "{}",
            p.changes
        );
    }

    #[test]
    fn each_program_is_told_its_own_words() {
        let gate = setup(Ptt::Cat, 8300);
        let trimode = Install {
            program: Program::RmsTrimode,
            program_name: "RMS Trimode",
            ..install(&gate)
        };
        let p = plan(&trimode, &gate, Some(Keying::Aether), &aether());
        let said: Vec<(&str, bool)> = p
            .program_must
            .iter()
            .map(|m| (m.what.as_str(), m.settable))
            .collect();
        assert_eq!(said, [("Radio Manual, PTT off", true)], "both written");
        let p = plan(&install(&gate), &gate, Some(Keying::Aether), &aether());
        assert_eq!(p.program_must[0].what, "Radio Manual, PTT port External");
        assert!(p.program_must[0].settable, "Winlink Express's is written");
    }

    #[test]
    fn vox_and_no_radio_leave_the_keying_alone() {
        let s = setup(Ptt::None, 8300);
        assert!(keyings(&s).is_empty(), "{:?}", keyings(&s));
        let p = plan(&install(&s), &s, Some(Keying::Program), &aether());
        assert_eq!(p.keying, None);
        assert!(p.changes.get("ptt.kind").is_none(), "{}", p.changes);
        let mut manual = setup(Ptt::None, 8300);
        manual.radio = None;
        let p = plan(&install(&manual), &manual, None, &aether());
        assert!(p.changes.get("ptt.kind").is_none());
        // an unknown radio keyed by command: only the program can key it
        let mut odd = setup(Ptt::Cat, 8300);
        if let Some(radio) = odd.radio.as_mut() {
            radio.rig = Some("Ten-Tec Orion".into());
        }
        assert_eq!(keyings(&odd), [Keying::Program]);
    }

    #[test]
    fn vhf_and_vara_fm_wait_for_aether_fm_but_kiss_alone_is_served() {
        let mut vhf = setup(Ptt::Cat, 8300);
        vhf.frequency_hz = Some(145_530_000);
        let p = plan(&install(&vhf), &vhf, None, &aether());
        assert!(
            p.refused
                .as_deref()
                .is_some_and(|r| r.contains("Aether FM"))
        );
        let mut fm = setup(Ptt::Cat, 8300);
        fm.tncs = vec![Tnc::tcp(TncKind::VaraFm, "127.0.0.1", 8250)];
        let p = plan(&install(&fm), &fm, None, &aether());
        assert!(
            p.refused
                .as_deref()
                .is_some_and(|r| r.contains("Aether FM"))
        );
        let mut kiss = setup(Ptt::Cat, 8300);
        kiss.tncs = vec![Tnc::tcp(TncKind::Kiss, "127.0.0.1", 8100)];
        let p = plan(&install(&kiss), &kiss, None, &aether());
        assert!(p.refused.is_none());
        assert_eq!(p.changes["kiss.enabled"], true);
        assert!(p.changes.get("host.enabled").is_none());
        assert_eq!(p.pointing, Some(Pointing::Aether));
    }

    #[test]
    fn winlink_express_is_told_to_leave_its_launch_off() {
        let mut s = setup(Ptt::Cat, 8300);
        s.tncs[0].launches = true;
        s.tncs[0].program = Some("C:\\VARA\\VARA.exe".into());
        let p = plan(&install(&s), &s, None, &aether());
        assert!(
            p.program_must
                .iter()
                .any(|m| m.what.contains("Launch") && m.settable),
            "what Aether can write for it: {:?}",
            p.program_must
        );
        let radio = p.notes.iter().find(|m| m.what.contains("radio setup"));
        assert!(radio.is_some_and(|m| !m.settable), "{:?}", p.notes);
        let theirs = plan(&install(&s), &s, Some(Keying::Aether), &aether());
        assert!(
            theirs
                .program_must
                .iter()
                .any(|m| m.what.contains("Manual") && m.settable),
            "the radio's keying is written for it too: {:?}",
            theirs.program_must
        );
        assert!(is_aether_app(
            "C:\\Users\\op\\AppData\\Local\\Aether HF\\Aether-HF.exe"
        ));
        assert!(!is_aether_app("C:\\VARA\\VARA.exe"));
        assert_eq!(aether_bandwidth(Some(500)), Some(500));
        assert_eq!(aether_bandwidth(Some(0)), None);
    }
}
