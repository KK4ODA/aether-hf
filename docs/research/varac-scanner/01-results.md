# VarAC's Frequency Scanner on the bench: results

VarAC 15.0.18's Frequency Scanner says nothing to the modem when it changes frequency. A `PENDING`
holds it on its frequency for as long as the `PENDING` stands, and so does a session. `BUSY ON`
holds it once, for the scanner's **Busy hold time** (10 s by default), and then it moves on while
the channel is still busy. The modem's `PTT ON` does not hold it: VarAC retuned the radio twice
while it was keyed. In the harness's terms VarAC is `moves=silent`, `holds=pending`, the case that
answered 30 of 30 calls with **Scans channels** clear (ADR-0138 §7).

Measured on 2026-11-26 with the procedure in `00-bench.md`, on the author's main PC rather than the
second bench PC (see *The bench as run*). Each run's timeline, plan and report are in `runs/r0` to
`runs/r6`. Times in the table are seconds on the run's own clock.

## What VarAC says around a change

Nothing. In R0 the scanner made 20 changes, alternating 7.105 and 14.105 MHz, and VarAC sent no
line to the modem within 3 s of 18 of them. The other two were VarAC's first two changes, which
fall within 2 s of the lines it sends when it attaches (`BW500`, `CHAT ON`, `LISTEN ON`,
`IGNOREKISSDCD ON`, `LISTEN CQ`, `VERSION`, `MYCALL`). R6 confirms it over 84 changes: nothing
around 82, and the same two start-up changes. VarAC never said `LISTEN OFF` or `LISTEN ON` around
a change.

A change on the rig side is a mode command and then the frequency, 50 ms apart
(`M PKTUSB -1`, `F 7105000`). VarAC's log says `Resetting to Slot 0` and
`Scanner - Changing frequency: 7.105.000` at each one.

## What holds VarAC on a frequency

| Run | What the modem said | Window | Changes inside | Next change after the window |
|---|---|---|---|---|
| R0 | nothing; 20 changes | | | stay 6.1 s (median), 5 s set |
| R1 | `BUSY ON`, `BUSY OFF` | 49.0 to 79.0 s (30.0 s) | 3 | 2.4 s |
| R2 | `PENDING`, `CANCELPENDING` | 17.2 to 47.2 s (30.0 s) | 0 | 1.2 s |
| R3 | `PENDING`, ended by hand after 4 minutes | 16.2 to 276.5 s (260.3 s) | 0 | 1.2 s |
| R4 | `CONNECTED KK4XYZ KK4ODA 500`, `DISCONNECTED` | 17.3 to 57.3 s (40.0 s) | 0 | 1.3 s |
| R5 | `PTT ON`, `PTT OFF` | 22.6 to 37.6 s (15.0 s) | 2 | 2.5 s |
| R6 | nothing; 84 changes, beacons ticked | | | stay 11.2 s (median), 10 s set |

**R1, a busy channel.** `BUSY ON` came 2.0 s after the second change. The third change came 16.1 s
after the second, the 6.1 s stay plus 10 s, and VarAC's log gave the reason: "Scanner - Extending
hold time by 10 as the frequency was busy and a busy hold time is activated." From then on it
moved every 6.1 s with `BUSY ON` still standing: three changes fell inside the 30 s window, the
delayed one included. The scanner
screen's help for **Busy hold time** says the same: the extra time is added once per busy
detection, not for as long as the channel stays busy, and 0 turns it off.

**R2 and R3, a call arriving.** A `PENDING` stopped the scanner completely. In R2 the stay that held
it lasted 33.2 s, and the next change came 1.2 s after `CANCELPENDING`: the stay's timer had run
out while it was held. In R3 VarAC sat on 7.105 MHz for 260 s without giving up the `PENDING`. I
then sent `CANCELPENDING` by hand (in `inject.txt`) to see the scan resume, and it did, 1.2 s
later. Within 260 s VarAC set no limit on a `PENDING`; the modem ended it.

**R4, a session.** VarAC made no change in the 40 s session and changed 1.3 s after
`DISCONNECTED`, to 14.105 MHz: the next frequency of the plan, not the one the session was on. It
wrote 5 bytes to the data port as the session opened and did not send `DISCONNECT` itself.

**R5, the modem keying.** VarAC keyed the dummy rig 33 ms after `PTT ON` (`T 1`) and unkeyed it
1 ms after `PTT OFF` (`T 0`). In between it changed frequency twice, 5.3 s and 11.4 s into the
keying, on its usual 6.1 s beat. A scanning VarAC retunes the radio during any transmission the
modem tells it of outside a `PENDING` or a session.

