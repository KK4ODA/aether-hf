# Aether host interfaces — specification v0.1

Status: **draft**, and implemented in `core/aetherd/src/host/`. This documents the
compatibility interfaces that let software people already run — Winlink Express, Pat, VarAC,
BPQ32, and APRS and packet programs over KISS (§8) — use an Aether station without being
modified.

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
| `CONNECT <from> <to>` | Starts a session, as `<from>` when that is one of the `MYCALL` callsigns | A station that answers to a club or tactical call besides its own chooses between them here; the called station answers as whichever of its callsigns was called. When the modem refuses the call — a session already up, an answer-only station, the rules (ADR-0018) — the `OK` is followed at once by `DISCONNECTED`; the reason is in the daemon's log |
| `DISCONNECT` | Closes the session in order: a sending station sends what is queued first; a receiving station sends its disconnect between the other station's bursts (ADR-0023) | Orderly. During a call it does not stop calling (a call that is answered is closed at once); `ABORT` does |
| `ABORT` | Ends it now: the rest of a burst on the air is cut and one disconnect follows; during a call, stops calling | Not orderly |
| `LISTEN ON` / `LISTEN OFF` | Whether to answer incoming calls | Recorded: this station answers calls to its callsigns either way (§9) |
| `LISTEN CQ` | VarAC: hear only CQ frames | Recorded as listening; this station hears everything and answers calls to its own callsigns either way |
| `CHAT ON` / `CHAT OFF` | VarAC's chat mode | While this host is attached, frames from KISS programs are dropped until it says `CHAT ON` — VARA's "Winlink priority" (§8.4); VarAC says it on every start, Winlink Express never. It says nothing about sessions, which run as they always do |
| `IGNOREKISSDCD ON` / `OFF` | The KISS port's channel access | `ON`: frames from KISS programs go without waiting for a clear channel while this host is attached (§8.4). VarAC says it when its *Ignore DCD* box is ticked |
| `BW2300`, `BW500` | Names the bandwidth | `OK` when it is the one the station runs (`[radio] bandwidth`; a host learns it from `capabilities`), `WRONG` otherwise: the bandwidth is the modem's configuration, not a session setting, and both stations of a session run the same one |
| `BW2750` | — | **Refused.** Not a waveform this version has; see §5 |
| `PUBLIC ON` / `PUBLIC OFF` | Whether the station may be listed publicly | Recorded |
| `COMPRESSION OFF\|TEXT\|FILES\|ON` | What the host wants compressed | Recorded; see §5. `ON` is what Winlink Express sends and means `TEXT` |
| `WINLINK SESSION` / `P2P SESSION` | Which kind of session is running | Recorded |
| `CWID ON` / `CWID OFF` | Identify in Morse after a transmission | Recorded; see §5 |
| `CQFRAME` | Sends a beacon: this station's callsign, addressed to nobody, on the tone floor (the control API's `beacon`) | `OK` once understood; a beacon the modem refuses — in a session, on an answer-only station, or by the rules (ADR-0018) — is not sent, and nothing more is said |
| `TUNE ON` / `TUNE <seconds>` / `TUNE OFF` | Keys and plays a steady 1500 Hz tone at the transmit level, for an antenna tuner. Drive is set with real bursts (the panel's *Set drive*, the control API's `drive.set`): the waveform's peaks stand 6–7 dB above the tone's | `TUNE ON` (VarAC's TUNE button) is 10 s; `TUNE <seconds>` takes 0–30 and is held to 10; `TUNE OFF` cuts a tone short. `OK` once understood: a tone the modem refuses — in a session, on a busy channel, or by the rules — is not sent, and nothing more is said |
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
| `ENCRYPTION DISABLED` | After `CONNECTED`: the link carries no encryption, which VARA states of its links and is simply true of this modem |
| `DISCONNECTED` | A session ended — or, straight after the `OK` to a `CONNECT`, the modem refused the call |
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
`P2P SESSION`, `LISTEN ON` / `LISTEN OFF` / `LISTEN CQ`, `DRIVELEVEL` (`CHAT` and
`IGNOREKISSDCD` govern the KISS port, §8.4). The setting is remembered and reported back, and the
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

The adapter is a *client of the modem's own control API*. It sends `capabilities` (once, on
attach: the bandwidth and the bit rates `BITRATE` reports), `callsigns.set`, `connect`, `send`,
`disconnect`, `abort`, `beacon`, `tune` and `config.get` (for `TUNE ?`), and listens for
`state`, `data`, `ptt`, `frame` (for `SN`) and `metrics` (for `BUFFER`, `BITRATE`, `BUSY`)
events. It has no privileged access to the station and can do nothing a scripted client could
not do.

