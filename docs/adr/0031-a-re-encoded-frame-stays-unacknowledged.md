# ADR-0031: A frame re-encoded and left out of its burst stays unacknowledged

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`: `_TxRecord.sent`,
`_unacked`, `_on_ack`), then the port (`aether-link` `engine.rs`: `TxRecord::sent`; `sim.rs`:
`with_thresholds` takes one entry per rung). No frame, link protocol or configuration change.

## 1. Context

Found while measuring ADR-0032 on the chat bench (`tools/bench_chat.py`, ADR-0027): at 0 dB on
the 500 Hz air a sending station went silent with data queued — no burst, no poll, no timer but
the link timeout — until the session ended. Traced on the engine at `f17fec5` (trial 113 of the
drop run, today's turn-taking, ITU Good):

| time (s) | |
|---|---|
| 89.83 | B re-encodes six frames stranded at rung 4 (3–8) on the tone floor; five go out (3–7) |
| 89.83 … | frame 8 is never sent again: everything B sends after it arrives and waits behind it |
| 428.34 | sixteen frames outstanding (8–23), the window full: B's burst has nothing to send |
| 539.06 | A, which has heard nothing since, ends the session: link timeout, four lines lost |

For 340 s the conversation ran one way only: A's lines arrived, B's did not, and neither
station knew.

A frame sent `max_combines` times at a rung the recommendation has since left is re-encoded at
the slowest rung that carries its body (P6-7): a new codeword, whose transmissions are counted
from zero. Every stranded frame of the burst is re-encoded at once. The burst then carries one
family (ADR-0009) and no more frames than fit the key time (ADR-0017): five tone frames of
5.36 s in the daemon's 28.85 s. At 500 Hz the first OFDM rung carries 12 bytes a frame, so a
chat line of 60 bytes or more fills a six-frame burst, and six stranded frames became five tone
frames and one re-encoded frame with no transmission. `_unacked()` counted a frame only if sent
under its current codeword (`tx_count > 0`), so that frame dropped out of the unacknowledged
frames and nothing sent it again:

* the receiving station waited for it forever, and delivered nothing sent after it (the stream
  arrives in order);
* the window filled behind it, and `_send_burst` returned without sending or arming anything —
  the sender held the turn in silence until a link timeout, or, as the caller, ignored the
  other station's polls (ADR-0023) until that station gave up: "no response";
* with nothing sent after it, the sender counted everything acknowledged: `disconnect()` sent
  its DISC and both ends closed "cleanly", the frame missing.

Before this change, in the drop run's 1 200 sessions at 0 dB (100 a point; Good, Moderate and
Poor; both airs; both turn policies), seven left a frame so, and five of them dropped or stalled
— every session that dropped at 0 dB. At −12 and −6 dB, two of 2 400; one stalled.

## 2. Decision

**A frame counts as sent once it has gone on the air under any codeword** (`_TxRecord.sent`:
`tx_count > 0 or reencoded > 0`). It stays unacknowledged until acknowledged and goes out in
the next burst that has room for its family, and an acknowledgement covers it whichever
codeword the other station decoded (it may have decoded the old one and had its
acknowledgement lost). The two places that already counted a frame as sent this way (a frame
resent, and the Test ladder's fresh frames) use the same property.

Not changed: all of a burst's stranded frames are still re-encoded together. A frame given its
new codeword a burst early loses nothing, and the family and key-time rules decide what goes in
each burst as before.

## 3. Measured

The engine before (`f17fec5`) and after, on the fading pipe with the floor's reading cap (P9-6,
ADR-0016), bursts held to the daemon's key time, the same seeds. `tools/bench_chat.py` on Good,
Moderate and Poor, both airs: today's turn-taking (`base`) and ADR-0027's request (`request`).

| `bench_chat.py` | sessions | identical | drops | stalls | lines lost | median, 90th percentile, keyed per line |
|---|---|---|---|---|---|---|
| today's, −12, −6, 0 dB, 100 a point | 1 800 | 1 797 | 9 → 6 | 0 → 0 | 71 → 55 of 27 612 | unchanged (1.00, keyed ≤ 1.01) |
| today's, −12 to +12 dB, 30 a point | 900 | 899 | 3 → 2 | 0 → 0 | 26 → 19 of 13 710 | unchanged (keyed ≤ 1.01) |
| request, −12, −6, 0 dB, 100 a point | 1 800 | 1 799 | 1 → 1 | 1 → 0 | 20 → 12 of 27 612 | unchanged |
| request, −12 to +12 dB, 30 a point | 900 | 899 | 4 → 3 | 0 → 0 | 40 → 33 of 13 710 | unchanged (keyed ≤ 1.02) |

Six sessions changed, all at 500 Hz: each had dropped or stalled with a frame left out, and
each now completes. Only such a session can change, because the property differs from
`tx_count > 0` for no other frame. Two sessions (Poor, 0 dB, trials 117 and 180) leave a frame
out and end as before. There the frame was left out of the first floor burst after four
unanswered OFDM bursts, and the session ends during that burst: the 45 s link timeout, armed
with the last frame heard while the link ran ordinary, runs out before the 27 s tone burst and
its acknowledgement are over. That is its own fault, the link timeout not following the sender
down to the floor, and it is not changed here.

`tools/bench_link.py` (2 kB, Good, Moderate and Poor, −12 to +12 dB, both airs, 20 sessions a
point): identical in all 600 sessions. The transfer bench holds no burst to the key time, so six
tone frames fit.

`bench/baselines/reencoded_chat.csv` (`engine` before/after, `policy`, `first_trial` 0 the
grid and 100 the drop run) and `bench/baselines/reencoded_link.csv` have the rows.

## 4. Consequences

* Tests, model and port: `a_frame_re_encoded_and_left_out_of_its_burst_goes_in_the_next` (500 Hz,
  a path that never carries rung 4, one burst of six frames: before, the session closed with
  the last frame missing) and `a_frame_sent_under_an_earlier_codeword_is_still_unacknowledged`
  (the bookkeeping: unacknowledged, sent in the next burst, holding the DISC, and covered by an
  acknowledgement of its old codeword).
* The daemon holds every burst to `burst_limit_s` (ADR-0017). This happened on the air wherever
  six stranded frames went to the floor together.
* `TwoStationSim::with_thresholds` took fourteen entries, from when the ladder was the OFDM
  table. Nothing called it, and it now takes a slice, one entry per rung of either ladder.
