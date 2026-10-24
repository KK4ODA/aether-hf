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
* **Where it stands**: §3.19's form — C1 at the sender with every fix of §3.10–§3.18 — over
  six draws completes every run and passes K1(b) and K7, but **fails K1(a) clearly** (+26.80 %
  against +19.41 %, §3.20); K3 and WC4Y's file rate are near misses. The fix of K3's dip (§3.21,
  §3.22) cost three floor-top paths 14–29 % and did not pass K3: taken out by the author's call.
* **Next**: K2, K4 and K5 for this form (§3.23); K1(a)'s miss is accepted as recorded by the
  author's call (§3.20).

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
| … priced, a try decided on three frames (§3.15) | **+21.85 %** | 1 (slow machine 0.81) | 6, 0, 2 | **0** (72 runs) |
| … and a try across the edge takes the transmission (§3.16) | +23.46 % | 1 (slow machine 0.77) | 2, 1, 2 | 0 (72 runs) |
| … and the wait between tries in air at the rung tried (§3.17) | +24.41 % | 1 (polar path 0.81; slow machine 0.87) | 6, 1, 2 | 0 (72 runs) |
| … and a slave on its own floor rung asks for floor slots (§3.19) | +23.76 % | 1 over 3 draws (polar path 0.81); **none over 6** | 2, 1, 1, 0, 1, 1 (6 draws) | **0** (144 runs) |
| the same, K1(a) over six draws (§3.20) | **+26.80 %** against +19.41 % | none | — | — |
| … and the try past the dip (§3.21, `2db2fb8`) | +40.25 % (6 draws) | 3 (weak wide Test 0.23, weak both ways 0.52, polar path 0.82); WC4Y's path 0.31 | 0, 1, 2, 3, 1, 1 | 1 (the AGC carrier, draw 0) |
| … and the try waits for data to cut (§3.21, `ca7587c`) | +30.18 % (6 draws) | 3 (weak wide Test 0.53, weak both ways 0.74, polar path 0.82); WC4Y's path 0.49 | 0, 1, 2, 1, 1, 2 | 0 (144 runs) |
| … and a failed try holds the family back (§3.22, `55e5139`; 6 scenarios and the start pair, 3 draws) | — | 2 of 4 run (polar path 0.76, weak both ways 0.79); WC4Y's path 0.71 | 3, 0, 2 | 0 (24 runs) |

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

### 3.15 A try decided on three frames (2026-10-24)

The author's answer 3 (`03-candidates.md` §4): a try of the rung above runs until its own frames
outweigh the prior — more than `prior_frames`, so three — or twice the transmissions that takes,
and a rung above becomes choosable only once its frames outweigh its prior (`995ff5e`; no new
parameter). The 69 runs and WC4Y's path (`80m-floor-top-start-500`, the author's answer 5), 72:

* **Completion**: 72 of 72; `80m-agc-carrier-outside-500` three of three (328, 336 and 371 s).
* **K1(a) passes**: +21.85 %.
* **K1(b)**: `80m-slow-machine-500` 0.81 (126 / 43 / 86 against 119 / 91 / 106; draw 1's 43 is
  the floor after a stall again); every other scenario at 0.87 or over.
* **K3** fails: 6, 0, 2.
* **WC4Y's path** (the current's Test: message 73.9 / 81.9 / 81.9, file 92.7 / 92.6 / 104.5
  bit/s): message 91.6 / 81.9 / 91.6, file 77.2 / 92.6 / 89.4 — the message ×1.12, the file ×0.96
  (medians).

**K3's draw 0, traced**: from the low reading C1 at the sender sat on rung 4 (the floor's top)
from 30 to 188 s — 27 frames — without one try of rung 5, while the receiver advised 6 to 10. A
transmission goes in the family of its oldest unacknowledged frame, and on this path a floor frame
was always in flight, so the try of OFDM — a whole transmission in the other family — never went
and expired unmeasured, again and again. **A defect** of the try, not the path: a try across the
edge now takes the transmission and the floor frames in flight wait it out (`25995cb`, model and
port; the review's vectors gain a session with a floor frame always in flight, and removing the
exception in the port fails them). On the harness next (§3.16).

### 3.16 The try across the edge takes the transmission (2026-10-24)

`25995cb`, the same 72 runs:

