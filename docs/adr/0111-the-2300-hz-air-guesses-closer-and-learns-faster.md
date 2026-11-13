# ADR-0111: The 2300 Hz air guesses closer and learns faster

**Status:** accepted, 2026-11-13, the author's decision at the end of the rate controller's third
review (`docs/research/rate-control-3/`: the brief `00-brief.md`, the evidence `01-evidence.md`,
the survey `02-survey.md`, the candidates and criteria `03-candidates.md`, the results
`04-results.md`). It amends ADR-0094 §2.2 (the prior's margin and the family offset's gain) for
the 2 300 Hz air. No wire change: each station's guess is its own.

## 1. Context

C1 (ADR-0094, ADR-0099) rates a rung it has not measured by a prior: the rung's AWGN curve read
3 dB under the other station's reading, worth two frames, its family's offset moved 0.5 dB a frame
by surprise. The RadioPost mail of 2026-11-12 found it holding a steady 80 m path a rung low, and
the author asked whether the guess was too cautious.

Gate 0 measured it. On a clean path the prior is 3 dB too cautious at every rung of both airs: a
rung's 10 % point arrives 0.2–0.8 dB over its AWGN point by the reading. On a fading path it is not
cautious enough: the 10 % points lie 2.6–6.3 dB over at 2 300 Hz and 5.8–9.7 dB at 500 Hz. On the
strong 2 300 Hz paths C1 left about a fifth of the throughput unused against the best rung held
throughout (40 m Good +23 %, steady mail +22 %), at 500 Hz 3–8 %.

Gate 3 ran four candidates on the oracle (26 scenarios × ten draws, today's controller on the same
draws):

* **The try** (the prior stepping aside once a rung has its own frames, no doubled wait after a
  clean try, no aging during a try): fixed the 2026-11-12 near miss (+4.5 % on steady mail) but not
  the gap, and lost on a weak 500 Hz path. Not taken.
* **A 1 dB margin**: the gap down to +6.3 % and +6.5 %, several fading 2 300 Hz paths 7–46 %
  faster, but the 500 Hz NVIS gateway test lost its reply in 3 draws of 10: the client opened on
  OFDM under a 7 ms echo and kept going back to it.
* **1 dB with the family offset's gain doubled**: the same gains, still 2 of 10 lost on each NVIS
  test.
* **1 dB with the doubled gain on the 2 300 Hz air only**: the gains kept, both NVIS tests
  delivered in every draw, the 500 Hz air unchanged.

## 2. Decision

1. **On the 2 300 Hz air** the prior is read **1 dB** under the reading (`WIDE_PRIOR_MARGIN_DB`) and
   a frame moves its family's offset by **1 dB** (`WIDE_FAMILY_GAIN_DB`).
2. **On the 500 Hz air** the controller keeps 3 dB and 0.5 dB.
3. The air is the one the station's capability byte states (`GrantedEngine._air_guess` in the
   model, `RateController::for_air` in the port), so a station moving between airs (ADR-0026)
   takes the values with it.

The prior's weight (two frames), the try (three frames, one a transmission), the doubled wait, the
account's half-life and the slope are unchanged.

## 3. What it measured

Against today's controller, the same draws (`04-results.md` §5.3–5.4):

| check | result |
|---|---|
| K1, the gap to the best single rung on the strong 2 300 Hz paths | 40 m Good +22 % → **+6.3 %**; steady mail +22 % → **+6.5 %** |
| faster, paired | equatorial flutter 211 → 522 bit/s, VARA QRM 719 → 928, steady mail 963 → 1 104, thunderstorm ×0.88, 40 m Good ×0.93 |
| 500 Hz | today's controller: seven scenarios sent the same rungs in all ten draws |
| K3, the start pair | as today |
| K8, both NVIS echo tests | delivered in every draw |
| K4, every scenario at three draws | 190 of 192 (today 192), no radiated overlap |
| K5, the real-modem link bench | 500 Hz identical; 2 300 Hz ×0.964 pooled, AWGN ×0.67–0.79 from 0 to +12 dB |

**Three narrow misses, accepted by the author:**

* **20 m DX at 2 300 Hz** (ITU Moderate, 8 dB): over twenty draws the median 3 % slower, the
  geometric mean 4 % (K2's range ended over 1.10 on both sets of draws).
* **The link bench at 0 dB on Moderate and Poor**: ×1.071 and ×1.057 against K5's 1.05.
* **The RTTY contest, one draw of three**: four RTTY stations sit in the passband, rung 11 decoded
  1 frame in 39, and the controller kept going back to it between stretches at rungs 7–8; the
  Test's file ran out of its budget (92 bit/s; today's slowest run on this scenario 96).

Each is the same mechanism: with 1 dB of caution a rung that keeps failing is rated good again as
its failures age out of its account, and it is tried again. The reading cannot see narrowband
interference, and on a weak fading path the cheaper climb costs more failed frames. The gains on
strong and clean 2 300 Hz paths are larger than these costs.

The other K4 loss, the 500 Hz AGC echo test's draw 1031, is today's controller (the candidate
runs it there) on a draw today's also lost in oracle run 24.

## 4. What it removes and adds

* **Adds** two values for the 2 300 Hz air (`WIDE_PRIOR_MARGIN_DB`, `WIDE_FAMILY_GAIN_DB`): the
  controller's settings go from seven to nine numbers. K7 asked for no more than seven; the
  author's acceptance covers it. The evidence the per-air split rests on is Gate 0 §2's
  calibration (the 500 Hz rungs pay the larger fading penalty) and run 26 against run 28 (the same
  values on both airs lost the NVIS tests; on the 2 300 Hz air alone they did not).
* **Removes** the review's bench switches: the try's three (`prior_steps_aside`,
  `clean_try_keeps_wait`, `try_unaged`), the port's `RateReview`, the daemon's `AETHER_BENCH_LINK`,
  the oracle workflow's `bench_link` input and `candidate.yml` (it lives in the history at
  `1f847c0`).
* **Tests:** `test_a_bulk_transfer_leaves_out_headers_that_change_nothing` pins its rung, so that it
  holds R7 and not where the controller ends a transfer; the floor-answer test sends 6 000 bytes at
  2 300 Hz, where the faster controller ends a 3 000-byte transfer before a second silent slot
  (with 6 000: 5 silent slots to 1, 120 s to 65 s), and holds its time strictly on both airs.

## 5. Owed

* Long steady 2 300 Hz sessions on the air, both sides uploaded, on the build that carries this.
* The re-tries of a failing rung under narrowband interference: a rung's own failures, not only
  its family's, should keep it down. The next review's, not a patch (CLAUDE.md's rule).
* The nightly's baseline (`bench/baselines/scenarios.csv`) from the first nightly on this build.
