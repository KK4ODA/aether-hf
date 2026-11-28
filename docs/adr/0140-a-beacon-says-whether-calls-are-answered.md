# ADR-0140: A beacon says whether calls to its callsign are answered now

**Status:** accepted, 2026-11-28 (RadioPost's design 07, the author's decisions 7 and 10). The
beacon body's capability byte: **two bits that were zero, no version change**; by the author's
reading not an air-protocol change for P5-7's four weeks.

## 1. Context

A station heard beaconing may or may not take a call. RadioPost's client beacons while it listens
for P2P, and its operator may untick "only while listening" and beacon while it does not. VarAC
beacons as `KK4ODA-9`, a name nobody answers. A listener choosing whom to call could not tell
these apart: a beacon said who and how strong, never whether a call would be taken.

## 2. Decision

The beacon's capability byte (air-interface §7.4, offset 7) gains two bits:

- bit 6 (`0x40`): the beacon says whether calls to its callsign are answered now;
- bit 5 (`0x20`), with bit 6: they are.

A current Aether sets bit 6 on every beacon and bit 5 when `answers_call_for` passes for the
beacon's callsign at the moment it is rendered: the frame is built again as it goes, since a
program may start or stop listening while the beacon waits. RadioPost's beacon while it listens
says yes, one while it does not says no; the panel's says what the station does for its own
callsign (no while a program speaks for the station, ADR-0108); VarAC's `-9` says no.

A receiver reads both bits clear as "not said" (a beacon of before this ADR). The `frame` event,
the heard list (`heard.json`'s `answering`, serde default), `heard.list` and the `heard` event
carry `answering`.

## 3. Alternatives considered

- **A new beacon kind for "listening".** The DATA kind's three bits are all in use (air-interface
  §7.2), and a kind would split one fact across two frames.
- **The answer in the beacon's name** (a suffix, as VarAC's `-9` means "not to be called"). A
  name is the program's; a bit is the modem's, and true of every program.

## 4. Consequences

An earlier Aether reads the bandwidth bits (1–2) and ignores bits 5 and 6, so beta.182 and this
build hear each other's beacons as before. RadioPost's adapter reports `answers` from
`answering`. Tests: the aether-link beacon body tests (bits round trip, worked bytes), and the
station test of ADR-0139.
