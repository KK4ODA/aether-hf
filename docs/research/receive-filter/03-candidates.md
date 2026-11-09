# Gate 2: candidates and criteria, registered before the runs

*Written 2026-11-09, after ADR-0104 shipped in beta.144. Nothing below has been run. The criteria
in §3 are fixed before any candidate is built, as the brief's method asks; a candidate that misses
one narrowly goes to the author with its trace, case by case, and does not pass by rule.*

## 1. What the warning has to do, measured on the receiver that ships

The job (`00-brief.md` §2): tell the operator, while it matters and in words they can act on,
when the radio's receive audio does not carry the whole of the modem's signal; say which side is
cut and by about how much; never warn about a station that works.

**Where the line goes.** The author's answer (brief §6.2) was to measure what a cut costs and warn
where the cost starts to show. Gate 0 measured it on the receiver of beta.143 (`01-evidence.md`
§8); ADR-0104 changed the receiver, so it was measured again on beta.144's (`tools/rxfilter_cost.py`,
three draws, each run's Test rate against the uncut run on the same draw, median of three):

| Cut at the signal's edge | 2300 Hz message | file | 500 Hz message | file |
|---|---|---|---|---|
| none | 1.00 | 1.00 | 1.00 | 1.00 |
| top 3, 6, 10 dB | 1.00, 1.00, 1.00 | 0.97, 1.00, 1.01 | 1.00, 1.00, 0.96 | 1.00, 1.00, 0.99 |
| top 20 dB (floor 15 / 50 dB down) | 0.70 / 1.00 | 0.86 / 0.96 | 0.87 / 0.93 | 0.98 / 0.99 |
| bottom 3, 6, 10 dB | 0.70, 0.86, 0.86 | 0.96, 0.93, 1.02 | 1.00, 0.96, 1.02 | 1.00, 0.99, 0.99 |
| bottom 20 dB (floor 15 / 50 dB down) | 0.60 / 0.57 | 0.83 / 0.86 | 0.16 / 0.28 | 0.09 / 0.12 |
| Saturday's receiver | 0.67 | 0.78 | 1.00 | 1.00 |

The measure moves in steps of about 14 % at 2300 Hz (one rung more or less on one draw), so a
0.86 alone is a step, and the 0.70 of the bottom 3 dB cut, which ADR-0104's bench shows costs no
frame, is two. Read that way: **10 dB down at an edge still costs nothing**; **20 dB costs** at
either edge on at least one bandwidth, the bottom one most (at 500 Hz it is the tone detector's,
ADR-0104 §3). The criteria therefore ask for a warning at 20 dB and silence at 10 dB and leave the
band between free: a candidate places its line there and says why.

A line in that band has one more constraint, the audio chain's own floor. The mobile's floor sits
15 dB or less under the band's noise; a 20 dB cut over it reads about 14 dB down (the cut noise and
the floor add), and a 10 dB cut about 9. A line near 12 dB separates both floors; a line at 15 dB
would miss a 20 dB cut over a shallow floor.

## 2. The candidates

The estimate has four parts, and the candidates change them one at a time, so each change's worth
shows on its own. R0 is today's, kept as the comparison.

### 2.1 The reading

