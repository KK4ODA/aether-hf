# ADR-0103: A called program hears its acceptance keyed before CONNECTED

**Status:** accepted, 2026-11-07, the author's go. Amends ADR-0096 §2.3 (which keyings a program
that keys nothing hears). Host adapter only (`core/aetherd/src/host/server.rs`): **no wire
change, no model change, no configuration key.** It also logs the data port's connections
(§2.4), which changes no behaviour.

## 1. Context

RMS Trimode has a watchdog of its own. On 2026-11-07 at 15:02:06 KK4ODA-1's Trimode (beta.138, the
host program keying the radio) logged *No PTT from Vara. Connection appears to be hung* one second
after it heard `CONNECTED` for the mobile's call, and dropped the session. That was the day's only
such abort; its last PTT before it was 13 minutes old. The author repeated it on the Trimode bench
the same afternoon (beta.140, two `[sim]` daemons, a scratch Trimode and Winlink Express; CLAUDE.md
has the notes):

| Test | Since the last PTT Trimode was told of | At `CONNECTED` |
|---|---|---|
| A | none; Trimode had started its "Vara" 8 min before | session runs |
| B | none; Trimode had restarted it 1 min before | session runs |
| C | 16 min (the last session's closing identifier) | *Vara connection is active, but no PTT is being done* and *No PTT from Vara. Connection appears to be hung*, in the second of `CONNECTED` |

The host trace was the same at all three connects. The station takes the call, says `PENDING`,
and keys its acceptance. The `CONNECTED` goes during that keying, and the keying itself is never
told: the first `PTT ON` Trimode hears comes with the next transmission. The only difference was
how long ago the last PTT had been.

How Trimode judges this beyond its log lines is not known: the limit lies somewhere over 8 min,
or it skips the check until a run's first PTT. Either way the same fact is missing. When it hears
`CONNECTED`, the station has just transmitted, and Trimode has not been told.

Why it was not told:

* **When the program keys nothing** (the bench), ADR-0096 §2.3 counts a call whose `CONNECTED` is
  held (ADR-0051, ADR-0100) as a session the program was not told of, and says no `PTT` for it.
  That rule came from the night of 2026-10-25. There the `PENDING` waited with the `CONNECTED`,
  and an acceptance's `PTT ON` held back the `LISTEN OFF` of a Trimode that had heard nothing
  of the call. Since ADR-0096 §2.1, `PENDING` goes as the call is taken. A Trimode that heard it
  holds its scan and sends no `LISTEN` until `CONNECTED` or `CANCELPENDING` (ADR-0100), so a
  `PTT ON` has nothing to hold back. The rule outlived its reason for every call the program
  heard the `PENDING` of.
* **When the `CONNECTED` is not held** (the field gateway, early in a listening window), it went
  at once, before the acceptance keyed. The `PTT ON` came after it.

## 2. Decision

1. **A called session whose `PENDING` the program heard: its `CONNECTED` waits for the
   acceptance's `PTT ON`.** The program hears `PENDING`, `PTT ON`, `CONNECTED`, `PTT OFF`. Every
   program already hears this order for a call it places, where its call keys first. The
   `CONNECTED` goes at the first pass after the `PTT ON` is told (50 ms at most). The holds of
   ADR-0051 and ADR-0100 still apply on top, and whichever ends later decides.
2. **An acceptance that never keys does not hold the program for ever.** The rules or the keying
   gate may refuse it. The `CONNECTED` then goes `ACCEPTANCE_KEY_WAIT` (4 s) after the call was
   taken, with a warning in the log: *CONNECTED for the call from X said without PTT ON: its
   acceptance did not key within 4 s*. 4 s covers the slot the call grants: the call's end, the
   turnaround (at most 1.5 s), the answer gap (at most 2 s), and the keying.
3. **What a program that keys nothing hears.** ADR-0096 §2.3's "a call answered while its
   `CONNECTED` waits for the program to listen" now means a call whose `PENDING` it never heard:
   its `LISTEN` was off when the station took the call. That call still gets no `PTT`, and its
   `CONNECTED` waits for `LISTEN` as before. A call it heard the `PENDING` of is its own business
   from the `PENDING` on. If such a call comes to nothing (`CANCELPENDING`), the closing
   identifier is not told: the program is back to scanning, and a `PTT ON` would hold its next
   `LISTEN`. A program that keys the radio hears every keying, as before. Only the order changes
   for it: `CONNECTED` after the acceptance's `PTT ON`.
4. **The daemon's log says when a program opens or closes a data connection**: *data port:
   connection n opened by the program*, *closed by the program*, *let go for a newer one* or
   *failed*. The `host data:` trace lines say the same in order. The bench showed that Trimode
   decides how the session ends as it writes `FQ`, before any `BUFFER` the adapter says for it.
   The published interface gives it no other way to ask the modem to hang up once `FQ` is sent.
   One untested reading is that it closes the data port. The next gateway session's log will
   say whether it does. Nothing acts on it.

### Not taken

* **Holding `CONNECTED` until the acceptance's `PTT OFF`**, so that the whole keying is told
  first. That would answer a watchdog timed from the release too. It costs the program up to
  5.6 s, a floor acceptance's length. Trimode's lines do not say it times from the release.
  If the message comes back with this fix in place, this is the next step.
* **A `PTT` for keying that did not happen**, to keep the watchdog fed. That would be a lie in
  the published interface, and a program that keys the radio would transmit for nothing.
* **Telling a program the keying of the panel's sessions.** Those sessions are not the program's
  business (ADR-0096 §2.3), and the 2026-10-25 failure stands.

## 3. Consequences

* A called program hears `CONNECTED` 0.3–3.5 s after the call was taken, where it had heard it at
  once when it was listening. Its first bytes go in the same slot as before: the master's first
  header comes after the acceptance ends.
* VarAC answering a ping or a connect, and Winlink Express listening for P2P, now hear `PTT ON`
  before `CONNECTED` on the called side. Each already hears that order for its own calls. The
  bench check of both is owed.
* Tests (`host/server.rs`): `the_acceptances_keying_is_told_before_connected`,
  `a_connected_whose_acceptance_never_keys_goes_after_the_wait`,
  `a_program_with_listen_off_hears_no_keying_for_the_call`,
  `the_log_says_when_a_program_opens_and_closes_the_data_port`.
  `pending_goes_at_once_and_a_held_connected_follows_at_the_next_window` now expects the
  acceptance's `PTT ON` during the hold and its `PTT OFF` after. The tests that publish a called
  `connected` now key its acceptance, as the station does.
* The harness: every host stand-in (`tools/session_matrix.py`, `Host`) counts it a fault to be
  told `CONNECTED` for a call it heard the `PENDING` of with no `PTT ON` between. In the six
  host-program scenarios, beta.140's daemon has the fault at every answered call and this build
  at none (§4).
* Owed: the bench's test C again (connect, wait 15 min, connect) on the scratch Trimode, and a
  gateway session with `[host] trace = true` for §2.4.

## 4. Measured

The gateway and host-program scenarios, one draw each (the scenario's own seed), on this build
and on beta.140's daemon, with the same harness and stand-ins:

| Scenario | beta.140: answered calls told `CONNECTED` with no `PTT ON` since the `PENDING` | beta.140 air | this build: faults | this build air |
|---|---|---|---|---|
| `40m-trimode-gateway-2300` | 1 of 1 | 79.2 s | none | 79.1 s |
| `80m-gateway-fq-hangup-2300` | 1 of 1 | 103.1 s | none | 107.2 s |
| `40m-gateway-busy-when-called-2300` | 2 of 2 | 148.1 s | none | 148.1 s |
| `40m-gateway-three-clients-2300` | 3 of 3 | 309.2 s | none | 244.2 s |
| `80m-trimode-gateway-client-vanishes-500` | 2 of 2 | 1012.2 s | none | 951.2 s |
| `80m-varac-chat-and-ping-500` | 2 of 2 (the ping and the connect) | 113.1 s | none | 113.1 s |
| `40m-gateway-client-vanishes-2300` (no host program: a control) | — | 344.2 s | none | 344.1 s |

Every run passes on this build, none with a radiated overlap. In every run the called program
hears `PENDING`, `PTT ON`, `CONNECTED`. The air times differ only where the run took another
course after the change had done its work. The FQ hang-up runs are line for line the same to
55 s: the same `CONNECTED` second and the welcome in the same transmission. They part at the
client's reply (57–58 s against 61 s), as two runs of one build do. The scenario's
`80m-trimode-gateway-client-vanishes-500` seed runs past the baseline's range (737–771 s) on both
builds. Its second call goes unanswered for over a minute on that NVIS path, and that is the
draw, not the adapter.
