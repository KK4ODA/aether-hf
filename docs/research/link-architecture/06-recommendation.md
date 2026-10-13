# Recommendation and roadmap

## 1. The recommendation

**Do not ship ADR-0068, or its six-frame amendment, as protocol 8. Replace its turn model
with granted slots, and keep the part of ADR-0068 that is right: the exact stated count.**

The new protocol 8:

* **One master per session.** The caller is the only station that ever keys on its own
  initiative, for the whole session.
* **Granted slots.** Every master transmission grants the other station one bounded slot,
  defined by what that transmission states.
* **The forfeit.** The other station keys only inside a slot granted by a frame it decoded in
  time to start the slot on time. The keying gate refuses to start late.
* **Only decoded content changes protocol state.** A detection, an announcement or a silence
  moves no deadline and changes no role.
* **Gone:** the turn, TURN, OFFER/TAKEN, the turn request, the resend backoff and every
  arrival-driven deadline.
* **Kept** from the model and the port: the PHY, the frame families, the exact counts of
  ADR-0068 §2.1, FEC, HARQ, selective repeat, the rate controller and the connect body.

This is a recommendation to **build and test**, not to ship. It reaches the air only through
the gates in §4.

## 2. Why, in the order of the brief's priorities

1. **Correctness of authority and recovery.**
   * Granted slots has one safety invariant with a short argument
     ([04-formal-spec.md](04-formal-spec.md) §7). TLC checks it in two configurations.
   * It produced no RF overlap in 5 040 randomised sessions and 224 adversarial ones.
   * ADR-0068's engine has model-checked counterexamples *with perfect carrier sense and every
     late answer forfeited* (`ImplicitTurn_forfeit_perfect`, `ImplicitTurn_deadlock`). It
     overlaps 0.79 times per session in the field profile.
   * The six-frame amendment changes none of the mechanisms: 0.76 per session.
2. **Session completion.**

   | | Granted slots | ADR-0068 | Protocol 7 |
   |---|---|---|---|
   | Field profile | 100.0 % | 98.6 % | 99.4 % |
   | Hostile profile | 99.8 % | 95.3 % | 93.9 % |
   | Adversarial scenarios | 224/224 | 202/224 (215 with a matched link timeout) | 203/224 (216) |

   Its few non-completions were still delivering at the horizon. The others' were link
   timeouts.
3. **Competitive goodput.** In the field profile granted slots costs ×1.10 the completion
   time of protocol 7 (bulk and exchange) and ×1.035 on chat. That is less than ADR-0068's
   ×1.15–1.22, and about the six-frame amendment's ×1.11–1.13.

   **The cost is real and stays.** About 9–13 % of bulk goodput at +12 dB, where the
   per-transmission header and the reserved gaps are a larger share. Half of it is the
   header, half the reserve for the slowest legitimate decode (§6.2 of
   [05-experiments.md](05-experiments.md)). Protocol 7 is faster *because* it answers whenever
   it decodes and stretches its waits over anything it hears. That is what produces its
   overlaps.
4. **Predictable performance in poor conditions.**
   * Losses cost a slot, never an overlap or a split role.
   * A slow station gets a wider gap instead of colliding: 16/16 on the slow-computer
     scenario. The implicit-turn arms overlapped 12.6–17.6 times a session there, and the
     fixed cycle completed none.
   * Phantoms change nothing: 16/16 at 30 false detections a minute, against 10–13/16 for
     the implicit-turn arms.
5. **Low complexity.**
   * The prototype has four timers against eleven, no turn flags against sixteen, and one
     control frame layout.
   * About a quarter of the port's non-test engine (950–1 000 lines) exists to infer burst
     ends, lost answers or the turn. Granted slots has no counterpart for any of it.
   * Estimated: a port of 2 000–2 500 lines against 4 327. **This is an estimate from the
     prototype, not a measurement.**

## 3. What it costs and what it risks

* **A breaking protocol change.** The author has accepted that for protocol 8.
* **Time.** About 10 % against protocol 7 in the field profile, and 9–13 % less bulk goodput
  at high SNR. A line typed at the called station on an idle link waits up to an open window:
  the p90 of the slowest line is 44 s against ADR-0068's 37 s.
* **The keying gate becomes load-bearing.** Safety assumes a transmission never *starts* on
  the air later than its slot allows (S2). The daemon must check that where the radio is
  keyed, after every hold. The engine cannot check it.

  **A host-keyed station** (`[ptt] kind = "host"`, ADR-0025) hands keying to a host program
  whose PTT latency Aether does not see. Such a station must declare a keying latency, and
  audio must start no later than the slot allows. Its slack is the host's jitter. **This is
  the riskiest assumption and must be measured** with VarAC's real keying (§4, step 4).
* **A VOX that holds longer than declared** is outside every protocol's contract. Granted
  slots makes the gap include a declared turnaround, learned per station as ADR-0054 does.
  What exceeds it loses frames. It does not create protocol overlaps, but it still overlaps
  on the air.
* **The simulator is not the air.** Phantoms are synthetic, the profiles are judgement, and
  the PHY is a calibrated pipe (§9 of [05-experiments.md](05-experiments.md)). The harness,
  the bench and the air decide.

**Not recommended:**

* **The six-frame amendment as the end state.** It recovers part of ADR-0068's time cost but
  touches none of the overlap mechanisms. Its "three or more" brings back an unstated end.
* **The fixed cycle (C).** It is safe but 34–63 % slower, doubles chat latency, and cannot
  serve a slow decoder.
* **A transferable token with a lease (B).** It is safe under bounds, but for two stations it
  is dominated by never moving the timing authority at all.
