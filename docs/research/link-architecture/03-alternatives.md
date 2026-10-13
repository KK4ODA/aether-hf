# Alternative link architectures

This report has three parts:

* how established half-duplex ARQ protocols decide who transmits next (§1);
* four candidate architectures for Aether that differ in their synchronisation and
  channel-access model, not in parameters (§2);
* how the candidates compare (§3).

Citations [Rn] refer to the reference list in §4. Labels:

* **[D]** a published or normative property;
* **[C]** read from the named source code;
* **[S]** a secondary source;
* **[O]** an observation, not a specification;
* **[I]** this review's own inference or proposal.

VARA HF is described only from its published specification. Nothing here is inferred about its
internals.

## 1. How other protocols decide who transmits next

### 1.1 Five authority models

| Model | Who may key, and when | Examples |
|---|---|---|
| **A. Fixed synchronous cycle** | the master's clock owns the timing; the slave answers at a fixed offset, even when it heard nothing (the flywheel) | AMTOR/SITOR: 450 ms cycle, constant `tE` [R1, D]. PACTOR I–IV: 1.25/3.75 s cycles [R2, R4, D]. G-TOR: 2.40 s [R10, S]. CLOVER: 19.488 s [R11, S]. MIL-STD-188-141B 3G-HDL: `THDL(n)` slots [R16, D] |
| **B. Asynchronous stop-and-wait, sender as timing master** | the receiver answers when it decodes; the sender repeats on a timer | ARDOP: "the ISS is the master of frame timing" [R6, D]. WINMOR [R12, D]. FreeDATA [R22, C]. VARA HF 2.0's ISS/IRS with fixed-length frames [R9, D] |
| **C. Stated duration** | every frame says how long the transmission still lasts; nobody keys before then | STANAG 5066's EOT field, "shall not be violated or contradicted" [R14, D]. 802.11's Duration and NAV [R26, S]. Mercury's frames-left [R24, D] |
| **D. Explicit token, timeout fallback** | a token message moves; a timer reclaims a lost one | CCSDS Proximity-1 "Send Token" with `Receive_Duration` back-up [R19, D]. IEEE 802.4 [R29, D]. WTRP / STANAG 5066 Annex L [R32, D] |
| **E. Single master, slave in slots** | one station schedules both directions; the other keys only when addressed | Bluetooth BR/EDR [R36, S]. Mercury's 2026 "carousel turns" [R24, D, "on-air validation pending"]. 3G-HDL inside a transfer [R16, D] |

Aether's protocol 7 is model B with turn handovers added. ADR-0068 adds model C to the
receiver's answer only. Authority to *initiate* stays model B, on both sides.

### 1.2 The findings that bear on Aether

1. **Only model B lets a missed frame end or a false detection change when a receiver keys**
   [I, from R1, R6, R14, R16, R24]. Mercury's 2026 design notes reached the same conclusion
   from its own collisions [R24, D]: "the residual collisions in the sim, with NAV headers on,
   come down to two such timers meeting... Every fix so far has added a timer rule that
   predicts the peer's timers... This design removes the cause: only one station ever keys on
   a timer." ARDOP has the same structure: the ISS's repeat timer and the IRS's BREAK repeats
   are independent "heard nothing" timers [R6, C; I].

2. **No message exchange makes a handover loss-proof** [R33, R34]. HF protocols handle a moving
   token in one of three ways:
   * a three-step exchange the old holder must complete: AMTOR's CS3 → β α β → RQ [R1, D];
   * the new holder repeating its claim while the old one purges its queue "to prevent a
     continual deadlock": ARDOP's BREAK [R6, D];
   * never moving timing authority at all: AMTOR's master clock "regardless of which station
     ... is the ISS or IRS" [R1, D], 3G-HDL, Bluetooth, Mercury.

   Exclusivity is achievable only in *time*:
   * a lease with bounded clock drift [R35, D];
   * or a schedule fixed in advance.

