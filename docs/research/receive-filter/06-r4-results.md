# R4 judged: the busy-band fix misses its criteria

*Written 2026-11-17, from `05-busy-band.md` §10, steps 1–6. The criteria are §7's, unchanged. R4
was built as §10 step 2 specifies and judged on them. **It misses Q1 and Q2 on both corpus draws,
K2 and K3 on the held-out draw, and H(a) on the harness**, so by the rules this stops before step
7: no ADR amends ADR-0105, the manual says nothing new about the banner, and R4 is not merged. (§10
named ADR-0130 as the next free number; it went the same evening to another change, a8f1385, so a
reading accepted later takes the next one.) Its patch is kept (`scratch/r4.patch`) for the author's
decision (§9). The instruments built on the way (steps 1, 3 and 4) change nothing the reader says,
and are merged.*

| Criterion (§7) | R3 | R4 | Verdict |
|---|---|---|---|
| Q1, qrm clips over uncut radios warned (of 84; draws 0 / 1) | 18 / 20 | 22 / 20 | **missed** (§3.1–3.5) |
| Q2, qrm cuts named right by the end (of 24) / moderate extra side (of 12) | 22, 22 / 0 | 23, 23 / 0 | **missed by one** (§3.6) |
| K1, cuts named (of 48, both draws) | 48, 48 | 48, 48 | met |
| K2, uncut clips warned (of 160) | 0, 0 | 0, 2 | **missed on draw 1** (§3.4) |
| K3, a side not cut named | 0, 0 | 0, 2 | **missed on draw 1** (§3.4) |
| K5, in time / cleared within 60 s / cut made warned | 24/24, 12/12, 12/12 | the same (clears in up to 55.5 s) | met |
| K6, C1 | 52 of 52 hot, 0 others | the same | met |
| K2 field, K4 | 3 of 16 warn; 5 of 5 | not judged | **owed** (§5) |
| H(a), runs with a warning on an uncut radio | the RTTY contest 3 of 3 (6 of 6 stations) | 2 of 24 (station b of the RTTY contest) | **missed** (§6.1) |
| H(b), the cut grid | 21 of 24 named, 0 of 138 warned | 22 of 24, 0 of 138, 96 of 96 runs, 0 overlaps | met (§6.2) |
| K7, constants and code | 11 | 12; removes the per-look ninth-decile division and the median over looks; 13 lines of code for 13 | met |
| K9, the trace's cost | 0.074–0.077 % of real time | 0.072 % | met on a harness recording; the mini PC's owed (§8) |

## 1. What was built

* **Step 1, the trace follows the scan's hops.** `aetherd --passband-trace` hands the reading each
  hop's deaf span from the sidecar (`Expectation::restarts`), as `station/scan.rs` does at a hop.
  `rxfilter_corpus.py`'s `preroll_keying` also adds the pre-roll's hops, from the log's
  `dial: moved to <Hz> Hz` lines. Tests: `no_look_is_kept_over_a_scans_move_of_the_radio`
  (`passband_trace.rs`) and `test_a_preroll_keeps_the_keying_and_the_scans_hops_its_log_holds`.
* **Step 2, R4**, exactly as §10 says: each look divided by its median over the span (the ninth
  decile kept for the floor test), each bin's 20th percentile over the looks (`TIME_QUANTILE`),
  `set_air` dividing again by the span's median. The two new tests failed on R3's rules first
  (R3 read *both* in each, the bottom 20 dB down) and pass on R4. The nineteen tests pass
  unchanged.
