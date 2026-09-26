# VarAC, Winlink Express and Pat with Aether

Aether answers on VARA HF's published host interface, so a program set up for VARA HF works
with it: point the program at Aether's port instead of VARA's. On the air, Aether talks only
to other Aether stations — the compatibility is in the software, not the signal.

There are two ways to set up the radio. Pick one per program, and save each as its own
**profile** (Setup's Profile bar, *Save as…*) so switching is one click.

| | The host program owns the radio | Aether owns the radio |
|---|---|---|
| Like | VarAC with VARA: VarAC tunes the radio and keys it | VARA keying the radio itself |
| Aether's keying (Setup step 2) | **host program** | CAT, a serial line, a CM108 pin or `rigctld` |
| The host program's rig control | PTT and frequency on CAT | none (only one program can open the CAT port) |
| Aether's rules check (step 1) | **None** — you check every transmission, as with VARA | on: every transmission judged at the dial |
| Tuning | the host program (VarAC's Auto-QSY, CQ slots) | Aether's dial list (Session tab), or by hand |
| Best for | VarAC | Winlink Express, Pat, operators who want the safety net |

## VarAC, the host program owning the radio

In **Aether**:

1. Setup step 2, *Keying*: **host program — VarAC or Winlink Express keys the radio on PTT
   ON**. The rules in step 1 change to **None** with it: Aether cannot read the dial of a radio
   it does not control. *ms before the audio* (150) is how long Aether waits after `PTT ON`
   before it starts the audio; VARA allows 100.
2. Setup step 4, *Bandwidth*: whichever you use most. VarAC says `BW500` when it starts, and
   Aether moves to 500 Hz for as long as VarAC is attached, without a restart; it goes back to
   this one when VarAC closes.
3. Setup step 5: **Host programs** on (port 8300), and **KISS programs** on (port 8100) for
   VarAC's broadcasts.
4. Save, and save it as a profile — *VarAC*, say.

In **VarAC**, *Settings*:

* *Vara* tab: modem type *VaraHF*, IP/host `127.0.0.1`, main port `8300`, KISS port `8100`.
  *VARA file path*: Aether's application,
  `C:\Users\<you>\AppData\Local\Aether HF\aether-hf.exe`, with *Start modem upon startup* ticked
  — VarAC starts Aether, and closes it when VarAC closes — or leave the path empty and start
  Aether yourself first. Leave *VARA monitor path* empty. Tick *Log VARA commands* while you are
  trying things out: the log is what to send with a report.
* *RIG* tab: *PTT Configuration* **CAT** and *Frequency Control* **CAT**, for your radio. VarAC
  keys the radio when Aether says `PTT ON`, exactly as it does for VARA.

With no host program attached, Aether refuses to start anything (the Session tab says why):
nobody would key the radio.

## Aether owning the radio

In **Aether**, Setup step 2 keys the radio the way VARA's own PTT settings would — CAT on the
radio's port (which also reads the dial), a serial RTS/DTR line, a CM108 interface's pin, or
`rigctld` — and step 1 has your rules, control and licence class.

In the **host program**, turn its radio control off: VarAC's *RIG* tab *PTT* and *Frequency
Control* **None**; Winlink Express's radio setup to none. Only one program can hold the radio's
CAT port. Tune from the Session tab's dial list (*Tune*) or by hand.

## Winlink Express

* A **Vara HF P2P** session, with another Aether station. Aether cannot reach a VARA RMS
  gateway — and an Aether gateway must never be listed as a VARA one.
* *Vara HF* setup: the TNC path is Aether's application (above), with or without auto-launch;
  host `127.0.0.1`, port `8300`.
* The bandwidth is Winlink Express's session setting: Aether moves to it when it is asked, and
  runs 2300 Hz for Winlink Express's widest, 2750.
* Winlink Express leaves keying to the modem, as with VARA: keep **Aether keying the radio**
  (the second way above). If Aether keys on a serial line or a CM108 pin, Winlink Express can
  still do its own frequency control on the radio's CAT port.

## Pat

```json
"varahf": { "host": "localhost", "cmdPort": 8300, "dataPort": 8301, "bandwidth": "2300" }
```

Aether runs the `bandwidth` Pat asks for (`"500"` or `"2300"`). Pat can tune through Hamlib's
`rigctld`; with Aether keying the radio, leave Pat's own PTT control off.

## How Aether follows the program

* **The bandwidth.** Setup step 4 is Aether's own. A program's `BW500` or `BW2300` moves it while
  the program is attached — between sessions, never during one — and it comes back when the
  program closes. A 2300 Hz station also answers a 500 Hz call at 500 Hz, as VARA's *Accept
  500 Hz connections* does, and comes back 20 s after the session; a 500 Hz station never answers
  a 2300 Hz call. The panel's header shows the bandwidth, and why, whenever it is not Aether's own.
* **Answering calls.** With a program attached, Aether answers calls only once the program has
  said `LISTEN ON` (VarAC and Winlink Express do when they start), as VARA does. The header says
  *host program · not answering* until then, and the log names every call left unanswered.

## Known limits

* A VarAC **ping** to a station whose callsign has an SSID fails: VarAC calls `<callsign>-T`, and
  `KK4ODA-1-T` is ten characters with two suffixes — longer than Aether's frames carry, and not a
  callsign VARA's own rules allow either (three to seven characters, then an optional `-1` to
  `-15`, `-T` or `-R`). Plain callsigns work.
* VarAC's *Remember VARA audio level per band* is ignored: set the level with Aether's *Set
  drive* (Session tab, Keying and drive).
* The rules check needs Aether to know the dial; with the host program owning the radio it is
  off. Keep the second way if you want Aether's safety net.
