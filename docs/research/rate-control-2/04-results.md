# Gate 3: the candidates on the harness

*2026-11-03/04. The outcome is D1 with D3 (the author's decision 8, `03-candidates.md` §6); the climb is parked (decision 10). The last checks on D1 + D3 are §4; the decision record waits on the author's reading of them. The order is the author's (`03-candidates.md` §4, answer 4): D3 first,
then D1 against D2, then the dip and the climb. Each is judged on the criteria of
`03-candidates.md` §3 against today's pair (`01-evidence.md` §1, `bench/baselines/
rate_oracle_beta127.csv`).*

## 1. D3: a family refused

### 1.1 What was built

Bench only, off by default: `GrantConfig.family_refusal` in the model and the port, turned on in
a simulated daemon by `AETHER_BENCH_LINK=family_refusal` (`aetherd`'s `bench_link`; a daemon
with a radio never reads it) and in the oracle workflow by its `bench_link` input. With it:

1. **The echo.** Every OFDM rung gets the family's echo limit (`FAMILY_ECHO_LIMIT`, model and
   port), so above it both controllers and the start keep off OFDM with no new rule
   (`_echo_limits`). The floor's own limits (500 Hz rungs 3–4) stand.
2. **Outcomes.** C1 at the sender refuses the OFDM family while its choice is a floor rung and
   the last OFDM frames it sent delivered nothing (`DeliveryController.refuses`).
3. **Stuck blocks.** A block cut at a refused family comes down at its next transmission — whole
   to the most robust floor rung that fits it, or in pieces — instead of after
   `max_combines` copies and the wait for the combining copy (`_data_frames`).

Item 2 and the inheritance cap differ from the text of `03-candidates.md` §2.3; the reasons are
recorded there.

### 1.2 The family's echo limit: not set

`tools/bench_echo.py --harq` measured every floor rung and OFDM rungs 5, 6, 7 and 9 of the
500 Hz air on `bench_echo`'s 22 paths at 10 and 20 dB, 16 blocks a point, each block sent until
it decoded or four times (RV 0, 0, 2, 3) and combined, every copy through its own fade. Priced
per second of an exchange with the answers in the data's own family, **OFDM's best rung holds
level with the floor's best on a 7 ms echo**: ITU NVIS 43 against 44 bit/s at 10 dB and 57
against 42 at 20 dB; two rays 7 ms apart 48 against 52 and 51 against 42; 8 ms apart 52 against
40 and 69 against 40. The echo reading cannot tell the paths apart on this measure: OFDM loses
from high-latitude moderate (a reading of 0.0019) and wins again at 8 ms (0.0194), so
`--family-limits` sets no limit (`FAMILY_ECHO_LIMIT` stays empty). The rows are
`bench/baselines/echo_harq.csv`; the 2 300 Hz half was not run to its end (a container restart
stopped it at rung 1) and is not owed: the limit is not taken from this bench.

The harness says otherwise. On the Trimode NVIS test at 8 dB (the oracle's per-rung rows of
draws 0–2), data pinned at OFDM rungs 5, 6 and 7 lost the send in two draws of three each, and at
rungs 10 and 12 in all three, every loss a link timeout; rung 2 delivered in all three. In one
rung-5 session the sender's 17 ordinary control frames
(its headers) were never detected at the other end, 21 of its 142 OFDM data copies decoded, and
none of the gateway's 30 OFDM replies decoded the other way. The floor's frames came through
(`01-evidence.md` §1: rung 2 delivered 40–45 bit/s in every draw). What the bench leaves out is
what makes the difference there: an OFDM transmission's header and answers in the ordinary
family, and copies that share the session's one fade. A limit drawn from it would be no limit,
which the harness contradicts. **Item 1 is not set from this bench;** if items 2–3 leave the
NVIS cost standing, the limit is measured on the harness itself: the oracle on two-ray paths
across the delays, each run's echo read from its sidecars.

### 1.3 On the harness

D3 as committed (`5d57f18`: items 2 and 3, no family limit), the oracle workflow with
`bench_link = family_refusal`, ten draws, every rung (runs 5–7 of 2026-11-03; per draw in
`data/oracle_d3.csv`, the base in `bench/baselines/rate_oracle_beta127.csv`).

**Where it acted.** On ten of the 22 scenarios every draw came out exactly as the baseline's,
bit/s for bit/s: the harness repeats a draw when the controller makes the same choices, and D3
never fired there (all the 2300 Hz regret scenarios but three, and the 500 Hz paths without
echo). It changed draws on twelve, nine of them 500 Hz paths, in both directions.

| criterion | today | D3 | verdict |
|---|---|---|---|
| K1(a), median regret, 21 scenarios | +22 % | +20 % (bootstrap +12 … +28 %) | far from +11 %: D3 is a component, not a controller |
| K1(b), no scenario's median delivery 15 % under today's | — | lowest `80m-nvis-evening-500` 40 against 44 bit/s (−9 %) | passes |
| K8 Trimode: (i) all delivered in budget | 9 of 10 | **10 of 10** | passes |
| K8 Trimode: (ii) median regret ≤ +20 % | +25 % | **+9 %** | passes |
| K8 Trimode: (iii) time ratio | — | 0.90 (0.71–1.08) | passes |
| K8 AGC: (i) | 10 of 10 | 10 of 10 | passes |
| K8 AGC: (ii) | +10.4 % | +20.5 % | **narrow miss** (below) |
| K8 AGC: (iii) | — | 0.99 (0.85–1.12) | fails as registered (below) |
| K2, the 22 scenarios (the 21 and the Trimode test), range below 1.10 | — | 10 pass, 12 "confirm on fresh draws" | **the criterion is at fault** (below) |

**The AGC test's regret.** Neither median moved: the controller delivered 36 bit/s on both
builds, the best fixed rung 42 and 43. The pinned runs have D3's stuck-block rule too, and both
the controller's draws and the best rung's moved by up to 14 and 6 bit/s, both ways; the median of
their ratio went from +10 % to +20 % on which draws the moves fell in. Ten draws do not tell this
from D3; it is the case-by-case call the criteria leave to the author.

**K2 as registered cannot pass identical runs.** Of the ten scenarios whose ten draws were exactly
the baseline's, five get a 90 % range whose top end is above 1.10 — the equatorial flutter
0.74–1.35, the VARA QRM 0.82–1.23 — because the unpaired range is set by the
spread between draws (`40m-good-throughput-2300` delivers 1 829–2 909 bit/s over its ten), not by
any difference between the builds. Fresh draws do not help: the spread is the scenario's. The
reason it was registered unpaired (`03-candidates.md` §3: two controllers on one draw drift onto
different stretches of the fade once their choices differ) does not make a paired test wrong: the
seed is fixed before the run, pairs are independent across draws, and under "no effect" the two
builds' runs on a seed are exchangeable, so the exact Wilcoxon signed-rank test and its
Hodges–Lehmann range hold; the drift only adds spread to the differences once the choices part.
Paired on the seed (`tools/oracle_report.py compare --paired`), the identical scenarios read
1.00 (1.00–1.00), and three read "may be
slower": the AGC test 0.98 (0.87–1.12), `80m-slow-machine-500` 1.00 (0.98–1.13) and the Trimode
test 0.90 (0.51–1.12, its range widened by the baseline's lost draw). Changing K2 to the paired
form is the author's call: it changes a criterion registered before the runs.

**Reading.** D3 removes a failure — the Trimode test's lost send — and brings that test's regret
from +25 % to +9 %; elsewhere it moves draws both ways by amounts the ten draws cannot tell from
the drift. It adds no tunable. Item 1, the echo limit, was not built (§1.2).

## 2. D1 against D2

Runs 8–10 (D1: `family_refusal,controller=d1`) and 11–13 (D2: `family_refusal,controller=c1`) of
2026-11-03, on `76dc97d`, each run's switch and draws confirmed from its own logs; ten draws, every
rung pinned on the candidate's build. Per draw in `data/oracle_d1.csv` and `data/oracle_d2.csv`.
Both carry D3 (answer 7). K2 is paired by seed (answer 6).

| criterion | today | D3 alone | D2 | D1 |
|---|---|---|---|---|
| K1(a) median regret, 21 scenarios (≤ +11 %) | +21.8 % | +20.5 % | +18.2 % | **+14.5 %** (bootstrap +11 … +32 %) |
| K1(b) lowest median delivery against today (≥ 0.85) | — | 0.91 | **0.84** `80m-nvis-evening-500` | 0.94 `80m-flutter-500` |
| K2, "may be slower" (90 % range above 1.10) | — | 3 | 3: RTTY crashes 1.03 (1.00–1.13), NVIS evening 1.09 (1.01–1.18), Trimode 0.96 (0.65–1.12) | 3: lopsided acks 1.00 (1.00–1.12), weak both ways 1.00 (1.00–1.27), thunderstorm 1.04 (0.97–1.11) |
| K2, faster | — | — | AWGN 0.91 | AWGN 0.91, Trimode 0.81 (0.75–1.05) |
| K8 AGC: delivered, regret, time ratio | 10/10, +10.4 % | 10/10, +20.5 %, 0.98 (0.87–1.12) | 10/10, +8.6 %, 0.98 (0.85–1.08) | **10/10, +3.6 %, 0.95 (0.85–1.01)** |
| K8 Trimode | 9/10, +25.4 % | 10/10, +8.6 %, 0.90 (0.51–1.12) | 10/10, +17.0 %, 0.96 (0.65–1.12) | **10/10, +13.5 %, 0.81 (0.75–1.05)** |
| K7 | 31 tunables, two controllers | — | two controllers (C1 twice) | one decision-maker; the old controller unread (deleted at Gate 6) |

**D1 against D2 directly**, paired (D1's time over D2's): D1 ahead on the NVIS evening path, 0.90
(0.82–1.00), and the Trimode test, 0.90 (0.81–1.08); D2 ahead on the slow machine, 1.11
(0.97–1.21), and on 80 m flutter, 1.05 (0.98–1.13); the other eighteen within a few hundredths.

**What the K1 median hides.** Scenario by scenario D1 is close to today: its median regret is
+36 % over the nine 2 300 Hz scenarios (today +34 %) and +11.7 % over the twelve 500 Hz ones
(today +12.0 %). The 21-scenario median moved from +21.8 % to +14.5 % because scenarios near the
middle moved: `80m-strong-moderate-500` +22 → +12 %, `80m-flutter-500` +30 → +26 %, the AGC test
+10 → +4 %. Where the regret is large neither candidate acts: the equatorial flutter (+232 %), the
polar path (+92 %), the VARA QRM (+48 %), the thunderstorm and the RTTY crashes (+36 … +39 %), all
2 300 Hz paths where the best fixed rung is an OFDM rung in the middle of the ladder that the
controller does not climb to. That is §2.4's climb, next in the agreed order.

**D1's three K2 cases**, draw by draw against today: on the lopsided-acks path two draws of ten
went slower (91 → 71 and 126 → 104 bit/s), six were identical; on the weak-both-ways ladder two
(38 → 30 and 75 → 40), eight identical; the thunderstorm moved both ways (four up, five down). K2's
rule runs each again on ten fresh draws (draws 10–19), today's build on the same seeds for the
pairing: runs 14 (D1) and 15 (today; its lopsided-acks job lost its runner and was run again).
Paired, D1 over today: the lopsided acks 1.00 (1.00–1.00), the thunderstorm 1.00 (0.98–1.03), the
weak-both-ways ladder 1.00 (1.00–1.06). **All three pass: D1 meets K2.**

**Reading.** D1 leads: lower regret than D2 and today, K1(b) and every part of K8 met, D2's K1(b)
miss avoided, and one decision-maker where D2 keeps two. Neither meets K1(a) — the regret that
is left belongs to the climb (§2.4), which is the next step, on D1.

## 3. The climb, on D1

### 3.1 Why D1 does not climb

Read from D1's own runs (run 8, draws 0–2) at the master, the station whose data is measured.

* **The equatorial flutter path** (15 m, 10 Hz flutter at 10 dB; best fixed rung 10, 800 bit/s).
  Every draw read the call on the floor at −1 … +1 dB, the floor's reading falling short on fast
  fading. From it the sender started at the floor's top (rung 5) in two draws and sent the whole
  1 500-byte message there in two transmissions, before a try's wait (six full transmissions at
  the rung tried) ran out: 211 bit/s, exactly what rung 5 delivers pinned. In the third draw the
  same kind of reading started it at OFDM rung 9: 522 bit/s.
* **The polar path** (20 m, ITU high-latitude moderate at 12 dB; best fixed rungs 15 and 9,
  300–440 bit/s). The answers went on the floor (the readings −2 … +5 dB), and priced with floor
  answers the first OFDM rung (7) delivers less than the floor's top even with nothing lost —
  the dip of §2.4. The next-rung try is the only try there is, so in one draw no try of OFDM was
  ever due (the whole message at rung 5) and in another the one try of rung 7 judged it worse and
  went back. Rungs 9 and 15 were never tried.

### 3.2 What was built

`GrantConfig.climb` (model and port, `AETHER_BENCH_LINK=climb`), on C1 at the sender, with no new
tunable:

1. **The try's rung.** Of the usable rungs above the choice whose lossless delivery, priced per
   exchange, beats the choice's estimate, the one the estimate rates best (SampleRate's pruning,
   `03-candidates.md` §2.4) — from the 2 300 Hz floor's top with floor answers that skips rung 7.
2. **When.** A transmission whose frames at the choice all decoded alone starts the try at once,
   across the floor's edge too; a try that failed since the rung last carried has doubled its
   wait, as before.
3. **The whole transmission.** A try's frames fill the transmission, so it is decided in one
   exchange.

The dip's own fix (pricing a try with the answers it would earn) is not built: with item 1 the
first OFDM rung is no longer the only candidate.

### 3.3 On the harness

D1 with D3 and the climb (`family_refusal,controller=d1,climb`), runs 16–18 of 2026-11-04,
switch and draws confirmed from each run's logs; per draw in `data/oracle_d1c.csv` (the
lopsided-acks job of run 17 lost its runner, as the same job of run 15 did, and was run again).

| | today | D1 | D1 + climb |
|---|---|---|---|
| K1(a), median regret, 21 scenarios | +21.8 % | +14.5 % | +15.5 % |
| K1(b), lowest delivery against today | — | 0.94 | **0.75** `80m-weak-wide-test-2300`, 0.80 weak both ways |
| K8 AGC: delivered, regret | 10/10, +10.4 % | 10/10, +3.6 % | 10/10, **+32.6 %** |
| K8 Trimode | 9/10, +25.4 % | 10/10, +13.5 % | 10/10, +19.3 % |

**The climb's own effect** (paired against D1, the time ratio): where it was aimed it works — the
equatorial flutter 0.83 (0.79–0.88), the polar path 0.87 (0.80–0.95), the VARA QRM 0.84
(0.74–0.98), 80 m flutter 0.88 (0.78–0.99). Elsewhere it costs: the weak 2 300 Hz Test path 1.25
(1.17–1.33, every draw slower: 175 → 123–165 bit/s), the AGC echo test 1.23 (1.09–1.41), the
Trimode test 1.11 (1.00–1.22), AWGN 1.10 in every draw (D1 2 400 bit/s at rung 16 → 2 182),
the lopsided acks 1.05 (1.00–1.14).

**Why.** Item 2 starts a try after every clean transmission whatever the estimate says, and a
try is a whole transmission. Where the choice is already the best rung — AWGN at rung 16, the weak
path's floor top, the NVIS floor at 500 Hz where OFDM cannot carry the echo — every clean
transmission buys a failed try. On the flutter and polar paths the same rule is what pays, because
there the floor's reading (a lower bound) hides that OFDM carries. One trace (the weak path, a
local run): rung 5, a try at rung 8 for two transmissions, rung 5 to the end; message 140 and
file 163 bit/s against 175.

**A failing criterion stops the step** (K1(b), K8): the refinement is the author's call
(`03-candidates.md` §4).



## 4. The last checks on D1 + D3

`.github/workflows/candidate.yml` (started by hand): every scenario at three draws with the
candidate's daemon switches (`family_refusal,controller=d1`; K3's start pair and K4), and the
real-modem link bench, both arms on its registered grid (K5: AWGN, Good, Moderate, Poor; −4 …
+20 dB; 8 000 B; three trials; both airs). K4's base is the nightly of 2026-11-03 on today's build
(the switches off; the granted vectors are the same byte for byte), the same 61 scenarios × 3
draws.

### 4.1 The first run found an overlap

Run 1 (`b696e5c`): 183 of 183 runs passed, but one radiated overlap, the first in this review:
`80m-trimode-gateway-client-vanishes-500`, base draw (seed 46), 3.2 s at 95.8–99.0 s of the air.
The trace (channel time):

* ND1J (master) keyed a floor header granting KK4ODA (slave) five frames, `rung 2 n 5`.
* KK4ODA's C1 had moved to the floor after its OFDM frames delivered nothing (D3). It answered
  at 86.2 s with an ordinary answer (`count 5`) and five tone frames at rung 1: 27.6 s on the air.
* ND1J did not decode the answer (a failed ordinary control frame at 86.6 s). It decoded the first
  tone frame (`follows 3`, "three or more"), failed the second, and keyed its next header at
  95.7 s, 9.5 s into the slave's transmission.

The cause was D1's slot sizing as built (§2's prototype). The master sized the slave's slot in the
family the slave last stated (`robust_mode(data_floor)`: OFDM rung 5, 1.05 s frames), but the
header's rung field still carried the old controller's advice (rung 2, a tone rung), and the slave
reckons its room as `n` frames of the header rung's family. The master's 5 × 1.05 s became the
slave's 5 × 5.36 s. When the master decodes the answer, its stated count places the slot's end and
nothing overlaps, which is why it took a lost answer to show. Fixed in `356bc77`, model and port:
under D1 the header states the rung the slot is sized at (without D1 that is the advice, as
before). `test_d1_sizes_the_slaves_slot_in_the_family_the_slave_states` now checks that the
slave's room fits the master's slot; it fails on the old code. The rate-review vectors' D1
sessions were regenerated, and the port's replay catches the old port code.

**For link protocol 12** (decision 9): the slave must know the family its slot was sized in, so
the header's five bits cannot all go to "frames never detected". One bit says the slot's family;
the count takes three (0–7: a transmission holds six data frames at most); one is spare.

### 4.2 The checks on the fix

Run 2 (`356bc77`), each job's switch and grid confirmed from its log.

| criterion | result | verdict |
|---|---|---|
| K3, the start pair × 3 draws, rungs apart | D1 3, 0, 1; today 3, 0, 0 | **fails in draw 0, as today does** (both builds settle at 10 and 7 there) |
| K4, every scenario × 3 draws | 183 of 183 passed (nightly 183 of 183); radiated overlaps 0 | **met** |
| K5, the real-modem bench, 56 points | completion 3 of 3 at every point, both arms; median time ≤ ×1.05 at 55 points, ×1.000 pooled | **one point over**: 500 Hz Good +4 dB, ×1.066 |
| K7 | one decision-maker, fourteen settings (decision 11) | met as decided |

**K4's readings.** Frames keyed into a deaf receiver: only `80m-vox-caller-learned-gap-500`'s VOX
caller, 6.1–6.7 s, as on the nightly (5.8–8.3 s). Air time, sum of the scenario medians, ×0.997 of
the nightly. The slowest scenario is `20m-ft8-neighbours-2300`, ×1.18, from one draw of three
(seed 56; the other two identical to the nightly): the slave's 2 000-byte reply took 82 s against
42 s. The master's second data transmission went without its header under D1, where today's rule
keeps a header on data above the advice. The master did not decode the slave's answer to it, took
the slot as silent, asked for floor answers for the next four slots, and one more slot went silent.
That is D1's "a header kept on a try" (§2.1), on one draw.

**K5's point over.** 500 Hz Good +4 dB: two of the three sessions are the same in both arms
(143.1 and 104.6/104.9 bit/s); the third went 136.2 → 127.8 bit/s, which moves the median of
three. Every other point is within ×1.05, 50 of them within ×1.01. The two arms agree this
closely because today's default already chooses its own data's rung with C1 (ADR-0094); D1
differs from it only where the advice still steered (the stuck frames' targets, the header rule,
the slot's size), and the link bench's one-way session seldom reaches those.

### 4.3 K1, K2 and K8 on the fix, and what a rerun showed

The fix changes the build the decision rested on, so D1 + D3's own choice was run again on the
oracle's ten draws (run 19, `controller_only`: a pinned rung does not consult the controller, so
each draw is judged against the pinned best of runs 8–10). Per draw in `data/oracle_d1fix.csv`.

| | today | D1 (runs 8–10) | D1 fixed (run 19) |
|---|---|---|---|
| K1(a), median regret, 21 scenarios | +21.8 % | +14.5 % | **+24.2 %** |
| the same, bootstrap 5–95 % over each scenario's draws | +14 … +24 % | +13 … +22 % | +14 … +26 % |
| K1(b), lowest median delivery against today | — | 0.94 | 0.90 `80m-flutter-500` |
| K8 AGC: delivered, median regret | 10/10, +10.4 % | 10/10, +3.6 % | 10/10, **+26.0 %** |
| K8 Trimode | 9/10, +25.4 % | 10/10, +13.5 % | 10/10, +11.8 % |

**The rerun moved K1(a) by ten points, and the fix is not why.** Scenario by scenario the two D1
runs are close (19 of 22 within five points of regret); the median over 21 moved because the
middle of the distribution has a gap — no scenario between +15 % and +24 % — and one scenario, the
AGC test, crossed it (+3.6 → +26.0 %). The AGC test's message goes one way, master to slave, and
the fix only changes the room a slave with data reckons; on the model the same one-way sessions are
identical before and after it (eight real-modem sessions, 500 Hz Moderate and Poor). On the harness
the daemons CI built before and after the fix, run here twice each on two of the AGC draws:

| draw | before the fix | after the fix |
|---|---|---|
| 3031 | 48, 48 bit/s | 34, 48 bit/s |
| 31 | 41, 41 bit/s | 41, 35 bit/s |

The same binary on the same draw gives 34 or 48. What differs is when the call goes: the
harness's steps (`at 61`, `clear a 4`, `connect`) are taken by polling the air clock every 20 ms of
wall time, and the air runs several times faster than the wall, so the call lands anywhere from
63.29 to 63.41 s of the air from one run to the next (`../link-architecture/08-steps-0-and-1.md` §1.4 found the same:
air-time stepping was not built). A session started a few blocks apart rides another stretch of the
simulated fading, which on this path decides whether the first data goes at rung 2 or rung 4.

**What that means for the review's numbers.** A single ten-draw run's K1(a) median is not a
measurement of a controller to better than about ten points; today's, D1's and the fixed D1's
bootstrap ranges overlap almost entirely. §2's "+14.5 % against +21.8 %" was within that noise, and
so is this run's +24.2 %. K8 (ii) on the AGC test moves the same way (+3.6 % and +26.0 % for builds
that behave the same on it). The paired comparison (K2) is the firmer measure, and where a step
lands a few blocks apart even its pairs become fresh draws.

**K2 for the fixed build against today**, paired by seed: faster on `80m-strong-moderate-500`, 0.89
(0.83–0.97), and on AWGN, 0.91 (0.91–0.91); "may be slower" on three, each to run again on ten fresh
draws under K2's rule: the AGC test 1.02 (0.87–1.15), 80 m flutter 1.00 (0.94–1.11), the
thunderstorm 1.02 (0.95–1.12) — the last already passed its fresh draws on §2's D1, 1.00
(0.98–1.03).

**K2's fresh draws** (oracle runs 20 and 21, draws 10–19, the fixed D1 + D3 and today's default,
controller only, each run's switch and draws confirmed from its log; per draw in
`data/oracle_k2_fresh.csv`): the AGC test 0.93 (0.85–1.03), every run delivered on both builds;
80 m flutter 1.00 (0.94–1.06). **Both pass: the fixed D1 + D3 meets K2.**

### 4.4 Reading

* **K4 is met on the fix**, and the overlap the first run found was the prototype's, not granted
  slots': a slot sized in one family and read in another. Protocol 12 keeps the slot's family in
  the header.
* **No scenario is slower** than today's controller on the paired measure (K2, with its fresh
  draws), and two are faster: the strong Moderate path 0.89 and AWGN 0.91. K5 is within ×1.05 at
  55 of 56 points and ×1.000 pooled; its one point over is one session of three.
* **K1(a) cannot separate D1 + D3 from today** on this harness: two runs of D1 gave +14.5 % and
  +24.2 % against today's +21.8 %, and the three bootstrap ranges overlap. The case for D1 + D3 is
  then decision 8's second half and decision 11: no slower anywhere, faster where it differs, and
  one decision-maker with fourteen settings where today has two controllers and thirty-one.
* **K3** fails in one draw of three, the same draw and the same rungs as today's controller: the
  start is not D1's to change.
* **The harness's step timing** makes a comparison of two builds at one seed a fresh draw wherever
  a step lands a few blocks apart (§4.3). Taking the steps on the air's clock would make every
  pair a pair; it is a harness item, not this review's.