3. **Stated duration has a normative form worth copying** [R14, D]:
   * rounded up;
   * monotonically decreasing within a transmission;
   * never contradicted by later frames;
   * the sender stops at zero;
   * others never key before it expires.

   Isode measured the cost of STANAG's 0.5 s resolution and proposes a block count [R15, S].
   Counting frames, as ADR-0068 and Mercury do, avoids that cost only because every frame kind
   has a fixed air time.

4. **A late decode should be data, not a timing reference.** Mercury's rule R4: "S answers only
   if the decode is at most D late (otherwise the frame is data, not a timing reference)"
   [R24, D]. AMTOR and PACTOR need no such rule because the slot comes whether or not the
   frame decoded [R1, R2, D]. ADR-0068 as built answers late (`max(now, due)`). That is the
   opposite.

5. **The flywheel rule** (3G-HDL): "If either station fails to receive a PDU at the expected
   time, it sends its own next outgoing PDU at the same time as if the incoming PDU had been
   received successfully" [R16, D]. It makes timing independent of loss, at the cost of a
   fixed cycle.

6. **Rate adaptation is decided by the receiver in nearly every protocol**, one step at a time,
   signalled in the acknowledgement:
   * PACTOR CS4/CS5 [R4, D];
   * G-TOR [R10, S];
   * CLOVER [R11, S];
   * VARA ACK2/ACK3 [R9, D];
   * STANAG 5066 Ed3 [R14, D].

   Aether already does this. Nothing in the comparison below turns on rate adaptation.

