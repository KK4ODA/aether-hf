# ADR-0108: Every program speaks for the station, and Setup says whose callsign it is

**Status:** accepted and built, 2026-11-12. These are the author's answers after an audit of how
callsigns move between Aether and RadioPost ("I just want whatever simplifies things the most").
The ADR amends ADR-0106 §2.5. It touches:

* the daemon (`station/claims.rs`, `station/bandwidth.rs`, the beacon, the KISS datagrams, the
  Test's and the recordings' names, `status.callsign`);
* the panel (Setup step 1's callsign box, the header, the apps chip, the mismatch banner);
* the manual;
* RadioPost's guides, in that repository.

**No wire change, no model change, no configuration key.**

## 1. Context

A station's callsigns come from three places:

* **Aether's Setup.** This is the station's own callsign. The panel's calls, probes and Tests
  go out as it.
* **A host program's `MYCALL`.** Trimode, Winlink Express and VarAC use the VARA-compatible port.
* **A RadioPost program's claim** on the control API (ADR-0106):
  * RadioPost Gateway claims the callsign in its own Setup. That callsign is registered with the
    post office and listed for callers.
  * The RadioPost app claims the callsign in its credential, which is the licence's.

A call goes to whoever holds the callsign called. The audit found that this was hard to follow:

1. **Two rules for one idea.** A host program spoke for the station: while it was attached,
   Aether's own callsigns were not answered, and Setup's box read *Host*. A RadioPost program did
   not. Aether went on answering its own callsign beside the program's: fully under local
   control, and for probes under automatic control. Setup's box stayed editable, as if nothing
   else were involved. So a box saying *Set by the gateway* would not have been true.
2. **The callsign typed twice.** RadioPost's guides told the operator to copy the gateway's
   callsign, or their own, into Aether's Setup. Since claims (ADR-0106), the claim alone answers
   calls, places them and sets the Morse ID. The copy was only a second place to get it wrong,
   and changing it restarted Aether.
3. **A leftover callsign.** Some places used the engine's `my_call`. It keeps the callsign of the
   last call or probe answered, until the next call or a change of callsigns:
   * the header (`status.callsign`);
   * the panel's Beacon;
   * KISS datagrams;
   * a Test's recording name and a recording started by hand;
   * the shared and debug zips' names.

   Connect, Probe and Test used Setup's callsign. After a session the gateway answered as
   KK4ODA-1, the header read KK4ODA-1 and the Beacon went as KK4ODA-1, while Connect went as
   KK4ODA.
4. **A program gone, silently.** Say Setup is KK4ODA and the gateway is KK4ODA-1. With the
   gateway down, a call to KK4ODA-1 drew ADR-0057's banner: *Answer as KK4ODA-1 too*. Under local
   control, pressing it made Aether answer the gateway's callers itself, ADR-0107's case again.
   The apps chip meanwhile named KK4ODA, not the callsign callers were calling.

## 2. Decision

### 2.1 One rule for every program

While any program holds a claim, it speaks for the station, on either port (the author's choice):

* The station's own callsigns that no program holds are not answered.
* They come back when the last program goes.

This is ADR-0106 §2.5's rule for the host interface, now applied to every program.
`Station::answers_for` asks whether any claim stands, not whether one is the host interface's.

The reason a call to the station's own callsign goes unanswered names the program. For a host
program it is *the host program attached answers only for the callsigns its MYCALL names*, as
before. For a program on the control API it is *RadioPost Gateway answers for this station while
it runs, as KK4ODA-1*.

What it costs: say Setup is KK4ODA and the gateway is KK4ODA-1, under local control. A call to
KK4ODA, such as another station's Test, is no longer answered while the gateway runs. Under
automatic control a call to KK4ODA was already unanswered (ADR-0107); now a probe to it is too.
When Setup and the gateway use the same callsign, nothing changes.

### 2.2 Setup's callsign box says whose callsign it is

The box in Setup step 1 is greyed and replaced by text:

* **Host**, with Host programs ticked, as before.
* **The callsign and the program** (the author's choice), while a program on the control API
  holds callsigns. It reads *KK4ODA-1 · RadioPost Gateway* at a gateway, and *KK4ODA ·
  RadioPost* on a mobile while the app holds its callsign.
  * The tooltip says the program answers for the station as that callsign, and that calls to the
    station's own are not answered meanwhile.
  * It also says the station's own callsign comes back when the program stops.

The box follows the program, whenever it holds a callsign (the author's choice). At a gateway
that is all the time. On a mobile it is only during a RadioPost call or while the app listens,
because the app connects to Aether only then. The box changes as soon as the daemon says a claim
changed (its `claims` log line), not at the next poll.

The saved callsign stays underneath and is saved as it was. A placeholder callsign (`N0CALL`)
keeps the box editable, so a first setup can be finished with a program already running.

### 2.3 No leftover callsign

* **The header.** `status.callsign` is the callsign in use
  (`Station::callsign_in_use`):
  * the session's or call's while one is up;
  * otherwise the first callsign a program holds;
  * otherwise the station's own.
* **What the station starts by itself goes as its own callsign** (`Station::station_callsign`,
  Setup's), as Connect, Probe and Test already did:
  * the panel's Beacon;
  * KISS datagrams;
  * a Test's recording name;
  * a recording started while idle;
  * the shared and debug zips.

  A session's recording still takes the session's callsign. `state` events still carry the
  session's callsign.

### 2.4 A program gone is said, not offered

Aether remembers which program held each callsign since it started. When a call or probe arrives
for a callsign nobody holds now, the station not holding it either, it raises a new kind of
mismatch, `what` = `program`. `called` is the callsign called. The sentence names the program:
*W4ODA called KK4ODA-1, which RadioPost Gateway answered for: RadioPost Gateway is not running
now, so nothing answers KK4ODA-1. Start RadioPost Gateway again.* The banner offers no *Answer as
it too* and no Setup button. ADR-0057's offer stays for a callsign no program has held.

The apps chip no longer says *not answering \<own callsign\>* while a program is there. The
program speaking for the station is no news, and Setup says whose callsign it is. The chip still
says it when no program is there (ADR-0107).

## 3. Consequences

* **RadioPost's guides** no longer ask for Aether's callsign to be set to the gateway's, or to
  the credential's. The program's own Setup, or its enrollment, is the one place for its
  callsign. Aether's Setup is the station's own callsign, used for what the operator starts from
  Aether's window.
* **The panel** and **the manual** say so (step 1, section 12). The manual names the cost of §2.1.
* **The harness** runs no control-API claimant, so its scenarios are unchanged.
* **Tests:**
  * `a_program_on_the_control_api_speaks_for_the_station_too` (station): the station's own
    callsign rests while the gateway holds KK4XYZ-10, with the gateway's reason. The gateway's
    callsign is answered and its session is its own. The Beacon after that session goes as
    KK4XYZ. The station's own comes back when the gateway goes. A later call to KK4XYZ-10 is a
    `program` mismatch naming it. It fails without §2.1.
  * `a_program_claims_its_callsigns_and_acts_only_on_its_own_sessions` (control API): now
    expects `answering` false with the program's reason, and `status.callsign` as the program's,
    then Trimode's once the gateway goes.

## 4. What it removes

* ADR-0106 §2.5's "Programs on the control API do not silence the station's own callsigns": the
  difference between the two ports.
* The leftover callsign in the header, the Beacon, KISS datagrams and the files' names.
* The instruction, in RadioPost's guides, to type a program's callsign into Aether as well.
* ADR-0057's *Answer as it too* for a callsign a program held.

## Not taken

* **Separate callsigns always** (RadioPost on KK4ODA-10 beside the station's KK4ODA-1). It keeps
  both answering, but leaves the operator to keep two callsigns straight, and ADR-0107 already
  declined it.
* **Aether taking its callsign from RadioPost**, or RadioPost writing Aether's Setup. A callsign
  change restarts Aether, and the gateway's callsign belongs to its registration.
* **"Set by gateway" as the box's words.** The app on a mobile is a program too, and the callsign
  itself is the value the operator wants to see.
* **The box only while a program listens.** The rule of §2.1 follows a claim, as a host program's
  follows its attachment, not its `LISTEN`. Trimode turns `LISTEN` off every 3.6 s, and the box
  and the rule would have followed two conditions.
