# ADR-0099: One rate controller, at the sender, and link protocol 12

**Status:** accepted for building, 2026-11-04, the author's decisions 8–14 at the end of the rate
controller's second review (`docs/research/rate-control-2/`: the brief `00-brief.md`, the evidence
`01-evidence.md`, the survey `02-survey.md`, the candidates, criteria and decisions
`03-candidates.md`, the results `04-results.md`). It amends ADR-0094: the advice that ADR kept
goes. It supersedes, for the rung a station's data goes at and for everything the advice steered,
the current controller (ADR-0007, ADR-0008's `first_mode_back`, ADR-0012's silence step on the
advice, ADR-0016's re-seed, ADR-0020's counting, ADR-0066's steps in hand, ADR-0084's partial
rule, ADR-0093's edge rule) and ADR-0094 §2.6. **Link protocol 12**: the control frame's `rung`
field changes meaning.

## 1. Context

ADR-0094 left two decision-makers. C1 chose each station's own data's rung, at the sender; the
old `RateController` made the rung each station recommended to the other, fed by the engine's
rules as before, and that advice still steered three things: a stuck frame's re-encode and split
targets, the header kept on data above it, and the size of the slave's slot. The station had two
controllers and thirty-one settings, and the nightly's failures since beta.125 traced to the
advice and the stuck-frame targets (`00-brief.md` §1.2).

The second review asked the first one's question again — pick the rung that delivers the most data
per second of air on this path, counting a frame never heard as lost — and measured four
candidates against today's pair on the oracle (`.github/workflows/oracle.yml`: 22 scenarios × 10
draws × every usable rung, paired by seed), the scenario harness and the real-modem link bench.

## 2. Decision

1. **One decision-maker.** Each station's C1 (ADR-0094 §2.1–2.5: the account, the prior, the
   choice, the try) chooses the rung its own data goes at. No station recommends a rung to the
   other, and no control frame carries a recommendation.
2. **What the advice steered, the station's own choice steers** (D1, `03-candidates.md` §2.1):
   * a stuck frame's re-encode and split targets (ADR-0085, ADR-0086) are bounded by the
     station's own choice;
   * the header is kept on data above the station's own choice — a try's frames — for ADR-0076's
     reason: a count the receiver cannot read from frames that may not decode;
   * the master sizes the slave's slot in the family the slave states in its control frames
     (`data_floor`), and **the header states that family**: the slave reckons its room as the
     header's `n` frames of it. Sizing in one family and stating another gave the first final
     check its one radiated overlap — five OFDM frames' slot read as five tone frames, 27 s, and
     the master's next header 9 s into it (`04-results.md` §4.1);
   * R9's silence step steps C1 alone.
   * **Pieces** (ADR-0086, as built): a stranded frame goes in pieces only below the station's
     choice when it last read an answer — never on silence steps alone, ADR-0086's reason: with
     the band gone, fast frames split onto the floor made each try 25 s and the master's tries
     outlasted the timed-out slave's reckoning, and the two closing identifiers overlapped (the
     prototype bounded the split by its silence-stepped choice, and the model's band-closing
     tests caught it). A block in pieces that went its copies at their rung without arriving is
     split again below that rung, in its family, whatever the choice, and every later split in the
     session goes below it: new data can decode at a rung a block's pieces never do, and with no
     advice — whose receiver counted the failed pieces — nothing else would bring the block down
     (`test_a_stuck_frame_goes_in_pieces_and_its_block_arrives`).
3. **A family refused** (D3, `03-candidates.md` §2.3): C1 refuses the OFDM family while its
   choice is on the floor after OFDM frames that delivered nothing, and a block cut at a refused
   family comes down at its next transmission, not after its copies and the wait for the
   combining copy. The family's echo limit stays unset (`04-results.md` §1.2).
4. **The start** is C1's, as since ADR-0094: its prior from the connect frame's reading.
5. **Link protocol 12.** The control frame's last byte keeps `n` in its low three bits; the five
   bits that carried the recommended rung become:

   | bits | header | answer |
   |---|---|---|
   | 7 | the slot's family: 1 when the slave's slot is sized in floor frames | 0 |
   | 6–4 | `missed`: frames of the slave's last transmission the master never detected | `missed`: frames of the master's last transmission the slave never detected |
   | 3 | 0 | 0 |

   `missed` counts the data frames a stated count says were sent and the receiver neither decoded
   nor detected (0–6); 7 when it read no count for that transmission. Only the receiver can know
   it (`02-survey.md` §1; the author's decision 9). The sender records it (its stats, the `rate`
   recording event, the panel's diagnostics); C1 does not act on it, since its account already
   counts a frame never detected as air that delivered nothing. Builds before cannot connect to
   builds after; the release notes say "Update required", and the shell's `LINK_PROTOCOL` is 12.

## 3. What was measured

D1 with D3, against today's pair, on the criteria registered before the runs (`03-candidates.md`
§3; the author relaxed K1(a) in decision 8):