7. **PTT latency is a known budget item**:
   * VARA's specification promises timing that "will handle up to 100 ms delay for the PTT
     switching" [R9, D];
   * ardopcf asks for under 50 ms and fixed a bug where PTT was held too long after TX
     [R8, D];
   * FreeDATA's 3.5 s timeout carries the comment "3 seconds seems to be too small for radios
     with a too slow PTT toggle time" [R22, C].

   An Aether daemon behind a CAT thread can add up to 1.2 s (`ThreadedPtt`'s bound, beta.98).
   The engine is not told.

### 1.3 Mechanisms named in the brief, and where each lands

| Mechanism | Precedent | Used by |
|---|---|---|
| Explicit token passing | Proximity-1 [R19], 802.4 [R29], WTRP [R32] | B |
| Deterministic cycle with guarded slots | AMTOR [R1], PACTOR [R2, R4], 3G-HDL [R16] | C |
| Selective repeat with windows | STANAG 5066 [R14], WINMOR bitmap [R12] | all (kept from the model) |
| Adaptive bursts with reserved answer opportunities | 802.11 TXOP [R28], 3G-HDL n packets [R16] | D (grants), C (gears) |
| Sequence-numbered grants, idempotent retransmission | 802.4 [R29], WTRP generation numbers [R32] | B (generation), D (air-time attribution) |
| Receiver-driven scheduling and credits | Mercury carousel POLL [R23] | D: the slave's queued count sizes its grant |
| Robust independently decodable control frames | PACTOR CS in DBPSK [R4], VARA's FSK ACK [R9] | all: the tone-floor control frame |
| Hybrid deterministic/adaptive | 3G-HDL set-up then fixed slots [R16] | D |
| Session epochs, resynchronisation | AMTOR rephasing [R1], WTRP generation [R32] | B (lease generation), C (relock) |
| Selective acknowledgement and HARQ feedback piggybacking | STANAG DATA-ACK [R14], 3G-HDL IR [R16] | all; D also piggybacks reverse data on every answer |

## 2. Four candidates

All four keep the PHY, the frame families, FEC, HARQ (RV sequence, soft combining, slot
inference, re-encoding), the rate controller, selective repeat and the connect body. They
differ in **who may key, when, and on what evidence**.

### Candidate A — refined ADR-0068

**Synchronisation.** The receiver answers at the time the burst states (end + count × slot +
turnaround). Authority to initiate is implicit and belongs to whoever believes it is ISS.

**The amendment:**

* bursts of up to six frames;
* a count of 3 means "three or more follow". A frame that says 3 states no end; a later frame
  does.
* the failed-burst acknowledgement experiment is dropped;
* the working rung is restored after the Test ladder.

**Prototype:** `engines/stated6.py`. It is the model engine with `stated_answers` on and the
burst capacity raised.

**What it keeps:**

* ISS/IRS beliefs;
* OFFER/TAKEN (0047);
* the unsolicited turn request (0027/0044);
* TURN and its take-back;
* the late answer (`max(now, due)`);
* the resend backoff (`Timer::Resend`);
* the sender's arrival extension (0030).

**Analysis.** Six-frame bursts recover some of the four-frame cap's cost. The "three or more"
count reintroduces the ambiguity ADR-0068 removed: a receiver that reads only frames saying 3
knows no end, and the burst is not answered (port audit S5). That costs a slot, but not an
overlap. Restoring the working rung after the ladder is a Test-session fix
(`top_acked_mode`, ADR-0061) and is orthogonal to the link architecture. None of it touches
the mechanisms that still overlap (§3 of [01-root-cause.md](01-root-cause.md)). Measured in
[05-experiments.md](05-experiments.md): somewhat faster than ADR-0068, the same overlaps.

### Candidate B — explicit transmit ownership (token with lease)

**Synchronisation.** One token. Its holder may initiate: data, or a GRANT handing the token
over. The other station only answers.

**First design, naive** (`TokenTransfer.tla`, `Design = "naive"`):

* the grantee holds the token from the GRANT on;
* its first data frame proves it took the token;
* the grantor's answer to that frame confirms it;
* a grantor that hears nothing for `W` takes the token back.

TLC finds the obvious counterexample in 11 steps. The GRANT is decoded, the grantee's frames
are lost, and the grantor takes the token back while the grantee holds it. **Exclusive
authority is violated.** Messages cannot fix this [R33].

**Second design, a lease** (Gray and Cheriton [R35]):

* Authority is a lease that **only the other station renews**, by answering.
* A station may initiate only while `now + 1 ≤ renewed + Renew`.
* An unconfirmed grantee holds `grant end + Lease`.
* Taking back a lost grant restores no lease.
* The only path to authority without a renewal is the **caller's** reclaim. It is allowed once
  every renewal the caller itself gave can have expired at the other end:
  `Renew + LMax + 1` after its own last frame, and `Lease` after its last GRANT.
* The called station never reclaims. This is the fixed tie-break, as in AMTOR and 802.5's
  highest address [R31].

TLC verifies ExclusiveAuthority, NoOverlap and BoundedVacancy over 11.7 million distinct
states, with loss runs of up to three frames. `SomeoneMay` fails. That is the designed price:
after losses there are intervals in which *nobody* may initiate, until the caller's reclaim.

**Can ownership be exclusive during failures?** Yes, but only in time, under stated bounds:

* decode latency ≤ `LMax`;
* clock drift folded into the lease margin.

It cannot be exclusive by agreement. The safe recovery procedure is a vacancy, then the
caller's reclaim.

**Not built as an engine.** Candidate D is the degenerate case of B in which the token never
moves:

* the master holds a lease renewed by its own schedule;
* the slave holds a one-slot lease per grant, which expires at `start + slack`.

B's handover machinery would add these over D:

* GRANT, CONFIRM, renewals, the reclaim and vacancy timers;
* three message kinds;
* two timers per side.

It would buy what D gets by granting reverse data inside every exchange: the slave sends its
data without owning anything. The one thing B gives that D does not is a slave that can send a
long run of data with no per-exchange header. D's grants of up to six frames per exchange make
that difference small. **[I]**: B is dominated by D for two stations. It is the right model
only if a third station must hold the channel (Phase 8 multi-station, not in scope).

### Candidate C — deterministic ARQ cycle

**Synchronisation.** A fixed superframe per *gear* (the data family and the control family).
Cycle `k` starts at `t0 + k·T` with the master's header and data. It ends with the slave's
transmission, whose answer always ends at the same point. The `D` data frames of a cycle are
shared: the header grants the slave `n` of them. The master keys at every cycle start
whatever it heard. The slave answers at its fixed point in every cycle, **blind** ("nothing
heard") when it decoded nothing, while its lock is fresh. This is the 3G-HDL flywheel [R16].

**Gear changes are two-phase.** The change is announced in a header, confirmed in the answer,
and made at the next cycle only if the master read the confirmation. This is the
GlobeWireless rule for a cycle change [R4, D].

**Prototype:** `engines/fc.py`.

**Analysis.** Zero overlaps, as expected of a fixed schedule. It is slow for a structural
reason: a fixed cycle must be sized for the slowest legitimate case (decode latency, keying
jitter and turnaround at their worst) in *every* cycle. A cycle can never end early.

Variable bursts and adaptive modulation do coexist with it, through gears. Every gear change
costs a confirmed exchange, and a lost confirmation keeps the old gear. The rigidity shows in:

* **chat**: twice the line latency;
* **the slow-computer scenario**: 0 of 16 complete. A slave whose decodes arrive after the
  fixed guard misses every cycle, and nothing in the protocol can widen the guard without a
  gear change it cannot complete.

### Candidate D — hybrid: granted slots

**Synchronisation.** One immovable timing master, the caller. Each master transmission grants
the slave exactly one slot, defined by the exact count of what it carries. The slave keys only
in a slot granted by a header it decoded in time. Otherwise it forfeits, and the keying gate
enforces this.

**What adapts:**

* grant sizes, from the slave's queue;
* burst lengths, up to six frames;
* rungs;
* the gap level, from measured decode lateness;
* open windows, on an idle link.

Control and recovery are one rule: the master keys again after the stated end, or after the
slot's latest end. It is the PACTOR and 3G-HDL discipline for the slave, with the master's
cycle adapted to what it carries and what it heard. It is close to Mercury's carousel turns
[R24]. Granted slots was derived independently in this review, from the TLA+ counterexamples
of [04-formal-spec.md](04-formal-spec.md) §8.

The differences from Mercury:

* selective repeat with HARQ instead of erasure coding;
* sender-side grants instead of receiver polls;
* a gap level for slow decoders;
* the forfeit enforced at the keying gate.

**Prototype:** `engines/gs.py`.

**Specification:** [04-formal-spec.md](04-formal-spec.md).

### Considered and not pursued

* **p-persistent CSMA inside the session** (AX.25/KISS [R30]). Collisions are its recovery
  mechanism by design: "taken care of by the retry nature of the T1 timer" [R30, D]. On a
  half-duplex HF link the detector misses frames below its threshold (ADR-0040's 22 control
  frames never detected), so sensing cannot be the safety mechanism. It remains the right rule
  *outside* sessions (beacons, datagrams, calls).
* **Mercury's receiver-polled carousel with erasure coding** [R23]. It would replace HARQ and
  selective repeat, which the brief asks to keep. Its turn discipline is what D adopts.
* **Underwater "juggling" ARQ** [R37]. It needs known propagation and full scheduling. HF
  propagation is known, but decode latency is not, so it reduces to C.

## 3. Comparison matrix

Evidence labels:

* **[M]** measured in the simulator campaign ([05-experiments.md](05-experiments.md));
* **[F]** model-checked ([04-formal-spec.md](04-formal-spec.md) §8);
* **[H]** harness or field evidence from the repository's ADRs;
* **[A]** analysis;
* **[?]** hypothesis, untested.

| Dimension | A: ADR-0068 refined | B: ownership (lease) | C: scheduled cycle | D: granted slots |
|---|---|---|---|---|
| Protocol safety (both radiate) | overlaps remain: 0.76 [0.70, 0.82] per session in the field profile, 0.86 in hostile-c, the same classes as ADR-0068 [M]; ImplicitTurn traces apply [F] | exclusive under bounded latency [F]; overlaps [?] not built | 0 overlaps in 25 200 runs bar one outside the contract (jitter beyond the slack) [M] | 0 RF overlaps in 5 040 runs and 14 adversarial scenarios [M]; NoOverlap holds [F] |
| Recovery from lost ACK | resend after the slot; a late ACK still keys (`max(now, due)`) [M, F: `ImplicitTurn_late`] | as D within a lease [A] | the flywheel: the next cycle repeats [M] | one slot, the next header repeats the grant; a late answer is forfeited [M, F] |
| Recovery from lost TURN | both-ISS and both-IRS traces [F]; dual-holder overlaps [M] | vacancy, then the caller's reclaim (bounded) [F] | no turn [A] | no turn; reverse data rides grants [A, M] |
| Session completion | 97.9 % field, 94.5 % hostile; failures are link timeouts at both ends [M] | [?] | 99.8 % field, 98.6 % hostile [M] | 100.0 % field, 99.8 % hostile; the 10 failures in 5 040 were still delivering at the horizon [M] |
| Predicted throughput (bulk / exchange / chat) | ×1.105 / ×1.133 / ×1.009 [M] | ≈ D less the handover exchanges [A, ?] | ×1.338 / ×1.626 / ×1.479 [M] | ×1.103 / ×1.101 / ×1.035; against ADR-0068 ×0.96 / ×0.91 / ×1.02 [M] |
| Low-SNR robustness (−6…0 dB bulk goodput vs protocol 7) | 78–86 % of protocol 7 [M] | [?] | 76–84 % [M] | 86–94 % [M] |
| Hardware sensitivity (PTT jitter, VOX, slow decoders) | late answers collide: `late answer (held)` is its largest class [M]; slow-computer 15/16 at 17.6 overlaps per session [M] | needs `LMax` bounded [F] | fixed guard: slow-computer 0/16 [M] | jitter beyond the slack and late decodes forfeit, never collide [M, F]; gap levels adapt; slow-computer 16/16, no overlap [M] |
| Implementation complexity | the current engine plus 6-frame handling: ~4 300 non-test lines in the port, 11 timers, 16 turn flags [H] | D plus GRANT/CONFIRM, renewals, reclaim [A] | prototype 610 lines [M] | prototype 820 lines, on a 650-line shared core [M]; three engine timers (proceed, answer, link) plus connect [A] |
| Verification difficulty | 16 places where a station keys on its own belief ([02-sequences.md](02-sequences.md) §11); no compact invariant [A] | lease arithmetic; 11.7 M states to check two stations [F] | simplest to verify [A] | one invariant (I1) with a five-step argument; 8–104 k states [F] |
| Migration effort | smallest: a switch already in the port [H] | a new engine [A] | a new engine and new waits everywhere [A] | a new engine (protocol 8), the daemon's gate gains `not_after`, the identifier moves; PHY, HARQ and rate unchanged [A] |

Time ratios are paired geometric means against protocol 7 in the field profile (bulk / exchange / chat). Low-SNR robustness is the median bulk goodput at −6 and 0 dB on both airs, as a share of protocol 7's. Details and intervals are in [05-experiments.md](05-experiments.md).

## 4. References

[R1] ITU-R M.476-5 (1995), *Direct-printing telegraph equipment in the maritime mobile service*.
https://www.itu.int/dms_pubrec/itu-r/rec/m/R-REC-M.476-5-199510-I!!PDF-E.pdf

[R2] NB6Z, *PACTOR: System details, operation, memory ARQ* (the WAA/DL6MAA description).
https://myplace.frontier.com/~nb6z/pactor.htm

[R4] ITU-R M.1798-2 (2021), Annex 2 (DATAPLEX/OFDM) and Annex 3 (*The PACTOR-III protocol*,
Helfert and Rink, SCS). https://www.itu.int/dms_pubrec/itu-r/rec/m/R-REC-M.1798-2-202102-I!!PDF-E.pdf

[R6] R. Muething KN6KB, *ARDOP Specification*, rev. 0.3.1 (2015) and 2.0 (2017).
https://raw.githubusercontent.com/pflarue/ardop/master/docs/refs/ARDOP_Specification_20171127.pdf

[R8] ardopcf `changelog.md` and `docs/Troubleshooting.md`, https://github.com/pflarue/ardop
(`e27af04`).

[R9] J. A. Nieto Ros EA5HVK, *VARA HF Modem Specification*, rev. 2.0.0 (2018), mirror.
https://sbcara.org/wp-content/uploads/2025/08/VARA-Specification.pdf

[R10] ARRL, *G-TOR*. https://www.arrl.org/g-tor

[R11] ARRL, *CLOVER*. https://www.arrl.org/clover

[R12] R. Muething KN6KB, *WINMOR Protocol Specification*.
https://centennial-qp.arrl.org/files/file/WINMOR.pdf

[R14] NATO STANAG 5066 Annex C v1.2, *Data Transfer Sublayer* (mirror).
http://www.n2ckh.com/MARS_ALE_FORUM/S5066AnnexC_v1_2.pdf

[R15] Isode, *Reducing Turnaround Times in STANAG 5066*.
https://www.isode.com/whitepaper/reducing-turnaround-times-in-stanag-5066/

[R16] MIL-STD-188-141B, Appendix C (3G HDL/LDL).
https://hflink.com/standards/ALE_standard_188_141B.pdf

[R19] CCSDS 211.0-B-6, *Proximity-1 Space Link Protocol — Data Link Layer* (2020).
https://public.ccsds.org/Pubs/211x0b6e1.pdf

[R22] FreeDATA, https://github.com/DJ2LS/FreeDATA (`2b3f037`).

[R23] Mercury, `docs/CAROUSEL-ARQ.md`. https://github.com/Rhizomatica/mercury

[R24] Mercury, `docs/CAROUSEL-TURNS.md` and `specs/turns/CarouselTurns.tla` (`e9f0072`).
https://github.com/Rhizomatica/mercury/blob/mercuryv2/docs/CAROUSEL-TURNS.md

[R26] *802.11 RTS/CTS Duration rules* (summary). https://mrncciew.com/2014/10/26/cwap-802-11-ctrl-rtscts/

[R28] *IEEE 802.11e-2005* (TXOP). https://en.wikipedia.org/wiki/IEEE_802.11e-2005

[R29] Motorola, *MC68824 Token Bus Controller User's Manual* (1987), Appendix A.
https://bitsavers.mirrorservice.org/components/motorola/68000/MC68824_Token_Bus_Controller_1987.pdf

[R30] TAPR, *AX.25 v2.2* (1998). https://www.ax25.net/AX25.2.2-Jul%2098-2.pdf

[R31] *Token Ring* (active monitor). https://en.wikipedia.org/wiki/Token_Ring

[R32] M. Ergen et al., *WTRP — Wireless Token Ring Protocol*, IEEE Trans. Veh. Technol. 53(6),
2004. https://ptolemy.berkeley.edu/projects/ofdm/ergen/docs/ergen_wtrp_ieeetransonvt.pdf

[R33] *Two Generals' Problem* (Akkoyunlu, Ekanadham, Huber 1975; Gray 1978).
https://en.wikipedia.org/wiki/Two_Generals%27_Problem

[R34] J. Y. Halpern and Y. Moses, *Knowledge and Common Knowledge in a Distributed Environment*,
JACM 37(3), 1990. https://arxiv.org/abs/cs/0006009

[R35] C. G. Gray and D. R. Cheriton, *Leases*, SOSP 1989.
https://pages.cs.wisc.edu/~remzi/Classes/739/Fall2015/Papers/leases89.pdf

[R36] J. Mišić and V. Mišić, Bluetooth chapter in Boukerche (ed.), *Handbook of Algorithms for
Wireless Networking and Mobile Computing*.

[R37] B. Tomasi et al., *Cross-layer analysis ... of incremental redundancy hybrid ARQ over
underwater acoustic channels*, Ad Hoc Networks, 2014.
https://www.dei.unipd.it/~badia/papers/2014_ADHOC.pdf
