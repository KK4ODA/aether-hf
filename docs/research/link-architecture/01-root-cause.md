# Why collisions persist: the root-cause report

Every finding below is labelled with its strength:

* **Confirmed** findings are backed by a measurement in this review's simulator campaign, a
  TLC counterexample, or code read at `7546df3`.
* **Hypotheses** are consistent with the evidence but not tested against the scenario harness
  or the air.

The numbers come from [05-experiments.md](05-experiments.md). The timing classes are
defined there in §3.

## 1. The answer, first

**ADR-0068 contracts *when a receiver answers a burst it read*. It contracts nothing about
*who may start a transmission*, and nothing about *how late* an answer may go.** Three
mechanisms survive it. Each lets both stations key on their own beliefs:

1. **The station that does not hold the turn still initiates.** The turn request
   (ADR-0027, on in every session since ADR-0044) is keyed after a quiet interval, on the
   receiver's own timer. The holder's next burst or poll is keyed on *its* own timer. In the
   simulator's field profile, requests are **0.39 overlaps per session, half of ADR-0068's
   total**. They are *more* frequent than under protocol 7 (0.23): exact answer timing leaves
   less margin around the request. **Confirmed.**
2. **Late answers are sent late.** The model computes `self._arm("ack", max(0.0, due − now))`,
   so an answer whose decode arrived after its due time goes immediately. The port does the
   same. The daemon then holds it further: the answer gap (ADR-0036/0054, up to 2 s), the
   identifier, the busy waits, and the CAT thread (up to 1.2 s). The sender's wait is fixed
   at the stated slot and has expired, so it repeats. Late-answer overlaps are **0.39 per
   session**, the other half of the total. **Confirmed** in the simulator. TLC finds the same
   in `ImplicitTurn_oneway_late` and `GrantedSlots_late`.
3. **Authority moves by belief, and a lost frame splits the belief.** OFFER/TAKEN
   (ADR-0047), TURN with its take-back (ADR-0023/0029) and the inferred taken turn
   (`_turn_taken_unread`) each change one station's role on evidence the other station may
   never see. TLC finds two ISS keying together in 18 steps even with **perfect carrier sense,
   no phantoms and every late answer forfeited** (`ImplicitTurn_forfeit_perfect`). It also
   finds both stations IRS, so nobody acts until the link timeout (`ImplicitTurn_deadlock`).
   In the simulator, the `handover-lost` scenario gives ADR-0068 1.25 overlaps per run.
   **Confirmed.**

The deeper statement: **in Aether, transmission authority is each station's private belief,
and each station may act on its own belief at times of its own choosing.** Any such protocol
collides whenever one frame's loss changes one belief and not the other. No message exchange
can prevent that. This is the two-generals result [R33]: agreement on who may transmit cannot
be guaranteed over a lossy channel, only bounded in time. ADR-0068 moved one timing decision
from inference to statement, and that part is correct and worth keeping. It left authority
where it was.

The design that follows: give authority to one station for the whole session. Let the other
key only in a bounded slot defined by a frame it decoded in time, and refuse to key late.
That is [granted slots](04-formal-spec.md).

## 2. Method

* **The audit** read the timing and authority ADRs (0012 … 0068) as evidence, not as the spec.
  It read the model engine (3 038 lines), the port (6 165 lines, 4 327 of them non-test), the
  daemon's gating and the harness. Every rule was tagged by the evidence it keys on and the
  decision it changes.
