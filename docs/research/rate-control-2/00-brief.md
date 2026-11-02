# The rate controller's second review

*Started 2026-11-02 at the author's request ("start the rate controller review"). The first
review is `docs/research/rate-control/` 00–04 and `docs/adr/0094-the-rate-controller-chooses-by-measured-delivery.md`.
Its method stands, and this brief says only what is new: evidence first, criteria registered before
the runs, prototypes beside the shipping code, gates the author decides at, and the old machinery
deleted at the end. CLAUDE.md's rules hold throughout, the rate controller's rule above all: no
patches, a review.*

## 1. Why a second review

ADR-0094 moved the rung a station's own data goes at to C1, chosen by measured delivery. It left
four things owed, and the nightly has since shown what they cost.

### 1.1 What the first review left

* **Two controllers, not one.** Each station chooses its own rung with C1 (`DeliveryController`
  in `model/aether_model/link/rate_review.py`, `GrantConfig.controller = "delivery-s"`, seven
  tunables). The rung it *advises* the other station, in every control frame, is still the old
  `RateController` (`rate.py`, about nineteen tunables), fed by the engine's rules as before
  (ADR-0020's counting, `_asked`, the seeds and the re-seed, the silence step). K7 asked for that
  to go; the author kept it at Gate 5 because C1 as the advice was optimistic on NVIS.
* **Where the advice is still read** (`granted.py`): the control frame's `rung` field; `_recommended`
  (the peer's advice) for keeping a header on data above it (ADR-0076), for the stuck frames'
  re-encode and split targets (ADR-0085, ADR-0086, bounded by the sender's own choice under
  ADR-0094 §2.6), for the master's silence step (R9), for the first rung at the acceptance, and as
  `current_mode`.
* **Gate 6 never ran.** The prototypes the first review rejected are still in the tree and the
  port: `OuterLoopController` (C2, `"olla"`), C1 at the receiver (`"delivery"`), C1 in both roles
  (`"c1"`), `cautious_start`, the `controller` switch itself, and
  `core/aether-link/tests/data/rate_review_vectors.json`.
* **Owed by ADR-0094 §5**, each traced there:
  * *The dip*: from the floor's top, the first OFDM rung priced with floor answers carries less
    than the top even with nothing lost, so no try of OFDM is due; a session that falls to the
    floor stays there.
  * *The climb's pace*: one rung a try, three frames a try, where the old controller jumped
    several rungs on the reading.
  * *The stranded frames' bound*: a re-encoded copy's floor air is charged to the OFDM rung its
    block was cut at, which kept a strong path low in three of twelve runs.
  * *C1 at the receiver's optimism on NVIS*, the reason the old controller stayed as the advice.
  * *Two engine rules* justified under the old controller and only partly under C1:
    `floor_answer_memory` and `hold_unknown`.

### 1.2 What the nightly showed since (CLAUDE.md, 2026-11-02)

* **Two 500 Hz ITU NVIS scenarios fail on about half the nights**, at the edge of their budgets:
  `80m-trimode-gateway-client-vanishes-500` (a 1 000-byte send in 400 s) and
  `80m-agc-carrier-outside-500` (a 1 000-byte message in 300 s; its base draw also failed once in
  CI's quick set). Local runs of one draw on four builds: on 10-21's build the send and the
  message took 229–264 s and 232–245 s; today they take 296–367 s and 278 s.
* **Where the time goes.** The 10-21 build stayed on the floor's tone rungs. Today's sends at OFDM
  rungs the 7 ms echo kills: the advice at rung 5 from the first reading, C1's first choice
  (rung 6 to 10), or its tries across the floor's edge. Each block cut there goes four failing
  copies, then in OFDM pieces at rung 5 for four more rounds, before it reaches the floor
  (ADR-0085, ADR-0086).
* **The RTTY contest** (`40m-rtty-contest-2300`): the advice sits at rungs 4–5 at +8 to +13 dB
  readings under the RTTY QRM, and the Test's file runs at 116–133 bit/s against a baseline of
  165–392. It predates C1 (flagged from 10-22).
* **The nightly's baseline** (`bench/baselines/scenarios.csv`) is still `83eabd0`'s, from before
  every change since 10-21. It cannot judge this review until it is refreshed, and refreshing it
  before the review would record these slowdowns as normal.

### 1.3 The field record

No upload has arrived since 2026-10-25 14:37Z. The sessions on a C1 build are the author's own of
2026-10-25 (two Winlink sessions through the gateway, the mobile's Test) and KQ4SAX's of the same
morning. The review starts on the harness and reads those sessions; more Tests on 80 m NVIS at
500 Hz, both sides uploaded, are what it most needs from the air (§6).

## 2. The job (unchanged; restate it better if the evidence says so)

> Pick the rung that delivers the most data per second of air on this path, counting a frame
> never heard as lost; get there quickly from the first exchange; do not let the start decide
> where the session settles; and step down before a fade costs more than the step.

Added for this review, from §1:

* **One controller.** A station should not run two controllers that disagree. Either the
  advice is derived from the same account as the sender's choice, or it goes.
* **A path that refuses a family.** On a 7 ms echo the floor's slow tones carry and OFDM mostly
  does not, whatever the SNR. A choice, a try or a stuck block must not keep paying for a family
  the path has already refused.

## 3. Scope

* **In:** the sender's choice and its tries; the advice, or its removal; the start; what a
  transmission's outcome counts as (ADR-0020's rules); the stuck frames' re-encode and split
  targets and the bound; the silence step (R9); the two engine rules of §1.1; the first review's
  leftover prototypes (Gate 6).
* **Out, inputs only:** the slot timing (granted slots), the busy detector (its own review when
  due), the receiver's detection (ADR-0095 changed it: its effect shows in the oracle, not in the
  controller), the PHY and its echo reading (ADR-0087's limits are a measurement; the controller
  may use them differently).
* **A wire change** is allowed if the design needs it (for example a control frame that no longer
  carries a recommended rung). It follows the protocol-bump procedure in CLAUDE.md.

## 4. Gates

### Gate 0: the baseline on today's build (`01-evidence.md`)

1. **The oracle again.** ADR-0095 changed what the receiver detects, so the first review's
   oracle (beta.122) no longer stands. `tools/rung_oracle.py` on the same 20 regret scenarios,
   three draws, today's daemon: draw 0's coarse rungs run first (started 2026-11-02, the
   session's scratchpad), then the best's neighbours and draws 1–2, as `03-candidates.md` §5.1
   did. `80m-agc-carrier-outside-500` joins the set (its message is the measure).
2. **Today's pair on every scenario**, three draws: the K2/K4 reference, and the baseline the
   nightly should move to once this review lands.
3. **The NVIS trace.** On the two nightly scenarios and `80m-nvis-disturbed-500`: per rung,
   frames sent, never detected, decoded alone, combined; the air spent on OFDM; how many copies
   and rounds of pieces each stuck block took; what the advice said and when.
4. **The field sessions on a C1 build** (§1.3), read with `tools/rung_reading.py`.
5. **The count for K7**: today's tunables in both controllers, the engine rules around them, and
   the lines of code in the model and the port.

### Gate 1: the survey's update (`02-survey.md`)

The first survey (`docs/research/rate-control/02-survey.md`) stands. Add only what this review
asks that it did not: where a link decides its rate when both ends can measure (STANAG 5066
Edition 4's sender choosing on the receiver's advice, Minstrel's sender-only statistics, cellular
OLLA at the transmitter on the receiver's CQI), and how adaptive HF modems handle a path whose
multipath refuses a whole waveform family.

### Gate 2: candidates and criteria, registered before the runs (`03-candidates.md`)

Candidates to weigh, at least:

* **D1, one controller at the sender.** C1 decides alone; the receiver reports what it measured
  (it already acknowledges every frame and states its reading), and the old `RateController` goes
  with the engine rules that only feed it. The stuck-frame targets come from the sender's own
  account. This is the largest simplification on offer.
* **D2, C1 in both roles, made right on NVIS.** The advice is C1's, with a prior that knows what
  the floor's echo reading says about OFDM.
* **D3, a family refused.** Whatever decides, a family the path has refused (by the echo reading,
  or by its frames' outcomes) is not tried again soon, and the blocks a failed try cut there come
  down at once instead of after four copies and four rounds of pieces.
* **The dip and the climb's pace** (ADR-0094 §5), in whichever candidate leads.

The criteria start from the first review's K1–K7 (`docs/research/rate-control/03-candidates.md`
§3), measured against today's pair on today's build, with the oracle of Gate 0. Two changes are
proposed for the author to agree before any run:

* **K7 made firm**: one controller, at most eight tunables in all, and the old `RateController`
  gone or reduced to tables.
* **K8, the nightly**: both NVIS scenarios of §1.2 pass every draw with room (their transfer
  inside 80 % of its budget), without loosening either budget.

### Gates 3–6

As the first review (`docs/research/rate-control/00-brief.md` §7): prototypes in the model beside
the shipping code, the harness A/B with three or more draws, the decision ADR with the list of
what is deleted, the port and the proof, then the deletion. **Gate 6 deletes the first review's
leftovers too** (§1.1).

## 5. Harness lessons

Those of the first brief (§8) hold. One more, from 2026-11-02: **a long run outlives a tool's
two-hour background limit**; start it detached (`setsid nohup … &`), let `rung_oracle.py` resume
from the runs that finished, and never let a wait loop's pattern match its own command line
(`pgrep -f "[c]argo build"` inside a command that runs `cargo build` waits on itself).

## 6. For the author

1. **Field builds.** Do field builds hold still for rate changes until the decision ADR, as they
   did for the first review? Fixes outside the controller (ADR-0098) could still ship.
2. **The advice.** May the review consider removing the advice outright (D1), with a protocol bump
   if the control frame's `rung` field changes meaning?
3. **Gate 6 of the first review.** Delete now the prototypes that no candidate here needs (C2 and
   the cautious start), keeping C1 at the receiver and C1 in both roles as comparison arms until
   this review's own Gate 6?
4. **The air.** A few Tests on 80 m NVIS at 500 Hz with the testers, both sides uploaded, are the
   field evidence this review lacks.
