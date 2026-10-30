# Gate 2: candidates and criteria

*The rate controller's review (`00-brief.md` §6), 2026-10-24. The candidates for Gate 3, what
each removes and adds, and the criteria they will be judged by — registered here, with the
current controller's own numbers (§5), before any candidate is run. The author agreed the
criteria on 2026-10-24 (§4).*

## 0. Summary

* **Five candidates**, each against the current controller:
  * **C0, the current controller simplified**: the baseline any redesign must beat. Its
    hysteresis, dwell, decay schedule, re-seed and the first rung's steps in hand are each
    removed in turn, and together; whatever goes with no loss is gone.
  * **C1, choose by measured delivery**: per rung, the data delivered per second of the air its
    frames took (every copy charged, a frame never detected charged as air spent), with an SNR
    prior for rungs not yet measured; exploration on a frame of a transmission sent anyway.
  * **C2, outer-loop adaptation to a derived target**: one offset per family, moved per frame,
    its steps derived from a target error rate that is itself derived from the rungs' rate ratios
    and HARQ; the rung chosen by expected delivery from the model's error curves.
  * **C3, where the decision lives**: C1 and C2 each at the receiver (as today) and at the sender
    (which sees its acknowledgements, its queue and its own air).
  * **C4, the start**: the first rung from the prior, with the first transmission's exploration,
    in place of two rungs in hand.
  * A sixth, **C5, a soft reading as the prior**, waits on an instrument check (§2.6).
* **Seven criteria with numbers** (§3), agreed by the author (§4): regret against the oracle,
  no scenario slower, start independence, completion and no new overlap, the real-modem link
  bench, the field replay, and fewer parameters and rules.
* **What it costs to judge**: about 20 scenarios × 3 draws × (the current controller + each
  candidate) on the harness, the full scenario matrix × 3 draws for the best two, the link bench,
  and the field replay; the oracle's fixed-rung runs are made once (§5).

## 1. What the candidates share, and what they may change

### 1.1 Fixed for every candidate

* The ladders, their frames and the HARQ (RV 0, 0, 2, 3, combining, re-encoding and splitting a
  stranded frame).
* Granted slots (ADR-0069): every transmission states its count; a frame never detected is
  countable at whichever station decides.
* The regulatory ceiling (`set_ceiling`), `max_mode`, the echo limits as rung exclusions
  (ADR-0087), and the Test's ladder of measurement frames (ADR-0077).
* The busy detector and the receiver's detection (`00-brief.md` §2: not the controller).

### 1.2 What the controller is today, counted

The current controller (`model/aether_model/link/rate.py`) and the engine's rules that feed it or
act in its place (`granted.py`), the inventory a candidate is measured against:

| part | where | what it does |
|---|---|---|
| AWGN thresholds at 10 % frame error | `rate.py` tables | the SNR each rung needs |
| `margin_db`, `min_margin_db`, `max_margin_db` | `RateController` | the learned margin and its bounds |
| `up_step_db`, `max_jump_db` | | the widening on a failure |
| `down_step_db`, `decay_growth`, `max_down_step_db`, `decay_every` | | the decay after a failure |
| `up_hysteresis_db`, `up_dwell`, `max_up_step` | | the climb's headroom, wait and stride |
| `first_mode_back` | | the start's steps in hand |
| `partial_share` | | a failure in part (ADR-0084) |
| `reseed_margin_db` | | re-seeding past a floor reading (ADR-0016) |
| `floor_modes`, `_at_edge` | | the floor's edge rule (ADR-0084, ADR-0093) |
| `echo_limit`, `echo_weight`, `echo_min_snr_db` | | the echo exclusions (ADR-0087) |
| the 0.7/0.3 SNR smoothing | | hard-coded |
| what counts as a failure (trusted, RV 0 or 3, or combined) | `_observe` | ADR-0020 |
| the sender's-choice rule (`_asked`) | `_observe`, `_recommend` | ADR-0020 |
| the slave's silence rule (`observe(None, 0, 1)` after two silent slots) | `_close_slot` | P9-7 |
| the master's silence step (`silence_step` 2) | `_back_off` | R9 |
| the seeds at the call and the acceptance, `first_mode`, `_first_move` | `_on_connect` | ADR-0008, -0016, -0066 |
| `reencode_early_after`, `reencode_hopeless_db`, `_peer_rung` | `_hopeless`, `_split_rung` | ADR-0038, -0086 |

