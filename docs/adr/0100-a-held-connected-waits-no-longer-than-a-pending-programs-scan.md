# ADR-0100: A held CONNECTED waits no longer than a pending program's scan

**Status:** accepted, 2026-11-06, a fix to ADR-0096 §2.1 and ADR-0051. Host adapter only
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
