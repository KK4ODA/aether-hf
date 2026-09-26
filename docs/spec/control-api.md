# Aether control API — specification v0.1

Status: **draft**, and implemented in `core/aetherd/src/control/`. It is the contract the
daemon, the panel, the host adapter, the KISS port and third-party tools are built against,
rather than against each other.

Companion documents: `air-interface.md` (what goes over the air), `host-interfaces.md`
(the VARA-compatible host interface and the KISS port, both clients of this one),
`../adr/0001-language-and-stack.md`.

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
  handful of capability fields. Nothing in the shape of a method or an event names a
  modulation; where this document names one it is describing today's HF ladder, which a
  client reads from `capabilities`.
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
| Panel | `GET /` and the files under it, from `[control] ui_dir` when it is set |
| HTTP status | 200 for `ok: true`, 400 for a refused request (the same error object), 401 without a token that is needed, 404 for anything but `POST /v1/<method>` or a panel file |

**Authentication.** No token is needed on a loopback bind, and a connection from the
loopback address never needs one. Binding to any other interface requires `[control] token`
in the configuration — the daemon refuses a non-loopback bind without one rather than
starting an open one — and a connection from another address must present it as
`Authorization: Bearer <token>`, on the WebSocket upgrade request or on each `POST`; without
it the answer is HTTP 401, code `unauthorised`. The operator chooses the token, and
`config.get` shows it only as `<set>`.

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
{ "id": "7", "ok": false, "error": { "code": "not_idle",
  "message": "Cannot probe KK4XYZ: a probe is already out. The answer, or its absence, is reported as a probe event.",
  "retryable": true } }
```

Events are unsolicited and carry no `id`:

```json
{ "event": "state", "data": { "name": "connected", "detail": "KK4XYZ (iss)",
  "state": "Connected", "callsign": "W4ODA", "remote": "KK4XYZ" } }
```

A request the regulatory policy refuses — a `connect`, `probe`, `beacon`, `tune`, `drive.set` or
`ptt.test` the rules do not allow here — fails with code `regulatory`, a message that says why in a
sentence, and the whole decision in `result.decision` (§4.10). A `test.start` the rules refuse
fails with code `not_idle` and the decision's summary as its message.

**Error messages are written for operators, not developers.** `"The sound device 'USB Audio
CODEC' was unplugged. Reconnect it or choose another in Setup, step 2."` — not a stack
trace, and not `ENODEV`. The `code` is the stable machine-readable field; `message` is
human-facing and may be localised.

---

## 4. Methods

### 4.1 Status and configuration