**R6, beacons while scanning.** No beacon reached the modem. VarAC's log and traffic log recorded
four one-time beacons, each about 3 s after a change, as VarAC waits 3 s before firing one
("SENDING ONE-TIME BEACON ON 40m - 7.105.000" at
16:28:50Z and on 20m at 16:29:06Z, then again at 16:43:55Z and 16:44:09Z, the 15-minute beacon
interval). Its own log of VARA commands shows no `CQFRAME`, the tool saw nothing on the command
or the data port, and no `PTT` line came. With **Send Beacon** ticked the stays were 11.1 to 11.3 s
(78 of 82, the start-up change left out), and 14.3 to 15.2 s on the four with a beacon. Whether a change can fall inside a
beacon's keying is not answered (see *Not checked*).

## The procedure's results table

| Finding | Seen | Harness option |
|---|---|---|
| nothing said around a change (R0) | yes, 18 of 20 changes; the other 2 only beside the start-up lines | `moves=silent` |
| `LISTEN OFF` and `LISTEN ON` around each change (R0) | no | |
| no change while `PENDING` (R2) | yes, 0 in 30 s; R3 0 in 260 s | `holds=pending` |
| no change while `BUSY ON` (R1) | no: one 10 s extension, then 3 changes in 30 s | |
| changes in both windows | no | |

VarAC 15.0.18 matches `moves=silent`, `holds=pending`. On the harness that case answered 30 calls
in 30 with **Scans channels** clear and none on the wrong band, so the box stays clear for VarAC,
as the manual and the panel already say. Its `BUSY ON` behaviour is none of the harness's options:
one hold of **Busy hold time** seconds per busy detection, then the scan goes on.

R4 confirms the harness's first assumption (a session holds VarAC). R5 breaks its second: VarAC
retunes while the modem's `PTT ON` stands. By `00-bench.md`, the harness's VarAC stand-in changes
before anything else. Aether's acceptance is told inside the `PENDING` (`PENDING`, `PTT ON`,
`CONNECTED`, `PTT OFF`, ADR-0103) and a session's closing identifier before `DISCONNECTED`
(ADR-0075), so a `PENDING` or a session holds the scanner through those transmissions. A
transmission told with neither standing (a beacon, a KISS broadcast) is not held.

## What VarAC showed

- On `PENDING`, a row in the chat pane and, in its traffic log, "INCOMING CONNECTION REQUEST.
  Still don't know who that is."; on `CANCELPENDING`, "NO LUCK. Maybe next time...".
- On `BUSY ON`, the log line quoted under R1. I took no screenshot during the busy window.
- On the session, no separate window: the main window's **In QSO with** box read KK4XYZ, the
  callsign and band (40m) were filled in, and **Currently sending** showed `<Q>`. The traffic log
  read "CONNECTED TO KK4XYZ (BANDWIDTH: 500 FREQUENCY: 7.105.000)", then a QSO summary of 39 s
  and "DISCONNECTED FROM KK4XYZ". The scratch copy logged the QSO in its own database.
- On `PTT ON`, the **TX** lamp at the bottom left lit red and its VARA log showed `PTT ON`.
- While scanning, the main screen's button read **FREQ SCANNER ON** (green) and the beacon button
  read **BEACONS MANAGED BY SCANNER**.

## The bench as run

**The PC.** The author's main PC (user `Facundo`), with the FTDX10's CAT on COM6, not the second
bench PC the procedure names. Its scratch copy `C:\Dev\AetherBench\varac-a` is VarAC 15.0.18 (title
bar "VarAC by 4Z1AC (V15.0.18)"), left from the October bench with its rig control on CAT, COM6,
38400 Bd, Yaesu FTdx-10. Its first start today, at 15:22:09Z, sent one CAT command to COM6
before I closed it: "CAT: Changing frequency to 14105000". The radio may have moved to 14.105 MHz
then. No aetherd and nothing else was running, and ports 8300, 8301, 4532 and 4533 were free.

