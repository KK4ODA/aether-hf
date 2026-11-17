# A station that scans: the harness's results

*Against ADR-0129 §6's criteria, registered before any run. Run 2026-11-17 in a cloud container
(four cores, two runs at a time) on the daemon at `09662d3`, the scenarios from
`scratch/make_grid.py`, judged by `tools/session_matrix.py` and summed by
`scratch/grid_summary.py`, which judges S1 again from each run's own record of the air.*

## 1. S3, a good path: passes

A gateway (b) scans 2, 3 or 4 dials and a caller (a) sits on the last of them, calling with the
time the scan's answer gives (`connect call_s=scan`), then sends a kilobyte and disconnects. Ten
draws a cell, the call placed at ten points spread evenly over the gateway's cycle, +10 dB.

| cell | connected | dial faults | collisions | call, median | call, longest |
|---|---|---|---|---|---|
| AWGN, 2300 Hz, 2 dials | 10 of 10 | 0 | 0 | 13.8 s | 32.6 s |
| AWGN, 2300 Hz, 3 dials | 10 of 10 | 0 | 0 | 25.4 s | 56.4 s |
| AWGN, 2300 Hz, 4 dials | 10 of 10 | 0 | 0 | 38.6 s | 68.8 s |
| AWGN, 500 Hz, 2 dials | 10 of 10 | 0 | 0 | 13.8 s | 32.3 s |
| AWGN, 500 Hz, 3 dials | 10 of 10 | 0 | 0 | 25.6 s | 56.4 s |
| AWGN, 500 Hz, 4 dials | 10 of 10 | 0 | 0 | 37.4 s | 69.0 s |
| Moderate, 2300 Hz, 2 dials | 10 of 10 | 0 | 0 | 13.8 s | 32.2 s |
| Moderate, 2300 Hz, 3 dials | 10 of 10 | 0 | 0 | 25.4 s | 90.2 s |
| Moderate, 2300 Hz, 4 dials | 10 of 10 | 0 | 0 | 38.6 s | 68.9 s |
| Moderate, 500 Hz, 2 dials | 10 of 10 | 0 | 0 | 13.8 s | 34.2 s |
| Moderate, 500 Hz, 3 dials | 10 of 10 | 0 | 0 | 25.5 s | 56.4 s |
| Moderate, 500 Hz, 4 dials | 10 of 10 | 0 | 0 | 37.3 s | 68.8 s |

Every message crossed. The median call is about half a cycle of the gateway's scan, as it should
be for calls placed evenly over the cycle: 19.6 s a dial on the harness's stations.

## 2. S1, the wrong dial: passes so far

No acceptance and no frame keyed on a dial other than the one its call was heard on, in any of
the 140 runs (`dial_faults`, an invariant on every run). The planted calls of §5 are its sharpest
test.

**S2, held,** is not judged on the harness: the daemon records announcements only inside a
recording, and a gateway idle between calls makes none. Its rules are held by `scan.rs`'s station
tests (a move refused while a frame is announced and unreported, while an answer is queued, while
a call or session is up, while the identifier is owed), and broken they would show as S1 faults.

## 3. S4, a weak path: fails as registered

S4 asks for 8 of 10 calls connected "at the SNR where one try decodes about half the time (the
calling bench)", three dials. The calling bench's Moderate path gives two capped tries 26 of 30 at
−16 dB and 16 of 30 at −18 dB (`docs/research/calling/02-capped-call.md`), about 0.6 and 0.3 a try,
so the grid ran at −16 dB. The call is carried no further than its acceptance.

| cell | connected | dial faults | collisions |
|---|---|---|---|
| Moderate −16 dB, 2300 Hz, 3 dials | **6 of 10** | 0 | 0 |
| Moderate −16 dB, 500 Hz, 3 dials | **5 of 10** | 0 | 0 |

**The trace.** For each try, whether the gateway's radio was on the caller's dial at its start,
middle and end (`Y`), from the channel server's record of both stations' dials:

| draw | connected | tries (start, on the dial) |
|---|---|---|
| 2300 p1 | no | 8.9 `...`, 20.3 `...`, 31.7 `...`, 43.6 `YYY`, 55.5 `YYY`, 85.4 `YYY` |
| 2300 p2 | no | 14.8 `...`, 26.2 `...`, 37.6 `.YY`, 49.5 `YYY`, 61.4 `Y..`, 91.3 `...` |
| 2300 p7 | no | 44.3 `YYY`, 55.7 `YYY`, 67.1 `YYY`, 79.0 `YYY`, 90.9 `YYY`, 120.8 `YYY` |
| 2300 p9 | no | 56.0 `YY.`, 67.3 `...`, 78.8 `...`, 90.7 `...`, 102.5 `YYY`, 132.4 `...` |
| 500 p1 | no | 8.9 `...`, 20.3 `...`, 31.7 `...`, 43.6 `YYY`, 55.5 `YYY`, 85.4 `YYY` |
| 500 p3 | no | 20.6 `...`, 32.0 `...`, 43.4 `YYY`, 60.4 `...`, 72.3 `...`, 102.2 `YYY` |
| 500 p6 | no | 38.1 `.YY`, 49.5 `YYY`, 63.3 `YYY`, 75.2 `YYY`, 87.1 `YYY`, 117.0 `YYY` |
| 500 p7 | no | 44.2 `YYY`, 55.6 `YY.`, 67.0 `...`, 78.9 `...`, 90.8 `...`, 120.7 `...` |
| 500 p9 | no | 56.0 `YY.`, 67.3 `...`, 78.8 `...`, 90.7 `...`, 102.5 `YYY`, 132.4 `YYY` |

