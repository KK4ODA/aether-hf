# ADR-0096: A scanning gateway hears PENDING first, and a host that reconnects is served

**Status:** accepted, 2026-10-25 (the first Winlink sessions through an Aether gateway on the
air). Host adapter only (`core/aetherd/src/host/server.rs`, the `ptt` event in `main.rs`): no
wire change, no model change, no configuration key.

## 1. Context

On the night of 2026-10-24/25 the author ran a Winlink RMS gateway at home — RMS Trimode
1.4.4.0 on the VARA-compatible port of KK4ODA-1's daemon (beta.122, 80 m, 2300 Hz, Yaesu CAT
keying, Trimode's radio *Manual* with no PTT port) — and called it from the mobile, Winlink
Express 1.8.6.0 on KK4ODA's daemon (RTS keying). Both stations uploaded their recordings and
logs; the author added Trimode's main log and its VARA command log. All times UTC.

**The first session (03:30) failed, and took the gateway down for ten minutes.**

| Time | What happened |
|---|---|
| 03:30:09.979 | Trimode's last `LISTEN ON` before the call. It scans its one channel: `LISTEN OFF` every ~3.66 s, on again 0.5 s later |
| 03:30:12.862 | KK4ODA-1 decodes the call (−1.1 dB) and the session is up, 2.88 s into a ~3.15 s listening window: in its last quarter, so the adapter holds the whole block — `PENDING`, `REGISTERED`, `CONNECTED` — for the next window (ADR-0051, the `Scan` beat) |
| 03:30:13.058 | The station keys its acceptance, a 5.6 s floor frame, and tells Trimode `PTT ON` |
| ~03:30:13.1 | Trimode's scan goes deaf. Its `LISTEN OFF` never arrives: Trimode sends no command while it believes the transmitter keyed |
| 03:30:16 | No `LISTEN OFF` for twice the window: the beat reads "stopped scanning", and the block is said. Trimode: `Ignoring CONNECTED while in scan state Deaf Sending ABORT to Vara TNC` |
| 03:30:33–38 | `Trying to send command while PTT is on: LISTEN OFF`, then a five-second timeout. Its reader seems to have stalled behind the abort it could not send: it never read the `PTT OFF` |
| 03:31:08 | `Vara command wasn't acknowledged. Restart Vara`. Only now does it read the `ENCRYPTION DISABLED` and `LINK REGISTERED` sent 52 s before |
| 03:31:13 – 03:40:05 | Five restarts: it launches its "VARA program" (`aether-hf.exe`, which only brings the window forward), opens the command port again and is told `WRONG` — the adapter refused a second host while the old connection, which Trimode never closed, stayed open. `Error induced restarts: Vara=5` |
| 03:40:18–39 | The author closes and restarts Trimode. The leaked connection closes with its process, and the port opens |

Meanwhile the session ran on the air with nobody at the gateway's end: Winlink Express waited
for the welcome and aborted at 03:31:31.

**The second session (03:41:50) worked** — the B2F exchange completed — and its ending was
slow:

| Time | What happened |
|---|---|
| 03:41:56.29 | KK4ODA-1 decodes the call mid-window: `PENDING` … `CONNECTED` at once. Trimode: `Connect pending received -- Holding scanner for 120 seconds` |
| 03:42:03.7–15.0 | The welcome, 225 bytes, in one transmission; Winlink Express's 108 bytes back at 03:42:20 |
| 03:42:26.37 | Trimode writes `FQ`, 0.1 s after the station had decided its answer to that transmission; it goes in the next slot, 03:42:31.5–40.3 (a floor header and one rung-5 frame), and reaches Winlink Express about 03:42:40.4 |
| 03:42:26 | Trimode: `Starting the disconnect timer. Currently connected` — it waits for the client to hang up |
| 03:43:10.63 | The mobile's first `disconnect`, 30.2 s after `FQ` arrived, and a second at 03:43:18.85 (Winlink Express's or the panel's: the log does not say which) |
| 03:43:20.27 | The mobile's DISC, after the open window its last header had granted (ADR-0069) |
| 03:43:24.3–31.9 | KK4ODA-1's `DISC_ACK` and Morse identifier; Trimode hears `DISCONNECTED` at 03:43:31; the mobile's identifier 03:43:32.5–36.8 |

From `FQ` to the gateway's `DISCONNECTED`, 65 s: 14 s carrying three bytes from slave to
master, 30 s that Winlink Express waited, 10 s of open window, 11 s of the closing exchange and
identifiers.

