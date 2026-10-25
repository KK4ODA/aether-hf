# The hidden-frame bench: results

*2026-10-25. The plan, its criteria and its two amendments are `00-plan.md`; every run is
`tools/bench_hidden.py` on the model's streaming receiver, fed 20 ms blocks as the daemon feeds
it, at the tree of `f9ac90d` (the bench's switches; the receiver as built is unchanged). The runs'
CSVs are not in the tree; the commands are below each table, and the seeds are the tool's
defaults.*

## 1. In short

* **Two faults, not one.** The stream drops a candidate that starts inside a frame it already
  knows. That lets a weak false detection hide a real frame after it (the case the bench was
  for: five on the air now). It also hides the next frame of a burst whenever the last one's
  timing locked a few samples late, because the span runs to the very end of that frame and a
  burst's frames go back to back (§5). The second is the larger: on ITU Poor the receiver as
  built misses about one decodable frame in six or seven, at either bandwidth.
* **R3h** — R1 (a known frame acquired under 1.3 that has not decoded does not hide a candidate
  at 1.3 or more) with a half-symbol guard at the span's end — passes K1, K3, K4 and K5:
  every hidden frame on the air recovered and nothing lost; on 7 467 frames of traffic 133 more
  decoded (+2.6 % at 2300 Hz, +3.7 % at 500 Hz; +16 % and +19 % on Poor), none lost, no false
  frame added; noise unchanged; decode attempts +2.7 % and +1.0 %. It misses K2 as registered
  (91.0 % and 94.1 % of M2's frames), by design: in 70 of 74 trials where the cut preamble read
  1.3 or more, R1 trusts it (§3).
* **R2** — the stronger candidate wins, as `decode_buffer` does — passes K1 and K2 (99.8 % at
  both airs) and fails on traffic: three losses (it has no end guard) and 41 false frames added,
  10.6 per 1 000 at 500 Hz — a weak real frame's own data symbols, taken whenever they read
  stronger than its preamble. It gains about half what R3h does.
* **What R3h cannot recover** is a real frame behind a false candidate that reads 1.3 or more.
  Noise does not make one: in half an hour of noise per air the stream's candidates read at most
  1.07, and the false candidates that hid frames on the air read 1.04–1.11. A candidate at 1.3 is
  a signal — another station's frame, or a frame's own copy (the late copies at 1.34–1.38 are the
  span's end, which the guard handles).
* **Report 15's frames "never detected"** were partly this: six of the 26 that KK4ODA-1 never
  detected in WC4Y's 2300 Hz Test of 10-23 are found under R3h (and fail to decode — they were
  weak). For the rate controller's review, a share of "never detected" is the receiver's span
  rule, not the path (§2.2).
* **The guard alone (R0eh) is not enough**: it fails K1 on the air (§2).

Nothing on the air changes until the author decides. The decision needed is §8.

## 2. The field (M1)

### 2.1 Frames decoded

Every recording in hand, both sides of the five Tests of report 17, both sides of ND1J's two of
10-23 (report 16) and KK4ODA-1's side of WC4Y's 2300 Hz Test of 10-23 (report 15):

| recording | air | as built | R1 | R3 | R3h | R0eh | R2 |
|---|---|---|---|---|---|---|---|
| ND1J, Test of 10-23 16:04Z | 500 | 104 | 104 | 104 | 104 | 104 | 104 |
| KK4ODA-1, the same | 500 | 83 | | | 84 | | 84 |
| ND1J, Test of 10-23 16:07Z | 500 | 162 | 163 | 163 | 163 | 162 | 163 |
| KK4ODA-1, the same | 500 | 135 | | | 135 | | 135 |
| KK4ODA-1, WC4Y's Test of 10-23 00:52Z | 2300 | 41 | | | 41 | | 41 |
| KQ4SAX, Test of 10-23 21:08Z | 2300 | 16 | 16 | 16 | 16 | 16 | 16 |
| KK4ODA-1, the same | 2300 | 68 | 68 | 68 | 68 | 68 | 68 |
| KQ4SAX, Test of 10-23 21:18Z | 2300 | 13 | 13 | 13 | 13 | 13 | 13 |
| KK4ODA-1, the same | 2300 | 58 | 58 | 59 | 59 | 59 | 58 |
| WC4Y, Test of 10-24 00:20Z | 2300 | 13 | 13 | 13 | 13 | 13 | 13 |
| KK4ODA-1, the same | 2300 | 30 | 30 | 30 | 30 | 30 | 30 |
| ND1J, Test of 10-24 15:55Z | 500 | 191 | 192 | 192 | 192 | 191 | 192 |
| KK4ODA-1, the same | 500 | 148 | 150 | 152 | 152 | **149** | 152 |
| ND1J, Test of 10-24 16:00Z | 500 | 172 | 172 | 172 | 172 | 172 | 172 |
| KK4ODA-1, the same | 500 | 129 | 130 | 130 | 130 | 129 | 130 |

Blank: not replayed under that rule (the three recordings fetched for this report were replayed
under R3h and R2 only). No rule but R0eh lost a frame the rule as built decoded. R0eh lost one:
in KK4ODA-1's side of ND1J's Test of 10-24 15:55Z, a phantom (206.81 s, 1.03) hides ND1J's data
frame at 206.88 s as built; with the guard alone a second weak candidate (207.855 s, 1.01) gets
in through the phantom's last half symbol, and its span hides the next real frame (207.93 s,
decoded as built). Under R3h the chain cannot start: R1 recovers the strong frames at 206.44 and
206.88 s, so no weak phantom stands in front of them. The guard needs R1.

### 2.2 What was hidden

Hidden by a weak false candidate (the case of §1 of the plan), with the model's readings:

| recording | the false candidate | the frame it hid |
|---|---|---|
| ND1J, 10-23 16:07Z | 212.375 s, data r6, 1.04 | 212.846 s, KK4ODA-1's answer, 1.75, +11.4 dB |
| KK4ODA-1, 10-23 16:04Z (new) | not traced | 137.329 s, ND1J's control frame, 1.72, +9.8 dB |
| KK4ODA-1, 10-24 15:55Z | 206.184 s, control, 1.04 | 206.441 s, ND1J's header, 1.75, +8.4 dB, and the data frame after it |
| ND1J, 10-24 15:55Z | 231.276 s, data r12, 1.11 | 231.608 s, KK4ODA-1's answer, 1.74, +11.2 dB |
| KK4ODA-1, 10-24 16:00Z (new) | 106.818 s, control, 1.10 | 106.855 s, ND1J's control frame, 1.54, −5.0 dB: its own early copy, 37 ms ahead |

Hidden by the span's end (§5), on the air: at KK4ODA-1 in ND1J's Test of 10-24 15:55Z, two
rung-9 frames back to back at 88.60 and 89.65 s, lost as built to a copy of the first 31 ms late
(88.627 s, read as r7 at 1.38, failed) whose span hid the second; at KK4ODA-1 in KQ4SAX's Test of
10-23 21:18Z, a rung-6 frame at 117.32 s lost the same way (its late copy at 117.350 s, 1.34).

**Report 15's "never detected".** In KK4ODA-1's side of WC4Y's Test of 10-23 00:52Z, R3h hands
out six frames more than the rule as built, and all six fail. On WC4Y's clock (KK4ODA-1's
recording starts 19.7 s later) each falls on a data frame WC4Y sent and KK4ODA-1 never detected:

