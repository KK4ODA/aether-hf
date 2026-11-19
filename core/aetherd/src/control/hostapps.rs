//! The `hostapps.*` methods: Winlink's programs found on this computer and a setup of one made
//! into an Aether profile ([`crate::hostapps`], ADR-0133).
//!
//! `hostapps.find` answers with the last find and begins another off the run loop (a
//! `hostapps` event carries it when it finishes); it also adds or forgets a folder to look in.
//! `hostapps.import` makes a profile of each setup asked for, or brings one made before up to
//! date, and with `load` switches the station to it, declaring the program's dial for the rules
//! check when the program keys the radio. `hostapps.point` (or `point` on an import) writes
//! into a program's settings what it needs to reach Aether — never while it runs, the file kept
//! as it was beside it (ADR-0133 §7).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    control::{
        methods::DaemonState,
        profiles,
        protocol::{ApiError, Response},
    },
    hostapps::{
        Program, ProgramSetup,
        find::{self as finding, Install, same_path},
        import::{self, Aether, Keying, Plan},
        point,
    },
    log::Level,
    profile::{Inventory, Profile},
    ptt::Ptt,
    station::Station,
};

/// The methods this module answers.
#[must_use]
pub fn handles(method: &str) -> bool {
    method.starts_with("hostapps.")
}

/// Answer one of this module's methods.
pub fn dispatch<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    method: &str,
    params: &Value,
    id: Option<String>,
) -> Response {
    let Some(daemon) = daemon else {
        return Response::failed(
            id,
            ApiError::new(
                "unsupported",
                "This modem keeps no files to make profiles in.",
                false,
            ),
        );
    };
    let result = match method {
        "hostapps.find" => find(daemon, params),
        "hostapps.import" => import(station, daemon, params),
        "hostapps.point" => point_method(station, daemon, params),
        other => Err(ApiError::new(
            "unknown_method",
            format!("This version does not have a method called {other:?}."),
            false,
        )),
    };
    match result {
        Ok(value) => Response::ok(id, value),
        Err(error) => Response::failed(id, error),
    }
}

/// Aether's ports as the station stands.
fn aether(daemon: &DaemonState) -> Aether {
    Aether {
        host_bind: daemon.config.host.bind.clone(),
        kiss_bind: daemon.config.kiss.bind.clone(),
    }
}

/// Aether as a program should reach it: the host port, and the application for a program
/// that starts its TNC itself.
fn target(daemon: &DaemonState) -> point::Target {
    point::Target {
        host_port: aether(daemon).host_port(),
        app: crate::varac::app_path().map(|p| p.display().to_string()),
    }
}

/// What writing an install's settings would change, read from its files now; nothing when
/// they cannot be read.
fn program_changes(install: &Install, target: &point::Target) -> Vec<point::Change> {
    point::files_of(install)
        .map(|files| point::changes(install.program, &files, target))
        .unwrap_or_default()
}

/// Every plan a setup allows: one per keying, the default first, or the one plan when the
/// setup leaves the keying alone.
fn plans(install: &Install, setup: &ProgramSetup, aether: &Aether) -> Vec<Plan> {
    let keyings = import::keyings(setup);
    if keyings.is_empty() {
        return vec![import::plan(install, setup, None, aether)];
    }
    keyings
        .into_iter()
        .map(|k| import::plan(install, setup, Some(k), aether))
        .collect()
}

/// An install as the clients are given it: each setup with its plans, and each TNC with where
/// it points.
fn install_json(install: &Install, aether: &Aether, target: &point::Target) -> Value {
    let mut value = json!(install);
    value["program_changes"] = json!(program_changes(install, target));
    let mut points_at_aether = false;
    if let Some(setups) = value["setups"].as_array_mut() {
        for (json, setup) in setups.iter_mut().zip(&install.setups) {
            if let Some(tncs) = json["tncs"].as_array_mut() {
                for (t, tnc) in tncs.iter_mut().zip(&setup.tncs) {
                    let pointing = import::pointing(tnc, aether);
                    points_at_aether |= pointing == import::Pointing::Aether;
                    t["pointing"] = json!(pointing);
                }
            }
            json["plans"] = json!(plans(install, setup, aether));
        }
    }
    value["points_at_aether"] = json!(points_at_aether);
    value
}

