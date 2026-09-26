# KISS programs: APRS, packet and VarAC's broadcasts

Aether HF has a **KISS port** that answers the way VARA HF's does, so a program set up to use
VARA's KISS port can use Aether without being changed: point it at the same address and port.
Every frame the program hands over goes out on the air as its own transmission, and every frame
Aether hears from another station goes to every program connected to the port.

This is for programs that send **frames**, not sessions: APRS clients, packet programs, and
VarAC's broadcast messages. Winlink Express and VarAC **sessions**, Pat and BPQ32's VARA driver
use the VARA-compatible *host* interface instead (Setup → *Host programs*; port 8300), which is
Aether's ARQ link with retransmissions and rate control. A packet program *can* run its own
AX.25 connected sessions over the KISS port, but at HF datagram speeds that is slow — see
*Limitations*.

## Turning it on

In the panel, **Setup → 5. Application settings → KISS programs**:

1. Tick **let APRS and packet programs send and receive frames**.
2. Leave the address at **127.0.0.1** and the port at **8100** unless you have a reason not to.
   8100 is VARA HF's KISS port. If VARA HF or a soundmodem (UZ7HO uses 8100 too) is running on
   the same computer, one of them has to move; the panel says *not listening* and why, and Aether
   tries the port again every ten seconds, so closing the other program is enough.
3. **KISS frames go at**: rung **1 — tone-36** is the default and the right choice for most
   uses: it is the tone floor, which stations of *either* bandwidth decode (a 500 Hz station and
   a 2 300 Hz station both hear it). Rung 0 (tone-24) is the most robust and slowest. A faster
   rung carries more, sooner — but only to stations running the same bandwidth, on a path good
   enough for it.
4. **Wait for a clear channel** (on by default) holds a frame until the busy detector says the
   channel is clear, then waits a random number of slots (the program's KISS *P* and *SLOTTIME*,
   0.25 and 100 ms unless it sets them), as a KISS TNC does.
5. **Save these settings**. The port opens, moves or closes at once — no restart.

The line under the settings says whether the port is listening, where, and which programs are
connected, with a **Disconnect** button for each and one for all. The header shows a small chip
while a host or KISS program is using the station; the Diagnostics tab has a *KISS* reading with
the frames in and out.

The equivalent `station.toml`:

```toml
[kiss]
enabled = true
bind = "127.0.0.1:8100"
rung = 1              # tone-36: heard by stations of either bandwidth
wait_for_clear = true
max_clients = 4
trace = false         # log each frame's type, length and fate (never its contents)
```

**Security.** The KISS port asks no password: whoever can connect to it can make your station
transmit. Keep it on 127.0.0.1 unless the program runs on another computer of yours, on a
network you control. The panel warns about any other address, and so does the log when the port
opens.

## Setting up programs

Menu names differ between programs and versions; the setting to look for is always a KISS TNC
reached over **TCP/IP** — the choice a program offers for Dire Wolf or a soundmodem — with the
host `127.0.0.1` and the port `8100`. None of these has been tried against Aether yet (see
*Status*).

### VarAC

VarAC uses both of Aether's ports: its sessions and pings go through the host interface (8300),
and its **broadcast messages** through the KISS port (8100).

1. In Aether: turn on **Host programs** (port 8300) and **KISS programs** (port 8100), and set
   Setup step 4's bandwidth to 500 Hz for VarAC's calling frequencies.
2. In VarAC's VARA HF modem settings: the command port 8300 and the KISS port 8100 — its
   defaults. Leave VarAC's *launch VARA* option off; Aether is started by its own application.
3. VarAC says `CHAT ON` when it starts, which is what allows the KISS port to transmit while it
   holds the host interface (see *Winlink priority* below), and `IGNOREKISSDCD ON` when its
   *Ignore DCD* option is ticked, which sends broadcasts without waiting for a clear channel.

VarAC's broadcasts are AX.25-shaped frames with eight-byte address fields, sent as KISS frame
type 1; Aether sends them back to the receiving VarAC as type 1, byte for byte, which is what it
reads. The SNR VarAC shows beside a broadcast comes from the `SN` line Aether sends on the host
interface for every frame it decodes.

### APRS programs

APRS over HF is AX.25 UI frames: a position, a status or a message, each one transmission. Set
the program's TNC or port to **KISS over TCP/IP**, host `127.0.0.1`, port `8100`:

* **APRSIS32**: a new port of type **KISS**, configured for **TCP/IP** with the host and port.
* **YAAC**: a port of type **KISS-over-TCP**.
* **PinPoint APRS**: a KISS TNC over TCP/IP.
* **APRSdroid** (on a phone, with Aether on the same network — so not on 127.0.0.1; read the
  *Security* note): the connection protocol **TNC (KISS)** over **TCP/IP**, the computer's
  address and 8100.
* **Xastir** has no KISS-over-TCP interface of its own: bridge a pseudo-terminal to the port with
  `socat` and give Xastir a *Serial KISS TNC* on the pseudo-terminal —
  `socat pty,link=/tmp/aether-kiss,raw,echo=0 tcp:127.0.0.1:8100`.

Keep beacons infrequent. At tone-36 a typical position report (60–70 bytes) takes three frames,
about 16 s of air, and on HF a channel carries one transmission at a time.

### Winlink Express

For Winlink, use a **Vara HF** session in Winlink Express (Setup → *Host programs*): that is
Aether's ARQ link, and it is far faster than AX.25 over datagrams. The KISS port is there for
Winlink Express's **Packet** sessions only if you want to try them:

1. Open a *Packet Winlink* or *Packet P2P* session and, in its TNC settings, choose a KISS TNC
   reached over TCP/IP (the same choice as for Dire Wolf or a soundmodem), host `127.0.0.1`,
   port `8100`.
