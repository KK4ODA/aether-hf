# ADR-0052: A session ends at both ends, and takes its data with it

**Status:** accepted, 2026-10-08. Model first (`LinkEngine._end_session`, `_answer_left`,
`_left_data`, `_repeat_leave`; `LinkConfig.leave_repeats`; stat `left_answered`), then the port
(`end_session`, `answer_left`, `left_data`, `repeat_leave`, `Timer::Leave`). No wire change: the
frames are the DISC and DISC_ACK every version knows, and beta.91 and this build interoperate.

## 1. Context

The scenario harness's robustness set (`bench/scenarios/*-gateway-*`, `*-abort-*`, `*-leave-*`,
written after the host benches of 2026-10-07 to find on the bench what the RMS trial would
otherwise find on the air) failed three ways that are one problem seen from two sides:

1. **Data outlived its session.** A client vanished mid-transfer (`outage`); both stations
   timed out to idle; its next call connected and began by sending the 18 kB the dead session
   had left in the engine's queue, ahead of the 2 kB it was asked to send — to the same gateway
   here, but the queue does not know where it is going: the next session could be to another
   station entirely. The same after `disconnect both` and after an abort. Nothing cleared the
   queue when a session ended: `_reset_transfer_state` dropped the frames in flight and kept
   what was not yet framed.
2. **An abort was one frame.** `abort()` sent a single DISC and went idle. In the harness it met
   the other station's acknowledgement and was lost; the other station — receiving, so it
   transmits only to answer — heard nothing more and stayed in the session until its link
   timeout, 144 s at 2300 Hz. A gateway left that way is closed to every other caller for
   minutes.
3. The same holds after any end without the other station's word: a DISC never answered, a
   sender's polls never answered ("no response"), a link timeout.

VARA clears its buffer at `DISCONNECTED`; its `ABORT` is a "dirty disconnect" that leaves the
other side to time out. Aether can do better than the second at no cost on the wire.

## 2. Decision

1. **A session's data is that session's.** `_end_session` clears the send queue, whatever the
   reason. A message given to a call that fails, or a session that ends, is not carried into
   the next. (The station layer already marks what was not delivered as undelivered, with the
   reason; the host adapter already keeps what a program writes with no session up in its own
   pipe, not in the engine.)
2. **A station that left a session unheard answers it.** When a session ends by `aborted`,
   `closed (no DISC_ACK)`, `no response` or `link timeout` (`LEFT_UNHEARD`), the station keeps
   its number for as long as the other station's link could still be waiting (the link
   timeout from then). While idle, a frame of that session —
   * a control frame: answered at once with a DISC (a DISC with its DISC_ACK), in the family it
     was heard in;
   * a data frame: the burst is answered with a DISC once it has ended — a frame's length and a
     turnaround after the last frame of it heard, re-armed by each.
   A DISC_ACK from the other station, a new session (called or calling), or the window running
   out forgets it.
3. **An abort's DISC is said again.** After an abort the DISC goes again on the floor, up to
   `leave_repeats` (3) more times, a response wait apart, until the other station's DISC_ACK
   (or anything of the session, answered as in 2) is heard. The station is idle meanwhile: the
   operator's abort is immediate; only the courtesy to the other station continues.

## 3. Consequences

* The robustness scenarios that failed now end both stations together: `40m-abort-and-recall`,
  `80m-leave-together`, `40m-gateway-client-vanishes`, and their soak variants.
* A station that has just aborted may transmit up to three short DISCs in the next twenty-odd
  seconds while idle; each is a response to its own session and goes through the regulatory
  gate as any frame does.
* A message typed for a session that then failed must be sent again in the next. That is what a
  host program does after `DISCONNECTED` anyway.

## 4. Not done

* Repeating the DISC after the other ends (`no response`, `link timeout`): the other station
  had stopped answering, and repeating into silence only keys the radio. Answering its frames
  (2) covers the case where it comes back.
