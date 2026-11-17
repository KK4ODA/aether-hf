# The first burst after a call: results

**Status:** 2026-11-16. Results in for ADR-0128 and for its variant J (§8), awaiting the author.
Both sit behind switches that are off by default. Nothing ships.

## In short

**What was tried.** After a call, each station picks the rung for its first data from one reading:
how strong the call or its acceptance was. On a fading path that reading often catches a peak. The
first burst then goes too fast, and most of it is lost and sent again (`00-plan.md` §1). ADR-0128's
change was to start the first burst at a slower, safer rung and send one of its frames at today's
faster rung as a test.

**What happened.**

* **The first burst improved, as intended.** Today 37 % of its blocks have to be sent again. With the
  change, 23 %. Over all the runs, the extra copies took 219 s of air instead of 486 s.
* **Whole sessions got slower.** Over all runs, sessions took 8 % more air time. Six scenarios took
  19–40 % more, and a second run confirmed it. Test messages were 13 % slower overall. One
  scenario's message went at two thirds of today's speed.
* **Nothing broke.** Sessions completed as often (113 of 114 against 112) with no new collisions on
  the air.
* **It helped only where it was aimed.** The 500 Hz NVIS scenarios at the edge of their time budgets
  ran about 7 % faster.

**Why.** Starting low costs little; climbing back up costs a lot. From the safe rung a station
climbs one rung for every three transmissions. The test frame at the fast rung usually got through,
but by design one frame does not move the station up. On good paths most of the data therefore went
one to three rungs slower than it does today. That cost far more time than the resends saved.

**What it means.** On most paths the time is lost by sending slowly afterwards, not by losing the
first burst. A safer start pays only where today's first burst is lost almost entirely and the
session is short: the 500 Hz NVIS edge. October's cautious start ended on the same lesson
(`rate-control/04-results.md` §3.6).

**Verdict.** It fails three of the six criteria set before the runs (F2, F3, F4) by a wide margin,
not a narrow one. It passes the other three (F1, F5, F6). The switch stays off. What happens to it
is the author's decision (§5).

**Variant J (§8).** J moves the station straight to the fast rung when its test frame decodes. It
fails the same three criteria, by less: sessions took 5 % more air time overall instead of 8 %.
Wherever a test frame could be sent, J is level with today (×1.006). The whole loss comes from
first bursts that start on the tone floor with no test frame, where today starts on OFDM (×1.14).
Those floor starts also hold J's gains: the 500 Hz AGC and NVIS edge scenarios ran 11–17 % faster.

## 1. What was run

An A/B on the scenario harness, exactly as set out in `00-plan.md` before any run:

