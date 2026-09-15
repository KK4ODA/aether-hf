# Aether host interfaces — specification v0.1

Status: **draft**, and implemented in `core/aetherd/src/host/`. This documents the
compatibility interface that lets software people already run — Winlink Express, Pat, VarAC,
BPQ32 — use an Aether station without being modified.

Companion documents: `control-api.md` (the modem's own interface, which is the one to build
new things against), `air-interface.md` (what goes over the air),
`../COMMUNITY-CONCERNS.md` (why this exists at all).

---

## 1. What this is, and what it is not

Aether speaks the **published VARA TCP host interface**. That is a software compatibility
layer and nothing more:

* **It is not over-the-air compatibility.** An Aether station cannot talk to a VARA station.
  The waveform is Aether's own and is specified in `air-interface.md`. Over-the-air
  compatibility with a proprietary waveform is not achievable and is not attempted.
* **Nothing here is derived from VARA's internals.** Only the documented command set is
  implemented, from its documented behaviour.
* **The modem never claims to be VARA.** `VERSION` answers `VERSION Aether HF <version>` —
  three words, the shape of the published reply, so a host that takes the version from the
  fourth token finds one (VarAC does); the name is this modem's.

That last point is a decision, not an oversight. `COMMUNITY-CONCERNS.md` §1 is that a new mode
lives or dies by how many working gateways it has; §13 is the corollary — a gateway listed as
VARA that actually runs Aether would strand every real-VARA client that called it. A station
that tells the truth about what it is can be listed correctly. **Operators must not advertise
an Aether gateway as a VARA gateway.**

New software should use `control-api.md` instead. It is PHY-agnostic, it exposes what the
modem is actually doing, and it is not constrained by what another program already expects.

---

## 2. Transport

| | |
|---|---|
| Command port | TCP, line-oriented ASCII, default `127.0.0.1:8300` |
| Data port | TCP, raw binary payload, **command port + 1** |
| Line terminator | carriage return (`\r`); a trailing newline is tolerated |
| Case | commands are matched case-insensitively; callsigns are upper-cased |
| Concurrency | one host at a time |

The two ports are taken together and must both be free. A client that found the command port
and then could not find the data port would connect and hang, so the daemon refuses to start
and names the port that is in the way rather than quietly moving it.

A second host connection is answered with `WRONG` and closed. The published interface has no
notion of two hosts sharing a radio, and two programs taking turns keying one transmitter is
not a situation to invent semantics for — but a silent hang would be the worst of the
available answers, so the refusal is explicit.

The interface is **off by default** (`[host] enabled = false`). It allows other software to
key this radio; that is not something to enable without being asked.

---

## 3. Commands — host to modem

Every command is answered with `OK` or `WRONG` unless a specific reply is listed.

| Command | Effect | Notes |
|---|---|---|
| `MYCALL <call>[ <call>…]` | Sets the callsigns this station answers to; the first is the one it calls as | Space- or comma-separated. Refused if any callsign is not one the link layer can carry (`A–Z 0–9 - /`, at most nine characters). Reaches the modem (`callsigns.set`), so the host's callsign replaces the one in the configuration file — the host owns the operator's callsign; VARA has none of its own. Given during a session it takes effect when the session ends |
| `CONNECT <from> <to>` | Starts a session, as `<from>` when that is one of the `MYCALL` callsigns | A station that answers to a club or tactical call besides its own chooses between them here; the called station answers as whichever of its callsigns was called |
| `DISCONNECT` | Closes it once the queue drains | Orderly |
| `ABORT` | Drops it immediately | Not orderly |
| `LISTEN ON` / `LISTEN OFF` | Answer incoming calls, or not | |
| `LISTEN CQ` | VarAC: hear only CQ frames | Recorded as listening; this station hears everything and answers calls to its own callsigns either way |
| `CHAT ON` / `CHAT OFF` | VarAC's chat mode | Recorded. The short frames it means are a different air interface this modem does not have; `OK` tells the host the modem heard, which is what keeps VarAC from calling it broken |
| `IGNOREKISSDCD ON` / `OFF` | A KISS-port detail | Heard; there is no KISS port |
| `BW2300`, `BW500` | Names the bandwidth | `OK` when it is the one the station runs (`[radio] bandwidth`; a host learns it from `capabilities`), `WRONG` otherwise: the bandwidth is the modem's configuration, not a session setting, and both stations of a session run the same one |
| `BW2750` | — | **Refused.** Not a waveform this version has; see §5 |
| `PUBLIC ON` / `PUBLIC OFF` | Whether the station may be listed publicly | Recorded |
| `COMPRESSION OFF\|TEXT\|FILES\|ON` | What the host wants compressed | Recorded; see §5. `ON` is what Winlink Express sends and means `TEXT` |
| `WINLINK SESSION` / `P2P SESSION` | Which kind of session is running | Recorded |
| `CWID ON` / `CWID OFF` | Identify in Morse after a transmission | Recorded; see §5 |
| `CQFRAME` | Sends a `BEACON` frame: this station's callsign, unproto | Refused while a session is running |
| `TUNE ON` / `TUNE <seconds>` / `TUNE OFF` | Keys and plays a steady 1500 Hz tone at the transmit level, so the operator can set drive by the rig's ALC | Bounded at 10 s, which is what `TUNE ON` (VarAC's TUNE button) gets. `TUNE OFF` cuts a tone short; the level follows `audio.tx_level` live, so the drive can be set while the tone plays |
| `TUNE ?` | → `TUNE <dB>`: the transmit level, in decibels below full scale (`20 log₁₀ audio.tx_level`) | VarAC asks after every connection, to keep a level per band. The scale VARA answers on is not published; decibels below full scale is the one a sine amplitude has an honest reading on |
| `DRIVELEVEL <n>` | The transmit level a host would set | Recorded, not acted on: the scale is not published, and a wrong guess would change the operator's drive. Setting the drive is the operator's, in Setup |
| `VERSION` | → `VERSION Aether HF <version>` | |
| `BUFFER` | → `BUFFER <bytes>` | Payload bytes still to send |

