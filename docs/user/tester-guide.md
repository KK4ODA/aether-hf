# Testing Aether HF on the air: a guide for trusted testers

Thank you for helping. Aether HF is an open-source HF data modem that does what VARA HF does for
Winlink Express, Pat and VarAC: the programs talk to it the way they talk to VARA, and it puts
its own signal on the air. It has passed every test we can run without a radio. What it needs
now is real paths, real radios and real operators — yours.

This page is everything you need: install, set up, connect, and what happens to your files. The
detailed guides are linked where you need more.

**What to expect.** This is a beta. Sessions will sometimes fail, and that is useful: a failed
session with its recording is exactly what we learn from. You do not need to write anything up —
Aether sends what we need (see *Debug mode* below).

---

## 1. Install

Download the latest `-beta` release from the
[releases page](https://github.com/KK4ODA/aether-hf/releases) — on Windows,
`aether-hf_<version>_x64-setup.exe` — and run it. Windows will warn that the installer is not
signed: *More info*, then *Run anyway*. The details, Linux and macOS are in
[install.md](install.md).

Aether updates itself: *Help > Check for Updates*. **Keep it current** — when a release changes
the link protocol, stations on different versions cannot connect, and the update is marked
*Update required*.

## 2. First start: Setup

The panel opens on the **Setup** tab, a numbered list of steps. Each step gets a green mark when
it is done. Nothing changes until you press **Save** at the bottom.

1. **Callsign and rules.** Your callsign, the rules you operate under (*United States — FCC Part
   97*), how the station is controlled (**local**: you are at the radio), your licence class and
   your sideband (USB). Aether checks every transmission against these before it keys the radio,
   and transmits nothing until they are set.
2. **Radio interface.** Pick your interface from the list (an Icom with USB audio, a Yaesu, a
   SignaLink, a DRA or URI board) and the sound card's input and output. *Keying* is how Aether
   keys the radio — see §3 for which to choose.
3. **Receive level.** With the radio on a quiet frequency, set the radio's or sound card's
   receive level so the meter reads *Good*.
4. **Modem settings.** *Bandwidth*: **500 Hz** for VarAC and P2P on the calling frequencies,
   **2300 Hz** for Winlink to a gateway — a program that asks for the other one moves Aether
   there for its session, so either is fine. If your interface is a **SignaLink** (or anything
   keyed by VOX), set *Answer gap* to **500–800 ms**: its DLY knob holds the key after the audio
   stops, and Aether's answers would otherwise start while your radio is still transmitting.
   Aether also learns each station's gap by itself: when a station calls or probes again after
   being answered, its gap grows (the Log tab says so), and it is remembered. The setting is the
   least it uses.
5. **Application settings.** *Host programs* on (port 8300). *KISS programs* on (port 8100)
   only for VarAC's broadcasts. **Debug mode** — leave it on (§6).

Then **Save**. To set your transmit level, use **Session > Keying and drive > Set drive**: it
sends four six-second bursts of the real signal so you can watch the ALC. Set the level so the ALC just moves — not into the
red, and not at full power: an Aether signal's peaks are what the ALC sees.

**Profiles.** Setup's Profile bar saves everything under a name. *Winlink* and *VarAC* profiles
make switching between programs one click.

## 3. Your program

Aether answers on VARA HF's host port, so you set your program up as for VARA and point it at
Aether. The full guide is [host-programs.md](host-programs.md); here is the short version.

### Winlink Express

* **Use your plain registered callsign** — no SSID. Winlink Express proposes no messages to a
  station whose callsign has an SSID: the session connects and ends with nothing sent.
* *Vara HF* setup: host `127.0.0.1`, port `8300`. The TNC path can be Aether's application,
  `C:\Users\<you>\AppData\Local\Aether HF\aether-hf.exe` — or leave auto-launch off and start
  Aether first. **Never point it at `VARA.exe`.**
* **Aether keys the radio** (Setup step 2: CAT, a serial line, a CM108 pin or `rigctld`), not
  Winlink Express. If Aether keys on a serial line or a CM108 pin, Winlink Express can still do
  its own frequency control on the radio's CAT port.
* The bandwidth is set in Winlink Express's session window; Aether follows it.
* Winlink Express asks to update its propagation estimates at every HF session: *No – Wait till
  later* is fine.

**P2P with another tester:** open a *Vara HF P2P* session, enter the other station's callsign
and the agreed frequency, and connect.

**Through the Aether gateway:** open a *Vara HF Winlink* session, and enter the gateway's
callsign and dial frequency **by hand** — KK4ODA sends them to testers directly. The Aether
gateway is not in Winlink's public channel list on purpose: a VARA station that picked it could
not connect, and would blame VARA. Send yourself a short message first, then check for it.

### VarAC

VarAC usually keys the radio itself (Setup step 2: **host program**), as it does with VARA:
VarAC's *Vara* tab with host `127.0.0.1`, main port `8300`, KISS port `8100`; its *RIG* tab with
PTT and frequency control on CAT. Ping, chat, files, CQ and broadcasts all work. Use plain
callsigns (no SSID), and set VarAC's callsign before the first run. One VarAC feature does not
reach Aether: its periodic *Send beacons* — VarAC sends Aether nothing for it, so use *Call CQ*.

### Pat

`"varahf": { "host": "localhost", "cmdPort": 8300, "dataPort": 8301, "bandwidth": "2300" }` in
Pat's configuration, with Aether keying the radio.

## 4. On the air

* **Aether talks only to Aether.** It cannot decode VARA or any other mode, and VARA cannot
  decode it. Both stations need Aether, and the same link protocol (keep updated).
* **Where:** inside the data segments, away from FT8 and the other weak-signal frequencies;
  [frequency-plan.md](frequency-plan.md) proposes frequencies per band, and the Session tab's
  dial list has them. Aether listens before it transmits and waits for a clear channel.
* **Identify** as you would with any data mode: Setup step 4's Morse ID, or by voice or CW.
* **What you will see:** the banner across the top of every tab says what the station is doing
  (calling, connected, how a session ended); the **Status** tab shows the signal-to-noise ratio
  each way, the speed and the rung (how fast the link is running — it starts low and climbs);
  the **Stations** tab lists every station heard and every session.
* **A probe** (Session tab, *Probe*) is a one-frame check: it tells you how the other station
  hears you, and how you hear it, without starting a session. Try one before calling.
* **A Test session** (Session tab, *Test session*) runs a fixed sequence against a listening
  Aether station — a probe, a call, a message, a climb up every rung, a file — and records it.
  It is the single most useful thing you can do on a new path.

## 5. When something goes wrong

* **The log:** the **Log** tab, filterable by kind. *Help > Open the configuration folder* has
  the daemon's log of this run (`aetherd.log`), the one before (`aetherd.prev.log`, which is the
  one to read if a restart made the problem go away), and every run of the last 30 days in
  `logs/`.
