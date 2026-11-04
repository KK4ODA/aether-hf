# Gate 3: the candidates on the harness

*2026-11-03/04. The outcome is D1 with D3 (the author's decision 8, `03-candidates.md` §6); the climb is parked (decision 10). Remaining: K3, K4, K5, K7 on D1 + D3, then the decision record. The order is the author's (`03-candidates.md` §4, answer 4): D3 first,
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


