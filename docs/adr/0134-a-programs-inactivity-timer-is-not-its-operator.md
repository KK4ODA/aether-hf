# ADR-0134: A program's inactivity timer is not its operator

**Status:** accepted, 2026-11-24 (the author's "go ahead with all fixes", after report 21 §9's
fix 1). The host adapter only: **no wire change, no configuration key, no schema bump.** Whether
Winlink Express goes on waiting once its `DISCONNECT` is held is the air's question (§5).

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
