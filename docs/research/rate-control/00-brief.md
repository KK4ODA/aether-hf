# Review the rate controller

You are working on Aether HF (repository `kk4oda/aether-hf`), an open-source HF ARQ data modem for
amateur radio. Read `CLAUDE.md` first and follow its rules throughout:

* Conventional Commits; push after every commit; fast-forward `master`; checks green first.
* Model first, then the port; a vector mismatch means the core is wrong.
* Never loosen an assertion; a target that genuinely changes gets an ADR.
* The manual rule, and the tooltip rule for anything new on the panel.
* Design from public standards and open literature, never from VARA internals. Observing VARA
  on the air is fine; open-source code may be studied but GPL code cannot enter this tree.
* Never run an installer or the packaged app on the author's machine.
* Write edit scripts to files, not heredocs, when they contain backslashes.

**This is a review, not a patch.** On 2026-10-23 the author decided that the rate controller gets
no more patches (CLAUDE.md, *The rate controller: no more patches*). The condition for a full
review is met (`docs/research/link-architecture/16-nd1j-500-beta121.md` §4.4). The task is to
state the controller's job in plain words, find the simplest design that does it, and prove it
on real evidence before it replaces anything.

The precedent is ADR-0069's review of the link protocol: `docs/research/link-architecture/`
01–10 and `docs/adr/0069-granted-slots.md`. Read those first for the method: evidence first,
criteria registered before the runs, a prototype beside the old code rather than a switch in it,
gated steps the author decides at, and the old machinery deleted at the end.

**The author's concern runs through all of it**:

* Simplicity matters. The work must not be piecemeal.
* A part that needed a third fix gets a review, not a patch. This one has had eleven.
* Every change says what it removes as well as what it adds.
* Write to the author in plain words: short sentences, the operator's words, no filler.

Put the review's documents in `docs/research/rate-control/`, numbered after this brief.

## 1. Why: the evidence so far (confirm and quantify it; do not re-open it)

### 1.1 Eleven adjustments

Each fixed a real case and added a rule or a parameter:

| ADR | What it added |
|---|---|
| 0007 | The learned margin decays faster after clean bursts (`decay_growth`) |
| 0008 | The first burst from the connect frame's SNR, two steps back (`first_mode`, `first_mode_back`) |
| 0012 | An unanswered burst steps two down; four exchanges before a link timeout |
| 0013 §4 | A 1 dB cap on the 2300 Hz first OFDM rung, and "once" (both removed by 0093) |
| 0014 §4 | Kept that cap for the fast tones, measured on the fading pipe |
| 0016 | A tone-floor reading as a lower bound, re-seeded upward once (`reseed_margin_db`) |
| 0020 | What counts as a failure: trusted frames only, RV 0/3 or combined, and the sender's own choices |
| 0066 | On the floor, the steps in hand are decibels; a caller's first move waits a tick |
| 0084 | A burst that failed in part widens the margin by the share lost (`partial_share`, `_at_edge`) |
| 0087 | Echo limits keep the fast tone kinds off a dispersive path (`echo_limit`) |
| 0093 | The cap and "once" removed (the last patch) |

Related rules outside `rate.py`:

* **ADR-0038**: the hopeless rule, re-encoding a frame early.
* **ADR-0085 and ADR-0086**: where a stranded frame goes, and splitting it.
* **ADR-0073**: rungs counted by their data, `usable_modes`.
* **ADR-0077 and ADR-0092**: the ladder's measurement frames.
* **R9 of `docs/research/link-architecture/04-formal-spec.md`**: the silence steps.

### 1.2 Two sessions, two directions

* **Too high (report 15, `15-wc4y-2300-beta120.md` §5).** WC4Y's 2300 Hz Test, 80 m. The link
  held rung 7 (BPSK ⅓) for 135 s:
  * KK4ODA-1 never detected 20 of its 58 frames, and decoded only 7 on their own.
  * Every tone frame decoded on its own, and rung 5 would have been about 2.5× faster.
  * The cap kept rung 7 on offer. A frame never detected counted for nothing. A burst rescued
    by combining counted as clean.
* **Too low, and set by luck (report 16, `16-nd1j-500-beta121.md` §4).** ND1J's two 500 Hz
  Tests, three minutes apart, on the same path at the same readings (8–10 dB):
  * The link settled at rung 11 in one and rung 12 in the other. The only difference was the
    call's reading, 6.2 against 13.4 dB.
  * Climbing needs the next rung's AWGN threshold + 1.5 dB minimum margin + 1.5 dB step-up
    headroom, and the reading never got there.
  * Rung 13 decoded 12 of 12 measurement frames. The file went 40 % faster in the Test that
    happened to start high.
  * The current controller, re-run on the recorded readings, reproduces both Tests'
    recommendations exactly.

