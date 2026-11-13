# The rate controller's third review: how C1 guesses a rung it has not measured

*Started 2026-11-13 at the author's request: "Start a review of how that guess is made (the 3 dB
cut, and how much weight it gets), with an eye to speeding up the modem. I think it may be too
conservative." The first two reviews are `docs/research/rate-control/` and
`docs/research/rate-control-2/`, decided by ADR-0094 (C1, choosing by measured delivery) and
ADR-0099 (one controller, at the sender, link protocol 12). Their method stands: evidence first,
criteria registered before the runs, prototypes beside the shipping code, gates the author decides
at, and what the change removes said as well as what it adds. CLAUDE.md's rate controller rule
holds: no patches, a review.*

## 1. Why

The RadioPost mail of 2026-11-12 (14:08–14:29Z, 80 m, the mobile KK4ODA to the gateway KK4ODA-1,
beta.150 at both ends, both sides with audio) is the first long run of C1 on a steady path: 22.5
kB at 500 Hz and the same again at 2300 Hz. The gateway read the mobile at +4.3 to +5.6 dB
throughout. The link never reached the rung the path carried.

C1 was replayed on the sessions' own frames (the scratch replay of §4, Gate 0 item 1): the model's
`RateController`, built as the engine builds it, seeded from the acceptance, fed each transmission's
outcomes and the gateway's reading as the sender's engine fed them. It repeats the sessions'
choices: 21 of 21 transmissions at 2300 Hz, 50 of 54 at 500 Hz (the four apart are resends of
failed frames at their own rung, which the scratch replay does not model).

**2300 Hz.** The mobile sent rung 11 (QPSK ⅔): 115 frames, all decoded on their first copy. C1
tried rung 12 (8PSK ½) at 88–105 s, one frame a transmission; all four of its frames decoded. The
verdict went against it by 1.2 %: rung 12 valued at 136.5 bytes a second of exchange, rung 11 at
138.2. The prior, worth two frames, still rated rung 12 at 0.77 although the family's offset had
moved 2.5 dB over the session's successes, and the try's own frames had aged by half under the rung
11 air sent with them. The wait before the next try doubled to twelve transmissions, and the
session ended first. Rung 12 carries 12.7 % more per exchange.

