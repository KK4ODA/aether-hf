//! The `profile.*` methods and `config.schema`: the daemon's side of the profile system.
//!
//! A profile is applied the way `config.set` applies a change — on a copy, validated,
//! written atomically, live keys taken on at once and the rest reported for a restart —
//! and everything the load had to leave out or could not find is in the answer, so the
//! panel can say it instead of guessing. The panel is a client of these methods and
//! nothing else: it never reads or writes a profile file itself.

use serde_json::{Value, json};

use crate::{
    control::{
        methods::DaemonState,
        protocol::{ApiError, Response},
    },
    profile::{Applied, Inventory, Profile, ProfileError},
    ptt::Ptt,
    station::Station,
};

/// The methods this module answers.
#[must_use]
pub fn handles(method: &str) -> bool {
    method.starts_with("profile.") || method == "config.schema"
}

/// The methods that change something.
#[must_use]
pub fn is_mutating(method: &str) -> bool {
    matches!(
        method,
        "profile.save"
            | "profile.load"
            | "profile.create"
            | "profile.rename"
            | "profile.duplicate"
            | "profile.delete"
            | "profile.import"
    )
}

/// Answer one of this module's methods.
pub fn dispatch<P: Ptt>(
    station: &mut Station<P>,
    daemon: Option<&mut DaemonState>,
    method: &str,
    params: &Value,
    id: Option<String>,
) -> Response {
    if method == "config.schema" {
        return Response::ok(
            id,
            json!({
                "settings": crate::settings::schema(),
                "live_keys": crate::config::LIVE_KEYS,
                "profile_format": crate::profile::FORMAT,
                "profile_schema": crate::profile::SCHEMA,
                "settings_schema": crate::config::SCHEMA_VERSION,
            }),
        );
    }
    let Some(daemon) = daemon else {
        return Response::failed(
            id,
            ApiError::new("unsupported", ProfileError::NoDirectory.to_string(), false),
        );
    };
    let result = match method {
        "profile.list" => Ok(status_json(daemon)),
        "profile.save" => save(daemon, params),
        "profile.load" => load(station, daemon, params),
        "profile.create" => create(station, daemon, params),
        "profile.rename" => rename(daemon, params),
        "profile.duplicate" => duplicate(daemon, params),
        "profile.delete" => delete(daemon, params),
        "profile.export" => export(daemon, params),
        "profile.import" => import(daemon, params),
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

/// What every answer about profiles carries: the list, which is active, and whether the
/// running configuration has moved from it. Also the `profile` event's body.
#[must_use]
pub fn status_json(daemon: &DaemonState) -> Value {
    let store = &daemon.profiles;
    let profiles = store.list();
    let active = store.active().map(str::to_owned);
    let name = active
        .as_deref()
        .and_then(|a| profiles.iter().find(|p| p.id.eq_ignore_ascii_case(a)))
        .map(|p| p.name.clone());
    json!({
        "active": active,
        "name": name,
        "dirty": store.dirty(&daemon.config, daemon.memories.entries()),
        "profiles": profiles,
        "dir": store.dir().map(|d| d.display().to_string()),
        "extension": crate::profile::EXTENSION,
    })
}

fn api(error: &ProfileError) -> ApiError {
    let code = match error {
        ProfileError::NotAProfile(_) | ProfileError::Newer(_) | ProfileError::BadName(_) => {
            "bad_params"
        }
        ProfileError::Invalid(_) => "refused",
        ProfileError::Io(_) => "cannot_save",
        ProfileError::NoSuchProfile(_) => "not_found",
        ProfileError::NameTaken(_) => "conflict",
        ProfileError::NoDirectory => "unsupported",
    };
    ApiError::new(
        code,
        error.to_string(),
        matches!(error, ProfileError::Io(_)),
    )
}

fn text_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn id_param(params: &Value) -> Result<&str, ApiError> {
    text_param(params, "id").ok_or_else(|| {
        ApiError::new(
            "bad_params",
            "Which profile? `id` names one, as `profile.list` lists them.",
            false,
        )
    })
}

fn name_param(params: &Value) -> Result<&str, ApiError> {
    text_param(params, "name")
        .ok_or_else(|| ApiError::new("bad_params", "A `name` for the profile is needed.", false))
}

/// This machine's devices, as the daemon's own inventory reports them.
fn inventory(daemon: &DaemonState) -> Inventory {
    Inventory::from_json(&(daemon.devices)())
}

/// A profile of the running configuration under a name.
fn capture(daemon: &DaemonState, name: &str) -> Profile {
    Profile::capture(
        name,
        &daemon.config,
        daemon.memories.entries(),
        Some(&inventory(daemon)),
        &crate::profile::now(),
    )
}

/// `profile.save`: the running configuration into the active profile, or — with `name` —
/// into a new one that becomes active.
fn save(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let entry = if let Some(name) = text_param(params, "name") {
        let profile = capture(daemon, name);
        let entry = daemon.profiles.write(&profile, None).map_err(|e| api(&e))?;
        daemon
            .profiles
            .set_active(Some(entry.id.clone()))
            .map_err(|e| api(&e))?;
        entry
    } else {
        let Some(active) = daemon.profiles.active().map(str::to_owned) else {
            return Err(ApiError::new(
                "refused",
                "No profile is active: give the settings a `name` to save them as a new one.",
                false,
            ));
        };
        let existing = daemon.profiles.read(&active).map_err(|e| api(&e))?;
        let mut profile = capture(daemon, &existing.name);
        profile.created = if existing.created.is_empty() {
            profile.created.clone()
        } else {
            existing.created.clone()
        };
        daemon
            .profiles
            .write(&profile, Some(&active))
            .map_err(|e| api(&e))?
    };
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["saved"] = json!(entry);
    Ok(result)
}

/// The load's report, as the answer carries it.
fn report(applied: &Applied) -> Value {
    json!({
        "changed": applied.changed,
        "restart_required": applied.restart_required(),
        "unknown": applied.unknown,
        "ignored": applied.ignored,
        "invalid": applied.invalid,
        "missing_hardware": applied.missing_hardware,
    })
}

/// Apply a profile to the station: validated, written, live keys taken on, made active.
fn commit<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    profile: &Profile,
    id: &str,
) -> Result<Value, ApiError> {
    let applied = profile
        .apply(&daemon.config, Some(&inventory(daemon)))
        .map_err(|e| api(&e))?;
    // the dials are checked before anything is written, so a bad list cannot leave the
    // configuration switched and the dials not
    let mut memories = crate::memories::Memories::open(None);
    if let Some(list) = &applied.memories {
        memories
            .replace(list.clone())
            .map_err(|reason| ApiError::new("refused", reason, false))?;
    }
    applied
        .config
        .save(&daemon.path)
        .map_err(|e| ApiError::new("cannot_save", e.to_string(), true))?;
    if let Some(list) = applied.memories.clone() {
        // already checked above; the only failure left is the disk's
        let _ = daemon.memories.replace(list);
        if let Err(error) = daemon.memories.save() {
            return Err(ApiError::new(
                "cannot_save",
                format!(
                    "the settings were applied but the dial list could not be written: {error}"
                ),
                true,
            ));
        }
    }
    station.apply_live(&applied.config);
    daemon.config = applied.config.clone();
    daemon
        .profiles
        .set_active(Some(id.to_owned()))
        .map_err(|e| api(&e))?;
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["loaded"] = json!({ "id": id, "name": profile.name });
    result["report"] = report(&applied);
    Ok(result)
}

/// `profile.load`: switch the station to a profile.
fn load<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let id = id_param(params)?.to_owned();
    let profile = daemon.profiles.read(&id).map_err(|e| api(&e))?;
    commit(station, daemon, &profile, &id)
}

