# ADR-0133: Aether finds Winlink's programs and reads their settings into profiles

**Status:** accepted for building, 2026-11-19 (the author's brief for the host-program retest).
The keying default (§2.4) and the port rule (§2.5) are this ADR's choices. The author answered §6
the same day, yes to both: §7 is what they changed. The finder now writes what a program must
change to reach Aether, when asked, and a profile keyed by the program keeps the rules check.
On 2026-11-22 the author made a tick set a program up in one step, and an untick go back to
VARA: §8.

## 1. Context

An operator moving to Aether from VARA has Winlink Express, RMS Trimode or RMS Packet set up
already: the radio, its CAT port and speed, how the program keys it, the TNC's address and port,
the bandwidth of each session type, the callsigns. Today the only way to bring that across is
`docs/user/host-programs.md`: read each setting in the program and type it into Aether's Setup.
`VarAC` already has better (`crate::varac`): Aether finds it, writes a `VarAC` profile beside the
operator's and makes the Aether profile that matches. The author asked for the same for Winlink's
three programs, with three differences:

* An operator may have **several installs** of one program (a copy for a second radio, a test
  copy), and **Winlink Express keeps several callsigns** in one install, each with its own
  session type and bandwidth. The finder must find all of them and let the operator pick one or
  several.
* The finder is **read only**. Winlink's programs keep the operator's address, telephone,
  position, e-mail and password in the same files as the radio; nothing is written to them,
  and only the keys a reader needs are read.
* Three programs, three readers, written by three sessions in parallel (the Winlink Express
  retest, the Trimode retest, the RMS Packet retest): one interface they all write to.

## 2. Decision

### 2.1 One module, one reader per program

`core/aetherd/src/hostapps/`:

| File | What it is |
|---|---|
| `mod.rs` | The types: `Program`, `Reader`, `Reading`, `ProgramSetup`, `Radio`, `Ptt`, `Tnc`, `TncKind` |
| `winlink_express.rs`, `trimode.rs`, `packet.rs` | One `READER` each: the executable's name, the installer's folder, the names its uninstall entry and shortcuts carry, the files it reads, and `read` |
| `find.rs` | Finding installs; `Install` |
| `import.rs` | A setup → a profile's settings (`Plan`) |
| `state.rs` | The finder as the daemon runs it, off the run loop, and `hostapps.json` |

A reader is a **pure function** from the files' bytes (`Files`, keyed by the names it lists) to a
`Reading`: the version, one `ProgramSetup` per thing an operator would make a profile of, and
notes in words. A `ProgramSetup` is plain: the callsigns, the program's own name for its session
type, its bandwidth (its own figure, unmapped), a frequency, whether it is a gateway, the radio
(rig, CAT port and speed, Icom address, how it keys) and every TNC it points at (kind, address,
port or serial port, what it starts, its own link parameters). Each reader is tested on synthetic
N0CALL files that also carry the personal keys a real file has, and a test checks that none of
their values reaches the reading. `crate::ini::Ini` (moved out of `varac.rs`) reads INI files as
bytes.

Winlink Express's reader: `RMS Express.ini` beside the executable. `[Properties]` lists the
callsigns (the one in use first); each callsign's section gives `Mode` and the bandwidth and
centre frequency of its VARA HF session types (P2P and Winlink); `[Vara]` is the radio as the
*Radio Setup* dialog keeps it, shared by every callsign; `[Vara TNC]`, `[Vara FM TNC]`,
`[Ardop TNC]` and `[Packet TNC]` the TNCs. The PTT port's values are its help file's: *External*
is VOX, a radio's name or *CI-V* is a command on the control port, a serial port is its RTS/DTR
line.

### 2.2 Finding installs

An install is a folder that holds the program's executable. The finder gathers, once per find
(`Sources::gather`):

* the installer's folder under the system drive and the program folders;
* the registry's uninstall entries (both views of the machine's, and the user's), by
  `DisplayName`: `InstallLocation`, else the folder of `DisplayIcon` or `UninstallString`;
* the shortcuts in the Start menu (the user's and everyone's) and on the desktops, read with a
  small parser of the Shell Link format (MS-SHLLINK), by the target's file name;
* the processes running now, by executable (which also says which install is running);
* the folders the operator added (`hostapps.find {add}`, kept in `hostapps.json` beside the
  configuration: state, not a setting, so no schema change).

Each folder is kept once, with every way it was found. The version is the reader's, else the
executable's version resource. A find takes a fraction of a second, so it runs on a thread of its
own, as the audio devices' listing does (`crate::devices`): `hostapps.find` answers with the last
result and begins another, and a `hostapps` event carries it when it finishes.

The registry and the processes need the operating system: `winreg` and `sysinfo` (its `system`
feature only), on Windows only — the workspace forbids `unsafe`, and both wrap the calls safely.
Elsewhere only added folders are looked in.

### 2.3 A setup becomes a profile

`hostapps.import` makes one profile per setup ticked, named for the program and the callsign
(*Winlink Express KK4ODA*), through the existing profile machinery: the plan's settings merged
over the station's (or over the profile of that name, when one was made before, so its sound
card and levels stay as the operator left them), validated, and captured. With `load` and one
profile, the station switches to it. The plan sets:

