# ADR-0061: A frame still arriving holds the acknowledgement

**Status:** accepted, 2026-10-11. Model first (`LinkEngine._arriving`, `_account_for`,
`_burst_end`; `top_acked_mode`), then the port (`arriving`, `account_for`, `burst_end`,
`top_acked_mode()` in `aether-link/src/engine.rs`; the Test's ladder start in
`aetherd/src/station/fieldtest.rs`). Engine only: no wire change, no configuration key.

## 1. Context

WC4Y's Test of 2026-10-11 00:57Z (80 m, 3.590 MHz, 500 Hz, −2 to −7 dB both ways), recorded
at both ends with audio. It completed: probe, connect, a 512-byte message, an eight-rung
ladder and a 512-byte file. The operator at KK4ODA saw what looked like a PTT hiccup on the
radio about six minutes in: two keyings in quick succession.

That is what happened. KK4ODA-1's sidecar has the key released at 383.113 s and keyed again
at 383.122 s: 9 ms. Two acknowledgements went out back to back, 6.9 s keyed in all against
3.4 s for every other answer. The first (keyed at 379.64 s) started 0.34 s before WC4Y's
burst ended (the fifth tone frame ran 374.62–379.98 s); WC4Y released at 01:03:28.19 by his
clock, KK4ODA-1 keyed at 01:03:27.83 by its own. WC4Y could not hear it, being keyed. The
second carried the acknowledgement that mattered, and he decoded it.

The cause was in the engine:

1. Frame 4 decoded; the acknowledgement was armed for the silence after it.
2. Frame 5's preamble was heard (a tone frame, trusted) and `on_preamble` moved the
   acknowledgement to that frame's end plus the reply delay — 379.98 s and later.
3. At 377.49 s, in the middle of frame 5, the detector took noise for an ordinary frame
   (rung 11, acquisition 1.02, untrusted, chips reading "none follow"). It failed to decode,
   but a failed frame of the session is still a frame of the burst: `_burst_end` took the
   latest end of any frame *reported* — the phantom's, 378.55 s — and the burst path
   re-armed the acknowledgement there plus the reply delay (0.99 s on the floor). The arm
   replaced the deadline frame 5's preamble had set.
4. The acknowledgement keyed at 379.64 s. Frame 5 decoded at 380.14 s from audio already
   captured, new data arrived, and the second acknowledgement followed the first in the same
   keying.

ADR-0040 and ADR-0042 had already made a later frame never bring the burst's end earlier
than an earlier frame's *countdown*. Tone frames carry no countdown, and the arrival
announced by a preamble was not part of `_burst_end` at all.

## 2. Decision

1. **The engine remembers the frames announced and not yet reported** (`_arriving`: start
   and end, from `on_preamble` on a receiving station). A frame reported (`on_frame`, any
   container) accounts for the arrival that began within half a frame of its own start, and
   for any that ended before it began. **The burst does not end before a frame still on the
   air**: `_burst_end` is no earlier than the end of any arrival left. A frame reported inside
   an arrival — a false detection in its middle — therefore cannot set the answer for its
   own end.
2. The arrivals are forgotten wherever the burst is (the acknowledgement sent, a turn taken,
   a session ending).
3. **The Test's ladder starts a rung under the fastest rung the message was acknowledged at**
   (`top_acked_mode`), not under the recommendation at the message's end. The same session's
   message went at rungs 1 and 3; two lost single-frame retries of its last block stepped the
   recommendation down to 1, so the ladder started at rung 0 and climbed rungs 0–3 again,
   4/4 each, for 105 s of a 412 s Test. Started under rung 3 it would have skipped two of
   them.

## 3. Consequences

- A phantom announced arrival (a heeded preamble with nothing behind it) still holds the
  acknowledgement to its end, as `on_preamble` alone always did; a later frame report no
  longer shortens that. ADR-0040 accepted the same cost: a late answer rather than one keyed
  over a frame.
- `test_a_frame_reported_inside_an_announced_one_does_not_end_the_burst` (model) and
  `a_frame_reported_inside_an_announced_one_does_not_end_the_burst` (port) replay the
  session's order of events.
- Not changed: the first frame of the file's first burst (353.18 s, starting 0.44 s after
  KK4ODA-1's release, acquired at 54 but failed) still fails on replay. The audio is there and
  the receiver's level rises about 3 dB over that half second. One frame in five, at
  −4.2 dB, on a rung that decodes at −5 to −6 dB elsewhere in the session; worth a look if
  first frames keep failing on this path.
