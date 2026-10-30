# ADR-0069: Granted slots — one master per session; the other station keys only in a slot it was granted

**Status:** accepted for building, 2026-10-13. Written after the architecture review in
`docs/research/link-architecture/`. The author's answers: granted slots is the direction, with
ADR-0068 kept only for its exact frame counts (§2.1); the cost against protocol 7 (about 10 %
of completion time, 9–13 % of bulk goodput at high SNR) is the price of no collisions; and
nothing ships before it — it is built the best way possible, not the fastest. It supersedes
ADR-0068's turn model and answer timing (its §2.2–2.4). It is built in the gated steps of §5, and
judged on the harness and the air before any build reaches a tester.
**Step 3 closed, 2026-10-14** (`docs/research/link-architecture/10-step-3.md` §6): the rerun's
one overlap was a count read short off a frame HARQ rescued, and its rule is in; the one PACTOR
run short of protocol 7 the author ruled the accepted cost. By the author's decision the first
build of it, `v0.2.0-beta.106`, goes to the beta channel for Step 4, the air with the testers.
**Step 5 of the review's plan, the retirement, done 2026-10-20** (`06-recommendation.md` §4),
protocol 8 having shipped in beta.106 and held through beta.114: protocol 7's engine is deleted in
both suites with its control frame, its simulator and its tests, the daemon's paths that served
it are gone, the review's research code is frozen at `b0d422c`, and the air-interface
specification's link layer (§7) describes protocol 9.

## 1. Context

ADR-0068 replaced inferred burst ends with stated ones. The review measured what that bought
and what it left. Details are in `docs/research/link-architecture/01-root-cause.md`.

* **It ends early answers.** In the review's simulator, early answers fall from 0.134 a
  session (field profile) and 0.473 (hostile) to zero.
* **It leaves the overlap count unchanged in the field profile.** 0.80 → 0.79 a session. The
  removed early answers are replaced by two classes it does not touch:
  * **unsolicited turn requests**, 0.39 a session. The receiving station keys on quiet, on its
    own timer (ADR-0027/0044).
  * **late answers**, 0.39 a session. Answers go at `max(now, due)`, after the daemon's holds or
    a late decode, into the sender's repeat.
* **Its handover can split authority.** TLC finds two ISS keying together with perfect carrier
  sense and every late answer forfeited (`ImplicitTurn_forfeit_perfect`, 18 steps). It also
  finds both stations IRS until the link times out (`ImplicitTurn_deadlock`). A lost
  ACK|TAKEN is enough.
* **It costs time.** ×1.15 / ×1.22 the completion time of protocol 7 (field, bulk / exchange).
  The harness measured messages −17 % and files −26 %.

These share one cause. **The right to transmit is each station's private belief, and each
station acts on it at times of its own choosing.** No exchange of messages over a lossy channel
makes two stations agree on who may transmit (the two-generals result). Exclusivity can only be
bounded in time.

## 2. Decision

Protocol 8 is **granted slots**. The full specification is
`docs/research/link-architecture/04-formal-spec.md`.

### 2.1 One master

The caller is the master for the whole session. It is the only station that keys on its own
initiative. The called station is the slave.

### 2.2 Every master transmission grants one slot

A master transmission is a header control frame and up to six data frames. Every data frame
states exactly how many follow (ADR-0068 §2.1). The header:

* acknowledges the slave's data;
* recommends the slave's rung;
* states its own count;
* grants one slot: an answer; an answer and *n* data frames; or an open window.

The slot starts at `end + gap(family) + level` and lasts at most the grant. The gap is:

```
gap(family) = decode budget + keying latency + guard + the other's declared turnaround
```

### 2.3 The slave keys only in a slot granted by a frame it decoded in time

This is the **forfeit** rule. A grant decoded too late to start the slot on time is not
answered. The daemon's keying gate drops any transmission whose radiation would start after
`slot start + slack`, whatever held it.

Values the master can change (level, grant size, answer family) are taken only from the
current transmission's header. Without that header, the slave uses the values that fit every
slot the master can have granted: level 0, answer only, its last family. The master's slot
reserves for the slowest answer family the slave may hold.

### 2.4 The master keys again only when the slot can no longer be in use

That is:

* after the end a decoded slave frame of this slot stated, plus the decode budget;
* or, if it decoded nothing, after the slot's latest end plus the decode budget.

A slave frame belongs to the slot if it began after the master's transmission ended.
An older frame's content is kept, and its timing is ignored.

### 2.5 Only decoded content changes protocol state

A detection, an announcement or a silence moves no deadline and changes no role. The single
use of silence is that the master closes a slot it heard nothing in. A silent data slot steps
the rate down. Two silent slots in a row put control frames on the floor and widen the gap
level.

### 2.6 What goes

The following are removed:

* the turn (ISS/IRS);
* TURN, OFFER/TAKEN, POLL and the turn request;
* the resend backoff;
* every arrival-driven deadline;
* the follow-on heed;
* the inferred-acceptance and inferred-turn rules;
* the floor switches of ADR-0034/0059. The answer family is a field of the grant.

Reverse data rides the grants: the slave states its queue in every answer. On an idle link, an
open window bounds the slave's latency.

### 2.7 What stays

