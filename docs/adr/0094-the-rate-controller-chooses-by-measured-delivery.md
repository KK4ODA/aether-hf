# ADR-0094: The rate controller chooses by measured delivery, at the sender

**Status:** accepted for building, 2026-10-25, the author's decision at Gate 4 of the rate
controller's review (`docs/research/rate-control/00-brief.md` §7; the evidence in
`01-evidence.md`, the survey in `02-survey.md`, the candidates and the registered criteria in
`03-candidates.md`, the results in `04-results.md`). **Amended at Gate 5** (2026-10-25, the
author's decisions on `04-results.md` §3.26 and §3.27): C1 chooses at the sender only, the advice
in every control frame stays the current controller's, and a stranded frame comes down under the
lower of the advice and the sender's own choice. Gate 5 builds it into the daemon and proves it;
Gate 6 deletes the prototypes not adopted. For the rung a station's own data goes at, this ADR
supersedes the current controller (ADR-0007, ADR-0008's `first_mode_back`, ADR-0016's re-seed,
ADR-0066's steps in hand, ADR-0084's partial rule, ADR-0093's edge rule); for the advice those
rules stand, with ADR-0020's failure counting and the sender's-choice rule that feed it. No wire
change: the control frame's rung field keeps its meaning (the rung the sender of the frame would
receive the other station's data at best).

## 1. Context

The rate controller (`rate.py`, `rate.rs`) had been adjusted eleven times (ADR-0007, 0008, 0012,
0013, 0014, 0016, 0020, 0066, 0084, 0087, 0093): thresholds, a learned margin with its bounds and
decay schedule, hysteresis, a dwell, a stride, a cap at the floor and its removal, partial
failures, re-seeding, echo limits. Report 15 showed what it could not see: a frame never detected
counted for nothing, a burst rescued by combining counted as clean, and the link bench it was tuned
on delivers every frame and adds 3 dB a copy. Report 16 met the author's condition for a review.

The review asked one thing of a controller: **pick the rung that delivers the most data per
second of air on this path, counting a frame never heard as lost.** On the harness, against the
best fixed rung found in hindsight (`tools/rung_oracle.py`), the current controller left a median
of 38.8 % of the delivery over twenty scenarios and six draws (45.2 % over the first three); on
the field sessions its climb stopped below what the session's own ladder decoded, and it chose the
first OFDM rungs where the floor's top carried more (`01-evidence.md`).

## 2. Decision

Each station chooses the rung its own data goes at with **C1** (`03-candidates.md` §2.2); the
rung it advises the other station's data to go at stays the current controller's.

1. **The account.** Per rung, the data its frames delivered per second of the air they took —
   every copy, every frame never detected — decayed with a half-life of twelve of the rung's own
   frames' air, whatever rung that air went at.
2. **The prior.** For a rung with few frames, the AWGN curve three decibels under the smoothed
   reading, worth two frames; the reading is moved, per family (the floor's, OFDM's), by each frame
   of the family by its surprise (`family_gain_db` 0.5).
3. **The choice.** The best estimate, per second of a whole exchange (the transmission's frames,
   its header, the answer in the families the two stations ask of each other, the gaps), of the
   current rung and the rungs below, and of a rung above once its own frames outweigh the prior.
4. **The try.** The rung above is tried — one frame a transmission at the sender; across the
   floor's edge the whole transmission, when a block can be cut at its rung — when its lossless
   rate beats the current rung's estimate and six full transmissions' air at it has gone since its
   last try; or at once when it is of the current rung's family, rated above it, no try of it has
   failed since it last carried, and every frame of the last transmission at the current rung
   decoded alone. A try is decided on three frames or six transmissions; one that delivered less
   doubles that rung's wait (at most 48 transmissions' air), one that delivered more moves there
   and tries the next at once.
5. **The sender's choice** is its own C1 (C3, at the sender: `03-candidates.md` §2.3). A
   transmission no answer was heard for steps it down two rungs the echo allows (R9).
6. **The advice** in every control frame stays the current controller's (`RateController`, fed
   as before). The sender reads it as before for keeping a header on data above it (ADR-0076),
   and does not take it as its own choice. A stuck frame's re-encode and split targets
   (ADR-0085, ADR-0086) go by the lower of the advice and the sender's own choice when it last
   read an answer (the author's decision at Gate 5): with C1 giving the advice, on ITU NVIS at
   500 Hz the receiver's C1 advised OFDM while the sender's own had moved to the floor, and
   blocks cut at OFDM went fourteen copies over 200 s (`04-results.md` §3.26). With the current
   controller's advice it is what keeps a block in pieces from sticking: the receiver never
   counts a failed piece, so the advice never learns the pieces' rung fails, and without the
   bound such a block went again at that rung for good (§3.28; the author kept it). C1 for the
   advice was built and measured (§3.26, §3.27) and is rejected (§4).
7. **Kept from today:** the AWGN threshold tables, the usable rungs, the echo exclusions
   (ADR-0087), the 0.7/0.3 SNR smoothing, the ceiling, the stuck-frame rules.

Seven tunables of its own (`half_life`, `prior_frames`, `probe_every`, `probe_wait_max`,
`slope_db`, `prior_margin_db`, `family_gain_db`) and the inherited SNR smoothing. **The station
as a whole is not simpler:** the advice keeps the current controller's nineteen and the engine
rules that feed it, so C1 is added beside it (about 350 lines in the model and as many in the
port, beside about 500). K7 was met by C1 in both roles and is missed by what ships; the author
chose the faster form knowing it (2026-10-25), and simplifying the advice is owed (§5).

## 3. What was measured

C1 at the sender, its advice the current controller's (`1e212e7`, `04-results.md` §3.25), against
the current controller, on the criteria registered before any run (`03-candidates.md` §3):

| criterion | C1 at the sender | the current | verdict |
|---|---|---|---|
| K1(a), median regret against the best fixed rung, 20 scenarios × 6 draws (≤ half the current's, +19.41 %) | +27.85 % | +38.81 % | **missed**: the author accepted it as recorded |
| K1(b), no scenario's median delivery 15 % under the current's | lowest 0.88 (`80m-nvis-disturbed-500`) | — | passes |
| K2, every scenario × 3 draws on one build: pooled air ≤ ×1.00 | ×0.995 | — | passes |
| K2, none slower or failing (`scenario_trend.py`) | none failing; `80m-slow-machine-500` and `80m-lopsided-acks-500` slower | — | **missed** |
| K3, start independence, 6 draws (≤ 1 rung apart in each) | 3, 1, 0, 1, 0, 0 | 2, 1, 2, 2, 2, 0 | **missed** in one draw |
| K4, completion; radiated overlaps | 180 of 180; 0 | 180 of 180; 0 | passes |
| K5, the real-modem link bench: completion; pooled time ≤ ×1.00; each point ≤ ×1.05 | 168 of 168; ×0.896; one point ×1.075 (2 300 Hz Poor +16 dB) | 168 of 168 | **missed** at one point |
| K6, the field replay, open loop (≥ the current, no session under 0.9) | ×1.206 summed over nine sessions (§3.19's form: ×1.057 over report 17's five) | — | **missed** at WC4Y 2026-10-22 (0.688); the author's condition, WC4Y's path on the harness at least as well, at the line (message ×1.05, file ×1.00) |
| K7, simpler (≤ 8 tunables; ≥ 3 of the 5 replaceable engine rules removed) | C1's own 8; with the advice the current's, about 26, and 0 or 1 of 5 removed | 23 | **missed**: accepted by the author at Gate 5 |

**The misses, traced** (`04-results.md` §3.20, §3.24, §3.25):

* **The dip.** From the floor's top the first OFDM rung, priced with floor answers, carries less
  than the top even with nothing lost, so no try of OFDM is ever due from there. A session that
  falls to the floor stays there: K3's draw 0 (the low start settled at the floor's top, the high
  one three rungs up) and `80m-slow-machine-500`'s draw 1 (the slave's stalls silenced slots, the
  silence steps put it on the floor at 94 s, and it stayed 200 s). A fix — a try to the OFDM rung
  the reading rates best — was built, its cascade traced and fixed, and taken out by the author's
  call: it cost three floor-top paths 14–29 % and did not pass K3 (§3.21, §3.22).