| Method | Params | Result |
|---|---|---|
| `status` | — | `state`, `role` (`iss`, `irs`, `none`), `callsign` and `callsigns`, `remote`, `session` (its id), `mode` (the rung in use), `uptime_s`, `version`, `transmitting`, `channel_busy`, `queued_bytes`, `compressing` and `compression_saving` (whether this session compresses, and the fraction it saves), `ptt` (what the radio is keyed through) and `ptt_fault` (why it cannot key, when the keying interface would not open), `audio_fault` (why the sound card would not open; the station runs on silence meanwhile), `recording` (§4.4), `recordings_dir` (where recordings go, whether or not one runs), `test` (§4.2; null when none runs), `sent` (`pending`, `recent`: §5), `probing` (a probe is out and its answer awaited — the state stays `idle`), `closing` (a `disconnect` was asked for and the DISC has not gone yet: the sender is finishing what is queued, or a receiver waits for the burst arriving to end — it sends its DISC at once otherwise; `abort` closes now), `supervised` (whether somebody will start the daemon again if it asks), `binary` (the executable it runs from — how the desktop shell tells a daemon of its own installation from somebody else's), `config_note` (set when the configuration file was written by a newer version and this one started from the copy kept before that version brought it forward, saying which, and where the newer file is kept), `frequency_hz` (the dial, when the keying interface can ask the radio), `can_tune` (whether `frequency.set` has a way to: CAT or `rigctld`), `link` (the session's account, §4.8), `host` (`enabled`, the command and data addresses, `connected`: whether a host program holds the port right now, and `chat`: whether it said `CHAT ON`), `kiss` (the KISS port, §4.11), `datagrams` (`queued` of `limit`, `sent`, `heard`, `incomplete_dropped`: §4.11), `regulatory` (where the station stands with the rules, §4.10), `beacon` (`every_s`: the repeating beacon's interval, null when none runs; `next_in_s`; `last_sent_ms`: when the last beacon went on the air, repeating or not; `sent` and `skipped` since the daemon started), `identifier` (the Morse identifier: `enabled`, `wpm` as set, `sent_wpm` — the speed it goes at, the rules' limit on an automatically keyed identifier when that is lower, 20 wpm under §97.119(b)(1) — `max_wpm` (the rules' limit, null when they set none) and `interval_s`; a `log` event named `identifier` says so when the rules hold it below the speed set), and `metrics` and `counters` as the event and the sidecar carry them — `counters`: the link's `frames_sent`, `frames_resent`, `frames_received`, `frames_failed`, `harq_rescues`, `bytes_delivered`, `bursts`, `turns`, `ack_timeouts`, `probes_sent`, `probes_answered`, `probe_replies`, `frames_reencoded`, and the station's `transmissions`, `frames_detected`, `deferred_for_busy`, `watchdog_trips`, `beacons_sent` (beacons queued, by hand or on the timer) and `beacons_heard`, since the daemon started or the last `counters.reset`. `metrics.tx_peak_dbfs` is the largest sample the modem handed the sound card on its last transmission, after the transmit level: the headroom figure no ALC meter can show, because it is measured before the radio |
| `counters.reset` | — | `counters`, every one at zero. Only the tallies are cleared: the session, its account and the settings are untouched |
| `config.get` | — | the configuration, the file it came from, and which keys apply without a restart |
| `config.set` | dotted key/value pairs | which keys changed, and which of them need a restart |
| `config.schema` | — | the settings registry (§4.9): every setting with its `key`, `type`, `default`, `nullable`, `scope`, `live`, and its `min`/`max`/`options` and `why` where it has a bound; `live_keys`; the profile format's name and both schema numbers |
| `capabilities` | — | `bandwidth_hz` (the waveform the station runs: 2300 or 500), `bandwidths_hz` (what this version has), the ladder of the running waveform (`modes`, one entry a rung: index, name, payload bytes and net bit rate *of the frame the rung goes out on*, AWGN threshold, `floor` — whether the rung is the tone floor's (ADR-0013; its fast kinds, ADR-0014, and the 500 Hz four-tone kinds, ADR-0015, included), whose frames are five times as long as an ordinary one; `usable_modes`), whether the PHY reports preambles, the SNR reference |
| `diagnostics` | — | everything a bug report needs, in one object (§4.6) |
| `shutdown` | `restart?` | `stopping`, `restart`, `supervised`. A session is ended on the air first — its DISC and, when the station identifies, the identifier — and the daemon waits until nothing is left to send or twelve seconds pass (ADR-0017); then the transmitter is released. With `restart: true` the daemon exits with status 75 (`EX_TEMPFAIL`), which the desktop shell and the systemd unit (`RestartForceExitStatus=75`) take as "start me again" — the way a setting that needs a restart is applied without the operator having to know. The reply is written before the daemon exits, and so is every other reply already on its way (for up to two seconds); a request that reaches it while it stops is answered `modem_stopped` |

`capabilities` is how a client discovers the mode table rather than hard-coding it, and is
what keeps this document PHY-agnostic. A mode number is a rung of the waveform's **ladder**
(ADR-0013, ADR-0014, ADR-0015): the tone floor's kinds, then the OFDM modes — twenty rungs
at 2 300 Hz (the floor's two and its four fast kinds at rungs 0–5, the fourteen OFDM modes
from BPSK ⅕ at rungs 6–19), fifteen at 500 Hz (the floor's two and its two four-tone middle
kinds at rungs 0–3, then QPSK ⅓ up at rungs 4–14) — and a
mode number means nothing without the `bandwidth_hz` it came with. `[radio] bandwidth` chooses the waveform and needs a
restart; `[radio] answer_only` (live) makes the station take calls and make none: `connect`,
`beacon`, `beacon.every`, `probe`, `test.start` and `datagram.send` are refused while it is set,
and probes are still answered. It does not decide what the rules allow — `[regulatory] control`
does (§4.10) — and under the US profile an automatically controlled station keeps to the
§97.221(b) segments whatever its bandwidth, since no Aether emission is 500 Hz or less by the
wider reading of §97.3(a)(8) (ADR-0018).

`config.get` and `diagnostics` return the configuration **with the secrets taken out**:
`control.token` comes back as the string `<set>` when one is configured. A loopback client
needs no token, so it must not be able to read the one that guards a network bind.

### 4.2 Session

| Method | Params | Result |
|---|---|---|
| `connect` | `remote`, `callsign?` | `session` (the id); then `state` events. `callsign` picks which of the station's callsigns to call as (the first, when absent). The bandwidth is the station's (`[radio] bandwidth`), never a call's. Refused `bad_params` (no `remote`; a `callsign` not the station's), `already_connected` (a session is up — or the station is answer-only), `regulatory` |
| `callsigns.set` | `callsigns` (list) | the callsigns the station answers to from now on, the first being the one it calls as, and `applied`: `false` when a session is up, in which case they take effect as it ends. Replaces the configuration's `callsign` for the daemon's lifetime without touching the file: a host program's `MYCALL` is the operator's callsign, and the file is what the station answers to until one says otherwise |
| `disconnect` | — | accepted (`orderly: true`). A sending station sends what is queued, has it acknowledged, then its DISC; a receiving station sends its DISC at once, or — when a burst is arriving — in place of that burst's acknowledgement (ADR-0023). `status.closing` is true until the DISC is queued. The DISC and the other station's answer wait out a Morse identifier being sent (at most 15 s, ADR-0022). During a call it does not stop calling — a call that is answered is then closed at once; `abort` stops calling |
| `abort` | — | accepted (`orderly: false`); ends the session now: the rest of a burst on the air is cut, the card flushed, and one DISC follows after a pause for the cut frame and the other station's answer to pass (ADR-0017). During a call it stops calling and sends nothing |
| `beacon` | — | accepted; one frame with this station's callsign, addressed to nobody, on the tone floor — the most robust frame there is, heard by a station of either bandwidth (ADR-0016). Refused during a session and on an answer-only station. When it goes on the air a `log` event named `beacon` says `sent` (with the repeating beacon's next, when one runs); a beacon heard from another station is a `log` event named `beacon`, `heard <call> at <snr> dB` |
| `beacon.every` | `minutes` (10–240; `0` or `null` stops it) | `status.beacon` as it now stands. A beacon now and another every `minutes` until stopped or the daemon restarts — nothing on the disk sets a station beaconing on its own. Each waits for the station to be idle (no session, probe or test) and the channel clear, whatever `wait_for_clear` says; one that could not go within two minutes of falling due is skipped and said so (`beacon` log event: `skipped: …`), the next kept to the interval. Refused (`bad_params`) outside the range, on an answer-only station, and under automatic control: a beacon may be automatically controlled only on 28.20–28.30, 50.06–50.08, 144.275–144.300, 222.05–222.06 or 432.300–432.400 MHz, or on 33 cm and up (§97.203(d)); a station that becomes either while one runs stops it at the next one due |
| `probe` | `remote`, `callsign?` | accepted; one frame on the tone floor asking `remote` whether it hears this station, and how well (ADR-0006, ADR-0016). The answer, or its absence, arrives as a `log` event named `probe`: `<call> hears us at <x> dB, heard at <y> dB` — the SNR the other station measured on the probe, and the SNR this one measured on the answer; tone-floor readings, exact to about +10 dB and a lower bound above (at most 17.5 dB on a clean path, and lower on a dispersive one: 15.5, 12 and 5 dB on ITU Good, Moderate and Poor) — or `<call>: no answer` after one frame's turnaround. One probe out at a time (`not_idle`, retryable); refused during a session and on an answer-only station, which answers probes and sends none. The other end reports a probe it answered as a `log` event named `probed` |
| `test.start` | `remote`, `callsign?`, `remote_grid?`, `message_bytes?` (2048), `file_bytes?` (16384), `ladder?` (true), `rung_frames?` (4), `budget_s?` (600) | accepted; the **Test session** of P6-7 with `remote`: a probe, a call, the message (incompressible bytes, timed), the **mode ladder** — a burst pinned at each mode from the floor up, its frames small enough to be re-encoded at a slower mode if that mode fails, until three rungs in a row decode fewer than half their frames — then the file, and an orderly disconnect (ADR-0017: the ladder comes before the file, which used to take the whole budget on a slow path). The sizes are ceilings: the SNR the other station reports hearing this one at in the probe's answer (this station's own reading of it when the answer did not say) sizes the message under its ceiling — half a kilobyte below 0 dB, a kilobyte below 6 dB — and the message's measured rate sizes the file to about two minutes' worth of what the budget has left; the whole run keeps to `budget_s`, shortening or skipping what would not fit (`results.adjustments` says what), and a transfer or rung that stalls past its time ends the run with an abort, keeping what was learned. All of it is one recording named `…_test`, with the report under the sidecar's `session.test` and the operator's `[operator]` grid, rig, power and antenna beside it. Progress arrives as `log` events named `test`, and while it runs `status.test` gives `remote`, `step` (`probe`, `connect`, `message`, `ladder`, `file`, `disconnect`), `elapsed_s`, `budget_s` and `remaining_s` (the most the budget leaves — not a forecast), `rungs` (tried so far), `transfer` during the message and the file (`bytes`, `acked`), `ladder` (`total` rungs, `done`, `testing` and `testing_name` — the rung under test — `frames` per rung, `highest_passed` and `highest_passed_name`, `failures_in_row` of `failures_allowed`, and `last`, the last rung as `results.ladder` has it) and `link` (the `rung` in use and its `rung_name`, and `heard_there_db`, how the other station last said it heard this one). The other station only listens; an answer-only station is a fine partner. Refused (`not_idle`) during a session, a probe or another test, on an answer-only station, and with the decision's summary when the rules do not allow this station to start an exchange here |
| `test.status` | — | `running`, and `results` — the running test's, or the last one's until the next starts: `remote`, `started`, `elapsed_s`, `step`, `outcome` (`complete`, or `aborted: <why>`), `bandwidth_hz`, `probe` (`heard_there_db`, `heard_here_db`; null when unanswered), `message` and `file` (`bytes`, `seconds`, `bps`), `ladder` (per rung: `mode`, `frames`, `decoded`, `snr_db` as the other station measured it — null when none of the rung's frames was one it trusted (ADR-0020) — `seconds`), `ladder_rungs` (how many the ladder had to climb), `highest_passed` (the fastest rung that decoded at least half its frames; null when none did, or the ladder never ran), `path` (`my_grid`, `their_grid`, `km` from the two grids) |
| `test.abort` | — | `aborted`: whether one was running. The session is aborted at once (one DISC on the way out), the outcome is `stopped by the operator`, and the report keeps every step that finished |
| `listen` | `enabled` | `{"enabled": true}` for true; `false` is refused `unsupported`: this version always answers a call to its callsigns |
| `send` | `data` (base64), `ref?` (1–64 printable ASCII characters, no spaces) | `accepted`: bytes accepted into the queue, and `ref` when one was given. A message sent with a reference is followed: a `sent` event says when the other station has all of it, in order — from the link's acknowledgements, so after every byte before it too — or that the session ended first. Refused (`not_connected`) with no session up; a bad reference is `bad_params` |

`disconnect` is orderly: what this station has queued is sent and acknowledged first.
`abort` is not. The distinction matters to an operator watching a transfer and is why both
exist.

### 4.3 Devices and calibration

| Method | Params | Result |
|---|---|---|
| `devices.list` | — | `devices`: each audio device's `name`, whether it can capture (`input`) and play (`output`), and the sample rates it runs at (`input_rates`, `output_rates`); `serial_ports` as `{name, description}` — the description is what the driver says is behind the port, which is how an operator tells a radio's CAT port from the one that keys — and `gpio_interfaces` as `{path, name}`: the CM108-class interfaces that key through their codec's GPIO pin (`[ptt] kind = "cm108"`). Refused `audio_unavailable` when the platform cannot list its audio devices |
| `ptt.test` | `duration_s` (0.2–5) | keys the radio with no audio for that long, so the operator can watch the rig and the interface's PTT light. Refused during a session |
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
transmissions: refused during a session, by the rules (`regulatory`), and — with
`[radio] wait_for_clear`, the default — while the channel is busy or the busy detector is still
learning the noise floor (refused, not deferred, because a transmission that starts on its own
a minute later would surprise the person holding the drive control). Neither can measure whether the *radio* keyed — only the operator can see
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
| `heard.list` | — | `stations`: every station heard, most recent first — `callsign`, `first_heard_ms` and `last_heard_ms` (Unix milliseconds), `count`, `snr_db` (last) and `best_snr_db`, `frequency_hz` (when the radio could say), `mode`, `activity` (`beacon`, `calling`, `probing`, `answering`, `connected`, `datagram`), `detail` (whom it was calling, probing or answering), `connected` (whether a session with it has ever been up from here), `beacons` (how many of its beacons were heard: a station that beaconed and then called is one line whose activity says `calling`, and its beacons are counted here) and `last_beacon_ms`; `limit` (200) and the `path` of the file the list lives in |
| `heard.clear` | — | `cleared`: how many were forgotten |
| `sessions.list` | `remote?` | `sessions`: the session history, newest first, one entry per session — only those with `remote` when it is given: `remote`, `started_ms` and `ended_ms` (Unix milliseconds), `duration_s`, `role` (`caller`: this station called; `called`), `bandwidth_hz`, `frequency_hz` (when the radio could say), `bytes_sent` (handed to the link), `bytes_acked` (acknowledged by the other station, after compression), `bytes_received`, `end` (in the link's words: `closed`, `closed (no disc ack)`, `peer disconnected`, `link timeout`, `no response`, `aborted`), `snr_db` and `best_snr_db` (frames decoded from the other station), `heard_there_db` (how it last said it heard this one), `top_rung_sent` and `top_rung_heard` (the fastest rung of data each way; null when none went), `test` (a Test session's), `recording` (the file name, when it was recorded); `limit` (500) and the `path` of `sessions.json`, beside the configuration. The stations heard keep one entry per callsign; this keeps one per session |
| `sessions.clear` | — | `cleared`: how many sessions were forgotten; the file is written at once. Recordings are not touched |
| `frequencies.list` | — | `memories`: the remembered dials, by frequency — `hz` and `name` (the operator's name or comment) — starting as the frequency plan's proposals; `limit` (200) and the `path` of the file the list lives in (`frequencies.json` beside the configuration) |
| `frequencies.set` | `memories`: `[{hz, name}]` | the list replaced whole, written to its file, and returned as `frequencies.list` would; a frequency given twice keeps its last name, names are trimmed and at most 60 characters, `bad_params` for a frequency no radio has a dial for |
| `frequency.set` | `hz` | `hz`: the radio tuned there, over CAT (`FA`, or CI-V `05`, checked by reading the dial back or by the Icom's acknowledgement) or `rigctld` (`F`); `refused` while transmitting, during a session, or with a keying interface that cannot tune (a serial line, a CM108 codec, none) — `status.can_tune` says which in advance. The next `status` reads the dial from the radio again within two seconds |

A recording is a mono 16-bit WAV at the modem's 48 kHz of everything the sound card
delivered, and a JSON sidecar of what the modem made of it: every frame the receiver found
(`t_s`, `kind`, `mode`, `rv`, `snr_3k_db`, `cfo_hz` — null when the acquisition was a probable noise trigger — `confidence`, `detect_confidence`, `decoded`, `bytes`), every event with
every frame the station **sent** (`sent`: `t_s`, `kind`, `mode`, `rv`, `floor`, `bytes`), every change of the busy state as a `busy` event whose detail gives, in a fixed order, the level, the floor, their difference, the margin, the threshold, the change and why (`-19.8 dBFS | floor -27.2 | delta +7.4 dB | margin 6.0 | threshold -21.2 dBFS | OFF -> ON | energy over the threshold for the attack`; the reason may also be `a frame decoded (acquired at 1.72)` or `the passband is peaked (18.3 dB over its median)`, and `ON -> OFF … | hangover ran out with the energy under the threshold`),
every event with the modem's state, when the transmitter was keyed and released, a
`tx_peak` event after each transmission carrying its peak in dBFS — so a burst nobody
decoded can be read against how hard the transmitter was being driven for it — the counters
at the end,
the `notes`, and `frequency_hz` when the keying backend can ask the rig (CAT or `rigctld`; a
serial keying line or a CM108 codec cannot, and the field is null rather than a guess). Times are seconds from the
start of the file by the station's audio clock.
The sidecar's `format` is `aether-hf-session/4`. `status` carries `recording` — the path and
length so far — while one runs. With `[record] auto = true` every session records itself
from connect to disconnect, one file each, named `YYYYMMDD-HHMMSS_<mycall>_<remote>`.

`aetherd --replay <wav>` runs a recording back through the receiver and, with the sidecar
beside it, fails if fewer frames decode than did on the day; `field/` is where the ones worth
keeping live, and `core/aetherd/tests/field.rs` replays them all on every test run.

The stations heard are every frame that carried a callsign — a beacon, a probe or its
answer, a datagram, a connect request or its answer overheard between any two stations — and
every frame of a session with the station at the other end. One entry per callsign, at most 200, the one heard longest ago
making room for a new one; kept in `heard.json` beside the configuration and written a few
seconds after it changed, so a station left listening overnight can say in the morning who
was on. Each change goes out as a `heard` event.

---

## 5. Events

| Event | When | Key fields |
|---|---|---|
| `state` | a session came up, the turn changed hands, or a session ended | `name` (`connected`, `role`, `disconnected`); `detail` (for `connected` the other station and this one's role, `KK4XYZ (iss)`; for `role` the new role, `iss` or `irs`; for `disconnected` how it ended, in the link's words — `closed`, `peer disconnected`, `no answer`, `aborted`, …); `state` (the modem's state after it, as the log writes it: `Idle`, `Connecting`, `Connected`, `Disconnecting`); `remote`; `callsign` (the one this session runs under: a station that answers to several is addressed by whichever was called). A call being placed is not an event: `status.state` reads `connecting` |
| `metrics` | every 500 ms while a client listens | `mode`, `queued_bytes`, `noise_floor_db` and `level_db` (the busy detector's readings, null until it has settled), `excess_peak_db` (the largest level-over-floor the detector tested since the last reading — the decision is made forty times a second on a 50 ms quantity, so the excursions that cross the threshold are the ones a sampled reading almost never lands on), `shape_db` (the passband's highest spectral bin over its median bin, per 200 ms: flat noise reads about 6 dB, a narrowband signal — FT8, CW, PSK — 15 and up, and a receiver's AGC cannot compress it), `channel_busy`, `busy_reason` (`level` when the threshold last marked it, `shape` when the passband's spectrum did, `frame` when a decoded frame did, null if never — an acquired preamble alone never marks the channel busy: on a real band acquisition confidence overlaps between a phantom and a weak real frame, and only a decode is evidence), `transmitting`, `receiving` (a burst is arriving), `audio` (as `audio.level`), `snr_db` and `cfo_hz` (null when the last frame was a low-confidence non-decode) and `last_frame_s` (the last frame the receiver found), `peer_snr_db` (what the other station reports hearing this one at: from its acknowledgements, and since ADR-0021 from its polls, turns and disconnects too), `rate_snr_db` and `margin_db` (the rate controller's smoothed reading and the margin it keeps), `throughput_bps` (payload bytes the other station acknowledged and payload bytes received from it, as the link carries them — after compression — over the last 30 s, or since the session began when that is shorter), `tx_peak_dbfs` (the last transmission's peak, as `status` describes it), `rx_passband_hz` (the receiver's passband, learned from the noise between signals; null until enough quiet audio has been heard), `occupied_hz` (the audio width the modem's signal needs: a passband materially narrower says the radio's filter is set too narrow), `link` (§4.8) |
| `frame` | every frame the receiver finds, decoded or not | `t_s`, `kind` (`data`, `control`, `beacon`, `connect`, `answer`, `probe`, `probe-answer`, `datagram`), `mode`, `rv`, `snr_db`, `cfo_hz` (null for a low-confidence non-decode — the correlator on noise, not a real offset), `confidence` (the mode read off the pilot chips, which only an OFDM DATA frame carries — a CONTROL frame and a tone-floor frame always report 1.0), `detect_confidence` (how far above its acceptance threshold acquisition saw the preamble, 1.0 being exactly at it: defined for **every** frame type, so this is what tells a real connect, poll or acknowledgement from a noise trigger), `decoded`, `bytes`, `from` and `to` (the callsigns, when the frame carries them or the session implies them), `control` (a control frame's fields spelled out) |
| `heard` | a station was heard | the entry as `heard.list` reports it |
| `sent` | a message sent with a `ref` was settled | `ref`, `bytes`, `delivered` (the other station has all of it), `reason` (how the session ended, when it ended first). `status.sent` has the references still waiting (`pending`) and the last 32 settled (`recent`), for a client that missed the event |
| `session` | a session ended | the entry as `sessions.list` reports it |
| `datagram` | a datagram was heard and joined (§4.11) | `source` (the sending station), `frame_type` (0 AX.25, 1 AX.25 with eight-byte addresses, 2 unformatted), `data` (base64: the frame, byte for byte), `bytes`, `snr_db` and `rung` (of its last piece) |
| `datagram-sent` | a datagram sent with a `ref` has left, or will not | `ref`, `sent` (all of it went out), `reason` (why not: refused by the rules, cut short, dropped when the KISS port closed) |
| `regulatory` | the regulatory gate refused a transmission — or, with `log_permitted`, allowed one an automatically controlled station made (§4.10) | the decision as §4.10 describes it, with `callsign` and `session` (the state it was judged in) |
| `profile` | the settings, the dials or the profiles changed | what `profile.list` answers: `active`, `name`, `dirty`, `profiles` — so a panel's mark by the profile's name is never stale, whichever client made the change |
| `data` | payload received | data (base64) |
| `ptt` | transmit starts or stops | on |
| `log` | anything else the modem reports | `name` (what it is about: `beacon`, `probe`, `probed`, `test`, `identifier`, `busy`, `regulatory`, `recording`, `tune`, `tx`, `watchdog`, `ignored`, `error`, …), `detail` (the sentence), `state`, `callsign`, `remote`. The daemon's own log lines (`control`, `kiss`, `audio`, …) are not events: `diagnostics.log` has them. A Test session reports every step as a `log` event named `test`: the probe's answer, `connected`, each transfer's bytes and seconds, each rung as `rung mode <m>: <decoded>/<frames> decoded at <snr> dB`, and `complete` or `aborted: <why>` |

There is no `busy` event: a change of the busy detector's mind is a `log` event named `busy`,
with the detail §4.4 gives, and `metrics.channel_busy` carries the state. Devices are not
watched; `devices.list` is polled.

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

Four rules, because a settings interface that gets any of them wrong is worse than none:

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
  *machine*'s (`control.*`, `log.file`, `record.dir`, `sim.*`, `regulatory.dial_hz`,
  `kiss.bind`) and which are *secret*
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

### 4.10 The rules

The daemon judges every transmission against a regulatory profile before the radio is keyed
(ADR-0018, `docs/user/fcc-regulatory-controls.md`). What it decided, and what it would decide, is
data a client can show.

| Method | Params | Result |
|---|---|---|
| `regulatory.check` | any of `dial_hz`, `control` (`local`, `remote`, `automatic`), `license_class`, `sideband` (`usb`, `lsb`), `direction` (`originate`, `respond`, `operator`), `rung` — each in place of the station's own | `decision` (for the station's waveform up to `rung`, or its fastest mode), `safe_dials` (every dial range where that waveform fits), `ceiling` (the fastest rung allowed, and the decision that stops the next), `situation` (the facts it used). A "what if" — nothing is changed |
| `regulatory.profile` | — | `profile` (the profile in force, whole: bands, segments, privileges, the automatic-control segments, 60 m, power limits, the band plan, each with its citation; null without one) and `known` (the profiles this build carries: `id`, `name`) |

`status.regulatory` is where the station stands now:

* `policy`: `rules` (a profile applies), `none` (the operator chose none, and checks every
  transmission), `unset` (none chosen: nothing is transmitted) or `broken` (the profile would
  not read: nothing is transmitted; `error` says why); `profile`: `id`, `name`, `authority`,
  `rules_as_of`, `source`, `bandwidth_reading`;
* `situation`: `dial_hz` and `dial_source` (`radio`, read over CAT or `rigctld`, or `declared` by
  the operator), `sideband`, `control`, `license`, `itu_region`, `margin_hz`, `band_plan`,
  `power_w`; `direction`: who would begin the exchange of a transmission now;
* `indicator`: the decision for the station's widest transmission — every rung up to its
  fastest mode with its control frames — at the dial it is on: what the panel shows as LEGAL,
  WARNING or TX BLOCKED. When the link is held to its narrower rungs here it is a `warning` that
  says which;
* `ceiling`: `rung` (the fastest rung the rules allow; null when none is), `name`, `of` (the
  ladder's length) and `limit` (the decision that stops the next rung);
* `last`: the last decision the gate made — `t_s`, `age_s` and the `decision`;
* `occupied`: the audio edges of the `widest` waveform and of the tone `floor`, both readings of
  §97.3(a)(8); `safe_dials`: the dial ranges where each fits.

A **decision** is `verdict` (`legal`, `warning`, `blocked`), `code` (the reason in a word:
`permitted`, `automatic_segment`, `automatic_response`, `sixty_channel`, `sixty_segment`,
`no_rules`; or a refusal: `no_profile`, `profile_broken`, `region`, `no_control`, `no_license`,
`no_sideband`, `no_dial`, `unmeasured`, `out_of_band`, `too_wide`, `outside_data_segment`,
`no_data_here`, `privilege`, `automatic_excluded`, `automatic_bandwidth`, `automatic_originate`,
`automatic_outside`, `sixty_emission`, `sixty_channel`, `sixty_off_channel`), `rule` (its
citation), `summary` and `detail` (the reasoning in words, with the RF range), `what` (the
transmission, in words), `kind` (`data`, `cw`, `test`, `nothing`), `direction`, `dial_hz`,
`dial_source`, `sideband`, `control`, `license`, `audio_low_hz`/`audio_high_hz` and
`rf_low_hz`/`rf_high_hz` (the edges the reading decides by), `bandwidth_hz`, `margin_hz`, `band`,
`segment` and `segment_rule`, `automatic_segment`, `guidance` (the voluntary band plan's word) and
`notes` (power limits, 60 m's sharing).

The settings are `[regulatory]` in the configuration, all live: `profile` (`""`, `none`,
`us-fcc-part97`), `control`, `license_class`, `sideband`, `itu_region`, `edge_margin_hz`,
`band_plan`, `dial_hz` (for a radio that cannot report its dial; this machine's, not a profile's)
and `log_permitted`.

### 4.11 Datagrams and the KISS port

A **datagram** is another program's frame — an AX.25 frame from an APRS or packet program —
sent outside any session with no acknowledgement, and handed to the programs of every station
that decodes it (ADR-0019; the frame format is in `air-interface.md`). The daemon's KISS port
(`host-interfaces.md` §8) is a client of these methods, as the VARA-compatible adapter is of the
session ones.

| Method | Params | Result |
|---|---|---|
| `datagram.send` | `data` (base64: the frame), `frame_type?` (0, 1 or 2; 0), `ref?` (reported back by `datagram-sent`), `rung?` (the rung to send at; tone-36, rung 1), `wait_for_clear?` (true), `persistence?` (0.25) and `slot_s?` (0.1): the client's channel access | `accepted`, `queued` (datagrams waiting, this one included) of `limit` (16), `fragments`, `bursts` (keyings: each fits the key limit), `air_s` and `rung`. Refused `queue_full` (retryable) when sixteen are waiting — the KISS port stops reading its client until there is room — `bad_params` when the frame is of an unknown type or longer than sixteen fragments at the rung, and `refused` on an answer-only station |
| `kiss.status` | — | the KISS port as `status.kiss` has it |
| `kiss.disconnect` | `client?` (an `id` from `kiss.status`) | `disconnected`: how many connections were closed — that one, or every one. The programs may connect again; refused `not_listening` when the port is not open |

A datagram waits in a queue of its own and goes only while the station is in no session and has
nothing else to send: the session's turn-taking has no room for a stranger's burst. Then it
waits for a clear channel (when `wait_for_clear`) and draws p-persistence each slot, as a KISS
TNC does, and reaches the air through the regulatory gate like every other transmission — as
*originated* by this station (§4.10). A datagram whose rungs the rules do not allow is reported
by `datagram-sent` with the decision's words.

`status.kiss` is `enabled`, `bind`, `rung`, `wait_for_clear`, `ignore_dcd` (a host program said
`IGNOREKISSDCD ON`), `listening`, `address`, `exposed` (the address lets other computers in),
`error` (why it is not listening, when it should be), `paused` (why frames from programs are
not being sent — a host program holds the VARA-compatible port without `CHAT ON`), `frames_in`,
`frames_out`, `malformed`, and `clients`: one entry per connection with `id`, `peer`, `app` (a
guess from what it sends, for display only), `since_ms`, `frames_in`, `frames_out` and
`dropped`.

The settings are `[kiss]`, all live: `enabled` (false), `bind` (`127.0.0.1:8100`; this machine's,
never carried by a profile), `rung` (1),
`wait_for_clear` (true), `max_clients` (4, 1–16) and `trace` (log each frame's command, type,
length and fate — never its contents).

### 4.6 The diagnostic bundle

`diagnostics` answers the questions a maintainer asks first — what version, on what, with
what settings, doing what — in one object, so a panel can offer a single "copy" button and an
operator can paste the result into an issue from wherever they are:

| Key | Contents |
|---|---|
| `version`, `platform` | the daemon's version; `os` and `arch` |
| `generated`, `started` | RFC 3339 UTC timestamps for the bundle and for the daemon's start |
| `config`, `path` | the running configuration (secrets redacted) and the file it came from |
| `status`, `capabilities` | the station's `status` — without the daemon's own fields (`host`, `kiss`, `supervised`, `audio_fault`, `config_note`, `binary`) — and `capabilities` as the method returns it |
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
ring would hold nothing else; `datagram.send` and `kiss.disconnect` are not either, the KISS
port logging its own lines); every keying and release of the transmitter; every session
event the modem reports; dropped audio; and anything that failed.

---

## 7. Open items for v1.0

* `config.set` cannot reopen a sound card or rebind the control or host sockets; those keys
  are written and reported as needing a restart (the KISS port restarts in place).
* A capability flag for the FM PHY's differences.
* Rate limiting and back-pressure rules for `send` on a slow link.
