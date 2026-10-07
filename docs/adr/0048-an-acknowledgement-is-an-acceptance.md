# ADR-0048: A caller that reads an acknowledgement of its own session has been accepted

**Status:** accepted, 2026-10-07. Model first (`LinkEngine._accepted_unread`), then the port
(`accepted_unread`); stat `acceptances_inferred` (and the `counters` of `status`); event
`accepted`. No wire change: beta.85 and this build interoperate.

## 1. Context

WC4Y's Test of 2026-10-05 00:50Z (80 m, 500 Hz, 3 miles; GitHub issue #2) ended "the call timed
out". Both stations' sidecars, and WC4Y's audio, say what happened:

* KK4ODA-1 decoded WC4Y's call, accepted it and was connected. It sent four acceptances: two on
  the tone floor, two in OFDM.
* WC4Y read none of them. The two tone-floor acceptances are in his audio, 5–6 dB over the
  noise, and the first decodes with genie timing; the detector refused them (ADR-0049).
* WC4Y's next tries arrived at KK4ODA-1 unreadable, and KK4ODA-1, connected, answered them as
  ADR-0016 says: with an acknowledgement of the session, on the floor. WC4Y read one of those at
  −7.5 dB, 50 s into the call.
* A calling engine drops every control frame (`_on_control`: `state is CONNECTING` → return).
  WC4Y kept calling until its tries ran out, and KK4ODA-1 ended on the link timeout.

## 2. Decision

A caller that reads an **acknowledgement** carrying **its own session number** takes the call
as accepted, when the acceptance would have told it nothing more.

1. **The evidence.** The session number is the caller's own, drawn for this call (1–255), and
   the called station takes it only from a request it accepted. A station that has not accepted
   sends nothing in that session. Only an acknowledgement counts: it is what a connected called
   station sends to a try it cannot read; a DISC of the session does not make a session.
2. **What the acceptance would have said.** Its body carries the bandwidth, the link protocol
   and the capabilities. The first two are settled by the acknowledgement: a station accepts only
   a request in its own bandwidth and protocol (ADR-0026, ADR-0041). Capabilities beyond the
   bandwidth are compression alone, and compression is used only when both offer it
   (`compress::negotiated`). So the rule applies when the caller offered none: what the two agree
   is then nothing, whatever the other offered. A caller that offered compression keeps calling
   for the acceptance itself. Compression is off by default.
3. **What the caller does.** As on an acceptance: connected, ISS, the connect timer disarmed, the
   link timer armed, the peer's capabilities taken as its own offer (the same bandwidth, no
   compression). Its rate controller is seeded from the acknowledgement's own SNR (a lower bound
   when it came on the floor, ADR-0016), and its first burst starts from the acknowledgement's
   report of how the other station heard it when it carries one, else from `initial_mode`. It
   logs `accepted:<call>: its acceptance was not read; an acknowledgement of session <n> was`.

## 3. Measured

`test_an_acknowledgement_of_the_callers_session_is_its_acceptance` (both suites) is session 1 in
the simulator. The caller hears no data frame, and every try after its first arrives unreadable.
Before this change the call ends `no answer` with nothing delivered. After it, the caller is
connected by the first acknowledgement and the message crosses.
`test_a_caller_that_offered_compression_waits_for_the_acceptance` holds the limit of §2.2.

## 4. Not done

* A caller that offered compression could learn the other station's offer some other way, for
  example by sending its request again at once on reading the acknowledgement, so that the
  repeated acceptance arrives sooner. Not built: compression is off by default, and no field
  session has asked for it.
* The acceptances were lost to a detector that refused a frame it could decode. That is the
  first fault, and ADR-0049 is about it. This rule is the second line.
