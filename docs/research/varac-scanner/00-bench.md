# VarAC's Frequency Scanner on the bench

This procedure finds out what VarAC 15.0.18's Frequency Scanner tells the modem when it changes
frequency, and what keeps it on a frequency. Those two facts decide what Aether can do for a
station whose VarAC scans (ADR-0138 §7). VarAC's announcement and release notes say a scanning
plan holds a stay per frequency, its hours, a beacon and a tune, and nothing more. The harness
tried silent and bracketed changes with three kinds of hold, and gave these results, 30 calls
each, with **Scans channels** clear:

| VarAC stays on a frequency for | Answered | Answered on the wrong band |
|---|---|---|
| a `PENDING`, or a `BUSY ON` | 29 or 30 of 30 | 0 |
| a session only | 22 to 24 of 30 | 3 |

So the bench answers two questions. Does VarAC say anything to the modem around a change
(`LISTEN OFF`, `LISTEN ON`, anything else)? And does a `PENDING`, a `BUSY ON`, a session or the
modem's `PTT ON` keep it where it is?

The bench uses no radio, no VARA and no Aether daemon. It runs a scratch VarAC, Hamlib's dummy
rig and `tools/varac_scan_bench.py`. The tool answers VarAC on the VARA host ports as Aether's
adapter answers an idle program, and passes VarAC's rig commands through to the dummy rig. It logs
both in one file, `timeline.log`, on one clock, with each frequency change marked as a `move`. A
plan sends `PENDING`, `BUSY ON` and the rest at set times after a change, and the tool's report
counts the changes made while each was in force.

## The PC and what it needs

The second bench PC (user folder `C:\Users\User`) has what is needed except Hamlib:

- the scratch VarAC copy `C:\Dev\AetherBench\varac-a`, with its INI already safe for a bench (its
  modem launch off, `VarahfMainPath` in a stub folder, PSK Reporter upload and self-report off,
  update checks off);
- Python 3.12 or later, and a checkout of this repository at `master` (any commit from
  2026-11-26 on has the tool);
- Hamlib for Windows: the 64-bit zip from Hamlib's releases on GitHub, unpacked so that
  `rigctld.exe` is at `C:\Dev\AetherBench\hamlib\bin\rigctld.exe`. Only it and its DLLs are
  used.

Ports 8300, 8301, 4532 and 4533 must be free. Stop any `aetherd` and the old stand-in
(`stub-modem\vara_standin.py`) before starting.

Before each start of VarAC, open `varac-a`'s INI and check that `VarahfLaunchOnModemConnect=OFF`
and that `VarahfMainPath` names the stub folder, not `C:\VARA`. `C:\VARA` is a real VARA install,
and VarAC must never start it. Set `VarahfEnableKissInterface=OFF` as well: the tool has no KISS
port. Print only those keys, never the whole file.

## Setting it up

1. Start the dummy rig in a PowerShell window of its own. Model 1 is Hamlib's dummy rig; it keeps
   a frequency and a PTT state and drives nothing.

   ```bash
   C:\Dev\AetherBench\hamlib\bin\rigctld.exe -m 1 -t 4533
   ```

2. Start the tool in a second window, from the checkout. Each run gets its own folder.

   ```bash
   python tools\varac_scan_bench.py run --out C:\Dev\AetherBench\varac-scan\r0
   ```

3. Start `varac-a\VarAC.exe`. Its modem settings should already point at 127.0.0.1 port 8300,
   as on the bench of 2026-11-17. The tool's window shows `VarAC attached to the command port`,
   then VarAC's opening lines (`varac>`), each answered (`modem>`).

4. In VarAC's rig control settings, choose Hamlib's rigctld at 127.0.0.1, port 4532. That is
   the tool's pass-through; rigctld itself listens on 4533. Set PTT to go by CAT, so VarAC's
   keying shows as `ptt` lines in the timeline. Note whether *Read frequency every 2 seconds* is
   ticked, and leave it as it is. VarAC 15.0.18 offers rigctld (its 15.0.18 notes change how it
   sets the filter through rigctld). The names of the dialog's fields were not checked from here.

5. Make the scanning plan on VarAC's Frequency Scheduler screen, in its Frequency Scanner
   section: 7.105 and 14.105 MHz, all hours, no beacon, no tune, and the shortest stay it
   allows. Write down that shortest stay and the stay it offers by default; both go in the
   results. The dummy rig takes any frequency.

6. Start the scan with the main screen's Frequency Scheduler button, which switches between
   Scheduler and Scanner mode. The tool's window shows a `move` line at each change.

Stop a run with Ctrl+C in the tool's window. It prints its report, and keeps `timeline.log` and
`inject.txt` in the run's folder. Start the next run in a new folder (step 2 with another
`--out`). VarAC attaches again by itself; restart it if it does not.

## The runs

