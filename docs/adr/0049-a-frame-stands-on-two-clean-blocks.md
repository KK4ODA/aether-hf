# ADR-0049: A tone frame stands on two clean sync blocks

**Status:** accepted, 2026-10-07. Model first (`ToneDetector.confirmed`,
`_block_contradictions`, `CLEAN_BLOCK_HITS` = 7), then the port (`confirmed`,
`block_contradictions_in`, `TONE.clean_block_hits` through `preamble_tables.json`). The
transmitter is unchanged, and so is the wire: this concerns only how frames are recognised.

## 1. Context

WC4Y's Test of 2026-10-05 00:50Z (80 m, 500 Hz, 3 miles; GitHub issue #2) failed because his
station read none of KK4ODA-1's four acceptances (ADR-0048 is the other half). His recording holds
the two tone-floor acceptances, clean and 5–6 dB over the noise, and the first decodes when the
timing is given by hand (CONNECT_ACK, session 73). His detector, beta.72's and this build's alike,
refused both:

| | start block | middle block | end block | contradictions |
|---|---|---|---|---|
| hits of 8, first acceptance | 8 | 8 | 1 | 5, all in the end block |
| hits of 8, second acceptance | 7 | 6 | 1 | 5 |

The received frame had **slipped**. It holds about 40 ms more audio than was sent, entering
between symbols ~90 and ~127 and lining up with 6–10 dB dips in his audio level. From there every
symbol is a symbol late, so the end block's sync tones stand one slot after where the layout
expects them. Each is then the strongest tone of the *next* slot, and five of them were strong
enough to count as contradictions. ADR-0015's rule refuses any frame with more than two
contradictions. Where the slip came from (this station's transmission or his capture) is not
known.

## 2. Decision

A candidate that passes the hit counts (`MIN_HITS`, `MIN_BLOCK_HITS`) is confirmed when either
holds:

1. as before, no more than `MAX_CONTRADICTIONS` (2) of its 24 sync symbols are contradicted; or
2. its **two best blocks** each have at least `CLEAN_BLOCK_HITS` (7) of their 8 hits and **no
   contradiction**, whatever the third holds.

Why the second cannot admit what the contradiction rule was written against:

* **A shifted reading of a real frame** (ADR-0014's "block-spacing early" reading, the reading
  after a station's own silence). The three distances between a kind's blocks all differ
  (`ToneKind.block_offsets`), so no shift of a frame lines up more than one of its blocks with the
  pattern's. Such readings show one whole block: `[0, 8, 3]`, `[0, 8, 4]`.
* **Another air's frame read at a part-symbol offset** (ADR-0015's ghosts). These match about half
  of every block and are contradicted in every block (8–12 in all). Two blocks each at 7 or 8 hits
  with none contradicted is the opposite pattern.

## 3. Measured

* WC4Y's recording, replayed through this build's daemon (`aetherd --replay`, his sidecar's
  keying spans): both acceptances found and decoded (23.10 s, 69.64 s), where beta.85 found
  neither. The model's streaming receiver agrees.
* Synthetic slips (tone-24 at −4 dB on AWGN, 40 ms of the frame's own audio repeated at symbol
  *k*, 10 frames each): found 10/10 at every *k* from 90 to 120 (0/10 before). Decoded 0, 9,
  10 and 10 of 10 for *k* = 90, 100, 110 and 120: the code absorbs a slip that leaves enough data
  in place.
* False detections: see §4.
* `test_a_frame_that_slipped_a_symbol_stands_on_its_two_clean_blocks` (both suites), and the
  existing ghost, silence and noise tests unchanged.

## 4. False detections

A bench sends frames of every tone kind of both airs through both airs' detectors: AWGN, Good,
Moderate and Poor, at −8, 0, 10, 20 and 30 dB, half of them straight after a span of exact
silence (a receiver's own transmission). It counts every detection that is not the frame
sent, under the old rule and the new. The run is in progress. After 40 of its 80 points
(960 frames), the two rules agree on every detection: the same frames found, and the same
single false detection under both. This section is completed when the run ends.

## 5. Not done

* Re-timing the frame across the slip. A receiver that found the slip, by searching each block's
  own offset, could put the late data back in place and decode slips that fall earlier in the
  frame. It is worth it only if the air shows slips to be common.
* Finding the slip's source. Both stations' recordings of the same transmission, compared with
  `tools/tx_envelope.py` and `[record] tx_audio`, would say whether it is in the transmission or
  in the capture.
