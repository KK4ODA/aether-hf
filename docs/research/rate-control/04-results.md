# Gate 3: prototypes and results

*In progress. The rate controller's review (`00-brief.md` §7), from 2026-10-24. The prototypes of
`03-candidates.md` §2 in the model, the iteration that shaped them, and their results against the
criteria registered in `03-candidates.md` §3 (agreed 2026-10-24).*

## 0. Where this stands

* **Built in the model, bench-only**: `model/aether_model/link/rate_review.py`, selected by
  `GrantConfig.controller` (`"current"` by default, which is unchanged: the granted vectors
  regenerate byte for byte). C1 at the receiver (`delivery`), C1 at the sender (`delivery-s`),
  C2 (`olla`); C0 is the current controller with parameters overridden (`GrantConfig.rate`).
* **The tools**: `tools/bench_link.py --link controller=NAME` (the link bench, both pipes and the
  real modem); `tools/rate_k6.py` (criterion K6, the field replay); an iteration bench on the real
  modem on seeds, sizes and SNRs that are not K5's grid (§2.1).
* **Ported for the harness** (bench-only, `aether_link::rate_review`, `granted::Controller`,
  default `Current`): C2, C1 at the receiver and C1 at the sender. The port is held to the model
  by `tests/data/rate_review_vectors.json` (`tools/make_rate_review_vectors.py`): the granted
  engine's trace sessions with each prototype at both stations, four of its own (frames never
  detected, lopsided paths), and direct traces of the controllers' inputs. Planted faults each
  fail it.
* **On the harness** (§3.7): every candidate against the current controller on the 20 regret
  scenarios, the start pair and `80m-agc-carrier-outside-500`, three draws each. The leader is C1
  at the sender: with its silent step fixed (§3.10), 69 of 69 complete and K1(a), K1(b) and K3
  each a near miss; ranked per exchange (§3.11, §3.12), K1(a) passes, `80m-slow-machine-500`
  fails K1(b) and one run fails to complete; without its step down for silence it is worse
  (§3.13, reverted). Where it stands: §3.14.
* **Next**: the author's decision on §3.14's failures, case by case; then K2 and K5 for the
  candidate taken forward.

## 1. What the engine hands a prototype

The current controller is fed a transmission's counts (`ok`, `failed`, the rung most of it went
at) under ADR-0020's rules. A prototype is handed each data frame instead (`FrameOutcome`, built by
`GrantedEngine._outcomes`): its rung, its air, its capacity, whether it was detected, decoded
(alone or by combining) and combined. A frame the transmission's header stated — or a count the
station can believe from a frame that decoded alone — and that nothing reported is a frame
**never detected**, at the rung most of the transmission went at: air spent for nothing, which
the current controller cannot see (`01-evidence.md` §5, report 15). A frame neither decoded nor
trusted is no frame of the path, as before.

At the sender (`delivery-s`), each copy sent is judged by the next acknowledgement: a block it
acknowledges for the first time was delivered by one of the copies since the last one, every other
copy was air spent.

## 2. Iteration

### 2.1 The iteration bench

The real modem (`bench_link.run_point`, `phy`, one fade a session shared by both directions), both
airs, AWGN and ITU Good, Moderate and Poor, five SNRs an air (2 300 Hz −2 … +18 dB, 500 Hz −4 …
+12 dB), seeds 503 and 517, 4 000 bytes — none of them K5's registered grid (seeds 100, 107, 114,
8 000 bytes, −4 … +20 dB), so iterating here does not tune to the criterion. The fading pipe is
used for a first look only (`00-brief.md` §7).

### 2.2 What the iteration found

1. **The prior's margin** (C1's first form: the AWGN curve 3 dB under the reading). The field
   replay (K6) failed on WC4Y's 500 Hz Test of 2026-10-22 at 0.42 of the current controller: at
   a −1 dB floor reading the prior put the first OFDM rungs at twice the floor's top rung, where
   the session's own ladder decoded a quarter of rung 6's frames. The measured fading penalties are
   5–10 dB, not 3 (`bench/baselines/phy_fer*.csv`). The prior is now **the mean of the four
   channels' curves** at the reading (AWGN, Good, Moderate, Poor; `CLASS_PENALTY_DB`), which
   predicts that session's ladder (rung 6 ≈ 25 %, rung 4 ≈ 96 % against 86 % measured) and
   replaces a tuned margin with a measured table.
