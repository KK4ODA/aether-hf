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
| `status` | — | state, role, remote callsign, uptime, versions, capabilities |
| `config.get` | — | the configuration, the file it came from, and which keys apply without a restart |
| `config.set` | dotted key/value pairs | which keys changed, and which of them need a restart |
| `capabilities` | — | bandwidths, mode table, whether the PHY reports preambles |
| `diagnostics` | — | everything a bug report needs, in one object (§4.5) |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic.

`config.get` and `diagnostics` return the configuration **with the secrets taken out**:
`control.token` comes back as the string `<set>` when one is configured. A loopback client
needs no token, so it must not be able to read the one that guards a network bind.

### 4.2 Session

| Method | Params | Result |
|---|---|---|
| `connect` | `remote`, `bandwidth?` | session id; then `state` events |
| `disconnect` | — | accepted; closes after the queue drains |
| `abort` | — | accepted; drops the session immediately |
| `listen` | `enabled` | accepted |
| `send` | `data` (base64) | bytes accepted into the queue |

`disconnect` is orderly: queued data is sent and acknowledged first. `abort` is not. The
distinction matters to an operator watching a transfer and is why both exist.

### 4.3 Devices and calibration

| Method | Params | Result |
|---|---|---|
| `devices.list` | — | input/output devices with names and default flags, and serial ports |
| `ptt.test` | `duration_s` (0.2–5) | keys the radio with no audio for that long, so the operator can watch the rig and the interface's PTT light |
| `tune` | `duration_s` (0.5–10) | keys and plays a steady tone at the configured transmit level, for setting drive by the rig's ALC |
| `audio.level` | — | the last three seconds of received audio: RMS and peak in dBFS, clipping fraction, and a sentence of advice |

These exist because setup, not propagation, is what defeats most new users of an HF data mode
(`COMMUNITY-CONCERNS.md`). A modem that can key on demand and say whether its input is
clipping can lead the operator through setup instead of leaving them to guess.

`ptt.test` and `tune` are transmissions: they are refused during a session, held back by the
busy detector like any other, and counted against the key-time watchdog. Neither can measure
whether the *radio* keyed — only the operator can see that — which is why they exist: to let
the operator look. `audio.level` is always on; it reports `settled: false` and
"Still listening." until it has heard enough to mean anything, rather than a number that
does not.

---

## 5. Events

| Event | When | Key fields |
|---|---|---|
| `state` | session state changes | state, role, remote |
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

### 4.4 Changing settings

`config.set` takes dotted keys — `{"radio.max_mode": 8, "audio.input": "USB Audio CODEC"}` —
and answers with `changed` and `restart_required`.

Three rules, because a settings interface that gets any of them wrong is worse than none:

* **A refused change changes nothing.** The merge happens on a copy, the result is validated,
  and only then does it replace what is running. A half-applied configuration would leave a
  station in a state its operator never chose.
* **The file is replaced atomically** — written beside the target and renamed over it. A
  configuration half-written by a machine that lost power is a station that will not start,
  and its operator would have no way to know what it used to say.
* **What needs a restart is stated, not guessed.** A sound card is opened once and a socket is
  bound once. `config.get` returns `live_keys`, and `config.set` reports which of the keys it
  just changed are not among them. A setting that silently does nothing until the next restart
  is worse than one that says so.

### 4.5 The diagnostic bundle

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
| `log`, `log_forgotten` | the most recent log entries (§4.6), oldest first, and how many older ones have scrolled off |

It contains no traffic: a `send` is logged with the *length* of its payload, never the bytes.

### 4.6 The log

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