* `callsign`: the program's;
* `host.enabled` (a VARA HF TNC) or `kiss.enabled` (a KISS TNC);
* `radio.bandwidth`: 500 for 500, 2300 for VARA's 2300 and 2750;
* the keying (§2.4);
* `host.bind`, when the program points at another port on this computer (§2.5);
* for a gateway, `regulatory.control = "automatic"` and `radio.wait_for_clear`: only a program
  answers an automatically controlled station's calls (ADR-0107).

It also says, in words, what the program must say for the profile to work (`program_must`), and
the panel shows both before anything is saved. A setup above 30 MHz, or whose only TNC is VARA FM
or AGW, waits for Aether FM and makes no profile. A KISS TNC alone is served: Aether's KISS port
carries datagrams between Aether stations today.

### 2.4 Who keys the radio: the program, by default

When the program keys the radio itself (a command on the CAT port, or an RTS/DTR line), there are
two ways:

* **The program keys** (`Keying::Program`): Aether's keying is *Host program*, as with `VarAC`.
* **Aether keys** (`Keying::Aether`): Aether keys as the program did — CAT on its port and speed,
  with the protocol from the rig's maker (Yaesu, Kenwood/Elecraft/Flex, Icom with its CI-V
  address), or the program's RTS/DTR line — and the program's radio goes to *Manual*, its PTT port
  to *None*.

**The default is the program keying**, because it is the seamless one: nothing in the program
changes. Its radio setup keeps working as it did with VARA — it keys on Aether's `PTT ON`, and it
keeps tuning the radio, which a Winlink Express client does to its channel and a Trimode gateway
does when it scans. Only one program can hold the radio's CAT port, so the other way takes the
program's frequency control away (unless it keys on a line of another port). The cost is the
rules check: with the program keying, Aether reads no dial, so the profile sets the rules to
*None*, as Setup does for a host-keyed station, and the operator checks every transmission, as
with VARA. The panel offers the other way, with what the program must change, whenever Aether can
key as the program did. A program that keys by VOX, or controls no radio, leaves Aether's keying
as it is.

### 2.5 A program pointing at another port

The host interface's address (`host.bind`) is a **portable** setting in the registry (a profile
carries it). So a program on this computer that points at another port than Aether's — a second
copy set up beside VARA, say — gets a profile that listens where it points, and it needs no
change; the plan says so ("Aether will listen for it on port 8370"). The KISS port's address
(`kiss.bind`) is this computer's (Machine scope, never in a profile: a profile must never open it
to a network), so a KISS program on another port is told to change it. A program that points at
another computer is told so, and nothing follows it there.

### 2.6 The panel

Setup step 5, under *Host programs*: a *Find programs* row (Find, a folder to add) and a list of
every install, one row per setup: the program, the path, the version, the callsign, whether it is
running, and the port it points at (marked when it is Aether's). Each row has a tick and the
keying; ticking shows what the profile will set and what the program must say, and *Import*
saves every ticked one (*Import and switch* when one is ticked). Every control has a `title`.

## 3. What it removes

Nothing in the daemon: the by-hand setup in `host-programs.md` stays, for programs the finder does
not know and for anyone who wants it. It removes the need to read a program's settings and type
them into Aether, and the guesswork of which install is the one in use. `varac.rs` loses its INI
helper to `crate::ini`, which both now use.

## 4. Consequences

* Two Windows-only dependencies (`winreg`, `sysinfo` with `system` only).
* A new event, `hostapps`, and two methods (`docs/spec/control-api.md`).
* `hostapps.json` beside the configuration.
* A reader that misreads a program writes a wrong profile, never a wrong program: the profile is
  shown before it is saved and can be deleted.

## 5. Alternatives not taken

* **Writing the program's TNC settings** (pointing it at Aether by itself, as `VarAC`'s setup
  does). Not built: the author's brief asks for read only. It is put to the author (§6).
