# Gate 2: candidates and criteria

*Written 2026-11-13, after Gate 0 (`01-evidence.md`) and Gate 1 (`02-survey.md`). Registered before any candidate runs. The criteria are proposed here for the
author to agree; nothing in §2 is built until they are.*

## 1. What every candidate keeps

* **C1's shape.** Per rung, the data delivered per second of air, every copy and every frame
  never detected counted, priced per whole exchange; the choice is the best estimate around the
  current rung (ADR-0094, ADR-0099).
* **Out of scope, unchanged** (`00-brief.md` §4): the reading itself, the echo limits (ADR-0087),
  the family refusal (D3), the stuck frames' targets (ADR-0085, ADR-0086), the slots and the wire.
  Every candidate is the sender's own business: no protocol change.
* **The prior stays one AWGN curve read under the reading, corrected by the family's offset.**
  Gate 0 §2 found no one margin right for every path: 3 dB too cautious on a clean path, not
  cautious enough on a fading one. A curve per class of path is no answer either, since the
  sender cannot tell the class from one reading. The offset, which the frames move, is what tells
  the paths apart, so several candidates act on how fast it learns.

## 2. The candidates

Each alone first, then the best of them together. For each: what it changes, the evidence it
answers, where it could lose, and what it does to the count of tunables (K7; seven today).

### G1: the margin

`prior_margin_db` 1 or 0 dB, against today's 3 (2 dB is left out: Gate 0 §2 puts the clean path's
truth at 0.2–0.8 dB over the AWGN point, so 2 dB keeps more than half of today's caution).

* **Answers:** the clean path's caution (Gate 0 §2), and where C1 starts. The start is
  `first_mode`, the best rung by the same prior, so on 40 m Good at +20 dB a smaller margin starts
  a rung or two higher, at the best single rung (17) or next to it (Gate 0 §3).
* **Could lose:** on fading paths, where the prior is already near certain at readings where a
  rung still loses one frame in ten (Gate 0 §2); a higher start on NVIS and flutter paths, which
  the family offset then has to walk back. The second review's bolder climb lost 10–25 % on weak,
  echo and clean paths. Guards: the two NVIS echo tests (K8), the start pair (K3), the weak paths
  in K2.
* **Tunables:** a value changes; none added.

### G2: how fast the guess learns

* **G2a, the weight:** `prior_frames` 1 against today's 2. A try runs `prior_frames` + 1 frames,
  so the try shortens to two frames as well. That couples two things: the first review chose three
  frames because "one frame is a coin toss", and two is between them.
* **G2b, the family offset's gain:** `family_gain_db` 1.0 against today's 0.5. On the steady
  path of 2026-11-12 the offset had moved 2.5–2.8 dB by the end of a 22.5 kB session, short of the
  3 dB the calibration says that path wanted, and slowing as the rung in use decoded reliably
  (Gate 0 §2). Twice the gain gets there in about half the frames, and walks a fading path's
  optimism back twice as fast.
