# Winlink through an Aether gateway

Aether as the modem at both ends of a Winlink session: **RMS Trimode** at the gateway and
**Winlink Express** in the field. This page is the first on-air test's own checklist — the two
stations of one operator, a gateway at home with nobody at it and a client in a vehicle — written
so that any sysop can follow it. What it rests on is the bench
(`docs/spec/host-interfaces.md` §7): a Winlink Express client completes a B2F session through an
RMS Trimode fed by an Aether station, over the simulated channel. On the air it has not been done
yet; this is that test.

**Three things first.**

* **The gateway program is RMS Trimode, not RMS Packet.** RMS Packet is Winlink's VHF/UHF
  gateway: AX.25 packet and VARA FM. RMS Trimode is the HF gateway, and the one that drives a
  VARA HF modem — which is what Aether is to it. The two run side by side on one computer if you
  already have a packet gateway there.
* **Aether talks only to Aether.** The gateway's channel must never appear in Winlink's public
  channel list: a VARA station that picked it could not connect, and would blame VARA. It is
  published under a **private service code** instead (§2.4), which only a client told the code
  can see.
* **Both stations run the same Aether release.** A release that changes the link protocol says
  *Update required* in its notes, and stations across one cannot connect. Check *Help › About*
  at both ends before the day.

The two stations in this guide:

| | The gateway | The client |
|---|---|---|
| Where | at home, nobody at the radio | the vehicle, the operator at the radio |
| Programs | Aether, RMS Trimode | Aether, Winlink Express |
| Callsign | Site Setup the base callsign `KK4ODA`; the channel the station's own, `KK4ODA-1` (§2.2) | the plain registered callsign, `KK4ODA` |
| Control (Setup step 1) | **Automatic** | **Local** |
| Who keys the radio | Aether — CAT, a serial line, a CM108 pin, rigctld or FLRig | Aether |
| Bandwidth (Setup step 4) | 2300 Hz | 2300 Hz |
| Dial | an automatic-control dial (§2.3) | the same |

---

## 1. What happens in a session

Winlink Express tells its Aether to call the gateway (`CONNECT KK4ODA KK4ODA-1`). The gateway's
Aether answers while Trimode is listening, and both programs hear `CONNECTED`. From there the
session is Winlink's own B2F between Winlink Express and Trimode, carried over the Aether link:
Trimode's welcome banner, the proposals, the message, `FF`/`FQ`. Trimode hands the message to the
Winlink servers (the CMS) over the Internet, and anything waiting for the client comes back down
in the same session. Each station then sends its Morse identifier, and the programs hear
`DISCONNECTED` after it. A short message took about 35 s on the bench at 15 dB.

Nothing in Trimode or Winlink Express knows it is not VARA: both set the modem up as they do
VARA's, on the same two TCP ports.

---

## 2. Before the day

### 2.1 Winlink's side

* The gateway's **base callsign** needs Winlink's *Gateway Authorized* flag, which the Winlink
  team sets on request (a sysop already running an RMS Packet gateway under the same base
  callsign has it). Trimode logs in to the CMS with that callsign's Winlink password.
* The client's callsign is the **plain registered one**, with its password. No SSID in Winlink
  Express's own callsign: it proposes no messages to one, and the session ends with nothing sent.

### 2.2 The gateway's callsigns

Trimode asks for two. Its **Site Setup** takes the **base callsign** (`KK4ODA`): the Winlink
account it logs in to the CMS with, the one the *Gateway Authorized* flag is on. Each **channel**
then has a callsign of its own, and that is the one on the air: give it the gateway Aether
station's **own callsign** (`KK4ODA-1` here). Trimode's `MYCALL` reaches the modem and is answered
to in any case, and one name keeps the heard list, the sessions list, the recordings and the
field log together. Not the packet gateway's SSID — that is another station on Winlink's map —
and not the client's.

Set up this way on 2026-10-24, not yet tried on the air. The host trace (§3.1) shows which
callsign Trimode hands the modem in `MYCALL`; the client calls that one.

### 2.3 Frequency and bandwidth

The gateway runs with nobody at it, so it is under **automatic control** (§97.221, Setup
step 1). Every Aether signal measures more than 500 Hz, so an automatic station transmits only
inside the §97.221(b) sub-bands; anywhere else the rules badge says *TX blocked* and nothing goes
out ([fcc-regulatory-controls.md](fcc-regulatory-controls.md), *Automatic control*). The dial list
on the Session tab proposes one dial on each band inside them ([frequency-plan.md](frequency-plan.md)):

