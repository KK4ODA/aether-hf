# The current link protocol, exchange by exchange

These diagrams show protocol 7 (the engine on the air, beta.105) and the ADR-0068 path
(`LinkConfig.stated_answers`, protocol 8 candidate). They were drawn from the code at `7546df3`:
the model engine `model/aether_model/link/engine.py`, the port `core/aether-link/src/engine.rs`
and the daemon `core/aetherd/src/station.rs`. The ADRs were checked against the code, not taken
as the spec. Where ADR-0068 behaves differently, the diagram says so.

A **⚠ Wn** marks a point where both stations can reasonably believe they may key at the same
moment. Section 11 lists every such point with its conditions, the rule that patched it, and
what is still open. "Holds" means the daemon's gates between the engine and the key: the answer
gap (ADR-0036/0054), the busy and identifier waits (ADR-0022), the Morse identifier, the CAT
thread (beta.98) and the regulatory gate. The engine is told of some of these
(`on_tx_delayed`) and not of others (queue order, CAT latency).

Notation: A is the caller and B the called station. ISS/IRS is each engine's own *belief* about
who holds the turn (`role`). `wait` is the sender's response timer; `ack` is the receiver's
answer timer.

## 1. Connection establishment

```mermaid
sequenceDiagram
    participant A as A (caller)
    participant B as B (called)
    Note over A: try 1 on the tone floor (ADR-0016), tries alternate families
    A->>B: CONNECT_REQ
    Note over B: decoded. CONNECTED, role IRS. Reply held by the answer gap, own or learned (0036, 0054)
    B->>A: CONNECT_ACK in the request's family, or the floor for a weak call (0056)
    Note over A: CONNECTED, role ISS, rate seeded from the body's SNR (0008, 0066)
    alt something queued
        A->>B: first burst at first_mode(snr)
    else nothing queued after Timer First
        A->>B: POLL with OFFER (0053)
    end
    rect rgb(255, 240, 230)
    Note over A,B: CONNECT_ACK lost or late
    Note over A: connect timer, jittered, moved past arrivals (0016 §2.5)
    Note over A,B: ⚠ W2 an acceptance decoded late or held by the answer gap meets try 2
    A-xB: CONNECT_REQ try 2, ordinary, not decoded at B
    Note over B: CONNECTED already. Answers the preamble alone with a floor ACK (0016)
    B->>A: ACK (floor)
    Note over A: ⚠ W1 try 3 is due. Keyed over B's ACK when the arrival was not announced
    Note over A: 0048 reads an ACK of its own session as acceptance, unless compression was offered
    end
    rect rgb(255, 240, 230)
    Note over A,B: acceptance keyed into A's VOX hold (KE4QCM), never heard
    A->>B: CONNECT_REQ, new session number
    Note over B: 0056 ends the dead session and answers. The ended session's identifier fell due (0055 §4) ⚠ W16
    end
```