**500 Hz.** The try of rung 11 (16-QAM ¾) at 74–91 s lost by 1.4 % (48.7 against rung 10's 49.4).
From 117 s C1 itself rated rung 11 above rung 10 (52–54 against 49.5), but could not choose it: a
rung above becomes choosable only once its own frames outweigh the prior, the try's three had aged
under it, and the doubled wait held the next try until 194 s. So 77 s went at the slower rung with
C1 knowing better. Rung 11 then decoded 171 of 174.

**Where the guess was right.** At 500 Hz the prior put rung 12 (64-QAM ⅔) at 0.01–0.03, and the
path decoded 1 of its 9 frames. A faster review must keep that.

What the evidence does not show yet: whether this is the margin, the weight, the try's length, the
aging, the doubled wait, or the reading itself. At 2300 Hz the reading was 4.5 dB, under rung 12's
AWGN 10 % point (4.7 dB), and rung 12 decoded 4 of 4; either the reading runs low on this path, or
the AWGN points sit high, or both. One session pair is a hint, not a law.

## 2. The guess today, part by part

All in `model/aether_model/link/rate.py` (`RateController`), ported to `rate.rs`.

| part | what it does | value | set by |
|---|---|---|---|
| the reading | the other station's report of how it hears this one, whole dB, smoothed 0.7/0.3 (`observe_snr`) | — | ADR-0094 |
| the prior | a rung's chance to decode: the AWGN curve, a logistic through 90 % at the rung's AWGN 10 % point, read `prior_margin_db` under the reading (`prior_probability`) | 3 dB, slope 0.5 dB | ADR-0094 §2.2; first review `04-results.md` §2.4 |
| its weight | the prior counts as `prior_frames` frames in a rung's estimate (`estimate`) | 2 | ADR-0094 §2.2 |
| the family offset | each frame moves its family's reading by `family_gain_db × (p − decoded)`, held to −5…+20 dB | 0.5 | ADR-0094 §2.2 |
| the account | per rung, data delivered over air spent, aged with a half-life of `half_life` of the rung's own frames, by any rung's air | 12 | ADR-0094 §2.1, first review §3.24 |
| the try | one frame a transmission at the rung above, when its lossless rate beats the current rung's estimate and `probe_every` full transmissions' air has gone (or at once on a clean climb in the family) | 6 | ADR-0094 §2.4 |
| the try's length | decided once its own frames outweigh the prior: `prior_frames` + 1, so three (`_try_over`) | 3 | first review §3.15 |
| the verdict | the rung above is taken if its value beats the current rung's; if not, its wait doubles, to `probe_wait_max` (`_judge_try`) | 48 | ADR-0094 §2.4 |
| choosing up | a rung above is choosable only once its own frames outweigh the prior (`_decide`) | — | first review §3.15 |

The first review chose the optimistic prior (the AWGN curve 3 dB under the reading) over the four
channels' mean because "a rung above found wanting costs one measured transmission, a rung under the
path's best costs every wait for a try". It chose a three-frame try because "one frame is a coin
toss": one lucky decode had put seventeen frames on OFDM over a 7 ms echo. Both were measured on the
harness's regret set, mostly fading and NVIS paths, and the 2026-11-12 sessions are the kind it had
least of: long, strong and steady.

**Already tried.** The second review tried a climb that tests faster rungs sooner (ROADMAP P9-17,
`docs/research/rate-control-2/04-results.md` §3): it gained 13–17 % on the 2300 Hz fading paths and
lost 10–25 % on weak, echo and clean paths, and was not taken. A guess made bolder everywhere is
the same bet; this review looks first for what holds a climb the path has already proved, as in §1,
where the try's own frames all decoded and the verdict still went against them.

## 3. The job

> Pick the rung that delivers the most data per second of air on this path, counting a frame never
> heard as lost; get there quickly from the first exchange; do not let the start decide where the
> session settles; step down before a fade costs more than the step.

For this review, from the author: **faster**. A guess that keeps the link a rung low costs every
transmission of the session; a try that fails costs one transmission. Where those two costs balance
is the question, and the answer must hold on the fading and NVIS paths that set today's values, not
only on the steady path that questioned them.

## 4. Scope

* **In:** the prior (its margin, its slope, its weight), the family offset (its gain and its
  limits), the try (when, how long, how judged), the doubled wait after a lost verdict, the rule for
  choosing a rung above, and the account's aging as it bears on a try's verdict.
* **Input, not changed by this review:** the reading itself, the receiver's SNR estimate. Gate 0
  measures whether it runs low against what the rungs decode; if it does, the author decides
  whether the review widens to it (§6, question 5).
* **Out:** the echo limits (ADR-0087), the family refusal (D3), the stuck frames' targets
  (ADR-0085, ADR-0086), the slots, the PHY, the busy detector. No wire change is expected: the
  guess is the sender's own.

## 5. Gates

### Gate 0: the evidence (`01-evidence.md`)

1. **A C1 field replay** (`tools/c1_replay.py`): the scratch replay of §1, made a tool with a test.
   For each two-sided session it reports where it repeats the session and where it parts, and per
   session: the tries, each verdict and its margin, the time a rung above was rated better but not
   choosable, and the time at a rung under the best the session's own frames show carried. Run on
   every two-sided upload since protocol 12 (2026-11-04: the gateway and mobile sessions of 11-06,
   11-07, 11-08 and 11-12) and, as a reading, on 11-02's (C1 at the sender, protocol 11).
   `tools/rung_reading.py` reads sessions over 256 data frames since `59da256`, so its per-rung
   account stands beside the replay.
2. **The reading's calibration.** Per rung, the share of frames decoded against the reading the
   receiver reported, on the harness's steady and fading scenarios and on the real-modem link bench
   (`bench_link.py --backend phy`), set against each rung's AWGN 10 % point. This says how far
   under the reading the truth lies on each class of path, which is what the margin stands in for.
3. **The oracle on strong paths.** The regret against the best fixed rung (`tools/rung_oracle.py`)
   on the strong and steady scenarios of the regret set (`quick-awgn-2300`,
   `80m-strong-moderate-500`, `40m-good-throughput-2300`) and a new one shaped like 2026-11-12 (80 m, steady,
   +5 dB, a 22 kB message, both bandwidths): ten draws, the weekly speed check's way.
4. **The count:** today's tunables that bear on the guess (§2: all seven of C1's), and the lines of
   code in the model and the port.