* **Completion**: 72 of 72.
* **K1(a)**: +23.46 % (§3.15: +21.85 %) — over the line by 0.84 points. Nothing changed for the
  scenarios whose tries stay in one family; the moves are those a change of timing makes on three
  draws (`80m-flutter-500` +6 → +28 %, `40m-good-2300-rtty-crashes` +28 → +22 %).
* **K1(b)**: `80m-slow-machine-500` 0.77.
* **K3**: 2, 1, 2 — the current controller's own (2, 1, 2), still failing.
* **WC4Y's path**: message 91.6 / 91.6 / 91.6, file 98.0 / 98.0 / 89.4 bit/s against the current's
  73.9 / 81.9 / 81.9 and 92.7 / 92.6 / 104.5 — ×1.12 and ×1.06 (medians), **at least as well**:
  the author's condition for excusing K6's WC4Y session is met.

**K3's draw 0 and the slow machine, traced, have one cause.** Draw 0's low start is the same as in
§3.15: rung 4 for 27 frames, and the try of rung 5 at 188 s — after exactly six transmissions,
the try's wait (`probe_every`). The wait counts transmissions, and a transmission on the floor is
five 5.36 s frames (≈ 27 s) where one on OFDM is six 1.05 s frames (≈ 8 s): six transmissions are
160 s on the floor and under a minute above it. The slow machine is the called station's stalls —
the sender cannot tell them from a path that loses frames — and what costs is the way back: each
stall puts the session on the floor, and the next try of OFDM waits six floor transmissions.

### 3.17 The wait between tries in air (2026-10-24)

The author's answer of the same day: the wait is `probe_every` full transmissions *at the rung
being tried*, in seconds of air (`275209a`; no new parameter). Building it found the model and the
port parting on a tie — six OFDM transmissions' air is the wait exactly, and Python's `sum()`
compensates where the port's does not — so the model now adds in order and the comparison allows
1e-9 s. The 72 runs:

* **Completion**: 72 of 72. **WC4Y's path**: message ×1.12, file ×1.06 (98.0 / 95.4 / 98.0).
* **K1(b)**: `80m-slow-machine-500` **0.87** (93 / 103 / 76 against 119 / 91 / 106) — over the
  line; `20m-polar-path-2300` **0.81** (160 / 143 / 172 against 198 / 198 / 172) — under it.
* **K1(a)**: +24.41 % (§3.16 +23.46, §3.15 +21.85).
* **K3**: 6, 1, 2.

**The traces.** *K3's draw 0*: the first try of OFDM now came at 88 s (it had at 188), a whole
transmission at rung 5 — and C1 judged it a loss and went back to rung 4, while the receiver,
reading the try's frames, advised rung 9 and then 11. Both stations' control frames were ordinary
all session, so the price was fair: per exchange the 500 Hz first OFDM rung carries 128 bit/s with
nothing lost against the floor's top's 125, and on this path it lost more than that. C1 tries only
the rung directly above, and the first OFDM rung is a dip: the rungs over it carry 181 to 780. It
sat at a local best. *The polar path's draw 1*: OFDM does not carry there; in §3.16 C1 stayed on
the floor's top (rung 5) for the whole message (198 bit/s), here its tries of rung 7 came sooner,
one was judged a gain, the transmissions after it went unanswered and the step for silence put it
on rung 3 (143 bit/s). How often to try OFDM from the floor is a trade between these two paths.

### 3.18 Six draws (2026-10-24)

The author chose to measure before changing anything else: draws 3–5 (seeds +3000…+5000) of the
24 scenarios for §3.17's form and for the current controller (master `144a384`, its default),
judged over six draws against the current. K1(a) stays on draws 0–2 (it needs the fixed rungs'
runs of every draw, some twelve hours more).