/// What `hostapps.find` answers and the `hostapps` event carries.
#[must_use]
pub fn answer(daemon: &DaemonState) -> Value {
    let (last, finding) = daemon.hostapps.last();
    let aether = aether(daemon);
    let target = target(daemon);
    let installs: Vec<Value> = last
        .iter()
        .flatten()
        .map(|i| install_json(i, &aether, &target))
        .collect();
    json!({
        "supported": cfg!(windows),
        "finding": finding,
        "found": last.is_some(),
        "installs": installs,
        "added": daemon.hostapps.remembered.folders,
        "took_ms": daemon.hostapps.took_ms().into_iter().map(|(stage, ms)| (stage.to_owned(), json!(ms))).collect::<serde_json::Map<String, Value>>(),
        "aether": {
            "host_port": aether.host_port(),
            "kiss_port": aether.kiss_port(),
        },
    })
}

fn text_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Whether a folder holds one of the programs the finder knows.
fn holds_a_program(folder: &Path) -> bool {
    Program::ALL
        .iter()
        .any(|p| folder.join(p.reader().exe).is_file())
}

/// `hostapps.find`: add or forget a folder if asked, begin a find, and answer with the last.
fn find(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    if let Some(folder) = text_param(params, "add") {
        let folder = PathBuf::from(folder);
        if !holds_a_program(&folder) {
            let names: Vec<&str> = Program::ALL.iter().map(|p| p.reader().exe).collect();
            return Err(ApiError::new(
                "not_found",
                format!("{} holds none of {}.", folder.display(), names.join(", ")),
                false,
            ));
        }
        daemon
            .hostapps
            .remember(&folder, true)
            .map_err(|e| ApiError::new("cannot_save", e, true))?;
    }
    if let Some(folder) = text_param(params, "forget") {
        daemon
            .hostapps
            .remember(Path::new(folder), false)
            .map_err(|e| ApiError::new("cannot_save", e, true))?;
    }
    daemon.hostapps.refresh();
    Ok(answer(daemon))
}

/// The install of the last find in a folder.
fn install_in(daemon: &DaemonState, folder: &str) -> Option<Install> {
    let (last, _) = daemon.hostapps.last();
    last.unwrap_or_default()
        .into_iter()
        .find(|i| same_path(Path::new(&i.folder), Path::new(folder)))
}

/// Write into an install's settings what its program needs to reach Aether, if anything, asking
/// the system first whether it runs; then read it again and put it in the last find's place,
/// so that a plan made next sees the program as it is now.
fn point_install(
    station_state: &str,
    daemon: &mut DaemonState,
    install: &Install,
) -> Result<(point::Pointed, Install), String> {
    let running = (daemon.hostapps.running)();
    let runs = running
        .iter()
        .any(|r| same_path(r, Path::new(&install.exe)));
    let pointed = point::point(install, &target(daemon), runs)?;
    let fresh = finding::reread(install, &running);
    daemon.hostapps.replace(fresh.clone());
    let detail = if pointed.changes.is_empty() {
        format!(
            "{} in {} reaches Aether already",
            install.program_name, install.folder
        )
    } else {
        let keys: Vec<String> = pointed
            .changes
            .iter()
            .map(|c| format!("{} {}", c.key, c.to))
            .collect();
        let kept: Vec<&str> = pointed.written.iter().map(|w| w.backup.as_str()).collect();
        format!(
            "{} in {} set to reach Aether: {} (the file as it was kept as {})",
            install.program_name,
            install.folder,
            keys.join(", "),
            kept.join(", ")
        )
    };
    daemon
        .log
        .record(Level::Info, "hostapps", &detail, station_state);
    Ok((pointed, fresh))
}

/// `hostapps.point`: write into the program in `folder` what it needs to reach Aether.
fn point_method<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let Some(folder) = text_param(params, "folder") else {
        return Err(ApiError::new(
            "invalid_params",
            "hostapps.point needs the install's folder",
            false,
        ));
    };
    let Some(install) = install_in(daemon, folder) else {
        return Err(ApiError::new(
            "not_found",
            format!("No program found in {folder}: find the programs again."),
            false,
        ));
    };
    let state = format!("{:?}", station.state());
    let (pointed, fresh) = point_install(&state, daemon, &install)
        .map_err(|why| ApiError::new("refused", why, false))?;
    let mut result = json!(pointed);
    result["install"] = install_json(&fresh, &aether(daemon), &target(daemon));
    Ok(result)
}

