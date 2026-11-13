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
