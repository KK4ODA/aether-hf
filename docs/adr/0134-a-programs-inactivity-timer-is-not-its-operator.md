# ADR-0134: A program's inactivity timer is not its operator

**Status:** accepted, 2026-11-24 (the author's "go ahead with all fixes", after report 21 §9's
fix 1). The host adapter only: **no wire change, no configuration key, no schema bump.** Whether
Winlink Express goes on waiting once its `DISCONNECT` is held is the air's question (§5).
**Amended 2026-11-25 (§6):** held only while the transfer can finish within 15 minutes, and said
in Aether's window.

## 1. Context

On 2026-11-24 at 18:55Z the portable's Winlink Express (1.8.6.0, P2P over the VARA HF port) sent a
50 kB incompressible file to home's (`docs/research/link-architecture/21-wle-p2p-40m-beta174.md`
§8.2). It wrote the whole file, 52 148 bytes, at 18:55:29.602. Aether carried it at rung 9, about
224 bit/s, which fit the path: the file needed about 31 minutes.

At 19:00:29.632, exactly 300 s after it wrote the file, Winlink Express sent `DISCONNECT`. Its log:

```
2026/11/24 18:55:29 1.8.6.0 Vara HF P2P *** Sending 07MVP34173Z0.
2026/11/24 19:00:29 1.8.6.0 Vara HF P2P *** P2P session inactivity timeout.  Disconnecting.
2026/11/24 19:01:47 1.8.6.0 *** Vara TNC reported disconnection
```

`BUFFER` had fallen every 9 s the whole time (52 148 → 43 665), and `SN`, `BITRATE`, `PTT` and
`IAMALIVE` kept coming, so none of them counts as activity to it. Home's Winlink Express, which was
handed data throughout, never timed out. Its settings have no such timeout, a web search found none
documented, and Mercury's pull request 581 (its issue 392) met the same timer: a modem cannot reset
it. Winlink Express waited 78 s after its `DISCONNECT` for the modem to report the end, sending
nothing more.

A message that takes more than five minutes to cross is common at the slow rungs: 10 kB at 250
bit/s takes 5.3 minutes. Each one is cut off at five minutes, whatever the path.

## 2. Decision

**A `DISCONNECT` that is the program's inactivity timer is answered and not acted on.** In the
program's own session, a `DISCONNECT` is taken for the timer when all of these hold:

* the program's own data is still going (`BUFFER` above 0);
* it has written nothing on the data port, and been handed nothing, for [`PROGRAM_IDLE`] (240 s)
  since the session was told to it;
* no other `DISCONNECT` was held in the last minute.

It gets its `OK` as every command does, the session goes on, and the log says so (a `warn` line:
*the program asked to disconnect 300 s after it last wrote or was handed data, with N bytes of
its own still going: taken for its inactivity timer and not acted on, so the transfer goes on. A
second DISCONNECT within a minute, or ABORT, ends the session*).

A second `DISCONNECT` within a minute ends the session as before, and so does `ABORT`: an operator
who means it presses again. A call being placed, a session the program did not start, and a
`DISCONNECT` with nothing left to send are never held.

240 s is under Winlink Express's 300 with a minute of room, and far over any real end: a program
ends a session seconds after its last write or the last data it was handed (the `FQ` it sent or
read; Mercury measured 4.1 s at most over 112 of them).

## 3. What it removes

Nothing: a rule added to the adapter's handling of `DISCONNECT` (`host/server.rs`,
`HostConfig::program_idle`).

## 4. Rejected

* **Setting the timer in Winlink Express** (the author's first choice, report 21 §9): its settings
  have none to set.
* **Sending something that resets it.** Nothing on the command port does, and data handed to the
  program would be data in its B2F session.
* **Holding every `DISCONNECT` while data is going.** An operator's Disconnect in the middle of a
  transfer would be lost.

## 5. Owed

A long P2P transfer on the air with Winlink Express at both ends. What it shows: that Winlink
Express keeps waiting after its `DISCONNECT` is held (it waited 78 s on 2026-11-24 with nothing
more sent), and that it takes the rest of its B2F session, the other side's `FF`/`FQ`, normally
once the file has crossed.

## 6. Amended 2026-11-25: only a transfer that can finish, and said in the window

The air answered §5 the same evening
(`docs/research/link-architecture/22-wle-p2p-80m-beta175.md` §1). The portable's Winlink Express
wrote a 161.6 kB file at 23:35:28Z over a 2300 Hz path on 80 m that carried about 67 bit/s. It sent
`DISCONNECT` at 23:40:27 and again at 23:45:27, 300 s apart: its timer fires again every five
minutes while it waits. Both were held and the transfer went on, as built. Winlink Express printed
*Disconnecting* in its own window each time, while Aether's window said nothing but a Log line.
The operator aborted at 23:48 with 6.4 kB delivered. At that rate the file needed about five
hours, so holding the `DISCONNECT` had kept going a transfer that could never finish.

The author's decisions (report 22 §9):

* **The hold has a limit.** A timer `DISCONNECT` is held only while what is queued could go within
  [`HOLD_FINISH_S`] (15 minutes) at the rate the session has delivered: the engine's
  `queue_finish_s`, its acknowledged bytes since the session came up over that time (ADR-0135's
  measure, the same as the drain's, now `session_rate` in both suites and `status.session_rate_bps`
  and `status.queue_finish_s`). Past it the `DISCONNECT` is acted on, the drain gives up at once
  (ADR-0135 §2.3) and the program hears `DISCONNECTED`; the Log says how long the rest would have
  taken. Before the rate is known (half a minute of acknowledgements) the `DISCONNECT` is held, as
  before. At 67 bit/s the portable would have closed at 23:40:27.
* **The window says so.** While a `DISCONNECT` is held, `status.host.disconnect_held_ms` says when,
  and Aether's window shows a banner on every tab: *The external program asked to disconnect (its
  inactivity timeout). The transfer goes on.*, with **Disconnect** (closes once what is queued has
  gone, or after the drain's minute) and **Abort**. The adapter clears it when the program's
  session ends or the program goes.

Checked by the host adapter's tests (timer `DISCONNECT`s 300 s apart each held; acted on when the
transfer could not finish; held when it would) and by two harness scenarios in real time, with a
Winlink Express stand-in whose timer fires as the real one's does (`host a wle`):
`80m-wle-long-message-500` (held, the message arrives whole) and `80m-wle-hopeless-file-500` (the
first timer `DISCONNECT` closes the session).
