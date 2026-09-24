# Aether control API — specification v0.1

Status: **draft**. This specifies an interface that Phase 3 will implement; it is published
now so the daemon, the GUI and third-party tools can be built against the same contract
rather than against each other.

Companion documents: `air-interface.md` (what goes over the air), `host-interfaces.md`
(Winlink/Pat compatibility adapters), `../adr/0001-language-and-stack.md`.

---

## 1. Why this exists

The control API is the modem's own interface: everything a GUI, a script, or a remote web
client needs to configure a station, start a session, move data, and watch what is happening.
It is deliberately *not* the Winlink-compatible TCP interface. That one exists for
compatibility and is constrained by what an existing application already expects; this one is
free to expose SNR, carrier offset, constellations, spectra, device enumeration and PTT
testing, which is what makes a modem debuggable by its operator rather than opaque.

Two design rules follow from the audit and from `COMMUNITY-CONCERNS.md`:

* **PHY-agnostic.** HF and the future FM physical layer differ only in the mode table and a
  handful of capability fields. Nothing in this document names a modulation.
* **Nothing hidden.** Every quantity the modem uses to make a decision — measured SNR, the
  mode the rate controller chose and why, buffer occupancy, PTT state — is observable. A
  mode whose behaviour an operator cannot see is one they cannot trust or report a bug
  against.

---

## 2. Transport

| | |
|---|---|
| Primary | JSON over WebSocket, `ws://127.0.0.1:8515/v1` |
| One-shots | HTTP REST on the same port, `POST /v1/<method>` with the same body |
| Encoding | UTF-8 JSON, one object per WebSocket message |
| Default bind | loopback only |

**Authentication.** No token is required on a loopback bind. Binding to any other interface
requires a bearer token (`Authorization: Bearer <token>`, or `{"token": …}` in the first
WebSocket message); the daemon refuses a non-loopback bind that has no token configured
rather than starting an open one. The token is generated on first run and shown in the GUI.

**Versioning.** The path carries the major version. Fields may be added within a version;
existing fields do not change meaning. A client must ignore fields it does not recognise.

---

## 3. Message shape

Request:

```json
{ "id": "7", "method": "connect", "params": { "remote": "KK4XYZ" } }
```

Response, one per request, correlated by `id`:

```json
{ "id": "7", "ok": true, "result": { "session": 42 } }
```

Error:

```json
{ "id": "7", "ok": false, "error": { "code": "busy_channel",
  "message": "The channel is busy. Waiting for it to clear.", "retryable": true } }
```

Events are unsolicited and carry no `id`:

```json
{ "event": "state", "data": { "state": "connected", "role": "iss", "remote": "KK4XYZ" } }
```

**Error messages are written for operators, not developers.** `"The sound device 'USB Audio
CODEC' was unplugged. Reconnect it or choose another in Settings → Audio."` — not a stack
trace, and not `ENODEV`. The `code` is the stable machine-readable field; `message` is
human-facing and may be localised.

---

## 4. Methods

### 4.1 Status and configuration