### Gate 1: the survey (`02-survey.md`)

The first survey stands. Added only what bears on a prior: how much weight rate-adaptation schemes
give an unmeasured rate against measured ones (Minstrel-HT's lookaround, SampleRate's lossless rule,
Thompson sampling and Bayesian bandits' priors, OLLA's step sizes as a margin learned on line), and
how long their tries run before a verdict.

### Gate 2: candidates and criteria, registered before the runs (`03-candidates.md`)

Candidates to weigh, at least, each alone and the best together:

* **G1, the margin**: 0, 1, 2 dB against today's 3, one value or one a family.
* **G2, the weight**: the prior worth one frame, or a weight that shrinks as the family's offset is
  learned (the offset already carries what the family's frames said).
* **G3, the verdict on the try's own frames**: the rung above taken when its own frames' delivery
  beats the current rung's estimate, the prior left out of the verdict once the try has run.
* **G4, the near miss**: a try that lost by under a set share waits the ordinary six transmissions,
  not twice that, and a rung above that C1 rates better is tried at once (the 500 Hz session's
  117–194 s).
* **G5, the aging**: a try's frames not aged by the current rung's air until the verdict.

The criteria start from the second review's K1–K8 (`docs/research/rate-control-2/03-candidates.md`
§3), measured against today's build on the same scenarios and draws. Proposed for the author to
agree before any run:

* **K1 (speed)**: the median regret over the strong and steady scenarios (Gate 0 item 3) down by at
  least a third, and over the whole regret set not up. A ten-draw median regret moves by about ten
  points from one run of the same build to the next (ROADMAP P9-18), so a gain smaller than that is
  run again on fresh draws before it counts.
* **K2, K4, K8 unchanged**: nothing slower than today by the 10 % rule, completion and no overlap,
  both NVIS echo tests inside their budgets.
* **K6 (the field replay)**: on the 2026-11-12 sessions the candidate takes rung 12 at 2300 Hz and
  rung 11 at 500 Hz by the end of its first winning try, and keeps off 500 Hz rung 12. Open loop, a
  reading, not a veto.
* **K7**: no more tunables than today, and fewer if a candidate allows it.

### Gates 3–6

As before: prototypes in the model beside the shipping code (`GrantConfig.rate` takes every value
of §2, so a candidate of values needs no new switch), the harness A/B at ten draws, the decision
ADR with what it removes, the port and its vectors, then the deletion of what lost. After it the
nightly's baseline is refreshed and a few strong-path sessions on the air confirm it.

## 6. For the author

1. **Field builds.** Do they hold still for rate changes until the decision ADR, as in the first two
   reviews? Fixes outside the controller (such as the Windows read timeouts and the keyed-audio
   watch, beta.153 onward) would still ship.
2. **How much faster, at what risk.** Is "no scenario slower than today by more than 10 %, the NVIS
   budgets kept" the right guard, or would you take a small loss on fading paths for a larger gain
   on steady ones?
3. **Where the runs go.** The oracle and the A/B on GitHub's runners (about two hours a Rate oracle
   run, four for the weekly speed check, each started only with your go and its cost), or in a
   cloud session's container (slower, no Actions minutes)?
4. **The air.** A few more long sessions on a steady path, both sides uploaded (RadioPost mail like
   2026-11-12's is ideal), before the decision and on its build after.
5. **The reading.** If Gate 0 finds the receiver's SNR reading runs low on steady paths, does the
   review widen to it, or stay with the guess and let the margin absorb it?