* **Step 3, L1.** A `filter` line in the log at each change of verdict:
  `filter: waiting -> clear | bottom 0.4 dB, top 4.9 dB, 12 looks` (a harness station's log).
  The station reads the verdict only when a look folds in, which is when it can change. Test:
  `the_log_says_when_the_receive_filter_changes_its_verdict`. It passes on R3 and on R4. The
  manual's Log section says what the line is (version 1.6.0).
* **Step 4, H1.** `session_matrix.py`'s `filter_warnings` reads, for each station whose radio
  cuts nothing, the daemon's `filter:` log lines and its recordings' live readings, into the
  result and the CSV's new `filter_warnings` column; `nightly_report.py` lists them every night
  as a warning, and on GitHub each is a `::warning`. §10 asked for the recordings only; the log
  was added when the recordings missed a warning given before them (§6.1). It stays a report: it
  becomes an invariant only once a fix is in (§10 step 4).
* **Step 5, S1: not built** (§7).
* **Ablations**: R4a, the median division alone (`TIME_QUANTILE = 0.5`); R4b, the 20th percentile
  alone (each look divided by its ninth decile, as R3).

The field set could not be judged: the upload script refused the read key this session had
(`fetch_uploads.py list`: "Reading back needs the read key"), so **K2 field and K4 are owed** (§5).

## 2. The corpus, both draws

`rxfilter_corpus.py make` (draw 0) and `make --seed 1` (draw 1), 250 clips each with the 72 qrm
clips, judged by each build. R3 reproduces `05-busy-band.md`'s figures on draw 0 exactly.

| Draw | Build | K1 (of 48) | K2 (of 160) | K3 | K5 in time / cleared / warned | K6 | Q1 (of 84) | Strong own (of 36) | Q2 (of 24) | Q2 moderate extra (of 12) | Strong extra |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | R3 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | **18** | 18 | **22** | 0 | 5 |
| 0 | R4 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | **22** | 21 | **23** | 0 | 6 |
| 0 | R4a | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | 21 | 21 | 22 | 0 | 5 |
| 0 | R4b | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | 25 | 21 | 22 | 0 | 6 |
| 1 | R3 | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | **20** | 18 | **22** | 0 | 5 |
| 1 | R4 | 48 | **2** | **2** | 24/24 · 12/12 · 12/12 | 52/52, 0 others | **20** | 22 | **23** | 0 | 6 |
| 1 | R4a | 48 | 0 | 0 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | 22 | 20 | 22 | 0 | 6 |
| 1 | R4b | 48 | 5 | 5 | 24/24 · 12/12 · 12/12 | 52/52, 0 others | 24 | 22 | 22 | 0 | 6 |

Q1 asks 0 of 84; Q2 asks 24 of 24 named right by the end and 0 moderate extra. "Strong own" and
"strong extra" are reported, not judged. K5's joined clips: a cut taken away clears 14.5–55.5 s
after the knob under R4 (R3 21.5–41.0 s); a cut made is named 14.0–60.0 s after it (R3 31.5–52.0
s). Both are inside K5's 60 s, as §6.1 expected.

**What R4 does well.** Once the station has listened 35 s, R4 leaves fewer judged clips warning
than R3: 6 of 84 on each draw, against 8 and 10. It clears the strong signals that come and go
over the FTDX10-like slope at 2300 Hz: the strong 500 Hz multicarrier there, on half the time,
warned *both* from 11 s to the end under R3 on both draws (to 22.7 dB), and under R4 until 26–27 s
and once at 50 s. It names one more strong always-on cut right by the end (Q2: 23 of
24, R3 22). It does not clear the moderate PACTOR pair over that slope, nor the 500 Hz
multicarrier there on its own rhythm on draw 1 (§3.3, §3.5).

## 3. The misses, with their traces

`scratch/q1_timing.py` traces each Q1 clip and says whether it warned only in the first 35 s of
listening or after it too. `scratch/warn_spans.py` gives each clip's warning spans.

| Q1's judged clips that warn | R3 d0 | R4 d0 | R4a d0 | R4b d0 | R3 d1 | R4 d1 | R4a d1 | R4b d1 |
|---|---|---|---|---|---|---|---|---|
| only in the first 35 s | 10 | 16 | 14 | 11 | 10 | 14 | 15 | 11 |
| after 35 s too | 8 | 6 | 7 | 14 | 10 | 6 | 7 | 13 |

### 3.1 Q1: the station's first half minute

Most of R4's Q1 count is the start of the clip. The *half* clips put the other station on from
the first second for 15 s (on 0–15, 30–45 and 60–75 s). The first reading comes once 12 looks are
kept, at 11.5 s, and every one of those looks holds the signal. A rule that tells a signal from
the filter by time cannot do it before the signal has stopped once. R4 then clears once a fifth
of the looks are without it, at about 23 s; R3 needs half, and clears at about 30 s. So R4's
warning at the start is shorter than R3's, but there is one, and Q1 counts any:

```
qrm-ofdm500-1300-strong-half-ssb2700-deep@2300  R3: 11-30 s both 17.2 dB, 44-53 s top 13.0 | R4: 11-23 s both 18.9 dB
qrm-ofdm2400-2900-strong-half-ssb2700-deep@2300 R3: 11-27 s bottom 17.6 dB                 | R4: 11-23 s bottom 17.6 dB
qrm-rtty-contest-strong-half-ssb2700-deep@500   R3: none                                   | R4: 11-14 s top 14.3, 18-20 s top 15.1 dB
```

The last one is R4's own. With 12 looks, the 20th percentile is the third lowest, and an edge
read from it is noisier than from the median (§3.4).

Q1 as registered cannot be met by R4, or by any rule that reads time, on clips whose signal is on
from the first look. That is the author's to judge (§9).

### 3.2 Q1: a signal over more than half of the 500 Hz span

The 500 Hz multicarrier at 1300 Hz covers 1050–1550 Hz: 60 % of the 500 Hz air's span
(1260–1740 Hz). In a look where it is on, the span's median is the signal's level, not the
noise's, so the noise above 1550 Hz reads as far under it as the signal stands over the noise.
The 20th percentile then picks those looks for those bins, because they are the low ones. Over a
radio whose 1740 Hz edge is 6.8 dB down, R4 reads the top 14.3 dB down for the whole clip:

```
qrm-ofdm500-1300-moderate-own-shift-down-1100-deep@500  R3: 11-90 s top 17.1 | R4: 11-35 s top 17.6, 46-90 s top 14.3 dB
qrm-ofdm500-1300-strong-half-tilt-deep@500              R3: until 56.0 / 53.5 s | R4: until 53.0 / 57.0 s (draw 0 / 1), top
```

Dividing by the median works only while the signals cover less than half the span. On the
2300 Hz span the field's 300–900 Hz signals do; on the 500 Hz span a 500 Hz signal does not.

### 3.3 Q1: a signal on more than four looks in five

The moderate PACTOR pair at 650–950 Hz stands 6 dB or more over a look's span median in 94 % of
the looks of draw 0 and 98 % of draw 1 (`scratch/duty.py`); the moderate 500 Hz multicarrier over
the same radio in 84 and 87 %. Their bins' 20th percentile is still their level, and each covers
more than the tenth of the span the reading's ninth decile needs, so it sets the reading's level.
Over the FTDX10-like slope the top then reads the slope plus the signal's height:

```
qrm-pactor-800-moderate-own-tilt-deep@2300   R3: 27-90 s top 13.7 dB | R4: 28-90 s top 13.9 dB (draw 0; draw 1 alike)
qrm-ofdm500-1300-moderate-own-tilt-deep@2300 R3: 16-90 s top 13.3 dB | R4: 21-85 s top 13.7 dB (draw 1; draw 0 ends clear at 11.4)
```

This is §6.1's accepted cost ("a signal on more than about 80 % of the window still reads as a
cut"), but the clips are *moderate* ones, which Q1 judges.

### 3.4 K2 and K3 on draw 1: the 20th percentile over few looks

Draw 1's `ssb2100` radio has its bottom edge 8.1 dB down at 2300 Hz, inside the free band where
no warning is allowed. R4 warns *bottom* at its first readings and then clears:

```
ssb2100-steady-deep@2300  R3: none | R4: 12-14 s bottom 13.1 dB | R4a: none | R4b: 12-14 s bottom 13.6 dB
ssb2100-ft8-deep@2300     R3: none | R4: 11-17 s bottom 12.1-12.2 dB | R4a: none | R4b: 24 s bottom 12.1 dB
```

With 12–20 looks the 20th percentile of each bin is a few low values, so each edge's smoothed
value scatters more than the median's, and the reading's ninth decile, taken over the span's
noisiest bins, is a little higher. An edge 8 dB down then reads 12–13. §6.1 named this cost and
counted on the ±50 Hz smoothing; on this draw the smoothing was not enough. R4b, the 20th
percentile alone, warns on five such clips of draw 1, and on one (`tilt-ft8-deep` at 500 Hz) for
the whole clip.

### 3.5 Why the 20th percentile buys less than §6.1 expected

§6.1 reasoned that a signal on in up to four looks of five no longer sets a bin's 20th
percentile. That holds if a bin's value in a look is steady. It is not: a look is one 85 ms
periodogram, so each bin's power is drawn afresh in every look (an exponential, about ±5.6 dB in
decibels). A bin a signal holds in a share *p* of the looks has its 20th percentile over all the
looks at the 0.2/(1 − *p*) quantile of its noise alone, while a free bin reads its 0.2 quantile.
The signal's bins therefore stand over their neighbours by the gap between those two quantiles:

