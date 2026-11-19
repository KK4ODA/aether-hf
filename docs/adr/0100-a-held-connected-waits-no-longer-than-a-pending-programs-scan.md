# ADR-0100: A held CONNECTED waits no longer than a pending program's scan

**Status:** accepted, 2026-11-06, a fix to ADR-0096 §2.1 and ADR-0051; amended 2026-11-19 (§4). Host adapter only
(`core/aetherd/src/host/server.rs`) and the harness's Trimode (`tools/session_matrix.py`): **no
wire change, no model change, no configuration key.**

## 1. Context

On 2026-11-06 the author called his gateway (KK4ODA-1: RMS Trimode 1.4.4.2 on Aether
0.2.0-beta.136, 40 m, radio *Manual*, Aether keying by CAT) twice from the mobile (KK4ODA:
Winlink Express 1.8.6.0). Both stations uploaded their recordings and logs, and the author added
Trimode's main log and its VARA command log. The first session completed its B2F exchange. The
second came up on the air and carried nothing for two minutes, until the author aborted it at the
mobile.

The gateway, its daemon's log and Trimode's command log, all times UTC:

| Time | What happened |
|---|---|
| 22:40:53–22:41:36 | Trimode cycles `LISTEN` as it always does, on one channel and with scanning off: 23 turns in this minute, about one every 1.8 s |
| 22:41:36.022 | The station decodes the call (6.6 dB) and the session is up. The call came in the last quarter of Trimode's listening window, so the adapter says `PENDING` at once and holds the `CONNECTED` for the next `LISTEN ON` (ADR-0096 §2.1, ADR-0051): *CONNECTED … held: the program is at the end of a listening window* |
| 22:41:36 | Trimode: `In: PENDING`, `Connect pending received -- Holding scanner for 120 seconds`, then `Request LISTEN=False while Connected=False and Pending=True` and `Request LISTEN=True while … Pending=True` |
| 22:41:36–22:43:31 | The adapter's last `LISTEN` from Trimode was `OFF`, sent as the `PENDING` arrived; no other `LISTEN` comes. Trimode's requests while a call is pending are not sent. No `CONNECTED` is said |
| 22:43:25 | The author aborts at the mobile; Winlink Express had been waiting for the gateway's welcome |
| 22:43:31 | The session ends. The adapter says `CANCELPENDING`; Trimode: `Pending Vara connection cancelled. Restarting scanner`, and its `LISTEN ON` follows 1.5 s later |

ADR-0096 had `PENDING` go at once so that Trimode would hold its scan for the call, and kept
ADR-0051's hold of the `CONNECTED` for a call taken at the end of a listening window. The two
together can wait for each other: a program holding its scan for a pending call sends no `LISTEN`,
and a `CONNECTED` held for a `LISTEN ON` is never said. Trimode's log shows what its scan does
instead: it finishes the deaf step it is in and goes back to listening on its own, saying nothing.

The harness's Trimode stand-in did not hold its scan on `PENDING`, so it went on saying `LISTEN ON`
and released every held `CONNECTED`: the stand-in could not find this.

## 2. Decision

1. **A `CONNECTED` held behind a `PENDING` the program heard waits `PENDING_SETTLE` (1.5 s) from
   that `PENDING`, or for the program's next `LISTEN ON` if that comes first.** Trimode's deaf step
   is half a second, so 1.5 s is past the end of any deaf step the `PENDING` landed in.
   `PENDING` is said again just before it, as ADR-0096 has it.
2. **Unchanged:** `PENDING` goes at once when the station takes a call while the program listens
   (ADR-0096 §2.1); a call taken mid-window hears `PENDING` and `CONNECTED` together, as the first
   session did; a `CONNECTED` for a call taken while the program's `LISTEN` was off (no `PENDING`
   said) waits for the next `LISTEN ON` (ADR-0051); `CANCELPENDING` when the session ends first.
3. **The harness's Trimode holds its scan on `PENDING`** as tonight's log shows: it finishes the
   deaf step it is in without saying `LISTEN ON`, says no `LISTEN` until `CONNECTED` or
   `CANCELPENDING` (120 s at most), and takes a `CONNECTED` whenever it is not in a deaf step. A
   `PENDING` that lands in the last quarter of its window, by the wall clock the adapter judges
   windows by, still meets the `LISTEN OFF` its scan had decided: the crossing of that night. A
   `CONNECTED` in a deaf step is a fault, as before, and so is a `PENDING` left 20 s without
   `CONNECTED` or `CANCELPENDING`.

### Not taken

