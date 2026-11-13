# Gate 3: results

*2026-11-13. The candidate the author chose to screen (`03-candidates.md` §5): G3 + G4 + G5
together, "the try". Judged against the criteria registered in `03-candidates.md` §3 before the
runs.*

## 1. The runs

Oracle runs 24 (today's controller) and 25 (the try, `bench_link =
prior_steps_aside,clean_try_keeps_wait,try_unaged`), both at `c61bb64` on GitHub's runners,
controller only, the 26 scenarios of the workflow (K2's 24 and the start pair), ten draws each,
the same seeds. All 520 of run 25's daemons logged the switches on; none of run 24's did. Per
draw in `data/gate3_try.csv`. About four hours of runner time together.

## 2. Verdict: the try fails

| # | criterion | result | |
|---|---|---|---|
| K1 | the gap to the best single rung on both strong 2300 Hz paths, +15 % or less | 40 m Good: today 2 246 bit/s, the try 2 207, rung 17 2 695: **+22 %**. Steady mail: today 963, the try 1 006, rung 12 1 176: **+17 %** | **fails** |
| K2 | no speed scenario slower (90 % range end under 1.10) | unpaired, 12 of the 24 have a range end at or over 1.10 (the ranges are as wide as the draws' spread); paired by seed, four: the weak ladder path 1.11 (0.97–1.24), the AGC echo test 1.03 (0.86–1.15), the slow machine 0.98 (0.87–1.13), Trimode 1.01 (0.94–1.11) | not run on fresh draws (K1 already fails) |
| K3 | the start pair: no more draws over one rung apart than today | today 1 draw of 10 (7 against 10); the try 3 (7–10, 10–12, 9–12) | **fails** |
| K8 | both echo tests inside their budgets in every run | the AGC test lost its message in one draw of ten on **both** builds (today draw 1, the try draw 4); Trimode inside 400 s in all | fails, as today does |

K4–K7 were not reached: they are judged on a candidate that passes.

## 3. Why

**Where it acts.** Per draw, the two builds sent their data at exactly the same rungs on AWGN
(10 of 10 draws), Trimode (10), 40 m Good (6) and most of the fading 2300 Hz paths (paired ranges
1.00–1.00). Their differences there are the harness's own: AWGN's 3 kB transfer took 10 s or 11 s
on either build, whatever the rungs. The try acts where a try of the rung above ends with all its
frames decoded and still loses, and that is rare.

**The steady path: +4.5 %, every draw.** On `80m-steady-mail-2300` the try took rung 12 where
today's controller lost the near miss: 963 → 1 006 bit/s in all ten draws, the change the
2026-11-12 sessions asked for. It is a fifth of the gap: rung 12 held throughout gives 1 176.

**40 m Good: nothing.** The gap there is where C1 starts (rung 16, the best single rung 17) and how
fast it climbs in a 16 kB message: Gate 0 §5 item 5 said a candidate acting on the near miss alone
would not reach it in that time, and it did not (2 246 → 2 207, the same rungs in six draws).

**The weak 500 Hz ladder path: slower in most draws.** `80m-weak-both-ways-ladder-500` (ND1J's
second morning; both stations weak) ran its file at 55.6, 60.5 and 53.9 bit/s in draws 0, 3 and 8
against today's 78.4, 79.1 and 69.5. The traces (the caller's data rungs, run-length):

| draw | today | the try |
|---|---|---|
| 0 | 4 ×14, 3 ×6 | 4 ×8, 3 ×5, **5 ×6**, 3, 4 ×3, **5 ×5**, 1 ×5, 4 ×5 |
| 8 | 4 ×10, 3 ×13 | 4 ×5, 3 ×4, 4 ×6, 3 ×2, **5 ×12**, 1, 4 ×2 |
| 3 | 3 ×7, 2, 3 ×3, 4, … | 3 ×7, **2 ×10**, 3 ×3, 4, 3 |

A clean try of the first OFDM rung (5) from the floor's top, three frames on a path that carries
it one time in several, was taken once the prior stepped aside (G3), and the rung's failures then
cost a step down to rung 1. This is the loss `03-candidates.md` §2 named for G3 ("a try of three
frames that decode by luck on a fading path"), and the first review's reason for a three-frame
try: on a weak path three frames are not enough to trust. In draw 3 the try sat on rung 2 for ten
frames where today's went back to 3 after one; one draw, not read further.

**The start pair.** With the prior out of the way the high-reading session settled at rung 12 in
three draws, where today's never went past 10, and the pair parted by more than one rung in three
draws instead of one.

## 4. What it says

* The near miss of 2026-11-12 is real and the try fixes it, but it is worth about 4.5 % on a long
  steady path and nothing on a short strong one.
* The speed left on the strong 2300 Hz paths is where C1 starts and how fast it climbs, which the
  try does not touch (Gate 0 §5 item 5).
* Letting a three-frame clean try decide costs the weak 500 Hz paths, where the floor's edge to
  OFDM is a real cliff. A G3 limited to the current rung's family (no step across the floor's
  edge on a try alone) would avoid that trace, but would leave K1 failing: its gain is the steady
  path's 4.5 %.

No runs beyond these two. The next step is the author's (§5).

## 5. The author's decision, and G1 (2026-11-13)

The author chose to screen **G1 at 1 dB** next (`prior_margin_db` 1 against today's 3), the lever
for where C1 starts. The port takes it as `RateReview.prior_margin_db`, a simulated daemon from
`AETHER_BENCH_LINK=prior_margin_db=1`; the granted vectors gain two sessions with it
(`margin-good-wide`, `margin-moderate-narrow`), and a model test holds that it starts higher at the
same reading. Its run is judged against run 24, today's, on the same draws and criteria.

### 5.1 G1's run

Oracle run 26 at `bc6a003` (`bench_link = prior_margin_db=1`), controller only, the 26 scenarios,
the same ten draws; all 520 daemons logged `prior_margin_db: Some(1.0)`. Judged against run 24.
Per draw in `data/gate3_g1.csv`. About two hours of runner time.

| # | criterion | result | |
|---|---|---|---|
| K1 | the gap to the best single rung, +15 % or less | 40 m Good: today 2 246, G1 2 535, rung 17 2 695: **+6.3 %**. Steady mail: today 963, G1 1 104, rung 12 1 176: **+6.5 %** | **passes** |
| K2 | no speed scenario slower | paired, nine scenarios have a range end at or over 1.10 (below); faster: equatorial flutter 0.54, 80 m flutter at 500 Hz 0.77, VARA QRM 0.85, steady mail 0.87, thunderstorm 0.90, 40 m Good 0.93 | to be rerun, but see K8 |
| K3 | the start pair | 0 draws over one rung apart (today 1) | **passes** |
| K8 | both echo tests inside their budgets | the Trimode test lost its reply in **3 draws of 10** (today none); the AGC test lost one, as today did | **fails** |

The nine for K2's rerun (paired ratio, 90 % range): Trimode 1.74 (1.36–∞), the weak ladder path
1.06 (1.00–1.29), 80 m weak 1.00 (1.00–1.26), the weak 2300 Hz Test path 1.01 (1.00–1.21), 20 m DX
1.03 (0.88–1.20), RTTY crashes 1.06 (0.91–1.18), the AGC test 1.05 (0.95–1.18), NVIS evening 0.92
(0.82–1.15), NVIS disturbed 1.07 (0.94–1.14). All but two are 500 Hz or weak paths.

**Why Trimode fails.** The test is ITU NVIS at 500 Hz, where a 7 ms echo kills OFDM. In the lost
draws (1, 3, 8) the client went out at OFDM rungs 6 and 7 and stayed on OFDM (draw 1: 7 ×4, 6 ×2,
7 ×2, 6 ×2, 7, 6 ×2, 7, 6, then 5 ×8), delivering nothing the gateway could read, until the
transfer ran out of its 400 s. Today's controller starts at rung 6 too, but with the prior 3 dB under
the reading it gives OFDM up after a few failures and finishes on the floor's rungs 0–2. At 1 dB the
prior still rates the next OFDM rung down above the floor after each failure, and the family offset
(0.5 dB a failed frame) needs more failures to undo the extra 2 dB. This is the loss
`03-candidates.md` §2 named for G1: a higher start on an NVIS path that the offset then has to walk
back.

**What it says.** The margin is the lever for the strong 2300 Hz paths, as Gate 0 said: K1 passes
with room, and several fading 2300 Hz paths gain 7–46 %. What fails is the same 2 dB on the 500 Hz
NVIS and weak paths, where today's caution is what gets the session off OFDM. Options that keep the
first and not the second need a run each (§6 of the conversation's decision, recorded below when
made).

### 5.2 G1 at 1 dB with G2b

The author's next choice: the 1 dB margin with the family offset's gain doubled (`family_gain_db`
1.0), so that a path that fails a family's frames walks the guess off it twice as fast. Oracle run
27 at `12308ea` (`bench_link = prior_margin_db=1,family_gain_db=1`), the same 26 scenarios and ten
draws; all 520 daemons logged both values. Per draw in `data/gate3_g1g2.csv`. Before it, a local
check on Trimode draws 1 and 3 (both lost by run 26): 1 dB alone lost draw 3 again, with G2b both
delivered. Two draws, a hint only, and the runners then lost draw 1.

| # | criterion | G1 alone (run 26) | G1 + G2b (run 27) |
|---|---|---|---|
| K1 | gap to the best single rung | +6.3 % / +6.5 %, **passes** | the same: +6.3 % / +6.5 %, **passes** |
| K3 | start pair over one rung apart | 0, **passes** | 0, **passes** |
| K8 | Trimode delivered | 7 of 10, **fails** | 8 of 10, **fails** (today 10) |
| K8 | AGC test delivered | 9 of 10 (as today) | 8 of 10 (today 9) |
| K2 | paired range end at or over 1.10 | nine scenarios | seven: Trimode 1.28 (1.02–∞), AGC 1.07 (0.87–∞), 80 m weak 1.00 (1.00–1.26), 20 m DX 1.02 (0.93–1.17), NVIS evening 0.94 (0.81–1.15), the weak ladder path 1.06 (1.00–1.14), NVIS disturbed 1.01 (0.94–1.13) |

G2b moved the 2300 Hz paths little (the strong ones not at all) and the weak ones the right way:
RTTY crashes 1.06 → 0.97, the weak 2300 Hz Test path's range end 1.21 → 1.02, the weak ladder
path's 1.29 → 1.14, Trimode 1.74 → 1.28. The VARA QRM path went 0.85 → 0.81, 80 m flutter at
500 Hz 0.77 → 0.88.

**Why NVIS still fails.** The traces of the lost draws are the same shape as run 26's, and worse
at the start: the client opens at OFDM rungs 6–9 (today 5–7) and keeps going back to OFDM after
dropping to the floor. Trimode draw 5: 6, 5, 6, 5, 6, 5, then 1, 2; and in the session after it
9, 6, 9, 6, 9, 6, 5 ×9. The AGC test's draw 8: 6 ×11, 5, 6 ×4, 5, 0 ×4, **6 ×9**, 5 ×2, 2 ×2,
**5 ×6**, 0 ×2, **6 ×3**, 0 ×4, **6 ×8**, 1, 2. Faster learning pulls the offset down after each
run of failures, but with only 1 dB of caution the prior climbs back over the floor within a few
transmissions, and a 7 ms echo kills every OFDM frame. Today's 3 dB keeps OFDM under the floor
long enough for the session to finish there.

**What it says.** Both losing tests are 500 Hz NVIS, and most of the K2 flags are 500 Hz or weak
paths; the gains are mostly on 2300 Hz paths. Gate 0 §2 measured the 500 Hz rungs' fading
penalty as the larger (their 10 % points 5.8–9.7 dB over AWGN by the reading at 500 Hz, against
2.6–6.3 at 2300 Hz), which fits: the narrow air, with a fifth of the frequency diversity, needs
the caution the wide one does not. The next step is the author's.

### 5.3 The 2300 Hz air alone

The author's next choice: the 1 dB margin and G2b on the 2 300 Hz air only, today's controller on
the 500 Hz air. A bench switch, `wide_only` (`GrantedEngine._rate_review` in the model,
`RateReview::for_air` in the port, `AETHER_BENCH_LINK=…,wide_only`), applies the review's values
by the air the station's capability byte states, so a station that moves between airs
(ADR-0026) takes them on and off with it. The granted vectors gain `wide-only-good-wide` (the
same session as the 1 dB margin with G2b) and `wide-only-moderate-narrow` (the same session as
today's); a planted fault in the air check fails the replay.

**Run 28** at `0c0504f` (`bench_link = prior_margin_db=1,family_gain_db=1,wide_only`), the 26
scenarios, ten draws; all 520 daemons logged the values. Per draw in `data/gate3_wide_only.csv`.

| # | criterion | result | |
|---|---|---|---|
| K1 | the gap to the best single rung, +15 % or less | 40 m Good 2 535 bit/s, **+6.3 %**; steady mail 1 104, **+6.5 %** | **passes** |
| K3 | the start pair, no more draws over one rung apart than today | 1 (draw 0, 7 against 10, as today) | **passes** |
| K8 | both echo tests inside their budgets | Trimode 10 of 10, the AGC test 10 of 10 (today 10 and 9) | **passes** |
| K2 | no speed scenario slower (paired 90 % range end under 1.10) | three flags: 20 m DX at 2300 Hz 1.02 (0.93–1.17), the AGC test 1.00 (0.78–1.20), Trimode 1.04 (0.93–1.20) | **fresh draws owed** |

Faster, paired: equatorial flutter 0.54 (211 → 522 bit/s), VARA QRM 0.81 (719 → 928), steady mail
at 2300 Hz 0.87, thunderstorm 0.88, 40 m Good 0.93, the polar path 0.97. Every 500 Hz scenario
reads 1.00 paired; seven of them sent the same data rungs as today in all ten draws (20 m DX at
500 Hz, NVIS disturbed, PACTOR, QSB, 80 m weak, steady mail at 500 Hz, and eight or more of ten on
the lopsided and weak ladder paths).

**The two 500 Hz flags are the harness's.** On the 500 Hz air this build runs today's controller:
the station's engine is built with its air's capability byte, and the vectors' narrow session is
today's step for step. The AGC and Trimode tests sent the same data rungs as run 24 in one draw of
ten, but they do the same between any two runs: run 24 against run 25 (whose switches barely act
there) one and none of ten. Both have steps timed on the wall clock (a host program's, a clear
channel's), so a run lands a few blocks apart and takes another course (ROADMAP P9-18). K2's rule
still asks for their fresh draws, and they cost little.

**20 m DX at 2300 Hz** is the one real question: the candidate acts there in every draw (0 of 10
the same), at a median 1.02 with a range to 1.17. Its fresh draws (10–19, both builds) decide it.

**K2's fresh draws** (runs 29 and 30 at `ce1a165`: today's build and the candidate on draws
10–19 of the three flagged scenarios; per draw in `data/gate3_wide_only_fresh.csv`):

| scenario | today, median | candidate | paired ratio (90 % range) | |
|---|---|---|---|---|
| 20 m DX at 2300 Hz | 800 bit/s | 775 | 1.06 (0.99–1.13) | **fails again** |
| the AGC test (500 Hz) | 34 | 36 | 0.99 (0.90–1.31); unpaired 0.97 (0.87–1.08) | today's controller: noise |
| Trimode (500 Hz) | 38 | 38 | 1.03 (0.96–1.11) | today's controller: noise |

**20 m DX at 2300 Hz, over all twenty draws:** the candidate is slower in 10, faster in 7 and the
same in 3; the median time ratio is 1.03 and the geometric mean 1.04, spread 0.79–1.47. The
scenario is a long one (probe, call, four exchanges, a disconnect, then a Test on ITU Moderate at
8 dB with a DX offset), measured by the Test's file rate, and its worst draw shows the two builds
choosing much alike (both on rung 11 for most of the session, the ladder at the end), the file
rate set by where the fades fell. By K2's rule it fails: its range ends over 1.10 on both sets of
draws. By size it is a few per cent, inside the 10 % the guard allows for a median. A narrow miss,
the author's call (`00-brief.md` §6, answer 2).

**The author's decision (2026-11-13):** the 20 m DX miss is accepted as a narrow one, inside the
10 % guard by its median; the candidate goes on to the last checks (K4 every scenario at three
draws, K5 the real-modem link bench), asked for before they run.

### 5.4 The last checks (K4, K5)

`candidate.yml` run 3 at `1f847c0` (`bench_link = prior_margin_db=1,family_gain_db=1,wide_only`,
`--rate prior_margin_db=1,family_gain_db=1,wide_only=1`), about four hours of runner time.

**K4, every scenario at three draws**, against the nightly of the same build with the switches off
(run 65 at `bc6a003`: 192 of 192 passed, no radiated overlap): **190 of 192**, no radiated
overlap. Two failures:

* `80m-agc-carrier-outside-500`, draw 1 (seed 1031): the message did not arrive in 300 s. On the
  500 Hz air the candidate runs today's controller, and today's lost exactly this draw in run 24;
  the nightly passed it. The test's edge and the harness's course, not the candidate.
* `40m-rtty-contest-2300`, draw 0: the Test's file ran out of its budget (3 424 of 4 096 bytes, 92
  bit/s; the nightly 218 bit/s on the same draw, and the baseline's slowest run 96). The candidate
  acts here, and the trace says how. Four RTTY stations sit in the passband; at the receiver,
  rung 11 decoded **1 of 39** frames and rungs 7–8 decoded 52 of 56. The candidate kept going back
  to rung 11 between stretches at 7–8 (the caller's data: 11 ×34, 7 ×6, 11 ×11, 8, 11 ×6, …, 11 ×3
  five more times), where today's controller stayed on rungs 8–10. The reading, +12 dB, does not
  see narrowband interference; with 1 dB of caution the prior rates rung 11 near certain again as
  its failures age out of its account, and each return costs a transmission. The other two draws:
  400 against 362 and 142 against 139 bit/s.

**K5, the real-modem link bench, both arms** (8 000 B, three trials a point, −4 … +20 dB):

* 500 Hz: identical to today's at every point (the candidate runs today's controller there).
* 2300 Hz: completion equal at every point; session time pooled **×0.964**. AWGN ×0.67–0.79 from
  0 to +12 dB. Two points over ×1.05: **Moderate 0 dB ×1.071** (364 → 390 s) and **Poor 0 dB
  ×1.057** (350 → 369 s).

**What the last checks say.** The candidate is right for strong and clean 2300 Hz paths, and
costs a few per cent where a 2300 Hz path is weak and fading, or where interference the reading
cannot see sits on a rung (20 m DX Moderate 8 dB, the bench's 0 dB fading points, the RTTY
contest). In each the mechanism is the same: with 1 dB of caution, a rung that keeps failing is
rated good again as its failures age, and it is tried again. Three narrow misses (K2 on 20 m DX,
K5 at two points, K4 on one RTTY draw) for the author.

## 6. The decision

The author adopted the 2 300 Hz-only change as it is, with its three narrow misses (2026-11-13):
**ADR-0111**. Gates 5 and 6 in one change: the values became the 2 300 Hz air's defaults in the
model (`WIDE_PRIOR_MARGIN_DB`, `WIDE_FAMILY_GAIN_DB`, `GrantedEngine._air_guess`) and the port
(`RateController::for_air`); the try's switches, `RateReview`, `AETHER_BENCH_LINK`, the oracle's
`bench_link` input and `candidate.yml` were deleted; the granted and link vectors were regenerated
(the controller traces now state their air); two tests were made to hold their own rule rather than
the controller's course (ADR-0111 §4). The link bench and the replay tool now state their stations'
air in the capability byte, as a daemon does.