* **Two builds of the daemon** from one source tree. A has the switch off (today's behaviour) and B
  has it on. Nothing else differs. Each was built in its own `git worktree` with its own
  `CARGO_TARGET_DIR`.
* **25 scenarios.** 18 ran four times and 7 ran six times, each time on another random draw of the
  channel (seed). A and B ran the same draws, so every B run has a twin A run to compare with
  ("paired by seed"). That is 114 runs per build (`tools/session_matrix.py --jobs 2`, both builds
  side by side).
* **A second look.** The harness moves by several percent from run to run, so one bad result proves
  little. Every scenario that missed F3 or F4 on the first pass was run again on both builds with the
  same draws (46 runs per build). A scenario fails only if the second run agrees.
* **The measures** come from `tools/first_burst.py --compare`, which reads each station's first data
  transmission from its own recordings, and from the harness's results.

### The branch's history

This work started as ADR-0127. While the harness ran, the data identifier took that number on
`master` (`45797a2`…`2fb8102`). This work became ADR-0128, and its commits were rebased onto the
identifier's.

The commits as registered and run are kept on GitHub as the branch `first-burst-as-registered`
(`8d3fe9d`). There the plan, `7619eae`, comes before every run, then the model (`92f060e`) and the
port (`1fe98ca`). Both daemons were built from that tree. On `master` the same commits are `cbce7c6`,
`e42cca3`, `346853a` and `2cd7ed9`.

The identifier work is a Setup choice with Morse the default, and it does not touch the rate
controller. On the rebased tree the vectors regenerate as `master`'s plus this change's added cases,
and every check is green.

## 2. The six criteria, one by one

The criteria were fixed before the runs (`00-plan.md` §4). "B/A" is B's figure divided by A's. For
air time, 1.10 means B took 10 % longer. For speeds, 0.90 means B was 10 % slower.

| | What it asks | Result | Verdict |
|---|---|---|---|
| **F1** First burst | B sends at least a third fewer first-burst blocks again, and fewer in most scenarios where A sends a quarter or more again | 36.8 % → 22.6 % (×0.61). Fewer in 14 of those 16 scenarios. On the rerun: 22.0 % → 8.8 % | **pass** |
| **F2** Air time, overall | B's sessions take no more than 2 % longer (×1.02) | ×1.083: 8.3 % longer. The rerun scenarios alone: ×1.187 | **fail** |
| **F3** Air time, each scenario | No scenario takes more than 15 % longer, after the rerun | 7 over on the first pass, 6 still over on the rerun | **fail** |
| **F4** Test speeds | Messages and files lose no more than 3 % overall, and no scenario loses more than 15 % | Messages ×0.873, files ×0.921. Four scenarios under 0.85, one still under on the rerun (×0.67) | **fail** |
| **F5** Completion | B completes at least as many runs as A | A 112, B 113 of 114. Rerun 46 of 46 | pass |
| **F6** Safety | No collision on the air in B that A lacks, and no invariant broken | A had 3 radiated overlaps, all on one draw. B had none. No invariant broken | pass |

F1 is what the change was built for, and it worked. F2–F4 measure what it cost, and the cost is
larger than the saving. F5 and F6 show it is safe, only slower.

### The scenarios that are slower (F3)

The median of each scenario's paired B/A air time. 1.40 means B's sessions took 40 % longer.

| Scenario | The path | First pass | Rerun |
|---|---|---|---|
| `40m-trimode-gateway-2300` | Winlink Express through an RMS Trimode gateway | 1.19 | **1.40** |
| `15m-equatorial-flutter-2300` | flutter across the equator, read near 0 dB | 1.38 | **1.37** |
| `80m-qsb-dropout-500` | Moderate +8 dB, the path drops out mid-transfer | 1.31 | **1.31** |
| `80m-winlink-exchange-500` | a Winlink-shaped exchange | 1.31 | **1.31** |
| `20m-dx-500` | a VarAC-style contact, chat lines and a file | 1.29 | **1.29** |
| `80m-steady-mail-2300` | a long message on a steady strong path | 1.19 | **1.19** |
| `40m-good-throughput-2300` | Good +20 dB, 16 kB and 4 kB back | 1.21 | 1.07 (passes on the rerun) |

### Tests that are slower (F4), on the rerun

* `80m-start-low-reading-500`: message ×0.67, file ×0.80. Confirmed under 0.85.
* `80m-strong-moderate-500` message ×0.86, `80m-flutter-500` file ×0.86, `20m-dx-moderate-2300`
  message ×0.93: back over 0.85 on the rerun, but each still under the 0.97 F4 asks for overall.

### Where B helped

The change was meant for the 500 Hz NVIS paths at the edge of their time budgets, where today's first
burst is mostly lost. There it helped a little:

| Scenario | Air B/A | First-burst blocks sent again, A → B |
|---|---|---|
| `80m-agc-carrier-inside-500` | 0.93 | 85 % → 64 % |
| `80m-agc-carrier-outside-500` | 0.93 | 78 % → 56 % |
| `80m-trimode-gateway-client-vanishes-500` | 0.93 | 56 % → 38 % |
| `80m-nvis-disturbed-500` | 1.00 | 73 % → 54 % |

One A run of `80m-agc-carrier-outside-500` also missed its time budget, and no B run did.
`quick-awgn-2300` ran ×0.90 as well. The whole first-pass table is in §6.

## 3. Why it was slower: the climb back

The first burst did its job. The time was lost after it.

Here is the calling station's data on `40m-good-throughput-2300`, draw 20, a 16 kB message. Each
group between bars is one transmission. "18×6" means six frames at rung 18. A higher rung is faster.

```
A  18×6 | 19,18×5 | 18×3,17×3 | 17×6 | 17×5                        message 41 s
B  18,15×5 | 16,15×5 | 15,16,15×4 | 16,15×5 | 17,16×5 | 17,16×5 | 17,17,16,16   message 57 s
```

* A starts at rung 18 and stays at 17–18: five transmissions, 41 s.
* B starts at the safe rung 15 and sends one test frame at 18. That frame decoded, so rung 18 worked.
* By design (`00-plan.md` §2, step 3), a decoded test frame ends the test without moving the station
  up. In C1's account one frame also weighs less than its starting guess, which counts as two frames
  (`prior_frames` = 2). The measurement changed no decision.
* B then climbs the usual way: one frame at the next rung in each transmission, and a move up once
  three of them have decoded (`_try_over`). Getting from 15 to 16 took three transmissions, and the
  message ended before B was back at 18. Most of the 16 kB went at 15–16 where A sent it at 17–18:
  seven transmissions, 57 s.

The other slow scenarios lose time the same way, in three shapes:

* **Strong 2300 Hz paths.** On `80m-steady-mail-2300` the caller runs at rungs 8–9 for the whole
  session, where today it runs at 11–12.
* **2300 Hz paths read near 0 dB**, such as `15m-equatorial-flutter-2300`. Here the safe rung is the
  top of the tone floor (rung 5) and today's first rung is an OFDM rung. A transmission carries one
  family only, tone or OFDM, so there is no room for an OFDM test frame (`00-plan.md` §2.2). The
  session starts on slow 5.36 s tone frames and moves to OFDM only after C1's usual wait.
* **Short 500 Hz exchanges** (the Winlink and QSB scenarios). The session is only a few
  transmissions, so a few at a low rung are most of it.

**This was seen before.** In October the first rate review tried a cautious start and rejected it
for the same reason (`rate-control/04-results.md` §3.6): the gap to the best fixed rung grew from
+22.6 % to +28.7 %, and `quick-awgn-2300` fell from 2 400 to 1 714 bit/s. ADR-0128's one test frame was
meant to avoid that cost. It measured the fast rung correctly, but nothing acted on the measurement,
so the cost came back.

## 4. Seen on the way, not changed

* **The real limit is C1's, not the first burst's.** An upward try is decided on three frames, one per
  transmission. From any start below the path's best rung, the climb costs about three transmissions
  per rung. Left as it is: it was outside this task, and the rate controller rule covers it.
* **Collisions in A.** `40m-gateway-busy-when-called-2300` had three radiated overlaps in A's first
  pass and none in B's, all on one draw. They come from a third station calling into a session, not
  from this change. Not investigated.
* **A flaky daemon test.** `host::server::tests::payload_written_to_the_data_port_reaches_the_modem`
  failed once in a full `cargo test --release --workspace` run under the harness's load. It passed
  six times alone and in the next full run (§7). It does not touch the rate controller.

## 5. What the author decides

1. **Adopt it?** By the registered criteria, no. B is about 8 % slower overall and up to 40 % slower
   on six scenarios. In return it sends less of the first burst again, and it is about 7 % faster on
   the 500 Hz NVIS edge.
2. **If not, delete the switch or keep it?** It can go with its vectors (model, port,
   `tools/first_burst.py`), or stay on `master` as a record. While it is off it changes nothing.
3. **A variant not built or judged, for the record.** The time goes in the climb, and the climb's
   speed is a C1 rule. A variant could move the station straight to the fast rung when its test frame
   decodes. That would trust one frame over C1's starting guess, which is what the three-frame rule
   was written to stop: in October one lucky frame put seventeen frames on a rung that a 7 ms echo
   kills (`rate-control/04-results.md` §3.14). It would be a new change to the rate controller,
   outside what the author approved.

## 6. The first pass, every scenario

Medians of the paired B/A ratios. "Again" is the share of first-burst blocks sent again. Air over 1
and speeds under 1 mean B was worse. A blank speed means the scenario runs no Test.

| scenario | runs | passed A | passed B | again A | again B | air B/A | message B/A | file B/A | overlaps A/B |
|---|---|---|---|---|---|---|---|---|---|
| 15m-equatorial-flutter-2300 | 4 | 4 | 4 | 11% | 3% | 1.38 |  |  | 0/0 |
| 20m-dx-500 | 4 | 4 | 4 | 19% | 13% | 1.29 |  |  | 0/0 |
| 20m-dx-moderate-2300 | 4 | 4 | 4 | 56% | 25% | 1.08 | 0.73 | 0.95 | 0/0 |
| 20m-polar-path-2300 | 4 | 4 | 4 | 3% | 0% | 1.11 |  |  | 0/0 |
| 40m-gateway-busy-when-called-2300 | 4 | 4 | 4 | 34% | 22% | 1.15 |  |  | 3/0 |
| 40m-good-throughput-2300 | 4 | 4 | 4 | 27% | 0% | 1.21 |  |  | 0/0 |
| 40m-trimode-gateway-2300 | 4 | 4 | 4 | 62% | 62% | 1.19 |  |  | 0/0 |
| 40m-winlink-exchange-2300 | 4 | 4 | 4 | 50% | 50% | 1.04 |  |  | 0/0 |
| 80m-agc-carrier-inside-500 | 6 | 6 | 6 | 85% | 64% | 0.93 |  |  | 0/0 |
| 80m-agc-carrier-outside-500 | 6 | 5 | 6 | 78% | 56% | 0.93 |  |  | 0/0 |
| 80m-flutter-500 | 4 | 4 | 4 | 0% | 0% | 1.08 | 0.88 | 0.85 | 0/0 |
| 80m-nvis-disturbed-500 | 6 | 6 | 6 | 73% | 54% | 1.00 |  |  | 0/0 |
| 80m-nvis-evening-500 | 6 | 6 | 6 | 77% | 54% | 1.03 | 0.97 | 1.04 | 0/0 |
| 80m-qsb-dropout-500 | 4 | 4 | 4 | 17% | 5% | 1.31 |  |  | 0/0 |
| 80m-start-high-reading-500 | 6 | 6 | 6 | 17% | 6% | 1.01 | 0.88 | 0.97 | 0/0 |
| 80m-start-low-reading-500 | 6 | 6 | 6 | 9% | 3% | 1.13 | 0.69 | 0.74 | 0/0 |
| 80m-steady-mail-2300 | 4 | 4 | 4 | 0% | 4% | 1.19 |  |  | 0/0 |
| 80m-steady-mail-500 | 4 | 4 | 4 | 75% | 71% | 1.13 |  |  | 0/0 |
| 80m-strong-moderate-500 | 4 | 4 | 4 | 33% | 8% | 1.00 | 0.80 | 0.95 | 0/0 |
| 80m-trimode-gateway-client-vanishes-500 | 6 | 5 | 5 | 56% | 38% | 0.93 |  |  | 0/0 |
| 80m-weak-500 | 4 | 4 | 4 | 38% | 6% | 1.04 |  |  | 0/0 |
| 80m-weak-both-ways-ladder-500 | 4 | 4 | 4 | 29% | 18% | 1.00 | 0.97 | 0.96 | 0/0 |
| 80m-weak-wide-test-2300 | 4 | 4 | 4 | 33% | 0% | 1.00 | 1.00 | 1.00 | 0/0 |
| 80m-winlink-exchange-500 | 4 | 4 | 4 | 42% | 30% | 1.31 |  |  | 0/0 |
| quick-awgn-2300 | 4 | 4 | 4 | 6% | 0% | 0.90 |  |  | 0/0 |

The runs themselves (recordings and logs; the audio of passing runs deleted) stayed in that
session's container (`/home/user/fb/`). They are not in the tree.

## 7. Checks

* **The model.** The quick test suite, ruff, ruff format and mypy are green.
* **The vectors.** `tools/check_vectors.py` reports every vector file current. The existing granted
  sessions and controller traces are byte for byte as before. Three granted sessions and four
  controller traces with the switch on were added, and each of two faults planted in the port fails
  them.
* **The Rust.** `cargo clippy --all-targets --all-features -D warnings` and the `aether-link` tests are
  green. The first full `cargo test --release --workspace` run, under the harness's load, failed the
  host-port test of §4. That test passed six times out of six alone. The full workspace then passed
  with `--no-fail-fast`: 604 of 604 `aetherd` unit tests and every integration test, the two-daemon
  sessions included.

## 8. Variant J: straight to the bold rung (registered in `00-plan.md` §5)

The author asked for the variant §5.3 named. J is ADR-0128 as built, except that a decoded first try
moves the choice to its rung at once (`first_burst_jump`, model `8eda89f`, port `9d1a2a7`, the
registration `96114cc` before any run). Both daemons came from that tree: A′ with the switches off,
J with both on. They ran the same 25 scenarios and draws as §1, 114 pairs, and then the seven
flagged scenarios again on the same draws (30 pairs). A container restart stopped both arms at
102/97 runs. The 29 missing runs were deleted and run again from scratch (`/home/user/fb/resume.sh`);
no half-finished run was kept.

*History.* While J ran, master gained the data identifier's daemon side and beta.161 (`5934759`…
`c661de2`), and J's commits were rebased onto it. As run, the branch is kept on GitHub as
`first-burst-jump-as-registered` (`8d90c5e`): the registration `96114cc` comes before every run,
and A′ and J were built from `9d1a2a7`. The identifier is a Setup choice with Morse the default.
On the rebased tree the vectors regenerate unchanged and every check is green.

