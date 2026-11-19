# RMS Packet with Aether

RMS Packet is Winlink's gateway program for VHF and UHF. It serves two kinds of channel: a
**VARA FM** channel and **packet** channels (1200 or 9600 baud AX.25). Aether can stand behind
both of its doors today, but only on Aether HF's own signal. Its users are on VHF and UHF with
VARA FM or packet radios, and Aether HF cannot reach them. So:

- **Today:** RMS Packet works with Aether on the bench. Use it to learn the setup, not on the air.
- **On the air:** wait for Aether FM (Phase 10 of the roadmap). What Aether FM must offer RMS
  Packet is in [the FM plan's RMS Packet note](../research/aether-fm/03-rms-packet.md).

**Do not put RMS Packet on the air with Aether HF.** RMS Packet reports its VARA FM channel to
Winlink as *VARA FM*, on the frequency you give it. A VARA FM station that called it would get no
answer, and Winlink would list a gateway that cannot serve its users. Aether gateways are listed
under the service code `AETHER`, never as VARA (see [the manual](../manual/manual.md#finding-an-aether-gateway)).

## How RMS Packet reaches a modem

| RMS Packet setting | What it connects to | Aether today |
|---|---|---|
| *Settings › Vara FM settings* | A VARA-style TNC: host, command port (default 8300; the data port is the next) | Aether's **host programs** port answers it (Setup step 5) |
| *Packet Channels*, TNC type *KISS* or *KISS ACKMODE*, serial port *TCP* | A KISS TNC over TCP (default 127.0.0.1:8100) | Aether's **KISS** port answers it (Setup step 5). Frames go as Aether datagrams, heard by Aether stations only |
| *Site Properties › Use AGW Packet Engine* | An AGW-protocol program (default port 8000) | Not served: Aether has no AGW port |
| *Site Properties › Use BPQ32* | BPQ32 | Not served |
| *Packet Channels*, a Kantronics, AEA/Timewave or SCS TNC | A hardware TNC on a COM port | Not Aether's |

RMS Packet runs its VARA FM channel and its packet channels at the same time.

## Setting it up (the bench)

1. In Aether's Setup step 5, turn on **Host programs** (port 8300) and, for packet, **KISS
   programs** (port 8100).
2. **VARA FM door:** in RMS Packet, *Settings › Vara FM settings*: tick *Enable Vara FM TNC*, host
   `127.0.0.1`, command port `8300`, the port callsign (for example `N0CALL-10`). Untick *Launch
   Vara FM TNC when RMS Packet starts*, or point it at a program that does nothing, and start
   Aether yourself.
3. **Packet door:** *Site Properties*, choose *Use direct access to TNC*. Then *Settings › Packet
   Channels*: TNC type **KISS ACKMODE**, serial port **TCP**. RMS Packet always connects to port
   8100 on 127.0.0.1 unless you edit `TCP Host=` and `TCP Port=` under `[TNC Properties]` in
   `RMS Packet.ini`, with RMS Packet closed (it rewrites the file when it closes).
4. Without the Internet, RMS Packet needs **RMS Relay** (*Site Properties › Use RMS Relay*), or it
   turns every caller away.

**Use one door at a time on one Aether station.** RMS Packet never sends `CHAT ON` on the VARA
door. While a program holds that door without it, Aether sends no frame from the KISS port
(*Winlink priority*, as VARA does), so the packet door goes deaf. The KISS line in Setup says
*holding frames* when this happens.

## What was tried (2026-11-19)

Two Aether daemons joined by the simulated channel at 15 dB and 2300 Hz; a scratch RMS Packet
2.2.4.0 as the gateway `KK4ODA-10`, with a radio-only RMS Relay 3.3.5.0, kept off the Internet.

| Door | Client | Result |
|---|---|---|
| VARA FM | Pat 1.0.0, its VARA FM mode | **Passes.** Secure login, a message with a 6 kB attachment accepted (`FS Y`), `FF`/`FQ`, a clean end, in 2 min 13 s. RMS Packet said only `MYCALL` and `LISTEN ON`, and Aether answered both `OK` |
| VARA FM | Winlink Express 1.8.5.0, *Vara FM Winlink* | **Passes.** Secure login, `FF`/`FQ`, a clean end in under a minute. It said only `MYCALL`, `LISTEN ON`, `CONNECT` and `DISCONNECT` |
| VARA FM | A scripted VARA client | **Passes.** RMS Packet's welcome, its B2F prompt and the secure login challenge |
| KISS ACKMODE | A scripted AX.25 client on the other daemon's KISS port | **Passes.** Connect, RMS Packet's welcome, its B2F prompt, the client's reply, the Relay's answer, the disconnect. At rung 1 (the default) it took 212 s and RMS Packet repeated frames when its 10 s timer ran out; at rung 12, 43 s with none repeated |
| KISS ACKMODE | Winlink Express 1.8.5.0, *Packet Winlink*, TNC type *KISS*, model *ACKMODE*, TCP to the other daemon's KISS port, its default timers | **Passes at rung 12** (1 min 12 s, secure login, `FF`/`FQ`). **Fails at rung 1**: connected and got the welcome, then gave up after 4 minutes (FRACK 2 s, 5 retries) |
| KISS ACKMODE with the VARA FM door on | The same | **Fails, by design:** Winlink priority holds every frame RMS Packet writes to the KISS port |

## Packet timing on Aether HF

Each AX.25 frame is a datagram, sent on its own with its own wait for a clear channel. Measured on
the bench, from handing a frame over to its arrival at the other station:

| Frame | Rung 1 (default) | Rung 12 |
|---|---|---|
| An RR (15 bytes) | 6–8.5 s | 2–6 s |
| An I-frame of 128 bytes of data | 30 s | 3.6 s |

RMS Packet's KISS ACKMODE file waits 10 s for an answer after a frame has gone (`ACKTIMER 10`).
Winlink Express's default FRACK is 2 s. At rung 1 an answer takes about 8 s, so timers this short
repeat frames. Use **KISS ACKMODE**, and a faster KISS rung if both stations run 2300 Hz on a good
path.

Each frame is its own transmission, and each waits for a clear channel. While RMS Packet sends a
window of frames, the other station's answers wait; they then go out one transmission each. At
rung 1 Winlink Express's six RRs waited 72 s and took another 37 s on the air, and RMS Packet had
resent its window meanwhile. AX.25 expects a TNC to send a window in one keying; Aether's KISS port
does not yet.