| Signal on, share of looks | R4 (20th percentile) | R3, R4a (median) |
|---|---|---|
| 33 % | 2.0 dB | 3.0 dB |
| 50 % | 3.6 dB | the signal's level |
| 66 % | 6.0 dB | the signal's level |
| 77 % | 9.6 dB | the signal's level |

`scratch/offset_check.py` checks it with R4's own steps (span-median division, the time quantile,
the ±50 Hz smoothing, the reading's ninth decile) on looks of a sloped radio, 4.5 dB/kHz above
1 kHz, with a 500 Hz block 8 dB over the noise: with the block on in two looks of three, its bins
stand +6.4 dB over the flat part under R4 and +9.1 dB under the median, and the top edge reads
13.9 and 16.4 dB down, against 7.9 and 7.5 without the block. The field's signals were on in
33–77 % of the looks, so R4 lowers their bump by a few decibels, not to nothing. Where a bump
covers a tenth of the span, the reading's ninth decile is the bump, and an edge reads that much
deeper: on the FTDX10's 2–4 dB edges, a field signal on in two looks of three would read 8–10 dB
under R4, close to the line. The same scatter is §3.4's start-up noise.

### 3.6 Q2: one strong, always-on signal

`qrm-ofdm500-1300-strong-own-lowcut600-deep@2300` (a 600 Hz low cut under a strong multicarrier
on its own rhythm, with a fast AGC) ends *both* under R4 (44.4/12.6 dB on draw 0, 41.8/13.4 on
draw 1): the bottom is named, and the signal adds the top. Under R3 it and one more such clip
missed the same way. §7 reports strong clips' extra sides but asks every cut named right by the
end, so Q2 misses by one on each draw. Counted as "its cut side named at the end", all 24 pass
under every build.