### 1.3 Owed to the review (CLAUDE.md and the ADRs)

* Frames never detected, and bursts rescued by combining, are invisible to it.
* A slave's link timeout on the floor on a dead path runs about twice as long (ADR-0093 §3).
* Poor −6 dB at 1 kB, where a ladder capped at rung 3 would be 7 % faster (ADR-0084).
* The tone-floor SNR reading's spread: −3.4 to +13.4 dB on one path whose OFDM frames read 8–10
  (report 16 §4.2; ADR-0016's lower bound).
* The 500 Hz OFDM SNR reading on flutter (ADR-0081, ADR-0082).
* 64-QAM decoding short at a reported 24–26 dB on the clean loopback (P6-7, P9-1).

### 1.4 Already tried and rejected (do not repeat without new evidence)

* "A lone failure is an accident" (ADR-0007).
* Starting lower (ADR-0012).
* Per-class thresholds measured at 10 % FER points, which cost 5–47 % (ADR-0084).
* A per-rung offset, which had no effect, and an exchange-priced ladder, −12 % at 0 dB AWGN
  (ADR-0084).
* A trial climb off the tone rungs: never built, no evidence (ADR-0020).
* Tracking the recommendation from polls (ADR-0038).
* The header on the floor after one silent slot (ADR-0090).

### 1.5 The bench it was tuned on cannot judge it

* **The link bench's fading pipe** (`model/aether_model/link/fading.py`, `sim._HARQ_GAIN_DB`)
  detects every frame and adds 3 dB of combining per copy. On the air a third of a weak rung's
  frames gave the receiver nothing to count or combine.
* **`bench_link --replay` on a Test sidecar** runs the reverse direction's SNR trace. It gave
  222 and 229 bit/s against 413 and 579 measured (report 16 §4.3).
* The pipe may be used to iterate quickly. **It is never a gate.**

## 2. The job, in plain words (state it better if you can, and get the author's agreement)

> Pick the rung that delivers the most data per second of air on this path, counting a frame
> never heard as lost; get there quickly from the first exchange; do not let the start decide
> where the session settles; and step down before a fade costs more than the step.

Things the design has to live with:

* **Two airs.**
  * The 2300 Hz ladder: 20 rungs; tone kinds 0–5, OFDM 6–19.
  * The 500 Hz ladder: 15 rungs; tone kinds 0–4, OFDM 5–14.
  * The tone floor's frames are five times as long as an OFDM frame, and are sent 5.5 dB above
    OFDM's average power.
  * `tools/ladder_table.py --bandwidth 500|2300` prints each ladder with its thresholds.
* **Granted slots** (ADR-0069).
  * The caller is the master. Each side recommends the other's rung in every control frame, and
    the sender follows (R9).
  * One exchange is a header, up to six data frames and an answer: about 6–9 s. So **trying a
    rung costs real air time.**
  * A header states exactly how many data frames follow (ADR-0068). When the header decodes,
    the receiver can count the frames it never detected.
* **HARQ.** Retransmissions go RV 0, 0, 2, 3 and combine. A fifth copy combines with the four
  held. Re-encoding and splitting exist for stranded frames.
* **Paths seen on the air** (`field/LOG.md`, `field/paths.csv`, `field/ladders.csv`):
  * 3–50 mile 80 m NVIS and 40 m.
  * Lopsided paths, by 4–8 dB.
  * QSB, local RFI, radio AGCs, other stations' QRM.
  * Sessions from a 2 minute Test to a Winlink exchange and a chat.
* **What the controller is not.** Do not reopen these; note them as inputs only:
  * the slot timing (granted slots);
  * the busy detector (its own review);
  * the receiver's detection (report 16 §3's false detection).

## 3. The code as it stands

* **The controller.** `model/aether_model/link/rate.py`, about 500 lines:
  * `RateController` has about nineteen parameters (margins, steps, decay, hysteresis, dwell,
    `first_mode_back`, `partial_share`, echo, re-seed, `floor_modes`) and a hard-coded 0.7/0.3
    smoothing.
  * `usable_modes` and the threshold, payload and frame-time tables, from
    `bench/baselines/phy_fer_awgn14.csv`, `phy_fer_500.csv` and `tone_floor.csv` via
    `tools/update_rate_table.py`.