* **The climb's pace.** A try is one rung a try of three frames; the current controller climbs
  several rungs at once on the reading. On a short strong 2 300 Hz session (8 kB at Poor +16 dB)
  that is five seconds of sixty, K5's one point; across K1(a) the gain on the strong paths was
  matched by losses elsewhere (§3.25).
* **`80m-lopsided-acks-500`** is slower in air only: its transfers were faster (message ×1.43,
  file ×1.15); the longer air is the Test's ladder measuring further.
* **K6's WC4Y session** is judged open loop on the rungs that session happened to measure (§2.5,
  §3.11); on the harness's model of the same path the candidate is at the current's rate.

## 4. What was rejected, and why

* **C2**, one offset per family toward a target error rate derived from the rungs' rates (§3.1):
  K1(a) +30.5 % on three draws, `80m-nvis-disturbed-500` at 0.58 of the current.
* **C0**, the current controller with its hysteresis, dwell, stride, decay schedule, re-seed,
  steps in hand and partial rule taken out (§3.2): +46.7 %, eight scenarios under 0.85.
* **C1 at the receiver alone** (§3.4–§3.8): K1(a) +22.55 % on three draws, but K2 failed on the
  NVIS paths and the AGC carrier lost two messages; it was carried on at the sender (§3.9–§3.25).