| | Looks | Profile | Judgement |
|---|---|---|---|
| **R0** (today) | a spectrum every 0.5 s, only after a window of quiet the busy detector calls clear | each bin's 10th percentile over the last 120 looks, 12 needed | the edge 6 dB under the level at 1500 Hz or halfway to the floor, whichever is lower; warn when the width is under the signal's width less 400 Hz |
| **R1** (judge the span) | as R0 | as R0 | the response at each edge of the signal's span (the mean over ±50 Hz at 360 and 2640 Hz, or 1260 and 1740 Hz), against the profile's level over the span; warn past the line, naming the side |
| **R2** (looks that survive level swings) | as R0, each look divided by its own level over the span; a look kept only when that level stands at least the line plus 3 dB over the chain's floor read in the same look between 6 and 10 kHz, where no radio passes audio | each bin's median over the kept looks | as R1, and "cannot tell" when no look is kept |
| **R3** (busy bands) | as R2, but taken in any block not keyed or deaf: within each look the bins a signal occupies are removed first (FCME: drop the bins over a multiple of the look's own noise, read again from what is left, repeat until nothing more drops) | as R2, each bin's median over the looks where it was kept | as R2 |

Why each:

* **R1** answers Gate 0's main finding: the estimate sees the cut and judges it wrongly. It
  measures the span, not the width, so a moved passband and a cut low edge are caught. It
  removes the edge rule, the 150 Hz hold, the reference at 1500 Hz and the panel's 400 Hz.
* **R2** answers the AGC over a shallow floor (`01-evidence.md` §6: under AGC dips the band's
  noise sinks under the chain's and no filter shows) and the two Saturday sessions whose looks
  swung 28–32 dB. Normalising each look by its own level and keeping only looks with band noise
  well over the floor is what the survey's noise-estimation methods do (`02-survey.md` §1).
* **R3** answers the FT8 band: today's looks wait for whole quiet windows, which take a minute
  or two to gather on 7.074 MHz. Removing occupied bins within each look reads a busy band in
  seconds. It costs a few iterations over 172 bins a look.

### 2.2 The second witness

**W1**: in a session, the other station's decoded frames. Each OFDM frame's channel estimate per
carrier and each tone frame's energy at the tones it sent, each frame against its own median, the
median over the session's frames (`tools/passband_witness.py`, `02-survey.md` §5). It covers the
whole signal with OFDM frames, 700–2300 Hz with the fast tone kinds, 1300–1700 Hz with the others.
It measures the whole chain: the other station's transmit filter and the path as well as this
radio's receive filter. How it is used is a question for the author (§5, question 1).

### 2.3 Clipping

**C1**: clipping as its own reading. The share of samples at full scale over the last 10 s;
over 0.1 % it is a warning (the threshold the Receive level tile uses today, which Saturday's
1.3–1.7 % passed in every session). While it holds, the passband reading takes no looks: a clipped
spectrum is not the radio's response.

### 2.4 Where it shows

A banner in every view, Compact included, as the session and mismatch banners are (the author's
answer, `02-survey.md` §7). The passband banner names the side and how far down, and the knobs that
cause it: the top cut — the IF shift, the high cut, or the filter width; the bottom cut — the IF
shift, the low cut, or the filter width; both — the filter width, or run the modem at 500 Hz. The
clipping banner says to turn the radio's audio out or the sound card's input down. Each can be
dismissed, and comes back when its reading changes (the side, or 3 dB more) or the bandwidth does.

## 3. Criteria

Each is judged on the bench of §4. "A cut" is the loss at the edge of the signal's span in the
air the station runs.

* **K1, detection.** A warning naming the right side for every corpus clip whose loss at an edge is
  20 dB or more, at either bandwidth, over either floor, steady, under QSB, on the FT8 band and
  under the AGC: at least 46 of those 48, and every miss explained. Today: 8 of 48. The twelve
  such clips driven hot are C1's (K6): while the audio clips, no passband reading is taken.
* **K2, silence.** No passband warning for any corpus clip losing 10 dB or less at both edges
  (the notch, the tilt and the working filters included), for any of the field recordings of a
  working station (the gateway's nine; the mobile's four of 2026-11-06/07 and its quiet minute of
  2026-11-08 afternoon), or in any harness run uncut or cut 10 dB or less. None. Today: two
  false warnings on the corpus.
* **K3, the words.** Every warning names the side or sides actually cut. A narrow filter is "both
  sides", a moved passband is one side. Every warning, no exceptions.
* **K4, Saturday.** The mobile's three 2300 Hz sessions with the IF shift left
  (20261108-005459, -010450, -011008) each warn that the top is cut, before the session's end.
  Its two 500 Hz sessions that night lose about 8 dB at 1740 Hz (the frames' witness): free, but
  their reading must place that edge within 3 dB of the witness.
* **K5, time.** On a steady band, a warning within 15 s of the receiver starting to read; on the
  corpus's FT8 band, within 60 s. After the cut is removed (a corpus clip joined to its uncut
  twin), the warning clears within 60 s. Today needs 6 s of quiet to read at all, and up to two
  minutes on an FT8 band.
* **K6, clipping.** C1's warning on every one of Saturday's five sessions within 10 s of the
  recording's start; on none of the other fourteen recordings; on every corpus clip driven hot and
  none of the others; shown in Compact.
* **K7, simplicity.** Fewer constants than today's twelve (eight in `spectrum.rs`, three in
  `station.rs`, the panel's 400 Hz), each with a measured reason in its comment, and no more
  code than today's 200 lines for the reading. Every test today's has (the width, flat noise, the
  notch, the FT8 band, the wait for looks, the looks kept from frames and keying) passes in its
  new form, and new ones hold a moved passband and a cut low edge.
* **K8, the witness earns its place.** If W1 is kept, its edge readings on the field recordings
  agree with the noise's within 3 dB wherever both read, and on the harness's cut runs it warns,
  in sessions, where the noise has not yet; if it does neither, it goes.
* **K9, nothing else changes.** It only warns: on the harness, the prototype's runs pass as the
  shipped build's do, with no radiated overlap; and its cost on `aetherd --replay` is no more than
  2 % of real time over today's on the mini PC's recordings.

## 4. The bench

* **The corpus** (`tools/rxfilter_corpus.py`, `01-evidence.md` §6): the 130 clips as they are, plus
  two things the criteria need. Joined pairs for K5 (a cut clip followed by its uncut twin, and the
  reverse); and cuts of 12 and 15 dB at each edge, so the line each candidate places can be seen,
  though they judge nothing. The labels already give each clip's loss at both bandwidths' edges.
* **The field** (`01-evidence.md` §2): the nineteen recordings in `field/inbox/`, through
  `aetherd --passband-trace WAV --expect SIDECAR` with each candidate as a `--passband-variant`, as
  `--busy-variant` serves the busy detector's review. The FTDX10's own slope (5–9 dB down at 2.6–2.8
  kHz) is in the gateway's nine and must stay silent.
* **The harness** (`tools/rxfilter_cost.py`): the cut grid's runs on a daemon built with the
  candidate, reading the warning from each station's recording and log, three draws.
* **The witness** (`tools/passband_witness.py`): its readings on the field recordings, beside the
  noise's.

## 5. For the author

1. **What the frames may say.** The other station's frames measure its transmit filter and the
   path as well as this radio's receive filter. Should a cut seen in the frames (a) only make a
   warning the noise agrees with come sooner, (b) warn on its own, in words that say the other
   station's signal arrives cut and it may be either radio, or (c) be left out?
2. **When the cut cannot be measured.** On a quiet band with low audio, or under a pumping AGC, the
   band's noise may sit too close to the chain's floor for any cut to show. Should the panel say
   so (a line in the Status tab's readings, not a banner), or say nothing?

## 6. The order of work in Gate 3

1. The corpus additions and `--passband-variant` in the trace, with R0 reproducing today's
   readings exactly.
2. R1, R2 and R3 in turn, each judged on the corpus and the field (K1–K5, K7).
3. C1 and its banner (K6), and W1 as the author decides (K8).
4. The harness with the best candidate (K2, K9), then the results (`04-results.md`) for the
   author's decision at Gate 4.
