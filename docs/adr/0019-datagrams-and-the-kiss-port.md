# ADR-0019: Datagrams, and a KISS port that answers as VARA's does

**Status:** accepted, 2026-09-25. Model first for the frame format and the engine's rule
(`aether_model/link/datagram.py`, `frames.py`, `engine.py`), then the port (`aether-link`
`datagram.rs`, `frames.rs`, `engine.rs`) and the daemon (`core/aetherd/src/kiss/`,
`station/datagrams.rs`). Configuration schema 7. The link protocol is unchanged: a station of an
earlier version ignores the new DATA kind, and its sessions with this one are what they were.

## 1. Context

A large part of what amateurs run over a data modem is not a session. APRS clients send positions,
statuses and messages as single AX.25 UI frames; packet programs hand a TNC AX.25 frames and run
their own protocol over them; VarAC's broadcast messages are one frame to whoever hears it. All of
them talk to their modem over **KISS** (Chepponis and Karn, *The KISS TNC*, ARRL 6th Computer
Networking Conference, 1987): `FEND`-delimited frames with a type byte, over a serial line or a
TCP socket, one transmission per frame, no acknowledgement — the program does any repeating its
protocol needs.

VARA HF has a KISS port beside its command and data ports, and the programs that use it expect it
to behave as VARA's does, which is not quite standard KISS. What was learned from public sources
(VARA's published documentation and EA5HVK's *VARA KISS Interface*, 2024; the programs' own
manuals and forums; nothing from VARA's internals):

* **The port**: TCP 8100 by default (VARA's command and data ports being 8300/8301). Dire Wolf
  listens on 8001, UZ7HO's soundmodem on 8100, QtSoundModem on 8105; programs set up for VARA use
  8100.
* **The type byte** after `FEND` is a *frame type*, not a command: 0 an AX.25 frame, 1 an AX.25
  frame whose address fields are eight bytes (seven characters and the SSID byte — what VarAC
  sends), 2 unformatted data. 1 and 2 are TXDELAY and P in standard KISS.
* **Each frame is one broadcast**, with no acknowledgement, sent on the 500 Hz waveform so that a
  station of any bandwidth decodes it.
* **Channel access**: VARA waits for a clear channel (its DCD) and applies p-persistence; a host
  on the command port can turn the wait off with `IGNOREKISSDCD ON`.
* **"Winlink priority"**: while a program holds the command port, the KISS port does not transmit
  unless that program said `CHAT ON` — so a Winlink station's listening is not interrupted by APRS
  beacons. VarAC says `CHAT ON` on every start; Winlink Express does not.
* **`SN <dB>`** on the command port for every KISS frame received: VarAC shows a broadcast's SNR
  from it.
* **Several KISS clients** at once (VARA 4.8.0 and later).
* **What clients need back**: VarAC reads its type-1 frames at their exact length and type;
  APRSdroid expects type `0x00`. Winlink Express Packet, BPQ32 and QtTermTCP send the standard
  parameters (TXDELAY, P, SLOTTIME, TXTAIL, FULLDUPLEX) and may use **ACKMODE** (type `0x0C`, a
  two-byte identifier returned once the frame has left the radio: G8BPQ's extension, how a program
  paces AX.25 over a slow modem).
* **What does not use KISS**: Winlink Express's VARA sessions, VarAC's chat and pings, Pat, and
  BPQ32's VARA driver all use the command and data ports — Aether's host interface since P3-5.

Aether had nothing between a session and a beacon: no way to carry another program's frame.

## 2. Decision

### 2.1 A datagram on the air

A new DATA kind, `DATAGRAM` (6), carries a piece of another program's frame outside any session,
with no acknowledgement (`air-interface.md`). The header's two session bytes number the pieces:
`seq` is the fragment's index (high nibble) and the last index (low nibble), so a datagram is at
most sixteen fragments; `session` is the datagram's number, 1–255, advanced by the sender for every
datagram so that pieces of two are never joined. The joined body is the **sender's callsign** (7
bytes, packed), the **frame type** (1 byte: 0, 1 or 2 as VARA names them) and the frame. The
callsign is there because every transmission must identify its station (§97.119) whatever the
program's frame holds — an unformatted type-2 frame holds nothing — and because the receiving
station lists the sender among the stations heard.

Every fragment but the last is a full frame; the last is partial, with the explicit length; a
remainder exactly one byte too long for a partial frame (a full frame less one) goes as two
fragments, the rule ADR-0008's work found for session data. A receiver joins pieces by
`(number, last)`, hands the frame on once every piece has arrived, keeps at most eight datagrams
waiting, and drops one that has waited **two minutes** — nothing is retransmitted, so a lost piece
is not coming.

The rung is the station's choice (`[kiss] rung`), **1 by default: tone-36**, the tone floor, whose
frames are the same in both bandwidths (ADR-0013). That is VARA's reason for sending KISS frames on
its 500 Hz waveform, and it matters more here: an APRS or VarAC broadcast is for *whoever* hears it,
and a 500 Hz station and a 2 300 Hz station both decode the floor.

### 2.2 Never session data

`BEACON`, `PROBE` and `PROBE_ACK` carry session id 0 and so never matched a session; a `DATAGRAM`'s
number is 1–255, and one could match a session's id. The engine's `_accept` (model first, then the
port) now drops **every kind sent outside sessions** — `OUTSIDE_SESSIONS`, `DataKind::outside_sessions`
— before it looks at the session id. `test_a_datagram_numbered_like_the_session_is_not_session_data`
failed without the rule. A station of an earlier version decodes kind 6 as an unknown kind and drops
the frame, so it is equally safe.

### 2.3 The station's queue, and the way to the air

The station keeps a queue of its own for datagrams (`station/datagrams.rs`; `DATAGRAM_QUEUE` = 16).
A datagram is fed into the transmit queue only when that is empty, the engine is idle and no probe
is out: **a session's turn-taking has no room for a stranger's burst**, so datagrams wait for the
session to end. Then its channel access is the client's, as a KISS TNC's is — the busy detector
(unless the client's host said `IGNOREKISSDCD ON` or the configuration turns it off), then a
p-persistence draw each slot (KISS's own P and SLOTTIME, 0.25 and 100 ms by default) — and the
generic `wait_for_clear` hold does not apply twice. Its fragments go in bursts that each fit the
key (`burst_limit_s`, ADR-0017).

It reaches the air through the **regulatory gate** like every other transmission (ADR-0018), as a
transmission this station originates, described as "a KISS client's datagram"; an automatically
controlled station sends datagrams only where it may originate. A refusal, a burst cut short and a
queue dropped with the port are reported (`datagram-sent`, `sent: false`, with the reason).

### 2.4 The KISS port is a client of the control API

As the VARA-compatible adapter is (P3-5), the KISS server (`core/aetherd/src/kiss/`) is a client
of the modem's own API: `datagram.send` for each frame, the `datagram` event for each one heard,
`datagram-sent` for each one that left. It has no privileged access to the station and can do
nothing a script could not; the compatibility layer is the part most likely to change as programs
are tried against it, and it cannot destabilise the modem under it.

* `framing.rs`: KISS framing, escapes (the type byte escaped too), frames reassembled across TCP
  reads and split out of one, a 2 048-byte limit, malformed frames dropped and counted with the
  decoder resynchronising at the next `FEND`.
* `dialect.rs`: the VARA reading of the type byte (§2.5).
* `server.rs`: the listener, up to `max_clients` (4) clients, a hub that sends every frame heard to
  every client with its type and exact length, ACKMODE acknowledgements from `datagram-sent`, and
  **backpressure**: a `queue_full` refusal makes the client's reader retry every 250 ms without
  reading its socket, so TCP holds the rest and nothing is dropped for arriving early.

The server's settings are live: a changed `[kiss]` restarts it in place (`sync_kiss`), and turning
it off closes every client and clears the datagrams waiting.

### 2.5 The type byte, read as VARA reads it

Both kinds of client connect to a port like this one — VarAC sending type-1 frames, BPQ32 sending
TXDELAY — and the two readings collide at 1 and 2. They are told apart by **length**: TXDELAY and P
carry exactly one byte; a VARA type-1 frame carries at least sixteen bytes of addresses, and a
type-2 frame of one byte is not something any known program sends. P and SLOTTIME set the client's
channel access; TXDELAY, TXTAIL, FULLDUPLEX, SETHARDWARE and RETURN are accepted and ignored (the
modem keys with its own lead and tail, is half duplex, and is a TCP port); ACKMODE is honoured; a
frame for a port other than 0 is refused.

### 2.6 Beside the host interface

The host server and the KISS server share `HostFlags`: whether a host is attached, whether it said
`CHAT ON`, and whether it said `IGNOREKISSDCD ON`. While a host is attached without `CHAT ON`,
frames from KISS clients are **not sent** — dropped and counted, with one log line — as VARA's
Winlink priority has it; received frames still go to the clients. `status.kiss.paused` says so.
Every decoded frame already produced `SN` on the command port; a datagram's frames do too.

### 2.7 Security and diagnostics

The port asks no password, so it is **off by default** and listens on **127.0.0.1:8100**; another
address is logged as a warning, reported as `status.kiss.exposed` and shown in the panel. The log
(tag `kiss`) says when the port opens and closes, clients come and go, a frame is malformed or
refused (once per kind), and a datagram is heard or fails to go — lengths, types and callsigns,
**never contents**; `trace = true` adds every frame's fate.

### 2.8 Configuration and the panel

`[kiss]`: `enabled`, `bind`, `rung` (0–19), `wait_for_clear`, `max_clients` (1–16), `trace` — all
live, all in the settings registry and so in profiles (ADR-0011), except `bind`: where the port
listens is this computer's (`Scope::Machine`), so a profile made elsewhere never opens it to a
network. The panel's
Setup step 5 has *KISS programs* (enable, address, port, the rung, wait for a clear channel, the
port's state, the clients with a Disconnect each and one for all, the warning for an exposed
address); a chip in the header while a host or KISS program is attached; a KISS reading on the
Diagnostics tab; "KISS frame" as an activity among the stations heard.

## 3. Alternatives considered

* **Standard KISS only** (the type byte as port and command). Rejected: VarAC's broadcasts are
  type 1 and would be read as TXDELAY and dropped, and the whole point is that a program set up
  for VARA needs no change.
* **A datagram as a one-frame session** (connect, send, disconnect). Rejected: a broadcast has no
  one to connect to, and a handshake would triple the air time of every APRS beacon.
* **Datagrams inside a session**, between the session's bursts. Rejected for now: the ARQ's
  timers are sized for the other station's bursts, and a stranger's burst in the gap would be
  taken for the channel failing. A station in a session holds its datagrams until it ends.
* **Repeating each datagram** for robustness. Rejected: KISS programs repeat by their own rules
  (APRS's decaying beacons, AX.25's retries), and a modem that doubled every frame would double a
  busy channel's load behind their backs.
* **The session's own OFDM rungs by default**. Rejected: they reach only stations of the same
  bandwidth, and a broadcast is for everyone. The rung is configurable for a network that knows its
  stations.

## 4. Consequences

* Programs set up for VARA's KISS port can be pointed at Aether unchanged — within what an
  Aether-only air allows: **only Aether stations hear the frames** (no VARA, 300-baud packet or
  APRS digipeater does).
* It is slow: tone-36 carries 33 bytes a fragment in 5.4 s, so a 60-byte APRS position is 16 s of
  air, and the longest datagram at tone-36 is a 520-byte frame. AX.25 connected sessions over it
  work in principle and need long timers; the host interface is the way to run sessions.
* An older station ignores datagrams and interoperates otherwise.
* Configuration schema 7, a step that changes nothing: a version from before the KISS port
  cannot read a file with a `[kiss]` table (every table refuses keys it does not know), and the
  new number is what sends it, and the shell's restore, to the copy kept before
  (`station.toml.bak-v6`) rather than leaving the station unable to start.
* Open: VarAC, Winlink Express Packet, BPQ32, QtTermTCP and the APRS programs on the bench and the
  air (`docs/spec/host-interfaces.md` §8.6, `docs/user/kiss.md`).

## 5. Compatibility

| Program | Uses | What Aether does | Verified |
|---|---|---|---|
| VarAC (broadcasts) | KISS 8100, type 1; `CHAT ON`, `IGNOREKISSDCD` on 8300 | type 1 both ways at exact length; honours both commands; `SN` per frame | not yet |
| VarAC (chat, ping) | host 8300/8301 | the host interface (P3-5; `host-interfaces.md` §7) | bench, 500 Hz |
| Winlink Express (Vara HF sessions) | host 8300/8301 | the host interface; Winlink priority holds KISS frames while it is attached | bench |
| Winlink Express (Packet) | KISS over TCP, parameters, ACKMODE | parameters read or ignored; ACKMODE after the last burst | not yet |
| Pat | host 8300/8301 | the host interface | bench |
| BPQ32 / QtTermTCP | KISS over TCP (`IPADDR`/`TCPPORT`), parameters, ACKMODE; VARA driver on 8300 | as Winlink Express Packet | not yet |
| APRSIS32, YAAC, PinPoint APRS | KISS over TCP, type 0 | type 0 both ways | not yet |
| APRSdroid | KISS over TCP (network), expects `0x00` | type 0; needs a non-loopback bind (warned) | not yet |
| Xastir | serial KISS | via `socat` to a pseudo-terminal | not yet |
| `tools/kiss_test_client.py` | KISS over TCP, every command | — | bench, two daemons over `[sim]` |
