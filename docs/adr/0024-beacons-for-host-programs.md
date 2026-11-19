# ADR-0024: A beacon carries the name a host program gave it and its sender's bandwidth; no beacon under automatic control

**Status:** accepted, 2026-09-26. The daemon (`Station::beacon`, the host adapter) and the
BEACON frame's body in `docs/spec/air-interface.md` §7.1. No link-protocol change: a receiver of
an earlier version reads the callsign and never looks further. **Amended 2026-11-19 (§4):** the
refusal under automatic control applies only under a rules profile, none of the rules settings
acts while the host program keys the radio, and a refused `CQFRAME` is answered `WRONG`.

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

## 4. Amendment (2026-11-19): the refusal is the rules', and what Setup shows is what acts

**The case.** VarAC 16.1.8 at KK4ODA-1, beta.166, 40 m at 500 Hz. The station ran the Aether
profile *VarAC default*. That profile had been made from the station's own settings, which a
RadioPost gateway had left under automatic control. VarAC keys the radio there, so the rules were
*None* and Setup showed Rules, Control, License and Sideband greyed as **Host**. VarAC's **Call
CQ** went twice (`CQFRAME KK4ODA 500`, 13:39:11Z and 13:39:21Z). The station refused both
beacons under §2's last point, nothing went on the air, and the host adapter had already said
`OK`, so VarAC showed both as sent. The reason was in the log file only. The *No rules* text said
Aether makes no regulatory checks.

**The author's decisions**, as licensee: "No rules lifts it", and "the rules are not held by the
gateway or a past application".

1. **The refusal is the rules'.** A beacon is refused under automatic control only while a rules
   profile is in force (`Station::beacons_barred`). This covers the panel's beacon, the repeating
   one (`beacon.every`) and a host program's `CQFRAME`, VarAC's CQ and its `-9` beacon alike. With
   the rules at *None*, they go whatever the control says, and the control operator answers for
   them, as for every other transmission under *None*. With the US profile and automatic control,
   §2 stands.
2. **While the host program keys the radio, none of the rules settings acts**: no profile and no
   control (`Config::regulatory_in_force`). Setup shows them as **Host**, and what Setup shows is
   what acts. The file keeps its values for when another radio control is chosen. Aether could not
   read the dial of such a radio anyway (ADR-0025), so a profile chosen by hand had little it
   could judge. ADR-0107 §5 says what stands in for automatic control's other effect there.
3. ***VarAC default* is never made under automatic control.** `varac::aether_changes` sets
   `regulatory.control = "local"` when the settings it starts from say `automatic`, whoever keys
   the radio. VarAC has an operator at the radio. `varac::TEMPLATE` is 2, so a station whose
   profile was made before is offered the update. The gateway's own profile keeps its Automatic.
4. **A refused CQ is told.** The host adapter answers `CQFRAME` once the modem has answered the
   beacon: `OK` if the modem took it, `WRONG` if not, as for `BW<n>` (ADR-0026). The modem's
   reason goes to the log as a warning `host` line. The daemon passes the adapter's warnings to
   the panel as `log` events (`name` `host`, `level` `warn`), and the panel shows them in its Log
   and, for ten minutes, in the programs chip's tooltip. The operator is looking at the program,
   which hears only `WRONG`. The reply waits without blocking the adapter's thread
   (`ControlHandle::ask`, `BeaconAsk`): no later command is read until it comes, so replies keep
   their order, and notifications, `PTT ON` above all, go on being sent (ADR-0106's reason for
   `tell`). The automatic-control refusal now says where the setting is: *Control is Automatic in
   Setup step 1; with someone at the radio, set it to Local (or Remote) in the profile in use.*

**Consequences.** Under the US profile nothing changes on the air. A station with no rules, or
keyed by its host program, can beacon under any control setting, and the operator is responsible
for that, as the *No rules* text says. The *No rules* text names the one check it keeps: an
identifier carries only the station's callsigns (ADR-0127 §2.5). A program that treats `WRONG`
after `CQFRAME` badly has not been seen; VarAC's reaction to it on the air is still to be read
from its *Log VARA commands* file.

**Tests:** `with_no_rules_a_station_set_to_automatic_beacons`,
`a_host_keyed_station_set_to_automatic_beacons_and_answers_as_its_program_says`,
`a_station_under_automatic_control_sends_no_beacon` and
`a_repeating_beacon_needs_a_control_operator` (US rules, unchanged),
`a_beacon_under_automatic_control_is_refused_by_the_rules_alone_and_says_what_to_change`,
`varac_default_made_from_a_gateways_settings_is_under_local_control`, and the adapter's
`a_cq_the_station_refuses_is_wrong_and_its_reason_is_logged`,
`a_cq_the_station_takes_is_ok_and_keys_as_before` and `a_ptt_on_is_not_held_behind_a_cqs_answer`.
The last fails if the reply blocks on the modem's answer.

**What it removes:** the refusal under *None* and under host keying, the `OK` said before the
modem answered, and a gateway's control carried into *VarAC default*.