* **Holding `PENDING` with the `CONNECTED` until the next `LISTEN ON`**, which avoids the state
  altogether. It needs Trimode's scan to keep running while the station keys its acceptance, which
  ADR-0096 §2.3 gives only when Aether keys the radio. A gateway where Trimode keys the radio
  (`[ptt] kind = "host"`, the author's next setup) must hear the acceptance's `PTT ON`, and Trimode
  sends no command while it believes the transmitter keyed: that froze its scan on 2026-10-25
  (ADR-0096 §1). With `PENDING` first, Trimode holds its scan before it keys.

## 3. Consequences

* A call taken at the end of a listening window (about one in four at the gateway) is connected
  1.5 s after it is taken, or at Trimode's next `LISTEN ON`, instead of never.
* The premise, from one log: a Trimode holding its scan for a pending call takes a `CONNECTED`
  once its deaf step is over, whatever it last said about `LISTEN`. If it ever logs `Ignoring
  CONNECTED while LISTEN=FALSE` after a `PENDING`, this is the place to look again.
* Tests (`host/server.rs`):
  `a_program_holding_its_scan_for_pending_hears_connected_without_listening_again`, tonight's
  sequence: `PENDING`, the crossing `LISTEN OFF`, nothing more, and `CONNECTED` 1.5 s later.
  The three ADR-0096 tests are unchanged.
* Harness, `40m-trimode-gateway-2300`, `40m-gateway-three-clients-2300` and
  `80m-gateway-fq-hangup-2300`, four draws each, the same draws on both builds: with
  `PENDING_SETTLE` taken out, 4 of 12 pass and the other 8 fail on "PENDING … and no CONNECTED or
  CANCELPENDING in 20 s", tonight's failure; with it, 12 of 12 pass and every one of the 8 calls
  taken late in a window is connected by this rule. Before the stand-in crossed its `LISTEN OFF`
  with a late `PENDING`, both builds passed: a stand-in that held its scan before saying
  `LISTEN OFF` read to the adapter as one that had stopped scanning, which ADR-0051's beat
  already releases.
* Documents: `host-interfaces.md` (`PENDING`), `winlink-gateway.md` (Trimode's cycling, the
  troubleshooting row).

## 4. Amendment, 2026-11-19: the premise did not hold

§3 named the place to look again: a Trimode that logs `Ignoring CONNECTED while LISTEN=FALSE` after
a `PENDING`. The RMS Trimode bench (scratch Trimode 1.4.4.0, Aether keying the gateway, Pat as the
client) did, at 19:59:06–08 UTC:

| Time | What happened |
|---|---|
| 19:59:03.918 | Trimode: `LISTEN ON` |
| 19:59:06.963 | The call is taken 3.05 s into the window (its last quarter): `PENDING` at once, the `CONNECTED` held. Trimode: `Connect pending received -- Holding scanner for 120 seconds` |
| 19:59:07.236 | Trimode's `LISTEN OFF`, decided before the `PENDING`, reaches the adapter; then `Request LISTEN=False …` and `Request LISTEN=True while … Pending=True`, and no `LISTEN ON` is sent |
| 19:59:08.494 | `PENDING_SETTLE` after the `PENDING`: `PENDING`, `CONNECTED`. Trimode: `Ignore PENDING notification while LISTEN=OFF`, `Ignoring CONNECTED while LISTEN=FALSE` |
| 20:02:07.616 | Trimode's scan resumes with `LISTEN ON`; the session, up on the air all along, is never its own. The caller waited until its own timeout |

So a Trimode holding its scan for a pending call does not take a `CONNECTED` "whatever it last
said about `LISTEN`": its `LISTEN OFF` crossing the `PENDING` leaves it deaf until its hold ends
(three minutes here), and 2026-11-06's night was this, not a wait for nothing. The same night's log
shows the way out: told `CANCELPENDING`, Trimode logged *Pending Vara connection cancelled.
Restarting scanner* and said `LISTEN ON` 1.5 s later.

**Decision.** A `LISTEN OFF` after a `PENDING` the program heard, before its `CONNECTED`, draws
`CANCELPENDING` at once. The call is then held as one taken while the program had `LISTEN` off:
its `PENDING` once the program has listened again `LISTEN_ON_SETTLE` (0.3 s), then `CONNECTED`, with
the acceptance's keying told in between if it was not. §2.1 stands for a program that holds its
scan without that `LISTEN OFF`: the `CONNECTED` goes `PENDING_SETTLE` after the `PENDING`, as on the
bench at 16:13:44 the same day.

**Consequences.** A call whose `PENDING` meets a crossing `LISTEN OFF` is connected about two
seconds later instead of never. The harness's Trimode now holds with `LISTEN` off after a crossed
`LISTEN OFF`, ignores a `PENDING`, takes a `CONNECTED` then as a fault, and restarts its scan 1.5 s
after `CANCELPENDING`. Test:
`a_listen_off_crossing_the_pending_cancels_it_and_the_call_is_told_again` in place of
`a_program_holding_its_scan_for_pending_hears_connected_without_listening_again`. Not taken:
holding a call's `PENDING` until the next window when it is taken late in one. On a gateway that
scans several channels, Trimode would be listening on the next channel by then.