| scenario | current, 6 draws | C1 at the sender, 6 draws | ratio of medians |
|---|---|---|---|
| `quick-awgn-2300` | 1263 ×6 | 2182 / 2182 / 2182 / 2000 / 2182 / 2182 | 1.73 |
| `40m-good-throughput-2300` | 2327 / 2286 / 2133 / 2207 / 2000 / 2510 | 1542 / 2723 / 2169 / 2133 / 2169 / 2370 | 0.97 |
| `80m-strong-moderate-500` | 151 / 260 / 244 / 270 / 315 / 232 | 192 / 237 / 348 / 168 / 321 / 235 | 0.94 |
| `20m-dx-500` | 146 / 151 / 144 / 175 / 136 / 150 | 209 / 170 / 108 / 185 / 181 / 152 | 1.19 |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 / 600 / 706 / 750 | 800 / 706 / 800 / 800 / 923 / 857 | 1.20 |
| `80m-qsb-dropout-500` | 146 / 158 / 142 / 167 / 185 / 143 | 167 / 177 / 169 / 169 / 205 / 152 | 1.11 |
| `80m-weak-500` | 70 / 67 / 65 / 56 / 62 / 67 | 70 / 77 / 82 / 74 / 86 / 77 | 1.17 |
| `80m-flutter-500` | 129 / 120 / 127 / 152 / 115 / 130 | 128 / 162 / 150 / 105 / 151 / 130 | 1.09 |
| `15m-equatorial-flutter-2300` | 387 / 211 / 211 / 211 / 200 / 235 | 500 / 211 / 211 / 211 / 200 / 364 | 1.00 |
| `80m-nvis-evening-500` | 41 / 25 / 26 / 49 / 52 / 34 | 39 / 23 / 44 / 52 / 46 / 39 | 1.10 |
| `80m-thunderstorm-agc-2300` | 591 / 535 / 616 / 466 / 516 / 526 | 505 / 622 / 617 / 708 / 735 / 720 | 1.25 |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 / 66 / 70 / 91 | 78 / 74 / 76 / 70 / 60 / 91 | 1.00 |
| `20m-polar-path-2300` | 198 / 198 / 172 / 186 / 198 / 198 | 160 / 143 / 172 / 186 / 198 / 198 | 0.90 |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 / 171 / 176 / 168 | 175 / 175 / 175 / 164 / 176 / 175 | 1.03 |
| `40m-good-2300-rtty-crashes` | 425 / 442 / 496 / 427 / 455 / 674 | 484 / 601 / 662 / 684 / 191 / 478 | 1.21 |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 / 68 / 89 / 75 | 71 / 86 / 73 / 80 / 86 / 76 | 1.07 |
| `80m-nvis-disturbed-500` | 38 / 44 / 36 / 42 / 43 / 34 | 40 / 44 / 30 / 30 / 39 / 24 | 0.86 |
| `40m-moderate-2300-vara-qrm` | 842 / 390 / 667 / 780 / 640 / 627 | 1103 / 593 / 711 / 842 / 711 / 604 | 1.09 |
| `80m-lopsided-acks-500` | 77 / 76 / 83 / 87 / 104 / 58 | 103 / 82 / 106 / 102 / 106 / 97 | 1.28 |
| `80m-slow-machine-500` | 119 / 91 / 106 / 99 / 88 / 69 | 93 / 103 / 76 / 70 / 118 / 80 | 0.91 |
| `80m-floor-top-start-500` | 93 / 93 / 104 / 87 / 99 / 87 | 98 / 95 / 98 / 93 / 99 / 77 | 1.04 |

(The current's draws of WC4Y's path 0–2 are the K2 run's.)

* **K1(b) passes over six draws**: no scenario under 0.85 — the lowest `80m-nvis-disturbed-500`
  0.86, `20m-polar-path-2300` 0.90 and `80m-slow-machine-500` 0.91; fourteen of 21 over 1. Over
  three draws the polar path had read 0.81 and the slow machine 0.76–0.87: three draws were too
  few to judge a scenario by.
* **K3** over six draws: C1 at the sender 6, 1, 2, 4, 1, 1 apart; the current 2, 1, 2, 2, 2, 0.
  Both fail; the current's would not have passed either.
* **Completion**: 143 of 144 against 144. The one, `15m-equatorial-flutter-2300` draw 4: the
  called station's 300-byte reply never went. **A defect**: the master sizes a slave's slot in
  frames of the rung *it* recommends; C1 at the slave had chosen a floor rung of its own and asked
  for frames of it, was granted as many OFDM frames' air, which no 5.36 s floor frame fits, and
  sent nothing in any slot for 600 s (the current controller's slave sent the reply at rungs 7–8).
  The answer's `data_floor` bit, with no data after it, already tells the master the slave's data
  can go only on the floor; C1's own floor choice now sets it too (`91e6144`, no wire change; a
  model test reproduces the deadlock and fails without the change; the review's vectors change and
  the port without the clause fails them). That form on all six draws is running.

### 3.19 C1 at the sender, all fixes, over six draws (2026-10-24)

`91e6144` on the 24 scenarios, six draws (144 runs), against the current controller's six:

| criterion | result | verdict |
|---|---|---|
| Completion | 144 of 144 (the current: 144) | passes |
| K1(a), draws 0–2 | +23.76 % against a line of +22.62 % | **near miss**, 1.1 points |
| K1(b), six draws | no scenario under 0.85; lowest `80m-nvis-disturbed-500` 0.86, `20m-polar-path-2300` 0.90; `80m-slow-machine-500` **1.13**; 13 of 21 over 1 | passes |
| K3, six draws | 2, 1, 1, 0, 1, 1 apart (the current: 2, 1, 2, 2, 2, 0) | **near miss**, one draw of six |
| WC4Y's path (K6's condition), six draws | message ×1.11 (91.6 / 81.9 / 91.6 / 81.9 / 81.9 / 91.6 against 73.9 / 81.9 / 81.9 / 81.9 / 68.1 / 73.9), file ×0.98 (77.2 / 104.5 / 89.4 / 92.6 / 104.5 / 89.4 against 92.7 / 92.6 / 104.5 / 87.4 / 98.7 / 87.4) | **near miss** on the file |
| K2, K4's overlaps, K5 | not run for this form | — |
| K7 | eight of its own (§3.11's count, the author's answer 6) | passes |

