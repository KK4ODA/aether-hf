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
| `status` | — | state, role, callsign and callsigns, remote callsign, uptime, versions, capabilities, `supervised` (whether somebody will start the daemon again if it asks) |
| `config.get` | — | the configuration, the file it came from, and which keys apply without a restart |
| `config.set` | dotted key/value pairs | which keys changed, and which of them need a restart |
| `capabilities` | — | bandwidths, mode table, whether the PHY reports preambles |
| `diagnostics` | — | everything a bug report needs, in one object (§4.6) |
| `shutdown` | `restart?` | `stopping`; the transmitter is released on the way out. With `restart: true` the daemon exits with status 75 (`EX_TEMPFAIL`), which the desktop shell and the systemd unit (`RestartForceExitStatus=75`) take as "start me again" — the way a setting that needs a restart is applied without the operator having to know |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic.

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
| `listen` | `enabled` | accepted |
| `send` | `data` (base64) | bytes accepted into the queue |

`disconnect` is orderly: queued data is sent and acknowledged first. `abort` is not. The
distinction matters to an operator watching a transfer and is why both exist.

### 4.3 Devices and calibration

| Method | Params | Result |
|---|---|---|
| `devices.list` | — | input/output devices with names and default flags, and serial ports as `{name, description}` — the description is what the driver says is behind the port, which is how an operator tells a radio's CAT port from the one that keys |
| `ptt.test` | `duration_s` (0.2–5) | keys the radio with no audio for that long, so the operator can watch the rig and the interface's PTT light |
| `tune` | `duration_s` (0.5–10) | keys and plays a steady tone at the configured transmit level, for setting drive by the rig's ALC |
| `audio.level` | — | the last three seconds of received audio: RMS and peak in dBFS, clipping fraction, and a sentence of advice |

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

### 4.4 Recording

| Method | Params | Result |
|---|---|---|
| `record.start` | `name` (optional), `notes` (optional) | `path` of the WAV being written |
| `record.stop` | — | `wav`, `sidecar`, `seconds`, `frames` found, `decoded` |
| `record.notes` | `notes` | accepted; kept for the next recording that starts on its own |

A recording is a mono 16-bit WAV at the modem's 48 kHz of everything the sound card
delivered, and a JSON sidecar of what the modem made of it: every frame the receiver found
(`t_s`, `kind`, `mode`, `rv`, `snr_3k_db`, `cfo_hz`, `decoded`, `bytes`), every event with
the modem's state, when the transmitter was keyed and released, the counters at the end,
and the `notes`. Times are seconds from the start of the file by the station's audio clock.
The sidecar's `format` is `aether-hf-session/1`. `status` carries `recording` — the path and
length so far — while one runs. With `[record] auto = true` every session records itself
from connect to disconnect, one file each, named `YYYYMMDD-HHMMSS_<mycall>_<remote>`.

`aetherd --replay <wav>` runs a recording back through the receiver and, with the sidecar
beside it, fails if fewer frames decode than did on the day; `field/` is where the ones worth
keeping live, and `core/aetherd/tests/field.rs` replays them all on every test run.

---

## 5. Events

| Event | When | Key fields |
|---|---|---|
| `state` | session state changes | state, role, remote, callsign (the one this session runs under: a station that answers to several is addressed by whichever was called) |
| `metrics` | periodically while active | snr_db, cfo_hz, mode, throughput_bps, queued_bytes, retries |
| `data` | payload received | data (base64) |
| `ptt` | transmit starts or stops | on |
| `busy` | channel busy detector changes | busy |
| `device` | a device appears or disappears | kind, name, present |
| `log` | notable events | level, message |

`metrics` is the operator's window into the link. `snr_db` is referenced to 3 kHz, like every
SNR in this project; `mode` is the index into the table returned by `capabilities`.

Optional high-rate streams — constellation points and spectrum — are subscribed to explicitly
(`subscribe` with a stream name and a rate limit) so a client that does not display them
never pays for them.

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
