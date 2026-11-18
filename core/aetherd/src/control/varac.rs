//! The `varac.*` methods: `VarAC` set up to run on Aether in one step, and kept so after an
//! update ([`crate::varac`]).
//!
//! `varac.status` says where `VarAC` is, whether its Aether profile is there and up to date,
//! and what to offer the operator; `varac.setup` makes or brings up to date the `VarAC`
//! profile and the Aether profile; `varac.dismiss` stops the offer until there is another.

use std::path::Path;

use serde_json::{Value, json};

use crate::{
    config::PttConfig,
    control::{
        methods::DaemonState,
        profiles,
        protocol::{ApiError, Response},
    },
    log::Level,
    profile::{Entry, Inventory, Profile},
    ptt::Ptt,
    station::Station,
    varac::{self, Ini, Keying},
};

/// The methods this module answers; `varac.setup` and `varac.dismiss` change something, and
/// [`crate::control::methods::is_mutating`] names them.
#[must_use]
pub fn handles(method: &str) -> bool {
    method.starts_with("varac.")
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
                "This modem keeps no files to set VarAC up with.",
                false,
            ),
        );
    };
    let result = match method {
        "varac.status" => Ok(status(daemon, text_param(params, "folder"))),
        "varac.setup" => setup(station, daemon, params),
        "varac.dismiss" => dismiss(daemon),
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

fn text_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Aether's two ports, as `VarAC` must name them: the host programs' and the KISS port's.
fn ports(daemon: &DaemonState) -> (u16, u16) {
    let host = varac::port_of(&daemon.config.host.bind, 8300);
    let kiss = varac::port_of(&daemon.config.kiss.bind, 8100);
    (host, kiss)
}

/// The `VarAC` profile this writes, as it stands in `VarAC`'s folder; `None` when it is not
/// there.
fn current_profile(folder: &Path) -> Option<Ini> {
    let bytes = std::fs::read(folder.join(varac::PROFILE_FILE)).ok()?;
    Some(Ini::new(bytes))
}

/// What to offer the operator, by a key that names it — `setup`, `update:<what changes>` or
/// `template:<n>` — or nothing. The panel shows it unless that key was dismissed, so a later
/// release's change is offered again.
fn offer_key(
    setup: &varac::Setup,
    folder: Option<&Path>,
    current: Option<&Ini>,
    differences: &[varac::Difference],
    aether_there: bool,
) -> Option<String> {
    if folder.is_none() || setup.app.is_none() {
        return None;
    }
    if current.is_none() || !aether_there {
        return Some("setup".to_owned());
    }
    if !differences.is_empty() {
        let what: Vec<String> = differences
            .iter()
            .map(|d| format!("{}={}", d.key, d.wanted))
            .collect();
        return Some(format!("update:{}", what.join(";")));
    }
    (setup.remembered.template < varac::TEMPLATE).then(|| format!("template:{}", varac::TEMPLATE))
}

/// What `varac.status` answers, and what the other methods answer with.
fn status(daemon: &DaemonState, asked: Option<&str>) -> Value {
    let setup = &daemon.varac;
    let folder = setup.folder(asked);
    let (host_port, kiss_port) = ports(daemon);
    let app = setup.app.as_deref();
    let wanted = app.map(|app| varac::wanted(app, host_port, kiss_port));
    let current = folder.as_deref().and_then(current_profile);
    let differences = if let (Some(ini), Some(wanted)) = (&current, &wanted) {
        varac::differences(ini, wanted)
    } else {
        Vec::new()
    };
    // who keys the radio in the profile `VarAC` will run: this one, or the one it is made from
    let source = folder
        .as_deref()
        .and_then(|dir| varac::read_profile(dir, varac::MAIN_FILE).ok());
    let keying = current
        .as_ref()
        .or(source.as_ref())
        .map_or(Keying::Unknown, varac::keying);
    let aether_id = daemon.profiles.find(varac::AETHER_PROFILE);
    let active = daemon.profiles.active();
    let is_active = aether_id.as_deref().is_some_and(|id| active == Some(id));
    let offer = offer_key(
        setup,
        folder.as_deref(),
        current.as_ref(),
        &differences,
        aether_id.is_some(),
    );
    let show = offer.is_some() && offer != setup.remembered.dismissed;
    json!({
        "supported": cfg!(windows),
        "app": app.map(|p| p.display().to_string()),
        "asked": asked,
        "folder": folder.as_ref().map(|d| d.display().to_string()),
        "sources": folder.as_deref().map(varac::profiles_in).unwrap_or_default(),
        "keying": keying,
        "profile": {
            "file": varac::PROFILE_FILE,
            "exists": current.is_some(),
            "differences": differences,
        },
        "aether_profile": {
            "name": varac::AETHER_PROFILE,
            "id": aether_id,
            "active": is_active,
            "template": setup.remembered.template,
            "current_template": varac::TEMPLATE,
        },
        "offer": offer.as_deref().map(|key| key.split(':').next().unwrap_or(key)),
        "offer_key": offer,
        "show_offer": show,
    })
}

/// Why `VarAC`'s folder was not found.
fn not_found(asked: Option<&str>) -> ApiError {
    let message = match asked {
        Some(folder) => format!("{folder} is not VarAC's folder: it has no VarAC.ini."),
        None => "VarAC was not found: name the folder its VarAC.ini is in.".to_owned(),
    };
    ApiError::new("not_found", message, false)
}

/// What making the Aether profile did.
struct Made {
    entry: Entry,
    created: bool,
    changed: Vec<String>,
}

/// Make the Aether profile for `VarAC`, or bring it up to date: [`varac::aether_changes`] over
/// the profile's own settings when it is there — so its sound card and levels stay as the
/// operator left them — and over the station's when it is not.
fn aether_profile(daemon: &DaemonState, keying: Keying) -> Result<Made, ApiError> {
    let inventory = Inventory::from_json(&daemon.inventory());
    let existing = daemon.profiles.find(varac::AETHER_PROFILE);
    let (mut config, memories, created_at) = if let Some(id) = &existing {
        let profile = daemon.profiles.read(id).map_err(|e| profiles::api(&e))?;
        let applied = profile
            .apply(&daemon.config, Some(&inventory))
            .map_err(|e| profiles::api(&e))?;
        let kept = daemon.memories.entries().to_vec();
        let memories = applied.memories.unwrap_or(kept);
        (applied.config, memories, profile.created)
    } else {
        let memories = daemon.memories.entries().to_vec();
        (daemon.config.clone(), memories, String::new())
    };
    let host_keyed = matches!(config.ptt, PttConfig::Host { .. });
    let changes = varac::aether_changes(keying, host_keyed);
    let changed = config
        .merge(&changes)
        .map_err(|e| ApiError::new("refused", e.to_string(), false))?;
    let now = crate::profile::now();
    let name = varac::AETHER_PROFILE;
    let mut profile = Profile::capture(name, &config, &memories, Some(&inventory), &now);
    if !created_at.is_empty() {
        profile.created = created_at;
    }
    let entry = daemon
        .profiles
        .write(&profile, existing.as_deref())
        .map_err(|e| profiles::api(&e))?;
    Ok(Made {
        entry,
        created: existing.is_none(),
        changed,
    })
}

/// `varac.setup`: the `VarAC` profile made from one of the operator's (`source`, `VarAC.ini`
/// unless named) or brought up to date, the Aether profile made or brought up to date, both
/// remembered, and — with `load` — the station switched to the Aether profile.
fn setup<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let asked = text_param(params, "folder");
    let Some(folder) = daemon.varac.folder(asked) else {
        return Err(not_found(asked));
    };
    let Some(app) = daemon.varac.app.clone() else {
        return Err(ApiError::new(
            "unsupported",
            "Aether's application is not beside the modem, so VarAC cannot be told \
             where to start it.",
            false,
        ));
    };
    let source = text_param(params, "source").unwrap_or(varac::MAIN_FILE);
    let (host_port, kiss_port) = ports(daemon);
    let wanted = varac::wanted(&app, host_port, kiss_port);
    let written = varac::write_profile(&folder, source, &wanted)
        .map_err(|e| ApiError::new("cannot_save", e, true))?;
    let keying = current_profile(&folder).map_or(Keying::Unknown, |ini| varac::keying(&ini));
    let made = aether_profile(daemon, keying)?;
    let state = format!("{:?}", station.state());
    daemon.varac.remembered.folder = Some(folder.display().to_string());
    daemon.varac.remembered.template = varac::TEMPLATE;
    daemon.varac.remembered.dismissed = None;
    if let Err(error) = daemon.varac.save() {
        let detail = format!("what was set up is not remembered: {error}");
        daemon.log.record(Level::Warn, "varac", &detail, &state);
    }
    let what = if written.created {
        format!("made {} from {source}", varac::PROFILE_FILE)
    } else if written.changed.is_empty() {
        format!("{} was up to date", varac::PROFILE_FILE)
    } else {
        format!(
            "{} brought up to date ({} settings)",
            varac::PROFILE_FILE,
            written.changed.len()
        )
    };
    let profile_said = if made.created {
        "made"
    } else {
        "brought up to date"
    };
    let detail = format!(
        "{what} in {}; Aether profile \"{}\" {profile_said}",
        folder.display(),
        varac::AETHER_PROFILE
    );
    daemon.log.record(Level::Info, "varac", &detail, &state);
    daemon.note_profiles_changed();
    let load = params.get("load").and_then(Value::as_bool) == Some(true);
    let mut result = if load {
        let store = &daemon.profiles;
        let profile = store.read(&made.entry.id).map_err(|e| profiles::api(&e))?;
        profiles::commit(station, daemon, &profile, &made.entry.id)?
    } else {
        profiles::status_json(daemon)
    };
    result["varac"] = status(daemon, None);
    result["written"] = json!(written);
    result["aether_profile"] = json!({
        "entry": made.entry,
        "created": made.created,
        "changed": made.changed,
    });
    result["keying"] = json!(keying);
    Ok(result)
}