* **Could lose:** G2a, a lucky two-frame try on a fading path; G2b, a noisier offset (one failure
  moves a family's every rung by up to a decibel).
* **Tunables:** values change; none added.

### G3: the prior steps aside once a rung has its own frames

A rung's prior counts in full while the rung has no frames of its own, and less as they come in,
until it counts for nothing once the rung has a whole try's worth (`prior_frames` + 1, as aged):
the weight is `prior_frames × max(0, 1 − frames / (prior_frames + 1))`.

* **Answers:** the near misses (Gate 0 §1): tries whose every frame decoded, lost by 1.2–1.8 %
  because the prior, worth two frames, still rated the rung at 0.77. Every scheme the survey read
  drops its prior once a rate is measured, and Mercury, which met the same near miss, does not
  judge a clean probe on its prior (`02-survey.md` §1, §2). Judging only the verdict on
  the try's own frames would not do: the choice is made again from the estimates straight after
  the verdict (`_decide`), and with the prior still in the estimate it would step back down at
  once. So the prior has to step aside in the estimate itself.
* **Could lose:** a try of three frames that decode by luck on a fading path, followed by the rung
  failing (on 80 m Moderate at 500 Hz, rung 13 decoded 4 of 15 try frames, Gate 0 §3). The
  choice still follows the estimate afterwards, so a rung taken by luck is left at the next
  failures, at the cost of those failures. As frames age the prior comes back in, so a rung not
  sent at for a long time is guessed again.
* **Tunables:** a rule; none added.

### G4: no doubled wait after a clean try

A lost try none of whose frames failed does not double the wait for the next try of that rung.
Today it doubles whatever the frames did, and the doubled wait also closes the clean-climb path
(an immediate try after a clean transmission needs the wait undoubled).

* **Answers:** the 500 Hz session of 2026-11-12, where the try of rung 11 decoded all its frames and
  lost by 1.4 %, and C1 then rated rung 11 better from 117 s and could not try it again until 194 s
  (`00-brief.md` §1, Gate 0 §1).
* **Could lose:** on a path where a rung's frames decode but its value per exchange is lower (the
  floor's edge, where answers change family), it is tried again sooner, one frame a transmission.
* **Elsewhere:** Mercury doubles a probe's wait only while the rung is marginal and delivered under
  half, and measured doubling regardless at 17 % on a rising link (`02-survey.md` §2). If G4 passes
  and tries that lost one frame of three still hold the climb, Mercury's "under half" is the next
  variant to run; it would add a constant.
* **Tunables:** a rule; none added.

### G5: no aging during a try

While a rung is tried, its account is not aged by the air the current rung sends beside it; the
aging resumes at the verdict.

* **Answers:** the 2300 Hz session of 2026-11-12, where the try's four frames had aged by half
  under the rung 11 frames sent with them by the time of the verdict (`00-brief.md` §1).
* **Could lose:** little on its own: the frames are at most a few exchanges old at the verdict.
* **Tunables:** a rule; none added.

### G6: the try in one transmission

Within the current rung's family, the try's frames (`prior_frames` + 1) go together in one
transmission when it holds that many, so the verdict comes after one exchange instead of three.

* **Answers:** the pace on a short transfer (Gate 0 §3, §5 item 5): on 40 m Good the 16 kB
  message ends after one or two tries, each three transmissions long.
* **Could lose:** one fade takes all the try's frames together, so the try says less about the
  path than three frames in three transmissions. The second review's climb filled the *whole*
  transmission with the rung tried, and started a try after every clean transmission whatever the
  estimate said; its losses came from the two together, a whole transmission spent on a try that
  could not win, every time (`docs/research/rate-control-2/04-results.md` §3.3: AWGN at rung 16
  and the weak 2300 Hz path). G6 keeps today's conditions for starting a try. No scheme in the
  survey does this: their tries cost microseconds, or one transmission each (`02-survey.md` §2).
* **Tunables:** a rule; none added.

### Together

The values of G1 and G2 that pass, with the rules among G3–G6 that pass, as one candidate. If two
candidates pass that overlap (G3, G4 and G5 all act on the near miss), the simpler is kept unless the
pair is measurably better.

## 3. Criteria

Measured on today's build (beta.156) on the same scenarios and draws: today's controller and each
candidate, both on GitHub's runners, controller only. They start from the second review's K1–K8
(`docs/research/rate-control-2/03-candidates.md` §3) and change where Gate 0 says the old measure
misleads.

| # | criterion | measured on | pass |
|---|---|---|---|
| K1 | **Speed where the gap is** | the two strong 2300 Hz paths: `40m-good-throughput-2300` (10 draws) and `80m-steady-mail-2300` (3 draws; its draws repeat) | the gap to the **best single rung** (one rung held on every draw: today rung 17, +23 %; rung 12, +22 %) closed by at least a third: **+15 % or less** on each |
| K2 | **No speed scenario slower** | the 21 regret scenarios (the AGC echo test among them), the Trimode echo test and the two steady-mail scenarios: 24, 10 draws (3 for steady-mail) | the transfer-time ratio (Hodges–Lehmann, candidate over today) has its 90 % range end below **1.10** in each; a scenario that fails is run again on 10 fresh draws and fails only if it fails again |
| K3 | **The start** | the start pair (`80m-start-low-reading-500`, `-high-`) × 3 draws | no more draws settling over one rung apart than today on the same draws |
| K4 | **Completion and no overlap** | every scenario × 3 draws | passes per scenario ≥ today's; radiated overlaps **0** |
| K5 | **The real-modem link bench** | `bench_link.py --backend phy --continuous`, both airs, its default grid | completion ≥ today's at every point; median session time ≤ ×1.05 at every point and ≤ ×1.00 pooled |
| K6 | **The field replay** (a reading) | the 2026-11-12 sessions, open loop (`tools/c1_replay.py`) | takes rung 12 at 2300 Hz and rung 11 at 500 Hz by the end of its first winning try, and keeps off 500 Hz rung 12 |
| K7 | **No more tunables** | the code | seven or fewer; any rule added names the evidence it rests on |
| K8 | **The two NVIS echo tests** | `80m-agc-carrier-outside-500`, `80m-trimode-gateway-client-vanishes-500`, 10 draws | every run inside its budget (300 s, 400 s), neither loosened; not slower than today by K2's rule |

Notes:

* **Why K1 changed.** The brief proposed the median regret against each draw's best rung, cut by a
  third. Gate 0 §3 shows that measure counts luck no controller has: at 500 Hz the best rung moves
  from draw to draw (10, 12, 13 or 14) by more than the rungs differ, so its +15–18 % is mostly
  out of reach, and the median over the strong set would be set by the paths with nothing to win
  (AWGN, 0). The best single rung is the bar a controller can reach, and the two 2300 Hz paths are
  where the gap is.
* **The 500 Hz strong paths are guarded, not targeted.** `80m-strong-moderate-500` (+4–8 % to rung
  12, about as much as two runs of today differ) and `80m-steady-mail-500` (+3 %) are in K2: the
  candidate must not be slower there.
* **K2's ten draws, unpaired.** As in the second review: Mann–Whitney and the Hodges–Lehmann ratio
  with its exact range, because two controllers on one draw drift onto different stretches of the
  fading once their choices differ. With 24 scenarios at 90 % ranges, about one looks slower by
  chance; the fresh draws tell chance from a real loss.
* **K4 and K5 run on the final candidate only.** They test that nothing breaks; a screen does not
  need them.
* **K8's regret against the best fixed rung is dropped** (the second review's K8(ii)): it would need
  every rung of both tests pinned again on today's build (about two hours of runners), and the
  budgets and K2 already hold the tests where they matter.
