# A scan that stays on a call under way: the detector, and the numbers it gives

*For ADR-0129 §11. Run 2026-11-17 in a cloud container on the model at `65d89e7`, after S4 failed
(`01-results.md`). The author's design: a gateway need not sit a whole gap between a caller's tries
on a quiet dial; it can leave after a few seconds unless it notices a try under way, and then
stay for the next. The author asked that the detector be measured first, and that the number of
dials be capped where it is best. Scripts: `scratch/notice.py` (the detector),
`scratch/sense_timing.py` (the timing it gives).*

## 1. What a gateway that arrives partway through a try notices

`notice.py tries`: the caller's real first try to a station not heard lately (the model's engine:
the OFDM call frame at the robust rung, 1.05 s, then tone-24 with the same request, 5.36 s) through
the model's channel; a fresh receiver (the gateway just moved there) starts listening at a point of
the try, half a second after it arrived there (the move's deafness), and the first thing it
notices is recorded: an OFDM candidate, a tone frame announced as arriving, or a frame reported.
Both airs, AWGN and Moderate, +10, −10 and −16 dB, nineteen points from 1 s before the try to
6.2 s into it, six draws each.

Share of listens that noticed something at confidence 1.1 or more within 3.5 s of arriving (3 s of
listening), by where the gateway arrived (seconds into the try):

| | −1.0 … 2.6 | 3.0 | 3.4 … 5.4 | 5.8, 6.2 |
|---|---|---|---|---|
| AWGN and Moderate, +10 dB, both airs | 6 of 6 everywhere | 6 of 6 | 6 of 6 | 0 |
| −10 dB | 6 of 6 | 4–6 of 6 | 6 of 6 | 0 |
| AWGN −16 dB | 6 of 6 | 0–2 of 6 | 6 of 6 | 0 |
| Moderate −16 dB, 2300 Hz | 4–5 of 6 | 1 of 6 | 3 of 6 | 0 |
| Moderate −16 dB, 500 Hz | 3–4 of 6 | 2 of 6 | 5–6 of 6 | 0 |

* **A try is noticed from any point of it but its last second.** The tone frame's three sync blocks
  (at its start, after 45 % of its data and at its end) are each announced as an arrival by the
  receiver as it stands, without the block before them: a gateway that arrives after a try's start
  notices its middle or its end block within about 1.1–1.5 s.
* The one gap is a gateway arriving about 3.0 s into the try: its listening starts just after the
  middle block, and the end block's announcement comes after 3 s of listening. A dwell of 4 s
  (3.5 s of listening) closes it on good paths.
* A gateway that arrives before the tone frame's start reads the try as one listening all along
  does: at −16 dB on Moderate 4 of 6 at 2300 Hz and 3 of 6 at 500 Hz, about the S4 condition.

**Noise** (`notice.py noise`, ten minutes per air): at 2300 Hz one tone announcement at confidence
1.03; at 500 Hz four OFDM candidates, the largest 1.07, each reported as a failed frame. Nothing at
1.1 or more. So a notice is counted from confidence 1.1: noise then costs nothing measurable, and a
try under way loses at most one draw in six of its notices at −16 dB against counting from 1.0.

## 2. The timing it gives

`sense_timing.py` plays the rule against the engine's tries (0, 11.0, 22.0, 33.5, 45.1, 57.1,
69.1 s …), with the measured notices by arrival point, the measured share of tries read from their
start, and the measured noise, over random phases of the scan and the dial the caller is on (3 000
calls a cell). A gateway on a quiet dial leaves after the dwell; one that hears a try from its start
reads it; one that notices a try under way stays 19.2 s (the gap the engine leaves before its next
try, a try moved past a frame heard arriving, and the move's deafness) and reads the next.

Calls read, 4 s dwell, notices from 1.1:

| dials | good paths and −10 dB, call of 40 s | Moderate −16 dB, 500 Hz, 60 s | Moderate −16 dB, 2300 Hz, 60 s |
|---|---|---|---|
| 2 | 100 % | 91 % | 97 % |
| 3 | 100 % | 82 % | 86 % |
| 4 | 100 % | 85 % | 96 % |
| 5 | 100 % | 76 % | 85 % |
| 6 | 94 % | 72 % | 78 % |
| 8 | 90 % (99 % at 60 s) | 63 % | 70 % |

A 3.5 s dwell does a few points worse on good paths at 3 and 6 dials (the gap of §1); 4 s is taken.
The median call is read at the caller's second try (11 s) at every count. The model takes tries as
independent, where a fading path makes neighbouring ones alike: the harness's numbers will be lower
at −16 dB, as S4's control was against its timing model.

## 3. The choices

* **Dwell: 4.0 s** on a dial where nothing is noticed: the move's 0.5 s and 3.5 s of listening.
* **A notice:** an arrival announced, an OFDM candidate, or a frame reported that did not decode,
  at confidence 1.1 or more; a frame decoded for another station is not one.
* **The stay after a notice: 19.2 s** (19.6 s on the harness's stations), the dwell the scan had
  before: long enough for the caller's next try to begin and be announced, after which the hold
  keeps the dial while it is read. Stays do not chain on their own notices beyond the one gap.
* **What a caller must keep trying (`scan_s`):** a cycle of the dials and two stays,
  `n × 4.0 + 2 × 19.2` s rounded up: 47 s for 2 dials, 51 for 3, 55 for 4. Two stays, so a try
  noticed is followed by two that are read; the table's 60 s column is that much call, near enough.
* **The cap: 4 dials.** Up to 4 a scanning gateway reads every call on a good path and, in the
  model, more than 80 % at the weak path S4 is set at, both airs; at 5 the 500 Hz weak path falls
  to 76 %, at 6 good paths start to lose calls. Three-quarters of the Winlink HF gateways listed on
  2026-11-01 publish 4 dials or fewer (median 3, from the gateway list Pat ships).

## 4. On the harness, and the lock it found

The S3/S4 grid (`make_grid.py`, the dials 4 s each) on the daemon at `ed42e63`: S4 passed, 8 of 10
weak calls at 2300 Hz and 9 of 10 at 500 Hz (6 and 5 with the first design, 9 and 10 for a gateway on
a fixed dial). S3's good paths connected every call in a median 11–20 s and at most 31.5 s, but one
phase of the ten, the same in all four three-dial cells, never connected: three dials at exactly
4 s are a 12 s round, the caller's tries come 11.4–12.4 s apart on the harness's stations, and every
visit to the caller's dial fell in the quiet half of its cycle. The timing model had averaged over
phases and hidden it; asked for the worst of twenty phases, it shows it (three dials, good path: 93 %
on average, 64 % at the worst phase).

Each dwell drawn longer by a random part breaks the lock. Worst phase of twenty, `scan_s` from the
longest dwell:

| | 2 dials | 3 dials | 4 dials |
|---|---|---|---|
| 4 s exactly, good path | 100 % | 64 % | 100 % |
| 4–5 s, good path | 100 % | 100 % | 100 % |
| 4–5 s, Moderate −16 dB, 2300 / 500 Hz | 90 / 81 % | 86 / 77 % | 86 / 72 % |
| 4–6 s, good path | 100 % | 100 % | 98 % |
| 4–6 s, Moderate −16 dB, 2300 / 500 Hz | 80 / 74 % | 91 / 78 % | 79 / 67 % |
| a fresh order each round, 4 s, good path | 100 % | 87 % | 97 % |

A second of jitter is taken (`DWELL_JITTER_S`): it reads every call at every phase on good paths, and
more of the weak path than two or three seconds, which lengthen the round.

**The linger, on S4's draws** (the daemon at `ed42e63`, its notices' linger taken out, the contact's
kept): 3 of 10 at 2300 Hz and 5 of 10 at 500 Hz, against 8 and 9 with it; no draw connected without
it that failed with it. The linger earns its place.

## 5. The jittered build on the harness (`76853d1`)

**S1, S3 and S4.** The grid of `make_grid.py` (the call placed at ten phases of a 4.5 s mean dwell):

| | runs | connected | dial faults | call, median | call, longest |
|---|---|---|---|---|---|
| S3, good paths, 2–4 dials, both airs | 120 | 120 | 0 | 10.9–19.6 s | 43.4 s (4 dials) |
| S4, Moderate −16 dB, 3 dials, 2300 Hz | 10 | 9 | 0 | 61.0 s | 75.8 s |
| S4, the same, 500 Hz | 10 | 9 | 0 | 35.1 s | 101.8 s |

The lock of §4 is gone: every phase of every three-dial cell connected. S4 passes (8 of 10 a cell
asked). Run again on the same twenty draws, the build connected 9 and 8; the build of §4 (4 s
exactly) connected 8 and 8 on them. Medians move with the draw: the same build gave 61.0 and
46.2 s at 2300 Hz, the 4 s build 52.3 s, and a call that is read at its last try (70–76 s) or
its fourth (46 s) is the same pass. What one build gives on one draw differs by a call between
runs, so the jitter's gain on the weak path is within that; its gain at the locked phase is not.

**S7, the planted calls** (`make_grid.py --planted`, AWGN +10 dB, a call of the station's own two
tries, timed against the gateway's fifth dwell, on its home dial). The scan's dwells are drawn
from a sequence seeded by its dials and its connection, the same in every run, so each try is
placed against the dwell's planned end (the gateway's arrival, 4 s and the draw) whether or not
it left:

* **Ten tries began while the gateway was on the other dial**, and it came home 1.9–2.9 s into
  them. The full build noticed each, stayed, and read the call's second try (39 s), but for one
  it came home to 2.89 s into: the stay ended 2.6 s before the second try, which comes after
  the call's pause (15 s and up to a try's length), and the call was lost. The build without the
  hold did the same. **The build without the restart read the first try of all ten (10.9 s).**
