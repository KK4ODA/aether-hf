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

K3 moved from pass to fail between C1 and C1 v2, a change that touches only unmeasured rungs'
priors: on three draws a settled rung is noisy (a change that moves the session's timing puts it
on another stretch of the same fade, `00-brief.md` §8).