* **Where a try met the gateway, the call went on there.** In every failed draw at least one try
  fell wholly on the gateway's dial. In 2300 p7 and 500 p6 the gateway read a try, accepted it,
  and stayed on the caller's dial in the session it had accepted (76 and 79 s), but the caller
  never read either acceptance and its call ran out. Every call the gateway read drew an
  acceptance, in every draw of both arms.
* **No dial fault, no late answer.** The slow passes the 500 Hz gateways logged were the
  harness's lockstep sound card waiting on a loaded machine (a build ran beside these draws), not
  the station: a thread that only sleeps woke on time throughout.

## 4. S4's control: the same calls to a gateway that does not scan

The same twenty draws (same seeds, the call placed at the same moment, `call_s` 40, the scan's
own figure), the gateway sitting on the caller's dial (`s4-control`, made from the S4 files):

| | scanning (§3) | fixed |
|---|---|---|
| 2300 Hz | 6 of 10 | 9 of 10 |
| 500 Hz | 5 of 10 | 10 of 10 |

Draw by draw, eight calls the fixed gateway took were lost by the scanning one; none went the
other way (2300 p7 was lost by both). The fixed gateway took most calls at the first or second
try: tries on the dial from the start, where the fading happened to be kind. **So S4 measures a
real cost of the scan, and the reading at the end of §3 was wrong:** the path is not what lost
these calls. A scanning gateway catches about a third of a caller's tries until it reads one,
and at −16 dB on Moderate a try connects (read one way, its acceptance read the other) only
about one time in three, where the timing model (`00-timing.md` §4) assumed each part of a try
decoded half the time and tries were independent, giving 98–99 %. With six tries, of which
about two land on the gateway's dial, about half of the calls connect, as measured.

**The broken assumption** is the model's per-try success on a weak path: the ordinary part of a
both-ways try never decodes at −16 dB (the calling bench already said so), so only its tone part
counts, and a call needs its acceptance read too. What would meet S4 is a call long enough to
put four or five tries on the gateway's dial at such a path, about twice `scan_s`, or a gateway
that scans fewer dials. That is a change to the rule a caller follows (ADR-0129 §2.5), which waits
for the author.

## 5. S7, each part earns its place

Each part taken out of the daemon at `09662d3` in turn (one line each, in a scratch worktree), and
run on the same scenario files.

**The linger, on S4's twenty draws.** Off, 6 of 10 at 2300 Hz and 6 of 10 at 500 Hz, against 6 and
5 with it: 12 of 20 against 11. Paired by draw, linger on won 2300 p4 and 500 p8, linger off won
2300 p2 and 500 p6 and p7. **The linger fails nothing here, and S4's clause that it does at least
as well as no linger, paired by draw, misses by one call** (within what two runs of one build
differ by, ADR-0129 §6 having no room for it). The timing model credited it with taking a weak
path's calls from 79–89 % to 98–99 %, on the per-try success §4 found to be wrong.

**The hold and the restart, on twenty planted calls** (`make_grid.py --planted`): a caller on the
gateway's home dial, AWGN +10 dB, its first try timed against the end of the gateway's first
dwell there. The files aimed ten tries to end 0 to 0.9 s before it and ten to start 0.1 to 1.9 s
before it, from one run's timing; the scan's start landed 0.2–0.4 s later than that run's in
these, so below each try is placed by its measured lead, the dwell's end less the try's start.

* **The ten aimed to end before the move** started 6.0–7.7 s before the dwell's end: a try lasts
  6.4 s, so each was still on the air when the dwell ran out. Its first part (the OFDM call
  frame) decoded 5 s before, the call held the dial, and every build answered it at once
  (8.1–8.2 s). They test neither part.
* **The ten aimed to start before the move**, by measured lead:

| lead | full | hold off | restart off |
|---|---|---|---|
| −0.27 to +0.13 s | 39 s (3 calls) | 39 s (3) | 39 s (2) |
| +0.25 s | — | — | **lost** |
| +0.33 s | 39 s | 39 s | — |
| +0.37 to +0.55 s | 8.2 s (1) | 39 s (1) | 8.1 s (2) |
| +0.71 to +1.23 s | 8.2 s (3) | 39 s (3) | 8.1–8.2 s (3) |
| +1.33 to +1.47 s | 8.1 s (2) | 8.2 s (2) | 8.1 s (2) |
| dial faults | 0 | 0 | 0 |

* **A try that begins at least 0.37 s before the dwell's end holds the dial**: by then its call
  frame is announced, and the gateway stays to read it (8 s to connect). Closer than that, the
  gateway leaves before the announcement and the caller waits a cycle (39 s).
* **The hold earns its place, though not as registered.** Without it the gateway left mid-frame
  whenever the call frame could not also be decoded before the dwell ended (leads up to 1.23 s):
  four calls took 39 s instead of 8. It gave no S1 fault and lost no call, which is what S7 asked of
  it; the restart keeps a frame cut by a move from being answered on the next dial.
* **The restart earns its place, though not as registered.** Without it, at a lead of 0.25 s the
  receiver went on reading the start of a try heard on the dial it had left after the move, failed
  it as a trusted frame, and the gateway lingered 42 s on the dial it had moved to while the
  caller's next try went to the other: the call was lost. S7 expected an S1 fault; the frame never
  decoded, so nothing was keyed on the wrong dial, and the cost is a lost call.

## 6. S6, the end: passes

`bench/scenarios/40m-scanning-operator-wins-2300.toml`: another program's `frequency.set` and the
radio's knob each ended the scan, the owner was told why (`tuned by another program`, `knob`),
the radio stayed where it was put, and a later call there connected. The owner's connection
closing ends the scan and sends the radio home once it is free (`scan.rs`'s station tests).

## 7. S5, no harm

*Not run yet.*