## 4. The ablations

The rule was that a part whose removal fails no criterion goes. Every build fails Q1, so the
rule cannot be applied as written. What they show:

* **The median division (R4a) does most of the work after warm-up**: 7 judged clips still warning
  after 35 s on each draw, against R3's 8 and 10. It keeps K1–K6 on both draws.
* **The 20th percentile alone (R4b) is worse than R3**: 14 and 13 judged clips warning after
  35 s, and K2/K3 missed on draw 1 (5 clips, one for the whole clip). Without the median division,
  a look's other bins sit low whenever a signal sets its level, and a low quantile picks exactly
  those looks.
* **Both together (R4)** clear one clip more than R4a after warm-up, and bring R4b's start-up
  noise with them (K2/K3 on draw 1).

## 5. The field: owed

`fetch_uploads.py list` was refused: "Reading back needs the read key." The script did not
accept the key in this session's environment. So K2 field (none of the thirty working
recordings warns; R3: 3 of 16 of 2026-11-17) and K4 (Saturday, 5 of 5) are not judged, and step
1's check, that the trace prints the same lines over every field recording without a hop and
differs only around hops, was not run on the field.

## 6. The harness

### 6.1 H(a): the scenarios with other stations

§7 H(a)'s eight scenarios, three draws each (`--seeds 3`), on the R4 daemon. `session_matrix.py`
keeps every run's audio unless told `--drop-passing-audio`; §10's `--keep-passing` does not exist.
All 24 runs pass, with no radiated overlap.