Two faults of the adapter's made the first session's chain. `PENDING` — the line VARA says on
hearing a call, before it answers, and the one that stops Trimode's scan — waited with the
`CONNECTED`; and the `PTT ON` of an acceptance Trimode had not been told of froze its scan, so
the beat the adapter had learned from its `LISTEN` commands no longer described it. And one
fault made the outage: the adapter served the first program to connect, where a VARA that
Trimode restarted would have been serving the newest.

## 2. Decision

1. **`PENDING` goes at once.** When the station takes a call while the program's `LISTEN` is
   on, the called side hears `PENDING` before the station keys its acceptance — the state event
   comes before the keying, and the adapter handles them in order. The `CONNECTED` behind it is
   held as before (ADR-0051) when the program is deaf or at the end of a listening window, and
   when it is said, `PENDING` is said again with it: the first may have reached a program
   already deaf. A program that heard the first stops scanning, and the beat's "stopped
   scanning" reading then says what is true.
2. **`CANCELPENDING`** when a session that said `PENDING` ends before its `CONNECTED` was said:
   VARA's word for a pending call that came to nothing, so a Trimode holding its scanner for it
   goes back to scanning.
3. **No `PTT` for a session the program was not told of**, unless the program keys the radio
   (`[ptt] kind = "host"`, ADR-0025; the `ptt` event now says `host_keys`). That is a session
   the panel started (its closing identifier included), or a call answered while its
   `CONNECTED` waits for the program to listen. A program that keys nothing hears what a VARA
   would transmit for it — its own sessions, its calls, its beacons and tunes, the KISS frames —
   and a `PTT OFF` only after a `PTT ON` it heard. A station the program keys hears every
   keying, as before: it has to key them.
4. **The newest connection is the program's**, on either port. A connection while one is
   attached shuts the old one, waits up to 3 s for its thread to put the flags back (`LISTEN`,
   `CHAT`, the bandwidth go with it, as when a program leaves), and serves the new one; the
   payload pipe starts empty for a newcomer. The data port serves the newest connection on a
   thread of its own; it had served one at a time, so a second would have sat in the backlog
   unread while a leaked first stayed open. `WRONG` is left for a connection the old one would
   not let go of.
5. **The daemon's log says what happens on the port** (`host` lines): a program attaching,
   leaving or replaced (and how long the replaced one had been quiet), a `CONNECTED` held and
   said, a `CANCELPENDING`.

## 3. Consequences

* The first session's chain cannot start: Trimode hears `PENDING` while it listens and holds
  its scan; and if it was deaf, no `PTT ON` freezes its `LISTEN` turns, so the held block goes
  at its next real `LISTEN ON`.
* A Trimode that restarts its "VARA program" is served at its next connection; the window still
  comes forward each time it launches `aether-hf.exe`, which the log now explains.
* Programs set up for the same port take it from each other. That is a setup to fix; the log's
  `replaces` lines show it.
* Tests (`host/server.rs`): `pending_goes_at_once_and_a_held_connected_follows_at_the_next_window`,
  `a_call_in_the_middle_of_a_window_hears_pending_and_connected_together`,
  `a_call_that_ends_while_its_connected_is_held_is_cancelled`,
  `a_program_that_does_not_key_hears_no_ptt_for_a_session_it_was_not_told_of`,
  `a_program_that_connects_again_replaces_the_one_attached` (in place of
  `a_second_host_is_refused_rather_than_interleaved`); `a_session_the_panel_started_is_not_the_programs`
  states that its program keys the radio.

## 4. Not done, and open

* **The end of a B2F session.** After `FQ` each program waited for the other: Trimode started
  its disconnect timer, and Winlink Express sent its `DISCONNECT` 30 s after `FQ` reached it —
  or the operator did. The bench of 2026-10-07 ended a Trimode session within seconds of `FQ`.
  Which program hangs up after `FQ` under VARA, and what Winlink Express waited for, is for its
  session log of 03:42–03:44 and a Trimode log of a VARA session to say; nothing here changes
  until they have.
* **Three bytes took 14 s from the gateway to the client**: `FQ` missed the slot being decided
  by 0.1 s and went in the next one, a floor header and a rung-5 frame. The slot's answer is
  decided as soon as what it says is final (ADR-0069 render-ahead); waiting for late payload is
  a protocol question.
* **The master's DISC waits out an open window** it has granted (10 s on the floor). Cutting a
  window short is a protocol rule (ADR-0069 §2), the author's call.
* **Trimode's five-second command timeout during a long transmission of the program's own
  session.** The slave's transmissions here ran to 11.4 s, and the closing exchange with the
  identifier 7.5 s; a Trimode command that falls inside one (its disconnect timer, an abort)
  would time out as the first session's did. No such timeout was seen in the gateway's sessions
  so far.
