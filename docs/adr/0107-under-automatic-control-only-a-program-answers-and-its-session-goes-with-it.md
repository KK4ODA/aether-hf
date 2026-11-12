# ADR-0107: Under automatic control only a program answers calls, and a program's session goes with it

**Status:** accepted and built, 2026-11-12: the author's answers after the session between KK4ODA
and KK4ODA-1 of 2026-11-12 00:50Z. It amends ADR-0106 §2.1 and §2.2. The daemon
(`station/claims.rs`, `station/host.rs`, `Station::set_regulatory`), the control API
(`status.not_answering`), the panel's apps chip and the Control setting's tooltip, and the manual.
**No wire change, no model change, no configuration key.**

## 1. Context

KK4ODA (the mobile, the RadioPost client) called KK4ODA-1 (the gateway, RadioPost Gateway on
Aether's control API, `[regulatory] control = "automatic"`) at 2300 Hz on 7.101 MHz. Both stations'
files were uploaded. What they show:

* RadioPost Gateway claimed KK4ODA-1 at 23:05:01. Two Setup saves restarted the gateway's Aether
  at 23:14:30 and 23:15:29. RadioPost's Aether adapter kept the gateway's connection open with
  no Aether behind it, so the gateway never claimed again (`status.claims` was `[]` at 01:05).
  That is RadioPost's bug, fixed in RadioPost (RP-4 0.6.0 §2.1: an adapter that loses its modem
  closes the host's connection).
* KK4ODA-1 is also the callsign in the gateway Aether's own Setup. With no claim, ADR-0106 §2.1
  gave it back to the station, which answers its own callsigns always. Aether answered the call
  itself, under automatic control, with no program behind the session. The caller's 338-byte
  opening crossed in five minutes (00:56:00) and was delivered to nobody; the gateway sent 0 bytes.
* The RadioPost client on the mobile left Aether at 00:55:54, six seconds before its opening was
  delivered. No `disconnect` or `abort` reached Aether, and the session ran on, owned by nobody,
  the master polling every 22 s and the slave answering each poll with nothing, until the
  operator pressed Abort at 01:00:43.

ADR-0106 §2.1 meant "a caller gets no session with nobody behind it", but that held only for a
program's callsign that is not also one of the station's own. And it left a session in place
when the program that owned it went.

## 2. Decision

### 2.1 Under automatic control only a program answers calls

A call to a callsign no program holds is not answered while `[regulatory] control` is
`automatic`. The station's own sessions are the operator's (ADR-0106 §2.1), and under automatic
control no operator is at the station to carry one. A call to a callsign a program holds is
answered while that program listens, as before.

* **Probes are still answered** (the author's choice): an answer to a probe opens no session,
  and other stations can still measure the path to an unattended station whose program is down.
* Under local or remote control nothing changes: the station answers its own callsigns.
* The caller left unanswered is said in the log once a minute as before (`listen:` lines,
  `calls_unanswered`), with the reason *under automatic control only a program answers calls,
  and none holds KK4ODA-1*.
* A change of control (`config.set` of `regulatory.*`, a profile load), a claim taking the
  station's first callsign or a program going are each said in the log when they change whether
  it is answered.

### 2.2 A program's call or session goes with it

When the connection that owns the call or session under way closes (ADR-0106 §2.3: a claim's
connection, on the control API or the host interface):

* **a call** stops, as the panel's *Stop calling* and a host program's `DISCONNECT` while calling
  do (`abort` while connecting);
* **a session** closes as `disconnect` does (the author's choice over `abort`): what is already
  queued may still go within the usual drain (ADR-0072), then the closing exchange and the
  identifiers (ADR-0075).

The log says it in the claim's line: *RadioPost client has gone: no longer answering for KK4ODA;
its session with KK4ODA-1 closes, as Disconnect does*. A program that let its callsigns go
(`claim {callsigns: []}`) still owns the session it brought up, and its going ends it too. A
session the station owns (the panel's, a Test) is not touched by any program going.

RP-4 §1.1 already asks an adapter to abort its host's link when the host leaves. Aether now does
it whatever the adapter does, because the adapter may go with the program (RadioPost's is in the
program's own process) and nothing then reaches Aether.

### 2.3 What the operator sees

* `status.not_answering`: why calls to the station's first callsign are not answered, null while
  they are. `status.answering` now says whether *calls* are answered.
* The header's apps chip says *not answering KK4ODA-1* when calls to the station's first callsign
  are not answered and no program holds it, with the reason in its tooltip.
* The manual (step 1's Control, and section 12) says both rules.

## 3. Consequences

* An unattended gateway whose program has stopped, or lost Aether, answers nobody. A caller's
  probe is still answered, so it learns the path is there; its call goes unanswered, as a VARA
  gateway with its RMS program closed is.
* A gateway may still use the station's own callsign for its program (Setup KK4ODA-1, RadioPost
  Gateway KK4ODA-1); while the program holds it, nothing changes.
* A host program that restarts its modem connection mid-session (Trimode restarting its "VARA",
  the 2026-11-07 bench) now ends that session as Disconnect. It had been left with no program.
* The harness runs no station under automatic control, so its scenarios are unchanged. The two
  station tests of an automatic station that answers give it a listening program, as a real one
  has.
* **Tests:** `under_automatic_control_only_a_program_answers_calls` (a probe answered, a call
  not, then answered once a program claims and listens, not again when it goes, and answered
  under local control), `a_session_whose_program_goes_closes_as_disconnect_does` (the queued
  data crosses, the other station hears an orderly end),
  `a_call_whose_program_goes_stops`, and
  `a_program_going_leaves_the_stations_own_session_alone`. The first three fail without the
  change.

## 4. What it removes

* ADR-0106 §2.1's "the station holds its own callsigns, always listening": under automatic
  control it holds them and answers only probes for them.
* The session left up with no owner when its program goes.

## Not taken

* **Separate callsigns only** (RadioPost Gateway on KK4ODA-10, as RP-4 §9.1 suggests): it avoids
  the case for those who read the guide, and leaves the trap for anyone who reuses Setup's
  callsign.
* **Abort when the owner goes:** it drops what the program had already queued. Disconnect sends
  it within the drain and ends the same way.
* **Leaving it to the adapter** (RP-4 §1.1): the adapter can go with its program, as here.
