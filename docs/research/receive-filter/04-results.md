# Gate 3: the prototypes, judged

*Written 2026-11-09, after the author's go for Gate 3. The criteria are `03-candidates.md` §3's,
unchanged. Where a candidate misses one, or where the bench itself was found wrong, it is said
here with its trace, for the author's decision at Gate 4 (§11).*

## 1. What was built

* **`crate::passband`** (`core/aetherd/src/passband.rs`): the candidates beside today's estimate,
  and C1. Nothing in it runs in the station: it runs only in `aetherd --passband-trace WAV
  [--expect SIDECAR] --passband-variant r0|r1|r2|r3 [--no-clip-veto]`, which adds to every line
  the candidate's verdict (`waiting`, `unknown`, `clear`, `top`, `bottom`, `both`, and R0's
  `narrow`), the drop at each edge of the signal's span, how far the look stood over the audio
  chain's floor, the looks kept, and C1's share of samples at full scale. **R0 is today's
  estimate and reproduces today's trace column for column** (checked on Saturday's 00:54Z
  session).
* **The corpus** (`tools/rxfilter_corpus.py`): the 130 clips, and the two kinds §4 of the
  candidates registered. *Line* clips: a cut of 12, 15 or 20 dB at one edge of one bandwidth's
  signal, steady, over either floor (24; the 20 dB ones added so the 500 Hz air, where no corpus
  filter cuts 20 dB, has one). *Joined* clips: a cut clip followed by its uncut twin, or the
  reverse, steady and on the FT8 band, over either floor (24). `judge` runs any candidates at both
  bandwidths and judges every reading, not only the last: a side 20 dB or more down must be
  named, a side 10 dB or less must not be, a side between is free. `field` runs them over the
  nineteen field recordings with their sidecars; `harness` over both stations' recordings of the
  cut grid.
* **The cut grid** (`tools/rxfilter_cost.py`) keeps its audio with `--keep-audio`.
* **Two faults in the bench, found and fixed** (§6.1, §7): the cut grid's bottom cuts were deeper
  than named, and the corpus's FT8 band reached full scale.

## 2. R3, as it stands

Every half second that the station listens (not keyed, not deaf after keying, not counting
itself as receiving a frame), it takes a spectrum of the last 85 ms up to 10 kHz.

1. **It leaves out the bins a signal holds**: a bin 14 dB or more over the lower quartile of the
   150 Hz on each side of it, and the two bins beside it (the window's main lobe).
2. **It holds the look** for the air's longest frame and half a second (5.9 s), and drops it if
   a frame announced or reported by then lay under it.
3. **It divides the look by its own level**, the ninth decile of the bins left in the signal's
   span, and keeps it only if that level stands 12 dB (the line) or more over the chain's floor,
   the median of 6–10 kHz, where no radio passes audio. A look whose band noise has sunk to the
   chain's floor (an AGC's dip, low audio) says nothing about the filter.
4. **The profile** is each bin's median over the last 120 looks kept, from the looks it was free
   in, and only where it was free in half of them; smoothed by a running median over ±50 Hz
   (where half that window is present).
5. **The reading**: the drop at each edge of the span (360 and 2640 Hz, or 1260 and 1740 Hz)
   below the smoothed profile's ninth decile over the span. 12 dB or more names that side; both
   sides, "both". Twelve looks kept are needed; twelve left out for the floor before twelve are
   kept is "can't check".

It needs no quiet window, so a band of FT8 reads as fast as a quiet one, and no busy detector.
Its constants are eleven against today's twelve (§9).

## 3. How R3 got there

R3 was built in steps, each judged on the corpus (draw 0) and the field together; each step is
a change with the evidence that called for it. *K1* counts the 48 cuts of 20 dB or more named
right by the end, *K2* the 160 uncut clip-bandwidths (both edges 10 dB or less) that ever warned.