That is 19 parameters in the controller (`initial_mode` aside), four engine settings that act
on rates (`silence_step`, `reencode_early_after`, `reencode_hopeless_db`, `initial_mode`), and six
engine rules. The echo exclusions, the ceiling and the re-encode and split rule are kept by every
candidate (they choose *which* frames go, not how fast the link runs); the other five rules, and
the parameters, are what a candidate may replace.

### 1.3 The inputs every candidate can have

Per frame at the receiver: detected or not (the header's count), decoded alone, combined and
rescued (the engine's own note, `DataHeard`), the RV and rung read off the chips, trusted, the
SNR, the echo for a floor frame. Per transmission at the sender: which frames were
acknowledged, which went unacknowledged, its own air, its queue. Per session: the call's reading.
Nothing new on the wire is needed for C0, C1-R, C2 or C4; C1-S needs none either (§2.3).

## 2. The candidates

### 2.1 C0 — the current controller, simplified

**What it is.** The current controller with parts removed, each alone and then all that pass
together. The parts, and the reason each is a candidate for removal:

1. `up_hysteresis_db` → 0: the climb's extra 1.5 dB makes its target 0 % (`02-survey.md` §2.2).
2. `up_dwell` and `max_up_step` → none: one clean burst and two rungs a step were tuned on the
   pipe (ADR-0007, ADR-0008).
3. The decay schedule (`decay_every`, `decay_growth`, `max_down_step_db`) → a fixed step per clean
   burst derived from a target, OLLA's Δdown = Δup·T/(1 − T) with T = 10 %.
4. `reseed_margin_db` and the re-seed → none: a floor seed is a lower bound, and the climb
   corrects it if the climb works.
5. `first_mode_back` → 0: the start's steps in hand (ADR-0008, ADR-0066).
6. `partial_share` → none, with 3: a failure in part was ADR-0084's patch for a margin that
   ratcheted.

**Removes**: up to nine parameters. **Adds**: nothing.

**Why it is first**: it is the cheapest change and the baseline; if C0 with 1–3 removed already
meets the criteria, the redesign must beat that, not today's controller.

### 2.2 C1 — choose by measured delivery

**What it measures.** Per rung *r*, decayed over the session (half-life *H* frames sent at *r*):
the bytes delivered by frames sent at *r* (a block is credited to the rung of the copy that
delivered it, combining included) and the air the frames sent at *r* took (every copy, every
frame never detected, every frame that failed). The rung's measured delivery is
*D_r* = bytes / air.

**The prior.** A rung not measured, or measured little, takes the delivery the model predicts at
the session's smoothed reading: its payload over its frame time times its probability of
decoding at (reading − AWGN 10 % threshold − a class penalty), from the model's AWGN error curves
(`bench/baselines/phy_fer_*.csv`, `tone_floor.csv`), weighted as *w₀* pseudo-frames. One failure
does not banish a rung, and one lucky frame does not crown one (RRAA's observation of
SampleRate).

**How it decides.** The rung with the highest delivery estimate, among the current rung and its
usable neighbours, and one edge across the floor's boundary (the floor's top and the first OFDM
rung are neighbours both ways).

**How it explores.** A neighbour is tried only if its *lossless* delivery (payload / frame time)
beats the current rung's measured delivery (SampleRate's pruning). The try is one data frame of a
transmission sent anyway, at the neighbour, in the same family (data frames of a family take the
same air): if it fails, its block goes again at the current rung (Minstrel's single try). At most
one probe frame in *P* sent; a failed probe doubles that rung's wait (AARF; Mercury's dead
verdict). Across the floor's boundary, where a frame of the other family cannot share a
transmission, the probe is one whole transmission and its wait starts at *P* exchanges. The
Test's measurement frames (ADR-0077) feed the same estimates.