| Method | Params | Result |
|---|---|---|
| `status` | — | state, role, callsign and callsigns, remote callsign, uptime, versions, capabilities, `supervised` (whether somebody will start the daemon again if it asks), `binary` (the executable it runs from — how the desktop shell tells a daemon of its own installation from somebody else's), `frequency_hz` (the dial, when the keying interface can ask the radio), `can_tune` (whether `frequency.set` has a way to: CAT or `rigctld`), `link` (the session's account, §4.8), `host` (`enabled`, the command and data addresses, and `connected`: whether a host program holds the port right now), and `metrics` and `counters` as the event and the sidecar carry them. `metrics.tx_peak_dbfs` is the largest sample the modem handed the sound card on its last transmission, after the transmit level: the headroom figure no ALC meter can show, because it is measured before the radio |
| `config.get` | — | the configuration, the file it came from, and which keys apply without a restart |
| `config.set` | dotted key/value pairs | which keys changed, and which of them need a restart |
| `config.schema` | — | the settings registry (§4.9): every setting with its `key`, `type`, `default`, `nullable`, `scope`, `live`, and its `min`/`max`/`options` and `why` where it has a bound; `live_keys`; the profile format's name and both schema numbers |
| `capabilities` | — | `bandwidth_hz` (the waveform the station runs: 2300 or 500), `bandwidths_hz` (what this version has), the ladder of the running waveform (`modes`, one entry a rung: index, name, payload bytes and net bit rate *of the frame the rung goes out on*, AWGN threshold, `floor` — whether the rung is the tone floor's (ADR-0013, its fast kinds included, ADR-0014), whose frames are five times as long as an ordinary one; `usable_modes`), whether the PHY reports preambles, the SNR reference |
| `diagnostics` | — | everything a bug report needs, in one object (§4.6) |
| `shutdown` | `restart?` | `stopping`; the transmitter is released on the way out. With `restart: true` the daemon exits with status 75 (`EX_TEMPFAIL`), which the desktop shell and the systemd unit (`RestartForceExitStatus=75`) take as "start me again" — the way a setting that needs a restart is applied without the operator having to know. The reply is written before the daemon exits, and so is every other reply already on its way (for up to two seconds); a request that reaches it while it stops is answered `modem_stopped` |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic. A mode number is a rung of the waveform's **ladder**
(ADR-0013, ADR-0014, ADR-0015): the tone floor's kinds, then the OFDM modes — twenty rungs
at 2 300 Hz (the floor's two and its four fast kinds at rungs 0–5, the fourteen OFDM modes
from BPSK ⅕ at rungs 6–19), fifteen at 500 Hz (the floor's two and its two four-tone middle
kinds at rungs 0–3, then QPSK ⅓ up at rungs 4–14) — and a
mode number means nothing without the `bandwidth_hz` it came with. `[radio] bandwidth` chooses the waveform and needs a
restart; `[radio] answer_only` (live) makes the station take calls and make none — what
§97.221(c) allows an unattended station at 500 Hz outside the automatic sub-bands, and
what `connect`, `beacon` and `probe` are refused with while it is set.

`config.get` and `diagnostics` return the configuration **with the secrets taken out**:
`control.token` comes back as the string `<set>` when one is configured. A loopback client
needs no token, so it must not be able to read the one that guards a network bind.

### 4.2 Session

| Method | Params | Result |
|---|---|---|
| `connect` | `remote`, `callsign?`, `bandwidth?` | session id; then `state` events. `callsign` picks which of the station's callsigns to call as (the first, when absent) |
| `callsigns.set` | `callsigns` (list) | the callsigns the station answers to from now on, the first being the one it calls as, and `applied`: `false` when a session is up, in which case they take effect as it ends. Replaces `[station] callsign` for the daemon's lifetime without touching the file: a host program's `MYCALL` is the operator's callsign, and the file is what the station answers to until one says otherwise |
| `disconnect` | — | accepted; closes after the queue drains |
| `abort` | — | accepted; drops the session immediately |
| `beacon` | — | accepted; one frame with this station's callsign, addressed to nobody, at the most robust mode. Refused during a session and on an answer-only station |
| `probe` | `remote`, `callsign?` | accepted; one frame asking `remote` whether it hears this station, and how well (ADR-0006). The answer, or its absence, arrives as a `log` event named `probe`: `<call> hears us at <x> dB, heard at <y> dB` — the SNR the other station measured on the probe, and the SNR this one measured on the answer — or `<call>: no answer` after one frame's turnaround. One probe out at a time (`not_idle`, retryable); refused during a session and on an answer-only station, which answers probes and sends none. The other end reports a probe it answered as a `log` event named `probed` |
| `test.start` | `remote`, `callsign?`, `remote_grid?`, `message_bytes?` (2048), `file_bytes?` (16384), `ladder?` (true), `rung_frames?` (4), `budget_s?` (600) | accepted; the **Test session** of P6-7 with `remote`: a probe, a call, the message and the file (incompressible bytes, timed — the sizes are ceilings: the probe's SNR sizes the message under its ceiling, half a kilobyte below 0 dB, a kilobyte below 6 dB, and the message's measured rate sizes the file to about two minutes' worth), then the **mode ladder** — a burst pinned at each mode from the floor up, its frames small enough to be re-encoded at a slower mode if that mode fails, until three rungs in a row decode fewer than half their frames — then an orderly disconnect; the whole run keeps to `budget_s`, shortening or skipping what would not fit (`results.adjustments` says what), and a transfer or rung that stalls past its time ends the run with an abort, keeping what was learned. All of it is one recording named `…_test`, with the report under the sidecar's `session.test` and the operator's `[operator]` grid, rig, power and antenna beside it. Progress arrives as `log` events named `test`, and `status.test` gives the step, the elapsed time and the rungs so far while it runs. The other station only listens; an answer-only station is a fine partner. Refused (`not_idle`) during a session, a probe or another test, and on an answer-only station |
| `test.status` | — | `running`, and `results` — the running test's, or the last one's until the next starts: `remote`, `started`, `elapsed_s`, `step`, `outcome` (`complete`, or `aborted: <why>`), `bandwidth_hz`, `probe` (`heard_there_db`, `heard_here_db`; null when unanswered), `message` and `file` (`bytes`, `seconds`, `bps`), `ladder` (per rung: `mode`, `frames`, `decoded`, `snr_db` as the other station measured it, `seconds`), `path` (`my_grid`, `their_grid`, `km` from the two grids) |
| `test.abort` | — | `aborted`: whether one was running. The session is aborted at once (one DISC on the way out), the outcome is `stopped by the operator`, and the report keeps every step that finished |
| `listen` | `enabled` | accepted |
| `send` | `data` (base64) | bytes accepted into the queue |

`disconnect` is orderly: queued data is sent and acknowledged first. `abort` is not. The
distinction matters to an operator watching a transfer and is why both exist.

### 4.3 Devices and calibration

| Method | Params | Result |
|---|---|---|
| `devices.list` | — | input/output devices with names and default flags, serial ports as `{name, description}` — the description is what the driver says is behind the port, which is how an operator tells a radio's CAT port from the one that keys — and `gpio_interfaces` as `{path, name}`: the CM108-class interfaces that key through their codec's GPIO pin (`[ptt] kind = "cm108"`) |
| `ptt.test` | `duration_s` (0.2–5) | keys the radio with no audio for that long, so the operator can watch the rig and the interface's PTT light |
| `tune` | `duration_s` (0.5–10, or 0 to stop) | keys and plays a steady tone at the transmit level — what an antenna tuner needs, and **not** how to set drive (use `drive.set`). `audio.tx_level` is live and is applied as audio leaves, so the level can be moved while the tone plays; `0` cuts it short, and only an operator's own test transmission is ever cut |
| `drive.set` | `bursts` (1–10, default 4, or 0 to stop) | keys and sends that many real bursts at the fastest mode the station is allowed, so the rig's ALC is shown the peaks traffic will actually present it with. A tune tone is a sine and the daemon scales the waveform to the tone's RMS, so the waveform's peaks land about 6 dB (floor mode) to 7 dB (fastest) above anything the tone reaches — drive set on the tone is that far into limiting on traffic. The bursts carry filler, not protocol: a station that decodes one finds a data frame for a session it does not have and ignores it. `0` stops them, as for `tune` |
| `audio.level` | — | the last three seconds of received audio: RMS and peak in dBFS, clipping fraction, and a sentence of advice |
| `spectrum` | — | the last window of captured audio transformed: `bin_hz`, `bins_db` (dBFS per bin from 0 Hz to 4 kHz; empty until a window has been heard), `passband_hz` (where this modem's signal sits), `transmitting`. Polled, not streamed: it costs one transform per call and nothing otherwise |
| `constellation` | — | the last frame's equalised symbols as `points` (`[i, q]` pairs, thinned to at most 1024) and the `frame` they came from, as the `frame` event describes it |

These exist because setup, not propagation, is what defeats most new users of an HF data mode
(`COMMUNITY-CONCERNS.md`). A modem that can key on demand and say whether its input is
clipping can lead the operator through setup instead of leaving them to guess.

`ptt.test` keys at once — an SSB transmitter keyed with no audio radiates nothing, and an
operator watching a PTT light cannot be told "accepted" and kept waiting. `tune` and `drive.set` are
transmissions: refused during a session and while the channel is busy (refused, not deferred,
because a transmission that starts on its own a minute later would surprise the person
holding the drive control). Neither can measure whether the *radio* keyed — only the operator can see
that — which is why they exist: to let the operator look. `audio.level` is always on; it
reports `settled: false` and "Still listening." until it has heard enough to mean anything,
rather than a number that does not. `devices.list` also reports the sample rates each device
will run at, so a panel can say "this device is at 44.1 kHz" before the daemon refuses it.

### 4.4 Recording and the stations heard

| Method | Params | Result |
|---|---|---|
| `record.start` | `name` (optional), `notes` (optional) | `path` of the WAV being written |
| `record.stop` | — | `wav`, `sidecar`, `seconds`, `frames` found, `decoded` |
| `record.notes` | `notes` | accepted; kept for the next recording that starts on its own. Without one, an automatic recording carries the standing `[record] notes` from the configuration (a live key) — what an unattended station has to say about its band and antenna |
| `heard.list` | — | `stations`: every station heard, most recent first — `callsign`, `first_heard_ms` and `last_heard_ms` (Unix milliseconds), `count`, `snr_db` (last) and `best_snr_db`, `frequency_hz` (when the radio could say), `mode`, `activity` (`beacon`, `calling`, `probing`, `answering`, `connected`), `detail` (whom it was calling, probing or answering) and `connected` (whether a session with it has ever been up from here); `limit` (200) and the `path` of the file the list lives in |
| `heard.clear` | — | `cleared`: how many were forgotten |
| `frequencies.list` | — | `memories`: the remembered dials, by frequency — `hz` and `name` (the operator's name or comment) — starting as the frequency plan's proposals; `limit` (200) and the `path` of the file the list lives in (`frequencies.json` beside the configuration) |
| `frequencies.set` | `memories`: `[{hz, name}]` | the list replaced whole, written to its file, and returned as `frequencies.list` would; a frequency given twice keeps its last name, names are trimmed and at most 60 characters, `bad_params` for a frequency no radio has a dial for |
| `frequency.set` | `hz` | `hz`: the radio tuned there, over CAT (`FA`, or CI-V `05`, checked by reading the dial back or by the Icom's acknowledgement) or `rigctld` (`F`); `refused` while transmitting, during a session, or with a keying interface that cannot tune (a serial line, a CM108 codec, none) — `status.can_tune` says which in advance. The next `status` reads the dial from the radio again within two seconds |

A recording is a mono 16-bit WAV at the modem's 48 kHz of everything the sound card
delivered, and a JSON sidecar of what the modem made of it: every frame the receiver found
(`t_s`, `kind`, `mode`, `rv`, `snr_3k_db`, `cfo_hz` — null when the acquisition was a probable noise trigger — `confidence`, `detect_confidence`, `decoded`, `bytes`), every event with
every frame the station **sent** (`sent`: `t_s`, `kind`, `mode`, `rv`, `floor`, `bytes`), every change of the busy state as a `busy` event whose detail names the path and the numbers that decided it (`on: level -19.8 dBFS is +7.4 dB over the floor -27.2 (threshold 6.0); …` or `on: a frame acquired at confidence 1.72; …`, and `off: …`),
every event with the modem's state, when the transmitter was keyed and released, a
`tx_peak` event after each transmission carrying its peak in dBFS — so a burst nobody
decoded can be read against how hard the transmitter was being driven for it — the counters
at the end,
the `notes`, and `frequency_hz` when the keying backend can ask the rig (`rigctld`; a
keying line cannot, and the field is null rather than a guess). Times are seconds from the
start of the file by the station's audio clock.
The sidecar's `format` is `aether-hf-session/4`. `status` carries `recording` — the path and
length so far — while one runs. With `[record] auto = true` every session records itself
from connect to disconnect, one file each, named `YYYYMMDD-HHMMSS_<mycall>_<remote>`.

`aetherd --replay <wav>` runs a recording back through the receiver and, with the sidecar
beside it, fails if fewer frames decode than did on the day; `field/` is where the ones worth
keeping live, and `core/aetherd/tests/field.rs` replays them all on every test run.

The stations heard are every frame that carried a callsign — a beacon, a connect request or
its answer overheard between any two stations — and every frame of a session with the
station at the other end. One entry per callsign, at most 200, the one heard longest ago
making room for a new one; kept in `heard.json` beside the configuration and written a few
seconds after it changed, so a station left listening overnight can say in the morning who
was on. Each change goes out as a `heard` event.

---

## 5. Events

| Event | When | Key fields |
|---|---|---|
| `state` | session state changes | state, role, remote, callsign (the one this session runs under: a station that answers to several is addressed by whichever was called) |
| `metrics` | every 500 ms while a client listens | `mode`, `queued_bytes`, `noise_floor_db` and `level_db` (the busy detector's readings, null until it has settled), `excess_peak_db` (the largest level-over-floor the detector tested since the last reading — the decision is made forty times a second on a 50 ms quantity, so the excursions that cross the threshold are the ones a sampled reading almost never lands on), `shape_db` (the passband's highest spectral bin over its median bin, per 200 ms: flat noise reads about 6 dB, a narrowband signal — FT8, CW, PSK — 15 and up, and a receiver's AGC cannot compress it), `channel_busy`, `busy_reason` (`level` when the threshold last marked it, `shape` when the passband's spectrum did, `frame` when a decoded frame did, null if never — an acquired preamble alone never marks the channel busy: on a real band acquisition confidence overlaps between a phantom and a weak real frame, and only a decode is evidence), `transmitting`, `receiving` (a burst is arriving), `audio` (as `audio.level`), `snr_db` and `cfo_hz` (null when the last frame was a low-confidence non-decode) and `last_frame_s` (the last frame the receiver found), `peer_snr_db` (what the other station reports hearing this one at, from its acknowledgements), `rate_snr_db` and `margin_db` (the rate controller's smoothed reading and the margin it keeps), `throughput_bps` (application bytes both ways over the last 30 s), `link` (§4.8) |
| `frame` | every frame the receiver finds, decoded or not | `t_s`, `kind` (`data`, `control`, `beacon`, `connect`, `answer`, `probe`, `probe-answer`), `mode`, `rv`, `snr_db`, `cfo_hz` (null for a low-confidence non-decode — the correlator on noise, not a real offset), `confidence` (the mode read off the pilot chips, which only a DATA frame carries — a CONTROL frame always reports 1.0), `detect_confidence` (how far above its acceptance threshold acquisition saw the preamble, 1.0 being exactly at it: defined for **every** frame type, so this is what tells a real connect, poll or acknowledgement from a noise trigger), `decoded`, `bytes`, `from` and `to` (the callsigns, when the frame carries them or the session implies them), `control` (a control frame's fields spelled out) |
| `heard` | a station was heard | the entry as `heard.list` reports it |
| `profile` | the settings, the dials or the profiles changed | what `profile.list` answers: `active`, `name`, `dirty`, `profiles` — so a panel's mark by the profile's name is never stale, whichever client made the change |
| `data` | payload received | data (base64) |
| `ptt` | transmit starts or stops | on |
| `busy` | channel busy detector changes | busy |
| `device` | a device appears or disappears | kind, name, present |
| `log` | notable events | level, message. A Test session reports every step as a `log` event named `test`: the probe's answer, `connected`, each transfer's bytes and seconds, each rung as `rung mode <m>: <decoded>/<frames> decoded at <snr> dB`, and `complete` or `aborted: <why>` |

`metrics` is the operator's window into the link. `snr_db` is referenced to 3 kHz, like every
SNR in this project; `mode` is the index into the table returned by `capabilities`. Every
reading is the modem's own — the receiver's per-frame estimates, the rate controller's
state, the acknowledgements' reports — never something a client re-derived from audio.

The two displays that would be high-rate streams — the spectrum and the constellation — are
methods to poll (`spectrum`, `constellation`, §4.3) rather than events to subscribe to: a
client that draws them asks at the rate it draws, and a client that does not never pays
for them, with no subscription state for the daemon to keep. (An earlier draft of this
section promised a `subscribe` method; polling turned out to need nothing it would have
added.)

### 4.8 The session's account

`link` is null while no session is up, and otherwise `started_s` (station time), `seconds`
(how long it has been up), `remote`, `bytes_sent` and `bytes_received` — application bytes
before compression and after decompression, which is what the operator handed over and got,
not what went on the air. It is carried by `status` and by every `metrics` event.

---

## 6. State machine

```
        idle ──connect──► connecting ──► connected ──disconnect──► disconnecting ──► idle
         ▲                    │              │                                         ▲
         └────── failed ──────┘              └──────────── abort / link loss ───────────┘
```

`connected` additionally carries a role — sending or receiving — which changes without
leaving the state. A client should render the role, because on a half-duplex link it explains
why the modem is not currently transmitting the data it was given.

---

### 4.5 Changing settings

`config.set` takes dotted keys — `{"radio.max_mode": 8, "audio.input": "USB Audio CODEC"}` —
and answers with `changed` and `restart_required`.

Three rules, because a settings interface that gets any of them wrong is worse than none:

* **A refused change changes nothing.** The merge happens on a copy, the result is validated,
  and only then does it replace what is running. A half-applied configuration would leave a
  station in a state its operator never chose.
* **The file is replaced atomically** — written beside the target and renamed over it. A
  configuration half-written by a machine that lost power is a station that will not start,
  and its operator would have no way to know what it used to say.
* **A change of `ptt.kind` takes the old kind's fields with it.** `port` and `line` belong to
  a serial port, `address` to `rigctld`; a client cannot remove a key, only say which kind it
  wants now, so the merge drops what the new kind has no use for.
* **What needs a restart is stated, not guessed — and done, when somebody can.** A sound card
  is opened once and a socket is bound once. `config.get` returns `live_keys`, and `config.set`
  reports which of the keys it just changed are not among them. A setting that silently does
  nothing until the next restart is worse than one that says so. When `status.supervised` is
  true a client may then ask `shutdown {"restart": true}` and the daemon is back on the new
  file in a few seconds; the panel does exactly that, and from a terminal, where nobody would
  start it again, it says what to restart instead.

### 4.9 Profiles

A profile is the station's settings as one portable file: everything the settings registry
does not mark as this computer's, under a name, with the dial memories beside it. The
daemon keeps them in `profiles/` beside the configuration, one `<name>.aetherprofile`
each (JSON), and `profiles.json` says which is active. The configuration file stays what
the daemon runs from; a profile is applied *to* it. `docs/adr/0011-profiles.md` is the
design.

| Method | Params | Result |
|---|---|---|
| `profile.list` | — | `active` (the id, or null), `name`, `dirty` (whether the running settings have moved from what the active profile says — null with no active profile), `profiles` (`id`, `name`, `created`, `modified`, `aether_version`, `path`, and `error` for a file that cannot be read), `dir`, `extension` |
| `profile.save` | `name?` | the running settings written to the active profile; with `name`, to a new profile that becomes active (`conflict` if the name is taken, compared without regard to case). Answers as `profile.list`, plus `saved` |
| `profile.load` | `id` | the profile applied: the file written, live keys taken on at once, the profile made active. Answers as `profile.list`, plus `loaded` and a `report`: `changed`, `restart_required`, `unknown` (settings this version does not have — a newer version's, or a typo — left out), `ignored` (this computer's own settings the file carried, kept as they were), `invalid` (values that failed their rule, each with `key`, `value`, `reason`; the default was kept), `missing_hardware` (devices this computer does not report: `key`, `name`, and `suggestion` — the one device with the same description behind it, when there is exactly one; offered, never chosen). `refused` when the settings will not work *together* — nothing is changed then |
| `profile.create` | `name` | a new profile of the defaults, keeping the callsign and the operator's details, written and loaded; answers as `profile.load` |
| `profile.rename` | `id`, `name` | the file moves with the name, and so does the active mark |
| `profile.duplicate` | `id`, `name` | a copy under the new name, not made active |
| `profile.delete` | `id` | `refused` for the active profile: switch first |
| `profile.export` | `id?`, `name?` | `text` (the file's contents), `filename`, `path`; without an id, the running settings as a profile would hold them, under `name` or the callsign |
| `profile.import` | `text`, `name?`, `replace?` | the file parsed, brought forward, checked against the running settings and this computer's devices, and written to the store — **not loaded**: the answer carries `imported` and the same `report` a load would give, so a client can say what it found before switching. `bad_params` for a file that is not a profile or was written by a newer version; `conflict` for a name in use unless `replace` is true |

Four rules, the ones `config.set` keeps and one more:

* **What a profile holds is decided in one place.** The settings registry
  (`core/aetherd/src/settings.rs`) is derived from the configuration's own structure —
  every leaf of it — and a short table of rules says which are *portable*, which name
  *hardware* (portable, but checked against the machine on load), which are this
  *machine*'s (`control.*`, `log.file`, `record.dir`, `sim.*`) and which are *secret*
  (`control.token`). A setting added to the daemon is in the next profile saved without
  anyone listing it; only a setting that must *not* travel needs a rule.
* **A load is transactional.** The profile is applied to a copy of the running
  configuration, one setting at a time, and the whole is validated as the file would be;
  only then is anything written or taken on. A single bad value is reported and its
  default kept, so the rest of the profile still loads; a rule between two settings that
  fails refuses the whole load, and the station runs on as it was.
* **A profile means the same station wherever it is loaded.** A portable setting the
  profile does not name gets its default, not the running value — otherwise switching
  profiles would carry settings from one to the next. This computer's own settings are
  kept from the running configuration, whatever the file says.
* **A device the computer does not have is named, not replaced.** The profile's device
  names are applied as they are and reported; the daemon runs receive-only or silent on a
  device it cannot open, as it always has, and says so. A serial port with the same
  description behind it is *suggested* when there is exactly one.

The first start after the upgrade that brought profiles adopts the running configuration
as the profile *Default* — nothing in the file changes — so every existing station has a
profile from the first day. `dirty` is computed, not tracked: it is whether loading the
active profile again would change anything, so it is right after any client's change.

### 4.6 The diagnostic bundle

`diagnostics` answers the questions a maintainer asks first — what version, on what, with
what settings, doing what — in one object, so a panel can offer a single "copy" button and an
operator can paste the result into an issue from wherever they are:

| Key | Contents |
|---|---|
| `version`, `platform` | the daemon's version; `os` and `arch` |
| `generated`, `started` | RFC 3339 UTC timestamps for the bundle and for the daemon's start |
| `config`, `path` | the running configuration (secrets redacted) and the file it came from |
| `status`, `capabilities` | as the methods of the same names return them |
| `devices` | the audio devices and serial ports the machine reports |
| `audio` | how the sound card described itself; `dropped_samples`, captured samples the modem has dropped for falling behind; `starved_samples`, samples of silence the card had to play *inside a transmission* because the modem had not handed it the next ones — holes on the air, which the log also reports as they happen |
| `loop` | the run loop's slowest pass so far (`slowest_ms`), which phase it spent the time in (`slowest_phase`: `commands`, `capture` — the receiver — or `playback`), and `stalls`, how many passes exceeded a quarter second. A whole burst is queued at the sound card the moment it is rendered (ADR-0010), so a slow pass no longer puts a hole in a transmission; it still says the modem is slow, which is the receiver's cost per block |
| `log`, `log_forgotten` | the most recent log entries (§4.7), oldest first, and how many older ones have scrolled off |

It contains no traffic: a `send` is logged with the *length* of its payload, never the bytes.

### 4.7 The log

Every line the daemon writes carries a UTC timestamp with milliseconds, a level (`info`,
`warn`, `error`), an event name a machine can group on, the detail a person reads, and the
modem's state at the time. On a terminal it looks like

```
2026-09-14T11:11:25.900Z info  ptt: keyed (Idle)
```

and with `[log] format = "json"` in the configuration each line is one object with the keys
`ts`, `level`, `event`, `detail` and `state`, for a journal or a log shipper. `[log] file`
appends the same lines to a file — a packaged desktop daemon has no terminal, so without it
nothing the daemon says survives the session. The last `[log] keep` entries (default 500)
are held in memory for `diagnostics`.

What is logged: the daemon's start and stop; what the audio and keying are running on; every
*mutating* control request with its outcome (reads are not logged — a panel polls, and the
ring would hold nothing else); every keying and release of the transmitter; every session
event the modem reports; dropped audio; and anything that failed.

---

## 7. Open items for v1.0

* `config.set` cannot yet reopen a sound card or rebind a socket; those keys are written and
  reported as needing a restart.
* Whether `metrics` should be pull as well as push for scripted use.
* A capability flag for the FM PHY's differences.
* Rate limiting and back-pressure rules for `send` on a slow link.