* **Aether keying by default.** It keeps the rules check, but every program would need its radio
  setup changed by hand, and Winlink Express and Trimode would lose their frequency control.
* **A configuration key for the added folders.** It would need a schema bump and a release; they
  are state, like `varac.json`.

## 6. Left for the author

1. Whether the finder should also write the program's TNC address and port (and turn Winlink
   Express's VARA launch off), as `VarAC`'s setup writes `VarAC_Aether.ini`. A copy of the
   program's settings (`RMS Express.ini.bak`) would be kept; the program must be closed first.
2. Whether a profile made with the program keying should keep the rules check with a declared
   dial (the program's channel frequency, which the reader has) instead of *None*.

The author's answers (2026-11-19): yes, and yes. §7.

## 7. The author's decisions: the program set to reach Aether, and the rules kept

### 7.1 Writing what the program must change (`hostapps/point.rs`)

Each reader also says which of its program's settings it needs to reach Aether (`Reader::point`,
pure, a list of `Edit`s), and only a VARA HF TNC's:

* **Winlink Express**: `[Vara TNC] TNC Address` → `127.0.0.1` and `TCP Port` → Aether's host port
  when it names another computer, and `Auto Launch Vara` → `False` (it ends the TNC it started when
  it closes).
* **RMS Trimode**: the same address and port, and `Vara Program File` → Aether's application
  (`crate::varac::app_path`) — the gateway guide's first way (§3.2), which since ADR-0097 starts
  Aether only if no modem runs. Never `Enabled`, a channel's `Mode` or anything else that
  publishes the gateway's channels to Winlink.
* **RMS Packet**: nothing; it waits for Aether FM.

A program on another port of this computer still needs no change: the profile follows it (§2.5).
`point::changes` compares the edits with the file and keeps those that differ, and
`hostapps.find` gives each install's as `program_changes` (each `file`, `section`, `key`, `from`,
`to`, `why`). `point::point` writes them only when the operator asks — `hostapps.point {folder}`,
or `point: true` on an import, which writes first and plans from the file as it is then:

* **never while the program runs**. Winlink's programs read their settings when they start and
  write them back when they close, so a change made under a running one is lost. The processes
  are asked again just before (`find::running_now`; a find may be minutes old), and a running
  program is refused (`refused`, "close it first");
* only the keys named, every other byte kept (`crate::ini::Ini`, line ends as found), the file as
  it was kept beside it as `<name>.aether.bak`, the new one written whole or not at all;
* the install read again afterwards (`find::reread`) and put in the last find's place, so the
  panel and the next plan see the program as it is now; a `log` line named `hostapps` says what
  was written and where the old file is.

The panel shows the changes under a ticked row as **Set it in** *the program*, each as it is and
as it will be, ticked unless the operator unticks it, and disabled with the reason while the
program runs.

### 7.2 The program keying keeps the rules check

With `Keying::Program`, the plan no longer sets `regulatory.profile = "none"`. Aether still reads
no dial, so the dial is declared, as the Session tab's *Dial is at* does for any radio that cannot
report its own:

* **one channel** (a Winlink Express callsign's last centre frequency, a Trimode gateway with one
  VARA channel): the dial is that centre less 1.5 kHz, on the upper sideband (`regulatory.sideband
  = "usb"` in the profile; every VARA program tunes the audio centre 1.5 kHz above the carrier —
  Winlink Express's *USB Dial*). The plan carries it as `dial_hz`;
* **no frequency** in the program: the rules stay on and the dial is left to declare;
* **several channels** (a scanning Trimode, `ProgramSetup::channels_hz`): Aether cannot tell which
  one a transmission goes out on, so the rules stay *None*, as before, and the plan says why.

A dial is this computer's setting (`regulatory.dial_hz`, Machine scope), not a profile's, so it is
declared at the switch: `hostapps.import {load}` sets it after the profile is loaded, and the
panel's *Import and switch* does the same after its own switch. A profile switched to later from
the Profile bar keeps whatever dial was declared last. The program can move to another channel
without Aether knowing; the plan's note says to change *Dial is at* then. Setup shows the rules
fields of a host-keyed station whenever the rules are on (they were greyed to *Host* always);
choosing *Host program* keying in Setup still sets the rules to *None* (ADR-0025).

### 7.3 What this changes in §2 and §5

§2.4's cost (the rules off) holds only for a program that scans several channels. §5's first
alternative is built, behind the operator's tick.

## 8. The author's decisions of 2026-11-22: a tick sets it up

The author found the finder hard to follow: a tick that changed nothing, a preview under the
list, **Import** and **Import and switch**, a *Set it in* box, amber that meant both "Aether will
do this" and "you must do this", and the dial said twice and set only by one of two buttons. The
author asked for one step: ticking a row ports a program that works with VARA to Aether, with a
warning that its settings will change. The answers to the questions that raised (2026-11-22):

1. **Unticking goes back to VARA**, behind a confirmation; the Aether profile is kept.
2. **With Aether keying, the program's radio setup is written too** (its PTT port *None*, and its
   radio *Manual* and control port *None* when Aether keys on that port), as the TNC settings are.