| Build | Change | Why | K1 | K2 |
|---|---|---|---|---|
| 1 | as registered: the level at the span's upper quartile; a bin left out 10 dB over the median of its sides, three passes; each edge the mean of its ±50 Hz | — | 45 | 0 |
| 2 | the level at the ninth decile | a 500 Hz filter under the 2300 Hz signal passes a fifth of the span; the upper quartile fell in the cut, and over the shallow floor every look was taken for one at the floor (the three misses) | 48 | 9 |
| 3 | the main lobe beside a bin left out, left out too | on the FT8 band the window's skirt under 10 dB still stood over the noise and raised the level 2–4 dB | 48 | 6 |
| 4 | the profile smoothed by a running median before the edges are read | on a skirt the mean of decibels reads the slope's curve (8 dB at 360 Hz read 11), and a profile of few looks set the ninth decile high | 48 | 5 |
| — | a bin held in a look counted as high in its median (tried) | the looks that kept a half-held bin are those where a signal's skirt slipped under the test | 48 | 4, two standing to the end |
| 5 | each side's noise its lower quartile, 14 dB over it; a bin's median only when free in half the looks; the smoothing only where half its window is present | two FT8 signals 35 Hz apart: one held the other's side, and the bins at their margins kept medians 4–12 dB high | **48** | **0** |

The tried step was rejected: its false warnings stood to the end of their clips, where build 4's
came early and passed. Then each constant that might not earn its place was tried without: one
pass of the leave-out instead of three, and no margin over the line in the floor test, changed
nothing on the corpus or the field, and went; without the main lobe (K2 4), or with one
neighbourhood of 150 Hz (K2 1) or 100 Hz (K2 1) for both the leave-out and the smoothing, it
lost, and they stayed.

The harness then found the step that the corpus could not (§6.2): half of R3's looks in a
session fell under the other station's frames, which are announced only part way in and reported
at their end, and their edges are the span's own. **The hold** (step 2 of §2) came from that; it
delays every reading by 5.9 s and changes nothing else on the corpus or the field.

**A held-out draw.** The corpus drawn again (`make --seed 1`: new noise, new FT8 stations), after
R3 was settled: K1 48 of 48, K2 0 of 160, K3 0, K5 24 of 24, the joined clips 24 of 24 (§4).
Today's estimate there: 5 of 48.

## 4. The corpus (K1, K2, K3, K5)

Draw 0, every reading judged, at both bandwidths. *K5 in time*: of the cuts named on a steady or
FT8 band, those named within 15 s (steady) or 60 s (FT8). *Joined*: the cut-then-uncut clips
cleared within 60 s of the change, and the uncut-then-cut clips warned of the cut made.

| | K1 (of 48) | K2 (of 160) | K3 | K5 in time | joined: cleared / warned |
|---|---|---|---|---|---|
| R0, today | 8 | 4 | 4 | 2 of 5 | 10 / 2 of 12 |
| R1 | 41 | 3 | 3 | 13 of 24 | 6 / 7 |
| R2 | 41 | 6 | 7 | 12 of 23 | 7 / 4 |
| **R3** | **48** | **0** | **0** | **24 of 24** | **12 / 12** |
| R3, held-out draw | 48 | 0 | 0 | 24 of 24 | 12 / 12 |

