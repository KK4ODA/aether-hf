# ADR-0084: A burst that failed in part moves the margin by its share

**Status:** accepted, 2026-10-20. Model first (`RateController.partial_share`, `_at_edge` and
`_widen` in `model/aether_model/link/rate.py`), then the port (`RateConfig::partial_share`,
`at_edge`, `widen` in `aether-link/src/rate.rs`), held together by the regenerated
`granted_vectors.json`. **No wire change, no configuration key.** The rate a station recommends
moves differently; the frames are the same.

## 1. Context

ADR-0081 §3 left a cost: on the link bench's fading pipe, ITU Poor at −6 dB on the 500 Hz air
ran 2 % slower with the new rung 4 than with the receiver change alone. The controller climbs
by AWGN thresholds plus a learned margin, and rung 4's fading penalty (7.3 dB on Poor) is larger
than rung 3's (5.8). The question was whether thresholds per channel class would pay.

**Thresholds per class, measured first and rejected.** Told the class it was on, the
controller used that class's own 10 % FER points for every rung (the fading pipe's calibration
targets, `bench/baselines/fading_pipe.csv`) in place of AWGN's. 500 Hz, 1 kB sessions, 20 a
point, mean bit/s:

| | as it was | class thresholds | the same, 0.5 dB margin |
|---|---|---|---|
| Good −6 dB | 48.5 | 25.6 | 28.4 |
| Good +2 dB | 83.8 | 55.0 | 65.0 |
| Moderate 0 dB | 72.3 | 62.7 | 68.6 |
| Poor −6 dB | 57.4 | 41.8 | 47.2 |
| Poor +4 dB | 107.8 | 86.7 | 96.8 |

Slower at every point of the grid (Good, Moderate and Poor, −10 to +6 dB): by 12–47 % with the
controller's own margin on top, and still by 5–41 % with a 0.5 dB one. A 10 % FER point is the wrong operating point for a link with
HARQ: the learned margin settles where frames fail more often but combine, and the class tables
hold every rung to the point where they seldom fail.

**Where the cost actually is.** Capping the ladder showed what the free controller gives up:
at Poor −6 dB a ladder topped at rung 3 ran 7 % faster (62.4 against 58.3 bit/s), and at 0 dB on
every fading class a ladder topped at rung 4 ran 7–15 % faster. A trace of the Poor −6 dB
sessions showed why: a 1 kB session holds four to eight bursts, and its first one at rung 3 lost
one frame of six. That single frame took the margin from 3 to 6 dB, the most `max_jump_db`
allows, and the next burst went at rung 1. On AWGN a failed frame means the rung is past its
cliff. On a fading path one lost frame in six is the operating point HARQ is there for.

## 2. Decision

A burst that decoded some frames and lost fewer than half (`partial_share` = 0.5) **failed in
part**:

* the margin widens by the share of the burst that failed, and so does the cap on the jump to
  the margin the failure implies: one frame of six moves it a sixth as far as the whole burst
  lost would;
* the rung stays unless the SNR no longer carries it at the new margin;
* the clean run starts again, so the climb waits.

A burst lost in the main is a failure as before. So is any failure on **the first OFDM rung of
an air that does not cap its margin there** (the 500 Hz air: `floor_margin_db` is `None`). Below
that rung is the tone floor, which carries an exchange better than the rung does once the rung
loses frames: six 1.05 s OFDM frames of 22 data bytes and a 3.2 s floor answer carry about
104 bit/s against tone4x100-93's 118–120 in whole bursts. Holding the rung there on a partial
failure cost 3 % at 0 dB on Moderate and Poor. On the 2 300 Hz air, whose first OFDM rung is
held to a 1 dB margin and kept for one failed burst already (ADR-0013), exempting the edge as
well gave back gains: Poor 0 dB +7.5 → 0 %, Good +4 dB +15 → +10 % on 8 kB sessions.

This is not ADR-0007's rejected "a lone failure is an accident": the margin still moves, by the
share.

## 3. Measured

The link bench's fading pipe (`bench/baselines/link_partial_failures.csv`), beta.117's controller
(`partial_share = 0`) against this one, the same seeds; mean bit/s and the paired z. AWGN is
identical at every point on both airs: its failures are whole bursts at a cliff.

| 500 Hz | 1 kB, 40 sessions | 8 kB, 24 sessions |
|---|---|---|
| Good −8 dB | 38.0 → 40.9 (+8 %, z 5.0) | 32.0 → 41.6 (+30 %, z 12.1) |
| Good −2 dB | 65.1 → 65.8 (+1 %, z 2.0) | 69.2 → 85.2 (+23 %, z 12.0) |
| Good +4 dB | 99.9 → 103.9 (+4 %, z 3.4) | 108.5 → 112.7 (+4 %, z 3.5) |
| Moderate −6 dB | 55.9 → 57.6 (+3 %, z 2.5) | 61.9 → 70.3 (+14 %, z 7.7) |
| Moderate 0 dB | 71.8 → 72.5 (+1 %, z 2.2) | 97.4 → 100.1 (+3 %, z 3.3) |
| Moderate +4 dB | 94.1 → 102.4 (+9 %, z 5.8) | 114.1 → 116.6 (+2 %, z 1.6) |
| Poor −6 dB | 58.0 → 58.3 (+1 %, z 1.0) | 70.8 → 77.2 (+9 %, z 4.9) |
| Poor 0 dB | 76.1 → 76.2 (+0 %, z 0.5) | 101.6 → 101.1 (−0.5 %, z −0.9) |
| Poor +4 dB | 108.4 → 117.9 (+9 %, z 6.1) | 122.7 → 133.5 (+9 %, z 7.2) |
| Poor +8 dB | 171.8 → 179.8 (+5 %, z 3.2) | 197.2 → 237.9 (+21 %, z 10.1) |