| Band | Sub-band | Dial (USB) |
|---|---|---|
| 80 m | 3.585–3.600 MHz | **3.590 MHz** |
| 40 m | 7.100–7.105 MHz | **7.101 MHz** |

For a path of tens of miles by day, 80 m first; 40 m is the second try. The first mobile ↔ home
sessions of this project (2026-09-17, 40 m at 500 Hz) were a marginal path, and the 2300 Hz
waveform wants more signal than the 500 Hz one.

Both stations run **2300 Hz** (Setup step 4): the Winlink bandwidth. Trimode's channel is set to
2300 Hz and Winlink Express's session to 2300 as well. Winlink Express's 2750 runs as 2300 on
Aether and would work, but keep the two equal so that the channel entry says what the station
runs.

### 2.4 The service code

Trimode reports its channel to the CMS under its **service code**. `PUBLIC` is the public channel
list, the one every Winlink Express shows by default. Give the gateway a **private code** — say
`AETHER` — and that code only. A client that has the same code in its own settings sees the
channel after *Update Table Via Internet*; nobody else does. Private codes are a Winlink feature
for groups that keep their gateways to themselves; the Winlink team does not hand them out, a
group makes its own up.

If you would rather nothing reach the CMS at all, Trimode with RMS Relay in its radio-only mode
and the Internet cable out is how the bench ran: the message then stops in Relay and is never
delivered. For an end-to-end test the service code is the way.

### 2.5 A session at home first

Before driving anywhere, run one session with the client parked within sight of the gateway's
antenna, at 5–10 W. That is the test of the programs — ports, callsigns, the service code, the
path through Trimode to the CMS — with the radio path out of the question. The path test comes
after it.

---

## 3. The gateway

### 3.1 Aether

Work through Setup, then save it as a profile — *Gateway* — so that it is one click to come back
to.

1. **Callsign and rules.** The callsign (`KK4ODA-1`). *United States — FCC Part 97*. Control:
   **Automatic**. Your licence class, USB. Grid, rig, power and antenna: they go into every
   recording.
2. **Radio interface.** **Aether keys the radio.** CAT on the radio's own port is best: it also
   reads the dial, which the rules check needs. A serial line, a CM108 pin, rigctld or FLRig all
   work; with a serial line or a CM108 pin set **Dial is at** on the Session tab. Not *host
   program*: that is for a program set up to key the radio on Aether's `PTT ON`, and the rules
   check needs Aether to read the dial, which it cannot do for a radio it does not hold.
3. **Receive level.** *Good* on a quiet frequency.
4. **Modem settings.** Bandwidth **2300 Hz**. *Wait for a clear channel* on. Fastest mode: the
   last entry. Turnaround 0 for CAT or a serial line (a VOX interface: its delay plus 100 ms).
   Morse ID: a US callsign always sends it, at the end of every session, as `KK4ODA` (the SSID
   is dropped).
5. **Application settings.** **Recording** on. **Debug mode** on: every session Trimode runs is
   recorded and uploaded to the project a minute after it ends, which is the point of the test.
   **Host programs** on, port 8300. KISS programs off.
6. **Save these settings.** Then *Save as…* in the Profile bar: *Gateway*.

Tune to the dial from the Session tab's dial list (3.590 MHz). The rules badge in the header
must say **Legal**. Under automatic control it says *TX blocked* at any dial outside the
sub-bands — that is the rule working, not a fault; move the dial.

For the first run, log every line Trimode and Aether exchange: with Aether closed, open the
configuration folder (*Help › Open the configuration folder*), and in `station.toml` set

```toml
[host]
trace = true
```

The lines appear on the Log tab and in `aetherd.log`. Turn it off again once the setup is proven.

### 3.2 RMS Trimode

Install it from winlink.org (*Sysop software*). Its settings menus name what follows; the words
differ a little between versions.

* **Site Setup.** The **base callsign** (`KK4ODA`) and its Winlink password, the grid square,
  the sysop's details, and the **service code**: the private one alone (§2.4).
* **Modes.** VARA HF only. No Pactor, ARDOP, Robust Packet or VARA FM.
* **VARA HF.** TCP host `127.0.0.1`, command port `8300` — the data port is the next number up,
  as with VARA. Channel bandwidth **2300 Hz**; leave *accept 500 Hz connections* off for this
  test.
