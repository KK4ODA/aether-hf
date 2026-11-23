# VarAC, Winlink Express and Pat with Aether

Aether answers on VARA HF's published host interface, so a program set up for VARA HF works
with it: point the program at Aether's port instead of VARA's. On the air, Aether talks only
to other Aether stations — the compatibility is in the software, not the signal.

There are two ways to set up the radio — and a third, [sharing it through `rigctld` or
FLRig](#sharing-the-radio-through-rigctld-or-flrig), for a program that tunes through either. Pick one
per program, and save each as its own **profile** (Setup's Profile bar, *Save as…*) so switching
is one click.

| | The host program owns the radio | Aether owns the radio |
|---|---|---|
| Like | VarAC with VARA: VarAC tunes the radio and keys it | VARA keying the radio itself |
| Aether's radio control (Setup step 2) | **Host program** | CAT, a serial line, a CM108 pin, `rigctld` or FLRig |
| The host program's rig control | PTT and frequency on CAT | none when Aether keys by CAT (only one program can open the CAT port); frequency control only, when Aether keys by a serial line or a CM108 pin |
| Aether's rules check (step 1) | **None** — you check every transmission, as with VARA | on: every transmission judged at the dial |
| Tuning | the host program (VarAC's Auto-QSY, CQ slots) | Aether's dial list (Session tab), or by hand |
| Best for | VarAC; Winlink Express or RMS Trimode already keying the radio for VARA (what *Winlink programs* sets up by default) | Pat, operators who want the safety net |

## Finding your programs: Winlink Express, RMS Trimode, RMS Packet

**On Windows, Aether reads their settings for you** (ADR-0133). Setup step 5, *Winlink
programs*, lists every install of Winlink Express, RMS Trimode and RMS Packet on the computer:
in their usual folders (`C:\RMS Express`, `C:\RMS\…`), in Windows' list of installed programs,
in Start-menu and desktop shortcuts, among the programs running, and in any folder you add
there. Each install shows its folder, version, whether it is running and the TNC port it points
at (*Aether* when it is this Aether's). Winlink Express keeps several callsigns in one install:
each is a row of its own.

**Tick a row and it is set up for Aether in one step.** A window first says what will change:
each of the program's settings Aether will change, as it is and as it will be; what the Aether
profile will hold; and, in amber, anything Aether cannot change for you. **Set up** then changes
the program's settings so it reaches Aether, makes the profile, named for the program and the
callsign (*Winlink Express KK4ODA*), and with **Switch to it now** (ticked) switches the station
to it. **Close the program first**: it writes its settings back when it closes, so a change made
while it runs would be lost. The settings file as it was is kept beside it
(`RMS Express.ini.aether.bak`). Setting a row up again brings the profile up to date and keeps
its sound card and levels.

**To keep a copy that works with VARA**, press **Make a copy first** (Winlink Express only):
Aether copies the whole folder, mail included, to a new folder beside it (`C:\RMS Express
Aether`) and sets up the copy. Start Winlink Express from the new folder to use Aether. Each copy
has its own mail from then on.

**To go back to VARA**, untick the row and press **Go back to VARA**. Aether puts back each
setting it changed, unless you changed it since, and keeps the file as it was as
`RMS Express.ini.aether-undo.bak`. The profile stays. While another callsign of the same Winlink
Express is still set up for Aether, its settings (shared by every callsign) stay as they are.

A profile made this way sets:

* the program's callsign, and its bandwidth (500, or 2300 for VARA's 2300 and 2750);
* **Host programs** on (or **KISS programs**, for a program that uses a KISS TNC);
* the port the program points at, when it is another port on this computer: Aether listens
  there while the profile is in use, so the program needs no change;
* for a gateway (RMS Trimode), automatic control and waiting for a clear channel: only the
  program answers calls then (ADR-0107);
* who keys the radio, chosen in **Keying** before you tick. **The program** (named in the list)
  is the default whenever it keys the radio itself: its radio setup stays as it was with VARA —
  it keys on Aether's `PTT ON` and keeps tuning the radio. Aether cannot read the dial then, so it
  takes the program's channel, less 1.5 kHz, as the dial, set when the window's **Switch to it
  now and set the dial to …** is ticked, and its rules check stays on. Switch to the profile later
  from the Profile bar, or move the program to another channel, and set *Dial is at* on the
  Session tab. A gateway that scans several channels leaves the rules check off, as with VARA:
  Aether cannot tell which channel it is on. The other choice, **Aether**, keys the radio the way
  the program did (CAT on its port, speed and make, or its RTS/DTR line) and keeps the rules
  check on; Aether turns the program's PTT off (Winlink Express's PTT port *External*) and,
  when its radio control is on the same port, sets its radio to *Manual*, because only one
  program can hold a port. A program that keys by VOX, or controls no radio, leaves Aether's
  keying as it is.

What Aether changes in a program: Winlink Express's VARA TNC address when it names another
computer and its *Auto Launch Vara* (off); the program RMS Trimode starts as its VARA (Aether's
application); and, with Aether keying, the program's keying and radio control. Nothing that
publishes a gateway's channels to Winlink is ever changed. A setup above 30 MHz, or one whose
only TNC is VARA FM or AGW, waits for Aether FM and cannot be ticked.

## VarAC, the host program owning the radio

**On Windows, Aether does this for you.** When VarAC is installed (in `C:\VarAC`, or a folder
you name in Setup step 5), Aether offers **Set up**: it writes `VarAC_Aether.ini` in VarAC's
folder, a copy of your `VarAC.ini` with only the modem settings below changed, and saves the
Aether profile *VarAC default* with the Aether settings below. Then choose VarAC's **Switch
profile → VarAC_Aether**. After an update that changes a path or a port, Aether offers to bring
both up to date and keeps everything else. The manual's
[VarAC in one step](../manual/manual.md#varac-in-one-step) has the details. The steps below are
what it does, for doing it by hand.

In **Aether**:

1. Setup step 2, *Radio control*: **Host program** — VarAC or Winlink Express keys the radio on
   `PTT ON` and tunes it, and step 2's *Dial* reads **Host program**.
   The rules in step 1 change to **None** with it, and their fields read **Host**: Aether cannot
   read the dial of a radio it does not control. None of those settings acts while the program
   keys, the **Control** included. The milliseconds beside it (150) are how long
   Aether waits after `PTT ON` before it starts the audio; VARA allows 100.
2. Setup step 5: **Host programs** on (port 8300), and **KISS programs** on (port 8100) for
   VarAC's broadcasts. With Host programs on, Setup's *Callsign* and *Bandwidth* read **Host**:
   VarAC's `MYCALL` and `BW500` set them while it is attached, and Aether's own come back when it
   closes. Set them in VarAC.
3. Save, and save it as a profile — *VarAC*, say.

In **VarAC**, *Settings*:

* *Vara* tab: modem type *VaraHF*, IP/host `127.0.0.1`, main port `8300`, KISS port `8100`.
  *VARA file path*: Aether's application,
  `C:\Users\<you>\AppData\Local\Aether HF\aether-hf.exe`, with *Start modem upon startup* ticked
  — VarAC starts Aether — or leave the path empty and start
  Aether yourself first. Leave *VARA monitor path* empty. Tick *Log VARA commands* while you are
  trying things out: the log is what to send with a report.
* *RIG* tab: *PTT Configuration* **CAT** and *Frequency Control* **CAT**, for your radio. VarAC
  keys the radio when Aether says `PTT ON`, exactly as it does for VARA.

With no host program attached, Aether refuses to start anything (the Session tab says why):
nobody would key the radio.

VarAC's **TUNE** button plays Aether's tuner tone: a steady tone for ten seconds at most, which
VarAC keys the radio for (`PTT ON`) and can stop sooner (`TUNE OFF`). The tone plays at Aether's
transmit level. The level VarAC sends with it, and its drive level (`DRIVELEVEL`), change nothing
in Aether. Set the drive with Aether's *Set drive* (the Session tab's *Keying and drive*), not with
the tone. Like *Tuner tone*, it is refused during a session or while the channel is busy, and the
Log says so.

VarAC's CQs and beacons go through the host port, not the KISS port, so they work with *KISS
programs* off. Another VarAC lists a beacon under BEACONS and a CQ under CQ CALLS. VarAC holds
its beacon while Aether says the channel is busy, and for five minutes after a CQ, while it waits
on its slot (its beacon button then reads *BEACON PAUSED*). VarAC's relay notifications are the
exception: the station holding a VMail tells the addressee about it, on hearing its beacon, with a
frame on the KISS port. Turn on *KISS programs* at both stations, and VarAC's KISS interface, for
those.

**Call CQ and beacons** go out as Aether's beacon. With VarAC keying the radio, Aether applies
none of its rules settings, and they go. With Aether keying the radio (VarAC on VOX) under the US
rules, a station under *Automatic* control sends no beacon (§97.203(d)). If you set the station to
*Automatic* for a gateway (RMS Trimode, RadioPost Gateway), set **Control** back to **Local** (or
**Remote**) in Setup step 1, in the profile you run VarAC under. *VarAC default* never takes a
gateway's *Automatic*: it is made under *Local* control. A refused CQ is answered `WRONG`, not
`OK` (VarAC's *Log VARA commands* file shows which), and Aether's Log has a *host* line saying
why. Hold the mouse over the program's name in Aether's header to read it there too.

## Aether owning the radio

In **Aether**, Setup step 2 keys the radio the way VARA's own PTT settings would — CAT on the
radio's port (which also reads the dial), a serial RTS/DTR line, a CM108 interface's pin,
`rigctld` or FLRig — and step 1 has your rules, control and licence class.

In the **host program**, turn its radio control off: VarAC's *RIG* tab *PTT* and *Frequency
Control* **None**; Winlink Express's *Radio Setup* to radio **Manual**, COM port **None**. Only
one program can hold the radio's CAT port: if Aether keys by a serial line or a CM108 pin
instead, the program can keep its frequency control on CAT. Tune by picking a dial from the
Session tab's dial list, or by hand.

## Sharing the radio through rigctld or FLRig

A program that tunes through Hamlib's `rigctld` or W1HKJ's FLRig can share the radio with
Aether, and the rules check keeps working: Aether keys through `rigctld` or FLRig and reads the
dial there, and the program changes the frequency through the same one. Aether reads the dial again before it judges a
transmission whenever its last reading is more than two seconds old, so a frequency change by the
program is seen before the next transmission.

1. Start `rigctld` for your radio (`rigctld -m <model> -r <CAT port> -s <baud>`); it listens on
   `127.0.0.1:4532`.
2. In **Aether**, Setup step 2, *Radio control*: **rigctld**, address `127.0.0.1:4532`; step 1 has your
   rules, control and licence class.
3. In the **host program**, rig control through Hamlib's network `rigctld` at the same address, with
   its own PTT **off** — Aether keys the radio when it transmits.

`rigctld` serves several programs at once. A program that can only reach the radio over its own
CAT connection cannot share it this way; for it, choose one of the two ways above.

**With FLRig** instead: FLRig holds the radio's CAT port and serves fldigi, WSJT-X and VarAC at
once. In Aether, Setup step 2, *Radio control*: **FLRig**, address `127.0.0.1:12345` (FLRig's XML-RPC
server, *Config › Setup › Server*); in the host program, rig control through FLRig with its own
PTT **off**.

## Winlink Express

* A **Vara HF P2P** session, with another Aether station; or a **Vara HF Winlink** session to an
  RMS gateway that runs Aether. Aether cannot reach a VARA RMS gateway, so Aether gateways are
  listed under the service code **`AETHER`**, never under `PUBLIC`. To find them, set
  *Settings › Winlink Express Setup › Service Codes* to `AETHER` (or `PUBLIC AETHER` to keep the
  VARA gateways listed too), then *Channel Selection › Update Table Via Internet*. A gateway's
  callsign and dial can also be entered by hand.
* **A plain registered callsign**: Winlink Express proposes nothing to an SSID.
* *Vara HF* setup: the TNC path is Aether's application (above), host `127.0.0.1`, port
  `8300`. Leave its automatic launch off and start Aether yourself: Winlink Express ends the
  TNC it started when it closes, and with the launch on that was Aether's window (the truck's
  laptop, 2026-10-25).
* The bandwidth is Winlink Express's session setting: Aether moves to it when it is asked, and
  runs 2300 Hz for Winlink Express's widest, 2750. Calls go out in it, even to a station Aether
  last heard running 500 Hz. With Setup step 4's *Call tries* above 4 (6, 8, 10 or 15), if four
  tries at 2300 Hz get no answer, Aether tries the rest at 500 Hz, and the session runs at 500 Hz
  if it is answered there. At the default, 2 tries, a call stays at 2300 Hz to its end.
* Winlink Express keys the radio itself only when its *Radio Setup* names a PTT port: it then
  keys on the modem's `PTT ON`/`PTT OFF`, as VarAC does, and Aether's *Radio control* must be
  *Host program*. With the radio *Manual* and the COM port *None* it keys nothing and leaves it to the
  modem: keep **Aether keying the radio** (the second way above). One keyer, never both. If
  Aether keys on a serial line or a CM108 pin, Winlink Express can still do its own frequency
  control on the radio's CAT port.
* The gateway end — RMS Trimode fed by an Aether station, and a Winlink Express client calling
  it from the field — is set up step by step in [winlink-gateway.md](winlink-gateway.md).

## Pat

```json
"varahf": { "addr": "localhost:8300", "bandwidth": 2300, "rig": "", "ptt_ctrl": false }
```

Aether runs the `bandwidth` Pat asks for (`"500"` or `"2300"`). Pat can tune through Hamlib's
`rigctld`; with Aether keying the radio, leave Pat's own PTT control off.

Aether gateways are listed under the service code `AETHER`. `"service_codes": ["AETHER"]` (or
`["PUBLIC", "AETHER"]`) in Pat's configuration, then `pat rmslist`, lists them.

## Debug mode: your sessions help the project

While Aether is in its field trials, every session a program runs through it (a host program,
or RadioPost) is recorded
and, a minute after it ends, sent to the Aether project with the logs and the settings (never
a password) — **debug mode**, Setup step 5, on unless you turn it off. The recording is the
audio of the session, so what was sent can be decoded from it. The first time the panel opens,
a window says this and asks whether to keep it on, and nothing is sent until it is answered. A
gateway without a panel says it in its log at every start and sends nothing until it is answered
there too ([the gateway kit](gateway-kit.md)). See ADR-0050.

## How Aether follows the program

* **One place for each setting.** With Setup step 5's *Host programs* on, the callsign (step 1)
  and the bandwidth (step 4) read **Host**: the program sets them. With *Radio control* **Host
  program** (step 2), the rules' fields read **Host** too. What is underneath is kept for when no program
  is attached.
* **The callsign.** The program's `MYCALL` names the callsigns it answers to and identifies as
  while it is attached, and Aether answers no other callsign of its own meanwhile: the program
  speaks for the station. When the program closes, Aether goes back to the callsign in Setup.
  Another program on the control API may hold callsigns of its own beside it (a RadioPost
  gateway, for example: ADR-0106). A callsign that program holds is not the host program's: its
  calls, and its sessions, are the other program's, and the host program sees only a busy
  channel while they run.
* **The bandwidth.** A program's `BW500`, `BW2300` or `BW2750` (run as 2300) moves the station
  while the program is attached — between sessions, never during one — and it comes back to
  Setup's when the program closes. A 2300 Hz station also answers a 500 Hz call at 500 Hz, as VARA's *Accept
  500 Hz connections* does, and comes back 20 s after the session; a 500 Hz station never answers
  a 2300 Hz call. The panel's header shows the bandwidth, and why, whenever it is not Aether's own.
* **Two stations in different bandwidths** (ADR-0035). A probe is answered whatever the two
  bandwidths — probes and their answers are the same tone-floor frames in both — and its result
  says when the other station runs another bandwidth ("WC4Y hears us at 3 dB … — runs 2300 Hz,
  this station 500 Hz"). A 2300 Hz station calling one it has heard run 500 Hz — from a beacon, a
  call, a probe or a probe's answer — calls at 500 Hz and comes back 20 s after. A 2300 Hz call to a
  station whose bandwidth is not known goes on at 500 Hz after four unanswered tries, when Setup
  step 4's *Call tries* is above 4 (ADR-0058, ADR-0126 §9); a call of 2 tries, the default, does not.
  The crossing that cannot be made is a call that stays at 2300 Hz to a 500 Hz station: it is
  not answered, and the panel says so
  in a warning across every tab, naming the station, both bandwidths and the fix — Setup step 4's
  bandwidth (a 2300 Hz station still answers 500 Hz calls), the program's `BW` command when a
  program chose 500 Hz, or asking the other station to call at 500 Hz. The Stations tab shows each
  station's bandwidth, in amber when it is not this station's.
* **Answering calls.** With a program attached, Aether answers calls to its callsigns only once
  the program has said `LISTEN ON` (VarAC and Winlink Express do when they start), as VARA does.
  The header says *host program · listening* once it has, and *host program* alone until then;
  the log names every call left unanswered.
* **The Morse ID at the end** (ADR-0075, as VARA does). Each station's Morse ID is the last
  thing it sends for a session. A US callsign always sends it; `CWID OFF` from the program is
  answered `OK` and changes nothing. Any other callsign sends it if Setup step 4's *On* box
  beside *Identify by* is ticked, or while the program has said `CWID ON` (RMS Trimode does at
  start). It sends the callsign without its SSID: a program's `MYCALL KK4ODA-1` or `KK4ODA-1-T`
  is identified as `KK4ODA`. With *Identify by* set to *Data signal* the ID goes in Aether's own
  signal instead of Morse (ADR-0127), at the same place; `CWID ON` means "identify" and never
  turns Morse back on.
  * The program hears `DISCONNECTED` only once its station's ID has gone out. So VarAC going
    back to its calling frequency, a scanning Trimode changing channel, or Winlink Express placing
    its next call does not go over it. With the program keying the radio, it is still attached and
    keys for the ID (`PTT ON`, `PTT OFF`).
  * If the program has gone and nothing can key the ID, the log says so.
  * Nothing identifies a session between its call and its end. VarAC's *Call ID interval* sends
    `DE <your callsign>` as chat text, which Aether carries as data; Winlink sends the callsigns
    only at the start. See *FCC regulatory controls*, *What identifies a long session*.
* **A transfer that waits.** While a session's bytes wait for the other station to acknowledge
  them, Aether tells the program how many again (`BUFFER`) every 50 s, even when the count has
  not moved. Pat gives up on a transfer after a minute with no word of the count ("write: buffer
  timeout"), and on a weak path a burst can wait longer than that for its answer (ADR-0109).
* **The keying check.** The first transmission after a program attaches is judged on its own:
  if the radio's receive audio does not change while it is keyed, the Log says the radio may not
  be transmitting. A program keying a port nothing is wired to is caught at its first try.
* **Aether's own sessions.** A call or a Test session started from Aether's Session tab while a
  program is attached is not the program's: the program keys the radio for it, when it owns the
  radio, and sees the channel busy — and is told nothing else. Its idle timer cannot end the
  session, its Disconnect leaves it alone, and what you type in the program waits for a session
  of the program's own.

## Known limits

* A VarAC **ping** to a station whose callsign has an SSID fails: VarAC calls `<callsign>-T`, and
  `KK4ODA-1-T` is ten characters with two suffixes — longer than Aether's frames carry, and not a
  callsign VARA's own rules allow either (three to seven characters, then an optional `-1` to
  `-15`, `-T` or `-R`). Plain callsigns work.
* A call through a digipeater — `CONNECT <me> <them> via <digi>`, VARA FM's form in its published
  command list, which Pat and Winlink Express send when the path names one — is refused (`WRONG`):
  Aether relays nothing, and calling the station directly instead is not what was asked.
* VarAC's *Remember VARA audio level per band* is ignored: set the level with Aether's *Set
  drive* (Session tab, Keying and drive).
* The rules check needs Aether to know the dial; with the host program owning the radio it is
  off. Keep the second way, or share the radio through `rigctld` or FLRig, if you want Aether's safety net.