/// `profile.create`: a new profile of the defaults, keeping only who the station is, made
/// active. What *New profile* does.
fn create<P: Ptt>(
    station: &mut Station<P>,
    daemon: &mut DaemonState,
    params: &Value,
) -> Result<Value, ApiError> {
    let name = name_param(params)?;
    let profile = Profile::fresh(name, &daemon.config, &crate::profile::now());
    let entry = daemon.profiles.write(&profile, None).map_err(|e| api(&e))?;
    commit(station, daemon, &profile, &entry.id)
}

/// `profile.rename`.
fn rename(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let id = id_param(params)?.to_owned();
    let name = name_param(params)?.to_owned();
    let entry = daemon
        .profiles
        .rename(&id, &name, &crate::profile::now())
        .map_err(|e| api(&e))?;
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["renamed"] = json!(entry);
    Ok(result)
}

/// `profile.duplicate`: a copy under a new name, not made active.
fn duplicate(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let id = id_param(params)?.to_owned();
    let name = name_param(params)?.to_owned();
    let mut profile = daemon.profiles.read(&id).map_err(|e| api(&e))?;
    profile.name = name;
    let now = crate::profile::now();
    profile.created.clone_from(&now);
    profile.modified = now;
    let entry = daemon.profiles.write(&profile, None).map_err(|e| api(&e))?;
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["duplicated"] = json!(entry);
    Ok(result)
}