| R3h, KK4ODA-1's recording | WC4Y's clock | WC4Y's transmission, frame |
|---|---|---|
| 53.77 s, r7, 1.23, −3.6 dB | 73.5 s | 1, the sixth (#5, first copy) |
| 78.32 s, r7, 1.31, −0.8 dB | 98.1 s | 3, the fifth (#9, second copy) |
| 174.00 s, r7, 1.16, −2.3 dB | 193.7 s | 11, the third (#14, third copy) |
| 259.11 s, read as control, 1.04 | 278.8 s | 16, the ladder's rung 8, the third |
| 299.25 s, r5, 1.02 | 319.0 s | 21, the ladder's rung 11, the first |
| 301.36 s, r5, 1.11 | 321.1 s | 21, the third |

`rung_reading.py --timeline` counts 26 frames never detected in that session (20 of them rung 7,
six on the ladder); six were hidden by the span rule. They were weak and would not have decoded,
but the rate controller's review counts a frame never detected differently from one detected and
failed (`docs/research/rate-control/00-brief.md`).

## 3. Recovery (M2)

A cut preamble (acquisition 1.0–1.25 at the calibrated SNR) and a real frame 0.1 s to one span
later, 1 200 trials an air pair, AWGN. *Counted*: the reference decodes the real frame alone and
the rule as built handed out the false candidate (974 trials). *Aimed*: of those, the false
candidate read under 1.3 and the real frame at or over it (898). As built recovers none (0 of
974): the false candidate's span always hides the real frame.

| rule | 2300 Hz counted | 500 Hz counted | aimed, both | false read ≥ 1.3 (74 trials) | decode attempts |
|---|---|---|---|---|---|
| R1 | 451/498 (90.6 %) | 448/476 (94.1 %) | 897/898 | 2 | 2 130 (as built 1 563) |
| R3 | 455/498 (91.4 %) | 450/476 (94.5 %) | 897/898 | | |
| R3h | 453/498 (91.0 %) | 448/476 (94.1 %) | 897/898 | 4 | 2 132 |
| R2 | 497/498 (99.8 %) | 475/476 (99.8 %) | 898/898 | 73 | 2 180 |

Commands: `bench_hidden.py synth --rule r1|r3|r3h|r2 --out m2_<rule>.csv --trials 20`
(seeds from 1001).

The calibration's medians put the cut preamble at 1.0–1.25 offline; the stream read 74 of them at
1.30–1.49. R1 trusts those by its definition, so it leaves them hiding. Three more of R1's misses
are real control frames at 500 Hz, 1 dB over threshold, read just under 1.3. The aimed miss
is one such frame, read 1.300.

## 4. Traffic (M3) and cost (M5)

The link's own transmissions (a header and one to six data frames back to back, answers, tone
frames) through AWGN and ITU-R F.1487 Good, Moderate, Poor and flutter, −8 to +20 dB, 120 s of
air a condition, 80 conditions. Frames decoded, against the rule as built; `*` where the rule
lost a frame the rule as built decoded:

| air | channel | sent | as built | R1 | R3 | R3h | R0eh | R2 |
|---|---|---|---|---|---|---|---|---|
| 2300 | AWGN | 745 | 607 | +0 | +0 | +0 | +0 | +0 |
| 2300 | Good | 750 | 481 | +0 | +0 | +0 | +0 | +0 |
| 2300 | Moderate | 696 | 455 | +0 | +0 | +0 | +0 | +0 |
| 2300 | Poor | 746 | 327 | +5 * | +53 | +53 | +53 | +15 * |
| 2300 | flutter | 738 | 291 | +4 | +4 | +4 | +0 | +7 |
| 500 | AWGN | 782 | 641 | +0 | +0 | +0 | +0 | +0 |
| 500 | Good | 731 | 468 | +0 | +0 | +0 | +0 | +0 |
| 500 | Moderate | 754 | 451 | +9 | +28 | +28 | +26 | +19 |
| 500 | Poor | 768 | 242 | +19 * | +45 | +45 | +42 | +23 * |
| 500 | flutter | 757 | 224 | +3 | +2 * | +3 | +0 | +4 |

| rule | losses | false frames added (per 1 000) | unmatched candidates | decode attempts |
|---|---|---|---|---|
| R1 | 4 | 11 (2300 Hz 0.27, 500 Hz 2.64), none decoded | 243 → 189 | +0.6 %, +0.8 % |
| R3 | 1 | 1 (500 Hz 0.26), none decoded | 243 → 70 | +2.7 %, +1.0 % |
| R3h | 0 | 0 | 243 → 71 | +2.7 %, +1.0 % |
| R0eh | 0 | 0 | 243 → 81 | +2.5 %, +0.8 % |
| R2 | 3 | 41 (2300 Hz 0.27, 500 Hz 10.55), none decoded | 243 → 171 | +1.2 %, +1.9 % |

Commands: `bench_hidden.py traffic --rule <rule> --out m3_<rule>.csv` (120 s a condition, seeds
from 5001); the losses and additions are listed in each CSV's `.notes.txt`.

The extra decode attempts are real frames now decoded and late copies now handed out.

## 5. The span's end

A burst's frames go back to back, so the next frame starts exactly where the last one ends. The
stream counts a candidate as inside a known frame from a symbol before its start to the end of
its span, with no tolerance at the end. When a frame's timing locked on the later ray — 2 ms,
16 samples, on ITU Poor — its span ends 16 samples past its true end, and the next frame's true
start lies inside it: hidden. The next frame's copy a symbol late (its preamble's second symbol
correlates too) lies outside, is taken, is a symbol off, and fails; and its span, a whole data
frame long, hides the frame after that. As built, on M3's first trace: frame *k* hidden this way,
its late copy taken, frame *k*+1 hidden by the copy. R1 recovers *k*+1 (it reads 2.45), whose own
late end then hides *k*+2 — one gain and one loss, the four losses of §4.

The guard's length is bounded on both sides. It must be longer than the timing error that puts
the next start inside (the delay spread: 16 samples on Poor, 56 on ITU NVIS), and shorter than a
symbol: the detector's early sidelobe of a frame lies a symbol before its start, so a one-symbol
guard admits it whenever a header ends where its first data frame begins — R3's one loss, a
sidelobe at 1.08 whose span hid a data frame read 1.14. Half a symbol (124 samples, 15.5 ms)
does both, and R3h lost nothing.

The port has the same test (`known()` in `core/aether-phy/src/stream.rs`: `start < b`).

## 6. Noise (M4)

An hour of noise an air: candidates handed out, all failed.

| rule | 2300 Hz | 500 Hz |
|---|---|---|
| as built | 1 | 16 |
| R1 | 1 | 16 |
| R3 | 1 | 16 |
| R3h | 1 | 16 |
| R2 | 1 | 16 |

The candidates the rule as built hands out on noise read 1.01–1.07 at 500 Hz (nine in 30 minutes;
none at 2300 Hz), all under the 1.3 at which R1 trusts a frame. Commands: `bench_hidden.py noise
--rule <rule>` (six ten-minute chunks an air, seeds from 9001).

## 7. Against the criteria

| | criterion | R1 | R3h | R2 |
|---|---|---|---|---|
| K1 | the field | pass | pass | pass |
| K2 | recovery ≥ 95 % | miss (90.6 / 94.1 %) | miss (91.0 / 94.1 %) | pass (99.8 %) |
| K3 | no loss on traffic | fail (4) | pass | fail (3) |
| K4 | ≤ 1 false frame per 1 000, none decoded; noise ≤ R0 + 1 | fail at 500 Hz (2.64) | pass | fail at 500 Hz (10.55) |
| K5 | ≤ 5 % more decode attempts | pass | pass | pass |

R3 (the one-symbol guard) passes K1, K4 and K5 and misses K3 by one loss (§5); R0eh (the guard
alone) passes K3–K5 on traffic and fails K1 on the air (§2.1).

## 8. The decision

The plan sends a narrow miss to the author with its trace. R3h misses only K2, and the trace
is §3: every trial it cannot recover has a false candidate read at 1.30–1.49, which R1 trusts by
definition. Two ways on:

1. **Take R3h, with K2's miss accepted** (recommended). The case it leaves is a frame behind a
   *signal* at 1.3 or more; noise gives none (§6), and none of the five on the air was one. It
   passes everything else, and on Poor paths it decodes a sixth more frames at either bandwidth.
2. **Hold out for K2.** The only rule measured that passes it is R2, which fails K3 and K4 by
   wider margins; a rule between them (R1's trust threshold raised, or R2 with the guard) would
   be a new registration and another day of runs, for a case the evidence has not shown.

R3h changes no wire format and no timing: it is the receiver's rule for which candidates it
looks at. In a session under granted slots a frame that is now decoded is a frame the engine
would have answered for anyway, and one now detected and failed counts on its transmission's
stated grid as before.

## 9. What a change would take

As `00-plan.md` §5: an ADR; the model's rule; the port's `known()` the same, with streaming tests
in both suites (a weak candidate before a real frame; a burst whose first frame's timing is late
by the delay spread); the field recordings through the port before and after; the scenario
harness; then a beta. No wire change.
