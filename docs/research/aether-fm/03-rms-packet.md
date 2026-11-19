# RMS Packet with Aether: what works on HF today, and what Aether FM must offer it

Written 2026-11-19 (packet-bench), for the author's question: will Winlink's RMS Packet gateway
work with Aether once Aether FM exists? This note says how RMS Packet reaches a modem, what was
tried on the bench with Aether HF standing in for FM, and what Aether FM must provide. Its rows
in `docs/ROADMAP.md` Phase 10 are P10-9 to P10-12.

Sources: RMS Packet 2.2.4.0's own help file (`RMS Packet.chm`, its pages *Vara FM
Configuration*, *Direct Access Packet TNC Configuration*, *AGWPE Configuration*, *Installation*
and *Loss of Internet*), its revision history, the example `.aps` files it installs, and the
keys of its ini; Winlink Express 1.8.5.0's help (*Packet Setup*, *Making a Packet Connection to
an RMS Packet Server*, *Info on Vara HF and FM*). VARA appears only through its published
documents (the author's rule): EA5HVK, *VARA Protocol Native TNC Commands* (13 February 2022),
and what the programs were seen to send on the bench.

## 1. How RMS Packet reaches a modem

RMS Packet is a VHF/UHF gateway. It serves two kinds of channel at once: a **VARA FM** channel
and **packet** channels ("Vara FM can run at the same time as packet (but not simultaneous
connections)", its revision history). Each channel has a port callsign (by convention the
sysop's call with `-10`), a frequency and an on-air rate, which it reports to Winlink.

| Door | How it is set | What RMS Packet does itself | Aether HF today |
|---|---|---|---|
| **VARA FM** | *Settings › Vara FM settings*: host (127.0.0.1), command port (default 8300; data = +1, not configurable), channel width Narrow/Wide, the modem's path and *Launch Vara FM TNC when RMS Packet starts*, port callsign, frequency | The B2F session over the modem's ARQ link | The VARA-compatible host port answers it (§2) |
| **KISS**, on a COM port or over TCP | *Use direct access to TNC*, TNC type *KISS*, serial port *TCP* (default 127.0.0.1:8100; another port only by editing `TCP Port=` in the ini) | AX.25 2.2 connected mode, in its own KISS driver (G8BPQ's), with the timers of the `.aps` file it names | The KISS port carries the frames as Aether datagrams, between Aether stations only (§3) |
| **KISS ACKMODE** | TNC type *KISS ACKMODE*, the `ExampleKissAckmode.aps` file (ACKTIMER 10 s, MAXFRAMES 4, ACKMODE TRUE, IPOLL TRUE) — "for software modems supporting ACKMODE (UZ7HO Soundmodem for example)" | As KISS, with each frame's T1 started when the TNC says it went out | Honoured (ADR-0019): the acknowledgement comes after the last burst carrying the frame |
| **KISS TNC (Dual Port)**, **KISS Kenwood** | As KISS, two radio ports, or a radio's built-in TNC | As KISS | One port only |
| **Kantronics, AEA/Timewave, SCS PTC** in host mode | TNC type, COM port, `.aps` | The TNC runs AX.25 | Not served (hardware TNCs) |
| **AGW Packet Engine** (or any program that speaks the AGW protocol: Soundmodem, Dire Wolf) | *Use AGW Packet Engine*, host, port (default 8000), *remote* for another program, one callsign for every port | Nothing of AX.25: the engine runs the link and hands RMS Packet connections and data | No AGW port |
| **BPQ32** | *Use BPQ32* | BPQ32 runs the link, in-process | Out of scope (the author's decision, 2026-10-07) |
| **None** | TNC type *(None - No packet channels, Vara FM only)* | — | — |

What RMS Packet needs of the Internet: an on-demand TCP connection to a CMS on 8772/8773 for every
session, status reports to winlink.org on 8775, autoupdate on 8776 (its help's *Required Ports*).
"The program will only function without an Internet connection if the optional local RMS Relay
is available and enabled." So a bench copy runs with *Use RMS Relay* and a radio-only Relay, and
behind a firewall rule, or it would publish its channels under the real gateway's callsign.

## 2. The client side

Winlink Express reaches an RMS Packet gateway two ways:

- **Vara FM Winlink / Vara FM P2P**: host 127.0.0.1, command port 8300 by default. Narrow or Wide
  must match the radio's audio path (Wide needs a 9600 data port).
- **Packet Winlink / Packet P2P**: a host-mode driver for Kantronics and AEA/Timewave TNCs, and a
  native KISS driver for the rest. *KISS* type with *TCP* reaches a soundmodem (default
  127.0.0.1:8100); the TNC model *NORMAL* or *ACKMODE* ("Select ACKMODE when using XKISS, BPQKISS,
  or UZ7HO Soundmodem"). Its defaults: TX delay 300 ms, packet length 128, max frames 4, FRACK
  2 s, persistence 64, slot time 300 ms, retries 10. Connections direct, through one or two
  digipeaters, or by a node script.

Pat (not benched here) has the same two: its VARA FM driver and AX.25 over KISS or AGW.

## 3. On the bench, Aether HF standing in for FM

The bench (`C:\Dev\AetherBench\packet-2026-11\`, not in the repository): two daemons over `[sim]`
at 15 dB and 2 300 Hz; a scratch RMS Packet 2.2.4.0 as the gateway `KK4ODA-10` on the gateway
daemon's host port (VARA FM door) and KISS port (*KISS ACKMODE*, `ExampleKissAckmode.aps`); the
trimode bench's radio-only RMS Relay 3.3.5.0 behind it; an outbound firewall block on the scratch
RMS Packet (it tried to post its channels at every start, and failed). The clients were Winlink
Express 1.8.5.0, Pat 1.0.0 and two scripts (`vara_client.py`, `ax25_client.py` in the bench
folder).

### 3.1 The VARA FM door

| Client | Result |
|---|---|
| Pat 1.0.0, `varafm:///KK4ODA-10` | **Passes.** Secure login, a 6 000-byte random attachment accepted (`FC EM … 6302`, `FS Y`), `FF`/`FQ`, a clean end, 2 min 13 s |
| Winlink Express, *Vara FM Winlink* | **Passes.** Secure login, `FF`/`FQ`, a clean end in under a minute |
| A scripted VARA client | **Passes** to the secure login (no password in the script) |

What each program said, all of it answered `OK` (nothing `WRONG`):

- **RMS Packet**: `MYCALL KK4ODA-10`, `LISTEN ON`; `DISCONNECT` when its Relay ends a session. No
  `BW`, `PUBLIC`, `CWID`, `COMPRESSION` or version check. Its revision history says it stopped
  sending `FM1200`/`FM9600`, restarts itself when `IAMALIVE` stops or the TNC connection drops, and
  keeps a session ten minutes idle.
- **Pat's VARA FM driver**: `PUBLIC ON`, `COMPRESSION TEXT`, `MYCALL`, `LISTEN OFF`, `VERSION`,
  `CONNECT <me> <them>`, `DISCONNECT`.
- **Winlink Express's Vara FM sessions**: `MYCALL`, `LISTEN ON`, `CONNECT <me> <them>`, and
  `DISCONNECT` again every second or two until `DISCONNECTED` comes (Aether holds it for the closing
  identifier, ADR-0075). A scratch copy whose Vara FM TNC was never set up launches the real
  `C:\VARA FM\VaraFM.exe` (its default, auto-launch on) when the session window opens.

VARA FM's published command list (EA5HVK, *VARA Protocol Native TNC Commands*) could not be read in
this session (its copy is on mega.nz, and the browser prompt went unanswered). Comparing it with the
adapter, as ADR-0124 §2.2 asks, is still owed: until then this note rests on what the programs sent.

### 3.2 The KISS door

| Setup | Result |
|---|---|
| RMS Packet on the gateway's KISS port, a scripted AX.25 2.0 client (ACKMODE) on the client's, rung 1 | **Passes**: connect in 14 s, the welcome, the B2F prompt, the client's SID and `FF`, the Relay's answer, the disconnect, 212 s. RMS Packet polled and repeated frames as its 10 s `ACKTIMER` ran out |
| The same at rung 12 | **Passes** in 43 s, nothing repeated |
| Winlink Express 1.8.5.0, *Packet Winlink* (*KISS*, *ACKMODE*, TCP), its default timers (FRACK 2 s, 5 retries), rung 12 | **Passes**: secure login, `FF`/`FQ`, 1 min 12 s |
| The same at rung 1 | **Fails**: connected, the welcome, then Winlink Express gave up after 4 minutes |
| The same with RMS Packet's VARA FM door also on the gateway's host port | **Fails, as built**: RMS Packet never says `CHAT ON`, so Winlink priority (ADR-0019 §2.6) drops every frame it writes to the KISS port |

RMS Packet's KISS driver sends `P` (128), `SLOTTIME` (7) and `TXDELAY` (50) on connecting, and a
window as plain frames with ACKMODE on its last.

Why rung 1 fails Winlink Express: **every AX.25 frame is a transmission of its own**, each with its
own wait for a clear channel and p-persistence (`station/datagrams.rs` feeds one datagram at a time).
RMS Packet's answer and window went out as six transmissions over 62 s; Winlink Express answered
each I-frame with an RR, and those six RRs waited for the channel the whole time (72 s), then went out
as six transmissions of 5.4 s each, five of them already stale. RMS Packet's T1 had run out meanwhile
and it resent the window. A packet TNC sends a window in one keying and AX.25's timers assume it.
Sending a client's queued frames together in one transmission — several datagrams a burst, up to the
key limit — would cut both the waits and the stale answers; that is a change to the datagram queue,
left to the author (§5). **Found and fixed:** a new `[kiss] rung` closed and
reopened the port, and RMS Packet's driver does not connect again, so its packet channel stayed
deaf until it was restarted; the rung and channel access now change in place (ADR-0019 §2.4,
amended).

### 3.3 AX.25-sized frames through the KISS port

Two daemons over `[sim]` at 15 dB and 2 300 Hz, `kiss_rtt.py` (bench folder): a frame handed to
one daemon's KISS port with ACKMODE, the time to its acknowledgement and to its arrival at the
other daemon's port, then a 15-byte answer (an RR's size) back.

| Rung | 15 bytes | 40 | 80 | 146 (an I-frame at PACLEN 128) | 260 | The answer (15 bytes) |
|---|---|---|---|---|---|---|
| 1, tone-36 (the default) | 6.3 s | 13.5 s | 19.0 s | 30.2 s | — | 7.8–8.5 s |
| 12, OFDM | 1.9 s | — | — | 3.6 s | 4.8 s | 3.8–5.7 s |

Every frame arrived; the acknowledgement came 0.1–0.2 s before the frame was handed over at the
other end. The answer's extra seconds are the wait for a clear channel after the frame it answers
and the p-persistence slots.

## 4. What Aether FM must provide for RMS Packet

The roadmap's rows P10-9 to P10-12. Each is a bench gate in step 10.5 (ADR-0124 §5's H3).

| Row | What | Why |
|---|---|---|
| **P10-9** | **The VARA FM door.** The VARA-compatible host port on the FM air, answering what RMS Packet and Pat sent here (§3.1), checked against VARA FM's published command list once read. `CONNECT … VIA` refused (ADR-0124 §2.2). `IAMALIVE` kept. The port pair is whatever the operator sets (RMS Packet: any command port, data = +1) | RMS Packet's VARA FM channel is a host program on this port; today's adapter already carries it (§3.1) |
| **P10-10** | **An AGW port with AX.25 connected mode in the modem.** The AGW protocol as SV2AGW publishes it: the frames RMS Packet uses (registering its one callsign, the port list, connections, data, disconnects, the count of frames outstanding, beacons as unproto frames), default port 8000, several ports, several streams | RMS Packet's own help, the NS7C dual-mode guide and the author's own gateway all use AGW for packet: the guide found RMS Packet's KISS driver collides under load with its default timers. With AGW, AX.25's timers are the modem's, set for the radio it keys |
| **P10-11** | **A KISS TNC in packet mode.** Real AX.25 on 1200 and 9600 baud (ADR-0125), ACKMODE acknowledged when the frame has left the radio (RMS Packet's `ACKTIMER` counts from it), and `TXDELAY`, `P` and `SLOTTIME` honoured as a packet TNC does — on HF they are ignored or Aether's own | RMS Packet's *KISS ACKMODE* channel and Winlink Express's *Packet* sessions |
| **P10-12** | **Both doors on one radio.** RMS Packet runs its VARA FM and packet channels at once (not two sessions at once). Winlink priority must not mute the packet door while the VARA door's program has no session up; a packet frame heard holds an Aether FM call (ADR-0125 K3), and an Aether FM session holds packet frames: the NS7C guide warns that packet stations transmit over VARA FM | The single-radio dual-mode gateway is how RMS Packet is run (NS7C; the author's install) |

Two things outside the modem:

- **The listing.** RMS Packet reports its VARA FM channel to Winlink as `VaraFM_WIDE` or
  `VaraFM_NARROW`. An Aether FM gateway must not be listed as VARA FM (host-interfaces.md §1). RMS
  Packet has a custom service code (*Site Properties*), so `AETHER` keeps it out of `PUBLIC`, but the
  mode in the listing still reads VARA FM. That needs Winlink's agreement: the author's decision.
- **Narrow and Wide.** RMS Packet's *Channel width* (stored as *RF Baud Rate* 1200 or 9600) is the
  audio path: the voice path or the 9600 data port. In Aether FM that is the profile (voice or flat,
  ADR-0124 §2.1), set at the modem. The finder (`hostapps/packet.rs`) reads it into the setup's
  `session` ("Vara FM Wide"), so an import can suggest the profile.

## 5. Decisions for the author

1. **An AGW port** (P10-10): ADR-0125 §2.4 already plans one for FM. On HF it would carry AX.25 over
   Aether datagrams, as the KISS port does — slow (§3.3), and useful only on the bench. Build it with
   Aether FM, not before?
2. **Winlink priority with one program on both doors.** On HF today an RMS Packet with both doors on
   one Aether loses its packet door (§3.2), as VARA's published KISS note says it should. Keep that
   on HF and change it only for FM's packet mode (P10-12)?
3. **The listing** of an Aether FM channel behind RMS Packet (§4): ask Winlink for a mode of its own?
4. **A window in one transmission** (§3.2): let the KISS port's queued frames from one client go
   together, several datagrams in a burst, as a packet TNC sends a window? It would make AX.25 over
   Aether HF's KISS port usable at the default rung; Aether FM's packet mode (P10-11) needs it anyway.
