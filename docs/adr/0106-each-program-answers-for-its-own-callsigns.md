# ADR-0106: Each program answers for its own callsigns and owns the sessions under them

**Status:** accepted and built, 2026-11-09 (the author's "build ADR-0106"); §2.5 gained a rule in
the building (§6). The daemon (`station/claims.rs`, `station/host.rs`), the control API (`claim`,
`listen`, `owner` on state events and in `status`, `status.claims`, `status.own_callsigns`,
errors `claimed` and `not_owner`), the control channel (`control/protocol.rs`: `Connection`,
`ConnectionId`), the host adapter (`MYCALL` and `LISTEN` are its program's claim, and a session is
the program's when its claim owns it), the panel's apps chip and session banner, and the manual.
**No wire change, no model change, no configuration key.**

## 1. Context

RadioPost ([KK4ODA/RadioPost](https://github.com/KK4ODA/RadioPost)) is a federated email system
with its own client and gateway. Aether is its first modem. RadioPost reaches a modem through its
modem interface, RPMI (RadioPost RP-4), and RP-4 §9.1 maps RPMI onto this control API. RP-4
0.2.0 §1.1 sets out the rules for a shared modem (each host's callsigns, answering and links are
its own; `caps.shared`). This ADR is what Aether needs to keep them. Two things are missing.

**Answering off.** RP-4 §4 says a host's `listen` is off until the host turns it on. Here,
`listen {enabled: false}` is refused `unsupported`: with no VARA host program attached, the station
answers every call to its callsigns (ADR-0026; `HostPresence::answering`). Only the host
interface's `LISTEN` stops it, and that switch covers the whole station.

**One station, two systems.** During the move from Winlink, a gateway operator will want RMS
Trimode on the host port (say KK4ODA-1) and the RadioPost gateway on the control API (say
KK4ODA-10). Both would share one radio, one sound card and one frequency. Today that cannot work:

* **One callsign list.** The station has one list. The VARA program's `MYCALL` replaces it
  (`callsigns.set`), and RPMI's `callsigns` would replace it too. Whichever program spoke last
  decides what the station answers to.
* **One answering switch.** Trimode's `LISTEN` decides for everyone. Trimode turns it off for half
  a second every 3.6 s (ADR-0051). Calls for RadioPost would go unanswered in those gaps, and all
  calls would while Trimode holds its scan for a pending call (ADR-0100).
* **The host adapter takes every called session as its program's.** The only exception is a call
  the panel placed (`report_state`: `!called && !calling`). A RadioPost client calling KK4ODA-10
  would reach Trimode as `PENDING` and `CONNECTED`, payload included, and Trimode would answer
  with a Winlink greeting.
* **Nothing says whose session it is.** Control-API events go to every connection. A RadioPost
  adapter could not tell its own session from Trimode's.

Two separate modem builds cannot share one radio. One daemon can, if it knows which program a call
is for. A call names a callsign, so the callsign is how the daemon knows.

## 2. Decision

### 2.1 A claim

A program that wants calls answered for it **claims** callsigns. A claim belongs to one connection
to the modem: a control-API WebSocket connection, or the host interface's command port. KISS
clients hold no sessions and claim nothing. A claim has:

* `callsigns`: the callsigns it answers to and calls as. The first is its default.
* `listening`: whether calls to them are answered. It is **off when the claim is made**, as
  VARA's `LISTEN` and RPMI's `listen` are.
* `app`: the program's name, for the panel and the log.

The station holds its own callsigns (`[station] callsign`, and any the panel adds with
`callsigns.set`), always listening, as today. The station's sessions are the operator's: a call
placed from the panel, a Test, a probe.

**One holder per callsign.**

* A program's claim takes a callsign from the station's own list, as `MYCALL` does today. A
  gateway's Trimode says the callsign the station already has, and from then on its `LISTEN`
  decides whether that callsign is answered.
* Between two programs, the first claim holds the callsign. A later claim of a callsign another
  program holds is refused for that callsign (`claimed`, naming the holder) and takes the rest.
* Callsigns are compared exactly. KK4ODA-1 and KK4ODA-10 are different callsigns, and so are KK4ODA
  and KK4ODA-1. The modem never infers ownership from an SSID convention.

**A claim ends with its connection.** When the program exits, crashes or is replaced (ADR-0096
§2.4, the newest host connection wins), its callsigns go back to the station if they are in the
station's own list. Callsigns that are not stop being answered. `set_host` does this today for the
VARA program alone; now it is the rule for every claim. A RadioPost gateway that has died stops
answering for KK4ODA-10, and a caller gets no session with nobody behind it.

### 2.2 Answering

A call or probe addressed to a callsign is answered only when that callsign's holder is listening
and the daemon is not stopping. `Station::unanswered` already keeps such frames from the engine;
it looks up the holder's switch in place of the one station-wide switch. A call to a callsign
nobody holds is not a call to this station, as today.

The engine answers to every holder's callsigns together (`set_callsigns`). A change made during a
session waits for the session's end, as today (`pending_callsigns`). The station still runs one
session at a time: while one program's session is up, calls for the other are not answered.

### 2.3 Who owns a session

* **A called session** belongs to the holder of the callsign it was called at.
* **A calling session** belongs to the connection that asked for the call, if that connection holds
  a claim. A claim calls only as its own callsigns: `connect`'s `callsign` must be one of them, and
  defaults to the first. A callsign another program holds is refused `claimed`. Any other call
  belongs to the station: the panel's, a Test's, a client's with no claim.

What owning means:

1. **Told.** Every `state` event of a session carries `owner`: the claim's `id`, or null for the
   station's own sessions. `status` carries it too. Events still go to every connection, so the
   panel sees everything; a program reads `owner` to find its own sessions. The host adapter tells
   its program `PENDING`, `CONNECTED`, the payload and `DISCONNECTED` only for sessions its claim
   owns. That replaces the `!called && !calling` test.
2. **Acted on.** From a connection that holds a claim, `send`, `disconnect` and `abort` act only on
   a session that claim owns, and are refused `not_owner` otherwise. A `send` with no session up
   is refused `not_connected`, as it always was. The panel holds no claim and may act on any
   session, sending on it included, as before: the operator is the control operator (§97.105).
3. **Busy to the others.** A program that does not own the session up hears the station busy: the
   host adapter's `BUSY ON` for the whole session, as it is now for a session the panel started.
   ADR-0096 §2.3's sessions "the program was not told of" now include other programs' sessions,
   so a program that keys nothing hears no `PTT` for them. A host-keyed station
   (`[ptt] kind = "host"`) still keys every transmission through the program attached, whoever
   owns the session, as it does for the panel's sessions today.
4. **The identifier.** A session still ends with the identifier of the callsign it ran under
   (ADR-0075). A VARA program's `CWID ON` (ADR-0075 §2.7) applies to the sessions its claim owns.

### 2.4 The control API

* **`claim {callsigns, app?}`** is new. It returns `{id, callsigns, refused, listening}`:
  `callsigns` is what was taken, `refused` lists `{callsign, holder}` for each callsign refused, and
  `listening` is false. Calling it again from the same connection replaces the claim's callsigns,
  as `MYCALL` does. `claim {callsigns: []}` releases them. An invalid callsign is `bad_params`.
  Over `POST /v1/` it is refused `unsupported`: a POST has no connection to hold the claim.
* **`listen {enabled}`** with a claim sets that claim's `listening`. Without a claim, `true`
  answers OK as now. `false` stays refused, and the message now says what to do: claim the
  callsigns the program answers for first; the station's own callsigns are always answered.
* **`connect {remote, callsign?}`** follows §2.3.
* **`status.claims`** lists each claim: `id`, `app`, `kind` (`control` or `host`), `callsigns`,
  `listening`, `since_ms`. `status.answering` keeps its meaning for the panel: whether calls to the
  station's first callsign are answered now.
* **`callsigns.set`** stays the station's own list, the one the panel edits. It may include a
  callsign a program holds. The holder answers for that callsign while it holds it, and the station
  answers for it again once the claim ends.
* **The connection's identity.** The control server tags every request with its connection: a
  number the server assigns, never one a client sends. It tells the station when a connection
  closes. The in-process host adapter gets a connection of its own for each program it serves.

### 2.5 The host interface

* At attach, the adapter claims the station's own callsigns for its program, with listening off.
  The station's behaviour is unchanged: with a host attached, nothing is answered until `LISTEN ON`.
* `MYCALL` replaces the claim. `LISTEN ON`/`OFF` set its `listening`, as do `LISTEN CQ` and
  `CHAT ON`. The reply to `MYCALL` stays `OK`, said at once as now; a callsign refused because
  another program holds it is said in the log and on the panel.
* **While a program is attached on the host interface, it speaks for the station:** the station's
  own callsigns it does not hold are not answered. Before claims, `MYCALL` replaced the station's
  callsigns, so a gateway whose Setup says KK4ODA and whose Trimode says `MYCALL KK4ODA-1` did not
  answer KK4ODA while Trimode was attached. Without this rule the claim would hand KK4ODA back to
  the station, which would then answer it unattended, with no program behind the session.
  Programs on the control API do not silence the station's own callsigns: a RadioPost gateway
  claims callsigns of its own.
* `PENDING` (ADR-0096 §2.1), the acceptance's keying (ADR-0103) and the holds of ADR-0051 and
  ADR-0100 apply to the calls the program's claim owns, and only to those.

### Not taken

* **The last program to speak wins** (today's `MYCALL`). That is the problem this ADR solves.
* **Routing by SSID convention** (for example, `-10` is RadioPost). The modem would hard-code a
  convention, and callsigns are the operator's choice.
* **A daemon per program.** Two daemons cannot share one sound card and one radio.
* **Filtering events per connection.** The panel must see every session, and a filter would make
  every new event a decision about who may see it. An `owner` field leaves that to the client.
* **A station-wide answering switch on the control API.** Neither program needs one. A program
  that wants the station's own callsigns unanswered claims them.
* **A claim that outlives its connection** (a token presented again later). A program that died
  would keep its callsigns unanswered, or answered for nobody, until it came back. A program that
  reconnects simply claims again.

## 3. Consequences

* RPMI's adapter maps `callsigns` to `claim` and `listen` to `listen`, and knows its links by
  `owner`. The `listen` line of RP-4 §9.1 changes when this is built. That is the RadioPost
  repository's edit, not this one's.
* A Trimode and a RadioPost gateway share KK4ODA-1's radio. Calls to KK4ODA-1 are answered while
  Trimode listens, and calls to KK4ODA-10 while RadioPost does. Each sees the other's sessions only
  as a busy station.
* **Debug mode** (ADR-0050) records and uploads "a session a host program runs". That now means a
  session the host interface's claim owns. A session a control-API claim owns, such as
  RadioPost's, is not uploaded. Whether it should be is a decision of its own, because RadioPost
  sessions carry other people's email.
* The Test, beacons, the operator's probes and the panel's calls are the station's own, and are
  unchanged.
* **The panel.** The apps chip names each program attached and whether it is answering, from
  `status.claims`. The Session tab says whose session is up. The manual covers both, in the same
  commit as the code (the manual rule).
* **Tests:**
  * the station's: two holders, a call to each, a holder leaving, a claim refused, a probe to a
    callsign whose holder is not listening;
  * the control server's: a closing connection ends its claim, and `claim` over POST is refused;
  * the host adapter's: a called session at another program's callsign is not its own (no
    `PENDING`, no `CONNECTED`, `BUSY ON`), and `MYCALL` with a held callsign takes the rest;
  * `two_daemons.rs`: a host program and a control-API claimant on one daemon, each called in
    turn by the other daemon;
  * the harness: its host emulator gains a control-API claimant, so a gateway scenario can run
    Trimode and RadioPost side by side.

## 4. What it removes

* `HostPresence.listening` and `HostPresence::answering`, the station-wide switch: each claim has
  its own `listening`.
* `HostFlags.listening`, an atomic shared between the adapter and the run loop: the claim table
  lives in the station.
* `set_host`'s restore of the station's callsign when a program leaves: a claim ends with its
  connection.
* The host adapter's `!called && !calling` test for a session that is not its program's: `owner`
  decides.
* `listen`'s refusal of `false` for every client.

## 5. Open

* **Bandwidth.** A VARA program's `BW` holds the station's bandwidth while it is attached (ADR-0026,
  ADR-0102). Two programs that want different bandwidths, and RPMI's `connect` `bandwidth`, need a
  rule of their own. Not decided here.
* **`MYCALL` refused in part:** whether to answer the program `OK` or `WRONG`. No bench program has
  been seen to handle `WRONG` to `MYCALL`; try Trimode, Winlink Express and VarAC first.
* **Debug mode for sessions other programs own** (§3).

## 6. Built

* **A connection's identity.** `ControlHandle::open(kind)` gives a `Connection` a number of its
  own; every request sent through it is stamped with it (`Request.connection`, never read from
  the client), and dropping it sends the modem a `closed` command, which ends its claim. The
  WebSocket server opens one per client, the host adapter one per program that attaches. A
  `POST` has none.
* **The station.** `station/claims.rs` holds the station's own callsigns, the claims and the
  owner of the call or session under way (kept after it ends, for the events that report the
  end). The engine answers to the union, the station's own first. `Station::unanswered` asks
  `answers_for` the callsign called. `connect_for` places a program's call as one of its
  callsigns; `may_act_on_session` decides `send`, `disconnect` and `abort`. A claim's `listen`
  is said in the log at once and then at most once a minute, per claim, as the host's `LISTEN`
  was (a scanning Trimode turns it every 3.6 s).
* **The host adapter** claims the station's own callsigns at attach (from `status.own_callsigns`),
  sends `claim` for `MYCALL` and `listen` for each change of its `LISTEN`, and judges a session
  its program's by the event's `owner`. `listen` is *told*, not asked (`ControlHandle::tell`: sent
  in order, its answer not waited for). `LISTEN` had been an atomic flag read every pass. A
  scanning Trimode turns it every 3.5 s, and the program's thread, waiting a slow pass for the
  answer, would hold back the `PTT ON` that a program keying the radio (`[ptt] kind = "host"`,
  the gateway's setup since 2026-11-06) keys on. The run loop does not log a host program's `listen`
  requests: the station's own lines say what they come to.
* **Tests.** `protocol.rs`: a connection speaks for itself and its close reaches the modem.
  `methods.rs`: two programs claim, one is refused the other's callsign by name, a program calls
  only as its own, `abort`/`disconnect`/`send` are `not_owner` on another's session, the panel
  may, a closed connection's claim goes, a `POST` cannot claim. `station.rs`: two holders called
  in turn over the simulated air, a host program that names other callsigns speaks for the
  station, the scanning host's lines, the program's CWID for its own callsigns only, debug mode
  recording the host program's sessions. `host/server.rs`: a call for another program's callsign
  reaches the program as a busy channel and nothing else; its LISTEN is its claim's and goes
  with its connection. `two_daemons.rs`: a host program on KK4XYZ-1 and a WebSocket program on
  KK4XYZ-10 on one daemon, called in turn by another daemon — each session told to its holder
  alone, the host program refused the other's `DISCONNECT`, the WebSocket program refused
  `abort` on the host program's session, and the WebSocket program's claim gone with its
  connection.
* **Not built here:** a harness host emulator with a control-API claimant beside it (§3) — the
  RPMI adapter will be the first real one, and its scenarios come with it.