**H1 as §10 specified it missed the warnings.** It read the stations' recordings, and in every
recording R4's live readings were clear. But L1's log lines show that station b of
`40m-rtty-contest-2300` warned in two of three draws *before* its recording began, while it
idled in its first seconds of listening:

```
40m-rtty-contest-2300        b  23:00:51  filter: waiting -> both | bottom 12.1 dB, top 14.5 dB, 12 looks (Idle)
                                23:00:55  filter: both -> top    | bottom 10.7 dB, top 12.8 dB, 13 looks
                                23:00:55  filter: top -> both    | bottom 15.2 dB, top 18.4 dB, 14 looks
40m-rtty-contest-2300-s2019  b  23:02:25  filter: clear -> bottom | bottom 12.4 dB, top 10.8 dB, 34 looks (Idle)
```

H1 now reads the daemon's `filter:` lines as well as the recordings, so the nightly sees a
warning given while a station idles. Read that way, **H(a) misses under R4: 2 of 24 runs warn**,
both at station b of the RTTY contest, at 12–36 looks: §3.1's first half minute, on the
harness.

For R3's figure on the same instruments, `40m-rtty-contest-2300`'s three draws ran again on R3's
reader (the tree's daemon, with L1). **All three warn, at both stations** (the other scenarios do
not warn under either reader traced offline, below), and not only at the start: station b of the
base draw warned *bottom* again at 120 looks in its session, and its log has 17 changes of
verdict as the reading crossed the line and back; the s2019 draw's has 24. Under R4 the two
warnings came only in the first 6–18 s of listening, and the readings stayed clear once the
station had listened half a minute.

| `40m-rtty-contest-2300`, live | R3 | R4 |
|---|---|---|
| runs with a warning (of 3) | 3 | 2 |
| stations with a warning (of 6) | 6 | 2 |
| warnings after 35 s of listening | at 4 stations, to 120 looks | none |

Traced offline over each recording (`scratch/harness_traces.py`, the trace starting cold at the
recording's start), R3 warns at all six stations of the RTTY contest's three draws (21–146
readings, up to 18.2 dB) and R4 at the three b stations (5–25 readings, up to 18.2 dB). No other
scenario warns under either. The offline trace starts each recording without the looks the live
station had already kept, so it shows the warm-up again at every recording.

On the 60 recordings, step 1's trace (R3's reader with the hops) prints the same lines, byte
for byte, as the trace before it: none of them holds a hop.

### 6.2 H(b): the cut grid

