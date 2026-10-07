# ADR-0051: A connection waits for a scanning host to be listening again

**Status:** accepted, 2026-10-07 (the RMS Trimode bench). Host-adapter only
(`core/aetherd/src/host/server.rs`): a `CONNECTED` the station formed by answering a call is
held until the host's `LISTEN` is on again. No wire change, no model change.

## 1. Context

RMS Trimode is the Winlink gateway's TNC host, driving the modem over the VARA-compatible
command port (`host-interfaces.md` §1). It scans channels, and it turns answering on and off
with `LISTEN TRUE`/`FALSE` as it goes — off for the fraction of a second it is "deaf" between
scan steps, on while it dwells. Even configured for a single channel it cycles: on the bench it
sent `LISTEN ON`/`LISTEN OFF` about every 3.5 s, listening for ~3 s and deaf for ~0.5 s.

A station answers an incoming call only while it is listening (`station/host.rs`,
`HostFlags.listening`), so the answer begins inside a `LISTEN=ON` window. But answering is an
over-the-air exchange: the station transmits its acceptance and the engine reports the session
a second or two later. By the time the adapter had a `connected` state to turn into `CONNECTED`,
Trimode's scan had come round to a `LISTEN=OFF` blip — and Trimode **ignores a `CONNECTED` that
arrives while its own `LISTEN` is false**, logging `Ignoring CONNECTED while LISTEN=FALSE` and
starting its disconnect timer "currently not connected." The client's modem was connected and
waiting for the gateway's welcome; the gateway's Trimode never knew a session existed; nothing
ran, and the link timed out.

Found on the RMS Trimode bench (2026-10-07): two daemons over `[sim]`, Winlink Express
(`KK4ODA`) as the client, RMS Trimode 1.4.4.0 + RMS Relay 3.3.5.0 as the gateway. An earlier
attempt had connected and run a whole B2F session cleanly — its `CONNECTED` had simply landed in
a `LISTEN=ON` window. The failure was a race, not a dead end.

## 2. Decision

The host adapter holds the `CONNECTED` for an **answered** call until the host is listening
again, and emits the whole block then (`PENDING` for the called side, `REGISTERED`, `CONNECTED`,
`ENCRYPTION DISABLED`, `LINK REGISTERED`). The session is already up on both modems; only the
host's notification is timed, so that a scanning RMS receives it inside one of its own listening
windows.

* **Only the answering (IRS) side is gated.** A call *this* host placed (ISS) is never held: the
  host is not scanning for it, it is waiting for the outcome of its own `CONNECT`, and `LISTEN`
  does not bear on it.
* **A disconnection before `LISTEN` returns cancels the held connection** — the host is told of
  neither, which is right: it never learned a session began.
* A listening gateway toggles `LISTEN` back on within its scan period (sub-second on the bench),
  so the wait is short; the hold is cleared the moment the `LISTEN ON` command is read.

## 3. Consequences

* A gateway's Trimode accepts the session across its scan cycle, deterministically, instead of
  roughly once in the fraction of attempts whose `CONNECTED` happened to miss the deaf window.
* No wire change: beta.89 and this build interoperate; the two modems' exchange is untouched.
* No model change: the link engine already formed the session correctly. This is the adapter
  timing the host notification, nothing more.
* Tests: `a_connected_waits_for_a_scanning_host_to_be_listening_again` (the held-and-released
  path); `a_bandwidth_command_moves_the_station_and_connected_says_the_new_one` now sends
  `LISTEN ON` before its IRS connect, as an answering station is listening (as its sibling
  `a_station_that_was_called_puts_the_caller_first` already did).

## 4. Not done

* A safety timeout that emits the held `CONNECTED` anyway if `LISTEN` never comes back. A
  listening gateway always cycles back; a host that has stopped listening does not want the call,
  so holding is the honest outcome. Revisit only if a real host is found that answers calls yet
  stops toggling `LISTEN` on.
* Reporting the session to the host the instant the engine accepts the call (before the
  acceptance leaves the air). That is a model change for a smaller window than the deaf blip, and
  the adapter-side hold closes the window completely without it.
