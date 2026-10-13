---------------------------- MODULE GrantedSlots ----------------------------
(***************************************************************************)
(* Candidate D, "granted slots": one master for the whole session.         *)
(*                                                                         *)
(* Every master transmission grants the slave exactly one slot, which      *)
(* starts Gap ticks after the transmission ends and is at most SLen long,  *)
(* and carries an exchange id. The slave transmits only inside a slot it   *)
(* was granted by a frame it decoded, and only if it decoded it in time    *)
(* to start the slot on time (LatePolicy = "forfeit"); its frame states    *)
(* where it ends and echoes the exchange id. The master transmits again    *)
(* when the slot cannot be in use any more: a Gap after the end a slave    *)
(* frame of the current exchange stated, or a Gap after the slot's latest  *)
(* end when it decoded nothing.                                            *)
(*                                                                         *)
(* Every frame may be lost; a frame that is not lost reaches the other     *)
(* engine 0..LMax ticks after its last sample (decode latency). Time is    *)
(* one global clock: both stations agree on when a frame they hear ended   *)
(* (bounded propagation and clock error are folded into Gap).              *)
(*                                                                         *)
(* Two switches show which rule carries the safety argument:               *)
(*   LatePolicy = "send": the slave answers late, as ADR-0068's engine     *)
(*                        does (max(now, due));                            *)
(*   CheckXid   = FALSE:  the master takes any slave frame as the answer   *)
(*                        to its current grant (no exchange id).           *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Horizon, MLen, SLen, Gap, LMax, LatePolicy, CheckXid

ASSUME /\ MLen >= 1 /\ SLen >= 1 /\ Gap >= 1 /\ LMax >= 0
       /\ LatePolicy \in {"forfeit", "send"} /\ CheckXid \in BOOLEAN

Max(a, b) == IF a >= b THEN a ELSE b
Min(a, b) == IF a <= b THEN a ELSE b
None == 99999

VARIABLES
    now,
    mStart,   \* start of the master's transmission on the air, or None
    mEnd,     \* its end (a grant alone, or a grant with data: any length up to MLen)
    sStart,   \* start of the slave's transmission on the air, or None
    sEnd,     \* its end (the slave chooses a length up to SLen)
    mNext,    \* when the master transmits next, unless a slave frame lets it go earlier
    xid,      \* the master's current exchange id (0/1), in every frame it sends
    sDue,     \* when the slave starts its granted transmission, or None
    sXid,     \* the exchange id it will echo
    inbox,    \* frames on their way to an engine: [to, at, kind, slotStart, slotEnd, xid, end]
    exchanges \* completed exchanges (a slave frame of the current exchange decoded)

vars == <<now, mStart, mEnd, sStart, sEnd, mNext, xid, sDue, sXid, inbox, exchanges>>

Init == /\ now = 0 /\ mStart = None /\ mEnd = None /\ sStart = None /\ sEnd = None
        /\ mNext = 0 /\ xid = 0 /\ sDue = None /\ sXid = 0
        /\ inbox = {} /\ exchanges = 0

MOn == mStart # None /\ mStart <= now /\ now < mEnd
SOn == sStart # None /\ sStart <= now /\ now < sEnd

(* A frame ends: it is lost, or reaches the other engine 0..LMax later. *)
Deliveries(to, kind, slotStart, slotEnd, x, end) ==
    {{}} \cup {{[to |-> to, at |-> end + l, kind |-> kind, slotStart |-> slotStart,
                 slotEnd |-> slotEnd, xid |-> x, end |-> end]} : l \in 0..LMax}

(* The master keys when its time comes and it is not on the air. *)
MasterKey ==
    /\ mStart = None
    /\ now = mNext
    /\ \E len \in 1..MLen:
         /\ mStart' = now /\ mEnd' = now + len
         \* until a slave frame says otherwise: the granted slot's latest end and a Gap
         /\ mNext' = now + len + Gap + SLen + Gap
    /\ xid' = 1 - xid
    /\ UNCHANGED <<now, sStart, sEnd, sDue, sXid, inbox, exchanges>>

(* The master's transmission ends: the grant goes out. *)
MasterEnd ==
    /\ mStart # None /\ now = mEnd
    /\ \E d \in Deliveries("S", "grant", now + Gap, now + Gap + SLen, xid, now):
         inbox' = inbox \cup d
    /\ mStart' = None /\ mEnd' = None
    /\ UNCHANGED <<now, sStart, sEnd, mNext, xid, sDue, sXid, exchanges>>

(* The slave's engine gets a grant. *)
SlaveGetsGrant ==
    \E f \in inbox:
      /\ f.to = "S" /\ f.at = now
      /\ inbox' = inbox \ {f}
      /\ IF now <= f.slotStart
           THEN /\ sDue' = f.slotStart /\ sXid' = f.xid
           ELSE IF LatePolicy = "send"
                  THEN /\ sDue' = now /\ sXid' = f.xid   \* answers late
                  ELSE UNCHANGED <<sDue, sXid>>          \* forfeits the slot
      /\ UNCHANGED <<now, mStart, mEnd, sStart, sEnd, mNext, xid, exchanges>>

(* The slave keys at the start of its slot, for any length up to the grant. *)
SlaveKey ==
    /\ sDue = now /\ sStart = None
    /\ \E len \in 1..SLen: /\ sStart' = now /\ sEnd' = now + len
    /\ sDue' = None
    /\ UNCHANGED <<now, mStart, mEnd, mNext, xid, sXid, inbox, exchanges>>

(* The slave's transmission ends: its frame states that end and echoes the id. *)
SlaveEnd ==
    /\ sStart # None /\ now = sEnd
    /\ \E d \in Deliveries("M", "answer", 0, 0, sXid, now): inbox' = inbox \cup d
    /\ sStart' = None /\ sEnd' = None
    /\ UNCHANGED <<now, mStart, mEnd, mNext, xid, sDue, sXid, exchanges>>

(* The master's engine gets a slave frame. *)
MasterGetsAnswer ==
    \E f \in inbox:
      /\ f.to = "M" /\ f.at = now
      /\ inbox' = inbox \ {f}
      /\ IF ~CheckXid \/ f.xid = xid
           THEN /\ mNext' = IF mStart # None
                              THEN mNext            \* on the air: nothing to move
                              ELSE Max(now, Min(mNext, f.end + Gap))
                /\ exchanges' = exchanges + 1
           ELSE UNCHANGED <<mNext, exchanges>>      \* an answer to an old grant
      /\ UNCHANGED <<now, mStart, mEnd, sStart, sEnd, xid, sDue, sXid>>

(* Time moves on only when nothing is due now. *)
Due == \/ (mStart # None /\ now = mEnd)
       \/ (sStart # None /\ now = sEnd)
       \/ (now = mNext /\ mStart = None)
       \/ (sDue = now /\ sStart = None)
       \/ \E f \in inbox: f.at = now
Tick == /\ ~Due /\ now < Horizon /\ now' = now + 1
        /\ UNCHANGED <<mStart, mEnd, sStart, sEnd, mNext, xid, sDue, sXid, inbox, exchanges>>
Done == /\ now = Horizon /\ ~Due /\ UNCHANGED vars

Next == Done \/ MasterEnd \/ SlaveEnd \/ MasterGetsAnswer \/ SlaveGetsGrant
        \/ MasterKey \/ SlaveKey \/ Tick
Spec == Init /\ [][Next]_vars

(* ── properties ─────────────────────────────────────────────────────── *)
TypeOK == /\ now \in 0..Horizon /\ xid \in {0, 1}

(* Safety: the two transmitters are never on the air at the same tick. *)
NoOverlap == ~(MOn /\ SOn)

(* Bounded recovery: the master is never more than one exchange from keying. *)
BoundedSilence == mNext <= now + MLen + Gap + SLen + Gap

(* The run never gets stuck before the horizon (also checked as TLC deadlock). *)
SomethingScheduled == mNext # None

(* Progress is possible: the run can always continue (checked as TLC deadlock). *)
=============================================================================
