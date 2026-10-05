# ADR-0039: A sender's Disconnect gives up on what the path will not carry; the occupied width says which measure it is

**Status:** accepted, 2026-10-05.
* The link engine: `LinkConfig.disc_patience_exchanges` (2) and `disc_patience_s` (20 s),
  `_exchange_s`. Model first, both suites; the Rust simulator gains `set_thresholds`.
* The rules check's summary, and the panel's *Occupies* fact.
* The panel's banner (`bannerFor`, `syncBanner`), which now shows every action under way.
* No wire change, no configuration change.

## 1. Context

ND1J's messages from his two sessions with KK4ODA-1 (ADR-0038), as he typed them:

* "Hmm, disconnect button does not work. My last msgs did not send and my disconnect button
  would not work to disconnect." A sender's Disconnect means "deliver everything queued, get it
  acknowledged, then DISC" (`disconnect()`). His last messages were frames at a rung the faded
  path could not carry, and this station's acknowledgements were not reaching him. So the queue
  never emptied, and the DISC never went. His session ended only when the link timed out: 85 s
  or more on the 500 Hz floor. The panel said "Closing: sending what is still queued first…
  Abort closes at once." He read that as a button that did nothing. A host program sending
  `DISCONNECT` waits the same way.
* "Why is it saying 709 Hz wide and not 500?" The rules check's summary read "FCC: data
  permitted — 709 Hz fits the 80 m data segment", shown on the dial readout beside the 500 Hz
  setting.
  * The 709 Hz is the FCC's occupied bandwidth: §97.3(a)(8), out to 26 dB down, the wider of
    the two readings the check takes (ADR-0018).
  * The 500 Hz mode's OFDM rungs measure 691–709 Hz that way. Their power sits within about
    570 Hz; the tone rungs measure 551–609 Hz.
  * The number was right and the label made it look like a fault.

## 2. Decision

1. **A sender asked to disconnect waits for its queue, but not for ever.** Once it has been
   asked, the next time it would send a burst it starts a patience:
   * `disc_patience_exchanges` (2) whole exchanges at the family the link runs in, or
     `disc_patience_s` (20 s), whichever is longer. An exchange (`_exchange_s`, now shared with
     the link timeout) is a full burst of the longest frame either side sends, its
     acknowledgement, both turnarounds and the gap.
   * Every newly acknowledged frame starts it again.
   * When it runs out, the engine reports `disconnect: <n> bytes not acknowledged; leaving
     without them` and sends its DISC, and the session ends as any other does: DISC_ACK, or the
     DISC's retries.

   **Why exchanges and not seconds.** A fixed time is wrong at one end of the ladder or the
   other. One exchange is about 5 s on the fast OFDM rungs, so a fixed 30 s would wait out six
   exchanges for nothing. On the tone floor it is 30–60 s, so 30 s would cut off a link still
   making progress. Two exchanges is half the link timeout (`link_timeout_exchanges`, 4), so a
   Disconnect always finishes before the session would have died of silence anyway. The 20 s
   floor keeps a fast link from abandoning its data over a couple of lost acknowledgements.

   Tested in both suites
   (`a_disconnect_leaves_without_what_the_path_will_not_carry`): a 500 Hz session where data
   stops decoding and control frames still do ends with the DISC and the report, not the link
   timeout. Without the patience the same test ends by link timeout.
2. **The occupied width says which measure it is.** The summary now reads "FCC: data permitted
   — occupies 709 Hz by the FCC's 26 dB measure, inside the 80 m data segment". The panel's
   reasoning shows *Occupies (FCC 26 dB)*, with a tooltip on why that is wider than the mode's
   nominal width.

3. **The banner says what the station is doing until it is done.** It covers every tab, and it
   said only connected, calling and how a session ended. A Disconnect that was still sending what
   it had queued showed "CONNECTED" the whole time, which is how ND1J came to think the button
   did nothing. It now shows, from the status the panel polls:
   * **Calling**, **connected** and **ended**, as before.
   * **Disconnecting**, in amber, for both phases: "sending what is still queued first… Abort
     closes now" (or, for a receiving station, "after the burst now arriving"), then "waiting
     for the answer".
   * In blue, until done: **a Test session** and its step, **a probe** (with the station's name
     when the panel sent it), **a beacon waiting** for a clear channel or going out, and
     **a transmission** (a tuner tone, drive bursts, the identifier).

   A session's end stays up for its 15 s; the identifier that follows it does not replace it.
   The ended banner says what a sender left behind, and the engine's `disconnect` event is a
   warning in the log.

## 3. Consequences

* What a sender leaves behind is reported, and the daemon's sent-message tracking marks it
  undelivered with the session's end, as it does for any session that ends with data in flight.
* The ordinary close is unchanged: a queue that empties sends its DISC at once.
