# ADR-0063: An acknowledgement gives back a turn taken on noise

**Status:** accepted, 2026-10-11. Model first (`LinkEngine._took_unread`, `_give_back_turn`),
then the port (`took_unread`, `give_back_turn` in `aether-link/src/engine.rs`). No wire change.

## 1. Context

Found by the scenario harness while comparing ADR-0061 with beta.102
(`80m-lopsided-acks-500`, seed 1). The Test's ladder had run; the sender's last burst emptied
its queue and so ended with a TURN carrying OFFER (ADR-0047). Then:

1. 0.3 s after the sender released, its receiver reported a data frame that did not decode:
   acquired at 1.0007 of its threshold, CFO 41.9 Hz against the session's 2.5 Hz, SNR
   −14.7 dB. The physical layer marked it trusted.
2. ADR-0047's `_turn_taken_unread` reads exactly that — a trusted, undecoded ordinary data
   frame after an offer — as the other station's burst arriving, its acknowledgement with
   TAKEN lost. The sender became the receiver.
3. The other station's acknowledgement arrived a second later, without TAKEN: it had not
   taken the turn. A receiving station that reads an acknowledgement takes it for the other
   station answering a preamble of its own (ADR-0029) and does nothing more.
4. Both stations were now receiving. Each answered the other's occasional false detection with
   an acknowledgement and neither sent data; the link timed out 90 s later and the Test was
   lost ("the session dropped during the ladder").

The same seed passed on the other runs: the harness is not deterministic across runs, and the
false detection lands where it lands. Nothing in ADR-0061 is involved.

## 2. Decision

A station that became the receiver on a frame it could not read after offering the turn
remembers that it did (`_took_unread`). If it then reads an acknowledgement of its session
without TAKEN, that acknowledgement is the other station's word that it did not take the turn:
the frame was noise. The station becomes the sender again, drops the burst its noise started,
and handles the acknowledgement as the answer to its own burst. Decoded data from the other
station, a turn taken any other way, or a new session clear the flag.

## 3. Consequences

- `test_a_turn_taken_on_noise_is_given_back_by_the_acknowledgement` (model) and
  `a_turn_taken_on_noise_is_given_back_by_the_acknowledgement` (port): the noise arrives just
  before the acknowledgement, and the message sent afterwards is delivered. Without the change
  both stations sit receiving until the link times out.
- ADR-0047's inference still stands: when the acknowledgement with TAKEN really was lost, the
  other station's burst follows and no acknowledgement contradicts it.
