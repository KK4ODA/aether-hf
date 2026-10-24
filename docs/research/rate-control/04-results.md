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
  default `Current`): C2, the leader on the iteration bench (§2.4). The port is held to the model
  by `tests/data/rate_review_vectors.json` (`tools/make_rate_review_vectors.py`): the granted
  engine's trace sessions with C2 at both stations, two of its own with frames never detected,
  and three direct traces of the controller's inputs. Planted faults (the step size, the frames
  never detected, the echo's choice) each fail it.
* **Next**: the harness, K1 and K3 first (the 20 regret scenarios and the start pair, three
  draws each, against the oracle's runs of Gate 2), then K2 and K4 on the full matrix.

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