* **C1 in both roles**, its advice C1's (adopted at Gate 4, measured at Gate 5, §3.26–§3.27):
  with the stranded frames' bound it completes every run and pooled air is ×0.990, but against
  the current controller's advice it trails on `80m-nvis-disturbed-500` (×0.79 against ×0.88,
  repeated exactly), `80m-slow-machine-500` (×0.81 against ×1.11 over twelve runs) and the start
  pair (K3 fails in four draws of six against one): the receiver's C1 advises OFDM rung 5 at
  +1 to +3 dB on NVIS where the floor carries twice as much.
* **The cautious start** (§3.6): fixed `80m-nvis-disturbed-500` and cost every climb.
* **No step down for silence** (§3.13): +31.8 %, two failed runs.
* **The try past the dip** (§3.21–§3.22): above.
* **C5**, a soft per-frame reading as the prior (`03-candidates.md` §2.6): not built. Its
  instrument check was not run in Gate 3, and the decision did not wait for it.

## 5. Consequences

**Gate 5 (the port, the daemon, the proof):**

1. Model first: C1 at the sender is the engine's default (`GrantConfig.controller` =
   `delivery-s`), the advice `RateController` as before; the stranded frames' bound (`c2dfa9f`).
2. The port (`granted.rs`); the granted vectors regenerate for this deliberate change.
3. The daemon's metrics keep their meaning: `rate_snr_db` and `margin_db` are the advice's, the
   panel's *Receiver* line describes them, and `sending_rung` is C1's choice. Nothing the operator
   sees changes.
4. The harness A/B of the build that ships, against the registered criteria.
5. A beta, its tag the author's to push; then the air with the testers, both sides uploaded and
   read with `tools/rung_reading.py`.

**Gate 6 (delete):** the bench-only prototypes not adopted — `OuterLoopController`, C1 at the
receiver (the `delivery` and `c1` names), `cautious_start`, the `controller` switch and its
names — with their tests and vectors, and the sender's own uses of the current controller that C1
replaces (the master's first rung from `first_mode` on the acceptance, and whatever of
`_first_move` only moved it), each checked in the code at Gate 6. `rate_review.py` becomes C1's
module. Kept: `RateController` whole, for the advice, with the engine rules that feed it; the
threshold tables, `usable_modes`, the echo exclusions, `silence_step`, `reencode_early_after`,
`reencode_hopeless_db`, the ceiling and the stuck-frame rules.

**Owed, recorded here:** the dip and the climb's pace (§3) are the two places C1 is known to
leave speed, and the advice is the current controller with its nineteen parameters, which K7
asked to be gone: the next review's, not a patch. C1 at the receiver's optimism on NVIS (§3.8,
§3.26, §3.27) is where that review starts. Two engine rules were justified while the current
controller chose the rung, and hold only in part under C1 at the sender (§3.28; the author kept
both at Gate 5): answers on the floor after a silent slot (`floor_answer_memory`) still pay at
500 Hz but cost a few seconds on the 2 300 Hz unit bench, and holding frames whose answer was
lost (`hold_unknown`) still saves frames but no longer time. Their unit tests run under the
current controller, as measured, until that review re-justifies or removes each. And the bound
of item 6 costs a strong path (§3.28: `80m-strong-moderate-500` ×0.77 of the current
controller against ×1.02 without it, over twelve runs): when it moves a block cut at OFDM onto
the floor in a brief dip, the floor air of its copies is charged to the OFDM rung it was cut at,
which then reads worse than the path and is climbed back slowly. Charging a copy to the rung it
went at is the fix to judge; it changes how C1 judges a path, so it is that review's.