Each run after the first uses a plan: a text file of lines `moveN +SECONDS LINE`. `move2 +50 BUSY
ON` means "50 s after the second frequency change, tell VarAC `BUSY ON`". Call the stay R0
measures **D**. The examples below are for D = 60 s. For another stay, keep each line's place
relative to D: `D−10` is 10 s before the third change is due. `{mycall}` in a line becomes the
callsign VarAC said first in `MYCALL`. Start a run with its plan like this:

```bash
python tools\varac_scan_bench.py run --out C:\Dev\AetherBench\varac-scan\r1 --plan C:\Dev\AetherBench\varac-scan\plan-r1.txt
```

A line can also be sent by hand: type it in the tool's window, or append it to the run's
`inject.txt` from another window.

```bash
Add-Content C:\Dev\AetherBench\varac-scan\r1\inject.txt "BUSY ON"
```

**R0, the stay and what VarAC says.** No plan. Let it run for at least six changes. The report
gives the stay (median, shortest and longest) and every line VarAC sent within 3 s of a change.
Nothing there means VarAC moves silently; `LISTEN OFF` before and `LISTEN ON` after means it
steps as Trimode does. Anything else is new: copy it into the results.

**R1, a busy channel.** `plan-r1.txt`:

```
move2 +50 BUSY ON
move2 +80 BUSY OFF
```

If the report shows no change inside the `BUSY ON` window, a busy channel holds VarAC. It also
gives how long after `BUSY OFF` the next change came.

**R2, a call arriving.** `plan-r2.txt`:

```
move2 +50 PENDING
move2 +80 CANCELPENDING
```

No change inside the `PENDING` window means a `PENDING` holds VarAC. This is the case Aether can
use: it can say `PENDING` as soon as a call starts arriving.

**R3, a `PENDING` left open.** `plan-r3.txt`, and let it run 4 minutes past the `PENDING`:

```
move2 +50 PENDING
```

This shows whether VarAC gives up on a `PENDING` by itself, and when. Aether always ends a
`PENDING` with `CONNECTED` or `CANCELPENDING`, but a lost call must not hold the scanner for
long.

**R4, a session.** `plan-r4.txt`:

```
move2 +50 CONNECTED KK4XYZ {mycall} 500
move2 +90 DISCONNECTED
```

VarAC opens a chat window for KK4XYZ; leave it. The report shows whether it changed frequency
during the session and how long after `DISCONNECTED` it resumed. Note which frequency it went to
next. If VarAC sends `DISCONNECT` itself, the tool answers `DISCONNECTED` at once.

**R5, the modem keying.** `plan-r5.txt`:

```
move2 +55 PTT ON
move2 +70 PTT OFF
```

That is how Aether keys a VarAC that keys the radio: VarAC hears `PTT ON` and keys over CAT, which
shows as a `ptt 1` line. A change between `PTT ON` and `PTT OFF` means VarAC would retune a
transmitter on the air.

**R6, beacons while scanning.** Tick the beacon on both frequencies of the plan, and start the
tool with `--key-beacons`, which answers each `CQFRAME` with `PTT ON` and, 4 s later, `PTT OFF`.
No plan file. The timeline shows the order at each frequency: the change, then the beacon, then
its keying. Check that no change falls inside a beacon's keying.

## The results

For each run keep its folder (`timeline.log`, `inject.txt`) and the printed report. Add the
shortest and default stays from step 5, and VarAC's version from its title bar. The timeline
holds the lines VarAC said to the modem and the size of anything written to the data port,
never its contents, so it can go into this folder as `runs/r0` … `runs/r6`.

Read them against the harness's options:

| Finding | Harness option | What it means for Aether |
|---|---|---|
| nothing said around a change (R0) | `moves=silent` | Aether cannot see a change; **Scans channels** stays clear |
| `LISTEN OFF` and `LISTEN ON` around each change (R0) | `moves=listen` | Aether learns VarAC's steps as it does Trimode's |
| no change while `PENDING` (R2) | `holds=pending` | with the box clear, 30 of 30 on the harness |
| no change while `BUSY ON` (R1) | `holds=busy` | with the box clear, 29 of 30 on the harness |
| changes in both windows | `holds=none` | 3 calls in 30 answered on the wrong band: a design change, the author's call |

R3 bounds how long a `PENDING` may stand, and R4 and R5 check two things the harness assumed:
that a session holds VarAC, and that it does not retune while keyed. If either fails, the
harness's VarAC stand-in changes before anything else.

This bench does not check VarAC versions other than 15.0.18 (16.1.8 and 17.1.9 changed beacons
and the scanner's manual start), a real radio's time to retune, or anything of VARA, which stays a
black box. The stand-in answers only what an idle VarAC asks: a call VarAC makes is ended with
`DISCONNECTED` after 30 s, and no data is ever sent to it.