That layering is deliberate: the compatibility surface is the part most likely to need
changing as clients are tested against it, and it cannot destabilise the modem underneath.

---

## 7. Verification status

| Client | Status |
|---|---|
| The test suite's own host client | **Passing** — setup, call, session notifications, payload both ways, second-host refusal |
| Pat 1.0.0 | **Passing on the bench** (2026-09-14): two daemons over the simulated channel at 15 dB, Pat at both ends; a P2P B2F session — connect, SID exchange, a proposal, a message with a 6 000-byte incompressible attachment, `FF`/`FQ`, disconnect — in 84 s, the attachment byte-identical on arrival. One fix on the way: the called side's `CONNECTED` had named this station first. On the air: not yet |
| Winlink Express 1.8.5.0 | **Passing on the bench** (2026-09-14): two instances over the simulated channel at 15 dB, a Vara HF P2P session each, `MYCALL KK4ODA-1` and `KK4ODA-2`. A P2P message with a 6 000-byte incompressible attachment (zipped by Winlink Express, which does not allow `.bin`): 6 637 bytes in 36 s by its own count, the whole B2F session 62 s, the attachment byte-identical on arrival. Its opening line, verbatim in the adapter's test: `PUBLIC ON`, `CWID ON`, `COMPRESSION ON`, `BW<max>`, `MYCALL`, `LISTEN ON`. Two things it found. `COMPRESSION ON` is not in the published set and was `WRONG`; it is `TEXT` and is now heard. And **`MYCALL` had stopped at the adapter** — the modems kept the callsigns in their configuration files, and a call to the name Winlink Express chose was never answered — which is why `MYCALL` now reaches the modem (`callsigns.set`) and `CONNECT <from> <to>` says which callsign the session runs under. Quirks: it demands a TNC path even with auto-launch off (it launched `C:\VARA\Vara.exe` once before that was unchecked — set the path to `aetherd` and untick the launch), it opens a session on port 8300 by default so a second instance needs its own port before its session window is ever opened, and it wants a centre frequency before it will call. On the air: not yet |
| VarAC 15.0.18 | **Passing on the bench at 500 Hz** (2026-09-15): two VarAC copies over the simulated channel at 12 dB, both daemons at `[radio] bandwidth = 500`, plain callsigns at both ends. A **ping** — VarAC's is a `CONNECT` to the target's callsign with `-T` appended, answered by the station that registered that alias in `MYCALL`, one 14-byte report back, `DISCONNECT` — completes in about 8 s ("Report Received: 12"), twice in a row; a **connect** brings B's welcome frame across 0.8 s after `CONNECTED` and A reads B's capabilities from it. Its start-up conversation is verbatim in the adapter's test: `BW500`, `CHAT ON`, `LISTEN ON`, `IGNOREKISSDCD ON`, `LISTEN CQ`, `VERSION`, `MYCALL <call> <call>-T`, `BW500` again. **Six things it needed, found one attempt at a time:** (1) `SN <dB>` — it builds every signal report, the one a ping exists to fetch included, from the modem's SN lines, and the adapter had none; (2) the call's `SN` **before** `CONNECTED`, because it composes its opening report the instant it sees `CONNECTED`; (3) `TUNE ?` answered `TUNE <dB>` (it asks after every connection; `WRONG` stalled it); (4) `BUFFER 0` when it attaches — it sends nothing on the data port until it has heard how full the buffer is; (5) a quiet DCD during a session: with *Ignore DCD* off it holds "busy" after every `BUSY ON`, and the detector marked the channel busy at every frame of the partner, so it never found a moment to send — the channel is the session's now, as Mercury does it; (6) `PENDING` before `CONNECTED` on the called side and `ENCRYPTION DISABLED` after it, as VARA says them. Quirks: it strips the SSID from its own `-T` alias (`KK4ODA-1` listens as `KK4ODA-T`, while a ping to `KK4ODA-1` calls `KK4ODA-1-T`, ten characters the air interface cannot carry) — use plain callsigns; it cannot parse `VERSION Aether HF 0.2.0-beta.N` and, failing to, skips its "must be VARA 4.6.8 or higher" gate, which is left as it is rather than claim a VARA version; after a connect on a calling frequency it asks to QSY to a slot (answer No on the bench); `DRIVELEVEL n` is recorded, not acted on. On the air: not yet |
| BPQ32 | Not yet verified |

