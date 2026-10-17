# The link protocol, reviewed: an architecture study

This is an independent review of Aether HF's half-duplex link protocol, done on 2026-10-13.
It asks whether the architecture can reach the reliability of a mature HF modem and, if not,
what should replace it. It changes no production code. The research code is in
[`research/link_arch/`](../../../research/link_arch/), and the decision is
[ADR-0069](../../adr/0069-granted-slots.md), accepted for building on 2026-10-13.

## The three questions, answered first

**1. What is the deepest reason ADR-0068 still permits collisions?**

In Aether, the right to transmit is each station's private belief, and each station may act
on its own belief at times of its own choosing. ADR-0068 fixed *when a receiver answers a burst
it read*, by having the burst state it. That part is right, and it removes early answers
entirely. It left three things alone:

* **who may start a transmission.** The receiving station still sends unsolicited turn
  requests.
* **how late an answer may go.** Answers are sent at `max(now, due)`, after the daemon's
  holds and late decodes. By then the sender's fixed wait has expired and it repeats.
* **how authority moves.** OFFER/TAKEN and TURN change one station's role on a frame the other
  may never see.

In the simulator's field profile, requests are half of ADR-0068's overlaps and late answers the
other half. TLC finds both stations keying as ISS even with perfect carrier sense and every
late answer forfeited, and both stations waiting as IRS until the link times out. No message
exchange can make two stations agree on who may transmit over a lossy channel (the two-generals
result). Collisions can be ruled out only by bounding authority in time.
[01-root-cause.md](01-root-cause.md)

**2. What protocol would you design from scratch, keeping the PHY and HARQ?**

**Granted slots:**

* **One master.** The caller is the master for the whole session and the only station that
  ever keys on its own initiative.
* **Grants.** Each master transmission (a header, then up to six data frames with exact
  counts) grants the other station exactly one slot. The slot starts a fixed gap after the
  stated end, and holds an answer, an answer and *n* data frames, or an open window.
* **The forfeit.** The other station keys only inside a slot granted by a frame it decoded in
  time to start the slot on time. Otherwise it does not key at all, and the keying gate
  enforces this.
* **The master's next key.** The master keys again after the end the answer stated, or after
  the slot's latest end. Never earlier, never on silence alone.
* **What is gone.** There is no turn, no handover and no turn request. Data flows both ways in
  every exchange.
* **What is kept.** FEC, HARQ, selective repeat, the rate controller, the frame families and
  ADR-0068's exact counts.

[04-formal-spec.md](04-formal-spec.md)

**3. What is the smallest decisive experiment or counterexample?**

* **Formally**, one lost acknowledgement. In `ImplicitTurn_forfeit_perfect`, a burst offers
  the turn, the acknowledgement that takes it is lost, and both stations repeat as ISS at the
  same tick. Every timing rule is obeyed, and an 18-step TLC trace shows it. Under the same
  loss and latency model, `GrantedSlots_safe` holds NoOverlap over every reachable state
  (8 272, and 103 780 in the larger configuration). Its two mutants fail, which shows that the
  forfeit and the attribution carry the proof.
* **Empirically**, the same event as a scripted scenario. In `handover-lost`, the called
  station's first data transmission goes unheard. ADR-0068 overlaps 1.25 times a session;
  granted slots does not overlap.
* **On the harness**, the same scenario with two real daemons decides it, once the harness can
  attribute overlaps ([06-recommendation.md](06-recommendation.md) step 0).

## What the evidence says

| | Protocol 7 | ADR-0068 | ADR-0068, 6-frame | Granted slots | Fixed cycle |
|---|---|---|---|---|---|
| RF overlaps per session, field profile | 0.80 | 0.79 | 0.76 | **0** | 0 |
| Sessions completed, field / hostile | 99.4 / 93.9 % | 98.6 / 95.3 % | 97.9 / 94.5 % | **100.0 / 99.8 %** | 99.8 / 98.6 % |
| Adversarial scenarios completed (224 each) | 203 | 202 | 199 | **224** | 193 |
| Completion time against protocol 7, field (bulk / exchange / chat) | 1 | ×1.15 / ×1.22 / ×1.01 | ×1.11 / ×1.13 / ×1.01 | ×1.10 / ×1.10 / ×1.04 | ×1.34 / ×1.63 / ×1.48 |
| Model check | — | counterexamples | as ADR-0068 | **NoOverlap holds** | not modelled |

