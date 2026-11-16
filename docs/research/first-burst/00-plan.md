# The first burst after a call: start safe, try the bold rung with one frame — the plan

**Status:** registered 2026-11-16, before any run. ADR-0128 (proposed). The author approved this one
change to the rate controller as an exception to the rule of no more patches; no review follows it.
Nothing here becomes the default or ships until the author has read `01-results.md`.

## 1. The problem

Before a session has carried any data, C1 (ADR-0094, ADR-0099) chooses its rung from the reading the
call or the acceptance gave, by its prior: the AWGN curve `prior_margin_db` under the reading (1 dB on
the 2 300 Hz air, ADR-0111; 3 dB at 500 Hz), worth `prior_frames` = 2 frames. On a fading path a rung
needs 6–10 dB over its AWGN point, and a call of one or two frames often lands on a peak. A model study
on 2026-11-16 (beta.160, `3e272ad`; two `GrantedEngine`s through `TwoStationSim` and the link bench's
fading pipe; a call, 2 000 bytes, 400 back, a disconnect; the median seed of 8 per path) found the
caller's first burst lost most of its frames on five of six paths:

| Path | Call read (OFDM / tone) | First burst | Lost | Rung the channel carried |
|---|---|---|---|---|
| 2300 Hz Good +15 dB | +19 / +23 | rung 19 | 1 of 3 | ~+25 dB |
| 2300 Moderate +5 | +4 / +6 | rung 11 | 5 of 6 | ~+10 |
| 2300 Moderate −10 | −11 / −12 | rung 4 | 1 of 6 | ~−8 |
| 500 Good +10 | +11 / +11 | rung 12 | 5 of 6 | ~+17 |
| 500 Poor +2 | −3 / +3 | rung 9 | 6 of 6 | ~+8 |
| 500 Moderate −8 | −9 / −7 | rung 4 | 6 of 6 | ~−2 |

About 55 s of resending over the six exchanges came from first bursts alone. After that burst C1
corrected itself: every path settled 1–3 rungs below its start. The fading pipe detects every frame
and adds 3 dB a copy, so it shows where the cost is and cannot judge a fix: the harness judges.

**What was tried before.** The first review's C1 v3 (`rate-control/04-results.md` §3.6, 2026-10-24)
started at the four channels' mean and tried the rung above at the next transmission; it fixed the
disturbed NVIS path and cost every path where a climb is the job (K1(a) +22.6 → +28.7 %,
`quick-awgn-2300` 2 400 → 1 714 bit/s, three weak scenarios under 0.85), and was deleted. What differs
now: the bold rung is tried in the first transmission itself, with one frame, and the climb on clean
frames (`_clean_climb`, §3.25, which v3 did not have) carries the station up from the safe rung. The
harness may still say what it said in October; this plan is written so that it can.

## 2. The change (ADR-0128)

Behind `GrantConfig.first_burst_try` (model and port; default off, so today's behaviour and every
vector stay as they are), for the rate controller's first choice in a session — its seed, which both
the caller and the called station make from the first reading they get, and so each station's first
data transmission:

1. **The safe start.** The rung is the best by the four channels' mean (`prior_margin_db = None`:
   `path_decode_probability`, `CLASS_PENALTY_DB`) at the reading, or today's first rung if that is
   lower (the mean prior can rank a rung above the optimistic one's at a weak reading on the 500 Hz
   air: never start above today).
2. **The bold try.** C1's existing try is armed at the seed: one new frame of the first transmission
   goes at today's first rung (the optimistic prior's best). *Families:* a transmission carries one
   family, and a try across the floor's edge takes the whole transmission (`04-results.md` §3.16), which
   would be today's start again. So the try goes at the fastest usable rung of the **safe rung's
   family** that is no faster than today's first rung; when that is the safe rung itself (the safe
   rung is the floor's top and today's is OFDM, as at +2 dB at 500 Hz), there is no try in the first
   transmission and the climb across the edge waits as it does today.