The command set here is implemented from its published behaviour, and every client differs a
little in what it sends and what it tolerates. Until each has been run against a real station,
this table is what the compatibility claim rests on, and it should be read as exactly that.

---

## 8. The KISS port

APRS and packet programs, and VarAC's broadcast messages, do not use sessions. They hand a
modem whole frames over **KISS** — the TNC framing of Chepponis and Karn (1987) — and expect
each on the air as it is, with no acknowledgement; they do their own repeating. VARA HF has
a KISS port beside its command and data ports, and Aether has one that answers the same way
(`core/aetherd/src/kiss/`, ADR-0019). It carries frames as **datagrams**: the `DATAGRAM` DATA
kind of `air-interface.md`, sent outside sessions and handed to the KISS programs of every
station that decodes them. Like the adapter above it is a client of the control API
(`datagram.send`, the `datagram` and `datagram-sent` events, `control-api.md` §4.11), and it
is **off by default** (`[kiss] enabled = false`): it lets other software make this station
transmit.

### 8.1 Transport

| | |
|---|---|
| Port | TCP, default `127.0.0.1:8100` — VARA HF's KISS port, so a program set up for VARA needs no change |
| Framing | KISS: `FEND` (0xC0) … `FEND`, `FESC` (0xDB) `TFEND` (0xDC) for a 0xC0 in the frame and `FESC` `TFESC` (0xDD) for a 0xDB; the type byte after `FEND` is escaped too |
| Frames | Reassembled across TCP reads and split out of one; up to 2 048 bytes a frame, type byte included (the KISS paper asks for at least 1 024) |
| Clients | Several at once (`[kiss] max_clients`, 4; VARA 4.8 takes several). A client past the limit is closed at once, with a log line |
| Received frames | To every client, with the type they came with, at their exact length |
| Backpressure | Sixteen datagrams wait at most (`control-api.md` §4.11); past that the port stops reading the client until there is room, and TCP holds the rest — nothing is dropped for being early |

A malformed frame — a `FESC` followed by anything but `TFEND`/`TFESC`, an escape the frame
ended in, a frame over the limit — is dropped and counted (`status.kiss.malformed`), and the
decoder carries on at the next `FEND`. Empty frames (`FEND FEND`, the usual idle fill) are
nothing.

### 8.2 The type byte