Simulation: 25 200 paired sessions, 5 040 per arm, plus 14 deterministic adversaries. The
protocol 7 and ADR-0068 arms are the model's real engine. **Simulation is not the air.** The
limitations are in [05-experiments.md](05-experiments.md) §9, and nothing here claims parity
with VARA.

**The tradeoff, stated plainly.** Granted slots is about 10 % slower than protocol 7 in the
field profile, and gives up 9–13 % of bulk goodput at +12 dB. That cost buys:

* zero overlaps;
* sessions that do not fall apart;
* one invariant in place of sixteen places where a station keys on its own belief.

It is faster than ADR-0068 on bulk and exchange traffic.

## Reading order

1. [01-root-cause.md](01-root-cause.md):
   * why collisions persist;
   * where authority lives;
   * every collision classified;
   * assumptions A–H, fact against conjecture;
   * protocol defects against radio limits.
2. [02-sequences.md](02-sequences.md): the current protocol, exchange by exchange, with the
   sixteen points where both stations may key.
3. [03-alternatives.md](03-alternatives.md): what PACTOR, ARDOP, VARA (documented only),
   STANAG 5066, 3G-HDL, Proximity-1, 802.4/WTRP and Mercury do, with citations; the four
   candidates; the comparison matrix.
4. [04-formal-spec.md](04-formal-spec.md):
   * granted slots, specified: assumptions, frames, rules, state machines, transition tables;
   * invariants;
   * TLC results with counterexample traces.
5. [05-experiments.md](05-experiments.md):
   * design, results and distributions, the time decomposition and the adversarial scenarios;
   * limitations;
   * methodology for the harness, HIL and a fair VARA comparison.
6. [06-recommendation.md](06-recommendation.md): the recommendation, its costs and risks, and
   a gated roadmap.
7. [07-protocol-8.md](07-protocol-8.md): protocol 8 as it is being built — frames bit by bit,
   timing, ending, the identifier, the engine's interface — and (§12) what building the model
   engine decided.
8. [08-steps-0-and-1.md](08-steps-0-and-1.md): the first two steps of ADR-0069 §5. The harness
   now attributes overlaps (protocol 7 against the ADR-0068 build). The model engine,
   `aether_model.link.granted`, passes its gate on the review's simulator: no radiated
   overlap in 5 040 sessions and 224 adversarial ones, ×1.09 of protocol 7 in the field
   profile, completion no lower anywhere.
9. [09-step-2.md](09-step-2.md): Step 2. The engine ported to `aether-link`, checked against
   the model input by input; the daemon runs it with the keying gate, the engine's account of
   its own transmissions, and the Morse identifier where the engine puts it. Then (§3) the
   radio's declared turnaround, configuration schema 12.
10. [10-step-3.md](10-step-3.md): Step 3, the harness A/B. Its criteria, made exact and
   committed before either arm ran (§1), then the runs and the verdict.
11. [11-step-4-first-air.md](11-step-4-first-air.md): Step 4, the first session on the air
   (WC4Y's Test of 2026-10-14), read from both stations' recordings on one clock: no overlap,
   the timing budgets measured on the mini PC, every silent slot and failed frame, the
   interference, and what to change, ranked.
12. [12-two-paths-80m.md](12-two-paths-80m.md): Step 4, three more sessions on the air (ND1J's
   Tests of 2026-10-15, one strong night path and two weak morning ones), read the same way:
   still no overlap; the keying and render times ADR-0071 learns; every silent slot, 21 of 22
   at headerless ladder rungs; and the author's questions on the speed ladder, a rung between
   rungs 3 and 5, the lopsided signal and the Morse identifier.
13. [13-wc4y-beta111.md](13-wc4y-beta111.md): Step 4, WC4Y's Test of 2026-10-17 on beta.111,
   read the same way: still no overlap; KK4ODA-1's sound card losing audio inside two received
   frames; the Test's ladder leftovers taking a fifth of it; both Morse identifiers at exactly
   20 wpm; and the Status tab checked reading by reading, which found the busy detector learning
   the other station's signal as its noise floor.

## Reproduction

Everything runs from the repository root after `python -m uv sync`. The commands are in
[05-experiments.md](05-experiments.md) §11. The committed results are in
`research/link_arch/results/`. The TLA+ models and TLC outputs are in `research/link_arch/tla/`.
`uv run pytest research/link_arch/tests -q -o addopts=""` checks the prototypes' invariants. The
production modem, its tests and the field builds are untouched.
