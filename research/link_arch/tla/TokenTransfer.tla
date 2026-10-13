---------------------------- MODULE TokenTransfer ----------------------------
(***************************************************************************)
(* Candidate B: an explicit transmit token that changes hands.            *)
(*                                                                         *)
(* The holder may start a transmission on its own initiative (data, which  *)
(* the other station answers a tick after it ends, or a GRANT of the       *)
(* token); the other station only answers. The grantee's first data frame  *)
(* is the proof it took the token, and the grantor's answer to it is the   *)
(* CONFIRM. Frames are one tick long; each is lost (at most MaxLossRun in  *)
(* a row) or decoded 0..LMax ticks after it ends; an answer decoded too    *)
(* late for its slot is not sent.                                          *)
(*                                                                         *)
(* Design = "naive": the grantee holds the token from the GRANT on; a      *)
(*   grantor that hears nothing of the grantee for W ticks takes it back.  *)
(* Design = "lease" (Gray & Cheriton 1989): authority is a lease that only *)
(*   the other station renews, by answering. A station may initiate only   *)
(*   while now + 1 <= renewed + Renew (an unconfirmed grantee: grant end + *)
(*   Lease). Taking back a lost grant restores no lease. The only way to   *)
(*   authority without a renewal is the caller's reclaim, allowed once     *)
(*   every renewal the caller itself gave can have expired at the other    *)
(*   end: Renew + LMax + 1 after its own last frame, and Lease after its   *)
(*   last GRANT. The called station never reclaims (fixed tie-break).      *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Horizon, LMax, W, Lease, Renew, Design, MaxLossRun

ASSUME Design \in {"naive", "lease"} /\ Lease <= Renew

Stations == {"A", "B"}
Other(s) == IF s = "A" THEN "B" ELSE "A"
None == 99999
NoTx == [start |-> None, kind |-> "none"]
NoSched == [at |-> None, kind |-> "none"]
Max(a, b) == IF a >= b THEN a ELSE b

VARIABLES
    now,
    tok,       \* "held" | "granted" (its GRANT is out) | "unconfirmed" | "none"
    renewed,   \* lease: when the other station last renewed this one's authority
    grantAt,   \* when this station's last GRANT ended (the grantor), or when it heard one
    heard,     \* last tick each station decoded a frame from the other
    lastTx,    \* end of each station's last frame
    tx, sched, inbox, lossRun,
    free       \* the earliest tick a station may initiate: after the answer slot it solicited

vars == <<now, tok, renewed, grantAt, heard, lastTx, tx, sched, inbox, lossRun, free>>

Init == /\ now = 0
        /\ tok = [s \in Stations |-> IF s = "A" THEN "held" ELSE "none"]
        /\ renewed = [s \in Stations |-> 0]
        /\ grantAt = [s \in Stations |-> 0]
        /\ heard = [s \in Stations |-> 0]
        /\ lastTx = [s \in Stations |-> 0]
        /\ tx = [s \in Stations |-> NoTx]
        /\ sched = [s \in Stations |-> NoSched]
        /\ inbox = {}
        /\ lossRun = 0
        /\ free = [s \in Stations |-> 0]

On(s) == tx[s] # NoTx /\ tx[s].start = now
Busy(s) == tx[s] # NoTx \/ sched[s] # NoSched

Valid(s) ==   \* the lease still covers a one-tick frame starting now
    IF Design = "naive" THEN TRUE
    ELSE IF tok[s] = "unconfirmed" THEN now + 1 <= grantAt[s] + Lease
    ELSE now + 1 <= renewed[s] + Renew
MayInitiate(s) == tok[s] \in {"held", "unconfirmed"} /\ Valid(s)

Initiate(s) ==
    /\ MayInitiate(s) /\ ~Busy(s) /\ now >= free[s]
    /\ free' = [free EXCEPT ![s] = now + 3]
    \* only a confirmed holder hands the token on: two grants out at once undo each other
    /\ \E kind \in (IF tok[s] = "held" THEN {"data", "grant"} ELSE {"data"}):
         /\ tx' = [tx EXCEPT ![s] = [start |-> now, kind |-> kind]]
         /\ IF kind = "grant"
              THEN /\ tok' = [tok EXCEPT ![s] = "granted"]
                   /\ grantAt' = [grantAt EXCEPT ![s] = now + 1]
              ELSE UNCHANGED <<tok, grantAt>>
    /\ UNCHANGED <<now, renewed, heard, lastTx, sched, inbox, lossRun>>

Answer(s) ==
    /\ sched[s].at = now /\ tx[s] = NoTx
    /\ tx' = [tx EXCEPT ![s] = [start |-> now, kind |-> sched[s].kind]]
    /\ sched' = [sched EXCEPT ![s] = NoSched]
    /\ UNCHANGED <<now, tok, renewed, grantAt, heard, lastTx, inbox, lossRun, free>>

FrameEnd(s) ==
    /\ tx[s] # NoTx /\ now = tx[s].start + 1
    /\ \/ /\ lossRun < MaxLossRun /\ inbox' = inbox /\ lossRun' = lossRun + 1
       \/ /\ \E l \in 0..LMax:
               inbox' = inbox \cup {[to |-> Other(s), at |-> now + l, kind |-> tx[s].kind,
                                     end |-> now]}
          /\ lossRun' = 0
    /\ tx' = [tx EXCEPT ![s] = NoTx]
    /\ lastTx' = [lastTx EXCEPT ![s] = now]
    /\ UNCHANGED <<now, tok, renewed, grantAt, heard, sched, free>>

Owe(s, f, kind) ==
    IF now <= f.end + 1 /\ sched[s] = NoSched /\ tx[s] = NoTx
      THEN [sched EXCEPT ![s] = [at |-> f.end + 1, kind |-> kind]]
      ELSE sched

Receive(s) ==
  \E f \in inbox:
    /\ f.to = s /\ f.at = now
    /\ inbox' = inbox \ {f}
    /\ heard' = [heard EXCEPT ![s] = now]
    /\ CASE f.kind = "grant" ->
              /\ tok' = [tok EXCEPT ![s] = IF tok[s] = "none" THEN "unconfirmed" ELSE tok[s]]
              /\ grantAt' = [grantAt EXCEPT ![s] = IF tok[s] = "none" THEN f.end ELSE grantAt[s]]
              /\ UNCHANGED <<sched, renewed>>
         [] f.kind = "data" ->
              \* the sender holds the token: answer it; a grantor hearing its grantee's data
              \* has the proof the token moved, and its answer is the CONFIRM
              /\ tok' = [tok EXCEPT ![s] = IF tok[s] = "granted" THEN "none" ELSE tok[s]]
              /\ sched' = Owe(s, f, IF tok[s] = "granted" THEN "confirm" ELSE "ack")
              /\ UNCHANGED <<renewed, grantAt>>
         [] f.kind \in {"ack", "confirm"} ->
              \* an answer renews the lease; a CONFIRM makes an unconfirmed grantee the holder
              /\ tok' = [tok EXCEPT ![s] = IF f.kind = "confirm" /\ tok[s] = "unconfirmed"
                                             THEN "held" ELSE tok[s]]
              /\ renewed' = [renewed EXCEPT ![s] = f.end]
              /\ UNCHANGED <<sched, grantAt>>
    /\ UNCHANGED <<now, tx, lossRun, lastTx, free>>

(* A grantor that heard nothing from its grantee takes the token back after W. Under the  *)
(* lease design this restores no lease: it may initiate only on a renewal it had before. *)
TakeBack(s) ==
    /\ tok[s] = "granted" /\ now >= grantAt[s] + W /\ heard[s] < grantAt[s]
    /\ tok' = [tok EXCEPT ![s] = "held"]
    /\ UNCHANGED <<now, renewed, grantAt, heard, lastTx, tx, sched, inbox, lossRun, free>>

(* The caller's reclaim: authority without a renewal, once none it gave can be alive. *)
Reclaim(s) ==
    /\ Design = "lease" /\ s = "A" /\ ~MayInitiate(s) /\ ~Busy(s) /\ tok[s] # "granted"
    /\ now >= lastTx[s] + LMax + Renew + 1
    /\ now >= grantAt[s] + Lease + 1
    /\ tok' = [tok EXCEPT ![s] = "held"]
    /\ renewed' = [renewed EXCEPT ![s] = now]
    /\ UNCHANGED <<now, grantAt, heard, lastTx, tx, sched, inbox, lossRun, free>>

Due == \/ \E s \in Stations: tx[s] # NoTx /\ now = tx[s].start + 1
       \/ \E s \in Stations: sched[s].at = now /\ tx[s] = NoTx
       \/ \E f \in inbox: f.at = now
       \/ \E s \in Stations: ENABLED TakeBack(s) \/ ENABLED Reclaim(s)
Tick == /\ ~Due /\ now < Horizon /\ now' = now + 1
        /\ UNCHANGED <<tok, renewed, grantAt, heard, lastTx, tx, sched, inbox, lossRun, free>>
Done == now = Horizon /\ UNCHANGED vars

Next == \/ \E s \in Stations: Initiate(s) \/ Answer(s) \/ FrameEnd(s) \/ Receive(s)
                               \/ TakeBack(s) \/ Reclaim(s)
        \/ Tick \/ Done
Spec == Init /\ [][Next]_vars

(* Safety: never both entitled to start a transmission on their own initiative. *)
ExclusiveAuthority == ~(MayInitiate("A") /\ MayInitiate("B"))
NoOverlap == ~(On("A") /\ On("B"))
(* Vacancy: nobody may initiate. Under the lease it lasts no longer than the reclaim bound. *)
SomeoneMay == \E s \in Stations: MayInitiate(s) \/ tok[s] = "granted"
BoundedVacancy ==
    \/ SomeoneMay \/ Busy("A") \/ Busy("B") \/ inbox # {}
    \/ ENABLED Reclaim("A")
    \/ now <= Max(lastTx["A"] + LMax + Renew + 1, grantAt["A"] + Lease + 1)
=============================================================================