The PHY, the frame families, FEC, HARQ (RV sequence, combining, slot inference, re-encoding),
selective repeat, the rate controller, the connect body and the exact counts.

The connect request grants the acceptance's slot. Its level rises every second try, so that a
slow called station is reached.

### 2.8 The control frame

Seven bytes, one per transmission:

* kind and session;
* count, the data frames' family, and the gap level;
* the answer family, and a flag (open window / leave);
* selective acknowledgement (base and a 16-bit bitmap);
* SNR heard;
* the recommended rung;
* frames granted, or frames queued.

The header is left out when it would change nothing and at most four data frames follow.

## 3. Alternatives considered

Measured in `05-experiments.md`; formal results in `04-formal-spec.md` §8.

* **ADR-0068 as built.** Rejected for the reasons in §1.
* **ADR-0068 with six-frame bursts, count 3 meaning "three or more".** Rejected as the end
  state.
  * Gains: ×1.11 / ×1.13 against protocol 7, against ×1.15 / ×1.22 for ADR-0068.
  * Overlaps unchanged: 0.76 a session, by the same mechanisms.
  * "Three or more" brings back an unstated end.
* **An explicit transferable token with a lease** (Gray and Cheriton). Model-checked exclusive
  under bounded latency, over 11.7 M states. It has vacancies, and needs GRANT, CONFIRM,
  renewals and a caller-only reclaim. For two stations it is dominated by never moving timing
  authority at all.
* **A fixed superframe** with blind answers and two-phase gear changes, as in
  PACTOR / 3G-HDL. Safe, but:
  * ×1.34 / ×1.63 / ×1.48 against protocol 7;
  * twice the chat latency;
  * 0/16 sessions on a slow decoder: a fixed guard cannot widen.
* **Carrier sense inside the session** (CSMA). Detectors miss frames below threshold (ADR-0040),
  so sensing cannot carry safety. It stays outside sessions.

## 4. Consequences

* **Safety.**
  * One invariant: the transmitters never radiate together. It has a five-step argument
    under stated bounds: fixed frame lengths, the gate's start bound, guard ≥ clock and
    propagation error, and transmissions ending where they state.
  * TLC checks it (`GrantedSlots_safe`, `_safe_big`); its mutants without the forfeit or the
    attribution fail.
  * The prototype radiated together in none of 5 040 randomised and 224 adversarial sessions.
* **Completion.** 100.0 % in the field profile and 99.8 % hostile, against 98.6 % and 95.3 %
  for ADR-0068. All 224 adversarial sessions completed, against 202.
* **Time.** It is ×1.10 / ×1.10 / ×1.04 against protocol 7 in the field profile (bulk,
  exchange, chat), and faster than ADR-0068 on bulk and exchange. It gives up 9–13 % of bulk
  goodput at +12 dB, to the per-transmission header and the gap reserved for the slowest
  legitimate decode. A line typed at the slave on an idle link waits up to an open window: p90
  of the slowest line 44 s, against 37 s.
* **The keying gate is load-bearing.** The daemon must refuse to start a transmission late,
  after every hold, and log the drop.
  * A host-keyed station (ADR-0025) must declare its keying latency, and its slack absorbs the
    host's jitter. This is the riskiest assumption, measured first.
  * A VOX that holds longer than declared loses frames; the protocol cannot prevent it.
* **The daemon's identifier** goes only inside the master's transmissions, or inside the slave's
  slot when it fits.
* **The answer gap** (ADR-0036/0054) becomes the declared turnaround, learned per station.
* **Complexity.**
  * The engine needs four timers (proceed, answer, link, connect) against eleven, and no turn
    flags against sixteen.
  * The estimated port is 2 000–2 500 lines against 4 327. This is an estimate.
* **A protocol break.** Protocol 8 talks to nothing older, as ADR-0068 already accepted.

## 5. How it is judged

Gates, each stated before it is run. A failure stops the step. It is explained as a violation
of an assumption or a defect in a rule, with a trace, before anything changes.

1. **The harness can attribute overlaps.** Transmission tags, a radiated mask, overlaps counted
   at each receiver and split into guard, VOX and radiated, distributions over 10 seeds, air-time
   stepping. It reproduces protocol 7 against ADR-0068 with classes.
2. **The model.** A new engine beside `LinkEngine`, not a switch in it, with `TwoStationSim`
   extended as the review's simulator is. The criteria:
   * no RF overlap;
   * field ×1.10 ± 0.05 of protocol 7;
   * chat ≤ ×1.05;
   * completion no lower in any profile.
3. **The port and the daemon.** Vectors for the control frame and engine traces. The gate's drop.
   `LINK_PROTOCOL` 8.
4. **The harness A/B.** The criteria:
   * zero radiated overlaps of protocol cause;
   * completion at least protocol 7's, scenario by scenario;
   * median air ≤ ×1.15 a scenario class and ≤ ×1.10 pooled;
   * Test rates ≥ 0.85 of protocol 7's.
5. **Hardware in the loop, then the air.** Keying interfaces measured, host keying first. Then
   a beta to the testers. Then P9-1 against VARA on cables.

## 6. Field builds hold still

As under ADR-0068: no beta with link changes until step 4 passes. ADR-0067 (PHY) stands
regardless. ADR-0065 and ADR-0066's first-move timer have no counterpart here.