Anything else is answered `WRONG`. A client that got silence could not tell a missing feature
from a hung modem, so nothing is ignored.

---

## 4. Notifications — modem to host

Unsolicited, at any time.

| Line | When |
|---|---|
| `PTT ON` / `PTT OFF` | The transmitter was keyed or released |
| `BUSY ON` / `BUSY OFF` | The busy detector changed its mind about the channel — outside a session. While a session is up the channel is the session's and reads `BUSY OFF`: the detector marks it busy at every frame of the other station, and a host that honours DCD (VarAC with *Ignore DCD* off holds "busy" for ten seconds after each) would never find a moment to hand its data over; the modem does the turn-taking |
| `PENDING` | The called side, just before its `CONNECTED`: the order every client expects, from a modem that answers a call in one step |
| `CONNECTED <caller> <called> <bandwidth>` | A session came up. The caller first, whichever side this is: a host takes a `CONNECTED` whose second callsign is not its own as somebody else's business — Pat's listening side ignored the session until this was right |
| `DISCONNECTED` | A session ended |
| `BUFFER <bytes>` | The number of payload bytes still to send changed — and `BUFFER 0` once, as the first line a host hears when it attaches. VarAC sends nothing on the data port until it has heard how full the modem's buffer is (found on the bench: a ping sat for ninety seconds with both ends waiting) |
| `BITRATE (<mode>) <bps> BPS` | The mode in use changed during a session: the mode index (VARA's "speed level") and its net bit rate, which a host shows as the link speed |
| `REGISTERED <call>` | Sent before `CONNECTED` |
| `SN <dB>` | A frame decoded — any frame, as a modem that reports what it hears: the SNR it arrived at, whole decibels, 3 kHz reference. The call that brings a session up is reported before `PENDING`/`CONNECTED`, which is when VarAC builds its opening signal report. VarAC builds its signal reports from these — the report it sends on connecting, and the one a ping exists to fetch — so without them a ping never ends (found on the bench) |
| `IAMALIVE` | Every 10 s, so a quiet host knows the modem is there |

`REGISTERED` exists because some clients warn their user about a speed limit unless the modem
says its callsign is registered. Aether is free software with no registration and no speed
limit, so the honest answer to "is this station limited?" is no.

---

## 5. What is accepted, recorded, or refused

The three are deliberately distinguished, and a client can tell them apart.

**Refused** (`WRONG`): `BW2750`, and whichever of `BW2300` / `BW500` is not the bandwidth the
station runs. Accepting a request for one bandwidth and then transmitting another would put a
station outside the bandwidth its operator chose, which is an operator's decision and sometimes
a legal one — a 2 300 Hz signal on a 500 Hz calling frequency most of all. The bandwidth is set
in the station's configuration (`[radio] bandwidth`, 2300 or 500; the panel's Setup tab), and
`CONNECTED` reports it.

**Recorded but not yet acted on**: `COMPRESSION`, `CWID`, `PUBLIC`, `WINLINK SESSION` /
`P2P SESSION`, `CHAT`, `LISTEN CQ`. The setting is remembered and reported back, and the
modem answers `OK` because the command was understood.

Compression and Morse identification both exist (P3-6) but are configured on the station, not
per host session: compression is negotiated with the *other station* in the connect handshake
and cannot be turned on from one end alone, and whether to identify in Morse is a licence
question for the operator rather than a runtime choice for a client. Wiring these commands to
those settings is an open item.

