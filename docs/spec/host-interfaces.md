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
* **The modem never claims to be VARA.** `VERSION` answers `VERSION Aether-HF-<version>`.

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
| `MYCALL <call>[ <call>…]` | Sets the callsigns this station answers to | Space- or comma-separated. Refused if any callsign is not one the link layer can carry (`A–Z 0–9 - /`, at most nine characters) |
| `CONNECT <from> <to>` | Starts a session | |
| `DISCONNECT` | Closes it once the queue drains | Orderly |
| `ABORT` | Drops it immediately | Not orderly |
| `LISTEN ON` / `LISTEN OFF` | Answer incoming calls, or not | |
| `BW2300` | Selects the bandwidth | The only one this PHY has (ADR-0002) |
| `BW500`, `BW2750` | — | **Refused.** See §5 |
| `PUBLIC ON` / `PUBLIC OFF` | Whether the station may be listed publicly | Recorded |
| `COMPRESSION OFF\|TEXT\|FILES` | What the host wants compressed | Recorded; see §5 |
| `WINLINK SESSION` / `P2P SESSION` | Which kind of session is running | Recorded |
| `CWID ON` / `CWID OFF` | Identify in Morse after a transmission | Recorded; see §5 |
| `CQFRAME` | Sends a `BEACON` frame: this station's callsign, unproto | Refused while a session is running |
| `TUNE <seconds>` / `TUNE OFF` | Put a carrier on the air for tuning | Bounded at 30 s; see §5 |
| `VERSION` | → `VERSION Aether-HF-<version>` | |
| `BUFFER` | → `BUFFER <bytes>` | Payload bytes still to send |

Anything else is answered `WRONG`. A client that got silence could not tell a missing feature
from a hung modem, so nothing is ignored.

---

## 4. Notifications — modem to host

Unsolicited, at any time.

| Line | When |
|---|---|
| `PTT ON` / `PTT OFF` | The transmitter was keyed or released |
| `BUSY ON` / `BUSY OFF` | The busy detector changed its mind about the channel |
| `CONNECTED <mine> <remote> <bandwidth>` | A session came up |
| `DISCONNECTED` | A session ended |
| `BUFFER <bytes>` | The number of payload bytes still to send changed |
| `REGISTERED <call>` | Sent before `CONNECTED` |
| `IAMALIVE` | Every 10 s, so a quiet host knows the modem is there |

`REGISTERED` exists because some clients warn their user about a speed limit unless the modem
says its callsign is registered. Aether is free software with no registration and no speed
limit, so the honest answer to "is this station limited?" is no.

---

## 5. What is accepted, recorded, or refused

The three are deliberately distinguished, and a client can tell them apart.

**Refused** (`WRONG`): `BW500` and `BW2750`. This physical layer has one bandwidth. Accepting
the request and then transmitting 2300 Hz anyway would put a station outside the bandwidth its
operator chose, which is an operator's decision and sometimes a legal one.

**Recorded but not yet acted on**: `COMPRESSION`, `CWID`, `PUBLIC`, `WINLINK SESSION` /
`P2P SESSION`, `TUNE`. The setting is remembered and reported back, and the modem answers `OK`
because the command was understood.

Compression and Morse identification both exist (P3-6) but are configured on the station, not
per host session: compression is negotiated with the *other station* in the connect handshake
and cannot be turned on from one end alone, and whether to identify in Morse is a licence
question for the operator rather than a runtime choice for a client. Wiring these commands to
those settings is an open item.

**`TUNE` does not key the transmitter in this version.** An operator who needs a tuning
carrier must use their radio's own tune function.

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
| Pat | **Not yet verified.** Open item for P3-5 |
| Winlink Express | Not yet verified |
| VarAC | Not yet verified |
| BPQ32 | Not yet verified |

The command set here is implemented from its published behaviour, and every client differs a
little in what it sends and what it tolerates. Until each has been run against a real station,
this table is what the compatibility claim rests on, and it should be read as exactly that.

---

## 8. Open items for v1.0

* Field verification with Pat, then Winlink Express P2P between two instances, then VarAC and
  BPQ32 (`COMMUNITY-CONCERNS.md` §13 adds VarAC to the matrix).
* `TUNE` and `CQFRAME`, which both key a transmitter and therefore need the watchdog and the
  busy detector in the path.
* Compression negotiation (P3-6), which changes what `COMPRESSION` means from recorded to
  acted on.
* Whether `LISTEN OFF` should stop answering calls at the link layer. Today the station always
  answers; the setting is recorded, and the control API says so explicitly rather than
  pretending.
