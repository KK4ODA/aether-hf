# ADR-0011: Profiles — a station's settings as one portable file, projected from the configuration through a settings registry

**Status:** accepted 2026-09-19, port only (the model has no configuration) · **Roadmap:** Phase 4
(the panel), Phase 6 (field stations with more than one radio) · **Builds on:** P3-4 (the control
API, `config.get`/`config.set`, the schema-versioned file), P5 (the configuration's migration chain)

## 1. Context

A station has one configuration file, `station.toml`, and the daemon runs from it: the
`Config` struct is the authoritative model, serde gives every field its name, type and
default, `validate()` holds the rules, `config.set` merges dotted keys on a copy and writes
the file atomically, and `LIVE_KEYS` says what takes effect without a restart. That is a
good settings architecture for one radio in one place.

An operator with two radios, or a base and a truck, or a friend's computer for a weekend,
has to retype Setup — or hand-edit a TOML file with a serial port number in it that means
nothing on the other machine. What is wanted is the station's settings under a name: save,
switch, carry to another computer, and have a newer version still read what an older one
wrote. The obvious way — a list of the fields a profile contains — is the wrong way: the
daemon gains a setting every few betas, and a list is stale the week after it is written.

Three things in the file are not the station's. A path (`log.file`, `record.dir`,
`control.ui_dir`) and a socket (`control.bind`, `sim.*`) are this installation's wiring; the
control token is a secret; and a device name (`audio.input`, `ptt.port`) is the station's
choice *about hardware another computer may not have*. The panel also kept the waterfall's
controls in the browser's own storage, outside the model altogether.

## 2. Decision

1. **The configuration file stays what the daemon runs from.** A profile is applied *to*
   it; nothing about how the daemon starts, validates, migrates or saves the file changes.
   The shell, which reads `[update]` from the file, needs no change.
2. **A settings registry, derived from the struct.** `settings.rs` enumerates every leaf of
   a fully populated template of `Config` (plus one keying section of each kind, since the
   keying section is an enum) and describes each: type, default, whether it is live, and —
   from a short table of *rules* keyed by dotted name — its **scope** (portable, hardware,
   machine, secret), its bounds or options, and the *why* an error message carries. A
   setting without a rule is portable, unbounded and not nullable: the right default for
   almost everything that gets added. The registry's bounds are the configuration's bounds:
   `validate()` runs `check_bounds` over the serialised configuration, and a test holds
   every rule to it, so a bound is written once. Tests also hold the template to the struct
   (a null leaf is an `Option` field nobody added), the nullable flags to serde, the
   bandwidth list to the physical layer, and every rule to a real key. `config.schema`
   serves the registry.
3. **A profile is the configuration projected by exclusion.** `profile::portable` keeps
   every leaf whose scope is portable or hardware. A field added to any section is in the
   next profile saved; only a field that must *not* travel needs a rule. The file is JSON
   (`.aetherprofile`) with an envelope — `format`, the envelope's `schema`, the
   configuration `settings_schema` the settings are in, `aether_version`, `name`,
   `created`, `modified` — the `settings` in the configuration's own shape, a `hardware`
   block (the keying port's description as the driver gave it), and the dial `memories`.
   The panel's waterfall controls moved into `[panel.waterfall]` (live keys) so they are
   the station's and travel with it; Compact and the chart tab stay per window.
4. **Loading is transactional and degrades one setting at a time.** Parse; refuse a newer
   envelope or newer settings schema; bring the envelope forward through its own chain and
   the settings through the configuration file's `MIGRATIONS` (one chain for one rename:
   the settings become a TOML table, nulls stripped, and go through the same steps the file
   does). Then, on a copy of the running configuration: every portable setting reset to
   its default — a profile means the same station wherever it is loaded, and what it does
   not say must not leak from the profile before — and each of the profile's leaves taken,
   or set aside as *unknown* (not in the registry), *ignored* (machine or secret), or
   *invalid* (wrong type, or a bound) with the reason; the keying section rebuilt around
   its kind. The whole is validated as the file would be. Only then is the file written
   (atomically, as always), the dial list replaced, the live keys taken on, the profile
   made active, and `restart_required` reported for the rest — the same shape `config.set`
   has. A rule between two settings that fails refuses the load and changes nothing.
5. **A device the computer does not have is named, not replaced.** Device names are
   applied as they are and reported as `missing_hardware`; the daemon already runs
   receive-only on a port it cannot open and silent on a sound card it cannot find, and
   says so, so nothing keys a radio the profile did not choose. A serial port with the
   same description behind it is *suggested* when there is exactly one — a radio's two
   ports share one description, and choosing between two would be a guess. The panel shows
   a missing device in its list as *not on this computer* rather than letting the list fall
   back to *system default*.
6. **Dirty is computed, not tracked.** A profile is dirty when loading it again would
   change something (the portable projection differs, or the dial list does). Every client
   change — `config.set`, `frequencies.set`, any `profile.*` — makes the run loop publish a
   `profile` event with the list, so a panel's `Home FTDX10 *` is never stale.
7. **The first start adopts.** A store that has never been touched writes the running
   configuration as the profile *Default* and makes it active; the file is not modified.
   A store with a state file is left alone even when it names no active profile.

## 3. Alternatives considered

* **A hand-maintained list of profile fields.** Simple to write and wrong within a release:
  the reason the registry is derived from the struct. A derive macro would give the same
  guarantee with a proc-macro crate to maintain; the template-plus-tests approach gives it
  with none, at the cost of one test failing when an `Option` field is added without a
  template line — which is the reminder wanted.
* **The profile as the configuration file itself** (switch = copy a TOML over
  `station.toml`). Not portable — the file holds paths, a token and this machine's
  sockets — and the shell reads the file by name.
* **Machine-local keys taken from the profile when present.** A file edited by hand could
  then carry a token or a network bind onto another machine. Ignored and reported instead.
* **Missing settings keep the running value.** Convenient on the same machine and wrong
  across profiles: a setting the truck's profile never mentioned would follow the operator
  from the base profile. Defaults, and a report.
* **Choosing the suggested port.** The description is the best evidence there is that
  `COM11` is the `COM6` of the other computer, and still a guess; the operator confirms it
  in Setup.
* **A registry-driven panel form.** The registry could generate Setup; it is not asked to.
  The panel's form is hand-laid-out on purpose, and the ranges it checks for its ticks are
  a known duplication left for another change.

## 4. Consequences

* `config.rs` gains `WaterfallSection`, five live keys, `Config::with_callsign`, and loses
  eight bespoke range checks to the registry (messages now carry the registry's *why*).
  `validate()` keeps what a registry cannot say: the Icom address, the sim's alternatives,
  the token on a network bind, the Morse speed that is checked only when Morse is on.
* New: `settings.rs`, `profile.rs`, `control/profiles.rs`; the `profile.*` methods,
  `config.schema` and the `profile` event in `docs/spec/control-api.md` §4.9; the panel's
  Profile bar at the top of Setup, its name form, unsaved-changes prompt, import through a
  file input and export through a download; `tests/data/profiles/` holds a file as each
  version wrote it, and a test that each still loads with nothing lost.
* What a profile carries and what it does not is one table in one file, and the answer to
  "does the new setting go in profiles?" is "yes, unless you write a rule".
* Open: the `.aetherprofile` file association for the desktop shell (double-click to
  import) and export through a native save dialog under the shell; both are the shell's
  work and wait on its file-handling capabilities. The panel's own range checks
  (`checkModemSettings`) could read `config.schema` instead of repeating the numbers.