3. **Switching to the profile is a box in the window, ticked by default.**
4. **The window also offers a copy first**, besides *Set up* and *Cancel*.

### 8.1 What a tick does

The row's box shows what is so: ticked when the setup is set up for Aether (remembered in
`hostapps.json`), and it changes only once the setup, or going back, is done. A click opens a
window (a `<dialog>`, the panel's first) instead of toggling:

* **Setting up** lists each setting Aether will write into the program, as it is and as it will
  be; what the Aether profile will hold; in amber, only what the operator must still do by hand;
  and **Switch to it now**, which names the dial it declares when the program keys the radio
  (*and set the dial to 7.082 MHz*), since that switch is the only thing that sets it. A warning
  says the program's settings will change and where the old file is kept, and that a running
  program must be closed. `hostapps.setup` writes the program, makes the profile, remembers both
  and, with `switch`, switches and declares the dial. A refusal (the program still running) is
  shown in the window, which stays open: close the program and press again.
* **Going back** lists each setting put back. `hostapps.restore` writes back each setting Aether
  wrote, with the value it had before Aether first wrote it, but only while it still says what
  Aether set it to: one the operator changed since is the operator's, and is listed as left. A
  setting the file did not have is taken out again (`Ini::remove`). Winlink Express's settings are
  shared by every callsign of an install, so nothing is put back while another setup of the
  install is still set up. The profile stays.
* **Who keys the radio** is chosen before the tick, in the row's *Keying* list, which names the
  program (*Winlink Express*, *Aether*) where it said *Host program*. A set-up row shows its keying
  as text: changing it means going back and setting up again, since moving from Aether keying to
  the program's would need the program's radio setup back first.

The plan for the profile is made from the program as it was before it was written: with Aether
keying, the program no longer says how it keyed the radio once its keying is handed over.

### 8.2 The program's keying handed over (`Target::aether_keys`)

Each reader's `point` takes whether Aether keys the radio, and names, besides the TNC's:

* **Winlink Express** (`[Vara]`, its *Radio Setup*): `PTT Port` → `None`; unless the PTT is a line
  of a port of its own, also `Model` → `Manual` and `Control Port` → `None`. The values are the
  dialog's own words, which its help file and the setup guide use; they are owed a check against a
  Winlink Express set to *Manual* and *None* by hand before the first release that carries this.
* **RMS Trimode**: `[Vara TNC] Use CAT PTT` and `Use Serial Port PTT` → `False`; unless the PTT
  is a line of a port of its own, also `[Radio] Model` → `None` and `Control Port` emptied, which
  is how Trimode 1.4.4 writes its radio *Manual* (read from the author's gateway). Trimode then
  tunes and scans no more, which the window's note says.

The advice that said those by hand (*Radio Manual, PTT port None*) is now marked as carried out
(`settable`), and *Radio setup as it is*, which asked nothing and was amber, is the plain note
*Keeps its radio setup*. The *Rules check at …* note is gone: the dial is said once, beside the
switch that sets it.

### 8.3 A copy first (`hostapps.copy`)

For an operator who wants the copy that works with VARA left alone: the install's folder, mail
included, is copied to a new folder beside it (`C:\RMS Express Aether`, `… Aether 2` when that is
taken), on a thread of its own (a Winlink Express folder holds its mail and logs; the run loop
must not wait on it), refused while the program runs, and the new folder is remembered as an
added one and found. The panel then sets up the copy's row as if it had been ticked, and says to
start the program from the new folder. Only Winlink Express is copied: a second copy of a Trimode
gateway would publish the same channels to Winlink again, and RMS Packet is never set up. Each
copy has its own mail from then on, which the button's tip says.

### 8.4 What this removes

`hostapps.import` and `hostapps.point` (one call does both now), the preview list under the table,
**Import**, **Import and switch** and the *Set it in* box, the install-level `program_changes`
(now each plan's, since what is written depends on the keying), and the duplicate dial note. The
heading's tip that said nothing is written to the programs' files, untrue since §7, is replaced.