3. **Its verdict, from its one frame.** When the first transmission's outcome comes back: the try frame
   decoded — the try ends without moving the choice, and the climb on clean frames goes on from the safe
   rung (no wait); it was lost — the try ends and that rung's next try waits twice as long (as a failed
   try does today). A first transmission that carried no frame at the try's rung ends the try with no
   verdict. A silent slot or an echo that rules the rung out ends it as any try.
4. **Nothing else changes.** Every later decision uses today's prior (the third review measured the
   mean prior for every decision as worse, ×1.095 against ×0.942, `rate-control/04-results.md` §2).

*What it removes:* nothing from the default; with the switch on, the first transmission's choice by the
optimistic prior. If the author adopts it, the decision ADR deletes the switch and the start it
replaces.

## 3. How it is judged

**Builds.** The daemon from one source tree twice, `first_burst_try` off (A) and on (B) by its default
in the source, each in its own `CARGO_TARGET_DIR`. Nothing else differs.

**Scenarios** (25), one scenario file each, as committed at the registration:

* *The quick set:* `quick-awgn-2300`, `40m-trimode-gateway-2300`, `40m-gateway-busy-when-called-2300`.
* *500 Hz NVIS and the budgets' edge:* `80m-agc-carrier-outside-500`, `80m-agc-carrier-inside-500`,
  `80m-nvis-disturbed-500`, `80m-nvis-evening-500`, `80m-trimode-client-vanishes-500`.
* *Weak and fading:* `80m-weak-500`, `80m-weak-both-ways-ladder-500`, `80m-weak-wide-test-2300`,
  `20m-polar-path-2300`, `80m-flutter-500`, `15m-equatorial-flutter-2300`, `20m-dx-moderate-2300`,
  `20m-dx-500`, `80m-qsb-dropout-500`.
* *Strong, where starting low would cost:* `40m-good-throughput-2300`, `80m-strong-moderate-500`,
  `80m-steady-mail-2300`, `80m-steady-mail-500`, `80m-start-high-reading-500`,
  `80m-start-low-reading-500`, `40m-winlink-exchange-2300`, `80m-winlink-exchange-500`.

**Draws.** Four of each (`--seeds 4`: seed, +1000, +2000, +3000), paired by seed between A and B: 100
runs an arm. Six of the five 500 Hz NVIS and edge scenarios and the start pair (`--seeds 6`), which
take another course between runs of one build (CLAUDE.md): 114 runs an arm in all.

**Measures** (per run; `tools/first_burst.py` reads them from the run's sidecars and `result.json`):

* *First-burst blocks sent again:* for each station and session, the data blocks (sequence numbers)
  of its first transmission carrying data, and how many of them it sent again later — lost on the
  first copy, or their acknowledgement lost. With it the air those later copies took.
* *Air time:* the run's air (`session_matrix.py`'s), paired by seed as B/A.
* *Test rates:* message and file bit/s, paired by seed as B/A.
* *Completion:* the run passed; radiated overlaps; invariants.

## 4. Criteria (fixed before any run)

* **F1 — first bursts.** The share of first-burst blocks sent again, pooled over every run, is at most
  two thirds of A's (a third fewer), and lower in B than in A in the majority of the scenarios where
  A's share is 25 % or more.
* **F2 — air, pooled.** The geometric mean of the paired B/A air ratios over every run that passed in
  both arms is at most 1.02 (the harness moves a single build by several percent; ×1.00 is not
  measurable on 114 pairs).
* **F3 — air, per scenario.** No scenario's median paired ratio is over 1.15. A scenario over it is
  run again, both builds, on the same draws; it fails F3 only if the rerun's median is over 1.15 too.
* **F4 — Tests.** Over the scenarios that run a Test, the geometric mean of the paired message and file
  rate ratios is at least 0.97 each, and no scenario's median is under 0.85 (the same rerun rule).
* **F5 — completion.** B passes at least as many runs as A in all, and no scenario loses two or more
  runs to A.
* **F6 — safety.** No radiated overlap in B that A does not have on the same draw, and no invariant
  broken in B.

A narrow miss goes to the author with its trace, case by case; nothing passes by rule. Everything is
reported, criteria met or not.