**The INI, changed with VarAC closed** (backup `VarAC.ini.bak-20261126`): `VarahfMainPath` from
empty to `C:\Dev\AetherBench\varac-scan\vara-stub\VARA.exe`, `VarahfEnableKissInterface` and
`CheckForUpdates` from ON to OFF, the rig's `ComPort` emptied, `HamlibRigctldHost` from `localhost`
to `127.0.0.1`. `VarahfLaunchOnModemConnect`, `PSKReporterUpload` and `PSKReporterSelfReport` (the
copy's name for self-report) were already OFF. VarAC refused to save its settings while
`VarahfMainPath` named no file ("Can't find the main VARA-HF/FM executable file"), so the stub
folder holds an empty `VARA.exe`; with the launch off it is never run. The keys were read before
every start. After the runs the scanner was set back not to start by itself and its beacons off;
rig control stays on rigctld at 127.0.0.1:4532.

**Hamlib.** 4.7.2, the 64-bit Windows zip from Hamlib's GitHub releases, its SHA-256 checked
against the release's `SHA256SUM-w64-4.7.2`, unpacked to `C:\Dev\AetherBench\hamlib`.
`rigctld -l` lists model 1 as Hamlib's Dummy. The tool ran on Python 3.14.3.

**The rig control dialog.** Menu **Settings**, **RIG Control and VARA Configurations**, window
**Settings**, tab **RIG**. Its sections are **PTT Configuration** (CAT, None, DTR/RTS, FLRig,
OmniRig, Hamlib, with **TEST** **PTT ON** and **PTT OFF**), **CAT Configuration** (COM Port or TCP,
Port, Baud and the rest), **Frequency Control** (CAT, None, FLRig, OmniRig, Hamlib; **Load last
frequency**, ticked; **Read frequency every** 2 **seconds**, not ticked and left so) and
**Hamlib (Rigctld)** with **Host** and **Port**. Both PTT and frequency were set to **Hamlib**
(`HAMLIB-RIGCTLD` in the INI): VarAC's **CAT** choice for PTT keys over its own serial or TCP CAT
port, so **Hamlib** is the way to key by CAT through rigctld, where the tool sees it as `T 1` and
`T 0`. **SAVE AND EXIT** asks to restart, and VarAC restarts itself.

**The scanner.** Menu **Settings**, **Frequency Scheduler / Scanner**, window "Frequency scheduler /
scanner", its right half **Scanner**. A plan row has **Frequency (Hz)**, **Wait time (seconds)**,
**Start UTC time**, **End UTC time**, **Send Beacon**, **Tune Antenna** and **Enabled**, up to 15
rows; above them **Busy hold time (seconds)**, 10. **Wait time** offers 5 to 10 s by one, 15 to 60
by five, then 70, 80, 90 and 100. The shortest stay is 5 s and a new row's default is 5 s. With
**Send Beacon** ticked VarAC set both rows to 10 s by itself, though the list still offered 5. A
stay of 5 s measured 6.1 s, and of 10 s 11.2 s. The beacon interval (tab **CQ / Beacon**,
**Beacon interval (minutes)**) offers 15, 20, 30, 45, 60, 90, 120 and 180, and was 15.

**Starting the scan.** The main screen's button read **FREQ SCHEDULE OFF**, and its tooltip speaks
only of the time-based scheduler. Clicking it with no time-based rules shows "No Frequency schedule
rules configured" and offers the scheduler editor; a right click did nothing. The scan ran once
**Startup mode** on the scanner screen (LAST_STATE, SCHEDULER, SCANNER, INACTIVE) was SCANNER and
VarAC was restarted.

**VarAC between runs.** When a run's tool stopped, VarAC dropped the modem ("Restarting VARA modem
1/3"), tried once 5 s later and stayed disconnected; it did not attach to the next run's tool by
itself. R1 started with a click on **CONNECT MODEM**. For R2 to R6 VarAC was closed, the tool
started, and VarAC started again, as the procedure allows. So in those runs `move1` is VarAC
setting its last frequency at start (14.105 MHz) and `move2` is its first scan step. Two starts of
R2 in which VarAC was not attached when the plan fired were set aside and are not in `runs/`.

**The plans.** R0 gave D = 6 s (6.1 s measured). With the procedure's offsets `D−10` is −4 s, before
`move2`, so every line of each plan is the procedure's offset plus one stay: `move2 +2` for `D−10`,
`+32` for `D+20`, `+42` for `D+30`, `+7` for `D−5`, `+22` for `D+10`. Each line keeps its place
before or after a change, one change later than the procedure's example. The plans are in
`runs/rN/plan.txt`.

**The reports.** Each `report.txt` is the tool's `report` command on the run's folder, the same
report `run` prints when stopped. R3's run was stopped by closing its window, so its timeline ends
without the tool's `stopped` line.

## Not checked

- What a scanner beacon looks like to a modem. With KISS off and the tool having no KISS port, none
  of four logged beacons reached the command or data port, so R6's check (no change inside a
  beacon's keying) is not answered. Whether the scanner sends its beacon over KISS was not
  checked.
- Whether the scanner holds while VarAC itself calls or pings (its `CONNECT`).
- Stays other than 5 and 10 s, a **Busy hold time** other than 10 s, a second `BUSY ON` in one
  stay, and **Tune Antenna** (VarAC logged "Activating the antenna tuner." at every beacon change
  with it unticked, and keyed nothing).
- Turning the scan off with the main screen's button.
- VarAC versions other than 15.0.18, a real radio's time to retune, and anything of VARA.