* **Send your files** (only needed if debug mode is off, or for sessions it does not send): Log
  tab → **Send files…** → *Send my files to the Aether project* → **Send**. One press: your logs
  and recordings go to the project, no email needed.
* **A session the other station could not hear you in** says the most with both sides' files.
  *Ask a station to send me its files* writes them an email with a link that opens their Aether
  ready to send.
* **A failed Test session:** *Send files…* → *Report my last test as a GitHub issue* opens a
  pre-filled report (a free GitHub account is needed).
* Or just email KK4ODA with the time (UTC), the frequency and what you saw.

## 6. Debug mode: what is sent, and how to stop it

During the field trials, **debug mode** is on by default (Setup step 5). After every session a
host program runs — Winlink Express, Pat or VarAC, P2P or through the gateway — Aether waits a
minute, and while the station is idle uploads one zip to the Aether project's folder:

* the session's **recording**: the audio Aether received during the session, and a summary of
  every frame it heard and sent, with its signal-to-noise ratio;
* the **logs** of that period, the session history and the stations heard;
* your **settings, without passwords**.

**The recording is the audio of the session, so what was sent can be decoded from it** —
including the text of Winlink messages. Amateur radio traffic is not private, but you should
know. Aether says so once, the first time you open it with debug mode on, with *Keep it on* and
*Turn it off*. To turn it off later: Setup step 5, untick **Debug mode**, Save. Sessions you
start from Aether's own Session tab and Test sessions are never sent automatically.

Every upload is written to the Log tab as it happens. If an upload fails (no internet), it is
tried again later, and after three tries the files stay in your recordings folder.

## 7. Known limits

* Pat at 500 Hz, and every program on the air, are what you are testing: they have passed on
  the bench, with two stations connected through a simulated channel, not yet on a real one.
* A **SSID** in Winlink Express's callsign stops messages (§3).
* VarAC's *Send beacons* does nothing with Aether (§3).
* The installer is not yet code-signed (§1); macOS builds are untested with a radio.
* Aether's own sessions with a host program attached are not the program's: the program keys
  the radio for them and is told nothing else ([host-programs.md](host-programs.md)).

73, and thank you — KK4ODA
