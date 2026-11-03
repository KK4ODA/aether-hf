# Gate 2: candidates and criteria

*The criteria were agreed by the author on 2026-11-03 (§4); the order of the candidates is still
open. The current pair's numbers come from Gate 0 (`01-evidence.md`).*

"Today's pair" is what ships: C1 chooses each station's own rung (`GrantConfig.controller =
"delivery-s"`), and the old `RateController` makes the rung each station recommends to the other,
fed by the engine's rules as before. Every candidate is measured against it on the same build,
scenarios and draws.

## 1. What every candidate keeps, and what it may change

**Kept.** The granted slots and their timing, the exact counts (ADR-0068), HARQ and the RV
sequence, the frame formats except the control frame's `rung` field (§2.1), the echo reading
(ADR-0087's measurement; its limits are a candidate's to keep or replace), the ceiling and
`max_mode`, the Test and its ladder (ADR-0074, ADR-0077), and C1's per-rung account (delivery per
second of every copy's air, frames never detected included) as the measurement every candidate
starts from.

**May change.** Who decides; the recommendation and the rules that feed it; the start; what a
transmission's outcome counts as (ADR-0020); the stuck frames' targets and timing (ADR-0085,
ADR-0086, ADR-0094 §2.6); the silence step (R9); `floor_answer_memory` and `hold_unknown`.

## 2. The candidates

### 2.1 D1: one controller, at the sender

**What decides.** Each station's C1 chooses its own data's rung from its own account, its prior
and its tries, as today. Nothing else chooses a rung.

**What goes.** The recommendation: the old `RateController` as the advice, its seeds and
re-seed, ADR-0020's counting at the receiver (trusted, RV 0 or 3, combined; the rungs asked
for), `_recommend`, `_burst_mode`'s use of the peer's rung, and R9's silence step on the advice
(C1's own `silent` stays). `header_above_recommendation` becomes "keep the header on a try"
(C1 knows when a rung is a try; ADR-0076's reason, a count a receiver cannot read from frames
that may not decode, is the same). The stuck frames' targets (`_reencode_target`,
`_split_rung`) take the sender's own account in place of the peer's rung: the most robust rung
whose estimate beats the frame's rung, in the family C1 would choose.

**The wire.** The control frame's `rung` field (five bits) has no reader. Either it is reserved
(written 0, ignored) or it carries what only the receiver knows (`02-survey.md` §1): how many of
the transmission's frames it never detected, which the sender cannot tell from a failed decode.
Either way the field's meaning changes, so link protocol 12 (`00-brief.md` §3; the author's
answer 2).

**Removes** the old controller entirely (17 tunables and the SNR smoothing it lends C1, which C1
would own) and about five engine rules. **Adds** at most a rule for the stuck frames' target.

### 2.2 D2: C1 in both roles, made right on NVIS

**What decides.** The receiver's C1 (`"delivery"`) makes the recommendation and the sender's C1
its own choice, as the first review's `c1` arm; the receiver's prior is told what the echo says
of each family (§2.3), which is what made it optimistic on NVIS (ADR-0094 §5).

**Removes** the old controller; **keeps** two controllers that can disagree, and the
recommendation's rules (`_recommend`, the stuck frames' bound by the peer's rung). Kept as the
comparison D1 must beat: the first review chose the sender's C1 and kept the old advice because
the receiver's C1 lost on NVIS, and D2 is that arm repaired.

### 2.3 D3: a family refused

A component for D1 or D2, judged on its own before it is combined.

1. **From the echo.** A family whose guard the session's echo reading exceeds is out until a try
   of it delivers: on the 500 Hz air the OFDM rungs, by a limit measured like ADR-0087's but
   comparing what each rung *delivers per second of an exchange with HARQ* rather than first
   copies (the comparison that left OFDM without a limit, `02-survey.md` §2). One limit per
   family, not per rung.
2. **From outcomes.** A try of a family's most robust usable rung that delivers nothing puts the
   family's faster rungs at no better than it (the monotonicity ADR-0087 assumes and Minstrel's
   inheritance states); its wait doubles as a rung's does.
3. **The stuck blocks come down at once.** A block cut at a family the session has refused goes
   at once to the target of the family in use, not after `max_combines` + 1 copies and up to four
   rounds of pieces (ADR-0085, ADR-0086): HARQ cannot rescue a frame whose family the path does
   not carry.

**Removes** the per-rung echo limits (one family limit in their place) and, with 3, most of the
time ADR-0094 §5 charges to the stranded frames' bound.

### 2.4 The dip and the climb's pace

For whichever of D1 and D2 leads, after D3 is settled.

* **The dip.** From the floor's top the first OFDM rung, priced with the floor's answers, carries
  less than the top even with nothing lost, so no try of OFDM is due and a session that falls to
  the floor stays there (ADR-0094 §5). A try is priced with the answers it would earn if it
  delivered (ordinary ones, when the reported SNR clears the ordinary control frame by the
  answer margin).
* **The climb.** One rung a try, three frames a try. A try goes to the rung the prior and the
  account rate best among those whose lossless delivery beats the current rung's measured one
  (SampleRate's pruning, not the next rung), and fills the transmission it rides in.

### 2.5 C1's own tunables

Seven today (`01-evidence.md` §5). Each candidate states which it keeps; any kept is justified by a
sweep on the harness, not on the link bench's fading pipe.

## 3. Criteria

Registered before any candidate is run, against today's pair on the same build, scenarios and
draws. K1–K6 are the first review's (`docs/research/rate-control/03-candidates.md` §3) with its
numbers replaced by today's; K7 is made firm and K8 is new (`00-brief.md` §4).

| # | criterion | measured on | pass |
|---|---|---|---|
| K1 | **Regret against the oracle** | the 21 regret scenarios × 10 draws (`01-evidence.md` §1) | (a) the median over scenarios of the per-scenario median regret at most **half** today's; (b) no scenario's median delivery more than **15 %** under today's |
| K2 | **No speed scenario slower** | the 21 regret scenarios and the two echo tests × 10 draws, the candidate's own runs against today's | in each, the transfer time ratio (Hodges–Lehmann, candidate over today) has its 90 % range end below **1.10**; a scenario that fails is run again on 10 fresh draws and fails K2 only if it fails again |
| K3 | **Start independence** | the start pair × 3 draws | in each draw the two sessions settle within **one rung** of each other |
| K4 | **Completion and no new overlap** | every scenario × 3 draws | passes per scenario ≥ today's; radiated overlaps **0** |
| K5 | **The real-modem link bench** | `bench_link.py --backend phy --continuous`, both airs, its default grid | completion ≥ today's at every point; median session time ≤ ×**1.05** at every point and ≤ ×**1.00** pooled |
| K6 | **The field replay** | every two-sided session in `field/inbox/` (ten), open-loop | as the first review's K6; open-loop, so a reading, not a veto (its caveat, `04-results.md`) |
| K7 | **One controller, simpler** | the code | one process chooses rungs; at most **8** tunables in all, the engine's rate rules included (31 today); the old `RateController` gone or reduced to tables; any rule added names the evidence it rests on |
| K8 | **The nightly's two echo tests** | `80m-trimode-gateway-client-vanishes-500` (its host program's 1 000-byte send) and `80m-agc-carrier-outside-500` (its 1 000-byte message), 10 draws each, with every usable rung pinned on the same draws | (i) every run finishes inside its budget (400 s, 300 s), neither loosened; (ii) the median over the draws of the candidate's time over the best fixed rung's time on that draw at most **1.20** (a median regret of at most +20 %); (iii) K2 for these two: not slower than today by more than 10 % |

Notes:

* **K1 includes `80m-agc-carrier-outside-500`** (its message is the measure), so 21 scenarios.
* **Which scenarios get statistics.** The 21 regret scenarios and the two echo tests measure speed,
  so they get ten draws and the comparisons of K1, K2 and K8. The other scenarios (gateways,
  aborts, VOX, host programs, slow machines) test that nothing breaks: three draws each, judged
  by K4.
* **Why ten draws.** With three runs a side no test can show anything: a Mann–Whitney comparison
  of three against three cannot get below 5 % even when every run of one beats every run of the
  other. Ten against ten can (`tools/oracle_report.py`, `model/tests/test_oracle_report.py`).
* **Why unpaired.** The simulated channel follows the air's clock, so two controllers given the
  same draw drift onto different stretches of its fading once their choices differ (CLAUDE.md,
  ADR-0088 §5). A paired test would claim more than the runs show; Mann–Whitney and the
  Hodges–Lehmann ratio with its exact (Moses) range do not assume pairs. A lost transfer ranks as
  the longest.
* **Chance across many scenarios.** At 90 % ranges over 23 scenarios about one will look slower
  by chance alone; K2's second set of ten fresh draws is what tells chance from a real loss.
* **K8's three parts** answer three questions: (i) the nightly passes; (ii) the controller loses
  little to its own choices on a path where even the best rung is slow, so a slow draw does not
  fail a good controller; (iii) it is no slower than today's.
* **K6 stays a reading**: the first review found an open-loop replay judges a learning
  controller's untried choices as if it never learned (`04-results.md`).
* **A failing criterion stops the step**; a narrow miss goes to the author with its trace, case
  by case (the author, 2026-10-24).

## 4. The author's answers (2026-11-03)

1. **K7 is firm**: one decision-maker, at most eight tunables in all.
2. **K8 combines a measure against the best possible with more draws**, and the speed comparisons
   are judged statistically where the decision hinges (§3: K1, K2, K8 at ten draws, unpaired
   Mann–Whitney and Hodges–Lehmann; the other scenarios at three draws by K4). The author asked
   for the combination and for a non-parametric test; the rules above are the result of that
   discussion.
3. **The wire under D1**: the five bits that carried the recommended rung carry the number of
   frames of the transmission the receiver never detected, which only it can know (link protocol
   12 if D1 is chosen).

Still open: **the order** — D3 first (it serves either), then D1 against D2, then the dip and the
climb.
