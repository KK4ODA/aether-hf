# ADR-0055: Nothing that sets up a contact carries the identifier

**Status:** accepted, 2026-10-10. Daemon only (`station.rs`: `IdentifierState.communication_began`,
`setup_at`, `append_cw_id(…, setup)`, `advance_identifier`, `sets_up_contact`, `SETUP_ID_WAIT_S`).
No wire change, no configuration key.

## 1. Context

The Morse identifier (`[radio] cw_id`) went on a transmission whenever ten minutes had passed since
the last one — and on the first transmission after a start, with no identifier yet. In the
sessions of 2026-10-09/10:

* KK4ODA-1 called KO4WX after a quiet spell, and the call carried the identifier: KO4WX's answer
  waited it out, and the call's next try came first.
* KK4ODA-1's first transmission after a restart was its answer to WC4Y's probe, and the answer
  carried the identifier (00:19:46), seconds of Morse on a frame the prober times its wait by.

§97.119(a) asks a station to identify **at the end of each communication, and at least every ten
minutes during a communication**. A call, an acceptance, a probe and its answer are the start of a
communication, not its end.

## 2. Decision

1. **A transmission that sets up a contact carries no identifier**: a connect request, an
   acceptance, a probe, a probe's answer (`sets_up_contact`, read from the frames' DATA header).
2. **The ten minutes count from the communication's first transmission** (`communication_began`):
   a call and the session it opens are one communication, so the interval is not met by an
   identifier an hour before it.
3. **The end of a session is identified, however it ended** — the DISC or DISC_ACK carries it, or
   it goes on its own after a link timeout — as before.
4. **A contact that goes no further than its setup is identified on its own** once the station has
   been idle `SETUP_ID_WAIT_S` (30 s) since: a probe answered and no call after it, a probe this
   station answered and no call came, a call never answered. A call that follows within the wait
   (the Test calls at once) makes it one communication, identified at its end.
5. Beacons and datagrams are communications of their own and carry the identifier when the
   interval says so, as before.

## 3. Consequences

* A session shorter than ten minutes is identified once, at its end, by each identifying station;
  before, the caller identified on its call too.
* The tests that used a call to provoke an identifier now use a beacon
  (`an_identifier_inside_a_burst_is_judged_as_the_cw_it_is`,
  `a_station_identifies_in_morse_when_asked_to_and_not_otherwise`,
  `the_identifier_does_not_repeat_inside_its_interval`); `a_session_ends_with_an_identifier_however_it_ends`
  expects one identifier for an orderly close instead of two.