Standard KISS gives the type byte's high nibble to the port and its low nibble to a command.
VARA's KISS port reads the byte after `FEND` as a **frame type** instead (EA5HVK, *VARA KISS
Interface*, 2024): 0 an AX.25 frame (the APRS programs), 1 an AX.25 frame whose address fields
are eight bytes rather than seven (VarAC's broadcasts), 2 unformatted data. The two readings
collide at 1 and 2 — TXDELAY and P — and both kinds of program connect to a port like this one,
so the port tells them apart by length:

| Byte | Data | Meaning here |
|---|---|---|
| `0x00` | a frame | Send it, type 0 |
| `0x01` | exactly one byte | TXDELAY: accepted and ignored — the modem keys with its own lead |
| `0x01` | anything else | Send it, type 1 (VarAC) |
| `0x02` | exactly one byte | P: the client's p-persistence, `(P + 1) / 256` |
| `0x02` | anything else | Send it, type 2 |
| `0x03` | one byte | SLOTTIME: the client's slot, in 10 ms units |
| `0x04`, `0x05` | one byte | TXTAIL, FULLDUPLEX: accepted and ignored — the modem holds the key with its own tail and is half duplex |
| `0x06` | anything | SETHARDWARE: accepted and ignored |
| `0x0C` | two bytes, then a frame | **ACKMODE** (the extension BPQ32, QtTermTCP and Winlink Express use to pace AX.25 on a slow modem): send the frame, type 0, and send the two bytes back as `FEND 0x0C id id FEND` once it has gone out — after the last burst that carries it has left the sound card. A frame that does not go out (refused by the rules, cut short, dropped with the port) is not acknowledged; the program's own timers take over |
| `0xFF` | — | RETURN: accepted and ignored; a TCP port has no KISS mode to leave |
| port ≠ 0 | — | Refused and logged: this modem has one port |

A one-byte type-2 frame would be read as P; that is the price of serving both kinds of program
on one port, and no program known sends one. Received frames go to the programs as
`FEND type frame FEND` — the type they were sent with — so VarAC gets its type-1 frames back as
type 1, and an APRS program gets `0x00`.

### 8.3 On the air

Each frame is one **datagram**: its bytes and type, with the sending station's callsign in front
— every datagram identifies its station in the emission itself, whatever the program's frame
holds — split into DATA frames of kind `DATAGRAM` at the rung `[kiss] rung` names (1, tone-36, by
default: the tone floor's frames are the same in both bandwidths, so a station of either decodes
them, as VARA sends its KISS frames on its 500 Hz waveform for the same reason), in bursts that
each fit the key limit. A datagram waits while a session is up — the session's turn-taking has
no room for a stranger's burst — and then for a clear channel, and draws p-persistence each
slot, as a KISS TNC does. It reaches the air through the regulatory gate like every other
transmission (ADR-0018), as a transmission this station originates: an automatically controlled
station sends datagrams only where it may originate, and an answer-only station
(`[radio] answer_only`) sends none — the port takes the frame and drops it, with a log line. A
fragment carries its frame's payload less
the three-byte DATA header — 33 bytes at tone-36, one 5.4 s frame each — so a 60-byte APRS
position (68 bytes with the callsign and type) is three fragments, 16 s on the air, and the
longest frame a datagram carries at tone-36 is 520 bytes (sixteen fragments). A faster rung
carries more, sooner, to fewer stations.

### 8.4 Beside the VARA-compatible port

VARA gives a program on its command port a say over its KISS port, and Aether does the same:

* **Winlink priority.** While a program holds the command port (§2) without having said
  `CHAT ON`, frames from KISS programs are **not sent** — they are dropped and counted, with one
  log line saying why — so that APRS beacons cannot key the radio in the middle of a Winlink
  station's listening. VarAC says `CHAT ON` on every start; Winlink Express does not. Received
  frames still go to the KISS programs. `status.kiss.paused` says when this is in force.
* **`IGNOREKISSDCD ON`**: frames from KISS programs go without waiting for a clear channel
  while that program is attached (`[kiss] wait_for_clear` otherwise, true by default).
* **`SN <dB>`** on the command port for every decoded frame, datagrams included: VarAC shows the
  SNR of a broadcast it received from the `SN` that came with it.

When the host disconnects, both are forgotten.

### 8.5 Security

The port asks no password: whoever reaches it can make the station transmit. It listens on
loopback by default; any other address is logged as a warning when the port opens, reported as
`status.kiss.exposed`, and said in the panel. The control API's token (`[control] token`) does
not cover it — a KISS program has no way to present one — so an address other than loopback
belongs only on a network the operator controls.

### 8.6 Verification status

| Client | Status |
|---|---|
| The test suite's own KISS clients | **Passing** — framing and escapes, partial and joined reads, malformed frames, the VARA type bytes and the standard parameters, ACKMODE, several clients, backpressure, Winlink priority, a client that vanishes, four clients writing a hundred frames each in ragged pieces at once; and `a_kiss_frame_crosses_from_one_daemons_kiss_port_to_the_others` (`tests/two_daemons.rs`): a frame written to one daemon's KISS port arrives, byte for byte and with its type, at a client of the other's, over `[sim]` |
| `tools/kiss_test_client.py` | **Passing on the bench** (2026-09-25): two daemons over `[sim]`, an AX.25 UI frame from one KISS port to the other |
| VarAC, Winlink Express Packet, BPQ32, QtTermTCP, APRSIS32, YAAC, PinPoint APRS, APRSdroid, Xastir | Not yet verified; `docs/user/kiss.md` says how each is set up |

---

## 9. Open items for v1.0

* Pat and Winlink Express on the air (the bench is done for both; `docs/user/field-test.md`),
  then BPQ32, not yet tried even on the bench; VarAC passes the bench at 500 Hz (§7) and waits
  for the air (below). All four can be tried with no radio over `[sim]`.
* Whether `COMPRESSION` should choose the station's compression, which the two stations
  already negotiate in the connect handshake (P3-6): it would then be acted on rather than
  recorded.
* Whether `LISTEN OFF` should stop answering calls at the link layer. Today the station always
  answers; the setting is recorded, and the control API says so explicitly rather than
  pretending.
* **VarAC on the air.** The bench passes (§7); the air with a VarAC station is what remains.
  VarAC's "ping" is a short session — connect, one report, disconnect — over the `CONNECT`
  the adapter serves; VARA's published command set has no `PING` (that vocabulary is
  ARDOP's), so the adapter has none either. Aether's own probe (ADR-0006) is reached
  through the control API. Open from the bench: the scale of VARA's `DRIVELEVEL`, and a
  `VERSION` form VarAC can parse without a VARA version number in it.
