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