(R1 and R2 with C1's veto off; with it, the same. R0's counts differ from Gate 0's "10 of 60" and
"two false" because every reading is judged here, at both bandwidths, and the hot clips are C1's.)

* **Time.** R3 names a cut at 11.5 s on a steady band and on the FT8 band alike (12 looks and the
  hold), by 13.0 s under QSB and 15.5 s under the AGC; a cut taken away clears 21–41 s after the
  knob, a cut made warns 31–52 s after it. R1 and R2 wait for whole quiet windows: on the FT8 band
  they took 30–90 s.
* **Where R3 puts its line** (the line clips, judged against nothing): over the deep floor it reads
  the cut itself, 12 dB as 11.2–13.1, 15 as 13.4–15.6, 20 as 15.7–20.8; over the shallow floor a
  20 dB cut reads 13–14 dB at 2300 Hz, as §1 of the candidates predicted. One 20 dB cut stays under
  the line: the 500 Hz top cut over the shallow floor reads 11.6, because a filter 20 dB down at
  1740 Hz has its corner at 1306 Hz and the whole 500 Hz span on its skirt, so the span's own
  level is already 2 dB down.
* **R2's misses** are the AGC over the shallow floor (no look stands over the floor: "can't
  check") and the narrow filter over the shallow floor; its false warnings, the FT8 band's signals
  in its quiet looks. **R1's**, the same AGC case read wrong, and the narrow filter.

## 5. The field (K2, K4)

The nineteen recordings, each with its sidecar, C1's veto off (§7):

| | working stations warned (of 14) | K4 (of 5) |
|---|---|---|
| R0, today | 0 | 2 (the two 500 Hz sessions, silent) |
| R1 | 1 | 4 |
| R2 | 1 | 5 |
| **R3** | **1** | **5** |

* **Saturday.** R3 names the top in all three 2300 Hz sessions at 11.5 s from the recording's
  start, 27.7–29.3 dB down (the frames' witness: 22–31 dB from 2.0 to 2.2 kHz), and reads the two
  500 Hz sessions' 1740 Hz edge at 7.5 and 6.6 dB, against the witness's 8: K4, 5 of 5. A live
  station, which listens before a session, would have its reading standing when the session began.
* **The gateway's FTDX10** reads 6–9 dB at 2640 Hz in its 2300 Hz sessions (its own slope, 7–9 dB
  at 2.6–2.8 kHz) and stays silent: 3 dB under the line.
* **The one working recording that warns** is the mobile's 2300 Hz session of 2026-11-07 15:06Z
  (`150601`): R3 says the top is 14–16 dB down from 11.5 s. Gate 0 found that afternoon's noise
  profile 16 dB down at 2.6 kHz (`01-evidence.md` §4), and Gate 1 found the gateway's OFDM frames at
  the mobile 7 and 11 dB down at 2.5 and 2.6 kHz (`02-survey.md` §5): two witnesses that agree with
  the reading, an edge in the free band between 10 and 20 dB, and a knob that may already have been
  part way round. The criteria list it as a working station. **For the author** (§11, 2).

## 6. The harness (K1, K2 on the harness; K8)

The cut grid, three draws of each configuration, both stations' recordings, R3 over each with its
sidecar. Every run passes as the shipped build's do, with no radiated overlap: the daemon is
today's, and the candidates run only in the trace.

### 6.1 A fault in the grid, and the cost table corrected

The grid placed a bottom cut's corner by an eighth-order low-pass prototype's formula, but the
channel server builds a band-pass, whose lower skirt is steeper. The bottom cuts were deeper than
named; the top cuts and Saturday's receiver were right:

| Named bottom cut | 3 dB | 6 dB | 10 dB | 20 dB |
|---|---|---|---|---|
| at 2300 Hz, really | 3.0 | 6.8 | 11.9 | 24.4 |
| at 500 Hz, really | 3.0 | 10.2 | 19.8 | 42.1 |

`rxfilter_cost.py` now finds the corner by bisection on the filter the channel server builds, and
the bottom cuts were run again (three draws each). Gate 0's table (`01-evidence.md` §8) and the
candidates' (`03-candidates.md` §1) read again with the real depths:

| Cut at the signal's edge | 2300 Hz message | file | 500 Hz message | file |
|---|---|---|---|---|
| none | 1.00 | 1.00 | 1.00 | 1.00 |
| top 3, 6, 10 dB | 1.00, 1.00, 1.00 | 1.00, 1.02, 1.00 | 1.00, 1.00, 1.00 | 1.00, 1.00, 0.65 |
| top 20 dB (floor 15 / 50 dB down) | 1.00 / 1.00 | 0.89 / 0.86 | 0.87 / 0.93 | 0.98 / 0.99 |
| bottom 3, 6, 10 dB | 1.00, 1.00, 1.00 | 0.93, 0.93, 1.00 | 1.00, 1.00, 0.96 | 1.00, 0.99, 0.99 |
| bottom 20 dB (floor 15 / 50 dB down) | 0.76 / 0.70 | 0.98 / 0.89 | **1.02 / 0.94** | **1.00 / 0.94** |
| Saturday's receiver | 0.63 | 0.82 | 0.96 | 0.90 |

*(A fresh run of the whole grid on beta.144's receiver, the bottom cuts at their real depths; every
run passed. The 500 Hz top 10 dB file's 0.65 is the median of 1.00, 0.65 and 0.60, its message's
1.00, 1.09 and 0.62. The earlier run of the same receiver, the candidates' table, had 1.00, 0.72,
0.99 and 1.00, 0.96, 0.77 there: the same two draws lost in both runs, and on the third draw the 6
dB cut lost in both runs too (message 0.62–0.77) while its median stayed 1.00. Ten draws of the 500
Hz top cuts, run again, settle it: 3, 6 and 10 dB cost nothing (medians 1.00/1.00, 0.99/1.00,
0.99/0.98 message/file; the 10 dB cut's worst single draws, 0.77 and 0.72, are the spread a 3 dB cut
shows too, 0.94 and 0.88), and 20 dB costs (0.90/0.84, five draws of ten under 0.85 on the file).
The 0.65 was three draws' spread.)*

What changes: **a 20 dB cut at the 500 Hz bottom edge costs nothing** (0.94–1.02). Gate 0's 68–82 %
there was the 42 dB cut, and the candidates' "the bottom one most (at 500 Hz it is the tone
detector's)" rested on it. ADR-0104's bench (`bench_cut.py`, its own analytic high-pass, as deep as
named) is not touched; its harness line on "the 500 Hz bottom 20 dB cut" was the 42 dB one. What
stands: **10 dB costs nothing anywhere; 20 dB costs at the 2300 Hz bottom (24–30 % of the
message), at the 500 Hz top (7–13 % of the message on three draws; 10 % and 16 % of message and
file on ten) and a step at the 2300 Hz top's file.** The line between 10
and 20 dB holds at three edges of four; at the 500 Hz bottom it is early (§11, 4).

### 6.2 R3 on the grid

*(R3 with the hold of §3, against each run's real losses.)*

| | cuts of 20 dB or more named right | uncut or ≤10 dB recordings warned | a side not cut named | C1 on hot runs / others |
|---|---|---|---|---|
| R0, today | 0 of 24 | 0 of 138 | 0 | (none) |
| **R3** | **21 of 24** | **0 of 138** | **0** | **24 of 24 / 0** |

*(Both stations of all 96 runs; a cut is the called station's, the caller's radio is untouched.
The hot runs are judged for C1 alone; without the veto R3 also names Saturday's receiver's top cut
in all three at 14.3–16.3 dB.)*

The three misses are the 500 Hz top 20 dB cut over the shallow floor, all three draws, and R3
says "can't check" in each, not a wrong word. Two things make it: a filter 20 dB down at 1740 Hz
puts the whole 500 Hz span on its skirt (§4, the line clips), and the called station keeps only 10
or 11 looks in its whole recording. In a 500 Hz session it is hearing a frame or answering one
nearly all the time, and the recording's 23 s before the session hold its answer to the probe,
whose keying the sidecar does not list, so those looks count as ones at the floor. A live station
reads its profile while idle, before its sessions; a recording of a session shows only the
session. The other 21 are named 23.3 s from the recording's start (17 of them), 41.8–42.8 s (3)
and 147.8 s (1), all before the session's end.

### 6.3 The witness (K8)

W1 may only bring forward a warning the noise agrees with (the author's answer 1). On the field, the
first frame that spans a cut edge came long after the noise had named the cut: in Saturday's 00:54Z
session the mobile decoded its first frame at 30 s, a slow tone frame that spans only 1300–1700 Hz,
and its first frame spanning the cut edge (its one tone100-105) at 263 s; R3 named the cut at 11.5
s. On the harness, a frame spanning the edge decoded before R3's warning in 4 of the 21 named cuts,
all at 500 Hz: at 31.7 s against R3's 41.8–42.8 s in three, and 147.8 s in one. In all four R3 had
no reading until it named the cut: it was waiting, or "can't check", with 11 looks. Under the
author's answer the frames only bring forward a warning the noise agrees with, and there was no
reading to agree with; to act there they would have to warn on their own, which the answer rules
out. **So W1 brings nothing forward, and by K8 it goes.** That case arises only where a session
begins as the station starts listening; a live station listens before its sessions.

## 7. Clipping (K6), and the veto

C1 is the share of samples at full scale (0.99 or more, as the Receive level tile counts them)
over the last 10 s of the audio the station heard; over 0.1 % the audio clips.

* **The corpus**: C1 on all 52 hot clips within 10 s, and on none of the others, once the corpus
  was fixed: its FT8 band (ten stations up to 15 dB over the noise, on a codec set for −30 dBFS of
  noise) reached full scale in 32 clips, 1.3 % of the samples in one, and clipping is the hot
  condition's alone. Clips outside it are now turned down to a −1 dBFS peak where they would pass
  it; that changes their level, not their shape or their labels.
* **The field**: C1 on all five of Saturday's sessions, 0.02–4.1 s from the recording's start
  (2.6–3.8 % of the samples heard clipped, up to 7.3 % over ten seconds), and on none of the
  other fourteen.
* **The harness**: C1 on all 24 hot runs and none of the others; the four that took 18–22 s are
  driven only 6–8 dB hot and reach full scale only when the other station's bursts arrive.

**The veto, as registered, cannot meet K4.** §2.3 of the candidates registered that while the
audio clips the passband reading takes no looks, "a clipped spectrum is not the radio's
response". Saturday's three 2300 Hz sessions are clipped and cut at once: with the veto, every
candidate stays at "waiting" through them, and K4 is 0 of 5. The evidence does not bear the
veto's premise out at these levels: without it R3 names all twelve hot corpus clips that are cut,
on the right side, warns on none of the forty uncut ones, names Saturday's top at 27.7–29.3 dB
while 2.6–3.8 % of the samples heard clip, and names the hot harness runs' cut (Saturday's
receiver) at 14.3–16.3 dB. **For the author** (§11, 1).

## 8. The other criteria

* **K3, the words**: R3 never named a side not cut, on the corpus (both draws), the field or the
  harness. A narrow filter reads "both", a moved passband one side.

## 9. Simplicity and cost (K7, K9)

* **Constants**: eleven against today's twelve. R3's: the line (12 dB), the edge smoothing
  (±50 Hz), the floor's band (6–10 kHz), the looks needed (12) and kept (120), and for the
  leave-out the threshold (14 dB), the sides' quantile (lower quartile) and width (150 Hz) and the
  main lobe (2 bins), the level's quantile (ninth decile); and the station's look interval (0.5 s).
  The span is the waveform's own (its centre and occupied width), so `AUDIO_CENTER_HZ` is not the
  estimate's; the minimum level (`PASSBAND_MIN_LEVEL_DBFS`) goes, the floor test refusing a silent
  input; the panel's 400 Hz goes; the hold is the air's longest frame. C1 has its own three: the
  full-scale sample as the Receive level tile counts it, 10 s, and the panel's 0.1 %.
* **Code**: the prototype module carries about 260 lines for R3 with its scaffolding (the
  variants, the trace's fields); an R3-only port comes to about 210 for the reading, against
  today's about 150 in `spectrum.rs`, 40 in the station and 20 in the panel. The port will hold
  the count (Gate 5); it is about level with today's, not under it.
* **Tests**: the prototype has its own (a moved passband, a cut low edge, a narrow filter both
  sides, a cut over a shallow floor, looks at the floor, a look at any level, a band of signals
  left out, an edge a signal always holds, the main lobe, clipping). Today's eight, in their new
  form, are the port's.
* **K9, the cost**: the trace over a six-minute field recording (011008, 362 s of audio) took
  1.41–1.55 s of CPU with today's estimate alone and 1.36–1.50 s with R3 beside it, best of three
  each, and over the quiet minute 0.47–0.50 s against 0.50–0.58 s: R3's own cost is inside the run's
  noise, under 0.1 s in 362 s, about 0.03 % of real time on this machine; ten times that on a
  slower one is still under K9's 2 %. The harness run with R3 live belongs to the port; offline it
  changes no session, and every run of the grid passed as the shipped build's do.

## 10. The criteria, for R3 and C1

| | Criterion | R3 + C1 |
|---|---|---|
| K1 | 46 of 48 cuts of 20 dB or more named right | 48 of 48 (both draws); harness 21 of 24, the three "can't check" (§6.2) |
| K2 | no warning on an uncut station | corpus 0 of 160 (both draws), harness 0 of 138; field 1 of 14 (§5, for the author) |
| K3 | every warning names the side cut | met everywhere |
| K4 | Saturday: top named, 500 Hz edge within 3 dB | 5 of 5 without the veto, 0 of 5 with it (§7, for the author) |
| K5 | 15 s steady, 60 s FT8; clear within 60 s | 11.5 s and 11.5 s; cleared 21–41 s |
| K6 | C1 on Saturday's five within 10 s, on no other; every hot clip | met: 0.02–4.1 s; 52 of 52; none other |
| K7 | fewer constants, no more code | 11 against 12; code about level (§9) |
| K8 | W1 stays only if it brings a warning forward | W1 brings nothing forward: it goes (§6.3) |
| K9 | runs as shipped; ≤ 2 % of real time | about 0.03 % of real time; every grid run passed (§9) |

## 11. For the author

1. **The veto.** Registered as "no look while the audio clips", it makes K4 impossible on
   Saturday's sessions, and the evidence says a clipped spectrum still shows the filter (§7). The
   proposal: drop the veto; C1 warns on its own (its banner: turn the radio's audio out or the
   sound card's input down), and the passband reading goes on.
2. **The 2026-11-07 15:06Z recording.** Listed as a working station, it reads the top 14–16 dB
   down, which both witnesses of that afternoon agree with (§5). The proposal: count it in the free
   band between 10 and 20 dB, where the criteria ask nothing, and not as a false warning.
3. **The witness.** W1 brought nothing forward under answer 1 (§6.3), and by K8 it goes. The
   frames would only have mattered where a session starts with the station, and only by warning
   on their own. Confirm that it goes.
4. **The cost table.** The bottom cuts of Gate 0's table were deeper than named (§6.1). The line
   at 12 dB stands on the top edge and on the 2300 Hz bottom edge; at 500 Hz the bottom edge costs
   only far deeper than 20 dB (§6.1's corrected table). Nothing in R3 changes for it, but the
   500 Hz bottom edge warns from 12 dB down, where a cut costs nothing yet.

### The author's answers (2026-11-09)

All four proposals agreed: the veto is dropped and C1 warns on its own; the 2026-11-07 15:06Z
recording counts in the free band; W1 goes; the line stays at 12 dB at every edge, the 500 Hz
bottom edge's early warning accepted. The decision is ADR-0105.

Gate 4 is the decision ADR: R3 and C1 (with the author's answers), what they remove —
`PassbandMonitor`, `passband_edges` and their eight constants, the station's wait for quiet
looks, `PASSBAND_MIN_LEVEL_DBFS`, the panel's 400 Hz and its width warning — and the port's plan:
the banner in every view, the quiet "can't check" line in the Status tab, the manual.

## 12. The port (Gate 5)

ADR-0105 built, 2026-11-09. `crate::passband` is the station's reading (`ReceiveFilter`) and C1
(`Clipping`); `Station::sample_passband` steps it every pass with the station's own rule for
listening (not keyed, not deaf to its tail, not counting a burst as arriving), and every frame
the receiver announces or reports goes to it. The old estimate, its constants, the quiet-window
wait, the review's variants and the panel's width warning are deleted.

**The code is about twice the old estimate's, not level as §9 and ADR-0105 §3 expected.** Counted
without comments and blank lines: the reading 342 lines and C1 34 (`passband.rs`, before its
tests), the station's look loop 15 and the panel 121 (the two banners with their dismissal, the
tile, the clipping line), about 510 in all; the old estimate was 180 (`spectrum.rs`), its loop 39
and the panel's warning 15, about 235. §9's "about 210" counted the reading as the prototype held
it, before the hold, the span shift, the move between bandwidths and the record of each look were
in it, and nothing for the panel. The difference is what the ADR asked for (the warnings in every
view with their dismissal, C1, the hold against frames), and the tests cover cases the old one
never did. The constants are eleven against twelve, as counted.

**Found by the port.** A frame's span is placed on the receiver's clock, which runs about 20 ms
behind the audio (`AudioToBaseband::rx_delay_samples`: the decimator and the band filter), while a
look is timed on the audio. The prototype compared the two as they came, so a look ending in a
frame's first 20 ms was kept. The station test that sends OFDM frames a little before a look is
due found it; the reading now shifts a frame's span by the delay. On the field it changes the
looks kept by one at most and no reading's verdict (below).

**The checks, through the built daemon's `--passband-trace`:**

| | corpus, draw 0 | corpus, held-out draw | field |
|---|---|---|---|
| K1, cuts of 20 dB or more named (of 48) | 48 | 48 | |
| K2, uncut warned | 0 of 160 | 0 of 160 | 1 of 14 (the 15:06Z recording, in the free band, §11) |
| K3, a side not cut named | 0 | 0 | 0 |
| K4, Saturday (of 5) | | | 5 |
| K5, in time / joined cleared / joined warned | 24 of 24 / 12 of 12 / 12 of 12 | 24 of 24 / 12 of 12 / 12 of 12 | |
| K6, C1 on hot clips in time / on others | 52 of 52 / 0 | 52 of 52 / 0 | Saturday's 5 of 5 / 0 |

Every number is Gate 3's: R3 as built reads the corpus and the field as the prototype did. The
cuts are named at 11.46 s on a steady band and on the FT8 band; Saturday's three 2300 Hz
sessions read 27.7–29.3 dB down at the top, the two 500 Hz ones 7.5 and 6.6 dB at 1740 Hz.

**The cut grid on the built daemon (K9)**: all 32 configurations × 3 draws, 96 runs, the reading
live in both daemons. 96 of 96 pass, with no collision and no radiated overlap. The Test rates
against each draw's uncut run (median of three): at 500 Hz every cut of 10 dB or less reads
0.96–1.00 and a 20 dB bottom cut 0.98–1.03; at the 2300 Hz top 10 dB or less reads 0.97–1.00.
The 2300 Hz bottom's message reads 0.70–0.86 at every depth, 3 dB included, and its file
0.89–1.00 at 10 dB or less (0.81–0.85 at 20): the message moves in whole steps on this grid, so the bottom edge's small cuts are not
told apart from its draws here. Saturday's receiver costs the 2300 Hz Test 37 % of its message and
23 % of its file, and the 500 Hz one nothing.

**The reading over the grid's recordings** (`rxfilter_corpus.py harness`, both stations of every
run):

| | Gate 3 (prototype, its own grid) | Gate 5 (as built, this grid) |
|---|---|---|
| cuts of 20 dB or more named right | 21 of 24 | 20 of 24 |
| uncut or ≤ 10 dB recordings warned | 0 of 138 | 0 of 138 |
| a side not cut named | 0 | 0 |
| C1 on hot runs / on others | 24 of 24 / 0 | 24 of 24 / 0 |

The four misses all say *can't check*, never a wrong side: the three 500 Hz top 20 dB cuts over
the shallow floor, as at Gate 3, and one draw of the 500 Hz bottom 20 dB cut over the deep floor
(`-s2602`). Each is the mechanism §6.2 described: the called station's recording of a 500 Hz
session keeps 10 or 11 looks clear of frames, and the looks under its answer to the probe, whose
keying the sidecar does not list, count as weak. The new miss is the same draw on another course
(this grid ran a different daemon, and a harness run is not repeatable across builds). C1 named
all 24 hot runs, 20 of them within 10 s of the recording's start and four at 18–22 s, as the first
strong frames of those runs came later; no other run clipped.

**The cost (K9)**: `aetherd --passband-trace` over the field's six-minute session (011008, 362 s)
takes 0.18 s of CPU and over the quiet minute (163057, 97 s) 0.07 s, best of three: 0.05 and
0.08 % of real time on this machine, for the whole of it (reading the WAV, a transform twice a
second, which the old estimate also took on quiet audio, and the reading). The trace had computed
a reading on every 20 ms block (0.6–0.8 %); it now computes one only when it writes a line, as the
station does when it records a look or answers `metrics`, and prints the same lines byte for byte.
Ten times slower on the mini PC is still under K9's 2 %.