2. **Measured rungs above.** C1 climbed only by its own try, so frames sent at a rung above for
   another reason (a copy at its first rung, the Test's measurement frames) counted for nothing; on
   ND1J's strong 500 Hz Tests it sat under rungs the session had measured. Now such frames judge
   that rung as a try does, and the choice takes any rung measured.

### 2.3 First numbers (C1's first form, the 3 dB prior)

The iteration bench, current against `delivery`, session time (lower is better):

| air, channel | median ratio |
|---|---|
| 2 300 Hz AWGN | 0.83 |
| 2 300 Hz Good | 0.94 |
| 2 300 Hz Moderate | 0.93 |
| 2 300 Hz Poor | 1.02 |
| 500 Hz AWGN | 0.92 |
| 500 Hz Good | 0.94 |
| 500 Hz Moderate | 0.91 |
| 500 Hz Poor | 0.98 |
| pooled (80 sessions) | median 0.935, geometric mean 0.923 |

No session failed on either. The worst cells were 1.25 (2 300 Hz Poor, +8 dB) and 1.24
(2 300 Hz Good, −2 dB): K5's "×1.05 at every point" would fail on this form.

### 2.4 The prototypes on the iteration bench

Every arm against the current controller on the same 80 sessions (session time, lower is better;
"worst" the worst single cell). The prior is the AWGN curve 3 dB under the reading unless named
("four channels": their mean, §2.2 item 1).

| arm | pooled median | geometric mean | worst | failed sessions |
|---|---|---|---|---|
| **C2** (`olla`) | **0.886** | **0.865** | 1.18 | 0 |
| C1 (`delivery`, with §2.2 item 2) | 0.942 | 0.921 | 1.18 | 0 |
| C1, first form | 0.935 | 0.923 | 1.25 | 0 |
| C1 at the sender (`delivery-s`) | 0.945 | 0.936 | 1.36 | 0 |
| C0, everything taken out (hysteresis, dwell, stride, decay schedule, re-seed, steps in hand, partial rule) | 0.961 | 0.962 | 1.36 | 0 |
| C2, four channels | 1.000 | 1.012 | 1.43 | 0 |
| C1, four channels | 1.095 | 1.106 | 1.99 | 0 |

C2 per class: 2 300 Hz AWGN 0.78, Good 0.89, Moderate 0.83, Poor 0.90; 500 Hz AWGN 0.83, Good 0.93,
Moderate 0.92, Poor 0.94. **The optimistic prior beats the four channels' mean for both C1 and
C2**, though the mean predicts the field's ladders better: a rung above found wanting costs one
measured transmission, and a rung under the path's best costs every wait for a try (C1) or the
offset's whole walk down (C2) — the survey's "optimism in the face of uncertainty"
(`02-survey.md`).

### 2.5 The field replay (K6), and what it can judge

| arm | sum against the current's | worst session | K6 |
|---|---|---|---|
| C2 | 1.141 | 0.499 (WC4Y, 500 Hz, 2026-10-22) | fails |
| C1 | 1.158 | 0.800 (ND1J, 500 Hz, 2026-10-15 15:42) | fails |

Both fail the worst-session rule. The trace of C2's: at that Test's −1 dB floor reading (a lower
bound, ADR-0016) the optimistic prior puts the first OFDM rungs (5, 6) above the floor's top
rung (4), where the session's ladder decoded rung 6 a quarter of the time and rung 4 86 %. Open
loop, a choice the session never sent at is never measured, so the replay keeps judging the
same unlearned choice; closed loop, the first transmission at rung 6 would move the OFDM offset
by up to +3 dB. On the iteration bench at 500 Hz around 0 dB C2 ran 0.97–1.08 of the current
controller, and the harness judges the closed loop (`80m-floor-top-start-500` is WC4Y's path).
The criterion stands as agreed; this is the explanation `03-candidates.md` §3 asks for, to put to
the author with the harness's numbers.

## 3. Results against the criteria

### 3.1 C2 on the harness: K1 and K3 (2026-10-24)

C2 in the daemon (a worktree of `b2b0efb` with `GrantConfig::default().controller` = `Olla`,
its own target directory), the 20 regret scenarios and the start pair, three draws each (the
scenario's seed, +1000, +2000); 66 runs, all complete. The fixed rungs are Gate 2's
(`03-candidates.md` §5.1).

| scenario | current, 3 draws | C2, 3 draws | C2's regret, median | the current's | C2 / current |
|---|---|---|---|---|---|
| `quick-awgn-2300` | 1263 / 1263 / 1263 | 2400 / 2400 / 2400 | −9 % | +73 % | 1.90 |
| `40m-good-throughput-2300` | 2327 / 2286 / 2133 | 1882 / 2032 / 2667 | +34 % | +19 % | 0.89 |
| `80m-strong-moderate-500` | 151 / 260 / 244 | 263 / 214 / 207 | +63 % | +70 % | 0.88 |
| `20m-dx-500` | 146 / 151 / 144 | 159 / 167 / 123 | +38 % | +50 % | 1.09 |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 | 750 / 800 / 857 | +25 % | +46 % | 1.27 |
| `80m-qsb-dropout-500` | 146 / 158 / 142 | 187 / 168 / 120 | +33 % | +38 % | 1.15 |
| `80m-weak-500` | 70 / 67 / 65 | 65 / 67 / 70 | +17 % | +17 % | 1.00 |
| `80m-flutter-500` | 129 / 120 / 127 | 154 / 155 / 136 | +9 % | +26 % | 1.21 |
| `15m-equatorial-flutter-2300` | 387 / 211 / 211 | 632 / 261 / 500 | +50 % | +255 % | 2.37 |
| `80m-nvis-evening-500` | 41 / 25 / 26 | 39 / 49 / 14 | 0 % | +56 % | 1.48 |
| `80m-thunderstorm-agc-2300` | 591 / 535 / 616 | 460 / 651 / 622 | +58 % | +60 % | 1.05 |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 99 / 84 / 84 | 0 % | 0 % | 1.07 |
| `20m-polar-path-2300` | 198 / 198 / 172 | 170 / 198 / 172 | +49 % | +44 % | 0.87 |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 | 175 / 175 / 175 | 0 % | +4 % | 1.04 |
| `40m-good-2300-rtty-crashes` | 425 / 442 / 496 | 442 / 604 / 616 | +28 % | +75 % | 1.37 |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 | 68 / 70 / 69 | +62 % | +58 % | 0.96 |
| `80m-nvis-disturbed-500` | 38 / 44 / 36 | 25 / 22 / 21 | +95 % | +3 % | **0.58** |
| `40m-moderate-2300-vara-qrm` | 842 / 390 / 667 | 1032 / 627 / 1000 | +19 % | +65 % | 1.50 |
| `80m-lopsided-acks-500` | 77 / 76 / 83 | 99 / 85 / 74 | +9 % | +28 % | 1.11 |
| `80m-slow-machine-500` | 119 / 91 / 106 | 76 / 89 / 148 | +48 % | +44 % | **0.83** |

* **K1(a) fails**: the median regret is +30.5 %, against the current's +45.2 % and the +22.6 %
  asked for.
* **K1(b) fails** on two scenarios: `80m-nvis-disturbed-500` at 0.58 of the current's median
  delivery and `80m-slow-machine-500` at 0.83 (0.85 asked).
* **K3 fails**, worse than the current controller: settled 4, 0 and 3 rungs apart (the current:
  2, 1, 2). Draw 0: the low start settled at rung 10, the high start at 6.

**The traces.** *K3, draw 0's high start*: it started at rung 9 and went to 11 on its second
transmission (15 frames there), bounced between 9 and 11 for a minute, then slid to 7, 6, 5 and
3 between 107 and 175 s at readings of 6–7 dB, and climbed back only to 9 by the ladder. C2's
offset rises 0.5 dB a failed frame and falls 0.125 dB a decoded one (4:1 at its target of about
20 %): the failures of a start too high, and of the fades that follow, are paid back four frames
for one, and a single offset for the whole OFDM family carries a failure at rung 11 into the
choice at rung 7. *K1(b), the disturbed NVIS*: no OFDM rung survives the 7 ms echo, and the floor's
reading swings −4 … +1 dB. C2 sent its first transmission at OFDM rung 5 (the optimistic prior at
the call's reading), and went back to rung 5 whenever the reading rose: 6, 16, 2 and 3 frames there,
against the current controller's floor rungs throughout. The iteration bench had neither NVIS nor
a start pair, which is why it ranked C2 first; it gains both (`iterate2`).

**What it says.** C2's steady-channel and flutter gains are large (`quick-awgn-2300` ×1.90,
`15m-equatorial-flutter-2300` ×2.37), but one offset per family, moved 4:1, is not start-
independent, and the optimistic prior goes back to a family the path has refused. Next: C1 on the
extended iteration bench and then on the harness, with the same two scenarios as its first test.

### 3.2 C0, everything taken out, on the harness (2026-10-24)

The current controller with `up_hysteresis_db` 0, `up_dwell` 0, `max_up_step` 20, the decay a
fixed 0.167 dB a clean burst (OLLA's step for a 10 % target), no re-seed, `first_mode_back` 0 and
no partial rule — no new code, the defaults changed in a worktree's `RateConfig`. Same 66 runs.

* **K1(a)**: +46.7 % (the current +45.2 %): no gain.
* **K1(b)**: eight scenarios under 0.85 of the current's delivery, the worst
  `40m-good-2300-rtty-crashes` at 0.69.
* **K3**: settled 4, 1 and 0 rungs apart.

The iteration bench had it ×0.961, and start-independent on the model's stand-in: the second time
the model's benches ranked a candidate the harness does not. Two of the changes compound: without
the partial rule a burst with any frame lost is a whole failure, and the fixed decay of 0.167 dB a
*burst* is far slower than the current controller's accelerating one, so the margin stays high.
C0 as registered removes the parts in turn; together is the wrong first test, and the order is
the harness's business, not the model bench's.

### 3.3 Where the current controller loses its +45 %

Each fixed rung's delivery on the harness (Gate 2's runs) is flat near the top and peaks high:
`40m-good-2300-rtty-crashes` rung 14 at 757 bit/s against the controller's 425,
`80m-thunderstorm-agc-2300` rungs 14–15 at 826–994 against 535–616, `80m-strong-moderate-500`
rung 13 at 316–415 against 151–260. Read with `tools/rung_reading.py`, the last: the readings sat
at +8 … +12 dB the whole session while the controller walked 11 → 10 → 9 → 7; rung 11 delivered
383 bit/s of frame air against rung 10's 223 with a quarter of its first copies lost, and a third
of rung 10's frames were never detected at all (500 Hz OFDM acquisition, every rung alike). On a
slowly fading path a rung held high through the fades delivers most — HARQ carries its frames
through the dips — and a controller that steps down in every dip and climbs back slowly spends its
time low. The operating point that pays has a first-copy loss of 25–30 %, as the survey's HARQ
sources say (`02-survey.md` §2.3), where the current controller aims at about 5 %.

### 3.4 C1 on the harness (2026-10-24)

C1 at the receiver in the daemon (`559baaa`, `Controller::Delivery`), the same 66 runs.

| scenario | current, 3 draws | C1, 3 draws | C1's regret, median | the current's | C1 / current |
|---|---|---|---|---|---|
| `quick-awgn-2300` | 1263 / 1263 / 1263 | 2400 / 2400 / 2400 | −9 % | +73 % | 1.90 |
| `40m-good-throughput-2300` | 2327 / 2286 / 2133 | 2133 / 2723 / 2327 | +12 % | +19 % | 1.02 |
| `80m-strong-moderate-500` | 151 / 260 / 244 | 192 / 282 / 252 | +65 % | +70 % | 1.03 |
| `20m-dx-500` | 146 / 151 / 144 | 188 / 219 / 198 | +3 % | +50 % | 1.36 |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 | 750 / 706 / 750 | +42 % | +46 % | 1.19 |
| `80m-qsb-dropout-500` | 146 / 158 / 142 | 181 / 188 / 160 | +19 % | +38 % | 1.24 |
| `80m-weak-500` | 70 / 67 / 65 | 70 / 77 / 82 | 0 % | +17 % | 1.15 |
| `80m-flutter-500` | 129 / 120 / 127 | 140 / 110 / 160 | +16 % | +26 % | 1.10 |
| `15m-equatorial-flutter-2300` | 387 / 211 / 211 | 500 / 222 / 240 | +212 % | +255 % | 1.14 |
| `80m-nvis-evening-500` | 41 / 25 / 26 | 4 / 44 / 13 | +215 % | +56 % | **0.50** |
| `80m-thunderstorm-agc-2300` | 591 / 535 / 616 | 548 / 618 / 602 | +61 % | +60 % | 1.02 |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 78 / 84 / 76 | 0 % | 0 % | 1.00 |
| `20m-polar-path-2300` | 198 / 198 / 172 | 165 / 198 / 172 | +49 % | +44 % | 0.87 |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 | 175 / 175 / 175 | 0 % | +4 % | 1.04 |
| `40m-good-2300-rtty-crashes` | 425 / 442 / 496 | 505 / 524 / 592 | +47 % | +75 % | 1.19 |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 | 83 / 62 / 88 | +33 % | +58 % | 1.15 |
| `80m-nvis-disturbed-500` | 38 / 44 / 36 | 29 / 44 / 21 | +34 % | +3 % | **0.76** |
| `40m-moderate-2300-vara-qrm` | 842 / 390 / 667 | 1103 / 582 / 711 | +41 % | +65 % | 1.07 |
| `80m-lopsided-acks-500` | 77 / 76 / 83 | 104 / 65 / 74 | +44 % | +28 % | 0.95 |
| `80m-slow-machine-500` | 119 / 91 / 106 | 117 / 83 / 153 | +9 % | +44 % | 1.10 |

* **K3 passes**: settled 1, 0 and 0 rungs apart (low start 10, 11, 11; high start 11, 11, 11) —
  the first candidate to, and the current controller does not.
* **K1(a) fails**: +33.5 % (the current +45.2 %, +22.6 % asked).
* **K1(b) fails only on the two NVIS paths**: `80m-nvis-evening-500` at 0.50 and
  `80m-nvis-disturbed-500` at 0.76; every other scenario at 0.87 or more, fourteen of the twenty
  above 1.

**The trace** (`80m-nvis-evening-500`, draw 0, 4 bit/s): C1's first rung was OFDM 6 (the
optimistic prior at the call's reading); 6 failed, and C1 stepped down to OFDM 5 — never sent at,
so still on the prior, which put it at twice the floor's rung 2 at a −1 dB reading. Over the
session it sent 90 frames at OFDM 5–6 on a path where no OFDM rung survives the echo; the
current controller stayed on the floor's rungs 1–2. The disturbed NVIS: the same, from OFDM 5–6
at the start. **The failures of one OFDM rung said nothing to the prior of the OFDM rung below
it.**

**The change** (C1 v2, `8b21148`): the prior of a family's rungs carries the family's offset,
moved by every frame of the family by `gain × (p − decoded)` (0.5 dB) — up by nearly the gain for
a failure the prior was sure of, down by nearly it for a success it ruled out. A measured rung is
still judged by its own account; the offset reshapes only the prior. One setting added; C2's one
good idea (a family's evidence) without its single-offset choice. On the harness next.

### 3.5 C1 v2 (the family's offset) on the harness, and v3 (2026-10-24)

C1 with the family offset (`8b21148`), the same 66 runs.

* **K1(a) passes by a hair**: +22.55 % against +22.62 % asked (the current +45.24 %) — inside the
  statistic's own spread on draws alone (±7 points), so a case for the author, not a pass by rule.
* **K1(b)**: `80m-nvis-evening-500` 0.50 → 0.95; `80m-nvis-disturbed-500` still 0.76, its
  deliveries the same as v1's to the bit (29 / 44 / 21): the change never acted there.
* **K3 fails**: 3, 0 and 2 apart (v1 passed: 1, 0, 0).

**The traces.** *The disturbed NVIS*: the master's first transmission went at OFDM 5 (its own
controller's first rung, the optimistic prior at the acceptance's reading of the call), before any
frame was measured, so the family offset had nothing to act on; its six blocks were then bound to
rung 5 until each had gone four times (`max_combines`), 24 OFDM frames on a path whose echo
refuses OFDM — the engine's re-encode rule, which every candidate keeps (`03-candidates.md`
§1.1). *The start pair*: the low start of draw 0 sent 35 frames at rung 6 before its first try
upward (the first try waited `probe_every`, six transmissions); the high start went 9 → 6 and
climbed to 12–14 only late.

**The change** (C1 v3, `6eb8b1d`): the first rung is the best by the four channels' mean
(`cautious_start`; the tables are in the port, `class_penalty_db`, held to the model's), and the
rung above it is tried at the next transmission. The optimistic prior stays for every choice
after the start. The reason is the asymmetry the iteration found (§2.4): after the start a rung
found wanting costs one measured transmission, but the first transmission's blocks are bound to
its rung before anything is measured.

### 3.6 C1 v3 (the cautious start) on the harness: rejected (2026-10-24)

C1 with the first rung by the four channels' mean and the first try at once (`6eb8b1d`), the same
66 runs. It did what it was built for — `80m-nvis-disturbed-500` 0.76 → 1.03 — and cost more
everywhere a climb is the job:

* **K1(a)**: +28.7 % (v2 +22.55 %). `quick-awgn-2300` 2400 → 1714 bit/s.
* **K1(b)**: three scenarios under 0.85, all weak paths where the cautious curves start low:
  `80m-weak-both-ways-ladder-500` 0.67, `20m-polar-path-2300` 0.74, `80m-weak-wide-test-2300`
  0.76.
* **K4**: one Test failed (`80m-nvis-evening-500`, the third draw: the message timed out).
* **K3**: 2, 0 and 2 apart.

Off by default (`cautious_start`), kept until the decision ADR says what goes. The disturbed
NVIS's cost is the first transmission's blocks bound to an OFDM rung until their copies run out
— the engine's re-encode rule, which every candidate keeps, and the echo exclusions (ADR-0087),
whose 500 Hz table has no OFDM rung. Neither is the controller's to change (`03-candidates.md`
§1.1).

### 3.7 The candidates on the harness so far

The 20 regret scenarios and the start pair, three draws each. "Under 0.85": scenarios whose median
delivery is under 85 % of the current controller's (K1(b)).

| arm | K1(a): median regret (≤ +22.6 %) | K1(b): under 0.85 | K3: settled apart (≤ 1 each) | failed runs |
|---|---|---|---|---|
| the current controller | +45.2 % | — | 2, 1, 2 | 0 |
| C2 | +30.5 % | 2 (disturbed NVIS 0.58, slow machine 0.83) | 4, 0, 3 | 0 |
| C0, everything taken out | +46.7 % | 8 | 4, 1, 0 | 0 |
| C1 | +33.5 % | 2 (NVIS evening 0.50, disturbed NVIS 0.76) | **1, 0, 0** | 0 |
| **C1 v2** (the family's offset) | **+22.55 %** | **1 (disturbed NVIS 0.76)** | 3, 0, 2 | 0 |
| C1 v3 (the cautious start) | +28.7 % | 3 | 2, 0, 2 | 1 |
| C1 at the sender (§3.9) | +26.1 % | 2 (disturbed NVIS 0.84, slow machine 0.77) | 1, 0, 1 | 3 (the AGC carrier, all three) |
| C1 at the sender, echo kept (§3.10) | +25.16 % | 1 (disturbed NVIS 0.63) | 3, 1, 1 | 3 (the AGC carrier, all three) |
| **C1 at the sender, its silent step fixed** (§3.10) | **+22.63 %** | **1 (disturbed NVIS 0.84)** | 3, 1, 1 | **0** |
| C1 at the sender, priced per exchange (§3.12) | **+21.30 %** | 1 (slow machine 0.76) | 5, 1, 1 | 1 (the AGC carrier, draw 0) |
| … and no step down for silence (§3.13, reverted) | +31.83 % | 1 (disturbed NVIS 0.66) | 5, 0, 0 | 2 (the AGC carrier, draws 0 and 2) |

The C1-at-the-sender rows count `80m-agc-carrier-outside-500`'s three runs too (69 runs).

K3 moved from pass to fail between C1 and C1 v2, a change that touches only unmeasured rungs'
priors: on three draws a settled rung is noisy (a change that moves the session's timing puts it
on another stretch of the same fade, `00-brief.md` §8).

### 3.8 C1 v2 on K2, K4 and K6 (2026-10-24)

**K2 and K4**: every scenario in `bench/scenarios/` (60), three draws (seeds s, s+1000, s+2000),
the current controller and C1 v2 on the same build (`3712e2e`, the default and a worktree with
`Controller::Delivery`; the cautious start off, so C1 v2's behaviour), 180 runs each.

* **Pooled air**: ×0.970 of the current's (48 586 against 50 100 s; the median of the
  per-scenario ratios ×0.964). K2's pooled rule passes.
* **Radiated overlaps**: 0 and 0. K4's overlap rule passes.
* **Completion**: 178 of 180 against 180. Both failures are `80m-agc-carrier-outside-500` (draws
  0 and 2: "message 1000: not within 300 s of air") — K4 fails.
* **`scenario_trend.py`** against the current's three draws: *failing*
  `80m-agc-carrier-outside-500` (1/3 against 3/3); *slower* `80m-nvis-disturbed-500` (air 310 s
  against the current's 217–267) and `80m-nvis-evening-500` (file 28 bit/s against 39–47) — K2
  fails; *faster* `20m-dx-moderate-2300`, `40m-drift-and-clocks-2300`,
  `40m-long-attachment-cwid-2300`, `80m-capture-skips-500`, `80m-start-low-reading-500`.

All three are 500 Hz paths on ITU's NVIS channel (7 ms, 1 Hz). **K6**: 1.139 of the current's
measured delivery summed, the worst session 0.424 (WC4Y, 500 Hz, 2026-10-22, §2.5) — fails.

**Why NVIS, and why not the echo table.** The 500 Hz OFDM rungs have no echo limit because the
measurement does not give them one: on ITU NVIS at 20 dB rung 5 delivers 7.8 B/s of its frames
against the floor's best 6.2 (`bench/baselines/echo.csv`; at 10 dB 5.2 against 6.2), so NVIS
*carries* it by `bench_echo.limits`'s rule, while 3 ms at 10 Hz (echo 0.0019) refuses it — an
echo reading cannot separate them. What NVIS costs C1 is what a try of OFDM costs beyond its own
air: C1 tries a rung with a whole transmission, and every block of a failed try stays at its
rung until it has gone four times (`max_combines`) before it can be re-encoded — up to 24 OFDM
frames for one try — where C1's account charges the try only its own frames. The current
controller never tries OFDM on these paths: its margin and its two steps in hand keep it on the
floor.

### 3.9 C1 at the sender on the harness (2026-10-24)

C1 at the sender (`78e6a12`, `Controller::DeliveryS`): the station chooses its own data's rung
with C1, fed by what each acknowledgement says of the copies sent since the last; the other
station's recommendation is advice; a try is one new frame at the rung above (across the floor's
edge, a whole transmission). The same 66 runs and `80m-agc-carrier-outside-500`'s three.

* **K1(a)**: +26.1 % (fails; C1 v2 +22.55 %).
* **K1(b)**: `80m-nvis-disturbed-500` 0.84 (C1 v2 0.76), `80m-slow-machine-500` 0.77 — two
  under 0.85.
* **K3 passes**: 1, 0 and 1 apart.
* **`80m-agc-carrier-outside-500`**: none of three complete (C1 v2 one, the current three).

**The trace** (`80m-agc-carrier-outside-500`): the sender sent at rungs 3 and 4 transmission
after transmission — the rungs the 7 ms echo rules out (ADR-0087). Its own controller had never
been told the echo readings: the engine gave them to the receive-side controller only (`_heard`).
**A defect**, not a finding: every candidate keeps the echo exclusions (`03-candidates.md` §1.1).
Fixed in `64296a7`, with one design change beside it: every copy of a block, re-encoded or not,
is now charged to the rung the block was cut at, so a try of OFDM whose blocks strand and go
again on the floor costs that choice all of it — what C1 at the receiver could not charge (it
sees frames, not blocks' histories) and what made the NVIS paths cost (§3.8). On the harness
in §3.10.

### 3.10 C1 at the sender: the echo kept, then its silent step (2026-10-24)

**With the echo kept** (`64296a7`): K1(a) +25.16 %, K1(b) `80m-nvis-disturbed-500` 0.63, K3 3, 1
and 1 apart, and `80m-agc-carrier-outside-500` none of three still. Its trace: transmission after
transmission at rungs 3 and 4 again, each after a silent slot. The engine's own step down after
silence (R9, `_back_off`) told C1 to step too (`DeliveryController.silent`), and C1 stepped by
the ladder's index alone — from rung 5 on a 7 ms echo, onto rung 4, which the echo rules out.
**A defect**: a step down after silence now lands on the highest rung at or under it that the
echo allows (`d3bf27b`, model and port, a test in each; the vectors never take such a step and
regenerate byte for byte).

**With the step fixed** (`d3bf27b`), all 69 runs:

| scenario | current, 3 draws | C1 at the sender, 3 draws | regret | candidate / current (medians) |
|---|---|---|---|---|
| `quick-awgn-2300` | 1263 / 1263 / 1263 | 2182 / 2182 / 2182 | +0 % | 1.73 |
| `40m-good-throughput-2300` | 2327 / 2286 / 2133 | 1803 / 2723 / 2169 | +20 % | 0.95 |
| `80m-strong-moderate-500` | 151 / 260 / 244 | 276 / 280 / 323 | +24 % | 1.15 |
| `20m-dx-500` | 146 / 151 / 144 | 184 / 198 / 118 | +19 % | 1.26 |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 | 800 / 706 / 750 | +42 % | 1.19 |
| `80m-qsb-dropout-500` | 146 / 158 / 142 | 171 / 177 / 168 | +17 % | 1.17 |
| `80m-weak-500` | 70 / 67 / 65 | 70 / 77 / 82 | +0 % | 1.15 |
| `80m-flutter-500` | 129 / 120 / 127 | 138 / 150 / 165 | +18 % | 1.18 |
| `15m-equatorial-flutter-2300` | 387 / 211 / 211 | 500 / 211 / 240 | +212 % | 1.14 |
| `80m-nvis-evening-500` | 41 / 25 / 26 | 39 / 44 / 44 | −5 % | 1.65 |
| `80m-thunderstorm-agc-2300` | 591 / 535 / 616 | 576 / 580 / 590 | +67 % | 0.98 |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 32 / 74 / 76 | +23 % | 0.94 |
| `20m-polar-path-2300` | 198 / 198 / 172 | 151 / 198 / 172 | +49 % | 0.87 |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 | 175 / 175 / 175 | +0 % | 1.04 |
| `40m-good-2300-rtty-crashes` | 425 / 442 / 496 | 672 / 614 / 549 | +26 % | 1.39 |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 | 82 / 86 / 76 | +36 % | 1.14 |
| `80m-nvis-disturbed-500` | 38 / 44 / 36 | 32 / 44 / 30 | +22 % | 0.84 |
| `40m-moderate-2300-vara-qrm` | 842 / 390 / 667 | 1103 / 593 / 711 | +38 % | 1.07 |
| `80m-lopsided-acks-500` | 77 / 76 / 83 | 104 / 82 / 62 | +14 % | 1.06 |
| `80m-slow-machine-500` | 119 / 91 / 106 | 149 / 94 / 110 | +40 % | 1.03 |

(Delivery in bit/s, the largest message's or the Test's file's; regret the median of the three
draws' against the best fixed rung of Gate 2's runs.)

* **Completion**: 69 of 69; `80m-agc-carrier-outside-500` three of three (328, 308 and 347 s of
  air), where both earlier forms completed none.
* **K1(a)**: the median regret is +22.6284 % against a line of +22.6222 % (half the current's
  +45.2444 %): **short by six thousandths of a point**.
* **K1(b)**: `80m-nvis-disturbed-500` at **0.84** of the current's median delivery (32, 44 and
  30 bit/s against 38, 44 and 36), the line 0.85. Every other scenario is at 0.87 or over;
  fifteen of the twenty are over 1.
* **K3** fails: 3, 1 and 1 apart (the current controller: 2, 1, 2). Draw 0's trace: from the
  low reading (the path at 9 dB, the call heard at 5) C1 at the sender started at rung 6, tried
  rung 7 at 90 s and at 99 s with one frame each, lost both, and doubled its wait each time; it
  stayed at rungs 5 and 6 to the ladder, at 91 bit/s, while the receiver advised 9 to 11 all
  along and the high draw settled at 9 (156 bit/s). One frame is a thin sample to shut a rung
  with, and the wait's doubling keeps it shut.

Three near misses, each for the author with its trace (`03-candidates.md` §4).

### 3.11 K6 for C1 at the sender, and the price of an exchange (2026-10-24)

**The judge.** K6 as built compared the receivers' recommendations, and C1 at the sender's
receiver advises with the current controller: K6 compared the current controller with itself.
`rung_reading.replay_sender` runs C1 at the sender on what the sender recorded, as the engine
feeds it (seeded by the slave's reading of the call, each acknowledgement judging the copies sent
since the last, a silent data transmission stepping it down, every decoded frame giving its
echo), and `rate_k6` judges each of the sender's data transmissions: the rung the advice had it
send at against C1's choice there, each worth what the session measured of that rung. Its first
run found a fault in the judge: a `/4` 500 Hz sidecar's rung 4 is QPSK ⅓, this tree's is
tone4x100-93, and a choice of "rung 4" had been credited with what QPSK ⅓ measured (WC4Y's Test
of 2026-10-14: 0.235 of the current; 1.031 with that rung left out). K6 now judges only rungs
whose frame is the session's (`rung_reading.foreign_rungs`), for every candidate.

**The finding.** With the judge fixed, C1 at the sender summed ×1.452 of the current and its
worst session was WC4Y's 500 Hz Test of 2026-10-22 at 0.607: it chose rungs 5 and 6 throughout
the message, where the session measured rung 4 (tone4x100-93) best. Part of that is K6's open
loop (§2.5): the session never sent data at 5 or 6, so the replay never shows C1 its own losses
there. Part is C1's measure. It ranked rungs by data per second of their frames' air, but an
exchange also spends a header, an answer and two gaps whatever the data's rung, and a
transmission holds at most six frames. The header and the answer go in the families the two
stations ask of each other, and on this path (ADR-0059: WC4Y read KK4ODA-1's ordinary answers 7
times in 15) the answers go on the floor. The 500 Hz rungs, nothing lost, in bit/s:

| rung | per second of frame air | per exchange, both ordinary | ordinary header, floor answer | both on the floor |
|---|---|---|---|---|
| 4, tone4x100-93 (5 × 5.36 s) | 134 | 125 | 114 | 105 |
| 5, QPSK ½ (6 × 1.05 s) | 167 | 128 | 96 | 77 |
| 6, QPSK ⅔ (6 × 1.05 s) | 235 | 181 | 135 | 108 |

(control frames 0.43 s ordinary and 3.2 s on the floor; the two gaps 1.05 s.) Per frame second
the first OFDM rung looked 24 % better than the floor's top; per exchange with floor answers it
is 16 % worse. **C1 now ranks rungs per second of a whole exchange** (`f7159e7`): the engine hands
it the protocol's lengths — a control frame by family, the two gaps, the frames a full
transmission carries at each rung — and, before each decision, the families both stations'
control frames go in (its own, and the one it asks of the other: `GrantedEngine._price`). No tuned
number is added. The port does the same; the review's vectors gain a families input in the
traces, lopsided sessions at both bandwidths and the families in each prototype session's digest,
and planted faults in the price and in the engine's wiring each fail them. The replays give C1 the
session's families too (`787abaf`).

K6 with the price (open loop, the nine two-sided sessions):

| arm | sum against the current's | worst session | K6 |
|---|---|---|---|
| C1 at the receiver (C1 v2 and the price) | 1.184 | 0.620 (WC4Y, 500 Hz, 2026-10-22) | fails |
| C1 at the sender (and the price) | 1.455 | 0.688 (WC4Y, 500 Hz, 2026-10-22) | fails |

The price moved C1 at the receiver's worst session from 0.424 to 0.620 and WC4Y's 2300 Hz Test
from 0.878 to 1.016. What is left on 2026-10-22 is the open loop: with floor answers rung 6 still
beats rung 4 lossless (135 against 114), the optimistic prior believes it decodes, and the
session — which sent no data there — never says otherwise; its ladder measured rung 6 a quarter
of the time. The harness judges the closed loop (§3.12).

### 3.12 C1 at the sender, priced per exchange, on the harness (2026-10-24)

`f9976ad`, the same 69 runs. Changed from §3.10 (delivery in bit/s, three draws):

| scenario | current | §3.10 | priced | priced / current (medians) |
|---|---|---|---|---|
| `80m-nvis-disturbed-500` | 38 / 44 / 36 | 32 / 44 / 30 | 40 / 44 / 30 | **1.05** (was 0.84) |
| `80m-nvis-evening-500` | 41 / 25 / 26 | 39 / 44 / 44 | 39 / 26 / 33 | 1.25 (was 1.65) |
| `80m-slow-machine-500` | 119 / 91 / 106 | 149 / 94 / 110 | 149 / 63 / 81 | **0.76** (was 1.03) |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 | 82 / 86 / 76 | 71 / 86 / 65 | 0.99 (was 1.14) |
| `80m-strong-moderate-500` | 151 / 260 / 244 | 276 / 280 / 323 | 267 / 323 / 183 | 1.10 (was 1.15) |
| `80m-lopsided-acks-500` | 77 / 76 / 83 | 104 / 82 / 62 | 103 / 76 / 106 | 1.33 (was 1.06) |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 32 / 74 / 76 | 78 / 78 / 76 | 1.00 (was 0.94) |

* **K1(a) passes**: +21.30 % against the line of +22.62 %.
* **K1(b)** fails on `80m-slow-machine-500` (0.76); every other scenario is at 0.87 or over.
* **K3** fails: 5, 1 and 1 apart (draw 0's low reading settled at rung 4).
* **Completion**: 68 of 69. `80m-agc-carrier-outside-500` draw 0 left its message undelivered in
  400 s.

**The traces.** *The AGC carrier, draw 0*: three single-frame tries of rung 5 (41.7, 85.3 and
99.5 s), then at 224 s seventeen frames at rung 5 while the receiver advised rung 2 — on a 7 ms
NVIS path, where OFDM decodes about a quarter of its frames. One try that happens to decode moves
a rung's estimate further than a thin sample should. *The slow machine*: after each of the
station's stalls (silent slots) C1 at the sender stepped its own rung down two — the engine's R9,
copied into it (`silent`, §3.10) — onto the floor (rungs 2 and 4) for long stretches, where the
current controller, set straight by the receiver's next advice, stayed at OFDM rungs 5–9. **That
step counts the silence twice**: the copies a silent transmission carried are judged at the next
answer anyway, delivered or not, and a slot goes silent mostly for a lost control frame or a
stalled station, which a slower data rung does not cure. It was taken out (`bd81150`, model,
port and replay; R9 still steps the engine's own recommendation) and run again (§3.13).

### 3.13 Without the step down for silence: rejected (2026-10-24)

`bd81150` on the same 69 runs: K1(a) **+31.83 %** (priced, with the step: +21.30 %); K1(b)
`80m-nvis-disturbed-500` **0.66** (1.05), `80m-slow-machine-500` 0.87 (0.76); K3 5, 0 and 0
apart; `80m-agc-carrier-outside-500` one run of three (two). The trace of the disturbed NVIS
path, draw 0: without the step C1 at the sender sent its first three transmissions at OFDM rung 5
(18 frames) before an answer judged them, and went back to rung 5 four times; with it, one
transmission at rung 5 and the floor from 41 s (198 against 317 s for the message). On these
paths a silent slot is what OFDM failing looks like — the copies are judged at the next answer,
but the next answers are what fails — so the step is the timely signal, and the argument of §3.12
was wrong. **Reverted** (`git revert`, the commit after `65b9576`). The slow machine's 0.76 stays
open: there the silences are the station's stalls, and the same step that saves the NVIS paths
puts the session on the floor after each one. K6 without the step was ×1.511 (worst 0.688); with
it, ×1.455 (§3.11).

### 3.14 Where C1 at the sender stands (2026-10-24)

Priced per exchange, with the step for silence (`f9976ad`), against `03-candidates.md` §3:

| criterion | result | verdict |
|---|---|---|
| K1(a) median regret ≤ half the current's | +21.30 % against +22.62 % | **passes** |
| K1(b) no scenario under 0.85 of the current | `80m-slow-machine-500` 0.76 (149 / 63 / 81 against 119 / 91 / 106) | fails |
| K3 start independence, ≤ 1 rung apart each draw | 5, 1, 1 (the current: 2, 1, 2) | fails |
| K4 completion ≥ the current's | 68 of 69 (`80m-agc-carrier-outside-500` draw 0) against 69 | fails |
| K6 field replay, sum ≥ current, no session under 0.9 | ×1.455; WC4Y 2026-10-22 at 0.688 (open loop, §3.11) | fails |
| K2, K5 | not run for this form | — |
| K7 | 7 tunables of its own plus the inherited SNR smoothing; 3 of the 5 rules removed (§3.11's note on what the cap counts) | the author's reading |

The three failures on the harness have traces: the slow machine is the step for silence meeting
a station's stalls (§3.12, §3.13); the AGC carrier's undelivered message and K3's draw 0 are both
a try of one frame — one lucky decode put seventeen frames on OFDM rung 5 on a 7 ms echo, two
unlucky ones shut rung 7 from a low start for the session (§3.10, §3.12). They are the author's to
decide, case by case (`03-candidates.md` §4).
