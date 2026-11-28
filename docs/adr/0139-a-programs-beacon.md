# ADR-0139: A program's beacon goes as its own callsign, within its window, on its dial

**Status:** accepted, 2026-11-28 (RadioPost's design 07, the author's decisions 7, 10 and 21).
The daemon and the control API: **no wire change, no configuration key, no schema bump.**

## 1. Context

RadioPost's client is to beacon for P2P (its design `docs/design/07-p2p-stations.md`, wishlist
W59): a station that listens says so every so often, and the stations that hear it list it in
**Call**. The client keeps the schedule. Aether sends each beacon.

Aether's `beacon` was the panel's Beacon button over the control API:

- It took a `callsign` only as a name with one of the station's callsigns as its base (VarAC's
  `KK4ODA-9`, ADR-0024). A program holding `WD5EMA` could not beacon as it: ADR-0106 makes a
  program call as its own callsigns, and nothing did the same for a beacon.
- It waited for a clear channel only when the operator's `wait_for_clear` said so, and then for
  as long as the channel stayed busy. A program that wants its beacon near a time, or not at all,
  could not say so.
- It went on whatever dial the radio was on. A gateway that scans (ADR-0129) moves the radio
  every few seconds, so a beacon asked for one of its dials could go on another.
- What became of it was a log line. A program had no way to learn whether it went, short of
  reading the log.

## 2. Decision

`beacon` takes four more members, and a `beacon-sent` event says what became of every beacon.

### 2.1 The callsign

A connection that holds a claim beacons as the claim's first callsign unless it names one. A
named callsign another program holds is refused `claimed`; one that is neither the program's nor
a name with one of its callsigns as its base (`WD5EMA-9`) is refused `bad_params`, as a host
program's name not the station's was (a host program then falls back to the plain beacon, as
ADR-0024 has it). A connection that holds none (the panel, a host program before `MYCALL`)
beacons as before: a name of this station's, or the station's own callsign.

### 2.2 `ref`, `within_s` and `dial_hz`

- `ref` is the program's own, carried back by `beacon-sent`.
- `within_s` (1–600) is how long the beacon may wait. With it, the beacon waits for a clear
  channel whatever `wait_for_clear` says (the program asked for a beacon on a quiet channel),
  for a call, a probe or a Test to finish, past the engine's closing hold (ADR-0091), and for a
  scan to reach its dial. It is dropped after, and said. Without it, a beacon waits as the
  panel's does.
- `dial_hz` names the dial it goes on: the radio's, within 50 Hz, or one of a running scan's.
  On a scan's dial the beacon waits for the scan to get there, takes the scan there next, and
  holds it there while it waits, as a datagram does (ADR-0129 §15). Any other dial is refused
  (`BEACON_ELSEWHERE`), and so is a dial when the radio's is not known.

A beacon with a window or a dial elsewhere waits off the transmitter's queue
(`Beacons.queued`) and is handed over by `feed_beacon` when it may go. One without goes on the
queue at once, as before. At most one beacon waits; a second is refused `not_idle`, and one still
waiting when a session starts is dropped (ADR-0038).

### 2.3 `beacon-sent`

Every beacon ends in one `beacon-sent` event: `ref` (or null), `callsign`, `sent`, `dial_hz` (the
radio's dial it went on; for one that did not go, the dial it was asked for) and `reason`. A
beacon is sent when it is rendered; it is not sent when its window ran out, a session started, the
rules refused it at keying (ADR-0018), or anything else took it off the queue. A beacon that did
not go is said in the log with its reason. `status.beacon.queued` says what waits.

`capabilities.beacon` is `{"program": true, "answers": true, "position": true}`, so RadioPost's
adapter can tell this Aether from an earlier one, which ignores the new members.

## 3. Alternatives considered

- **Aether's own repeating beacon (`beacon.every`) for programs.** It is the station's, one at a
  time, with its own interval rules; two programs would fight over it, and RadioPost's interval
  counts from its own callsign's last time on the air (its design §6), which only the program
  knows. Each program asks for each beacon instead.
- **A window counted from the request with no channel rule.** A program that asks for a window
  is asking for a beacon on a quiet channel; holding it to the operator's `wait_for_clear` would
  key over a QSO on a station whose operator turned that off.
- **Accepting any dial and tuning there.** A beacon that moves the radio is a tune, which the
  operator and a scan own (ADR-0129); the beacon goes where the radio is or will be.

## 4. Consequences

- RadioPost's adapter maps RP-4 1.7.0's `beacon` onto this (its `adapters/aether`), and the
  client's beacons follow its scan.
- A panel beacon and a host program's `CQFRAME` behave as before, and gain the `beacon-sent`
  event and the answering bit (ADR-0140) and position (ADR-0142).
- Tests: `a_programs_beacon_says_its_callsign_whether_it_is_answered_and_where_it_is`,
  `a_programs_beacon_goes_only_as_a_callsign_it_holds`,
  `a_beacon_with_a_window_is_dropped_when_it_cannot_go_in_it` (station).
