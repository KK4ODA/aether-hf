# ADR-0133: Aether finds Winlink's programs and reads their settings into profiles

**Status:** accepted for building, 2026-11-19 (the author's brief for the host-program retest).
The keying default (§2.4) and the port rule (§2.5) are this ADR's choices; §6 lists what is left
for the author.

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
* `host.bind`, when the program points at another port on this computer (§2.5).

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