/// `profile.delete`: not the active one.
fn delete(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let id = id_param(params)?.to_owned();
    daemon.profiles.delete(&id).map_err(|e| api(&e))?;
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["deleted"] = json!(id);
    Ok(result)
}

/// `profile.export`: the file's text, for a client to save wherever it likes. Without an
/// `id`, the running configuration as a profile would hold it — what an operator with no
/// profile saved yet still wants to carry away.
fn export(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let (profile, id) = if let Some(id) = text_param(params, "id") {
        (
            daemon.profiles.read(id).map_err(|e| api(&e))?,
            id.to_owned(),
        )
    } else {
        let name = text_param(params, "name")
            .map_or_else(|| daemon.config.callsign.clone(), str::to_owned);
        (capture(daemon, &name), crate::profile::id_for(&name))
    };
    Ok(json!({
        "id": id,
        "name": profile.name,
        "filename": format!("{}.{}", crate::profile::id_for(&profile.name), crate::profile::EXTENSION),
        "text": profile.to_text(),
        "path": daemon.profiles.path_of(&id).map(|p| p.display().to_string()),
    }))
}

/// `profile.import`: a file's text into the store, checked but not loaded. The answer
/// carries what a load would report, so the client can say before switching.
fn import(daemon: &mut DaemonState, params: &Value) -> Result<Value, ApiError> {
    let Some(text) = params.get("text").and_then(Value::as_str) else {
        return Err(ApiError::new(
            "bad_params",
            "The profile file's contents are needed, as `text`.",
            false,
        ));
    };
    let mut profile = Profile::parse(text).map_err(|e| api(&e))?;
    if let Some(name) = text_param(params, "name") {
        name.clone_into(&mut profile.name);
    }
    if profile.name.is_empty() {
        "Imported".clone_into(&mut profile.name);
    }
    let preview = profile
        .apply(&daemon.config, Some(&inventory(daemon)))
        .map_err(|e| api(&e))?;
    let replace = daemon.profiles.find(&profile.name).filter(|_| {
        params
            .get("replace")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let entry = daemon
        .profiles
        .write(&profile, replace.as_deref())
        .map_err(|e| api(&e))?;
    daemon.note_profiles_changed();
    let mut result = status_json(daemon);
    result["imported"] = json!(entry);
    result["report"] = report(&preview);
    result["aether_version"] = json!(profile.aether_version);
    Ok(result)
}

/// The store to use in a test: a temporary directory of its own.
#[cfg(test)]
pub(crate) fn temp_store(tag: &str) -> (crate::profile::Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("aether-profiles-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    (
        crate::profile::Store::open(Some(&dir.join("station.toml"))),
        dir,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{methods::dispatch_with, protocol::Request};

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

    /// A daemon whose configuration file and profiles live in a temporary directory.
    fn daemon(tag: &str) -> (DaemonState, std::path::PathBuf) {
        let (store, dir) = temp_store(tag);
        let mut config = crate::config::Config::parse(crate::config::EXAMPLE).expect("example");
        config
            .merge(&json!({
                "callsign": "KK4ODA",
                "audio.input": "USB Audio CODEC",
                "audio.output": "USB Audio CODEC",
                "ptt.kind": "serial",
                "ptt.port": "COM3",
                "radio.max_mode": 9,
            }))
            .expect("merge");
        let path = dir.join("station.toml");
        config.save(&path).expect("save");
        let mut daemon = DaemonState::new(config, path, crate::log::Log::memory(50));
        daemon.heard = crate::heard::HeardList::open(None);
        daemon.memories = crate::memories::Memories::open(None);
        daemon.profiles = store;
        daemon.devices = || {
            json!({
                "devices": [{"name": "USB Audio CODEC", "input": true, "output": true}],
                "serial_ports": [{"name": "COM3", "description": "Prolific PL2303GC USB Serial COM Port"}],
            })
        };
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
                token: None,
            },
        )
    }

    fn ok(response: Response) -> Value {
        assert!(response.ok, "{:?}", response.error);
        response.result.expect("result")
    }

    #[test]
    fn the_first_start_adopts_the_configuration_and_the_list_says_so() {
        let mut station = station();
        let (mut daemon, dir) = daemon("adopt");
        assert!(ok(call(&mut station, &mut daemon, "profile.list", json!({})))["active"].is_null());
        daemon
            .profiles
            .adopt(
                &daemon.config,
                daemon.memories.entries(),
                None,
                &crate::profile::now(),
            )
            .expect("adopt");
        let list = ok(call(&mut station, &mut daemon, "profile.list", json!({})));
        assert_eq!(list["active"], "Default");
        assert_eq!(list["name"], "Default");
        assert_eq!(list["dirty"], false);
        assert_eq!(list["profiles"].as_array().map(Vec::len), Some(1));
        assert_eq!(list["extension"], "aetherprofile");
        // and `config.set` moves the station off it
        ok(call(
            &mut station,
            &mut daemon,
            "config.set",
            json!({ "radio.max_mode": 5 }),
        ));
        assert!(daemon.take_profiles_changed(), "the panel is told");
        let list = ok(call(&mut station, &mut daemon, "profile.list", json!({})));
        assert_eq!(list["dirty"], true);
        // a save cleans it
        let saved = ok(call(&mut station, &mut daemon, "profile.save", json!({})));
        assert_eq!(saved["dirty"], false);
        assert_eq!(saved["saved"]["id"], "Default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_as_creates_and_switches_and_loading_switches_back() {
        let mut station = station();
        let (mut daemon, dir) = daemon("switch");
        daemon
            .profiles
            .adopt(&daemon.config, &[], None, &crate::profile::now())
            .expect("adopt");
        ok(call(
            &mut station,
            &mut daemon,
            "config.set",
            json!({ "radio.max_mode": 4, "audio.input": "Other Card" }),
        ));
        let saved = ok(call(
            &mut station,
            &mut daemon,
            "profile.save",
            json!({ "name": "Portable" }),
        ));
        assert_eq!(saved["active"], "Portable");
        assert_eq!(saved["dirty"], false);
        assert_eq!(saved["profiles"].as_array().map(Vec::len), Some(2));
        // the same name again is a conflict, not a silent overwrite
        let again = call(
            &mut station,
            &mut daemon,
            "profile.save",
            json!({ "name": "portable" }),
        );
        assert_eq!(again.error.expect("refused").code, "conflict");

        let loaded = ok(call(
            &mut station,
            &mut daemon,
            "profile.load",
            json!({ "id": "Default" }),
        ));
        assert_eq!(loaded["active"], "Default");
        assert_eq!(loaded["loaded"]["name"], "Default");
        assert_eq!(daemon.config.radio.max_mode, 9);
        assert_eq!(
            daemon.config.audio.input.as_deref(),
            Some("USB Audio CODEC")
        );
        let report = &loaded["report"];
        assert!(
            report["changed"]
                .as_array()
                .expect("changed")
                .iter()
                .any(|k| k == "radio.max_mode")
        );
        assert!(
            report["restart_required"]
                .as_array()
                .expect("restart")
                .iter()
                .any(|k| k == "audio.input")
        );
        assert!(
            !report["restart_required"]
                .as_array()
                .expect("restart")
                .iter()
                .any(|k| k == "radio.max_mode")
        );
        assert_eq!(report["missing_hardware"], json!([]));
        // the file on disk is the loaded profile's
        let on_disk = crate::config::Config::load(&daemon.path).expect("load");
        assert_eq!(on_disk.radio.max_mode, 9);
        assert_eq!(on_disk, daemon.config);
        // the live key reached the modem
        assert_eq!(station.config().link.max_mode, 9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_profile_is_the_defaults_with_the_stations_identity() {
        let mut station = station();
        let (mut daemon, dir) = daemon("new");
        daemon.config.operator.grid = "EM73tv".into();
        let created = ok(call(
            &mut station,
            &mut daemon,
            "profile.create",
            json!({ "name": "Bench" }),
        ));
        assert_eq!(created["active"], "Bench");
        assert_eq!(daemon.config.callsign, "KK4ODA");
        assert_eq!(daemon.config.operator.grid, "EM73tv");
        assert_eq!(daemon.config.ptt, crate::config::PttConfig::None);
        assert_eq!(daemon.config.audio.input, None);
        assert_eq!(daemon.config.radio.max_mode, 13);
        assert_eq!(
            daemon.config.control.token.as_deref(),
            None,
            "the example's has none"
        );
        let no_name = call(&mut station, &mut daemon, "profile.create", json!({}));
        assert_eq!(no_name.error.expect("refused").code, "bad_params");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_duplicate_and_delete_keep_the_active_mark_straight() {
        let mut station = station();
        let (mut daemon, dir) = daemon("names");
        daemon
            .profiles
            .adopt(&daemon.config, &[], None, &crate::profile::now())
            .expect("adopt");
        let renamed = ok(call(
            &mut station,
            &mut daemon,
            "profile.rename",
            json!({ "id": "Default", "name": "Home" }),
        ));
        assert_eq!(renamed["active"], "Home");
        assert_eq!(renamed["renamed"]["id"], "Home");
        let copied = ok(call(
            &mut station,
            &mut daemon,
            "profile.duplicate",
            json!({ "id": "Home", "name": "Home copy" }),
        ));
        assert_eq!(copied["active"], "Home", "a copy is not switched to");
        assert_eq!(copied["profiles"].as_array().map(Vec::len), Some(2));
        let refused = call(
            &mut station,
            &mut daemon,
            "profile.delete",
            json!({ "id": "Home" }),
        );
        assert_eq!(refused.error.expect("refused").code, "refused");
        let deleted = ok(call(
            &mut station,
            &mut daemon,
            "profile.delete",
            json!({ "id": "Home copy" }),
        ));
        assert_eq!(deleted["profiles"].as_array().map(Vec::len), Some(1));
        let missing = call(
            &mut station,
            &mut daemon,
            "profile.delete",
            json!({ "id": "Home copy" }),
        );
        assert_eq!(missing.error.expect("refused").code, "not_found");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_and_import_carry_a_profile_between_daemons() {
        let mut station = station();
        let (mut here, dir_here) = daemon("export");
        let inventory = Inventory::from_json(&(here.devices)());
        here.profiles
            .adopt(&here.config, &[], Some(&inventory), &crate::profile::now())
            .expect("adopt");
        let exported = ok(call(
            &mut station,
            &mut here,
            "profile.export",
            json!({ "id": "Default" }),
        ));
        assert_eq!(exported["filename"], "Default.aetherprofile");
        let text = exported["text"].as_str().expect("text").to_owned();
        assert!(text.contains("\"format\": \"aether-hf-profile\""));
        assert!(!text.contains("token"), "{text}");
        // without an id, the running configuration itself
        let running = ok(call(&mut station, &mut here, "profile.export", json!({})));
        assert_eq!(running["name"], "KK4ODA");

        let (mut there, dir_there) = daemon("import");
        there.devices = || {
            json!({
                "devices": [{"name": "Realtek Audio", "input": true, "output": true}],
                "serial_ports": [{"name": "COM9", "description": "Prolific PL2303GC USB Serial COM Port"}],
            })
        };
        let imported = ok(call(
            &mut station,
            &mut there,
            "profile.import",
            json!({ "text": text, "name": "From home" }),
        ));
        assert_eq!(imported["imported"]["name"], "From home");
        assert!(imported["active"].is_null(), "imported, not loaded");
        let missing = imported["report"]["missing_hardware"]
            .as_array()
            .expect("missing");
        assert_eq!(missing.len(), 3, "{missing:?}");
        let port = missing
            .iter()
            .find(|m| m["key"] == "ptt.port")
            .expect("port");
        assert_eq!(port["name"], "COM3");
        assert_eq!(port["suggestion"], "COM9", "the same bridge, renumbered");
        // the same name again is refused unless replacing is asked for
        let again = call(
            &mut station,
            &mut there,
            "profile.import",
            json!({ "text": text, "name": "From home" }),
        );
        assert_eq!(again.error.expect("refused").code, "conflict");
        ok(call(
            &mut station,
            &mut there,
            "profile.import",
            json!({ "text": text, "name": "From home", "replace": true }),
        ));
        // then loaded: the devices stay as named and are reported again
        let loaded = ok(call(
            &mut station,
            &mut there,
            "profile.load",
            json!({ "id": "From home" }),
        ));
        assert_eq!(there.config.audio.input.as_deref(), Some("USB Audio CODEC"));
        assert_eq!(
            loaded["report"]["missing_hardware"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );
        // and rubbish is refused with a reason
        let bad = call(
            &mut station,
            &mut there,
            "profile.import",
            json!({ "text": "{ nope" }),
        );
        let error = bad.error.expect("refused");
        assert_eq!(error.code, "bad_params");
        assert!(
            error.message.contains("not an Aether HF profile"),
            "{}",
            error.message
        );
        let _ = std::fs::remove_dir_all(&dir_here);
        let _ = std::fs::remove_dir_all(&dir_there);
    }

    #[test]
    fn a_profile_that_will_not_work_leaves_the_station_as_it_was() {
        let mut station = station();
        let (mut daemon, dir) = daemon("refused");
        daemon
            .profiles
            .adopt(&daemon.config, &[], None, &crate::profile::now())
            .expect("adopt");
        let mut broken = daemon.profiles.read("Default").expect("read");
        broken.name = "Broken".into();
        broken.settings["ptt"] = json!({ "kind": "cat", "port": "COM3", "protocol": "icom" });
        daemon.profiles.write(&broken, None).expect("write");
        let before = daemon.config.clone();
        let on_disk = std::fs::read_to_string(&daemon.path).expect("read");
        let refused = call(
            &mut station,
            &mut daemon,
            "profile.load",
            json!({ "id": "Broken" }),
        );
        let error = refused.error.expect("refused");
        assert_eq!(error.code, "refused");
        assert!(error.message.contains("civ_address"), "{}", error.message);
        assert_eq!(daemon.config, before);
        assert_eq!(
            std::fs::read_to_string(&daemon.path).expect("read"),
            on_disk
        );
        assert_eq!(daemon.profiles.active(), Some("Default"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_schema_is_served() {
        let mut station = station();
        let (mut daemon, dir) = daemon("schema");
        let schema = ok(call(&mut station, &mut daemon, "config.schema", json!({})));
        let settings = schema["settings"].as_array().expect("settings");
        assert!(
            settings.iter().any(|s| s["key"] == "radio.max_key_s"
                && s["type"] == "number"
                && s["live"] == true)
        );
        assert!(
            settings
                .iter()
                .any(|s| s["key"] == "control.token" && s["scope"] == "secret")
        );
        assert_eq!(schema["profile_format"], "aether-hf-profile");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_configuration_file_the_methods_say_so() {
        let mut station = station();
        let response = crate::control::methods::dispatch(
            &mut station,
            &Request {
                id: None,
                method: "profile.list".into(),
                params: json!({}),
                token: None,
            },
        );
        assert_eq!(response.error.expect("refused").code, "unsupported");
    }
}
