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
| `config.get` | `keys?` | the current configuration |
| `config.set` | key/value pairs | the applied configuration; rejects unknown keys |
| `capabilities` | — | bandwidths, mode table, whether the PHY reports preambles |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic.

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
| `devices.list` | — | input/output devices with names and default flags |
| `ptt.test` | `duration_s` | measured key-up and key-down delay |
| `audio.calibrate` | `mode` | measured levels, clipping, recommended setting |

These exist because setup, not propagation, is what defeats most new users of an HF data mode
(`COMMUNITY-CONCERNS.md`). A modem that can measure its own PTT delay and audio level can
lead the operator through setup instead of leaving them to guess.

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

## 7. Open items for v1.0

* Exact `config` key set, once the daemon exists to have opinions about it.
* Whether `metrics` should be pull as well as push for scripted use.
* A capability flag for the FM PHY's differences.
* Rate limiting and back-pressure rules for `send` on a slow link.
