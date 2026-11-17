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

* **The scan found the caller.** In every failed draw at least one try fell wholly on the
  gateway's dial, and in four of the nine, three to six did. The linger did its part: in 2300 p7
  and 500 p6 the gateway, having read the caller's frames and failed them, stayed on the caller's
  dial for 76 and 79 s, every try heard.
* **The path did not carry the try on the harness.** Of the tries wholly on the gateway's dial,
  8 of 19 drew an acceptance at 2300 Hz and 8 of 22 at 500 Hz: about 0.4 a try, where the calling
  bench gives about 0.6 at the same SNR, and fewer still connected, since the caller must read the
  acceptance too. Six tries on the dial in a row went unanswered in one draw of each air.
* So the registered SNR, read from the calling bench, is weaker on the harness than the criterion
  meant, and S4 as run measures the path's reach as much as the scan's. Whether the scan costs
  calls is the question the control of §4 answers.

## 4. S4's control: the same calls to a gateway that does not scan

*Running.*

## 5. S7, each part earns its place

*Running: linger off on S4's draws; the hold on an announced frame off and the move's restart
off, each on twenty planted calls whose first try ends 0 to 0.9 s, or starts 0.1 to 1.9 s,
before the scan's first move (`make_grid.py --planted`).*

## 6. S6, the end: passes

`bench/scenarios/40m-scanning-operator-wins-2300.toml`: another program's `frequency.set` and the
radio's knob each ended the scan, the owner was told why (`tuned by another program`, `knob`),
the radio stayed where it was put, and a later call there connected. The owner's connection
closing ends the scan and sends the radio home once it is free (`scan.rs`'s station tests).

## 7. S5, no harm

*Not run yet.*