2. Aether honours **ACKMODE**, which Winlink Express uses to learn when a frame has actually left
   the radio: the acknowledgement comes back after the last burst carrying the frame has been
   played, so its AX.25 timers start from the real end of the transmission.

### BPQ32 and QtTermTCP

A KISS port over TCP in BPQ32's `bpq32.cfg`:

```
PORT
 ID=Aether HF KISS
 TYPE=ASYNC
 PROTOCOL=KISS
 IPADDR=127.0.0.1
 TCPPORT=8100
 CHANNEL=A
ENDPORT
```

QtTermTCP's KISS ports take the same host and port. Both may use ACKMODE, which Aether honours.
BPQ32's *VARA* driver is the host interface (port 8300), not this.

## What Aether does with a frame

* A frame from a program is joined, as a **datagram**, by the sending station's callsign — so
  every transmission identifies the station, whatever the program's frame holds — and its type,
  split into fragments of the chosen rung (33 bytes each at tone-36, at most sixteen), and sent in
  bursts that each fit the key limit.
* It waits while a session is up (the session's turn-taking has no room for it), then for a clear
  channel, then for its p-persistence slot.
* It goes through the same **regulatory gate** as everything else (`fcc-regulatory-controls.md`):
  as a transmission your station originates. An automatically controlled station sends datagrams
  only where it may originate, and an **answer-only** station (`[radio] answer_only`) sends none
  — a datagram starts an exchange; what it hears still goes to its programs.
* A station that decodes every fragment hands the frame to its KISS programs with the type it was
  sent with. A fragment lost is not retransmitted: the datagram is dropped after two minutes, and
  the program's own protocol (APRS's repeats, AX.25's retries) does what it does on any lossy
  channel.
* Nothing waits forever: sixteen datagrams can wait at most; past that the port stops reading the
  program until there is room, so a fast program is slowed down, not cut off.

**Winlink priority.** While a program holds Aether's host interface without having said
`CHAT ON`, frames from KISS programs are **not sent** — so APRS beacons cannot key the radio in
the middle of a Winlink station's listening, as VARA does it. Winlink Express never says
`CHAT ON`; VarAC always does. Frames heard still go to the KISS programs. The KISS line in Setup
says *holding frames* while this is in force.

## Commands and frame types

The byte after the opening `FEND` is read the way VARA's KISS port reads it, and standard KISS
parameters are told apart from VARA's frame types by their length:

| Byte | What it means here |
|---|---|
| `00` + frame | send it (an AX.25 frame) |
| `01` + one byte | TXDELAY: ignored (Aether keys with its own lead) |
| `01` + frame | send it, type 1 (AX.25 with eight-byte addresses: VarAC) |
| `02` + one byte | P: the program's p-persistence |
| `02` + frame | send it, type 2 (unformatted data) |
| `03` + one byte | SLOTTIME: the program's slot, 10 ms units |
| `04`, `05`, `06` | TXTAIL, FULLDUPLEX, SETHARDWARE: ignored |
| `0C` + two bytes + frame | ACKMODE: send it; the two bytes come back once it has gone |
| `FF` | RETURN: ignored |
| any other port nibble | refused (Aether has one port, 0) |

## Diagnostics

* **Setup** shows the port's state, each program (what it seems to be, where from, its frames in,
  out and dropped) and why frames are being held.
* **Log**: lines tagged `kiss` — the port opening and closing, programs connecting and leaving,
  malformed or refused frames (once per kind, not per frame), a datagram heard.
* **`trace = true`** logs every frame's type, length and fate. Frame **contents** are never
  logged, with or without it.
* `tools/kiss_test_client.py` is a KISS client for trying the port by hand, with no other
  software: `listen`, send an AX.25 UI frame (`ax25 N0CALL APZAET ">hello"`), raw bytes,
  escapes, a burst of frames, and the parameter commands.
  `python tools/kiss_test_client.py --port 8100 --listen 30 ax25 N0CALL APZAET ">test"`.

## Limitations

* **Only other Aether stations hear it.** The frames go on Aether's waveform; a VARA station, a
  300-baud packet station or an APRS digipeater does not decode them. Aether is compatible with
  the *software*, not the air.
* **Slow.** tone-36 carries 54 bit/s; a 60-byte frame is 16 s of air. AX.25 connected-mode
  sessions (packet BBSes, Winlink Packet) work in principle but need long timers and patience;
  use the host interface for sessions.
* **No datagrams during a session.** A frame waits until the session ends.
* **No repeats.** A lost fragment loses the frame; the program repeats what needs repeating.
* **One port.** A frame for KISS port 1–15 is refused.
* **A one-byte type-2 frame** is read as the P parameter — the price of serving VARA's frame types
  and standard KISS on one port.
* **Not verified on the air yet**, and not yet with VarAC, Winlink Express Packet, BPQ32 or the
  APRS programs on the bench: the test suite's own clients and `kiss_test_client.py` between two
  daemons are what the claim rests on so far.

## Status

| Program | Status |
|---|---|
| Aether's test suite (framing, escapes, partial and joined reads, the type bytes, ACKMODE, backpressure, several clients, a stress run of four clients at once, Winlink priority, two daemons over `[sim]`) | passing |
| `tools/kiss_test_client.py`, two daemons over `[sim]` | passing (2026-09-25) |
| VarAC, Winlink Express Packet, BPQ32, QtTermTCP, APRSIS32, YAAC, PinPoint APRS, APRSdroid, Xastir | not yet tried |

Reports are welcome — what the program sent, what it expected, and a log with `trace = true`.
