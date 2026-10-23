# ADR-0093: The first OFDM rung is held to the learned margin on both airs

**Status:** accepted, 2026-10-23, the author's go on report 15's change 1
(`docs/research/link-architecture/15-wc4y-2300-beta120.md` §5, §11). Reverses the 2 300 Hz
air's floor-boundary cap and its "once" rule (ADR-0013 §4 item 5, measured again in ADR-0014
§4). Model first, then the port; **no wire change, no configuration key, no schema bump**. A
deletion: it adds nothing.

## 1. Context

The rate controller holds each rung to its AWGN threshold plus a learned margin, which failures
widen and clean bursts narrow. ADR-0013 gave the 2 300 Hz air one exception at the floor's top:
the first OFDM rung (now rung 7, BPSK ⅓) was held to its threshold plus at most 1 dB
(`floor_margin_db`), however wide the learned margin, and one failed burst there did not drop
the link to the floor; two in a row did. The reason was the link bench's fading pipe: the floor
was then a quarter of the rung's rate, and BPSK ⅓ with HARQ carried more than the floor a decibel
above its 10 % point. ADR-0014 measured it again once the fast tones made the floor's top
two-thirds of the rung's rate, and kept it (off, AWGN 0 dB fell from 263 to 142 bit/s on the pipe).
The 500 Hz air never had it.

On WC4Y's 2 300 Hz Test of 2026-10-23 (report 15 §5) KK4ODA-1 read WC4Y's data at about −2 dB.
Rung 7's AWGN threshold is −3.2 dB, so the cap kept it on offer for 135 s: 58 frames sent at
rung 7, 20 never detected, 7 decoded on their own, 9 more by combining two to four copies. Every
tone frame on the path decoded on its own (rungs 2, 4 and 5: 11 of 11). The message ran at
55 bit/s and the file at 61, where rung 5 (tone100-153) would have carried both about 2.5 times
as fast. Rung 7's 10 % point is +4.2 dB on ITU Moderate and +1.8 on Poor.

Three things together held it there:

1. The cap kept rung 7 "fitting" whatever the learned margin said.
2. "Once": leaving needs two failed bursts in a row, and a burst that combining rescued counts as
   clean, so the failures never came two in a row.
3. A frame the receiver never detected counts for nothing.

The link bench could not show this. The pipe delivers every frame to the receiver, detected, and
adds 3 dB of combining energy per copy (`sim._HARQ_GAIN_DB`); on the air a third of rung 7's frames
gave the receiver nothing to combine or count.

## 2. Decision

1. **No cap.** The first OFDM rung is held to the learned margin like any other, on both airs
   (`RateController.floor_margin_db`, `PhyTiming.floor_margin_db`, `WIDE_FLOOR_MARGIN_DB` and
   the daemon's setting are gone).
2. **No "once".** A failed burst there steps down like a failed burst anywhere
   (`_boundary_failures` is gone).
3. **The edge rule is both airs'.** A burst that fails in part on the first OFDM rung counts as
   failed (ADR-0084's exception, `_at_edge`), as it already did at 500 Hz: the floor below it
   carries an exchange better than the rung once the rung loses frames.
4. `first_mode` is unchanged. A session the SNR puts on the OFDM rungs still does not start on
   the floor; but rung 7 now needs its threshold, the margin and the hysteresis, −3.2 + 3 + 1.5
   = +1.3 dB, where the cap had asked for −0.7. WC4Y's call, read at −0.3 dB, starts at rung 5.

Points 2 and 3 do not cover the third cause, frames never detected; that belongs to the rate
controller's review (report 15 §11, change 2), which this ADR does not pre-empt.

## 3. Measured

**This path on the harness** (`bench/scenarios/80m-weak-wide-test-2300.toml`, WC4Y heard at
−2 dB on ITU Moderate, KK4ODA at −6, a Test), four draws, beta.120 and the cap off (a scratch
build):

| draw | beta.120: message, file bit/s | cap off |
|---|---|---|
| s1023 | 104, 167 | 104, 167 |
| s2023 | 104, 168 | 104, 168 |
| s3023 | 140, 175 | 140, 175 |
| s4023 | 80, 95 (started at rung 7: 33 of 61 detected frames decoded) | 173, 171 |

Three draws read the call below −0.7 dB and started at rung 5 on both builds; the fourth read it
higher and repeated the air's pattern on beta.120.

**Every other 2 300 Hz scenario**, one draw each: §5.5 of report 15 has the table. All 25 pass on
both builds, no radiated overlap; 16 run the same to the tenth of a second, 6 are faster without
the cap (`20m-polar-path` 192 → 155 s, `80m-thunderstorm-agc` file 441 → 584 bit/s,
`40m-moderate-2300-vara-qrm` file 622 → 732, `20m-dx-moderate` file 674 → 780,
`15m-equatorial-flutter`, `40m-gateway-busy-when-called`), and three are slower:


| three draws | beta.120 | cap off |
|---|---|---|
| `40m-closing-identifiers-2300` | 440, 453, 456 s | 538, 453, 554 s |
| `40m-rtty-contest-2300` | 438, 350, 384 s; file 120, 276, 127 bit/s | 423, 294, 384 s; file 126, 384, 127 bit/s |
| `40m-gateway-three-clients-2300` (one draw) | 231 s | 234 s |

* **The RTTY contest is not slower.** Draw for draw it is as fast or faster; its one-draw gap was
  the draw. The three clients' 3 s is within a build's own spread.
* **`40m-closing-identifiers-2300` runs about 100 s longer in two draws of three**, all of it in
  the last of its five endings, where the path dies for 240 s until both stations time out. The
  master ends when it did ("no response"). The slave, its frames failing as the path went, steps
  down to the floor, which the cap had kept it from, and its link timeout counts four whole
  exchanges at the family in use: it waits about twice as long, silent, before it calls the
  session over. Its identifier still goes last and nothing overlaps. How long a slave on the floor
  waits on a dead path is the link timeout's, not this change's; the rate controller's review can
  look at it.

**The model's tests.** `test_the_floor_boundary_is_crossed_by_what_the_rungs_are_worth` (both
suites) keeps the family rule of `first_mode` and now holds the uncapped rule: one lost burst on
the first OFDM rung drops to the floor, a call at −0.3 dB starts on the floor.
`test_frames_whose_answer_went_unheard_are_not_resent_blind` runs eight seeds where it ran four:
at 2 300 Hz seeds 0–3 tied to the second with the cap gone (`hold_unknown` slower in two of them),
while twelve seeds give it 20 % less time and a quarter fewer frames. Its assertions are as
strict as before.

## 4. Not done

* **The link bench is not re-run.** On the pipe the cap paid (ADR-0014 §4); the pipe cannot see
  why it costs on the air.
* **The nightly baseline is not refreshed.** `40m-closing-identifiers-2300`'s median of the new
  draws (538 s) is inside the trend check's step (15 % and 30 s past the baseline's longest,
  482 s).
* **Frames never detected still count for nothing**, and a burst rescued by combining still
  counts as clean. Those belong to the rate controller's review, not to another patch.
