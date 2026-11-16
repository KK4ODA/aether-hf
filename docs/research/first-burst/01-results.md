# The first burst after a call: results

**Status:** 2026-11-16. **Verdict: fails the registered criteria — F2, F3 and F4 — not by a narrow
margin.** F1 (fewer first-burst blocks sent again), F5 (completion) and F6 (safety) pass. The switch
stays off; the decision is the author's (§5).

## 1. What ran

As registered in `00-plan.md` (commit `7619eae`, before any run). The model and the port (`92f060e`,
`1fe98ca`) put ADR-0128 behind `GrantConfig.first_burst_try`. The daemon was built twice from that
tree, A with the default off and B with it flipped in the source (`git worktree`, its own
`CARGO_TARGET_DIR`). 25 scenarios ran: 18 at four draws and 7 at six, paired by seed, 114 runs an arm
(`tools/session_matrix.py --jobs 2`, the two arms side by side in this container). Every scenario F3
or F4 flagged was then run again, both builds, on the same draws (46 runs an arm). The measures come
from `tools/first_burst.py --compare` (each station's first data transmission is read from its own
sidecars).

## 2. The verdict per criterion

| | asked | first pass (114 pairs) | rerun of the flagged (46 pairs) | verdict |
|---|---|---|---|---|
| F1 first-burst blocks sent again | B ≤ ⅔ of A; lower in most lossy scenarios | 36.8 % → 22.6 % (×0.61); 486 → 219 s of later copies; lower in 14 of 16 | 22.0 % → 8.8 % | **pass** |
| F2 air, pooled B/A | ≤ 1.02 | **1.083** | 1.187 (the flagged only) | **fail** |
| F3 no scenario's median over 1.15 (after rerun) | none | 7 over | **6 confirmed** (below) | **fail** |
| F4 Tests, message / file | ≥ 0.97 each; none under 0.85 | **0.873 / 0.921**; 4 under | `80m-start-low-reading-500` message **0.67** confirmed | **fail** |
| F5 completion | B ≥ A, no scenario −2 | A 112, B 113 | 46 / 46 | pass |
| F6 safety | no new radiated overlap, no invariant | A 3 (one draw of `40m-gateway-busy-when-called-2300`), B 0; none broken | 0 / 0 | pass |

F3, confirmed on the rerun (median paired air B/A; first pass, then rerun):

| scenario | first pass | rerun |
|---|---|---|
| `40m-trimode-gateway-2300` | 1.19 | 1.40 |
| `15m-equatorial-flutter-2300` | 1.38 | 1.37 |
| `80m-qsb-dropout-500` | 1.31 | 1.31 |
| `80m-winlink-exchange-500` | 1.31 | 1.31 |
| `20m-dx-500` | 1.29 | 1.29 |
| `80m-steady-mail-2300` | 1.19 | 1.19 |
| `40m-good-throughput-2300` | 1.21 | 1.07 (under on the rerun) |

F4 on the rerun: `80m-start-low-reading-500` message ×0.67 (file ×0.80); `80m-strong-moderate-500`
message ×0.86, `80m-flutter-500` file ×0.86 and `20m-dx-moderate-2300` message ×0.93 came back
over 0.85, but each is still under 0.97.

Where B gained, it was on the 500 Hz NVIS and edge scenarios the change was meant for:
`80m-agc-carrier-inside-500` and `-outside-500` ×0.93, `80m-trimode-gateway-client-vanishes-500`
×0.93, `80m-nvis-disturbed-500` ×1.00, with first-burst losses 73–85 % → 54–64 %. One more A run of
`80m-agc-carrier-outside-500` missed its budget. `quick-awgn-2300` ran ×0.90. The whole first-pass
table is in §6.

## 3. Why: the climb back, not the first burst

The first burst did what was asked: fewer of its blocks went again. The cost is everything after it.
For example, the caller's data on `40m-good-throughput-2300`, draw 20 (16 kB; rungs a transmission):

