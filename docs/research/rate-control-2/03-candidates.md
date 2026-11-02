# Gate 2: candidates and criteria

*Proposed; to be agreed by the author before any candidate runs. The current pair's numbers come
from Gate 0 (`01-evidence.md`).*

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
| K1 | **Regret against the oracle** | the 21 regret scenarios × 3 draws (`01-evidence.md` §1) | (a) the median over scenarios of the per-scenario median regret at most **half** today's; (b) no scenario's median delivery more than **15 %** under today's |
| K2 | **No scenario slower** | every scenario × 3 draws | `scenario_trend.py` against today's three draws flags none *slower* or *failing*; pooled air time ≤ ×**1.00** |
| K3 | **Start independence** | the start pair × 3 draws | in each draw the two sessions settle within **one rung** of each other |
| K4 | **Completion and no new overlap** | every scenario × 3 draws | passes per scenario ≥ today's; radiated overlaps **0** |
| K5 | **The real-modem link bench** | `bench_link.py --backend phy --continuous`, both airs, its default grid | completion ≥ today's at every point; median session time ≤ ×**1.05** at every point and ≤ ×**1.00** pooled |
| K6 | **The field replay** | every two-sided session in `field/inbox/` (ten), open-loop | as the first review's K6; open-loop, so a reading, not a veto (its caveat, `04-results.md`) |
| K7 | **One controller, simpler** | the code | one process chooses rungs; at most **8** tunables in all, the engine's rate rules included (31 today); the old `RateController` gone or reduced to tables; any rule added names the evidence it rests on |
| K8 | **The nightly's NVIS pair** | `80m-trimode-gateway-client-vanishes-500` and `80m-agc-carrier-outside-500`, six draws each (the nightly's three variants and three more) | every draw passes, and its transfer finishes inside **80 %** of its budget (320 s of 400, 240 s of 300); neither budget loosened |

Notes:

* **K1 includes `80m-agc-carrier-outside-500`** (its message is the measure), so 21 scenarios.
* **K6 stays a reading**: the first review found an open-loop replay judges a learning
  controller's untried choices as if it never learned (`04-results.md`).
* **A failing criterion stops the step**; a narrow miss goes to the author with its trace, case
  by case (the author, 2026-10-24).

## 4. Questions for the author

1. **The criteria**: K7 firm and K8 as written?
2. **The wire under D1**: reserve the `rung` field, or have the receiver report the frames it
   never detected in it?
3. **The order**: D3 first (it serves either), then D1 against D2, then the dip and the climb?
