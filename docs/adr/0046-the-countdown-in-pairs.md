# ADR-0046: The burst countdown counts in pairs, and never short

**Status:** accepted, 2026-10-06, the author's request ("fix the countdown ceiling"). Link
protocol 6: `countdown_of` / `most_following` (`link/frames.py`, `frames.rs`), the receiver's end
of a burst (`_announced_end`, `_burst_end`), the sender's wait for the acknowledgement; the
shell's `LINK_PROTOCOL` is 6. The physical layer is unchanged: the same four quarter turns.

## 1. Context

ADR-0041 turned an ordinary DATA frame's mode chips by the number of frames of its burst that
follow it, capped at 3, "three or more". A burst is six frames, so its first three all said 3.
In the scenario harness (ADR-0045, 80 m at 500 Hz) a receiver that decoded only the first frame of
a burst of six took the burst to end three frames later and answered over the last two — twice in
one session. ADR-0042 had noted it as the countdown's open item.

Two cheaper fixes were tried and rejected:

* **Eight phases** (0–6 following, and an eighth value for ADR-0047's offer). Measured through the
  real modem (`bench_follows`-style, 60 frames a point, both airs, AWGN to flutter and NVIS, −6 to
  +12 dB), decoded frames read up to 39° off their sent turn on the 500 Hz flutter and NVIS
  channels — past the ±22.5° eight phases allow. Four phases have ±45°.
* **Reading a 3 at its slot** (no wire change): taking a 3 heard at slot `s` as "up to `5 − s`".
  The slot counts from the first frame heard, so a lost first frame made the receiver answer a
  frame late; the sender's wait ran out and it sent again over the answer. Over ten fade patterns
  of the faint-frame session collisions went from 2 to 72.

## 2. Decision

1. A frame's countdown is half the frames that follow it, rounded up: 0, 1 for one or two, 2 for
   three or four, 3 for five or six (`countdown_of`). It never says fewer than follow and at most
   one more; it is exact at the last frame, so ADR-0045's prompt answer stands.
2. A receiver takes `2f` frames after a believed frame as the latest the burst may end
   (`most_following`), and the burst's end as the tightest such bound — no earlier than the end of
   any frame heard. The rule that a later frame never brings the end earlier (ADR-0042) becomes:
   a frame neither decoded nor trusted gives no bound.
3. The sender's wait for the acknowledgement of an ordinary burst of two frames or more is one of
   its frames longer: a receiver that lost the last frame takes the one before at its word — one
   or two following — and answers that frame late.
4. Link protocol 6: a station of version 5 would read a 1 as one frame where it says one or two.
   The release notes say "Update required", the manifest carries 6 and the shell titles the offer
   so (ADR-0041's mechanism).

## 3. Measured

The link bench, a fade of 3.5 s every 11 or 15 s over a 3 kB transfer — the ends of bursts lost —
twelve patterns each on both airs: collisions 7 → 0, every transfer delivered, air time unchanged.
The faint-frame session (ADR-0042), ten patterns: 2 → 2. Tests in both suites:
`the_countdown_counts_in_pairs_and_never_short` and
`a_fade_over_the_end_of_a_burst_is_not_answered_over` (three patterns that each collided on
version 5).

## 4. Consequences

* Beta.84 and the build after this do not connect; the update is required.
* Bursts longer than seven frames (a configured `burst_frames` above six) saturate at 3, "five or
  six", and can be answered early again; the default is six.