/// `varac.dismiss`: the offer made now is not made again; another one is.
fn dismiss(daemon: &mut DaemonState) -> Result<Value, ApiError> {
    let now = status(daemon, None);
    let offer = now["offer_key"].as_str().map(str::to_owned);
    daemon.varac.remembered.dismissed = offer;
    daemon
        .varac
        .save()
        .map_err(|e| ApiError::new("cannot_save", e, true))?;
    Ok(status(daemon, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{methods::dispatch_with, protocol::Request};

    /// A `VarAC` profile with made-up values, keyed by CAT.
    const PROFILE: &str = "[RIG_CONTROL]\nRigPTTControlType=CAT\n\n[VARAHF_CONFIG]\n\
        VaraModemType=VaraHF\nVarahfMainPath=C:\\VARA\\VARA.exe\nVarahfMainPort=8300\n\
        VarahfMainHost=127.0.0.1\nVarahfEnableKissInterface=OFF\nVarahfMainKissPort=8100\n\
        VarahfMonitorPath=\nVarahfLaunchOnModemConnect=OFF\n\n[MY_INFO]\nMyCall=N0CALL\n";

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

    /// A daemon whose configuration, profiles and `VarAC` folder live in a temporary
    /// directory, with the application at a made-up path.
    fn daemon(tag: &str) -> (DaemonState, std::path::PathBuf) {
        let (store, dir) = profiles::temp_store(&format!("varac-{tag}"));
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
        let folder = dir.join("VarAC");
        std::fs::create_dir_all(&folder).expect("varac dir");
        std::fs::write(folder.join(varac::MAIN_FILE), PROFILE).expect("profile");
        daemon.varac = varac::Setup::open(Some(dir.join(varac::STATE_FILE)));
        daemon.varac.usual = vec![folder];
        daemon.varac.app = Some(dir.join("Aether HF").join("aether-hf.exe"));
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
    fn varac_is_found_and_setting_it_up_is_offered() {
        let mut station = station();
        let (mut daemon, dir) = daemon("offer");
        let status = ok(call(&mut station, &mut daemon, "varac.status", json!({})));
        assert!(status["folder"].as_str().is_some(), "{status}");
        assert_eq!(status["sources"], json!(["VarAC.ini"]));
        assert_eq!(status["keying"], "program");
        assert_eq!(status["profile"]["exists"], false);
        assert_eq!(status["offer"], "setup");
        assert_eq!(status["show_offer"], true);
        // dismissed, the same offer is not shown again
        let dismissed = ok(call(&mut station, &mut daemon, "varac.dismiss", json!({})));
        assert_eq!(dismissed["show_offer"], false);
        let elsewhere = json!({ "folder": dir.display().to_string() });
        let wrong = ok(call(&mut station, &mut daemon, "varac.status", elsewhere));
        assert!(wrong["folder"].is_null(), "{wrong}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_up_writes_both_profiles_and_an_update_is_offered_when_aether_moves() {
        let mut station = station();
        let (mut daemon, dir) = daemon("setup");
        let done = ok(call(&mut station, &mut daemon, "varac.setup", json!({})));
        assert_eq!(done["written"]["created"], true);
        assert_eq!(done["aether_profile"]["created"], true);
        assert_eq!(done["varac"]["show_offer"], false, "{done}");
        assert!(done["active"].is_null(), "not switched to unless asked");
        let ini = current_profile(&dir.join("VarAC")).expect("written");
        let app = daemon.varac.app.clone().expect("app");
        let path = app.display().to_string();
        assert_eq!(ini.get(varac::SECTION, "VarahfMainPath"), Some(path));
        assert_eq!(ini.get("MY_INFO", "MyCall").as_deref(), Some("N0CALL"));

        // the Aether profile keys by the host program and keeps the station's sound card
        let id = daemon.profiles.find(varac::AETHER_PROFILE).expect("made");
        let profile = daemon.profiles.read(&id).expect("read");
        let applied = profile.apply(&daemon.config, None).expect("apply");
        assert!(matches!(applied.config.ptt, PttConfig::Host { .. }));
        assert_eq!(applied.config.regulatory.profile, "none");
        assert!(applied.config.host.enabled);
        assert!(applied.config.kiss.enabled);
        let input = applied.config.audio.input.as_deref();
        assert_eq!(input, Some("USB Audio CODEC"));

        // Aether moved: the profile is out of date, and the update is offered
        daemon.varac.app = Some(dir.join("Elsewhere").join("aether-hf.exe"));
        let stale = ok(call(&mut station, &mut daemon, "varac.status", json!({})));
        assert_eq!(stale["offer"], "update");
        assert_eq!(stale["show_offer"], true);
        let changed = &stale["profile"]["differences"];
        assert_eq!(changed[0]["key"], "VarahfMainPath", "{changed}");
        let again = ok(call(&mut station, &mut daemon, "varac.setup", json!({})));
        let written = &again["written"];
        assert_eq!(written["changed"], json!(["VarahfMainPath"]), "{written}");
        assert!(written["backup"].as_str().is_some(), "{written}");
        assert_eq!(again["aether_profile"]["created"], false);
        assert_eq!(again["varac"]["offer"], Value::Null);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setting_up_with_load_switches_the_station_to_the_aether_profile() {
        let mut station = station();
        let (mut daemon, dir) = daemon("load");
        let done = ok(call(
            &mut station,
            &mut daemon,
            "varac.setup",
            json!({ "load": true }),
        ));
        let id = daemon.profiles.find(varac::AETHER_PROFILE).expect("made");
        assert_eq!(done["active"], json!(id));
        assert_eq!(done["varac"]["aether_profile"]["active"], true);
        assert!(matches!(daemon.config.ptt, PttConfig::Host { .. }));
        assert!(daemon.config.kiss.enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_varac_nothing_is_offered_and_setting_up_says_why() {
        let mut station = station();
        let (mut daemon, dir) = daemon("none");
        daemon.varac.usual = vec![dir.join("nowhere")];
        let status = ok(call(&mut station, &mut daemon, "varac.status", json!({})));
        assert!(status["folder"].is_null(), "{status}");
        assert_eq!(status["show_offer"], false);
        let refused = call(&mut station, &mut daemon, "varac.setup", json!({}));
        assert_eq!(refused.error.expect("refused").code, "not_found");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