| criterion | D1 + D3 | verdict |
|---|---|---|
| K1(a), median regret, 21 scenarios × 10 draws | +14.5 % and +24.2 % on two runs; today +21.8 %; the bootstrap ranges overlap | **cannot separate it from today** (`04-results.md` §4.3) |
| K1(b), lowest median delivery against today | 0.94, 0.90 | passes |
| K2, paired by seed, none slower | none slower, fresh draws included; faster on `80m-strong-moderate-500` (0.89) and AWGN (0.91) | passes |
| K3, the start pair | fails in one draw of three, the same draw and rungs as today | accepted (decision 13) |
| K4, every scenario × 3 draws | 183 of 183; radiated overlaps 0 on the fix | passes |
| K5, the real-modem link bench | every session completed; ×1.000 pooled; one point ×1.066 (one session of three) | accepted (decision 13) |
| K7 | one decision-maker; fourteen settings, against thirty-one | passes as decided (decision 11) |
| K8, the nightly's echo tests | AGC 10/10 both runs; Trimode 10/10 (today 9/10) | regret moves with the harness (§4.3) |

The harness's steps land a few blocks apart from run to run, so one build on one draw can deliver
34 or 48 bit/s; a single ten-draw median regret is not a measure to better than about ten points
(`04-results.md` §4.3; ROADMAP P9-18). The case is the rest: no slower anywhere on the paired
measure, faster where it differs, and one controller.

## 4. What is deleted

* **The old controller as a decision-maker** (`rate.py`, `rate.rs`): the margin and its bounds,
  the up and down steps, the decay schedule, the hysteresis and dwell, the stride, the jump cap,
  `first_mode_back`, the partial rule, the edge rule and the re-seed. What C1 inherited from it —
  the ladder, the AWGN thresholds and usable rungs, the echo reading (ADR-0087) and the SNR
  smoothing — becomes C1's own; C1 is the one class.
* **In the engine**: the recommendation (`_recommend`, `_recommended`, `_peer_rung`, `_asked`,
  `_own_rung`), ADR-0020's counting for the advice (trusted frames, the self-decodable RVs, the
  rungs asked for, the sender's-choice rule), the advice's seed and re-seed, and the switches that
  chose between the review's arms.
* **The review's prototypes and bench arms**: C1 at the receiver (`delivery`), C1 in both roles
  (`c1`), `delivery-s` as a name, the climb (decision 10; ROADMAP P9-17 keeps its finding),
  `family_refusal` as a switch (D3 is always on), `GrantConfig.controller`, the daemon's
  `AETHER_BENCH_LINK` switches, and the candidate and oracle workflows' inputs for them.
* **Their tests and vectors**: the old controller's own tests and vector traces, the review's
  vector sessions for the arms not kept; the kept engine's sessions are regenerated.

What stays: C1's seven settings (`half_life`, `prior_frames`, `probe_every`, `probe_wait_max`,
`slope_db`, `prior_margin_db`, `family_gain_db`), the echo reading's two (`echo_weight`,
`echo_min_snr_db`), and the engine's five (`silence_step`, `max_combines`,
`reencode_early_after`, `reencode_hopeless_db`, `floor_answer_memory`).

## 5. Consequences

* Builds before protocol 12 cannot connect to builds after it.
* Field builds, held still for rate changes since 2026-11-02, resume with the first protocol 12
  beta.
* The panel's readings that came from the advice change to the controller's own: the *Rate
  control* tile shows the smoothed reading it chooses by and the frames of the last transmission
  the other station never detected (`metrics.missed_there` in place of `margin_db`); a recording's
  `rate` event says `snr S missed M`. The manual changes with them.
* Two engine rules lose the gain their tests measured under the old controller: the floor
  answers while recent slots went silent (`floor_answer_memory`) still cut the silent slots, but
  their time gain holds at 500 Hz and ties at 2 300 Hz (45 s both); holding frames whose answer
  went unheard (`hold_unknown`) sends 1.5 % more frames at 2 300 Hz and ties in time at 500 Hz
  over the test's eight seeds. Their tests now hold what is still true — the silent slots cut, no
  slower; the hold's own mechanism — and their worth is owed, as their docstrings said under
  C1 at the sender (a candidate for removal).
* `tools/rung_reading.py --replay` replays the advice only on a session from before protocol 12,
  with `--tree` at its build.
* After the build: every scenario on the new default, the nightly's baseline
  (`bench/baselines/scenarios.csv`) refreshed from it, and the weekly speed check added
  (`03-candidates.md` §4, answer 5).
* Open, from the review: the 2 300 Hz fading paths' lost speed (ROADMAP P9-17), the harness's
  steps on the air's clock (P9-18), and the start's dependence on the call's reading (K3, shared
  with today's controller).