Changed from §3.18 by the slave's fix: the slow machine 0.91 → 1.13, `40m-good-2300-rtty-crashes`
1.21 → 1.38, `80m-flutter-500` 1.09 → 0.97; nothing that had passed fell under 0.85. The three
near misses are the author's, case by case. K1(a) is the median of twenty scenarios' medians over
three draws; on draws alone the current controller's own reads +36 … +50 % (`03-candidates.md`
§5.1).

### 3.20 K1(a) over six draws (2026-10-24)

The author's call on §3.19's K1(a) near miss: measure draws 3–5 too. The fixed rungs ran on draws
3–5 (366 runs, the rungs of draws 0–2 and the current's own), so K1(a) is judged as registered —
the median over the twenty scenarios of each scenario's median regret — over six draws. The line
is half the current controller's own: **+38.81 %** over six draws, so **+19.41 %**.

| scenario | best fixed rung by draw | the current's regret | §3.19's form | ratio of medians |
|---|---|---|---|---|
| `quick-awgn-2300` | 16 / 16 / 16 / 16 / 16 / 18 | +73 % | +0 % | 1.73 |
| `40m-good-throughput-2300` | 19 / 17 / 17 / 18 / 18 / 18 | +25 % | +34 % | 0.97 |
| `80m-strong-moderate-500` | 13 / 13 / 13 / 14 / 13 / 14 | +35 % | +56 % | 0.94 |
| `20m-dx-500` | 11 / 11 / 10 / 11 / 13 / 11 | +45 % | +30 % | 1.19 |
| `20m-dx-moderate-2300` | 14 / 11 / 14 / 11 / 15 / 14 | +44 % | +20 % | 1.20 |
| `80m-qsb-dropout-500` | 11 / 13 / 11 / 10 / 10 / 13 | +35 % | +17 % | 1.11 |
| `80m-weak-500` | 3 ×6 | +27 % | +0 % | 1.17 |
| `80m-flutter-500` | 9 / 9 / 7 / 9 / 10 / 9 | +26 % | +32 % | 0.97 |
| `15m-equatorial-flutter-2300` | 14 / 14 / 14 / 14 / 11 / 14 | +255 % | +255 % | 1.00 |
| `80m-nvis-evening-500` | 1 / 1 / 2 / 2 / 2 / 1 | +7 % | −5 % | 1.15 |
| `80m-thunderstorm-agc-2300` | 14 / 15 / 14 / 14 / 14 / 14 | +76 % | +46 % | 1.24 |
| `80m-weak-both-ways-ladder-500` | 4 / 4 / 4 / 4 / 3 / 4 | −3 % | +4 % | 0.98 |
| `20m-polar-path-2300` | 9 ×6 | +43 % | +60 % | 0.90 |
| `80m-weak-wide-test-2300` | 5 ×6 | +4 % | +0 % | 1.03 |
| `40m-good-2300-rtty-crashes` | 14 / 16 / 14 / 14 / 14 / 16 | +62 % | +22 % | 1.38 |
| `80m-poor-500-pactor-signalink` | 9 / 4 / 4 / 4 / 4 / 4 | +55 % | +40 % | 1.07 |
| `80m-nvis-disturbed-500` | 2 / 2 / 2 / 2 / 1 / 2 | +4 % | +23 % | 0.86 |
| `40m-moderate-2300-vara-qrm` | 14 / 15 / 14 / 14 / 14 / 14 | +59 % | +45 % | 1.09 |
| `80m-lopsided-acks-500` | 4 / 9 / 4 / 4 / 4 / 4 | +25 % | +0 % | 1.31 |
| `80m-slow-machine-500` | 7 / 11 / 11 / 9 / 11 / 9 | +49 % | +51 % | 1.13 |

**K1(a) fails over six draws: +26.80 % against a line of +19.41 %**, 7.4 points — not a near miss.
On draws 0–2 alone the same form had read +23.76 % against +22.62 %: the three new draws moved the
current controller's own regret more (+45.2 → +38.8 %) than the candidate's. Where the candidate
is furthest behind the best fixed rung is a climb on a strong path: `80m-strong-moderate-500`
(+56 %, settles at rungs 10–12 where 13–14 carry most), `20m-polar-path-2300` (+60 %),
`40m-good-throughput-2300` (+34 %), `80m-slow-machine-500` (+51 %), and `80m-nvis-disturbed-500`
(+23 % against the current's +4 %). In eight draws of five scenarios the best fixed rung is the
highest the oracle ran (`quick-awgn-2300` draw 5 at 18, `20m-dx-500` draw 4 at 13,
`80m-qsb-dropout-500` draws 1 and 5 at 13, `40m-good-2300-rtty-crashes` draws 1 and 5 at 16,
`80m-strong-moderate-500` draws 3 and 5 at the ladder's top, 14), so those regrets are a lower
bound for both controllers.

**The author's call: accept the miss and go on** — K1(a) stands as a recorded miss of this form,
untraced, and K2, K4 and K5 are run for it so that the decision ADR weighs it against everything
else.

### 3.21 The try past the dip, and what it broke (2026-10-24)

The author's call on §3.19's K3 near miss: fix the dip. From the floor's top a try now goes to the
rung of the OFDM family the reading rates best, not to the first OFDM rung (`_try_target`,
`_try_reach`; a try under way goes on while its rung is in reach; `2db2fb8`, model, port, tests
and the review's vectors). That form (v10) on the 24 scenarios × six draws:

* **One run did not complete** (`80m-agc-carrier-outside-500` draw 0, the message not delivered
  in 300 s of air) and three scenarios fell far under the current: `80m-weak-wide-test-2300` 0.23,
  WC4Y's `80m-floor-top-start-500` 0.31 (file ×0.31), `80m-weak-both-ways-ladder-500` 0.52; K1(a)
  +40.25 %, K3 0, 1, 2, 3, 1, 1.
* **The trace** (a scratch daemon logging each header it sent): the session's last block was
  re-encoded onto the floor while a try of OFDM was pending and the queue was empty; a try across
  the floor's edge takes the whole transmission (§3.16), so every transmission went out with no
  data frame — nothing of the try's family was left to cut, and the floor block could not ride
  with it — and the try never ended. The master sent empty headers for 160 s.
* **The fix** (`ca7587c`, v11): the try takes the transmission only when a new block can be cut
  at its rung (`_can_cut`: data queued, no measurement asked, room in the window); otherwise the
  transmission is the floor's frames. A model test builds the state and fails on the old rule; a
  new vector session (`edge-at-the-end`) reaches it, and the port with the old rule fails
  `granted_vectors.rs`.

v11 on the 24 scenarios × six draws: **144 of 144 complete**, but the regressions stay —
`80m-weak-wide-test-2300` 0.53, WC4Y's path 0.49 (message ×1.11, file ×0.49: 39.3 / 41.1 / 57.5 /
50.4 / 41.3 / 57.1 against 92.7 / 92.6 / 104.5 / 87.4 / 98.7 / 87.4),
`80m-weak-both-ways-ladder-500` 0.74, `20m-polar-path-2300` 0.82; K1(a) +30.18 %; K3 0, 1, 2, 1, 1,
2 (draw 0 now 10 and 10, draws 2 and 5 at 9 and 11).

**Traced** (the port's controller logging its state after every transmission it is fed, on
`80m-weak-wide-test-2300` draw 0, which repeated its 111.1 bit/s): from the floor's top (rung 5,
165 bit/s over its exchanges) every OFDM rung's estimate was nearly nothing — the prior at the
floor's reading of −3 dB — but each one's lossless rate beat rung 5's, so each was due. The try
went to rung 8 (2 of 6 decoded) and was judged a loss, which made rung 8 wait twice as long; but
rungs 9, 10 and 11 each had their own wait, already over, so the next transmission tried 9, then
10, then 11, then 12 — **a cascade of tries**, each cutting new blocks at its own rung, whose
copies stay at that rung. The session spent 85–196 s on rungs 8–11 delivering 0–98 bit/s. A
second effect rode on it: the OFDM family's offset fell to its clamp, −5 dB. The prior gave every
OFDM frame almost no chance at the floor's reading, so each decode moved the offset by nearly the
whole gain toward optimism and each failure by nearly nothing; once rung 9 had frames of its own
the optimistic prior carried it over rung 5 (choice 9 at two points). The same pattern is in the
rung sequences of WC4Y's path and the weak-both-ways ladder. Under §3.19's form the first OFDM rung
alone was in reach, its lossless rate did not beat the floor's top, and no try went.

### 3.22 A failed try holds the family back, and the dip fix judged (2026-10-24)

The cascade is a defect of how the dip fix was built, not of the decision: the author's call was
one try, at the rung the reading rates best. Now a failed try across the floor's edge doubles the
wait of every rung of the family above (`55e5139`, model and port; two scripted controller
traces on the floor's top reach the rule, and the port with the old rule fails them). The same
rule in a scratch daemon, on the six scenarios where v11 lost or that climb from the floor, and
the start pair, draws 0–2 (24 runs, all complete), against §3.19's form (no dip fix) and v11:

| scenario | the current | §3.19's form | v11 | a failed try holds the family back |
|---|---|---|---|---|
| `80m-floor-top-start-500` (WC4Y's path) | 93 / 93 / 104 | 77 / 104 / 89 ×0.96 | 39 / 41 / 58 ×0.44 | 53 / 66 / 86 ×0.71 |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 78 / 74 / 76 ×0.97 | 51 / 28 / 76 ×0.65 | 57 / 62 / 76 ×0.79 |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 | 175 / 175 / 175 ×1.04 | 111 / 89 / 141 ×0.66 | 144 / 132 / 156 ×0.86 |
| `20m-polar-path-2300` | 198 / 198 / 172 | 160 / 143 / 172 ×0.81 | 163 / 162 / 162 ×0.82 | 163 / 143 / 150 ×0.76 |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 | 800 / 706 / 800 ×1.27 | 800 / 706 / 800 ×1.27 | 800 / 706 / 800 ×1.27 |
| `quick-awgn-2300` | 1263 ×3 | 2182 ×3, ×1.73 | 2182 ×3, ×1.73 | 2182 ×3, ×1.73 |
| K3, settled (low / high) | 2, 1, 2 apart | 4 / 6, 9 / 10, 9 / 10 | 10 / 10, 10 / 11, 9 / 11 | 7 / 10, 10 / 10, 9 / 11 |

**What is left is the price of the try itself.** On `80m-weak-wide-test-2300` draw 0 (the port's
state logged after every transmission): one try of rung 8, six frames, two decoded, judged a
loss, and no other try followed; but the four blocks it had cut stay at rung 8 for their copies,
and they took three more OFDM transmissions (85–126 s) before the session was back on the floor's
top — 144 bit/s against §3.19's 175, where no try of OFDM ever went. On these paths a try of OFDM
from the floor's top costs a whole transmission and its leftovers, and it bought nothing on K3:
3, 0, 2 apart against §3.19's 2, 1, 1. **The dip fix, as built, fails K1(b) on three paths and
does not pass K3.**

**The author's call: take it out** (`b53d0e6`, reverting `2db2fb8` and `55e5139`). C1 at the
sender is §3.19's form again — a try from the floor's top goes to the first OFDM rung, and only
when its lossless rate beats the top's estimate — with v11's guard kept (a try across the edge
takes the transmission only when a block can be cut at its rung; the `edge-at-the-end` vector
session still reaches it). K3's draw-0 dip stays a near miss.