* **Ten tries began before the dwell's end**, by 0.12 to 1.84 s:

| try began before the dwell's end | full | no hold | no restart |
|---|---|---|---|
| −0.02 s (after it) | — | lost | 41.8 s |
| 0.12–0.28 s | lost (0.18) | 39 s (0.12) | 8.1–8.2 s (0.26, 0.28) |
| 0.40–1.84 s | 8.1–8.2 s (9) | 8.1–8.2 s (8) | 8.1–8.2 s (7) |
| dial faults | 0 | 0 | 0 |

* **The hold shows nothing on a strong path, and earns its place on a weak one.** A try heard
  from its start is announced at 1.1 or more, and the notice's stay keeps the dial longer than
  the hold would. On S4's twenty draws the build without it connected 15, the full build 17 and
  18 in two runs, and it lost two draws that both runs with the hold connected (2300 Hz phase 8,
  500 Hz phase 6). An announcement under 1.1 is what only the hold keeps the dial for.
* **The restart, as built, keeps the receiver from reading a try it arrived into.** A new
  receiver has no audio from before it started, so a try begun on the new dial before the
  gateway got there can be noticed (§1) but not read: its tone frame needs its first sample
  inside the receiver's stream. Without the restart the receiver carried on with the old dial's
  noise in that place and read the frame from its later sync blocks. On S4's draws the build
  without it connected 18 of 20. No dial fault came of taking it out here; on the first design
  one call was lost to a frame begun on the dial it had left (`01-results.md` §5), and the
  restart is what keeps the old dial's audio out of the new one's frames. §6 keeps it and gives
  the new receiver what it lacked.