**Verdict: fails F2, F3 and F4; F1, F5 and F6 pass.**

| | asked | first pass (114 pairs) | rerun of the flagged (30 pairs) | verdict |
|---|---|---|---|---|
| F1 first-burst blocks sent again | ≤ ⅔ of A′'s | 37.3 % → 20.1 % (×0.54); 509 → 192 s of later copies; lower in 13 of 15 | 25.0 % → 8.5 % | **pass** |
| F2 air, pooled | ≤ 1.02 | **1.051** (B: 1.083) | 1.182 (the flagged only) | **fail** |
| F3 no scenario over 1.15 | none | 5 over | **4 confirmed**: `15m-equatorial-flutter-2300` 1.33, `80m-qsb-dropout-500` 1.31, `20m-dx-500` 1.29, `80m-winlink-exchange-500` 1.28 (`20m-dx-moderate-2300` 1.18 → 1.13) | **fail** |
| F4 Tests | ≥ 0.97; none under 0.85 | **0.855 / 0.872** | **confirmed**: `20m-dx-moderate-2300` message 0.41 (file 0.58), `80m-start-low-reading-500` 0.75, `80m-strong-moderate-500` 0.82 | **fail** |
| F5 completion | J ≥ A′, no scenario −2 | A′ 111, J 114 | 30 / 30 | pass |
| F6 safety | nothing new | 0 / 0 radiated overlaps, no invariant broken | 0 / 0 | pass |