/// Declare the dial for the rules check, as the Session tab's *Dial is at* does: a dial is this
/// computer's setting, kept in the station's file, not in a profile.
fn declare_dial<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    hz: u64,
) -> Result<(), ApiError> {
    let mut candidate = daemon.config.clone();
    candidate
        .merge(&json!({ "regulatory.dial_hz": hz }))
        .map_err(|e| ApiError::new("refused", e.to_string(), false))?;
    candidate
        .save(&daemon.path)
        .map_err(|e| ApiError::new("cannot_save", e.to_string(), true))?;
    station.apply_live(&candidate);
    daemon.config = candidate;
    Ok(())
}

/// One profile asked for: an install's folder, a setup's name in it, the keying, and whether
/// the program's settings are to be set to reach Aether first.
struct Asked {
    folder: String,
    setup: String,
    keying: Option<Keying>,
    point: bool,
}

fn asked(params: &Value) -> Result<Vec<Asked>, ApiError> {
    let bad = |what: &str| ApiError::new("invalid_params", what.to_owned(), false);
    let Some(list) = params.get("imports").and_then(Value::as_array) else {
        return Err(bad("imports must be a list of {folder, setup, keying?}"));
    };
    list.iter()
        .map(|item| {
            let folder =
                text_param(item, "folder").ok_or_else(|| bad("an import needs its folder"))?;
            let setup =
                text_param(item, "setup").ok_or_else(|| bad("an import needs its setup"))?;
            let keying = match item.get("keying") {
                None | Some(Value::Null) => None,
                Some(k) => Some(
                    serde_json::from_value(k.clone())
                        .map_err(|_| bad("keying is \"program\" or \"aether\""))?,
                ),
            };
            Ok(Asked {
                folder: folder.to_owned(),
                setup: setup.to_owned(),
                keying,
                point: item.get("point").and_then(Value::as_bool) == Some(true),
            })
        })
        .collect()
}

/// Make the profile a plan describes, or bring the one of its name up to date: the plan's
/// settings over the profile's own when it is there (its sound card and levels as the operator
/// left them), over the station's when it is not.
fn write_profile(daemon: &DaemonState, plan: &Plan) -> Result<Value, ApiError> {
    let inventory = Inventory::from_json(&daemon.inventory());
    let existing = daemon.profiles.find(&plan.profile);
    let (mut config, memories, created_at) = if let Some(id) = &existing {
        let profile = daemon.profiles.read(id).map_err(|e| profiles::api(&e))?;
        let applied = profile
            .apply(&daemon.config, Some(&inventory))
            .map_err(|e| profiles::api(&e))?;
        let kept = daemon.memories.entries().to_vec();
        (
            applied.config,
            applied.memories.unwrap_or(kept),
            profile.created,
        )
    } else {
        let memories = daemon.memories.entries().to_vec();
        (daemon.config.clone(), memories, String::new())
    };
    let changed = config
        .merge(&plan.changes)
        .map_err(|e| ApiError::new("refused", e.to_string(), false))?;
    config
        .validate()
        .map_err(|e| ApiError::new("refused", e.to_string(), false))?;
    let now = crate::profile::now();
    let mut profile = Profile::capture(&plan.profile, &config, &memories, Some(&inventory), &now);
    if !created_at.is_empty() {
        profile.created = created_at;
    }
    let entry = daemon
        .profiles
        .write(&profile, existing.as_deref())
        .map_err(|e| profiles::api(&e))?;
    Ok(json!({
        "folder": plan.folder,
        "setup": plan.setup,
        "profile": plan.profile,
        "id": entry.id,
        "created": existing.is_none(),
        "changed": changed,
        "keying": plan.keying,
        "program_must": plan.program_must,
        "dial_hz": plan.dial_hz,
    }))
}

