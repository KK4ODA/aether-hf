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

(To be filled.)
