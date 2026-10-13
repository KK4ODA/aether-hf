---------------------------- MODULE ImplicitTurn ----------------------------
(***************************************************************************)
(* An abstraction of the link engine with ADR-0068's stated answers: the   *)
(* turn (ISS/IRS) is each station's own belief, moved by frames and        *)
(* timers; the answer to a burst is due a Gap after the end its frames     *)
(* state.                                                                  *)
(*                                                                         *)
(* What is kept, from engine.py with stated_answers on:                    *)
(*  - the ISS sends a burst; it waits Gap + ALen + Budget for the answer   *)
(*    and repeats on silence (`_send_burst`, `_on_response_timeout`);      *)
(*  - the IRS answers a decoded burst at max(now, stated end + Gap)        *)
(*    (`_state_answer`: a late decode answers late);                       *)
(*  - a burst that empties the queue offers the turn (ADR-0047); an IRS    *)
(*    with data takes it in its acknowledgement and sends its burst in the *)
(*    same transmission, and is ISS from then on (`_answer_burst`);        *)
(*  - an idle ISS polls after KA ticks (`keepalive`);                      *)
(*  - an IRS with data asks for the turn once the channel has been quiet   *)
(*    Q ticks and it senses nothing on the air (ADR-0027/0044,             *)
(*    `_maybe_request_turn`); an idle ISS answers a request with a TURN    *)
(*    and is IRS from then on (`_send_turn`); the station that reads the   *)
(*    TURN becomes ISS (`_take_iss`); an ISS that hears data from the      *)
(*    other station becomes IRS ("data wins", `_on_data`).                *)
(*  - carrier sense is perfect: a station never starts a transmission on   *)
(*    its own initiative while the other has been on the air for D ticks.  *)
(*    (The real detector misses frames; this is the favourable case.)      *)
(*                                                                         *)
(* Left out (each would add behaviours, never remove them): phantoms, the  *)
(* random backoff from the second silence, TURN retries, ADR-0029/0063     *)
(* recoveries, floor families, the daemon's holds and the identifier.      *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Horizon, BLen, ALen, PLen, Gap, LMax, Budget, Q, KA, D, Apps,
          Requests, LateAnswers, SenseMiss

ASSUME Requests \in BOOLEAN /\ LateAnswers \in BOOLEAN /\ SenseMiss \in BOOLEAN

Stations == {"A", "B"}
Other(s) == IF s = "A" THEN "B" ELSE "A"
None == 99999
NoTx == [start |-> None, end |-> None, kind |-> "none", sensed |-> FALSE]
NoSched == [at |-> None, kind |-> "none"]
Max(a, b) == IF a >= b THEN a ELSE b

VARIABLES
    now,
    role,      \* "ISS" | "IRS"
    tx,        \* [start, end, kind] of what each is sending, or None
    sched,     \* [at, kind] of a solicited answer each will send, or None
    wait,      \* deadline for the answer awaited, or None
    wfor,      \* what is awaited: "ack" (to a burst or poll) | "turn" (to a TURN) | "none"
    data,      \* the station has something queued
    offered,   \* the IRS saw the burst it answers offer the turn
    asked,     \* the IRS has asked for the turn since it last held it
    lastAct,   \* end of the last frame each heard or sent
    inbox,     \* deliveries: [to, at, kind, end]
    apps       \* application sends left (bounds the state space)

vars == <<now, role, tx, sched, wait, wfor, data, offered, asked, lastAct, inbox, apps>>

Init == /\ now = 0
        /\ role = [s \in Stations |-> IF s = "A" THEN "ISS" ELSE "IRS"]
        /\ tx = [s \in Stations |-> NoTx]
        /\ sched = [s \in Stations |-> NoSched]
        /\ wait = [s \in Stations |-> None]
        /\ wfor = [s \in Stations |-> "none"]
        /\ data = [s \in Stations |-> s = "A"]
        /\ offered = [s \in Stations |-> FALSE]
        /\ asked = [s \in Stations |-> FALSE]
        /\ lastAct = [s \in Stations |-> 0]
        /\ inbox = {}
        /\ apps = Apps

On(s) == tx[s] # NoTx /\ tx[s].start <= now /\ now < tx[s].end
Idle(s) == tx[s] = NoTx
Senses(s) == LET o == Other(s) IN tx[o] # NoTx /\ tx[o].sensed /\ now >= tx[o].start + D

Len(kind) == CASE kind = "burst" -> BLen
              [] kind = "burst_offer" -> BLen
              [] kind = "ack_taken" -> ALen + BLen   \* the acknowledgement and its burst
              [] kind = "poll" -> PLen
              [] kind = "turn" -> PLen
              [] OTHER -> ALen                       \* ack, ack_want, request

(* With SenseMiss the other station's detector may miss this transmission altogether. *)
Key(s, kind) == \E sensed \in (IF SenseMiss THEN BOOLEAN ELSE {TRUE}):
    tx' = [tx EXCEPT ![s] = [start |-> now, end |-> now + Len(kind), kind |-> kind,
                             sensed |-> sensed]]

(* ── the ISS on its own initiative ─────────────────────────────────── *)
SendBurst(s) ==
    /\ role[s] = "ISS" /\ Idle(s) /\ wait[s] = None /\ sched[s] = NoSched /\ data[s]
    /\ ~Senses(s)
    /\ Key(s, "burst_offer")         \* the burst empties the queue: it offers the turn
    /\ wait' = [wait EXCEPT ![s] = now + BLen + Gap + ALen + Budget]
    /\ UNCHANGED <<now, role, sched, data, offered, asked, lastAct, inbox, apps>>
    /\ wfor' = [wfor EXCEPT ![s] = "ack"]

Poll(s) ==
    /\ role[s] = "ISS" /\ Idle(s) /\ wait[s] = None /\ sched[s] = NoSched /\ ~data[s]
    /\ now = lastAct[s] + KA /\ ~Senses(s)
    /\ Key(s, "poll")
    /\ wait' = [wait EXCEPT ![s] = now + PLen + Gap + ALen + Budget]
    /\ UNCHANGED <<now, role, sched, data, offered, asked, lastAct, inbox, apps>>
    /\ wfor' = [wfor EXCEPT ![s] = "ack"]

(* Silence: the burst or poll goes again (no backoff modelled; ADR-0068's backoff only     *)
(* starts at the second silence and is random, so this is one of its draws).              *)
Repeat(s) ==
    /\ wait[s] # None /\ now >= wait[s] /\ Idle(s) /\ role[s] = "ISS" /\ wfor[s] = "ack"
    /\ ~Senses(s)
    /\ Key(s, IF data[s] THEN "burst_offer" ELSE "poll")
    /\ wait' = [wait EXCEPT ![s] = now + (IF data[s] THEN BLen ELSE PLen) + Gap + ALen + Budget]
    /\ UNCHANGED <<now, role, sched, data, offered, asked, lastAct, inbox, apps>>
    /\ wfor' = [wfor EXCEPT ![s] = "ack"]

(* A TURN nobody answered: the turn is taken back (after `turn_retries` tries in the engine; *)
(* one here — more tries only move the same race later).                                    *)
TakeBack(s) ==
    /\ wait[s] # None /\ now >= wait[s] /\ wfor[s] = "turn" /\ Idle(s)
    /\ role' = [role EXCEPT ![s] = "ISS"]
    /\ wait' = [wait EXCEPT ![s] = None]
    /\ wfor' = [wfor EXCEPT ![s] = "none"]
    /\ lastAct' = [lastAct EXCEPT ![s] = IF now >= KA THEN now - KA ELSE 0]  \* it polls at once
    /\ UNCHANGED <<now, tx, sched, data, offered, asked, inbox, apps>>

(* ── the IRS on its own initiative: the request for the turn ───────── *)
Request(s) ==
    /\ Requests /\ role[s] = "IRS" /\ Idle(s) /\ sched[s] = NoSched /\ data[s] /\ ~asked[s]
    /\ now >= lastAct[s] + Q /\ ~Senses(s)
    /\ Key(s, "request")
    /\ asked' = [asked EXCEPT ![s] = TRUE]
    /\ UNCHANGED <<now, role, sched, wait, wfor, data, offered, lastAct, inbox, apps>>

(* ── solicited answers, at their scheduled time ────────────────────── *)
Answer(s) ==
    /\ sched[s] # NoSched /\ now >= sched[s].at /\ Idle(s)
    /\ Key(s, sched[s].kind)
    /\ sched' = [sched EXCEPT ![s] = NoSched]
    /\ IF sched[s].kind = "ack_taken"
         THEN /\ role' = [role EXCEPT ![s] = "ISS"]
              /\ wait' = [wait EXCEPT ![s] = now + ALen + BLen + Gap + ALen + Budget]
              /\ wfor' = [wfor EXCEPT ![s] = "ack"]
              /\ asked' = [asked EXCEPT ![s] = FALSE]
         ELSE IF sched[s].kind = "turn"
           THEN \* the TURN's sender is IRS at once and waits for the first burst or poll
                /\ role' = [role EXCEPT ![s] = "IRS"]
                /\ wait' = [wait EXCEPT ![s] = now + PLen + Gap + BLen + Budget]
                /\ wfor' = [wfor EXCEPT ![s] = "turn"]
                /\ UNCHANGED asked
           ELSE IF sched[s].kind \in {"burst_offer", "poll"}   \* answering a TURN as ISS
             THEN /\ wait' = [wait EXCEPT ![s] = now + Len(sched[s].kind) + Gap + ALen + Budget]
                  /\ wfor' = [wfor EXCEPT ![s] = "ack"]
                  /\ UNCHANGED <<role, asked>>
             ELSE UNCHANGED <<role, wait, wfor, asked>>
    /\ UNCHANGED <<now, data, offered, lastAct, inbox, apps>>

(* A transmission ends: the frame is lost, or reaches the other engine 0..LMax later. *)
TxEnd(s) ==
    /\ tx[s] # NoTx /\ now = tx[s].end
    /\ \E d \in {{}} \cup {{[to |-> Other(s), at |-> now + l, kind |-> tx[s].kind, end |-> now]}
                           : l \in 0..LMax}:
         inbox' = inbox \cup d
    /\ tx' = [tx EXCEPT ![s] = NoTx]
    /\ lastAct' = [lastAct EXCEPT ![s] = now]
    \* what a burst carried is delivered if the answer comes; the model keeps data until then
    /\ UNCHANGED <<now, role, sched, wait, wfor, data, offered, asked, apps>>

(* ── what a station does with a frame it decoded ───────────────────── *)
Receive(s) ==
  \E f \in inbox:
    /\ f.to = s /\ f.at = now
    /\ inbox' = inbox \ {f}
    /\ lastAct' = [lastAct EXCEPT ![s] = Max(lastAct[s], f.end)]
    /\ CASE f.kind \in {"burst", "burst_offer", "ack_taken"} ->
              \* data: whoever hears it is the receiver now ("data wins"); it answers
              \* a Gap after the end the frames state, or at once if it decoded late
              /\ role' = [role EXCEPT ![s] = "IRS"]
              /\ wait' = [wait EXCEPT ![s] = None]
              /\ wfor' = [wfor EXCEPT ![s] = "none"]
              \* decoded too late for its slot: answered at once (ADR-0068's engine) or not at all
              /\ sched' = IF LateAnswers \/ now <= f.end + Gap
                           THEN [sched EXCEPT ![s] =
                                  [at |-> Max(now, f.end + Gap),
                                   kind |-> IF f.kind = "burst_offer" /\ data[s] THEN "ack_taken"
                                            ELSE IF data[s] THEN "ack_want" ELSE "ack"]]
                           ELSE sched
              \* an acknowledgement that took the turn also acknowledges what this
              \* station sent: its data is through
              /\ data' = IF f.kind = "ack_taken" THEN [data EXCEPT ![s] = FALSE] ELSE data
              /\ UNCHANGED <<offered, asked>>
         [] f.kind = "poll" ->
              /\ sched' = IF LateAnswers \/ now <= f.end + Gap
                           THEN [sched EXCEPT ![s] = [at |-> Max(now, f.end + Gap),
                                  kind |-> IF data[s] THEN "ack_want" ELSE "ack"]]
                           ELSE sched
              /\ role' = [role EXCEPT ![s] = "IRS"]
              \* a poll answers a TURN this station sent: the wait is over
              /\ wait' = IF wfor[s] = "turn" THEN [wait EXCEPT ![s] = None] ELSE wait
              /\ wfor' = IF wfor[s] = "turn" THEN [wfor EXCEPT ![s] = "none"] ELSE wfor
              /\ UNCHANGED <<data, offered, asked>>
         [] f.kind \in {"ack", "ack_want", "request"} ->
              IF role[s] = "ISS" /\ (wfor[s] = "ack" \/ f.kind = "request")
                THEN /\ wait' = [wait EXCEPT ![s] = None]
                     /\ wfor' = [wfor EXCEPT ![s] = "none"]
                     /\ data' = IF f.kind = "request" THEN data ELSE [data EXCEPT ![s] = FALSE]
                     \* the other station wants the turn and this one has nothing: TURN
                     /\ sched' = IF f.kind \in {"ack_want", "request"} /\ ~data[s]
                                   THEN [sched EXCEPT ![s] = [at |-> now, kind |-> "turn"]]
                                   ELSE sched
                     /\ UNCHANGED <<role, offered, asked>>
                ELSE UNCHANGED <<role, sched, wait, wfor, data, offered, asked>>
         [] f.kind = "turn" ->
              /\ role' = [role EXCEPT ![s] = "ISS"]
              /\ asked' = [asked EXCEPT ![s] = FALSE]
              /\ sched' = IF LateAnswers \/ now <= f.end + Gap
                           THEN [sched EXCEPT ![s] = [at |-> Max(now, f.end + Gap),
                                  kind |-> IF data[s] THEN "burst_offer" ELSE "poll"]]
                           ELSE sched
              /\ wait' = [wait EXCEPT ![s] = None]
              /\ wfor' = [wfor EXCEPT ![s] = "none"]
              /\ UNCHANGED <<data, offered>>
    /\ UNCHANGED <<now, tx, apps>>

(* The application hands a station something to send. *)
App(s) == /\ apps > 0 /\ ~data[s]
          /\ data' = [data EXCEPT ![s] = TRUE] /\ apps' = apps - 1
          /\ UNCHANGED <<now, role, tx, sched, wait, wfor, offered, asked, lastAct, inbox>>

Due == \/ \E s \in Stations: tx[s] # NoTx /\ now = tx[s].end
       \/ \E f \in inbox: f.at = now
       \/ \E s \in Stations: sched[s] # NoSched /\ now >= sched[s].at /\ Idle(s)
       \/ \E s \in Stations: wait[s] # None /\ now >= wait[s] /\ Idle(s) /\ wfor[s] = "turn"
Tick == /\ ~Due /\ now < Horizon /\ now' = now + 1
        /\ UNCHANGED <<role, tx, sched, wait, wfor, data, offered, asked, lastAct, inbox, apps>>
Done == /\ now = Horizon /\ UNCHANGED vars

Next == \/ \E s \in Stations: TxEnd(s) \/ Receive(s) \/ Answer(s) \/ SendBurst(s)
                               \/ Poll(s) \/ Repeat(s) \/ Request(s) \/ App(s)
                               \/ TakeBack(s)
        \/ Tick \/ Done
Spec == Init /\ [][Next]_vars

(* ── properties ─────────────────────────────────────────────────────── *)
NoOverlap == ~(On("A") /\ On("B"))
AtMostOneISS == ~(role["A"] = "ISS" /\ role["B"] = "ISS")
(* Someone will transmit: a scheduled answer, a wait, a transmission, a frame in flight, or *)
(* an ISS (which polls or sends) — else both wait for the other until the link times out.  *)
SomeoneWillAct == \/ \E s \in Stations: role[s] = "ISS" \/ sched[s] # NoSched \/ tx[s] # NoTx
                  \/ inbox # {}
=============================================================================