**What J fixed.** The strong paths that sank B are even or better. Against A′, J ran
`40m-good-throughput-2300` ×1.10 (B ×1.21), `80m-steady-mail-2300` ×1.03 (B ×1.19),
`40m-trimode-gateway-2300` ×0.96 (B ×1.19) and `quick-awgn-2300` ×0.86 (B ×0.90). The 500 Hz AGC and
NVIS edge scenarios, which the change was for, gained more than under B: ×0.83–0.89, and every
`80m-trimode-gateway-client-vanishes-500` run passed (A′ 4 of 6). On `40m-good-throughput-2300`,
draw 20, the caller's first transmission went `18,15×5` and its second at 18 throughout.

**What it did not fix: the start across the floor's edge.** J changes only what happens after a try,
and when the safe rung is on the floor and today's first rung is OFDM there is no try (`00-plan.md`
§2.2). The first pass splits cleanly on that:

| J's first burst against A′'s | runs passed in both | air J/A′, geometric mean | median |
|---|---|---|---|
| on the floor with no try, where A′ started on OFDM | 39 | ×1.14 | ×1.17 |
| every other start | 72 | ×1.006 | ×1.00 |

Wherever a same-family try was possible, J is level with today. Its pooled loss, and every F3 and F4
failure, comes from the floor starts. An example is `20m-dx-moderate-2300`, draw 1051, the Test's
session. A′ started at rung 14. J started at the floor's top (rung 5), sent its first 70 s on 5.36 s
floor frames, crossed to OFDM at 7 after the full wait, and climbed a rung a try to 11: message
195 bit/s against 707.

