//! The `hostapps.*` methods: Winlink's programs found on this computer, and a setup of one set
//! up for Aether or put back to VARA ([`crate::hostapps`], ADR-0133).
//!
//! `hostapps.find` answers with the last find and begins another off the run loop (a
//! `hostapps` event carries it when it finishes); it also adds or forgets a folder to look in.
//! `hostapps.setup` sets a setup up in one step (ADR-0133 §8): the program's settings written so
//! it reaches Aether — never while it runs, the file kept as it was beside it — the profile made
//! or brought up to date, and with `switch` the station switched to it, the program's dial
//! declared for the rules check. `hostapps.restore` goes back to VARA: the settings Aether wrote
//! are put back once no other setup of the install uses Aether, and the profile stays.
//! `hostapps.copy` copies a Winlink Express install first, off the loop, so the one that works
//! with VARA stays as it is.

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
        state::{self, Finder, SetUp},
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
        "hostapps.setup" => setup(station, daemon, params),
        "hostapps.restore" => restore(station, daemon, params),
        "hostapps.copy" => copy(station, daemon, params),
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

/// Aether as a program should reach it: the host port, the application for a program that
/// starts its TNC itself, and whether Aether keys the radio.
fn target(daemon: &DaemonState, keying: Option<Keying>) -> point::Target {
    point::Target {
        host_port: aether(daemon).host_port(),
        app: crate::varac::app_path().map(|p| p.display().to_string()),
        aether_keys: keying == Some(Keying::Aether),
    }
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

/// An install as the clients are given it: each setup with its plans (each with what setting
/// it up would write into the program) and whether it is set up, each TNC with where it
/// points, what going back to VARA would put back, and where a copy would go.
fn install_json(install: &Install, daemon: &DaemonState) -> Value {
    let aether = aether(daemon);
    let finder: &Finder = &daemon.hostapps;
    let files = point::files_of(install).unwrap_or_default();
    let mut value = json!(install);
    let mut points_at_aether = false;
    if let Some(setups) = value["setups"].as_array_mut() {
        for (json, setup) in setups.iter_mut().zip(&install.setups) {
            if let Some(tncs) = json["tncs"].as_array_mut() {
                for (t, tnc) in tncs.iter_mut().zip(&setup.tncs) {
                    let pointing = import::pointing(tnc, &aether);
                    points_at_aether |= pointing == import::Pointing::Aether;
                    t["pointing"] = json!(pointing);
                }
            }
            let plans: Vec<Value> = plans(install, setup, &aether)
                .iter()
                .map(|plan| {
                    let mut p = json!(plan);
                    let wanted = target(daemon, plan.keying);
                    p["program_changes"] = json!(point::changes(install.program, &files, &wanted));
                    p
                })
                .collect();
            json["plans"] = json!(plans);
            json["set_up"] = json!(finder.set_up_of(&install.folder, &setup.name));
        }
    }
    value["points_at_aether"] = json!(points_at_aether);
    let (put_back, left) = point::going_back(&files, &finder.originals(&install.folder));
    value["going_back"] = json!({ "put_back": put_back, "left": left });
    value["copy_to"] = if install.program == Program::WinlinkExpress {
        json!(state::copy_destination(Path::new(&install.folder)).map(|p| p.display().to_string()))
    } else {
        Value::Null
    };
    value
}

/// What `hostapps.find` answers and the `hostapps` event carries.
#[must_use]
pub fn answer(daemon: &DaemonState) -> Value {
    let (last, finding) = daemon.hostapps.last();
    let aether = aether(daemon);
    let installs: Vec<Value> = last
        .iter()
        .flatten()
        .map(|i| install_json(i, daemon))
        .collect();
    json!({
        "supported": cfg!(windows),
        "finding": finding,
        "found": last.is_some(),
        "installs": installs,
        "added": daemon.hostapps.remembered.folders,
        "copy": daemon.hostapps.last_copy(),
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

fn bad(what: &str) -> ApiError {
    ApiError::new("invalid_params", what.to_owned(), false)
}

fn refused(why: impl Into<String>) -> ApiError {
    ApiError::new("refused", why.into(), false)
}

fn not_found(why: impl Into<String>) -> ApiError {
    ApiError::new("not_found", why.into(), false)
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
            return Err(not_found(format!(
                "{} holds none of {}.",
                folder.display(),
                names.join(", ")
            )));
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

/// The install and setup a request names, from the last find.
fn named(daemon: &DaemonState, params: &Value) -> Result<(Install, String), ApiError> {
    let folder =
        text_param(params, "folder").ok_or_else(|| bad("the install's folder is needed"))?;
    let setup = text_param(params, "setup").ok_or_else(|| bad("the setup's name is needed"))?;
    let install = install_in(daemon, folder).ok_or_else(|| {
        not_found(format!(
            "No program found in {folder}: find the programs again."
        ))
    })?;
    if !install.setups.iter().any(|s| s.name == setup) {
        return Err(not_found(format!(
            "{} in {folder} has no {setup}: find the programs again.",
            install.program_name
        )));
    }
    Ok((install, setup.to_owned()))
}

/// Whether an install's program runs now: the system asked again, since a find may be minutes
/// old. Also what runs, for reading the install again.
fn runs(daemon: &DaemonState, install: &Install) -> (bool, Vec<PathBuf>) {
    let running = (daemon.hostapps.running)();
    let runs = running
        .iter()
        .any(|r| same_path(r, Path::new(&install.exe)));
    (runs, running)
}

/// Write into an install's settings what its program needs to reach Aether, if anything; then
/// read it again and put it in the last find's place, so that the clients see it as it is now.
fn point_install(
    station_state: &str,
    daemon: &mut DaemonState,
    install: &Install,
    target: &point::Target,
) -> Result<point::Pointed, String> {
    let (runs, running) = runs(daemon, install);
    let pointed = point::point(install, target, runs)?;
    if !pointed.written.is_empty() {
        daemon.hostapps.replace(finding::reread(install, &running));
    }
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
    Ok(pointed)
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
        .map_err(|e| refused(e.to_string()))?;
    candidate
        .save(&daemon.path)
        .map_err(|e| ApiError::new("cannot_save", e.to_string(), true))?;
    station.apply_live(&candidate);
    daemon.config = candidate;
    Ok(())
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
        .map_err(|e| refused(e.to_string()))?;
    config.validate().map_err(|e| refused(e.to_string()))?;
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
        "dial_hz": plan.dial_hz,
    }))
}

/// `hostapps.setup`: one setup set up for Aether — the program's settings written, the profile
/// made or brought up to date, both remembered; with `switch`, the station switched to it.
fn setup<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let (install, name) = named(daemon, params)?;
    let keying: Option<Keying> = match params.get("keying") {
        None | Some(Value::Null) => None,
        Some(k) => Some(
            serde_json::from_value(k.clone())
                .map_err(|_| bad("keying is \"program\" or \"aether\""))?,
        ),
    };
    let switch = params.get("switch").and_then(Value::as_bool) == Some(true);
    let Some(found) = install.setups.iter().find(|s| s.name == name) else {
        return Err(not_found("find the programs again"));
    };
    // planned from the program as it is before it is written: Aether keys the radio the way the
    // program did, which the program no longer says once its keying is handed over
    let plan = import::plan(&install, found, keying, &aether(daemon));
    if let Some(why) = &plan.refused {
        return Err(refused(format!("{}: {why}", install.program_name)));
    }
    let state = format!("{:?}", station.state());
    let wanted = target(daemon, plan.keying);
    let pointed = point_install(&state, daemon, &install, &wanted).map_err(refused)?;
    let mut done = write_profile(daemon, &plan)?;
    let record = SetUp {
        folder: install.folder.clone(),
        setup: name.clone(),
        profile: plan.profile.clone(),
        keying: plan.keying,
        at: crate::profile::now(),
    };
    daemon
        .hostapps
        .note_set_up(record, &pointed.changes)
        .map_err(|e| ApiError::new("cannot_save", e, true))?;
    let detail = format!(
        "{} {} in {} set up for Aether: profile \"{}\" {}",
        install.program_name,
        name,
        install.folder,
        plan.profile,
        if done["created"] == true {
            "made"
        } else {
            "brought up to date"
        },
    );
    daemon.log.record(Level::Info, "hostapps", &detail, &state);
    daemon.note_profiles_changed();
    // what is left for the operator: the advice Aether cannot carry out itself
    let by_hand: Vec<&import::Advice> = plan.program_must.iter().filter(|a| !a.settable).collect();
    done["pointed"] = json!(pointed);
    done["by_hand"] = json!(by_hand);
    let mut result = if switch {
        let id = done["id"].as_str().unwrap_or_default().to_owned();
        let profile = daemon.profiles.read(&id).map_err(|e| profiles::api(&e))?;
        let mut switched = profiles::commit(station, daemon, &profile, &id)?;
        if let Some(hz) = plan.dial_hz {
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
    done["switched"] = json!(switch);
    result["set_up"] = done;
    result["hostapps"] = answer(daemon);
    Ok(result)
}

/// `hostapps.restore`: go back to VARA. The setup is no longer set up; when no other setup of
/// its install is, the settings Aether wrote into the program are put back (those changed
/// since are left), never while the program runs. The profile stays.
fn restore<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let folder =
        text_param(params, "folder").ok_or_else(|| bad("the install's folder is needed"))?;
    let setup = text_param(params, "setup").ok_or_else(|| bad("the setup's name is needed"))?;
    let Some(record) = daemon.hostapps.set_up_of(folder, setup).cloned() else {
        return Err(not_found(format!(
            "{setup} in {folder} was not set up for Aether here."
        )));
    };
    let state = format!("{:?}", station.state());
    let others = daemon.hostapps.others_set_up(&record.folder, &record.setup);
    let mut result = json!({
        "folder": record.folder,
        "setup": record.setup,
        "profile": record.profile,
        "others": others,
        "put_back": [],
        "left": [],
        "written": [],
    });
    let detail = if others.is_empty() {
        let originals = daemon.hostapps.originals(&record.folder);
        if let Some(install) = install_in(daemon, &record.folder) {
            let (runs, running) = runs(daemon, &install);
            let back = point::put_back(&install, &originals, runs).map_err(refused)?;
            if !back.written.is_empty() {
                daemon.hostapps.replace(finding::reread(&install, &running));
            }
            result["put_back"] = json!(back.put_back);
            result["left"] = json!(back.left);
            result["written"] = json!(back.written);
            let keys: Vec<String> = back
                .put_back
                .iter()
                .map(|c| format!("{} {}", c.key, c.to))
                .collect();
            if keys.is_empty() {
                format!(
                    "{} {} in {} back to VARA: nothing to put back",
                    install.program_name, record.setup, record.folder
                )
            } else {
                format!(
                    "{} {} in {} back to VARA: {} put back",
                    install.program_name,
                    record.setup,
                    record.folder,
                    keys.join(", ")
                )
            }
        } else if Path::new(&record.folder).exists() {
            return Err(not_found(format!(
                "No program found in {}: find the programs again.",
                record.folder
            )));
        } else {
            format!("{} is gone: {} forgotten", record.folder, record.setup)
        }
    } else {
        format!(
            "{} in {} no longer set up for Aether; {} still use it, so the program's settings \
             stay",
            record.setup,
            record.folder,
            others.join(", ")
        )
    };
    daemon
        .hostapps
        .forget_set_up(&record.folder, &record.setup, others.is_empty())
        .map_err(|e| ApiError::new("cannot_save", e, true))?;
    daemon.log.record(Level::Info, "hostapps", &detail, &state);
    result["hostapps"] = answer(daemon);
    Ok(result)
}

/// `hostapps.copy`: copy a Winlink Express install to a folder beside it, off the run loop, and
/// find it; the copy is then set up as any install is. Never while it runs, never a gateway (a
/// second copy of one would publish its channels again).
fn copy<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let folder =
        text_param(params, "folder").ok_or_else(|| bad("the install's folder is needed"))?;
    let install = install_in(daemon, folder).ok_or_else(|| {
        not_found(format!(
            "No program found in {folder}: find the programs again."
        ))
    })?;
    if install.program != Program::WinlinkExpress {
        return Err(refused(format!(
            "{} is not copied: only Winlink Express is. A second copy of a gateway would publish \
             its channels to Winlink again.",
            install.program_name
        )));
    }
    let (running, _) = runs(daemon, &install);
    if running {
        return Err(refused(format!(
            "{} is running: close it first, so its settings and mail are copied as they stand.",
            install.program_name
        )));
    }
    let to = state::copy_destination(Path::new(&install.folder)).ok_or_else(|| {
        refused(format!(
            "No free name beside {} for a copy.",
            install.folder
        ))
    })?;
    daemon
        .hostapps
        .copy(Path::new(&install.folder), &to)
        .map_err(refused)?;
    let detail = format!("copying {} to {}", install.folder, to.display());
    let state = format!("{:?}", station.state());
    daemon.log.record(Level::Info, "hostapps", &detail, &state);
    let mut result = answer(daemon);
    result["to"] = json!(to.display().to_string());
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
        assert_eq!(plans[0]["program_changes"], json!([]), "no file to write");
        assert!(setup["set_up"].is_null(), "not set up yet");
        assert!(
            installs[0]["copy_to"]
                .as_str()
                .is_some_and(|to| to.ends_with("RMS Express Aether")),
            "{}",
            installs[0]["copy_to"]
        );
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
    fn setting_two_setups_up_makes_two_profiles_and_one_can_be_switched_to() {
        let mut station = station();
        let (mut daemon, dir) = daemon("import");
        daemon.hostapps.running = nothing_runs;
        ok(call(&mut station, &mut daemon, "hostapps.find", json!({})));
        let first = json!({ "folder": "C:\\RMS Express", "setup": "N0CALL" });
        let done = ok(call(&mut station, &mut daemon, "hostapps.setup", first));
        assert_eq!(done["set_up"]["profile"], "Winlink Express N0CALL");
        assert_eq!(done["set_up"]["created"], true);
        assert!(done["active"].is_null(), "not switched to unless asked");
        let row = &done["hostapps"]["installs"][0]["setups"][0];
        assert_eq!(row["set_up"]["profile"], "Winlink Express N0CALL", "{row}");
        let second = json!({ "folder": "c:/rms express/", "setup": "N1CALL", "keying": "aether" });
        ok(call(&mut station, &mut daemon, "hostapps.setup", second));
        let unknown = json!({ "folder": "C:\\RMS Express", "setup": "N9CALL" });
        let refused = call(&mut station, &mut daemon, "hostapps.setup", unknown);
        assert_eq!(refused.error.expect("refused").code, "not_found");

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

        // the same again, switched to: brought up to date, and the program's dial declared
        let again = json!({ "folder": "C:\\RMS Express", "setup": "N0CALL", "switch": true });
        let again = ok(call(&mut station, &mut daemon, "hostapps.setup", again));
        assert_eq!(again["set_up"]["created"], false);
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
        // going back while another setup of the install still uses Aether: only forgotten
        let back = json!({ "folder": "C:\\RMS Express", "setup": "N0CALL" });
        let back = ok(call(&mut station, &mut daemon, "hostapps.restore", back));
        assert_eq!(back["others"], json!(["N1CALL"]));
        assert!(
            daemon
                .hostapps
                .set_up_of("C:\\RMS Express", "N0CALL")
                .is_none()
        );
        assert!(
            daemon.profiles.find("Winlink Express N0CALL").is_some(),
            "the profile stays"
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

    /// This computer as a find sees it with only the folders added: the files really read.
    fn added_only(added: &[PathBuf]) -> crate::hostapps::find::Found {
        let sources = crate::hostapps::find::Sources {
            added: added.to_vec(),
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
    fn a_program_is_set_up_and_put_back_only_while_it_is_closed() {
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
        let plans = &found["installs"][0]["setups"][0]["plans"];
        assert_eq!(
            plans[0]["program_changes"].as_array().map(Vec::len),
            Some(2),
            "address and launch (the port is 8300 already): {}",
            plans[0]["program_changes"]
        );
        assert_eq!(
            plans[1]["program_changes"].as_array().map(Vec::len),
            Some(5),
            "and the radio's keying and control with Aether keying: {}",
            plans[1]["program_changes"]
        );
        let name = folder.display().to_string();
        let asked =
            json!({ "folder": name, "setup": "N0CALL", "keying": "aether", "switch": true });

        // running: refused, the file untouched, nothing remembered
        daemon.hostapps.running = it_runs;
        let refused = call(&mut station, &mut daemon, "hostapps.setup", asked.clone());
        let error = refused.error.expect("refused");
        assert_eq!(error.code, "refused");
        assert!(
            error.message.contains("close it first"),
            "{}",
            error.message
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), ini);
        assert!(daemon.hostapps.set_up_of(&name, "N0CALL").is_none());

        // closed: set up and switched to in one go, keyed by Aether as the program did
        daemon.hostapps.running = nothing_runs;
        let done = ok(call(&mut station, &mut daemon, "hostapps.setup", asked));
        assert_eq!(
            done["set_up"]["pointed"]["changes"]
                .as_array()
                .map(Vec::len),
            Some(5)
        );
        assert_eq!(done["set_up"]["by_hand"], json!([]), "nothing left by hand");
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
        assert_eq!(after.get("Vara", "Model").as_deref(), Some("Manual"));
        assert_eq!(after.get("Vara", "Control Port").as_deref(), Some("None"));
        assert_eq!(after.get("Vara", "PTT Port").as_deref(), Some("None"));
        assert!(folder.join("RMS Express.ini.aether.bak").is_file());
        assert!(
            matches!(daemon.config.ptt, PttConfig::Cat { .. }),
            "{:?}",
            daemon.config.ptt
        );
        let row = &done["hostapps"]["installs"][0];
        assert_eq!(row["setups"][0]["set_up"]["keying"], "aether");
        assert_eq!(row["setups"][0]["tncs"][0]["pointing"], "aether");
        assert_eq!(
            row["going_back"]["put_back"].as_array().map(Vec::len),
            Some(5),
            "{}",
            row["going_back"]
        );

        go_back_leaving_what_the_operator_changed(&mut station, &mut daemon, &name, &path, &folder);

        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Going back, after the operator changed one of the settings Aether wrote: that one is
    /// left, the rest put back, the file kept, the profile kept.
    fn go_back_leaving_what_the_operator_changed(
        station: &mut Station<crate::ptt::NullPtt>,
        daemon: &mut DaemonState,
        name: &str,
        path: &Path,
        folder: &Path,
    ) {
        // the operator changes one of them in the program: going back leaves it
        let edited = std::fs::read_to_string(path)
            .expect("read")
            .replace("Control Port=None", "Control Port=COM7");
        std::fs::write(path, edited).expect("edit");
        let back = json!({ "folder": name, "setup": "N0CALL" });
        daemon.hostapps.running = it_runs;
        let refused = call(station, daemon, "hostapps.restore", back.clone());
        assert_eq!(refused.error.expect("refused").code, "refused");
        daemon.hostapps.running = nothing_runs;
        let back = ok(call(station, daemon, "hostapps.restore", back));
        assert_eq!(back["put_back"].as_array().map(Vec::len), Some(4), "{back}");
        assert_eq!(back["left"][0]["key"], "Control Port");
        let after = crate::ini::Ini::new(std::fs::read(path).expect("read"));
        assert_eq!(
            after.get("Vara TNC", "TNC Address").as_deref(),
            Some("192.168.1.9")
        );
        assert_eq!(
            after.get("Vara TNC", "Auto Launch Vara").as_deref(),
            Some("True")
        );
        assert_eq!(after.get("Vara", "Model").as_deref(), Some("Yaesu FTDX10"));
        assert_eq!(
            after.get("Vara", "PTT Port").as_deref(),
            Some("Yaesu FTDX10")
        );
        assert_eq!(after.get("Vara", "Control Port").as_deref(), Some("COM7"));
        assert!(folder.join("RMS Express.ini.aether-undo.bak").is_file());
        assert!(daemon.hostapps.originals(name).is_empty());
        assert!(
            daemon.profiles.find("Winlink Express N0CALL").is_some(),
            "the profile stays"
        );
    }

    #[test]
    fn a_winlink_express_is_copied_beside_itself_and_found() {
        let folder =
            wle_folder().with_file_name(format!("aether-hostapps-copy-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("N0CALL")).expect("dir");
        std::fs::write(folder.join("RMS Express.exe"), b"MZ").expect("exe");
        std::fs::write(
            folder.join("RMS Express.ini"),
            "[Properties]\nCallsign=N0CALL\n[Vara TNC]\nTCP Port=8300\n",
        )
        .expect("ini");
        std::fs::write(folder.join("N0CALL").join("mail.mime"), b"mail").expect("mail");
        let mut station = station();
        let (mut daemon, dir) = daemon("copy");
        daemon.hostapps.source = added_only;
        daemon.hostapps.running = nothing_runs;
        let name = folder.display().to_string();
        ok(call(
            &mut station,
            &mut daemon,
            "hostapps.find",
            json!({ "add": name }),
        ));
        let copied = ok(call(
            &mut station,
            &mut daemon,
            "hostapps.copy",
            json!({ "folder": name }),
        ));
        let to = copied["to"].as_str().expect("to").to_owned();
        assert!(to.ends_with(" Aether"), "{to}");
        assert_eq!(copied["copy"]["state"], "done", "{}", copied["copy"]);
        assert!(Path::new(&to).join("N0CALL").join("mail.mime").is_file());
        assert!(daemon.hostapps.take_news());
        assert!(
            daemon
                .hostapps
                .remembered
                .folders
                .iter()
                .any(|f| same_path(Path::new(f), Path::new(&to))),
            "remembered, so every find finds it"
        );
        let found = answer(&daemon);
        let folders: Vec<&str> = found["installs"]
            .as_array()
            .expect("installs")
            .iter()
            .filter_map(|i| i["folder"].as_str())
            .collect();
        assert!(
            folders
                .iter()
                .any(|f| same_path(Path::new(f), Path::new(&to))),
            "{folders:?}"
        );
        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&to);
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
