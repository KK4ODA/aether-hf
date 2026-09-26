# ADR-0024: A beacon carries the name a host program gave it and its sender's bandwidth; no beacon under automatic control

**Status:** accepted, 2026-09-26. The daemon (`Station::beacon`, the host adapter) and the
BEACON frame's body in `docs/spec/air-interface.md` §7.1. No link-protocol change: a receiver of
an earlier version reads the callsign and never looks further.

## 1. Context

The author's first VarAC tests with Aether (2026-09-26, VarAC's own command log) show VarAC
sending its CQs and beacons with VARA's published `CQFRAME <source> <bandwidth>`:
`CQFRAME KK4ODA-9 500`, `CQFRAME KK4ODA-8 500`, `CQFRAME KK4ODA 500`. The suffix is VarAC's —
it tells the program at the other end what the frame is — and a VARA modem that hears one
tells its host `CQFRAME <source> <bandwidth>` as it was sent, which is how VarAC lists who is
on. Aether's adapter turned every `CQFRAME` into a beacon of the station's own callsign, so
the suffix never went on the air; and until the same day it told the receiving host nothing
but an `SN` line. The notification it gained then could only name the receiving station's
bandwidth, because a beacon did not say its sender's.

Separately, a host program's beacon timer on a station under automatic control is an
automatically controlled beacon, which §97.203(d) confines to a few segments — 28.20–28.30 MHz
the only one where Aether's data may go. The modem's own repeating beacon was refused under
automatic control (beta.67); a host program's, or a single one from the panel, was not.

## 2. Decision

* **A BEACON's body is a callsign, then a capability byte** (§7.3) whose bandwidth bits say
  the air its sender runs. The callsign is the station's own, or **the name a host program gave
  the beacon** when one of the station's callsigns is its base — the part before any `-`:
  `KK4ODA-9` for a station that is `KK4ODA`. A name with another base is refused
  (`bad_params`), and the host adapter then sends the plain beacon, which is what the published
  command asks for.
* **A receiver reports the name as sent and the bandwidth the byte says**; a beacon without the
  byte (an earlier version's) reads as the receiving station's bandwidth. The `frame` event of a
  beacon carries `bandwidth_hz`.
* **The host adapter passes `CQFRAME`'s source through**, and tells its host of every beacon
  heard as `CQFRAME <name> <bandwidth>`, after the `SN` line that gives its strength.
* **A station under automatic control sends no beacon at all** — the panel's, the repeating
  one, or one a host program asks for.

## 3. Consequences

* Two VarAC stations over Aether see each other's CQs and beacons as they would over VARA,
  suffix and bandwidth included.
* No protocol version change: the byte is added at the end, and both code bases read a
  callsign from the first seven bytes only (`unpack_callsign`), so a beta.68 station still
  hears a beta.69 beacon, and the reverse reads as "bandwidth not said".
* A beacon's body is eight bytes where tone-24 carries twenty-four: nothing about its air time
  changes.
* Refusing every beacon under automatic control is conservative on 10 m, where 28.20–28.30 MHz
  would allow one. An unattended Aether beacon is not something anyone has asked for, and the
  station's calls, answers and sessions are unaffected.