* **An adversarial simulator** (`research/link_arch/advsim.py`) drives unmodified engines.
  Protocol 7 and ADR-0068 run as the model's `LinkEngine`; the prototypes run as their own
  engines.
  * It uses real PHY timing and the calibrated fading pipe (P9-6).
  * It adds what `TwoStationSim` leaves out:
    * keying latency and jitter;
    * decode latency with a tail;
    * the daemon's holds (gate, answer gap);
    * VOX hold and receiver recovery;
    * machine stalls;
    * phantom detections, some of them trusted;
    * outages;
    * the key-time limit.
  * Every transmission records why it was keyed (the engine's call chain and timer), what
    each engine believed about the turn, and what it answered.
  * Every overlap of two radiating transmitters is classified by mechanism
    (`classify.py`).
* **TLA+ models** (`research/link_arch/tla/`) of the stated-answer engine, the
  token-transfer candidate and granted slots, model-checked with TLC.
* **Station profiles** ([05-experiments.md](05-experiments.md) §2):
  * `ideal` is the link bench's world;
  * `field` is what the scenario harness and the logs say real stations do;
  * `hostile-c` and `hostile` are worse, and `hostile` adds daemon holds the engine is not
    told of.

## 3. Where transmission authority lives today

There is no `turn` field. Authority is `role` (ISS, IRS), plus sixteen flags that patch the
cases where `role` is uncertain:

* `turn_tries`, `turn_unread`, `unread_there`, `poll_floor`;
* `poll_offer`, `offer_out`, `offer_unheard`, `offered`;
* `took_unread`, `caller`, `stated_want`, `peer_request`;
* `bursts_since_turn`, `break_requested`, `carrier_until`, `resending`.

There are eleven engine timers:

* `Link`, `Connect`, `Ack`, `Wait`, `Keepalive`;
* `Probe`, `Request`, `Leave`, `First`, `Resend`, `Close`.

Ten of them can start a transmission.

**Initiation: who may key without being asked.**

| Station | May key on its own timer | Evidence it acts on |
|---|---|---|
| ISS | bursts, polls (keepalive), repeats (`Wait`, `Resend`), TURN, DISC | its own belief that it holds the turn |
| IRS | turn request (`Request`, 0027/0044), DISC between bursts (0023) | quiet since the last frame heard |
| either | answers (`Ack`), at `max(now, due)` | a decoded statement (0068) or an inferred end (legacy) |
| idle | DISC to a left session (0052), leave repeats | its own memory of a session it left |

**Transfer: how the role changes hands.**

| Mechanism | Explicitly acknowledged? | Inferred from an unreadable frame? |
|---|---|---|
| Acceptance (CONNECT_ACK) | by the caller's first move | 0048: an ACK of the session read as acceptance |
| TURN | no: the sender becomes IRS *on sending* | 0029 C: unreadable frames keep the offer going |
| TURN with OFFER → ACK with TAKEN (0047, 0053) | yes, the only one | yes: a trusted, undecoded data frame counts as taken |
| Take-back after `turn_retries` | no | silence decides |
| "Data heard wins" | no | any data frame clears the TURN wait (port audit S2) |
| Give-back (0063) | it *is* the evidence | repairs the inference above |

Only OFFER/TAKEN carries an explicit acknowledgement, and it too is inferred when the
acknowledgement is lost. Each recovery rule (0023's yield, 0029 A, 0034, 0063) needs one
specific frame to be readable, so each leaves a case where that frame is also lost.

**The sixteen points where both stations can believe they may key** are drawn and tabulated
in [02-sequences.md](02-sequences.md) §11. ADR-0068 closes one of them: the early answer, W4.
It adds W9, the resend backoff that is not carrier-sensed. It keeps the other fourteen.

## 4. Every collision, classified

The simulator classifies each instant of RF overlap by why each of the two transmissions was
keyed. Frequencies are overlaps per session over 1 008 sessions per arm and profile, with
95 % bootstrap intervals for totals in [05-experiments.md](05-experiments.md) §4.
"Reproducible" says whether a deterministic scenario produces the class on demand.

| Category | Mechanism (simulator class) | Protocol 7: field / hostile-c | ADR-0068: field / hostile-c | Reproducible | Field and harness evidence | Root cause | Remedy |
|---|---|---|---|---|---|---|---|
| **Both stations believe they hold the turn** | `dual-holder` | 0.001 / 0.022 | 0.000 / 0.013 | yes: `handover-lost`, `handover-ctl-lost` give 1.0–1.3 per run for both engines; TLA `ImplicitTurn_forfeit_perfect` | 0023 (KE4QCM: both held the turn ×3), 0029 (trials 116, 123), 0063 (both IRS, 90 s) | transferable authority confirmed by frames that can be lost | no transfer: one master for the session (D) |
| **Premature answer from inconsistent burst-end knowledge** | `early-answer` | 0.134 / 0.473 | 0.000 / 0.001 | yes: `late-decode`, `partial-bursts` (legacy) | 0040 (17 of 21 at ND1J), 0041, 0045 §4, 0046, 0061 (WC4Y), 0065, 0067 | the end of a burst was inferred | **ADR-0068's stated count, kept in D** |
| **Lost-acknowledgement and lost-turn recovery races** | `late answer (holder-timeout)`, `(holder-moved-on)`, `answer-answer` | 0.038 / 0.170 | 0.054 / 0.010 | yes: `ack-lost`, `chat-handover-lossy` | 0022 (DISC over DISC_ACK), 0028, 0030, 0034 | the sender repeats on its timer while an answer to its earlier frame is still due | the sender may not key before the slot's latest end; the answer is forfeited after it |
| **Timeout and retransmission against a late answer** | `late answer (held)`, `(decoded-late)` | 0.401 / 1.656 | 0.347 / 0.477 | yes: `late-decode`, `slow-computer`, `ptt-jitter` | 0036 and 0054 (answers into VOX holds, held on purpose), 0038 §3, beta.98 (CAT stall of 17.5 s), 0053 §5 | an answer is sent whenever its decode or the daemon lets it; the sender's window is fixed | **forfeit at the keying gate**, so a slot is never started late (D, R3) |
| **Unsolicited initiation by the non-holder** | `unsolicited (request)`, `(irs_disc)`, `(leave)` | 0.227 / 0.276 | 0.392 / 0.307 | random: about one session in three | 0027 §6, 0030 §3 ("idle polls over a request just begun"), 0052 | the receiving station keys on quiet, on its own timer | none inside a session: reverse data rides grants and open windows (D, R1, R12) |
| **RF overlap caused by PTT, VOX, audio buffering or turnaround** | inputs to the simulator: VOX hold, keying jitter, rx recovery. A transmission whose hold the engine did not know of appears as `late answer (held)` | — | — | yes: `ptt-jitter` (1.0 s) | KE4QCM SignaLink and FT-100 VOX (0036, 0054); FTDX10 recovery (0037: the scope's flash, not audio); ADR-0038's card-latency tail | the radio's own timing, which no protocol controls | stated turnaround in the gap; jitter beyond the slack is dropped at the gate, never radiated late. A VOX that holds longer than stated is outside every protocol's contract |
| **False detection or acquisition error** | phantoms enter the classes above through the timers they move | `phantom-storm`: 10.5 per run, 10/16 sessions complete | `phantom-storm`: 0.56 per run, 13/16 complete | yes: `phantom-storm` (30 per minute) | 0061 (a false detection inside a tone frame), 0063 (a trusted phantom took the turn), 0065, 0067 | detections, announcements and silences move deadlines and change roles | only decoded content changes protocol state (D: 0 overlaps, 16/16) |
| **Harness classification artifacts** | — | — | — | — | see §5 | the collision counter counts keyed silence, and VOX holds are counted together with protocol overlaps | count radiated overlap at each receiver, by mechanism (§5) |
| **Identifier and daemon-inserted transmissions** | not modelled | — | — | — | 0022 §3 (open), 0055 §4, port audit S9 (ADR-0068 puts the identifier *ahead* of ACKs) | the daemon keys transmissions the engine did not ask for | the identifier only inside the master's own transmissions; the slave's is appended inside its slot when it fits |

What the totals show (RF overlaps per session, then seconds of overlap per session):

| Arm | ideal | field | field-asym | hostile-c | hostile |
|---|---|---|---|---|---|
| Protocol 7 | 0.24 (0.23 s) | 0.80 (1.32 s) | 1.16 (2.27 s) | 2.60 (4.94 s) | 3.12 (5.76 s) |
| ADR-0068 | 0.18 (0.14 s) | 0.79 (1.11 s) | 0.80 (1.52 s) | 0.81 (1.52 s) | 1.50 (2.29 s) |
| ADR-0068, six frames | 0.17 (0.14 s) | 0.76 (1.05 s) | 0.80 (1.46 s) | 0.86 (1.41 s) | 1.44 (2.13 s) |
| Granted slots | 0 | 0 | 0 | 0 | 0 |
| Fixed cycle | 0 | 0 | 0 | 0 | 0.001 (outside its contract) |

ADR-0068 removes early answers. Those are long overlaps: a median of 2.0 s of RF overlap
under protocol 7 in the field profile, against 0.43 s under ADR-0068. Its overlaps shrink to
the length of a control frame. In the **field profile the count does not fall**: the removed
early answers are replaced by more turn requests and late answers. In the hostile profiles,
where early answers dominate protocol 7, the count falls by 52–69 %.

**Hypothesis.** The harness's halving of non-VOX collisions (37 → 19) is the hostile-profile
picture: the harness's stations, with their card latency and daemon holds, sit between the
simulator's `field` and `hostile-c` profiles. This is consistent, but untested. The harness
records nothing that would classify its collisions (§5).

## 5. The harness's collision counter

From reading `tools/channel_server.py` and `tools/session_matrix.py`. Measurements are in the
audit notes.

* **A collision is two stations *keyed* at once, at the transmitters.** "Keyed" includes the
  daemon's 0.1 s lead and 0.3 s tail of zeros on every transmission, and the VOX hold. Two
  guard silences overlapping count as a collision, with no minimum duration.
* **The harness places the key 0.25 s later than a real PTT line.** It shifts the card latency
  from before the burst to after it, so receiver recovery is counted from a late end.
  `tx_delay_ms` blanks only the zero lead and is a no-op at the values the scenarios use.
* **There is no "non-VOX collision" in the repository.** The VOX scenarios share one counter
  with protocol collisions. `scenario_trend.py` ignores collisions altogether.
* **Nothing records why a transmission was keyed.** No trigger timer, no role, no control
  subtype, and no common clock between the daemons' sidecars and the channel's report.
* **Three seeds a scenario detect only large changes.** The median per-scenario spread of air
  time is 24 %, so a single scenario detects a change of about 37 %. Pooled across 43
  scenarios, about 6–7 %. The 32-run comparison in the brief (11 → 4 → 6 → 7 collisions) is
  within noise of itself.
* **The link bench and the chat bench cannot see any of these classes.** `TwoStationSim` has
  no keying latency, no decode latency, no phantoms (`trusted` is always true), and no misread
  counts. RV 1/2 decode alone there. Their A/Bs measure burst-length costs only.

Remedy, in [06-recommendation.md](06-recommendation.md) step 1:

* the daemon tags each `MSG_PLAY` with burst id, frame kinds, trigger and role;
* the channel server records the *radiated* mask;
* overlaps are reported per receiver after path delay, split into guard, VOX and radiated.

## 6. The assumptions A–H

### A. Burst-stated answer timing

**Facts:**

* Stating the count removes early answers: protocol 7 has 0.134 per session in the field
  profile and 0.473 in hostile-c; ADR-0068 has 0.000 and 0.001. **This is the part of
  ADR-0068 that is right.**
* TLC proves a one-way stated exchange with forfeit safe (`ImplicitTurn_oneway`, 56 612
  states).
* Counts are exact and every frame kind has a fixed length, so any decoded frame of a burst
  gives the same end. Two stations decoding different frames of a burst do not disagree.
* A partially lost statement leaves the receiver with *no* statement. ADR-0068 as built then
  does not answer (port audit S5), which costs a slot, not an overlap.
* The ambiguity returns with "count 3 = three or more" (candidate A): a receiver that reads
  only frames saying 3 knows no end.

**What it does not prevent.** Anything not an answer to a burst it read (§1 items 1 and 3),
and an answer sent after its time (item 2).

### B. Sender timeout behaviour

**Facts:**

* A sender that repeats when its wait expires, while the receiver answers whenever it can,
  produces the largest class under ADR-0068: late answers, 0.35–0.48 per session.
* Receivers answer late in four ways:
  * a decode later than the budget (tone-floor settling, a loaded machine — beta.95 measured
    a station 0.3–27 s behind);
  * the daemon's holds (answer gap up to 2 s, identifier 5–13 s, busy waits up to 15 s);
  * CAT keying up to 1.2 s;
  * a card whose real latency differs from the assumed 0.25 s (ADR-0038).
* ADR-0068's `Timer::Resend` backoff is not extended by announcements (port audit S1), so it
  can key over an answer whose preamble was just announced.

**The rule that works**, confirmed in TLC and the simulator, has two halves:

* the sender repeats only after the *latest* moment a legitimate answer could start
  (slot + slack);
* the receiver *forfeits* any answer it cannot start by then.

Neither half alone suffices: `GrantedSlots_late` shows the first without the second, and the
model engine shows the second without the first.

### C. Channel ownership

**Facts (TLC):**

* A naive token that the grantor takes back on silence violates exclusive authority in 11
  steps.
* A lease (Gray and Cheriton) with a caller-only reclaim is exclusive, checked over 11.7
  million states, but has vacancies.
* A strictly enforced initiator/responder machine, granted slots, is exclusive with one
  invariant and no vacancy in the safety sense. The master is never more than one exchange
  from keying.

**Fact (code):** the port carries about 420 lines and 16 flags of turn-ambiguity code. ADR-0068
does not touch them.

**Conclusion.** An explicit *transferable* token buys nothing over a fixed initiator for two
stations. A strictly enforced initiator/responder machine does simplify the protocol, and it
is the recommendation.

### D. Variable versus deterministic timing

**Facts (simulator):**

* A fixed superframe (candidate C) is safe but 34–63 % slower than protocol 7 in the field
  profile, with twice the chat latency.
* It completes 0 of 16 slow-computer sessions. A fixed guard cannot adapt to a station that
  decodes late.
* Granted slots, with deterministic *rules* and variable *durations* (each slot computed from
  stated counts and fixed frame lengths), is safe at ×1.10 the completion time of protocol 7
  (field).

**Conclusion.** Deterministic rules, adaptive durations. The superframe's rigidity is its
cost, not its safety.

### E. Multiple physical-layer frame families

**Facts:**

* Five of the sixteen collision points (W15) and six ADRs (0016 §2.4, 0028, 0032, 0034, 0059,
  0062) concern a wait sized for the wrong family.
* In granted slots, the family is a field of the grant, and the slot reserves for every family
  the slave may hold (R6). Timing comes from stated counts and fixed frame lengths, so it is
  independent of the family. The family remains a robustness choice, not a source of timing
  ambiguity.

**Hypothesis.** A single always-floor control frame would cost about 8 % of air: ADR-0062
measured +8 % for floor acknowledgements from the start. The report-driven family with a floor
fallback after two silences, as in granted slots, is near the best trade, but this is
untested against alternatives.

### F. Detection versus decoding

**Facts:**

* 17 accepted timing rules key on untrusted detections. Five of them exist only to filter the
  noise that other rules let in (ADR history §2).
* In the phantom-storm scenario (30 false detections a minute at both stations):

  | Arm | Overlaps per run | Sessions completed |
  |---|---|---|
  | Protocol 7 | 10.5 | 10/16 |
  | ADR-0068 | 0.56 | 13/16 |
  | Granted slots (only decoded content changes state) | 0 | 16/16 |

**Answer.** A detection without a decode may feed the rate controller's SNR statistics
(ADR-0020) and the diagnostics, and carrier sense *outside* a session. Inside a session it
should change nothing: no deadline, no role, no burst boundary.

### G. Rate adaptation

**Facts:**

* All engines share the model's rate controller. The first-transmission rung mix of granted
  slots matches protocol 7's at 0 and 6 dB (2300 Hz, field).
* In granted slots, rate and timing are separate: a rung change alters only frame lengths,
  which are stated.
* Unsupported rates are left by a two-rung step on each silent data slot and by the hopeless
  re-encode (ADR-0038). A lost answer costs one slot, never a long silence.

**Conjecture.** The Test's "restore the working speed after the ladder" is a Test-session
issue (`top_acked_mode`, ADR-0061), orthogonal to the architecture.

### H. Complexity

**Facts (port audit):**

* About 950–1 000 lines, 22–23 % of the non-test engine, exist to infer burst ends, lost
  answers or who holds the turn.
* ADR-0068 adds about 370 lines beside the about 300 legacy lines it would retire.
* The about 420 lines of turn-ambiguity code are untouched by it.
* The granted-slots prototype is 829 lines with its docstrings, plus a 650-line core shared
  with the fixed cycle (HARQ, selective repeat, rate, control frame). It has four timers:
  `proceed`, `answer`, `link`, `connect`.

**Hypothesis.** A port of granted slots would be about 2 000–2 500 lines against the port's
4 327, mainly because the turn, request, offer and burst-end machinery has no counterpart.
That is an estimate from the prototype, not a measurement of a port.

## 7. Obsolete and overlapping rules in the code

* **ADR-0068 sits beside the rules it retires.** `stated_answers` is a switch. Both paths are
  in both engines, and the legacy path's ~300 lines go only when the switch does. In daemon
  terms, `FOLLOW_ON_*`, `answers_burst` and `fragment_of_own_tx` only matter for the legacy
  path.
* **Implementation drift from ADR-0068's text** (port audit S1–S10):
  * the sender's wait is still extended by arrivals (S7);
  * `Timer::Resend` (random backoff) and `Timer::Close` are not in the ADR;
  * ADR-0034 and ADR-0059's floor switches still run (S4);
  * trusted failed OFDM frames do not state (S6);
  * the protocol version is still 7 (S8);
  * the identifier goes ahead of ACKs (S9);
  * `offered` and related flags survive into the next session (S10).
* **A port bug, D1.** `"closed (no disc ack)"` in `engine.rs` is not the spelling
  `LEFT_UNHEARD` holds (`"closed (no DISC_ACK)"`). A Rust session that ended that way is never
  remembered as left (ADR-0052). Independent of this review: a one-line fix in the constant.
* **Repair chains** ([adr-history] dependency graph):
  * 0040 → 0041 → 0045 → 0046 → 0047 fixes 3–4 → 0061 → 0065 → 0067 (burst end);
  * 0047 → 0053 → 0066 and 0047 → 0063 (turn offer);
  * 0029 → 0034 (TURN unread);
  * 0012 → 0032 → 0033 (silence and timeout);
  * 0036 → 0054 (answer gap).

  Each was a correct response to its trace, and each traces back to inferred ends, a
  transferable turn, or a late answer.

## 8. Protocol defects and radio limits, separated

| Effect | Protocol defect? | What a protocol can do |
|---|---|---|
| Answer keyed after the sender's repeat began | **yes** | forfeit late; the sender waits out the slot |
| Two stations initiating on their own timers | **yes** | one initiator |
| Lost handover splitting the role | **yes** | no handover |
| Early answer from an inferred end | **yes** (fixed by ADR-0068) | stated counts |
| A false detection moving a deadline | **yes** | decoded content only |
| VOX hold longer than stated | no: the radio's | state it at connection, learn it (ADR-0054); beyond that, a lost frame, never a protocol overlap |
| CAT or card latency beyond the slack | no: the computer's | drop at the gate (forfeit), which costs a slot, not an overlap |
| A decode later than every gap level | no: the computer's | gap levels (R8); beyond the highest, the session runs at the forfeit rate and the master's retries end it |
| The scope's flash after unkeying (ADR-0037) | no, and not real audio | nothing |
| Guard-silence overlaps in the harness | no: a measurement artefact | count radiated overlap |

[R33]: see [03-alternatives.md](03-alternatives.md) §4.
[adr-history]: the review's ADR synthesis, summarised in §3 and §7 above.
