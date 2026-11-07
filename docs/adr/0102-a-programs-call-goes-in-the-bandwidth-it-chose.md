# ADR-0102: A program's call goes in the bandwidth it chose

**Status:** accepted, 2026-11-07, the author's decision. Amends ADR-0035 (the move before a call)
and ADR-0058 (the move after four unanswered tries) for calls a host program places. Daemon and
host adapter only (`station.rs` `connect_keeping`, `station/bandwidth.rs`, `host/server.rs`,
`host/vara.rs`, the control API's `connect`): **no wire change, no model change, no
configuration key.**

## 1. Context

On 2026-11-06 the author's mobile called his gateway twice from Winlink Express, whose session
setting asks the modem for 2 300 Hz (`BW2750`, which Aether runs as 2 300). Both stations were set
to 2 300 Hz. Both calls went at 500 Hz: the mobile remembered the gateway as a 500 Hz station
(ADR-0101 says how that memory went wrong) and ADR-0035 moves a 2 300 Hz call to a station
remembered at 500 Hz before it goes. The log said *calling KK4ODA-1, which runs it*; Winlink
Express had asked for something else.

The author has since chosen to let the host programs own the radio at both ends. The program's
bandwidth setting is then where the operator decides the bandwidth, and the modem should not
override it on a memory.

ADR-0058's fallback — a 2 300 Hz call that four tries have not had answered goes on at 500 Hz —
excluded a bandwidth a host program chose. Winlink Express always says a `BW`, so its calls never
fell back.

## 2. Decision

1. **A call a host program places, after it has said `BW<n>`, goes in the bandwidth the station
   runs**, not the one the station remembers for the station called. The adapter marks the call
   (`keep_bandwidth` on the control API's `connect`, set once the program has said any `BW`); the
   station then makes no ADR-0035 move, and when the memory differs a `log` event named
   `bandwidth` says so: *calling KK4ODA-1 at 2300 Hz, as the host program asked, though it was
   last heard running 500 Hz; at 500 Hz after 4 unanswered tries*.
2. **ADR-0058 applies to every call**, a program's included. The program's bandwidth is the first
   one tried, not the only one: a 2 300 Hz call that four tries have not had answered goes on at
   500 Hz, where a 500 Hz station can answer and a 2 300 Hz one answers by moving. `CONNECTED`
   says the bandwidth the session runs, as it did for every move before (ADR-0026).
3. **Unchanged:** a call from the panel, or from a program that has said no `BW`, still moves to a
   station remembered at 500 Hz (ADR-0035); the station still answers a narrower call by moving
   (ADR-0026); the move back after a session or a failed call goes to the program's bandwidth while
   it is attached.

### Not taken

* **No fallback for a program's call**, as with a modem that runs only the bandwidth it was set
  to: a 2 300 Hz call to a 500 Hz station would never be answered, and the operator would have to
  know to choose 500 Hz in the program. The author chose to keep the fallback.
* **A "moved for this call" capability bit** (ADR-0101 §3) that would let a station remember the
  right bandwidth of a caller that moved for the call. The author's decision: wait, and build it
  only if wrong memories show up again on the Stations tab's *Hz* column.

## 3. Consequences

* A program's call to a station remembered wrongly at 500 Hz goes at 2 300 Hz and is answered
  there. A program's call to a station that really runs 500 Hz costs four unanswered tries (about
  a minute) before it goes on at 500 Hz, where before it moved at once.
* Tests: `a_host_programs_call_goes_in_the_bandwidth_it_chose` (the program's call stays at
  2 300 Hz and is answered there; the panel's call with the same memory moves),
  `a_host_programs_wide_call_nobody_answers_still_goes_on_at_500_hz` (fails with the old
  exclusion), and in the adapter `a_host_can_set_up_and_call` (a program that said `BW2300` marks
  its call) and `a_program_that_said_no_bw_leaves_the_bandwidth_of_its_call_to_the_station`.
* Documents: the control API's `connect`, the host interface's `CONNECT`, the Winlink Express
  section of `host-programs.md`, and the manual's entry on two 2300 Hz stations meeting at 500 Hz.
