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
| `status` | — | state, role, callsign and callsigns, remote callsign, uptime, versions, capabilities, `supervised` (whether somebody will start the daemon again if it asks), `binary` (the executable it runs from — how the desktop shell tells a daemon of its own installation from somebody else's), `frequency_hz` (the dial, when the keying interface can ask the radio), `link` (the session's account, §4.8), `host` (`enabled`, the command and data addresses, and `connected`: whether a host program holds the port right now), and `metrics` and `counters` as the event and the sidecar carry them |
| `config.get` | — | the configuration, the file it came from, and which keys apply without a restart |
| `config.set` | dotted key/value pairs | which keys changed, and which of them need a restart |
| `capabilities` | — | `bandwidth_hz` (the waveform the station runs: 2300 or 500), `bandwidths_hz` (what this version has), the mode table of the running waveform (`modes`: index, name, payload bytes and net bit rate *on the layout the mode goes out on*, AWGN threshold, `floor` — whether the mode rides the floor frame family of ADR-0009, four times as long as an ordinary frame; `usable_modes`), whether the PHY reports preambles, the SNR reference |
| `diagnostics` | — | everything a bug report needs, in one object (§4.6) |
| `shutdown` | `restart?` | `stopping`; the transmitter is released on the way out. With `restart: true` the daemon exits with status 75 (`EX_TEMPFAIL`), which the desktop shell and the systemd unit (`RestartForceExitStatus=75`) take as "start me again" — the way a setting that needs a restart is applied without the operator having to know |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic. The table depends on the bandwidth: fourteen modes
from BPSK ⅕ at 2 300 Hz, ten from QPSK ½ at 500 Hz, and a mode index means nothing without
the `bandwidth_hz` it came with. `[radio] bandwidth` chooses the waveform and needs a
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
| `tune` | `duration_s` (0.5–10, or 0 to stop) | keys and plays a steady tone at the transmit level, for setting drive by the rig's ALC. `audio.tx_level` is live and is applied as audio leaves, so the level can be moved while the tone plays; `0` cuts the tone short, and nothing but a tone is ever cut |
| `audio.level` | — | the last three seconds of received audio: RMS and peak in dBFS, clipping fraction, and a sentence of advice |
| `spectrum` | — | the last window of captured audio transformed: `bin_hz`, `bins_db` (dBFS per bin from 0 Hz to 4 kHz; empty until a window has been heard), `passband_hz` (where this modem's signal sits), `transmitting`. Polled, not streamed: it costs one transform per call and nothing otherwise |
| `constellation` | — | the last frame's equalised symbols as `points` (`[i, q]` pairs, thinned to at most 1024) and the `frame` they came from, as the `frame` event describes it |

These exist because setup, not propagation, is what defeats most new users of an HF data mode
(`COMMUNITY-CONCERNS.md`). A modem that can key on demand and say whether its input is
clipping can lead the operator through setup instead of leaving them to guess.

`ptt.test` keys at once — an SSB transmitter keyed with no audio radiates nothing, and an
operator watching a PTT light cannot be told "accepted" and kept waiting. `tune` is a
transmission: refused during a session and while the channel is busy (refused, not deferred,
because a tone that starts on its own a minute later would surprise the person holding the
drive control). Neither can measure whether the *radio* keyed — only the operator can see
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

A recording is a mono 16-bit WAV at the modem's 48 kHz of everything the sound card
delivered, and a JSON sidecar of what the modem made of it: every frame the receiver found
(`t_s`, `kind`, `mode`, `rv`, `snr_3k_db`, `cfo_hz`, `decoded`, `bytes`), every event with
the modem's state, when the transmitter was keyed and released, the counters at the end,
the `notes`, and `frequency_hz` when the keying backend can ask the rig (`rigctld`; a
keying line cannot, and the field is null rather than a guess). Times are seconds from the
start of the file by the station's audio clock.
The sidecar's `format` is `aether-hf-session/1`. `status` carries `recording` — the path and
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
| `metrics` | every 500 ms while a client listens | `mode`, `queued_bytes`, `noise_floor_db` and `level_db` (the busy detector's readings, null until it has settled), `channel_busy`, `transmitting`, `receiving` (a burst is arriving), `audio` (as `audio.level`), `snr_db` and `cfo_hz` and `last_frame_s` (the last frame the receiver found), `peer_snr_db` (what the other station reports hearing this one at, from its acknowledgements), `rate_snr_db` and `margin_db` (the rate controller's smoothed reading and the margin it keeps), `throughput_bps` (application bytes both ways over the last 30 s), `link` (§4.8) |
| `frame` | every frame the receiver finds, decoded or not | `t_s`, `kind` (`data`, `control`, `beacon`, `connect`, `answer`, `probe`, `probe-answer`), `mode`, `rv`, `snr_db`, `cfo_hz`, `confidence`, `decoded`, `bytes`, `from` and `to` (the callsigns, when the frame carries them or the session implies them), `control` (a control frame's fields spelled out) |
| `heard` | a station was heard | the entry as `heard.list` reports it |
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
| `audio` | how the sound card described itself, and how many captured samples the modem has dropped |
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