| 2 300 Hz | 1 kB, 30 sessions | 8 kB, 16 sessions |
|---|---|---|
| Good −8 dB | 49.6 → 54.9 (+11 %, z 5.4) | 35.5 → 56.3 (+59 %, z 9.0) |
| Good +8 dB | 253.2 → 254.4 (+0 %, z 0.4) | 462.5 → 536.4 (+16 %, z 3.8) |
| Moderate −8 dB | 65.0 → 66.8 (+3 %, z 3.2) | 68.5 → 89.0 (+30 %, z 6.1) |
| Moderate 0 dB | 106.5 → 110.9 (+4 %, z 2.4) | 138.8 → 131.3 (−5 %, z −1.7) |
| Moderate +8 dB | 275.8 → 280.2 (+2 %, z 2.1) | 464.5 → 602.0 (+30 %, z 9.0) |
| Poor 0 dB | 115.4 → 120.8 (+5 %, z 2.1) | 136.0 → 145.7 (+7 %, z 3.9) |
| Poor +4 dB | 206.3 → 208.8 (+1 %, z 0.9) | 282.8 → 341.1 (+21 %, z 9.0) |

Over the whole grid (10 SNRs at 500 Hz, 7 at 2 300 Hz, three fading classes each), the largest
loss was the 2 300 Hz Moderate 0 dB point on 8 kB sessions; a second run of 24 sessions there read
−2.4 % (z −0.8). Every other point is a gain or within a percent. The longer the session, the
larger the gain: in a long session on a fading path the controller as it was ratcheted its margin
up on every partial failure and gave it back slowly, by design (ADR-0007), so it drifted ever more
cautious.

**On the harness** (added 2026-10-21): ADR-0082…0085 together against beta.117, the 56
scenarios × 2 draws each, the real daemons on the channel server. 112 of 112 runs pass on both
builds, no radiated overlap, pooled air time 16 814 → 15 716 s (−6.5 %); the Tests' file rate
×1.14 (median over 15 scenarios), message rate ×1.00. Fastest: `80m-qsb-dropout-500` −36 %,
`80m-flutter-500` −23 % (message 60 → 106, file 74 → 127 bit/s), `40m-long-attachment-cwid-2300`
−21 %, `80m-winlink-exchange-500` −20 %, `40m-hidden-vara-2300` −20 %. Slowest:
`40m-winlink-exchange-2300` +14 % and `20m-dx-500` +11 %, each one draw of two: ADR-0082 §3 has
why. A third draw of every scenario: 56 of 56. `bench/baselines/scenarios.csv` is these three
draws.

This rule alone, before ADR-0085, lost `80m-agc-carrier-outside-500`'s message on its own draw
in 2 of 8 runs (beta.117: 1 in 10): it holds the recommendation higher after a partial failure,
and on that NVIS path the frames stranded at the first OFDM rung were re-encoded at rung 4,
which the path does not carry. ADR-0085 re-encodes them at the most robust rung of their family
instead: 11 of 12.

## 4. Alternatives

* **Thresholds per class**: §1. Even told the class, they lose 5–47 %.
* **A per-rung offset**: a failure's lesson put on the rung that failed and those above it,
  decaying by `down_step_db` a clean burst. No measurable effect: a session holds too few bursts
  at any one rung to learn from.
* **A ladder that prices the exchange**: a rung worth its burst's data over the burst, the two
  control frames in the families in use and the handovers, the usable rungs recomputed as the
  families change. +2 to +5 % on fading paths from 0 to +4 dB, but −12 % on AWGN at 0 dB and −6 %
  on Poor at +4 dB, where a session's first exchanges go on the floor and hid rung 5 just where
  it pays. Not built. The exchange's price is real (§2's edge rule is the part of it that measured
  clean), and a version that weighs it against what the session has measured is the open item.
* **A cap on the 500 Hz ladder** (rung 3 at Poor −6 dB, rung 4 at 0 dB): the +7–15 % of §1 is
  an upper bound for choosing the right rung by class. It needs to know the class, and the
  class thresholds of §1 are not the way to know it.

## 5. Consequences

* Sessions on fading paths run faster on both airs, most of all long ones; AWGN does not move.
* The Poor −6 dB cost of ADR-0081 is gone on long sessions (+9 %) and is within noise on 1 kB
  ones (+0.6 %). The cap's 7 % at 1 kB is left: rung 4 loses 42 % of its frames there, most of
  its failed bursts are failures in the main, and the rule leaves those as they were.
* Nothing an operator sees changes but the rates.