* **A call that does not last loses to the scan.** The planted calls are the station's own two
  tries with the pause between them, not a call lasting `scan_s` (50 s for 2 dials). Two of the
  twenty were lost on the full build, each after a try the gateway noticed and stayed for: the
  pause before a call's last try is longer than the stay. Through the timing model of §2 (a
  second try after 15 s and up to a try's length), a two-try call reaches a scanning gateway on
  a good path 72, 55 and 43 % of the time with 2, 3 and 4 dials; a stay long enough to cover the
  pause (26–29 s) would make it 98–100, 83–85 and 63–64 %, and `scan_s` 15–20 s longer.
  RadioPost's client calls for `scan_s`; a program that calls without it (the VARA port's hosts,
  the panel's Connect: neither can ask for a call's length) reaches a scanning gateway only as
  often as that. Not changed: the author's call (ADR-0129 §12).

## 6. Reading a try under way: silence behind the new receiver

`scratch/prefix.py` starts a receiver partway through the caller's real first try, as
`notice.py` does (the move's half second of deafness after the arrival), three ways: new with
nothing behind it, as built; new with 6 s of silence fed before the arrival; and new with 6 s of
another dial's noise there, as a receiver never started again would have. Did the try's tone frame
decode? Four draws a point; "early" is an arrival 1.4–2.6 s into the try (0.85–2.05 s into its
tone frame, after the deafness), "late" 3.0–4.6 s:

| | nothing behind | silence | another dial's noise |
|---|---|---|---|
| AWGN, +10 and −10 dB, both airs, early | 0 of 64 | 64 of 64 | 64 of 64 |
| Moderate +10 dB, both airs, early | 0 of 32 | 32 of 32 | 32 of 32 |
| Moderate −10 dB, 2300 Hz, early | 0 of 16 | 10 of 16 | 9 of 16 |
| Moderate −10 dB, 500 Hz, early | 0 of 16 | 7 of 16 | 7 of 16 |
| every row, late | 0 | 0 | 0 |

* **A tone frame begun before the receiver is read from its later sync blocks once it has
  somewhere to begin.** The detector takes a frame whose first block does not stand from its two
  best (ADR-0049, ADR-0070); the decoder reads the part before the arrival as erased. Silence does
  as well as noise: the receiver is fed silence already for its own transmissions (`Air::take_block`).
* **Late, nothing reads it**: the middle sync block (after 45 % of the data) has gone by, and an
  end block alone is a notice (§1), not a frame.
* **The cost** is the receiver's work on the silence at each move: 5.4 s of it (the longest tone
  frame) takes 80 ms on the cloud container, once every 4–5 s while the station scans.

