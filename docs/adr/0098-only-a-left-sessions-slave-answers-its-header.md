# ADR-0098: Only the station that was a left session's slave answers its header

**Status:** accepted, 2026-11-02, a fix to ADR-0052 as protocol 8 carried it (ADR-0069). Model
first (`GrantedEngine._end_session`, `_answer_left`), then the port (`end_session_with`,
`answer_left`); the trace vectors gain the session `left-header`. **No wire change, no
configuration key, no schema bump.**

## 1. Context

ADR-0052 has a station that leaves a session without the other station's word (an abort, "no
response", a link timeout, a DISC never answered) remember the session for a link timeout and
answer its frames in their slots: a DISC with a DISC_ACK, a header with a DISC, so that the station
still in the session leaves too instead of waiting out its own timeout.

Under granted slots only the master sends headers. To the station that was the slave, a header
with the left session's number is the master it left, still granting slots: the DISC is the
answer ADR-0052 meant. To the station that was the master, nobody in its own session sends
headers, so a header with that number belongs to another pair's session. A control frame carries
the session number's low seven bits and no callsign, so a match comes once in 127 draws.

The weekly gateway day found it (`bench/scenarios/long/40m-gateway-day-2300.toml`, the Weekly run
of 2026-11-01). Every round ends with an outage under ND1J's session with the gateway. ND1J, its
master, closes "no response" and remembers the session. In two rounds of 29, KE4QCM's next session
with the same gateway drew a number with the same low seven bits. ND1J, idle, heard KE4QCM's first
header and answered it with a DISC in the slot that header granted the gateway (0.23 s, keyed in
the same slot as the gateway's answer). KE4QCM decoded the DISC as the gateway's, closed with its
own, and both stations ended "peer disconnected", 20 s and 15 s after connecting. The client's
1 500 bytes went in its next session instead, 3 000 bytes there. The steps waiting for them used
their whole budgets, and the scenario's six hours of air ran out before its last rounds; the
"did not connect" in the report is the last round's.

On the air it is the same: a client that left a gateway's session as its master, and remembers
it, ends one in 127 of the gateway's next sessions with other clients for as long as it remembers.

## 2. Decision

The memory of a left session also says whether this station was its slave. A header is answered
with a DISC only then. Nothing else changes: a DISC is still answered with a DISC_ACK in either
role (a slave says DISC in a slot its master granted, and a master that has just left can still
hear it), a DISC_ACK still ends the leaving, and an aborting master still says DISC from idle
(`_repeat_leave`).

## 3. What was measured

* `test_only_a_station_that_was_the_slave_answers_a_left_sessions_header` (model, both airs): a
  path that dies; the master closes "no response", the slave "link timeout"; then a header with
  the session's number. The master that left sends nothing (before: a DISC); the slave that left
  sends a DISC, as before.
* `granted_vectors.json` gains `left-header`, appended, the same case through the port; no other
  session changed. With the port's check taken out, the replay fails at that session's step 57
  (`left_answered` 1 against the model's 0).

## 4. Not done

* The session number stays seven bits in the control frame: widening it is a wire change.
* A master does not check that a DISC came from its slave. Frames carry no callsign, and the
  stray DISC came in the slave's own slot, where only timing could have told them apart.