* **The "VARA program" path.** Trimode launches the program at this path and watches the
  process. Three ways, in the order to try them:
  1. Aether's application, `C:\Users\<you>\AppData\Local\Aether HF\aether-hf.exe`, with Aether
     already running when Trimode starts. If Trimode accepts the running modem and attaches,
     this is the one to keep.
  2. The same path with Aether closed: Trimode starts Aether, which starts the modem and opens
     the panel. (Aether runs one copy: a second one launched while it runs only brings the
     window up and exits, which Trimode may take for the modem stopping — hence the order.)
  3. The bench's way: a stand-in that stays running (Notepad, `C:\Windows\notepad.exe`, will
     do), and Aether started by hand before Trimode.

  The bench ran the third; the first ran on the air on 2026-10-25. Trimode closes what it
  launched when it exits — under the second way that is the panel, and the modem goes on
  running; the next Trimode start attaches to it again. When a command goes unanswered Trimode
  "restarts VARA": it launches the program again, so Aether's window comes forward, and opens
  the ports again. Aether serves the new connection (the Log tab says *replaces it*).
* **Channel.** One channel, with its own callsign `KK4ODA-1` (§2.2), the dial frequency
  (3.590 MHz), VARA HF, 2300 Hz. No scanning schedule.
* **Radio control: none.** Aether holds the radio's CAT port and keys the radio; Trimode changes
  no frequency. Set the dial from Aether's Session tab.
* **RMS Relay.** Not needed: Trimode forwards to the CMS itself. Leave it off.
* Session and daily-usage limits: the defaults.

What Trimode says to Aether when it attaches (the Log tab, with the trace on): `MYCALL` with the
channel's callsign, `BW2300`, `PUBLIC ON`, `CWID ON`, then `LISTEN ON` and `LISTEN OFF` about
every 3.5 s. The
cycling is Trimode scanning its one channel, and it is normal: Aether answers a call only while
`LISTEN` is on, tells Trimode `PENDING` the moment it takes the call (Trimode then stops
scanning, as it does for VARA), and holds the `CONNECTED` for Trimode's next listening window
if the call came at the end of one, so that it is not missed in the half second Trimode is
deaf (ADR-0051, ADR-0096). The Log tab says *LISTEN turned on and off … the program scans* once
a minute. The header's chip reading *host program · not answering* for a moment now and then is
that rhythm. `PUBLIC ON` is a VARA command about the
gateway being open to the public, nothing to do with the service code; Aether answers `OK`.

### 3.3 Before you leave

* Trimode's window says it is listening on the channel for VARA HF, and its log shows it logged
  in to the CMS.
* Aether's header shows the programs-attached chip, the rules badge **Legal**, the dial at
  3.590 MHz, the Busy lamp off between other stations' signals.
* The radio in its data mode (DATA-U, USB-D) at the drive you set with *Set drive*.
* The computer will not sleep, and nothing else will take the sound card.
* Leave the panel open: it is the only window that shows what the modem is doing, and the one the
  recording and upload lines appear in.

---

## 4. The client

### 4.1 Aether

1. **Callsign and rules.** The station's callsign (`KK4ODA-2` here; the `KK4ODA` Winlink Express
   names is answered to as well). Control: **Local**. Licence class, USB.
2. **Radio interface.** Aether keys the radio, as at the gateway.
3. **Receive level.**
4. **Modem settings.** Bandwidth **2300 Hz**. Turnaround as the interface needs.
5. **Application settings.** **Host programs** on, port 8300. **Debug mode** on. **Recording** on.
6. **Save**, and *Save as…* a *Winlink* profile.

### 4.2 Winlink Express

* *Settings › Winlink Express Setup*: the callsign `KK4ODA` (plain), its password, the grid, and
  under **Service Codes** the private code beside the public one: `PUBLIC AETHER`.
* Open a **Vara HF Winlink** session. In its window, *Settings › Vara TNC Setup*: host
  `127.0.0.1`, command port `8300`. The TNC path is Aether's `aether-hf.exe` with *auto-launch*
  off, Aether started first — the way that is known to work; never `VARA.exe`.
* *Settings › Radio Setup*: radio **Manual**, COM port **None**, and no PTT port. Winlink
  Express then neither tunes nor keys: Aether keys the radio, and you set the dial yourself, to
  the gateway's. A PTT port there would make Winlink Express key the radio as well, on the
  modem's `PTT ON` — one keyer, never both. Only if Aether keys by a serial line or a CM108 pin
  may Winlink Express have the radio's CAT port for frequency control instead.