`tools/rxfilter_cost.py OUT --keep-audio` on the R4 daemon: the 32 configurations × 3 draws, 96
runs (the session's worker restarted once, and the 21 runs it cut short were run again on their
own). Then `rxfilter_corpus.py harness` over both stations' recordings, with R3's reader (the
tree's daemon) and R4's, on the same audio:

| | Gate 5 (R3, its own grid) | R3, this grid | R4, this grid |
|---|---|---|---|
| runs passing, radiated overlaps | 96 of 96, 0 | (the same runs) | 96 of 96, 0 |
| cuts of 20 dB or more named right (of 24) | 20 | 21 | 22 |
| uncut or ≤ 10 dB recordings warned (of 138) | 0 | 0 | 0 |
| a side not cut named | 0 | 0 | 0 |
| C1 on hot runs (within 10 s) / on others | 24 (20) of 24 / 0 | 24 (20) of 24 / 0 | 24 (20) of 24 / 0 |
| live warnings on an uncut radio (H1) | — | — | 0 of 96 runs |

**H(b) holds for R4** on its counts. Its two misses are the 500 Hz top cut of 20 dB over the
shallow floor, which ends *clear* at 11.1 and 11.7 dB, just under the line; R3's three misses are
the same cut at 10.0–11.8 dB. At Gate 5 they said *can't check*. ADR-0105 §6 names this case: the
whole 500 Hz span sits on the filter's skirt, and such a cut reads about 12 dB. C1, which R4 leaves alone,
named every hot run, four of them after 10 s (18–33 s), as at Gate 5.

## 7. S1

§10 step 5 builds S1 only if the looks a hop leaks change a verdict. The field set, where the
gateway's 19:56Z pre-roll held 7 such looks, could not be read (§5), so the count there is owed.
On the harness, the two scanning scenarios ran three draws each on the R4 daemon
(`40m-scanning-gateway-2300`, the caller's tries landing on a scan's hop; and
`40m-scanning-operator-wins-2300`, 12 hops a run). All six pass with no radiated overlap. The
scanning station's reading never warned, in its log or its recording: each said
`waiting -> clear` once, and its drops stayed at 4.4 dB or less, nearly 8 dB under the line. A
handful of looks of a call's try cannot move a reading that far, under a median or a 20th
percentile. **S1 is not built.** None of the harness's hops fell inside a recording, so step 1's
"differs only around hops" was not seen there either; its test does it
(`no_look_is_kept_over_a_scans_move_of_the_radio`).

## 8. The cost (K9)

K9 asks the trace's cost on the mini PC's longest field recording, best of three, within 2 % of
real time. The field recordings could not be read (§5), so it was taken on the longest harness
recording of §6 (`40m-rtty-contest-2300-s2019`, station a, 390 s), on this session's machine, as
CPU time: R3 0.074–0.077 % of real time, R4 0.072 %. R4 adds a median a look and changes a
quantile's place, which costs nothing measurable. Ten times slower on the mini PC would still be
under 1 %. The mini PC's own recording is owed with the field.

## 9. For the author

R4 does what §6.1 meant it to do once the station has listened half a minute: fewer of the
judged clips warn after that, and on the harness its only warnings came in a station's first
18 s of listening, where R3 warned at every station of the RTTY contest and in session too (§6).
It misses the criteria in four ways, each a decision:

1. **Q1 and H(a) count the first half minute.** No reading that tells a signal from the filter by
   time can clear a signal that has been on since the station started listening. Keep Q1 and H(a)
   as they are (and no time rule can pass them), or judge them after a warm-up, or have the
   reading wait longer before its first verdict (12 looks is 11.5 s)?
2. **The 500 Hz span.** A 500 Hz signal covers more than half the 500 Hz span, so the median
   division does not reach it there (§3.2), under R3 or R4.
3. **A moderate signal on more than four looks in five** (§3.3) still warns over a sloped radio.
   §6.1 accepted this for strong signals; the corpus's moderate PACTOR pair and multicarrier over
   the slope are on in 84–98 % of the looks.
4. **The 20th percentile's start** (§3.4) costs two false warnings on draw 1's plain corpus, where
   R3 had none.

Behind 4, and behind R4's smaller gain than §6.1 expected on signals that come and go, is one
fact §6.1 did not count on (§3.5): a look's bin is a single periodogram, about as noisy as a
moderate signal is high, so a quantile over the looks only partly tells the looks a signal was on
from those it was off. The ablations agree: the median division carries most of R4's gain after
warm-up, and the 20th percentile carries its start-up misses. Which way to go from here is the author's to weigh, not
this document's: no variant was registered beyond R4, R4a and R4b, and none passes.

L1 and H1 are merged, so the next field report can place a warning and the nightly lists them.
The field criteria (K2 field, K4) are owed whatever is decided.

## 10. Method and scripts

The scratch scripts behind the numbers are in `scratch/` (its README):
* `q_counts.py` splits a judge CSV into Q1 and Q2;
* `criteria_table.py` makes §2's table;
* `q1_timing.py` splits Q1 into warm-up and after;
* `warn_spans.py` gives a clip's warning spans for several builds;
* `harness_traces.py` traces every uncut station of a harness run with several builds;
* `offset_check.py` is §3.5's synthetic check;
* `rejudge_filter.py` reads H1's `filter_warnings` again over a harness out dir.

`r4.patch` is R4 against the tree (`git apply`). R4a is it with `TIME_QUANTILE = 0.5`; R4b is it
with both divisions by `REFERENCE_QUANTILE`. The corpus draws are not kept: `rxfilter_corpus.py
make OUT` and `make OUT --seed 1` make them again, byte for byte.