* **A failing criterion stops the step.** A narrow miss goes to the author with its numbers and
  trace, case by case (`00-brief.md` §6, answer 2).

## 4. Gate 3's order and its cost

1. **Tooling, no runner time.** Each candidate in the model and the port as a bench-only switch
   beside today's code (G1 and G2 are values of `GrantConfig.rate` already). A simulated daemon
   takes them from an environment variable, as the second review's `AETHER_BENCH_LINK` did (it was
   deleted at that review's end and comes back for this one). The oracle workflow passes it, and
   gains the two steady-mail scenarios and the start pair. Model tests for each rule; with every
   switch off the granted vectors regenerate byte for byte.
2. **Today's baseline.** Today's controller on K2's 24 scenarios and the start pair, ten draws: one controller-only
   run, about **2 hours** of runner time (the second review's run 19, 23 scenarios, took 123
   minutes).
3. **The screen.** Each candidate alone on a screening set of nine: the two K1 paths, the 500 Hz
   strong and steady paths, AWGN, both echo tests, `80m-weak-wide-test-2300` and `80m-flutter-500`
   (where the second review's climb lost). About **50 minutes** of runner time a candidate; six
   arms (G1 at 1 and 0 dB, G2a, G2b, G3 + G4 + G5 together, G6), so about **5 hours** in all.
   G3–G5 go together first because each is small and all three act on the try's verdict; if
   together they pass, each is taken out in turn to see which earn their place.
4. **The candidate.** The best together, on K2's whole set (about **2 hours**), then K4 and K5
   (about **3 hours**: every scenario at three draws, and the link bench).

So about **12 hours** of runner time over several runs, each asked for first with its cost, no
audio kept. Gate 3's results go in `04-results.md`; the decision ADR is Gate 4.