So the scan keeps rule 2's restart (nothing of the old dial reaches the new one's frames) and
gives the second start, once the move's deafness is over, the longest tone frame's worth of
silence behind it (`Air::listen_after_silence`; `air/hf.rs`'s
`a_receiver_given_silence_behind_it_reads_a_try_begun_before_it` holds both airs to it: nothing
read with nothing behind, the frame read and placed where it began with silence; and
`station/scan.rs`'s `a_try_begun_before_the_scan_arrived_is_read` holds the scan to it, a real
call reached two seconds late, which fails with nothing behind the receiver). An OFDM frame begun
before the arrival is lost either way: its preamble is its start.

## 7. The build with silence behind the new receiver, on the harness

The daemon with §6's change (`aetherd-scan4`), on the same files as §5:

| | the build before (§5) | with silence behind the receiver |
|---|---|---|
| S3, good paths, 2–4 dials, both airs | 120 of 120 | 120 of 120 |
| S3, median call at 3 and 4 dials | 19.5 s | 15.2 s |
| S3, longest call (4 dials) | 43.4 s | 34.2 s |
| S4, three passes of the same 20 draws | 54 of 60 (cells 9, 9 · 9, 8 · 9, 10) | 54 of 60 (cells 10, 10 · 7, 10 · 10, 7) |
| S7, the planted calls | 18 of 20 | 20 of 20 |
| dial faults | 0 | 0 in 200 runs |

* **On a good path a try the gateway came home into is read at once.** Nine of the ten planted
  tries it reached 2.0–2.8 s late were read at the first try (10.9 s; 39 s before). The tenth,
  reached 2.93 s in, after its middle sync block, was read at the next try (41.7 s). The two
  tries that began 0.04 and 0.22 s before a move, which it left before they were announced,
  were read at the next try (41.8 s), where the build before lost one of its two. S3's calls at
  three and four dials come a try sooner.
* **On the weak path it changes nothing**, as §6 predicted (a part of a tone frame at −16 dB
  rarely decodes): 54 of 60 each. Two cells of the six fell to 7, under S4's 8; every call lost
  in them connected in another pass of the same build. At 90 %, a cell of ten comes out at 7 or
  fewer one time in fourteen, and once in six cells about a time in three: S4's per-cell count
  is noisier than the difference it was set to see, and the pooled count is the measure.