**But the floor starts also hold J's gains.** The 39 floor starts include the AGC and NVIS edge
scenarios: `80m-agc-carrier-inside-500` and `-outside-500` four runs each, `80m-nvis-evening-500`,
`80m-nvis-disturbed-500` and `80m-trimode-gateway-client-vanishes-500`. There a floor start is right:
a 7 ms echo kills OFDM. A J that kept today's start whenever the edge lies between the two rungs
would be about level with today (×1.006 on the other starts) and would give those gains up. It was
not built: it would be another change, and the author's call.

### 8.1 J against A′, every scenario (first pass)

Medians of the paired ratios. "Again" is the share of first-burst blocks sent again.

| scenario | runs | pass A | pass B | again A | again B | air B/A | msg B/A | file B/A | overlaps A/B |
|---|---|---|---|---|---|---|---|---|---|
| 15m-equatorial-flutter-2300 | 4 | 4 | 4 | 11% | 3% | 1.33 |  |  | 0/0 |
| 20m-dx-500 | 4 | 4 | 4 | 19% | 13% | 1.29 |  |  | 0/0 |
| 20m-dx-moderate-2300 | 4 | 4 | 4 | 59% | 13% | 1.18 | 0.41 | 0.57 | 0/0 |
| 20m-polar-path-2300 | 4 | 4 | 4 | 3% | 0% | 1.12 |  |  | 0/0 |
| 40m-gateway-busy-when-called-2300 | 4 | 4 | 4 | 40% | 7% | 0.99 |  |  | 0/0 |
| 40m-good-throughput-2300 | 4 | 4 | 4 | 24% | 2% | 1.10 |  |  | 0/0 |
| 40m-trimode-gateway-2300 | 4 | 4 | 4 | 62% | 62% | 0.96 |  |  | 0/0 |
| 40m-winlink-exchange-2300 | 4 | 4 | 4 | 50% | 50% | 1.01 |  |  | 0/0 |
| 80m-agc-carrier-inside-500 | 6 | 6 | 6 | 88% | 37% | 0.85 |  |  | 0/0 |
| 80m-agc-carrier-outside-500 | 6 | 5 | 6 | 79% | 56% | 0.83 |  |  | 0/0 |
| 80m-flutter-500 | 4 | 4 | 4 | 9% | 0% | 1.07 | 0.86 | 0.99 | 0/0 |
| 80m-nvis-disturbed-500 | 6 | 6 | 6 | 73% | 54% | 1.00 |  |  | 0/0 |
| 80m-nvis-evening-500 | 6 | 6 | 6 | 73% | 58% | 1.01 | 0.95 | 1.02 | 0/0 |
| 80m-qsb-dropout-500 | 4 | 4 | 4 | 17% | 5% | 1.31 |  |  | 0/0 |
| 80m-start-high-reading-500 | 6 | 6 | 6 | 19% | 6% | 0.99 | 0.92 | 0.96 | 0/0 |
| 80m-start-low-reading-500 | 6 | 6 | 6 | 6% | 3% | 1.11 | 0.76 | 0.73 | 0/0 |
| 80m-steady-mail-2300 | 4 | 4 | 4 | 0% | 4% | 1.03 |  |  | 0/0 |
| 80m-steady-mail-500 | 4 | 4 | 4 | 75% | 71% | 1.09 |  |  | 0/0 |
| 80m-strong-moderate-500 | 4 | 4 | 4 | 33% | 8% | 0.98 | 0.82 | 1.01 | 0/0 |
| 80m-trimode-gateway-client-vanishes-500 | 6 | 4 | 6 | 56% | 35% | 0.89 |  |  | 0/0 |
| 80m-weak-500 | 4 | 4 | 4 | 38% | 6% | 0.98 |  |  | 0/0 |
| 80m-weak-both-ways-ladder-500 | 4 | 4 | 4 | 29% | 24% | 1.01 | 1.04 | 0.97 | 0/0 |
| 80m-weak-wide-test-2300 | 4 | 4 | 4 | 33% | 0% | 1.00 | 1.00 | 1.00 | 0/0 |
| 80m-winlink-exchange-500 | 4 | 4 | 4 | 42% | 30% | 1.28 |  |  | 0/0 |
| quick-awgn-2300 | 4 | 4 | 4 | 6% | 0% | 0.86 |  |  | 0/0 |