/// `hostapps.import`: a profile for each setup asked for; with `load` and one profile, the
/// station switched to it.
fn import<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let asked = asked(params)?;
    if asked.is_empty() {
        return Err(ApiError::new(
            "invalid_params",
            "Tick a setup to import.",
            false,
        ));
    }
    let aether = aether(daemon);
    let state = format!("{:?}", station.state());
    let mut imported = Vec::new();
    let mut refused = Vec::new();
    for a in &asked {
        let refuse = |why: &str| json!({ "folder": a.folder, "setup": a.setup, "why": why });
        let Some(mut install) = install_in(daemon, &a.folder) else {
            refused.push(refuse("not found: find the programs again"));
            continue;
        };
        let mut pointed = Value::Null;
        if a.point {
            // the program first, then the plan from what it says now
            match point_install(&state, daemon, &install) {
                Ok((done, fresh)) => {
                    pointed = json!(done);
                    install = fresh;
                }
                Err(why) => {
                    refused.push(refuse(&why));
                    continue;
                }
            }
        }
        let Some(setup) = install.setups.iter().find(|s| s.name == a.setup) else {
            refused.push(refuse("not found: find the programs again"));
            continue;
        };
        let plan = import::plan(&install, setup, a.keying, &aether);
        if let Some(why) = &plan.refused {
            refused.push(refuse(why));
            continue;
        }
        match write_profile(daemon, &plan) {
            Ok(mut done) => {
                done["pointed"] = pointed;
                imported.push(done);
            }
            Err(error) => refused.push(refuse(&error.message)),
        }
    }
    for done in &imported {
        let detail = format!(
            "profile \"{}\" {} from {} in {}",
            done["profile"].as_str().unwrap_or_default(),
            if done["created"] == true {
                "made"
            } else {
                "brought up to date"
            },
            done["setup"].as_str().unwrap_or_default(),
            done["folder"].as_str().unwrap_or_default(),
        );
        daemon.log.record(Level::Info, "hostapps", &detail, &state);
    }
    if !imported.is_empty() {
        daemon.note_profiles_changed();
    }
    let load = params.get("load").and_then(Value::as_bool) == Some(true);
    let mut result = if load && imported.len() == 1 {
        let id = imported[0]["id"].as_str().unwrap_or_default().to_owned();
        let profile = daemon.profiles.read(&id).map_err(|e| profiles::api(&e))?;
        let mut switched = profiles::commit(station, daemon, &profile, &id)?;
        if let Some(hz) = imported[0]["dial_hz"].as_u64() {
            declare_dial(station, daemon, hz)?;
            let detail = format!(
                "the dial declared at {} MHz for the rules check: the program's channel",
                import::mhz(hz)
            );
            daemon.log.record(Level::Info, "hostapps", &detail, &state);
            switched["dial_hz"] = json!(hz);
        }
        switched
    } else {
        profiles::status_json(daemon)
    };
    result["imported"] = json!(imported);
    result["refused"] = json!(refused);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::PttConfig,
        control::{methods::dispatch_with, protocol::Request},
        hostapps::{
            Radio, Tnc, TncKind,
            find::{FoundBy, Install},
            state::Finder,
        },
    };

    fn station() -> Station<crate::ptt::NullPtt> {
        Station::new(
            crate::station::StationConfig {
                callsign: "W4ODA".to_owned(),
                wait_for_clear: false,
                ..crate::station::StationConfig::default()
            },
            crate::ptt::NullPtt::default(),
            1,
        )
    }

    /// Two made-up installs: Winlink Express with two callsigns keyed by CAT, pointing at
    /// Aether, and a copy on another port with no radio.
    fn computer(_added: &[PathBuf]) -> crate::hostapps::find::Found {
        crate::hostapps::find::Found {
            installs: installs(),
            took_ms: vec![("reading", 1)],
        }
    }

    fn installs() -> Vec<Install> {
        let radio = Radio {
            rig: Some("Yaesu FTDX10".into()),
            cat_port: Some("COM3".into()),
            cat_baud: Some(38400),
            icom_address: None,
            ptt: crate::hostapps::Ptt::Cat,
        };
        let setup = |call: &str, hz: u32, port: u16, radio: Option<Radio>| ProgramSetup {
            name: call.into(),
            callsigns: vec![call.into()],
            session: Some("Vara HF P2P".into()),
            bandwidth_hz: Some(hz),
            frequency_hz: Some(7_083_500),
            channels_hz: Vec::new(),
            gateway: false,
            radio,
            tncs: vec![Tnc::tcp(TncKind::VaraHf, "127.0.0.1", port)],
        };
        let install = |folder: &str, setups: Vec<ProgramSetup>| Install {
            program: Program::WinlinkExpress,
            program_name: "Winlink Express",
            folder: folder.into(),
            exe: format!("{folder}\\RMS Express.exe"),
            found_by: vec![FoundBy::Usual],
            running: false,
            version: Some("1.8.6.0".into()),
            callsigns: setups.iter().map(|s| s.name.clone()).collect(),
            setups,
            notes: Vec::new(),
        };
        vec![
            install(
                "C:\\RMS Express",
                vec![
                    setup("N0CALL", 500, 8300, Some(radio.clone())),
                    setup("N1CALL", 2750, 8300, Some(radio)),
                ],
            ),
            install("D:\\WLE 2", vec![setup("N2CALL", 2300, 8370, None)]),
        ]
    }

    fn daemon(tag: &str) -> (DaemonState, PathBuf) {
        let (store, dir) = profiles::temp_store(&format!("hostapps-{tag}"));
        let mut config = crate::config::Config::parse(crate::config::EXAMPLE).expect("example");
        let settings = json!({
            "callsign": "KK4ODA",
            "audio.input": "USB Audio CODEC",
            "audio.output": "USB Audio CODEC",
            "ptt.kind": "serial",
            "ptt.port": "COM3",
        });
        config.merge(&settings).expect("merge");
        let path = dir.join("station.toml");
        config.save(&path).expect("save");
        let mut daemon = DaemonState::new(config, path, crate::log::Log::memory(50));
        daemon.heard = crate::heard::HeardList::open(None);
        daemon.memories = crate::memories::Memories::open(None);
        daemon.profiles = store;
        daemon.devices = || {
            json!({
                "devices": [{"name": "USB Audio CODEC", "input": true, "output": true}],
                "serial_ports": [{"name": "COM3", "description": "USB Serial Port"}],
            })
        };
        let mut finder = Finder::open(Some(dir.join(crate::hostapps::state::STATE_FILE)));
        finder.source = computer;
        finder.inline = true;
        daemon.hostapps = finder;
        (daemon, dir)
    }

    fn call(
        station: &mut Station<crate::ptt::NullPtt>,
        daemon: &mut DaemonState,
        method: &str,
        params: Value,
    ) -> Response {
        dispatch_with(
            station,
            Some(daemon),
            &Request {
                id: Some("1".into()),
                method: method.to_owned(),
                params,
                connection: None,
            },
        )
    }

    fn ok(response: Response) -> Value {
        assert!(response.ok, "{:?}", response.error);
        response.result.expect("result")
    }

    #[test]
    fn the_installs_are_found_with_a_plan_for_each_keying() {
        let mut station = station();
        let (mut daemon, dir) = daemon("find");
        let found = ok(call(&mut station, &mut daemon, "hostapps.find", json!({})));
        assert_eq!(found["found"], true);
        let installs = found["installs"].as_array().expect("installs");
        assert_eq!(installs.len(), 2);
        assert_eq!(installs[0]["points_at_aether"], true);
        assert_eq!(installs[1]["points_at_aether"], false);
        assert_eq!(installs[0]["callsigns"], json!(["N0CALL", "N1CALL"]));
        let setup = &installs[0]["setups"][0];
        assert_eq!(setup["tncs"][0]["pointing"], "aether");
        let plans = setup["plans"].as_array().expect("plans");
        assert_eq!(plans.len(), 2, "program and Aether keying");
        assert_eq!(plans[0]["keying"], "program");
        assert_eq!(plans[1]["changes"]["ptt.kind"], "cat");
        // the copy on another port: Aether follows it there, and it controls no radio
        let other = &installs[1]["setups"][0]["plans"][0];
        assert_eq!(other["changes"]["host.bind"], "127.0.0.1:8370");
        assert!(other["changes"].get("ptt.kind").is_none());
        // a folder holding no program is refused, and nothing is remembered
        let nowhere = json!({ "add": dir.display().to_string() });
        let refused = call(&mut station, &mut daemon, "hostapps.find", nowhere);
        assert_eq!(refused.error.expect("refused").code, "not_found");
        assert!(
            daemon.hostapps.remembered.folders.is_empty(),
            "{:?}",
            daemon.hostapps.remembered.folders
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn importing_two_setups_makes_two_profiles_and_one_can_be_switched_to() {
        let mut station = station();
        let (mut daemon, dir) = daemon("import");
        ok(call(&mut station, &mut daemon, "hostapps.find", json!({})));
        let two = json!({ "imports": [
            { "folder": "C:\\RMS Express", "setup": "N0CALL" },
            { "folder": "c:/rms express/", "setup": "N1CALL", "keying": "aether" },
            { "folder": "C:\\RMS Express", "setup": "N9CALL" },
        ]});
        let done = ok(call(&mut station, &mut daemon, "hostapps.import", two));
        let imported = done["imported"].as_array().expect("imported");
        assert_eq!(imported.len(), 2, "{done}");
        assert_eq!(imported[0]["profile"], "Winlink Express N0CALL");
        assert_eq!(imported[0]["created"], true);
        assert_eq!(done["refused"].as_array().map(Vec::len), Some(1));
        assert!(done["active"].is_null(), "not switched to unless asked");

        let id = daemon
            .profiles
            .find("Winlink Express N0CALL")
            .expect("made");
        let profile = daemon.profiles.read(&id).expect("read");
        let applied = profile.apply(&daemon.config, None).expect("apply");
        assert!(matches!(applied.config.ptt, PttConfig::Host { .. }));
        assert_eq!(applied.config.callsign, "N0CALL");
        assert_eq!(applied.config.radio.bandwidth, 500);
        assert!(applied.config.host.enabled);
        assert_eq!(
            applied.config.regulatory.profile, daemon.config.regulatory.profile,
            "the rules check kept with the program keying"
        );
        assert_eq!(imported[0]["dial_hz"], 7_082_000);
        let input = applied.config.audio.input.as_deref();
        assert_eq!(
            input,
            Some("USB Audio CODEC"),
            "the station's sound card kept"
        );

        let id = daemon
            .profiles
            .find("Winlink Express N1CALL")
            .expect("made");
        let profile = daemon.profiles.read(&id).expect("read");
        let applied = profile.apply(&daemon.config, None).expect("apply");
        assert!(
            matches!(applied.config.ptt, PttConfig::Cat { .. }),
            "{:?}",
            applied.config.ptt
        );
        assert_eq!(applied.config.radio.bandwidth, 2300, "2750 runs as 2300");

        // the same again, with load: brought up to date and switched to
        let one = json!({ "imports": [{ "folder": "C:\\RMS Express", "setup": "N0CALL" }], "load": true });
        let again = ok(call(&mut station, &mut daemon, "hostapps.import", one));
        assert_eq!(again["imported"][0]["created"], false);
        let id = daemon
            .profiles
            .find("Winlink Express N0CALL")
            .expect("made");
        assert_eq!(again["active"], json!(id));
        assert!(matches!(daemon.config.ptt, PttConfig::Host { .. }));
        assert_eq!(daemon.config.callsign, "N0CALL");
        assert_eq!(
            daemon.config.regulatory.dial_hz,
            Some(7_082_000),
            "the program's dial declared at the switch"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder of the system's temporary one that holds a Winlink Express to write to.
    fn wle_folder() -> PathBuf {
        std::env::temp_dir().join(format!("aether-hostapps-point-{}", std::process::id()))
    }

    /// This computer as a find sees it, with the one folder added: the files really read.
    fn real_computer(_added: &[PathBuf]) -> crate::hostapps::find::Found {
        let sources = crate::hostapps::find::Sources {
            added: vec![wle_folder()],
            ..crate::hostapps::find::Sources::default()
        };
        crate::hostapps::find::Found {
            installs: crate::hostapps::find::installs(&sources),
            took_ms: Vec::new(),
        }
    }

    fn nothing_runs() -> Vec<PathBuf> {
        Vec::new()
    }

    fn it_runs() -> Vec<PathBuf> {
        vec![wle_folder().join("RMS Express.exe")]
    }

    #[test]
    fn a_program_is_set_to_reach_aether_only_while_it_is_closed() {
        let folder = wle_folder();
        std::fs::create_dir_all(&folder).expect("dir");
        std::fs::write(folder.join("RMS Express.exe"), b"MZ").expect("exe");
        let ini = "[Properties]\r\nCallsign=N0CALL\r\nCall signs=N0CALL\r\n\
            [N0CALL]\r\nMode=Vara HF P2P\r\nVara P2P Channel Bandwidth=2300\r\n\
            Vara P2P Center Frequency=7103.500\r\n\
            [Vara]\r\nModel=Yaesu FTDX10\r\nControl Port=COM3\r\nPTT Port=Yaesu FTDX10\r\n\
            [Vara TNC]\r\nTCP Port=8300\r\nTNC Address=192.168.1.9\r\nAuto Launch Vara=True\r\n";
        let path = folder.join("RMS Express.ini");
        std::fs::write(&path, ini).expect("ini");

        let mut station = station();
        let (mut daemon, dir) = daemon("point");
        daemon.hostapps.source = real_computer;
        let found = ok(call(&mut station, &mut daemon, "hostapps.find", json!({})));
        let changes = &found["installs"][0]["program_changes"];
        assert_eq!(
            changes.as_array().map(Vec::len),
            Some(2),
            "address and launch: {changes}"
        );
        let name = folder.display().to_string();

        // running: refused, the file untouched
        daemon.hostapps.running = it_runs;
        let refused = call(
            &mut station,
            &mut daemon,
            "hostapps.point",
            json!({ "folder": name }),
        );
        let error = refused.error.expect("refused");
        assert_eq!(error.code, "refused");
        assert!(
            error.message.contains("close it first"),
            "{}",
            error.message
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), ini);

        // closed: set to reach Aether, then imported and switched to in one go
        daemon.hostapps.running = nothing_runs;
        let one = json!({
            "imports": [{ "folder": name, "setup": "N0CALL", "point": true }],
            "load": true,
        });
        let done = ok(call(&mut station, &mut daemon, "hostapps.import", one));
        assert_eq!(
            done["imported"][0]["pointed"]["changes"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        let after = crate::ini::Ini::new(std::fs::read(&path).expect("read"));
        assert_eq!(
            after.get("Vara TNC", "TNC Address").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(after.get("Vara TNC", "TCP Port").as_deref(), Some("8300"));
        assert_eq!(
            after.get("Vara TNC", "Auto Launch Vara").as_deref(),
            Some("False")
        );
        assert!(folder.join("RMS Express.ini.aether.bak").is_file());
        // planned from the program as it is now: it points at Aether, nothing more to change
        let fresh = ok(call(&mut station, &mut daemon, "hostapps.find", json!({})));
        let setup = &fresh["installs"][0]["setups"][0];
        assert_eq!(setup["tncs"][0]["pointing"], "aether");
        assert_eq!(fresh["installs"][0]["program_changes"], json!([]));
        assert_eq!(daemon.config.regulatory.dial_hz, Some(7_102_000));
        assert!(matches!(daemon.config.ptt, PttConfig::Host { .. }));

        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_added_folder_is_remembered_and_forgotten() {
        let mut station = station();
        let (mut daemon, dir) = daemon("added");
        let folder = dir.join("WLE");
        std::fs::create_dir_all(&folder).expect("dir");
        std::fs::write(folder.join("RMS Express.exe"), b"MZ").expect("exe");
        let add = json!({ "add": folder.display().to_string() });
        let found = ok(call(&mut station, &mut daemon, "hostapps.find", add));
        assert_eq!(found["added"].as_array().map(Vec::len), Some(1));
        let again = Finder::open(Some(dir.join(crate::hostapps::state::STATE_FILE)));
        assert_eq!(again.remembered.folders.len(), 1, "kept in hostapps.json");
        let forget = json!({ "forget": folder.display().to_string() });
        let found = ok(call(&mut station, &mut daemon, "hostapps.find", forget));
        assert_eq!(found["added"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
