# ADR-0091: A beacon waits out the other station's closing identifier

**Status:** accepted, 2026-10-23, the author's go on report 15's change 3
(`docs/research/link-architecture/15-wc4y-2300-beta120.md` §3, §11). Completes ADR-0075 §2, which
holds the slave's own calls and probes until the master's closing identifier is over. The daemon,
and an accessor in the engine (model and port); **no wire change, no configuration key, no
schema bump**.

## 1. Context

ADR-0075 makes each station's Morse identifier the last thing it sends for a session. The slave's
goes behind its DISC_ACK; the master's goes alone after that slot, on the engine's timer, without
listening first. The slave's engine holds anything it starts on its own until the latest the
master's identifier can end (`_closing_hold`, `hold_until`): a call or a probe waits for it.

On 2026-10-23 KK4ODA-1, the slave of WC4Y's Test, had a beacon repeating every 15 minutes. One
fell due during the session. The station keyed it 10 ms after its own identifier ended: the
engine was idle, nothing was queued, and the busy detector said clear, because the station had
just released its key and was still deaf. WC4Y's identifier was due 0.8 s later and went out
entirely under the beacon (WC4Y's recording announces the beacon's tone frame at 347.04 s; WC4Y
keyed 347.53–350.93 s; the beacon ran to 352.40 s).

A beacon is queued by the daemon (`Station::beacon`), not by the engine, so it never asked the
engine's hold. Neither does a KISS client's datagram (`feed_datagram`).

## 2. Decision

1. **The engine says how long it holds** (`hold_until`, model and port): the earliest the
   station starts anything on its own. It is `_closing_hold` after a DISC_ACK this station
   answered, and earlier otherwise.
2. **A beacon and a datagram at the head of the queue wait for it** (`held_for_busy`), whatever
   the channel and whatever `wait_for_clear` says. The wait is not a busy hold: `deferred_for_busy`
   does not count it.
3. **The repeating beacon is not queued until then** (`advance_beacons`). Its clear-channel test
   runs once the other station's identifier is over, not in this station's deafness after its
   own release.

## 3. Measured

`a_beacon_due_at_a_sessions_end_waits_for_the_other_stations_identifier` (`station.rs`): two
stations with US callsigns, the called one with a repeating beacon set during the session; the
caller disconnects. Before: the beacon keyed over the caller's identifier (41 passes of the test's
loop with both stations keyed). After: both identifiers once, the beacon once, after them, and no
pass with both keyed.

## 4. Not done

A tune tone or a keying test the operator starts in those seconds is not held: the operator
pressed it. A beacon the operator presses by hand waits like the repeating one.
