# ADR-0058: An unanswered wide call goes on at 500 Hz

**Status:** accepted, 2026-10-10. Model first (`LinkEngine.move_call`, `connect_tries`,
`connect_due`), then the port (`move_call`, `connect_tries`, `connect_due`) and the daemon
(`station/bandwidth.rs`: `advance_call_bandwidth`, `NARROW_AFTER_TRIES`, `Why::Unanswered`). No
wire change.

## 1. Context

On 2026-10-10 KO4WX, running 2 300 Hz, called this station (500 Hz) for minutes. A 500 Hz station
ignores a call stating a wider bandwidth (the frame decodes, but the session it asks for is one the
station does not run), and nothing told KO4WX why. ADR-0035 moves a 2 300 Hz caller to 500 Hz
before the call when it *knows* the other runs 500 Hz — from a probe's answer, a beacon, any frame
that states a bandwidth — but KO4WX had heard nothing of this station first.

The asymmetry is the way out: a 2 300 Hz station answers a 500 Hz call by moving to 500 Hz
(ADR-0026). A call at 500 Hz reaches both kinds of station; a call at 2 300 Hz only one.

## 2. Decision

1. **A 2 300 Hz call that `NARROW_AFTER_TRIES` (4) tries have not had answered goes on at 500 Hz**
   for its remaining tries, when the station called is not known to run 2 300 Hz and no host
   program chose the bandwidth. Four tries are two on the floor and two ordinary — the half of a
   call (`connect_retries` 8) in which a station that hears it answers.
2. **The move is made just before the next try** (`connect_due`, within 0.25 s, nothing arriving):
   an acceptance of the tries before has had its whole wait to arrive, and one at 2 300 Hz after
   the move would be read at 500 Hz.
3. **The engine carries the call on** (`move_call`): the next tries state 500 Hz, the try count and
   the timers stand. The daemon rebuilds its receiver, modems and filters as for any move, and goes
   back after the session or the failed call and `RETURN_QUIET_S` of quiet, as after ADR-0035's
   move (`status.bandwidth.why` = `calling`, the log line names the reason).

A 2 300 Hz station that simply did not hear the first four tries answers the narrow ones by moving
too, and the session runs at 500 Hz — slower than it could, on a path that was already losing calls.

## 3. Measured

The scenario harness, `40m-wide-calls-narrow-unprobed` (a 2 300 Hz caller, a 500 Hz station,
Moderate at +6 dB, a call, 2 kB and a disconnect, no probe first), four seeds:

| build | connected | message |
|---|---|---|
| before | 0/4 (no answer in 180 s of air) | — |
| **this** | **4/4** | 2 kB in 102–123 s on three seeds (the fourth delivered it in the first burst) |

Passing on this build as before: `40m-wide-calls-narrow` (probe first, ADR-0035), `quick-awgn-2300`,
`80m-weak-500`, and `80m-band-closes-2300` (which is built to lose its message as the band closes).
The link simulator's `test_a_call_moved_to_the_narrow_air_is_answered_by_a_narrow_station` (both
suites) and the daemon's `a_wide_call_nobody_answers_goes_on_at_500_hz_and_a_narrow_station_answers`
hold the rule.