```
A  18×6 | 19,18×5 | 18×3,17×3 | 17×6 | 17×5                        message 41 s
B  18,15×5 | 16,15×5 | 15,16,15×4 | 16,15×5 | 17,16×5 | 17,16×5 | 17,17,16,16   message 57 s
```

B's bold frame (rung 18) decoded. As designed, it did not move the choice, and the climb on clean
frames then went a rung per try. A try is decided on three frames (`_try_over`, one frame a
transmission). Most of the 16 kB therefore went at 15–16 where A had sent it at 18–17.

On `80m-steady-mail-2300` the caller runs at 8–9 instead of 11–12 for the whole session. On
`15m-equatorial-flutter-2300` and other 2 300 Hz paths read near 0 dB, the safe rung is the floor's
top and today's is OFDM: by the family rule (`00-plan.md` §2.2) there is no try, so the session starts
on 5.36 s floor frames and crosses the edge only after the full wait (rung 5 for the first bursts,
×1.37). The 500 Hz Winlink and QSB scenarios are short exchanges, where a few transmissions at a low
rung are most of the session.

This is what the cautious start of October cost (`rate-control/04-results.md` §3.6: K1(a) +22.6 →
+28.7 %, `quick-awgn-2300` 2 400 → 1 714 bit/s, weak scenarios under 0.85). The bold frame of
ADR-0128 measured the bold rung, but one frame outweighs nothing in C1's account (`prior_frames` = 2),
so the measurement changed no decision.

## 4. Not changed, written down

* The rule found here is C1's, not the first burst's: an upward try is decided on three frames, one a
  transmission, so from any start below the path's best rung the climb costs about three
  transmissions a rung. Not changed (the task's limit, and the rate controller rule).
* `40m-gateway-busy-when-called-2300` had three radiated overlaps in A's first pass and none in B's,
  on one draw. This is a third station's call into a session, not this change. Not investigated.
* One daemon unit test, `host::server::tests::payload_written_to_the_data_port_reaches_the_modem`,
  failed once in a full `cargo test --release --workspace` run on this branch under load, and passed
  six times alone and in the next full run (§7). The test does not touch the rate controller.

## 5. What the author decides

1. Whether to adopt it. By the registered criteria, no: B is about 8 % slower pooled and up to 40 %
   slower on six scenarios. It does fewer resends of the first burst and gains about 7 % on the
   500 Hz NVIS edge.
2. If not, whether to delete the switch and its vectors (model, port, `first_burst.py`) or keep them
   on `master` as a record. They change nothing while off.
3. Something not built and not judged, for the record. The failure is the climb, and the climb's speed
   is a C1 rule. A variant that moves the choice to the bold rung when its one frame decodes would
   rest one frame against the prior that `_try_over` was written to stop trusting
   (`rate-control/04-results.md` §3.14). It would be a new change to the controller, outside what
   was approved.

## 6. The first pass, every scenario

Medians of the paired B/A ratios. "Again" is the share of first-burst blocks sent again.

| scenario | runs | pass A | pass B | again A | again B | air B/A | msg B/A | file B/A | overlaps A/B |
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

The runs themselves (sidecars and logs; the audio of passing runs deleted) stayed in this session's
container (`/home/user/fb/`). They are not in the tree.

## 7. Checks

The model's quick suite, ruff, ruff format and mypy are green. `tools/check_vectors.py` reports every
vector file current. The existing granted sessions and controller traces are byte for byte as before;
three granted sessions and four controller traces with the switch on are appended. Each of two planted
faults in the port fails them. `cargo clippy --all-targets --all-features -D warnings` and the
`aether-link` tests are green. The first full `cargo test --release --workspace` run under the harness's load failed the host-port
test of §4. That test passed six times out of six when run alone, and the full workspace then passed
with `--no-fail-fast`: 604 of 604 `aetherd` unit tests and every integration test, the two-daemon
sessions included.