* **Another rule on protocol 7 or ADR-0068.** Every remaining class traces to the authority
  model, not to a missing rule.

## 4. The roadmap

Each step has an exit criterion stated before it is run. **A failing criterion stops the
step.** Before any change is made, the failure is explained as a violation of an assumption
(S1–S7) or a defect in a rule (R1–R12), with a trace: a TLA+ counterexample or a deterministic
scenario. A fix that cannot name the invariant it restores is not made.

**Step 0 — now, no change to field builds, the model engine or the port's protocol.**

* **Instrument the scenario harness** (§10.1 of [05-experiments.md](05-experiments.md)):
  * the daemon tags every `MSG_PLAY` with the trigger, role and kinds;
  * the channel server records the radiated mask and counts overlaps at each receiver,
    split into guard, VOX and radiated;
  * results are reported as distributions over 10 seeds;
  * runs step in air time.
* **Fix port bug D1** (`"closed (no disc ack)"` against `LEFT_UNHEARD`) as its own change. It
  affects protocol 7 today.
* *Exit:* the instrumented harness reproduces the brief's protocol 7 against ADR-0068 numbers,
  and classifies its collisions with this review's classes.

**Step 1 — the model** (`model/aether_model/link/`).

* A new engine, `granted.py`, beside `LinkEngine`, not a switch inside it. The existing engine
  is not touched.
* Frames: the seven-byte control frame (`Ctl`) and the exact data counts in `frames.py`, as
  protocol 8.
* `TwoStationSim` gains what `advsim.py` has:
  * keying and decode latency;
  * phantoms and trusted failures;
  * the keying gate's `not_after`;
  * a radiated-interval record.
* Tests:
  * I1–I5 as strict tests on the simulator;
  * the fourteen scenarios;
  * exact delivery.
* The TLA+ model moves in-tree (`tools/tla/`) with its configurations.
* *Exit:*
  * no RF overlap anywhere;
  * the link bench and chat bench within the costs measured here: field ×1.10 ±0.05 against
    protocol 7, chat ≤ ×1.05;
  * no completion below protocol 7's in any profile.

**Step 2 — the port** (`core/aether-link/`, `core/aetherd/`).

* `aether-link/src/granted.rs`, cross-validated against the model by vectors:
  * control-frame encodings;
  * engine action traces on scripted inputs, held equal.
* `aetherd`:
  * the keying gate drops a transmission whose radiation would start after `not_after`, and
    says so in the log and the sidecar;
  * the identifier goes only inside the master's own transmissions, or in the slave's slot
    when it fits;
  * the answer gap becomes the declared turnaround: a station's learned gap (ADR-0054)
    becomes its gap level;
  * `LINK_PROTOCOL` 8 in `frames.rs` and the shell.
* The host adapter is unchanged: it is a client of the control API.
* *Exit:* `two_daemons.rs` and the harness's quick set pass. Every transmission's tag shows a
  granted slot or the master.

**Step 3 — the harness A/B, pre-registered.** The full scenario set and the stress set, at
least 10 seeds a scenario, instrumented. The criteria:

* **zero radiated overlaps of protocol cause** (guard and VOX reported, not counted);
* **completion at least protocol 7's**, scenario by scenario;
* **median air time** at most ×1.15 of protocol 7's in every scenario class, and at most
  ×1.10 pooled;
* **Test message and file rates** at least 0.85 × protocol 7's, pooled.

If any criterion fails, stop. The failure is analysed under the rule above, not patched.

**Step 4 — hardware in the loop, then the air** (§10.3–10.4 of
[05-experiments.md](05-experiments.md)).

* The keying interfaces' real timing: CAT, RTS, VOX, CM108, and host keying through VarAC.
* Then a beta of protocol 8 to the regular testers, with the instrumented sidecars.
* Then P9-1 against VARA on cables, under the stated criteria.

**Step 5 — retire.** Once protocol 8 has shipped and held for a release cycle:

* delete `LinkEngine`'s turn, request, offer, resend and arrival machinery, and the
  `stated_answers` switch, in both suites;
* delete the daemon's `FOLLOW_ON_*`, `answers_burst` and `fragment_of_own_tx` paths;
* keep the PHY ADRs (0060, 0067) and the counts.

**What waits on `master` today.**

* **ADR-0067** (the tone stream's end block) is PHY and stands.
* **ADR-0065** (a phantom starts no burst) and **ADR-0066's first-move timer** have no
  counterpart under granted slots. ADR-0066's first rung by decibels does.
* **ADR-0055's amendment** (the identifier) is subsumed by step 2's identifier rule.

Whether any of these ships before protocol 8 is the author's call. Nothing in this review
requires it.

### Essential and optional

| Essential (the safety and liveness argument rests on it) | Optional (performance; can be dropped without losing safety) |
|---|---|
| R1 one initiator; R2 the slot; R3 forfeit at the gate; R4 the master's next key; R5 attribution by air time; R6 reserve for anything the slave may hold; R10 ending; R11 connection | R7 header elision; R8 gap levels (needed for slow stations to *progress*, not for safety); R12 open windows (needed for chat latency); the first header's family from the acceptance |

## 5. The answer to the brief's last question

> I want a defensible engineering decision, not another incremental patch and a reassuring
> benchmark.

The decision rests on a counterexample, not a benchmark. ADR-0068's engine puts both stations
on the air with perfect sensing and no late answers, because a lost acknowledgement leaves
both stations believing they hold the turn. Granted slots cannot reach that state: there is no
turn to hold, and no slot outlives its deadline.

The benchmark says what the change costs: about 10 % against protocol 7, less than ADR-0068.
The next measurements, on the harness, the bench and the air, are set up to falsify it.