**Not a command here**: `PING` / `PINGACK`. VARA's published command set has none (that
vocabulary is ARDOP's host protocol), and VarAC's ping is a short session over `CONNECT`,
which works as it is. Aether's own link probe (ADR-0006) is reached through the control
API's `probe` and the panel's Probe button, not through this adapter.

**Everything else in §3 is acted on.**

---

## 6. How it is built

The adapter is a *client of the modem's own control API*. It sends `connect`, `send`,
`disconnect` and `abort`, and listens for `state`, `data`, `ptt` and `metrics` events. It has
no privileged access to the station and can do nothing a scripted client could not do.

That layering is deliberate: the compatibility surface is the part most likely to need
changing as clients are tested against it, and it cannot destabilise the modem underneath.

---

## 7. Verification status

| Client | Status |
|---|---|
| The test suite's own host client | **Passing** — setup, call, session notifications, payload both ways, second-host refusal |
| Pat 1.0.0 | **Passing on the bench** (2026-09-14): two daemons over the simulated channel at 15 dB, Pat at both ends; a P2P B2F session — connect, SID exchange, a proposal, a message with a 6 000-byte incompressible attachment, `FF`/`FQ`, disconnect — in 84 s, the attachment byte-identical on arrival. One fix on the way: the called side's `CONNECTED` had named this station first. On the air: not yet |
| Winlink Express 1.8.5.0 | **Passing on the bench** (2026-09-14): two instances over the simulated channel at 15 dB, a Vara HF P2P session each, `MYCALL KK4ODA-1` and `KK4ODA-2`. A P2P message with a 6 000-byte incompressible attachment (zipped by Winlink Express, which does not allow `.bin`): 6 637 bytes in 36 s by its own count, the whole B2F session 62 s, the attachment byte-identical on arrival. Its opening line, verbatim in the adapter's test: `PUBLIC ON`, `CWID ON`, `COMPRESSION ON`, `BW<max>`, `MYCALL`, `LISTEN ON`. Two things it found. `COMPRESSION ON` is not in the published set and was `WRONG`; it is `TEXT` and is now heard. And **`MYCALL` had stopped at the adapter** — the modems kept the callsigns in their configuration files, and a call to the name Winlink Express chose was never answered — which is why `MYCALL` now reaches the modem (`callsigns.set`) and `CONNECT <from> <to>` says which callsign the session runs under. Quirks: it demands a TNC path even with auto-launch off (it launched `C:\VARA\Vara.exe` once before that was unchecked — set the path to `aetherd` and untick the launch), it opens a session on port 8300 by default so a second instance needs its own port before its session window is ever opened, and it wants a centre frequency before it will call. On the air: not yet |
| VarAC 15.0.18 | **Talks to it; the bench at 500 Hz is next (P7-0d).** Its start-up conversation is verbatim in the adapter's test: `BW500`, `CHAT ON`, `LISTEN ON`, `IGNOREKISSDCD ON`, `LISTEN CQ`, `VERSION`, `MYCALL <call> <call>-T`, `BW500` again. Three of those had come back `WRONG` and are now heard (§3). The one this modem could not do — **VarAC's ecosystem is 500 Hz**; it disables its whole interface until `BW500` is answered `OK`, and refuses to call at 2300 Hz on a calling frequency ("you can't use a 2300/2750Hz bandwidth on a calling QRG") — it now can: a station configured with `[radio] bandwidth = 500` runs the 500 Hz waveform (`air-interface.md` §2.3) and answers `BW500` `OK`. Two VarAC instances over `[sim]` at 500 Hz, then the air, are what remains |
| BPQ32 | Not yet verified |

The command set here is implemented from its published behaviour, and every client differs a
little in what it sends and what it tolerates. Until each has been run against a real station,
this table is what the compatibility claim rests on, and it should be read as exactly that.

---

## 8. Open items for v1.0

* Pat and Winlink Express on the air (the bench is done for both; `docs/user/field-test.md`),
  then BPQ32 (`COMMUNITY-CONCERNS.md` §13 adds VarAC to the matrix, and VarAC waits on the
  500 Hz waveform below). All four can be tried with no radio over `[sim]`.
* Compression negotiation (P3-6), which changes what `COMPRESSION` means from recorded to
  acted on.
* Whether `LISTEN OFF` should stop answering calls at the link layer. Today the station always
  answers; the setting is recorded, and the control API says so explicitly rather than
  pretending.
* **VarAC over the 500 Hz waveform.** The waveform exists and `BW500` is answered `OK` by a
  station running it; what remains is the bench (two VarAC instances over `[sim]`, both
  daemons at 500 Hz) and the air. VarAC's "ping" is a short session — connect, exchange
  the reports, disconnect — over the `CONNECT` the adapter already serves; VARA's
  published command set has no `PING` (that vocabulary is ARDOP's), so the adapter has
  none either. Aether's own probe (ADR-0006) is reached through the control API.