* **Channel Selection › Update Table Via Internet** (a phone's hotspot does) lists the gateway
  under the private code; pick it. Without Internet, type the gateway's callsign and dial
  frequency into the session window by hand; Winlink Express works out the centre frequency
  (the dial plus 1.5 kHz). Bandwidth **2300**.
* Winlink Express asks to update its propagation estimates at every HF session: *No – Wait till
  later*.

---

## 5. The test

1. At home first (§2.5), parked, at low power, both stations on the dial.
2. In Winlink Express, write a message **to yourself** — your own callsign — and *Post to
   Outbox*. A few lines the first time; an attachment of a few kilobytes in the second message.
3. In the session window, **Start**. In Aether: the banner says *Calling KK4ODA-1*, then
   *Connected*, and the chime plays. In the session window: Trimode's welcome banner, the
   proposal, `FF`, `FQ`. Then both Morse identifiers, and *Disconnected*. The Status tab shows
   the rung the data moved at and the SNR each way.
4. The message is at the CMS now, addressed to you. The next session brings it back down to the
   client — both directions through B2F — or read it at winlink.org.
5. Drive. From each stop: a Winlink session, and a note of the distance, the band and whether
   the engine was running (alternator noise). An Aether **Test session** from the Session tab to
   the gateway's callsign is worth one stop too: it is Aether's own session, Trimode is only told
   the channel is busy, and it measures the path rung by rung.
6. Nothing to collect by hand. Both sessions are host-program sessions — the gateway's through
   Trimode, the client's through Winlink Express — so with debug mode on both stations upload
   theirs (audio, logs, settings) a minute after each session ends: the gateway at once, the
   client as soon as it has Internet (it tries again after 30 and 60 minutes, then keeps the
   files in the recordings folder for *Log › Send files…* later).

---

## 6. When something goes wrong

| What you see | Where to look |
|---|---|
| Winlink Express cannot reach the TNC | Aether running? *Host programs* on, port 8300, saved? Another program on 8300? |
| Trimode says the VARA program stopped, or keeps launching it | Trimode's log: `Restart Vara` after `Vara command wasn't acknowledged` is Trimode giving up on a command; Aether's Log tab shows each new connection (*replaces it*). Before 0.2.0-beta.125 the second connection was refused with `WRONG` until Trimode was restarted. Otherwise the program path (§3.2): try the next of the three ways |
| The call goes out and nobody answers (*no response*) | The gateway's dial and bandwidth (the Stations tab and the mismatch banner, at either end); Trimode listening (its window); the gateway's rules badge — *TX blocked* means the dial is outside the sub-band, or the radio's dial is not read (a serial line without *Dial is at*) |
| *Connected*, then nothing, then the session times out | Trimode's log: `Ignoring CONNECTED while LISTEN=FALSE` or `… in scan state Deaf` is the race 0.2.0-beta.90 and 0.2.0-beta.125 closed (ADR-0051, ADR-0096) — update the gateway; otherwise the Log tab's *host* lines and the host trace (§3.1) say what Trimode was told and when |
| *Connected* and the session ends with nothing sent | An SSID in Winlink Express's callsign; the message not posted to the outbox |
| The gateway is not in the client's channel list | The service code at both ends; *Update Table Via Internet* done; Trimode logged in to the CMS (its log) |
| Slow, or the transfer fails | The path, not the programs: the Status tab's SNR both ways; a Test session on the same path says more; try the other band |
| The gateway's Busy lamp stays on with nothing to hear | Another signal in the channel, or the radio's gain moving (the Diagnostics tab's busy line says). The gateway still answers: its own answers never wait |

---

## 7. Afterwards

* The message is in the client's inbox, and the reply came down: both directions worked.
* Both stations' uploads reach the project on their own; the session goes into `field/LOG.md`
  from them.
* Turn the host trace off at the gateway (§3.1). Keep the *Gateway* profile; the next session
  needs only Trimode started.
* What the bench could not show and this test can: the real turnaround of two radios through
  Trimode's scan, the CMS round trip, and a path that fades. Report what happened — a failed
  session with its recording is exactly what the project learns from.

The bench's notes on Trimode, and every command it and Winlink Express send, are in
[host-interfaces.md §7](../spec/host-interfaces.md#7-verification-status); the headless Linux
gateway is in [gateway-kit.md](gateway-kit.md); each program's settings in full are in
[host-programs.md](host-programs.md).