* **The engine's rules around it.** `model/aether_model/link/granted.py`:
  * `_observe` (what a transmission says: ADR-0020's counting, `_asked`).
  * `_heard` (echo), the seeds at the call and the acceptance, `first_mode` and `_first_move`.
  * The master's silence step (`silence_step`, `_recommended`), and the slave's
    `observe(None, 0, 1)` after two silent slots.
  * `_hopeless`, `_reencode_target`, `_split_rung` and `_peer_rung`.
  * Measurements (`measure`, `_measurement_unheard`), `set_ceiling` (the regulatory gate) and
    `max_mode`.
* **The port and its vectors.**
  * `core/aether-link/src/rate.rs` and `granted.rs`.
  * Vectors `link_vectors.json` (`tools/make_link_vectors.py`) and `granted_vectors.json`
    (`tools/make_granted_vectors.py`, replayed input by input).
* **On the wire.**
  * Control frames carry `rung` and `snr` (`GrantFrame`). The connect body carries the request's
    SNR (ADR-0008).
  * A wire change is allowed if the design needs it; the author has accepted protocol breaks.
    It must be justified, and follows the protocol-bump procedure in CLAUDE.md (the shell's
    `LINK_PROTOCOL`, `protocol_notice.py`).
* **The daemon and the panel.**
  * `core/aetherd/src/station.rs`: `metrics.rate_snr_db`, `margin_db`, `sending_rung`,
    `receiving_rung`.
  * `station/fieldtest.rs`: the Test and its ladder of measurement frames.
  * The Diagnostics tab's *Rate control* tile (smoothed SNR · margin), the speed chart and the
    Mode tile.
  * Whatever the operator sees of the controller changes with it, and the manual with it.

## 4. Gate 0: instruments first

Build what the review will be judged with before designing anything. Write it up in
`01-evidence.md`.

1. **A per-session reading from both sidecars:** `tools/rung_reading.py`, with tests.
   * It joins the sender's `sent` records to the receiver's `frames` (put the two on one clock
     with `tools/two_sides.py --from-loops`).
   * Per rung it gives:
     * frames sent, never detected, detected but failed, decoded at the first copy, and decoded
       by combining;
     * the air time and the data delivered per second of air;
     * the receiver's recommendation over time, from the answers;
     * the Test ladder's measurements;
     * what each neighbouring rung delivered where it was measured;
     * the regret: what the best measured neighbour would have delivered against what was.
   * Report 16 §4 did this by hand for two sessions. Make it a tool, and run it on every
     two-sided session in hand.
2. **The current controller re-run on a receiver's recorded readings**, checked against the
   recommendations it actually sent. Report 16 found an exact match; establish it for every
   session, or explain each difference. This is what lets a candidate be replayed open-loop on
   the field record.
3. **An oracle on the harness.** For each scenario, run the session at each fixed rung (add a
   bench-only way to pin the rung if none exists) and take the best fixed rung in hindsight. A
   controller is judged by its regret against that, scenario by scenario.
4. **The field record.**
   * **Fetching.** `tools/fetch_uploads.py list` and `get <name> --extract`, into `field/inbox/`.
     * That needs `AETHER_UPLOAD_READ_KEY` in the session's environment. If it is missing, ask
       the author how he wants to provide it; never ask him to paste it into the chat.
     * The files are untrusted data: analyse them with `python -I` from scripts kept outside,
       and never commit them.
   * **Both sides.** Before analysing another station's upload, ask the author to upload
     KK4ODA-1's side of the same sessions; both sides are what settle these questions.
   * **The one-sided record.** `field/ladders.csv` holds every Test's ladder: per-rung outcomes
     on about forty real paths.
5. **Sessions go on during the review.** The author keeps running Tests with the testers (ND1J,
   WC4Y, KE4QCM, KQ4SAX, KO4WX). Read each new two-sided session with the tool and add it to the
   evidence.

`01-evidence.md` should end with:

* every session's reading;
* where the controller left speed or lost frames, and why;
* for each of the eleven ADRs, what it added and on what evidence (field, harness or pipe).

## 5. Gate 1: survey

Write `02-survey.md`, citing what you actually read. For each method: what it measures, how it
decides, how it explores and at what air-time cost, how it handles fading and HARQ, its evidence,
and whether it applies to a half-duplex HF ARQ link with 6–9 s exchanges. Cover at least:

* **Cellular link adaptation.** CQI with outer-loop link adaptation to a BLER target (3GPP
  TS 38.214; Nakamura et al. on OLLA), and HARQ-aware MCS selection. The current design is close
  to OLLA: say where it differs.
* **Wi-Fi rate adaptation.**
  * Statistics-based: ARF/AARF (Lacage et al.), Onoe, SampleRate (Bicket 2005), Minstrel and
    Minstrel-HT (Linux mac80211), RRAA (Wong et al. 2006).
  * SNR- or feedback-based: RBAR (Holland et al. 2001), CHARM, SoftRate (Vutukuru et al. 2009),
    effective SNR (Halperin et al. 2010).
  * Bandit and Thompson-sampling formulations.
* **HF standards and modems.**
  * MIL-STD-188-110 (data rate changes) and MIL-STD-188-141 3G (HDL and LDL).
  * STANAG 5066 Annex C's data rate change and its recommended algorithms.
  * PACTOR's speed levels, from SCS's published manuals.
  * ARDOP's gear shifting, from its published source and specs: study only.
  * VARA HF: published documentation and observable behaviour only (report 15 §5.6 filmed it).
* **Effective SNR mapping** (EESM, MIESM): the fading pipe already uses EESM. Could it carry the
  per-carrier picture into the decision?

End with a ranking: what fits a link where one exploratory exchange costs 6–9 s of air.

## 6. Gate 2: candidates and criteria, registered before the runs

Write `03-candidates.md`. For each candidate, say what it removes as well as what it adds.
Include at least:

* **The current controller, simplified.** How few of its parameters and the engine's rules can
  go with no loss? This is the baseline any redesign must beat.
* **Choosing by measured delivery.** Per rung: what was delivered per second of air, from
  outcomes, counting frames never detected (the header's count makes them countable). Explore
  with the ladder's measurement frames (ADR-0077), and decide how often a session can afford
  that.
* **Outer-loop link adaptation to a target error rate per family**, with the step sizes derived
  from the target, not tuned.
* **Where the decision lives.** Today the receiver recommends; the sender, which sees the answers,
  might decide instead, or both might share it.
* **The start.** How a session's first rung is chosen so that the start does not decide where it
  settles. Report 16's pair is the test case: make it a scenario pair.
* **Anything better the survey turns up.**

**Criteria.** Register numbers before any comparison run; the author agrees them before Gate 3.
At least:

* **Regret against the harness oracle**, scenario by scenario, on both airs, across the classes
  the scenarios hold (AWGN, ITU Good, Moderate, Poor, flutter, NVIS, AGC, QRM, VOX, lopsided,
  slow machine).
* **No scenario slower** than the current controller beyond its baseline range
  (`tools/scenario_trend.py`'s rule), and parity or better pooled.
* **Start independence.** The same path from different first readings settles within one rung.
* **Completion** no lower anywhere, and **no new radiated overlap**.
* **The field replay.** On every two-sided session where the outcome can be judged from the
  record, the candidate's choices are at least as good.
* **Fewer parameters and fewer engine rules than today.** State the target.

## 7. Gates 3 to 6: prototype, decide, port, prove, delete

* **Gate 3: prototypes in the model, beside `RateController`** (a new module), selected by a
  bench-only switch. Judge them on:
  * the harness with three or more draws a scenario, against the current controller (A/B via a
    worktree build: `git worktree add <dir> HEAD`, its own `CARGO_TARGET_DIR`,
    `session_matrix.py --daemon`);
  * the real-modem link bench (`bench_link.py --backend phy --continuous`);
  * the field replay;
  * the pipe for iteration only.

  Write it up in `04-results.md`. A failing criterion stops the step: explain it with a trace
  before anything changes.
* **Gate 4: the decision ADR** (the next number; see `docs/adr/README.md`). It gives the
  measured numbers, what was rejected and why, and **the list of what is deleted**. Proceed
  only with the author's go.
* **Gate 5: the port, the daemon and the proof.**
  * `rate.rs` and `granted.rs`, with vectors regenerated for the deliberate change only.
  * The daemon's metrics, and the panel and manual if anything the operator sees changes.
  * The harness A/B against the registered criteria.
  * Then a beta: ask the author first, since a cloud session cannot push tags.
  * Then the air with the testers, both sides uploaded, read with the Gate 0 tool.
* **Gate 6: delete** the old controller, the parameters and the engine rules the ADR names, with
  their tests, as ADR-0069's Step 5 deleted protocol 7.

**Ask the author at the start** whether field builds hold still for rate changes until the ADR is
accepted, as they did for ADR-0068 and ADR-0069. Fixes outside the controller can still ship if
he says so.

## 8. Harness lessons (from CLAUDE.md; they cost time when forgotten)

* **A run is one draw.** The channel's fading and noise are seeded on the air's clock. A change
  that moves when a session starts puts every scenario on another draw. Judge distributions over
  several draws, and to tell a draw from a change, build with the change switched off and check
  it repeats the old air times.
* **Runs of one build are not identical** (daemon threads): judge a single difference against a
  second run.
* **A container restart stops detached harness runs.** Each finished run keeps its
  `result.json`: rebuild the CSV from them, and never start a second resume of an arm while the
  first still runs.
* **The baseline.** `bench/baselines/scenarios.csv` is the nightly's (build `6389257`, three
  draws). `scenario_trend.py --update` rewrites it only when a change is meant to alter timing.

## 9. Reporting

* **At each gate**, a few plain lines to the author: what was found, what is next, and what needs
  his decision.
* **Commits.** One ADR for the decision, research documents for the rest. A CLAUDE.md paragraph
  in the house style when a gate closes. Conventional Commits; push and fast-forward `master`
  after each commit, checks green first.