**Fading and HARQ.** Delivery after combining per second of all the air it took is the quantity
the HARQ literature says to maximise (`02-survey.md` §2.3); the estimate's half-life follows the
path: Minstrel's period, Kille's averaging window.

**Removes**: the learned margin and its nine parameters, the hysteresis, the dwell, the stride,
`partial_share`, the edge rule, the re-seed, `first_mode_back`; the failure-counting rules
(ADR-0020's trusted/RV/combined) become the delivery account; the sender's-choice rule goes (a
frame sent at the neighbour is the probe, its outcome is news of that rung). **Adds**: *H*, *w₀*,
*P*, the class penalty (or a per-rung prior table), and the probe mechanism. Target: four
parameters.

### 2.3 C3 — where the decision lives

C1 and C2 are each built twice:

* **-R, at the receiver** (today's place): it sees every frame, detected or not, decoded alone or
  combined, and recommends in every control frame as now. A single-frame probe needs the sender's
  cooperation: with no wire change the receiver can only recommend a whole transmission at the
  neighbour, so C1-R probes cost a transmission.
* **-S, at the sender**: it sees which frames were acknowledged and which were not (a frame
  never detected is a frame unacknowledged with no copy decoded), its own air, its own queue; it
  can put one frame of a transmission at a neighbouring rung without any wire change (a frame's
  rung rides in its chips, and the header counts frames, not rungs). The receiver's
  recommendation becomes the prior. STANAG 5066 moved the decision this way between editions
  (`02-survey.md` §4.3).

A sender-side decision changes what a recommendation means on the wire only by being advice; the
older build reading it as an order still interoperates. The sender may send faster than the
receiver recommends, one frame in *P* (the author, §4).

### 2.4 C2 — outer-loop adaptation to a derived target

**What it is.** One offset per family (floor, OFDM), moved per *frame* sent: up by Δup when a
first copy failed or was never detected, down by Δdown when it decoded, with Δdown = Δup·T/(1−T)
(OLLA's fixed point, `02-survey.md` §2.2). The target *T* is derived per family, not tuned: the
switch-point error rate between neighbouring rungs, 1 − R_{r−1}/R_r averaged over the family's
neighbours, raised by the decibels the family's combining buys at the RVs sent. The rung chosen
is the one with the highest expected delivery R_r·(1 − PER_r(reading − offset)), reading the
model's AWGN error curves, not a threshold.

**Removes**: the margin's schedule (seven parameters), the hysteresis, the dwell, the stride,
`partial_share`, the edge rule. **Adds**: Δup (one step size; Δdown follows), the curves as a
table. Target: three parameters.

**Why it is here**: it is the conservative redesign — the cellular answer, with the two survey
corrections (a derived target, delivery-maximising choice). If C1's exploration costs more than it
finds on 6–9 s exchanges, C2 is the alternative that explores nothing.

### 2.5 C4 — the start

**Today**: the first rung is the fastest that fits the call's reading with the margin and the
hysteresis, less two rungs (`first_mode_back`), on the floor in decibels (ADR-0066); a floor
reading is a lower bound, re-seeded from the first clean ordinary burst more than 3 dB above it.

**C4**: the first rung is the prior's best (C1) or the expected-delivery best at offset 0 (C2),
with no steps in hand, and C1's first transmission carries a probe at the neighbour above. The
start's test is the scenario pair `80m-start-low-reading-500` / `80m-start-high-reading-500`
(the same Moderate 9 dB path; the call heard 4 dB under and 4 dB over it), made for this gate from
report 16. C4 is judged as part of C1 and C2, not alone.

### 2.6 C5 — a soft reading as the prior (conditional)

If each frame, decoded or not, said how near its cliff it was — the LDPC decoder's margin, or an
effective SNR per rung from the per-carrier noise (EESM/MIESM) — C1's prior would predict the
neighbours and its exploration on the OFDM rungs would mostly go (SoftRate, Halperin et al.). It
is built only if an instrument check first shows it predicting: on the harness's recordings and
the field's, the reading of a frame at rung *r* predicts whether a frame at *r* ± 1 would decode
(the Test's ladders give the answers) to within 2 dB of effective SNR in at least 80 % of frames.
The check is part of Gate 3; C5 is not run before it passes.

## 3. Criteria

Registered before any candidate is run; numbers from the current controller are in §5. "The
current" is the current controller on the same build, the same scenarios and the same draws.

| # | criterion | measured on | pass |
|---|---|---|---|
| K1 | **Regret against the oracle** | the 20 regret scenarios (§5.1) × 3 draws; per draw the best fixed rung; regret = best / candidate − 1; per scenario the median over draws | (a) the median over scenarios of the per-scenario regret at most **half** the current's; (b) no scenario's median delivery more than **15 %** under the current's |
| K2 | **No scenario slower** | every scenario in `bench/scenarios/` × 3 draws | `tools/scenario_trend.py` against the current's three draws on the same build flags none *slower* or *failing*; pooled air time ≤ ×**1.00** of the current's |
| K3 | **Start independence** | the start pair × 3 draws | in each draw the two sessions settle within **one rung** of each other (settled: the rung most of the last half of the master's non-measurement data frames went at) |
| K4 | **Completion and no new overlap** | every scenario × 3 draws | passes per scenario ≥ the current's; radiated overlaps **0** in total |
| K5 | **The real-modem link bench** | `tools/bench_link.py --backend phy --continuous`, `--bandwidth 2300` and `500`, its default grid (AWGN, Good, Moderate, Poor; −4 … +20 dB; 8000 B; three trials) | completion ≥ the current's at every point; median session time ≤ ×**1.05** at every point and ≤ ×**1.00** pooled |
| K6 | **The field replay** | every two-sided session in `field/inbox/` (nine today), replayed open-loop with `tools/rung_reading.py` | at each step where the session measured both the candidate's choice and the current's choice (data or ladder), the candidate's measured delivery summed over the steps ≥ the current's, and no session **10 %** under |
| K7 | **Simpler** | the code | at most **8** tunable parameters where the current has 23 (§1.2); at least **three** of the five replaceable engine rules of §1.2 removed; any rule added names the evidence it rests on |

Notes on the criteria:

* **K1(a)'s half**: the regret's floor is not zero — a controller must find the path, the oracle
  knows it — and half is a gain three draws a scenario tell from noise with room to spare: on
  draws alone the statistic moves between +36 and +50 % about the current's +45 % (§5.1), and
  half of it is +22.6 %. (This note had said half was the *smallest* gain three draws could
  tell; the runs show a smaller one would show too. The criterion is unchanged.)
* **K2 and K4 cover what K1 does not**: the scenarios with hosts, gateways, aborts, VOX and slow
  machines, where speed is not the point but a change can break the session.
* **K6 is open-loop**: it judges decisions on the same evidence, not outcomes; a sender-side
  candidate is replayed on the sender's record of its acknowledgements.
* **A failing criterion stops the step** (`00-brief.md` §7): it is explained with a trace before
  anything changes. A narrow miss goes to the author, case by case (§4).

## 4. The author's answers (2026-10-24)

1. **The criteria** (§3) **are agreed** as written. A candidate that misses one narrowly is
   brought to the author with the criterion, by how much it missed and the trace, and decided case
   by case; it does not pass by rule, and a miss is still explained before anything changes.
2. **The sender may send faster than the receiver recommends**, one frame of a transmission in
   *P*, as C1-S's single-frame probe. C1 is built at both places (§2.3).

The author's answers on C1 at the sender's failures (`04-results.md` §3.14; later the same day):

3. **The speed test**: a try of the rung above is decided on a few frames, not one — the AGC
   carrier's undelivered message and K3's draw 0 were each a try of one frame. Built and run on
   the harness next.
4. **The slow machine** (K1(b), 0.76): decided again with that run's numbers; nothing added for it
   before then (a stall rule would cover only this station's own stalls).
5. **K6's WC4Y session of 2026-10-22** (0.688): excused if the harness agrees — WC4Y's path
   (`80m-floor-top-start-500`) joins the next run, and C1 at the sender must do at least as well
   as the current controller there.
6. **K7** counts only the candidate's own tunable parameters: the kept rules' (the echo
   exclusions' two, the re-encode's two) and the engine's `silence_step` and `initial_mode` are in
   neither count. The candidate stays at eight or under, a parameter derived from a measurement
   (as the prior's slope from the AWGN curves) counting as none.

## 5. The current controller's own numbers

*Filled from the runs of 2026-10-24: the harness with beta.122's daemon (`32bdf64`), three
draws a scenario (the scenario's seed, +1000, +2000).*

### 5.1 Regret against the oracle

Each scenario's measure is the Test's file rate or the largest message's (`tools/rung_oracle.py`
`metric`), for the controller and for every rung pinned (`rate.pin`). Draw 0 ran the coarse rungs
(1, 3, 4, 5, 7, 9, 11, 13 at 500 Hz; 1, 3, 5, 7, 9, 11, 14, 16, 18 at 2 300 Hz) and then the best's
neighbours; draws 1 and 2 ran every rung that delivered at least half draw 0's best, and the best's
neighbours. The best fixed rung is the best of those in that draw; the regret is best / controller
− 1. 284 runs, the 60 of the controller all complete (two runs of fixed rungs that faulted in the
harness after a container restart were run again).

| scenario | the controller, 3 draws (bit/s) | best fixed rung, 3 draws | regret, 3 draws | median |
|---|---|---|---|---|
| `quick-awgn-2300` | 1263 / 1263 / 1263 | 16: 2182 / 16: 2182 / 16: 2182 | +73 / +73 / +73 % | **+73 %** |
| `40m-good-throughput-2300` | 2327 / 2286 / 2133 | 19: 2723 / 17: 2723 / 17: 2612 | +17 / +19 / +22 % | **+19 %** |
| `80m-strong-moderate-500` | 151 / 260 / 244 | 13: 316 / 13: 347 / 13: 415 | +110 / +34 / +70 % | **+70 %** |
| `20m-dx-500` | 146 / 151 / 144 | 11: 219 / 11: 226 / 10: 201 | +50 / +50 / +40 % | **+50 %** |
| `20m-dx-moderate-2300` | 632 / 1000 / 600 | 14: 923 / 11: 1000 / 14: 1714 | +46 / 0 / +186 % | **+46 %** |
| `80m-qsb-dropout-500` | 146 / 158 / 142 | 11: 198 / 13: 224 / 11: 196 | +36 / +42 / +38 % | **+38 %** |
| `80m-weak-500` | 70 / 67 / 65 | 3: 82 / 3: 77 / 3: 82 | +17 / +15 / +26 % | **+17 %** |
| `80m-flutter-500` | 129 / 120 / 127 | 9: 163 / 9: 184 / 7: 149 | +26 / +53 / +17 % | **+26 %** |
| `15m-equatorial-flutter-2300` | 387 / 211 / 211 | 14: 800 / 14: 750 / 14: 750 | +107 / +255 / +255 % | **+255 %** |
| `80m-nvis-evening-500` | 41 / 25 / 26 | 1: 39 / 1: 39 / 2: 41 | −5 / +56 / +56 % | **+56 %** |
| `80m-thunderstorm-agc-2300` | 591 / 535 / 616 | 14: 826 / 15: 994 / 14: 984 | +40 / +86 / +60 % | **+60 %** |
| `80m-weak-both-ways-ladder-500` | 78 / 84 / 72 | 4: 70 / 4: 84 / 4: 93 | −11 / 0 / +30 % | **0 %** |
| `20m-polar-path-2300` | 198 / 198 / 172 | 9: 286 / 9: 296 / 9: 225 | +44 / +49 / +31 % | **+44 %** |
| `80m-weak-wide-test-2300` | 167 / 168 / 175 | 5: 175 / 5: 175 / 5: 175 | +4 / +4 / 0 % | **+4 %** |
| `40m-good-2300-rtty-crashes` | 425 / 442 / 496 | 14: 757 / 16: 772 / 14: 708 | +78 / +75 / +43 % | **+75 %** |
| `80m-poor-500-pactor-signalink` | 66 / 74 / 72 | 9: 110 / 4: 117 / 4: 109 | +67 / +58 / +51 % | **+58 %** |
| `80m-nvis-disturbed-500` | 38 / 44 / 36 | 2: 39 / 2: 43 / 2: 44 | +3 / −2 / +22 % | **+3 %** |
| `40m-moderate-2300-vara-qrm` | 842 / 390 / 667 | 14: 1231 / 15: 821 / 14: 1103 | +46 / +111 / +65 % | **+65 %** |
| `80m-lopsided-acks-500` | 77 / 76 / 83 | 4: 106 / 9: 93 / 4: 106 | +37 / +23 / +28 % | **+28 %** |
| `80m-slow-machine-500` | 119 / 91 / 106 | 7: 127 / 11: 131 / 11: 164 | +7 / +44 / +54 % | **+44 %** |

**The median over the 20 scenarios of the per-scenario median regret is +45.2 %** (mean +51.6 %):
K1(a) asks a candidate for **+22.6 % or less**. A bootstrap over each scenario's three draws
(4 000 resamples) puts the statistic between +36.0 and +49.6 % (5–95 %) on draws alone; the
per-scenario range of the three draws is a median 30 points. A fixed rung beat the controller in
54 of the 60 draws and tied it in three; the controller beat every fixed rung in three
(`80m-nvis-evening-500`'s and `80m-weak-both-ways-ladder-500`'s first draws,
`80m-nvis-disturbed-500`'s second), where a single rung cannot follow the path. K1(b) holds each
scenario's median delivery to at least 85 % of the controller's column above.

### 5.2 The start pair

Three draws each, all six complete (`80m-start-*-reading-500`, seeds 223, 1223, 2223). The call's
reading is the slave's of the call it decoded (a floor frame: a lower bound, ADR-0016); the
settled rung is K3's.

| draw | low start: call read, settled, file bit/s | high start: call read, settled, file bit/s | settled apart |
|---|---|---|---|
| seed 223 | −1.3 dB, rung 9, 186 | +4.3 dB, rung 7, 183 | **2** |
| seed 1223 | +5.8 dB, rung 9, 194 | +9.4 dB, rung 10, 203 | 1 |
| seed 2223 | +3.1 dB, rung 9, 241 | +8.0 dB, rung 7, 195 | **2** |

**The current controller fails K3 in two draws of three**, and the wrong way round: the session
that heard the better call settled lower. Started higher, it climbed into the rungs the path does
not carry, its margin learned the failures, and it stayed two rungs down for the rest of the Test
— report 16's pair (the 13.4 dB call settled one rung *above* the 6.2 dB one there) and
`01-evidence.md` §5 item 3's walk down, reproduced on one path. The 4 dB the scenarios put between
the starts reads as 3.6–5.6 dB at the slave, through the floor frame's own lower bound.
