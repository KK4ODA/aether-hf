# ADR-0070: A tone frame whose first sync block faded stands on its other two

**Status:** accepted, 2026-10-15. Model first (`ToneStream._settle`,
`ToneDetector.two_clean_blocks` in `aether_model/phy/tone.py`), then the port (`settle`,
`two_clean_blocks`, `two_clean` in `aether-phy/src/tone.rs`). Receiver only: no wire change, no
configuration key; the vectors are unchanged (they hold the offline detector, which this does not
touch).

## 1. Context

WC4Y's probe of 2026-10-14 23:59:03Z, the first thing protocol 8 put on the air
(`docs/research/link-architecture/11-step-4-first-air.md` §6). KK4ODA-1 neither answered it nor
reported hearing it, yet its recording holds the probe clean, as strong as the call twelve
seconds later that decoded at −6.2 dB. The offline detector confirms it. Its first 0.3 s
arrived about 3.5 dB under the rest, a fade, so its sync blocks read 3, 8 and 7 hits of 8.

The streaming receiver lost it in five steps:

1. The probe's first block did not stand (3 hits, under `MIN_FIRST_HITS`), so its start was
   never announced.
2. Its own middle block was announced as a frame arriving, at a first-block statistic of 10.0.
3. When the probe's start was judged (statistic 8.12), that arrival refused it. ADR-0014's rule:
   a frame announced as arriving that starts inside a candidate, with a first block at least as
   strong as the candidate's whole, makes the candidate the reading a block-spacing early.
4. ADR-0060 excuses an arrival at a candidate's own middle or end block, but only when the
   candidate's own first block stands. This one's did not.
5. The arrival was no frame and was never confirmed. The refusal was final.

On synthetic tone-24 frames at −8 to −12 dB with the first block faded 12–30 dB, the stream lost
the frame in all 26 cases where the first block fell to 0–5 hits. The offline detector confirmed
and decoded all 27. Calls and probes come out of silence, and a burst's first frame out of a
turnaround: these are the frames most exposed.

## 2. Decision

A candidate refused **only** by arrivals at its own middle or end block (to a symbol), whose
first block does not stand, is taken when its two best sync blocks are each whole to
`CLEAN_BLOCK_HITS` (7 of 8) with no contradiction: ADR-0049's condition (`two_clean_blocks`). It
must pass `confirmed` as before. An arrival anywhere else inside it still refuses it, as does
anything that refused it before.

Why ADR-0014's reading a block-spacing early cannot pass: its first block lies in the silence or
noise before the burst, its middle block on the real frame's first, and its end block over the
frame's data. The three distances between a kind's blocks all differ (`block_offsets`: 0, 57, 126
symbols for a data kind, 0, 33, 72 for the control kind), so no shift of a frame lines up two of
its blocks with the frame's. That reading has one clean block. A ghost of another air is
contradicted in every block (ADR-0015). Two clean blocks are a frame's alone (ADR-0049).

## 3. Measured

* **The probe's case and ADR-0060's.** `test_a_frame_whose_first_block_faded_is_not_refused_by_its_own_middle_block`
  (strict xfail until now) passes in both suites. ADR-0060's
  `test_a_burst_whose_first_frame_began_under_the_mute_is_not_lost_with_it` had recorded the
  first frame of a burst begun under the receiver's mute as lost. That frame's first block is
  silent and its other two are clean: it is now found and decodes, in both suites, and the tests
  say so.
* **The same audio through both rules.** A bench streamed each case through the old rule and
  the new one (`ToneStream` with the old settle step beside the new), every tone kind of both
  airs through both airs' detectors, so a kind the receiving air does not look for is a chance
  for a false detection:
  * **Frames after noise, after exact silence (the receiver's own transmission, where ADR-0014's
    early reading lives), and begun half a second under the mute.** AWGN, Good, Moderate and
    Poor, 2 frames a point. 1 622 frames a rule: −8, 0, 10, 20 and 30 dB on the 2 300 Hz air,
    −8, 0 and 20 dB on the 500 Hz air.

    | receiving air | case | frames of its kinds | found, old / new | false, old / new |
    |---|---|---|---|---|
    | 2 300 Hz | after noise | 280 | 279 / 280 | 0 / 0 |
    | 2 300 Hz | after silence | 280 | 280 / 280 | 0 / 0 |
    | 2 300 Hz | under the mute | 280 | 0 / 270 | 0 / 0 |
    | 500 Hz | after noise | 120 | 119 / 119 | 0 / 0 |
    | 500 Hz | after silence | 120 | 118 / 120 | 0 / 0 |
    | 500 Hz | under the mute | 120 | 0 / 114 | 0 / 0 |

    The 422 frames of kinds the receiving air does not look for gave no detection under either
    rule. On the 2 300 Hz air those were 134 of the 240 planned: the run met its time limit
    while sending the 500 Hz air's middle kinds, after every kind the air looks for was done.
  * **First block faded** 6, 12, 20 and 30 dB, at −8 and 0 dB on AWGN and Moderate, every kind
    the air looks for, 4 frames a point: 768 frames. The offline detector found 766. The stream
    found 365 under the old rule and 751 under the new, and 190 against 392 decoded through it.
    At a 30 dB fade, 1 of 192 against 187. No false detection under either rule.
  * **Noise alone**, ten minutes on each air, plain and broken by exact silence 4 s in every 12:
    no frame under either rule.
* **The harness.** Ten scenarios × 3 seeds on the build with this and ADR-0071 (the quick set,
  `80m-weak-500`, `80m-floor-top-start-500`, `80m-local-rfi-ladder-500`, `80m-qsb-dropout-500`,
  `80m-vox-caller-declared-500`, `80m-poor-500-pactor-signalink`, `20m-polar-path-2300`): 30 of 30
  pass, no RF overlap, nothing slower than the baseline (`scenario_trend.py`).

## 4. Consequences

A frame that begins while the receiver is still muted after its own transmission (`deaf_for`) is
no longer lost for want of its first block. A faded start is no longer lost either. The detector
offline, the announcement of arrivals and every other rule of the stream are unchanged.
