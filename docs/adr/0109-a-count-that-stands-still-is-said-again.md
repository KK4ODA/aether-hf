# ADR-0109: A count that stands still is said again

**Status:** accepted, 2026-11-13, the author's go. Host adapter only
(`core/aetherd/src/host/server.rs`): **no wire change, no model change, no configuration key.**

## 1. Context

Pat's VARA driver (Pat-Vara 1.2.0, the version Pat's `go.mod` names on 2026-11-13) waits on
`BUFFER` lines in two places, `vara/conn.go`:

* `Write` waits while the modem holds seven times the bytes it is about to write;
* `Flush`, at the end of a transfer, waits for the count to reach 0.

Each wait subscribes to `BUFFER` and `DISCONNECTED` only, and restarts a one-minute timer at every
`BUFFER` line. A minute with none, and it returns "write: buffer timeout" or "flush: buffer
timeout", and Pat's session ends.

Aether said `BUFFER` only when the count changed (the `metrics` arm of `serve_commands`). The count
is the program's bytes the other station has not acknowledged, and it stands still while a burst
waits for its acknowledgement. The link holds through that for the longer of 45 s and four
exchanges, about two minutes on the tone floor (ADR-0033). So on a weak path Pat can drop a
session that Aether would have carried to its end.

Mercury found the same on the air: its issue 397, a 53 KB Winlink upload that died at 1463 bytes
after 93 s with no `BUFFER` line (K7EK, near 0 dB). Mercury 1.9.23 says the count again after 50 s
(`data_interfaces/tcp_interfaces.c`, `TNC_BUFFER_REPEAT_MS`). Aether's bench passed Pat at 15 dB
(`host-interfaces.md` §7), where nothing waits a minute. It has not been seen on Aether's air: no
Pat session has been recorded on a path that slow.

## 2. Decision

In the program's own session, while its count is above zero, a `BUFFER` line goes again whenever
none has gone for 50 s (`BUFFER_REPEAT`): a change, the answer to the program's own `BUFFER`, or a
repeat. 50 s is Mercury's, ten seconds inside Pat's minute.

* **Only the program's own session.** A session the panel started is not the program's, and it is
  told nothing of it (ADR-0096).
* **Only a count above zero.** A program that waits for nothing to be acknowledged waits for no
  line. `BUFFER 0` still comes once, when the count reaches it, so its timing for RMS Trimode,
  which decides as it writes `FQ` (ADR-0096), does not change.
* **Not a setting.** `HostConfig::buffer_repeat` carries it so that a test can shorten it; the
  configuration file has no key for it.

## 3. Consequences

* A program with bytes waiting hears `BUFFER` at least every 50 s. On a link that moves, the
  count changes more often than that, and nothing changes.
* VarAC, Winlink Express and RMS Trimode read the line as the count, which a repeat leaves the
  same. The bench check of each is owed.
* Tests (`host/server.rs`): `a_count_that_stands_still_is_said_again`, with the interval at
  300 ms: the count said again while it stands, said as it moves, and `BUFFER 0` once and not
  again. It fails without the repeat.
* The session harness's host stand-ins (`tools/session_matrix.py`, `Host`) model RMS Trimode and
  VarAC, not Pat. A `pat` stand-in that counts a minute without `BUFFER` as a fault would show the
  drop on a slow scenario; it is not built.