## 2. Normal data and acknowledgement

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    A->>B: DATA f0, f1, f2 (each frame's chips state how many follow)
    Note over B: legacy: the burst end is inferred from countdowns (pairs, never short, 0046), announced arrivals (0061), phantoms filtered (0065), silence after a guessed length (0012)
    Note over B: ADR-0068: due = end of the latest decoded frame + count x slot + turnaround
    Note over B: ack timer fires. Holds may delay the ACK (gap, identifier, CAT)
    B->>A: ACK (bitmap, recommended rung, SNR, WANT_TX)
    Note over A: wait = burst end + answer slot (+1 frame for bursts of 2 or more, +2 when offering)
    Note over A: legacy and ADR-0068 as built: wait extended past every announced arrival (0030)
    A->>B: next burst
```

## 3. Multi-frame bursts

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    Note over A: legacy: up to the key limit (26 s), countdown c = ceil(n/2) capped at 3 (0046)
    Note over A: ADR-0068: at most 4 slots including any prefix ACK and the offer, exact count 0..3
    A->>B: f0 (count 3)
    A->>B: f1 (count 2)
    A-xB: f2 (count 1) lost in a fade
    A->>B: f3 (count 0)
    Note over B: the burst ends where the latest believed frame says (f3)
    B->>A: ACK (f2 missing)
    rect rgb(255, 240, 230)
    Note over A,B: f3 also lost (the tail fades)
    Note over B: legacy: f1's pair bound says at most 2 follow. Answer due after f3's slot, unless a phantom or an announcement moves it (0061, 0065, 0067)
    Note over B: ADR-0068: f1 stated 2, answer at f3's end + turnaround
    Note over B: ⚠ W4 legacy, early answer: a misread count, or "3 = three or more" with six frames (0045 §4). Answered over the last frames
    end
```

## 4. Lost data frames

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    A->>B: f0 RV0, f1 RV0
    Note over B: f1 fails. HARQ buffer kept, a trusted failure feeds the rate controller (0020)
    B->>A: ACK (f1 missing, rung recommended)
    A->>B: f1 RV0 again (RV_SEQUENCE 0,0,2,3, 0043), new frames
    Note over B: soft-combined
    alt stranded: max_combines copies at a rung the path cannot carry
        Note over A: re-encoded at the slowest fitting rung (0038 hopeless, P6-7)
    end
```

## 5. Lost acknowledgement

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    A->>B: burst
    Note over B: decoded. Answer due at the stated end
    alt ACK lost on the path
        B-xA: ACK
        Note over A: wait expires. back_off steps the rung down by 2 (0012). Burst repeated
        Note over A: ADR-0068 as built: from the second silence a random backoff (Timer Resend)
        A->>B: same burst
        Note over B: all blocks already had: ADR-0059 moves ACKs to the floor for the session
    else ACK late: decoded late, held by gap, identifier or CAT
        Note over B: legacy and ADR-0068 both answer at max(now, due), never forfeit
        Note over A: wait expired (only an announced arrival extends it)
        A->>B: burst again
        B->>A: ACK
        Note over A,B: ⚠ W3 both keyed. The largest class left under ADR-0068 in simulation
    end
    Note over A: Timer Resend is not extended by arrivals (port audit S1) ⚠ W9
```

## 6. A completely undecodable burst

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    A->>B: f0, f1, f2 (detected, nothing decodes)
    alt legacy
        Note over B: arms an ACK from announcements and silence (0012 §4.3). A phantom or a misread length moves it (0040, 0061)
        B->>A: ACK saying nothing received
    else ADR-0068
        Note over B: no statement read. Not answered. Timer Close ends the burst in the rate controller
        Note over A: wait expires, burst repeated
    end
    Note over B: legacy: an untrusted frame after the ACK starts a burst only after an ordinary frame of it (0065)
```

## 7. TURN and data in the reverse direction

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS, data queued)
    alt B asks unasked (0027, every session since 0044)
        Note over B: quiet for reaction_s since the last frame heard, nothing sensed
        B->>A: ACK with WANT_TX (unsolicited)
        Note over A,B: ⚠ W5 A started its next burst or poll in the same instant. The detector misses or has not yet announced it
        A->>B: TURN (A becomes IRS on sending, 0023)
        Note over B: reads the TURN, becomes ISS
    else A's burst empties its queue (0047)
        A->>B: last data + TURN with OFFER
        B->>A: ACK with TAKEN and WANT_TX, then B's burst in the same keying
        Note over B: role ISS from the moment it sends
        alt TAKEN lost
            Note over A: 0047 infers taken from a trusted undecoded data frame. Otherwise wait expires
            A->>B: repeats its offer burst
            Note over A,B: ⚠ W6 both ISS. TLA trace ImplicitTurn_forfeit_perfect
        else a phantom read as B's data
            Note over A: becomes IRS on noise. 0063 gives the turn back if B's ACK without TAKEN is read
            Note over A,B: ⚠ W8 both IRS until the link timeout when that ACK is lost. TLA trace ImplicitTurn_deadlock
        end
    end
    rect rgb(255, 240, 230)
    Note over A,B: plain TURN lost
    Note over A: TURN repeated up to turn_retries, more while unread frames are heard (0029 C). Then takes the turn back
    Note over A,B: ⚠ W7 both ISS if B had read a TURN. Caller keeps the turn, a called ISS yields to a read POLL (0023)
    end
```

## 8. Rate adaptation and the ladder

```mermaid
sequenceDiagram
    participant A as A (ISS)
    participant B as B (IRS)
    A->>B: burst at rung r (OFDM, 1 s frames)
    B->>A: ACK recommending r-4: the tone floor (5.36 s frames)
    Note over A: the family changes. Every wait in the engine is sized from the family in use
    A->>B: burst at the floor rung
    Note over B: ADR-0068: the answer goes on the floor unless B's report clears the ordinary threshold by 3 dB
    alt the wait was sized for the other family
        Note over A: wait for an ordinary ACK, answer comes on the floor 2-3 s later
        Note over A,B: ⚠ W15 retry keyed over the floor answer (0016 §2.4, 0028, 0034 each patched one case)
    end
    Note over A: silence steps the rung down by 2 (0012), poll silences from the second (0032)
    Note over A: Test ladder: pin_mode per rung, rungs 12-13 decode short on the clean loopback (P6-7)
```

## 9. Ending a session

```mermaid
sequenceDiagram
    participant A as A
    participant B as B
    alt the ISS leaves
        Note over A: queue acknowledged, or patience out (0039)
        A->>B: DISC
        Note over B: DISC_ACK never held by wait_for_clear, waits ANSWER_LISTEN_S (0022)
        B->>A: DISC_ACK and the closing identifier
        Note over A: ⚠ W14 a DISC retry keyed over the DISC_ACK and identifier (0022 fixed four ways)
    else the IRS leaves between bursts (0023)
        B->>A: DISC in place of its ACK
        Note over A,B: ⚠ W10 A's next burst. The harness found it held 2 s by 0022's identifier rule (0042)
    else an abort
        A->>B: DISC, then up to 3 more on the floor (Timer Leave, 0052)
        Note over B: if all lost, B stays in the session until its link timeout
        Note over A: frames of the left session answered with DISC from idle ⚠ W11
    end
    Note over A,B: the identifier never follows a burst's frames (0068 §2.5). In the port it goes ahead of ACKs (audit S9) ⚠ W12
```

## 10. Timeout and reconnection

```mermaid
sequenceDiagram
    participant A as A
    participant B as B
    Note over A,B: path fades. Link timer = max(45 s, 4 exchanges in the family in use) (0012, 0017, 0033)
    A-xB: bursts, polls, step-downs to the floor (0012, 0032)
    Note over A: max_retries (8) silences gives "no response". The session is remembered as left for a link timeout (0052)
    Note over B: its own link timer. Until then B may still answer or ask (W5, W11)
    A->>B: new call, new session number
    Note over B: 0056 ends B's dead session and answers. Without it the call was another session's frame until B's timeout
    Note over A,B: port bug D1: "closed (no disc ack)" is not in LEFT_UNHEARD, so a session ended that way is never remembered as left
```

## 11. Every point where both stations may key

| # | Point | Who keys | Patched by | Still open |
|---|---|---|---|---|
| W1 | Call retry against an answer to an undecodable try | caller's timer; callee's preamble answer | 0016 §2.5 (retry moved past arrivals), 0048 | when the arrival is not announced |
| W2 | Call retry against a late acceptance | caller's timer; callee's acceptance, decoded late or held by the gap | 0016 §2.5, 0036, 0054 | an acceptance held past the retry's jittered timer |
| W3 | Late answer against the sender's repeat | IRS's late ACK; ISS's timer | 0030 (sender waits out arrivals), 0036/0054 make answers later on purpose | **ADR-0068 answers late (`max(now, due)`)**; the sender's wait is fixed |
| W4 | Early answer from a wrong burst end | IRS's inferred deadline | 0040, 0041, 0045, 0046, 0047 f3/f4, 0061, 0065, 0067 | closed by ADR-0068 for decoded statements |
| W5 | Unsolicited turn request against the holder's next burst or poll | IRS on quiet; ISS on its own timers | 0027 §5 (holder waits out arrivals) | accepted race (0027 §6, 0030 §3); **kept by ADR-0068** |
| W6 | Lost TAKEN: both believe they hold the turn | both ISS | 0047 inference from trusted data | kept by ADR-0068 |
| W7 | Lost TURN, or a take-back after the other took it | both ISS | 0023 tie-break, 0029, 0034 | kept by ADR-0068 |
| W8 | Turn taken on a phantom: both IRS | nobody keys (deadlock) | 0063 | until the link timeout when the give-back ACK is lost |
| W9 | `Timer::Resend` backoff against an announced answer | ISS | — | introduced by ADR-0068's implementation |
| W10 | Receiving station's DISC against the next burst | IRS on its own initiative | 0023, 0042 | |
| W11 | A left session answered with DISC | idle station | 0052, 0065 (waits out arrivals) | |
| W12 | Morse identifier against the other's next frame | either | 0022, 0055, 0068 §2.5 | 0022 §3 open; port puts the identifier ahead of ACKs |
| W13 | Keepalive poll against a turn request | both on quiet timers | — | accepted race (0027 §6) |
| W14 | DISC retry against DISC_ACK and identifier | leaving ISS | 0022, 0053 busy fix | |
| W15 | A wait sized for the other family | ISS retry | 0016 §2.4, 0028, 0032, 0034, 0059 | each closes one case |
| W16 | An ended session's identifier against the new session's first move | the called-again station | 0055 §4 | |

Count: **sixteen** places where a station keys on its own belief about the other. ADR-0068
closes W4 for frames whose count was read. It adds W9 and keeps the rest. Each of the others
was patched for the case that was seen, not for the class.
