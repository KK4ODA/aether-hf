# ADR-0029: A TURN heard again is answered, and the turn is not taken back on a guess

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`), then the port
(`aether-link` `engine.rs`); tests in both suites. No frame changes and no link protocol change:
a station of an earlier version ignores a TURN repeated, as it always did, and a station of this
one works with it.

## 1. Context

ADR-0027 §7(2), found by the chat bench: **a TURN whose answer is lost can leave both stations
holding the turn.** A station that hands the turn over sends a TURN, becomes the receiving station
at once and waits for the other's answer — its first burst, or a poll when it has nothing to send.
Any data frame it hears counts as the answer; a poll counts only if it can be read. With no answer
it repeats the TURN and, after `turn_retries` (3), "carries on as ISS". The station that took the
turn ignored the TURN repeated: a TURN was acted on only by a receiving station or one waiting for
a TURN. So a sender whose TURN's answer was lost repeated it into a station that no longer listened
for it, took the turn back, and both sent. ADR-0023's yield (a called sender that hears a poll
yields) needs the called station to hear the caller's poll, which it cannot while it sends a long
burst.

Today's turn-taking rarely meets it: the answer to a TURN is nearly always a burst, since a station
is handed the turn because it has something to send, and one data frame heard confirms the TURN.
ADR-0027's rejected candidate — **hand the turn over after every line** — made every line a TURN
answered by a poll, and lost four times the sessions (66 of 1 800 against 15, with the request).
Two traces of that candidate on the bench, on the engine of ADR-0027 (2 300 Hz, Good, −12 dB, the
tone floor):

* **Trial 123.** B hands the turn over (67.9 s); A takes it and polls. B cannot read the poll and
  answers its preamble with an acknowledgement — the acknowledgement a receiving station keys when
  a frame it hears arriving turns out not to decode (it may be the end of a burst, or a poll) — and
  A, whose poll's wait ran out 0.2 s before that acknowledgement ended (ADR-0027 §7(1)), re-polls
  over it. B's second TURN goes out queued behind the acknowledgement, into A's re-poll, and its
  third into the next. B takes the turn back at 98.9 s and sends its next line as a 27 s burst;
  A, holding the turn, re-polls into it until it gives up: "no response".
* **Trial 116.** A hands the turn over; B cannot read the TURN and acknowledges its preamble. A
  reads that acknowledgement — B is still receiving — and acknowledges it in turn: its own
  preamble had armed an acknowledgement, and nothing withdrew it. That acknowledgement went out
  right behind A's third TURN, which B read: B took the turn and polled, under A's
  acknowledgement. A, having heard nothing after its third TURN, took the turn back and sent a
  burst over B's polls: "no response" again.

ADR-0030 (a sender waits out a frame it hears arriving, §7(3)) landed while this was measured, and
took most of the collisions out of the TURN's exchange — the handover candidates lose 12 and 5
sessions of 1 800 where they lost 72 and 66 — but not the fault: a TURN's answer lost for any
reason still ends with two senders, and on today's master six of the drop run's ten "no response"
sessions are that (§4).

## 2. The bench

`tools/bench_chat.py` (ADR-0027) runs the link bench's engines through keyboard-to-keyboard
contacts on the fading pipe with the floor cap and the key-time limit, under four policies:
today's (`base`), the chat request ADR-0027 adopted (`request`), and its two rejected handover
candidates (`handover`, `handover+request`), which meet the fault on every line and so measure it.
Two engines before, each with this change after:

* **ADR-0027's** — `master` at ce23476, the engine the finding was made on (the runs used the
  chat branch's copy, which differs only by ADR-0026's `set_air`, never called on the bench);
* **today's** — `master` at f17fec5, with ADR-0030.

On each: **the drop run**, 100 fresh sessions a point (`first_trial` 100) at −12, −6 and 0 dB on
Good, Moderate and Poor, both airs — 1 800 sessions a policy, ADR-0027's drop run — and **the
grid**, 30 sessions a point on AWGN, Good, Moderate and Poor from −12 to +12 dB, both airs. On
ADR-0027's engine, **the screen** that chose among the candidates of §3: 25 sessions a point on the
drop run's points (450 a policy), the candidates switched in a scratch copy of the engine. On
today's, **two ablations** on the drop run for the handover candidates: the answer to a TURN heard
again alone, and the other two rules without it.

Also: `tools/bench_link.py`, 2 kB sessions, both airs, the four classes, −12 to +12 dB, three a
point, the fading pipe with the floor cap — transfers, which never hand the turn over; and **a
fade that carries no data** (ADR-0023's KE4QCM case) after the link has climbed: the caller hears
none of the called station's data frames for 60 to 240 s while control frames get through both
ways, ten seeds a length (`TwoStationSim(unheard=…)`).

The results are in `bench/baselines/turn_offer.csv`: `base` is the engine's master, `engine` is
`before`, `after`, `answer-alone` or `without-answer`, and `run` is `drops` or `grid`.

## 3. The candidates

* **(A) The station holding the turn answers a TURN heard again** as it answered the first: its
  burst again, which the other station heard none of — any frame of it would have been the
  answer — or its poll.
* **Taking the turn back.** The offering station cannot tell a station that never read its TURN
  from one that took the turn and whose answers are being lost — except by what it hears after
  the TURN. A frame it could not read may be the answer (a poll too weak to read); an
  acknowledgement it can read says the other station is still receiving; silence says nothing.
  * **(C)** Offer again, up to `max_retries` TURNs, while the last frame heard since the first
    TURN was one it could not read; take the turn back after `turn_retries` otherwise.
  * **(K)** Offer again unless it read an acknowledgement — silence included.
  * **(R)** As C or K, with what was heard counted from the last TURN only.
  * **(P)** Take the turn back by polling first, not by bursting.
* **(N) An acknowledgement is not acknowledged.** A receiving station that reads an
  acknowledgement withdraws the acknowledgement that frame's preamble armed: the other station is
  receiving too and waits for nothing.

## 4. What was measured

### On ADR-0027's engine

**The screen** (sessions lost of 450 a policy, lines lost in brackets):

| Candidates | today's | request | handover | handover+request |
|---|---|---|---|---|
| none (before) | 4 (39) | 4 (32) | 22 (217) | 17 (162) |
| A | 4 (39) | 4 (32) | 21 (216) | 16 (153) |
| C | 4 (39) | 4 (32) | 8 (87) | 9 (71) |
| K | 4 (39) | 4 (32) | 8 (87) | 9 (71) |
| A + K | 4 (39) | 4 (32) | 8 (87) | 9 (70) |
| A + K + N | 6 (46) | 4 (32) | 8 (70) | 4 (39) |
| A + K + N + R | 6 (46) | 4 (32) | 8 (70) | 4 (39) |
| K + N + R | 6 (46) | 4 (32) | 7 (60) | 5 (43) |

(A) alone was nearly inert on that engine: the TURN repeated reached the station holding the turn
while it re-polled — its poll's wait had run out under the late acknowledgement of ADR-0027 §7(1),
and nothing held a re-poll for a frame arriving (§7(3)) — so it was not heard. Taking the turn back
on evidence (C or K) removed most of the losses, and (N) most of the rest. C and K are the same on
the bench, where every frame not keyed over is at least announced: they differ only when nothing
at all is heard, and there **K loses sessions C keeps** (below). (R) made no difference; (P) was
no better than taking the turn back as before (7 sessions of 25 against 6, hands over, at the first
point screened: 2 300 Hz, Good, −12 dB).

**The drop run**, before and after (A + C + N), 1 800 sessions a policy:

| Policy | sessions lost, before | after | lines lost, before | after |
|---|---|---|---|---|
| today's | 15 (0.8 %) | 16 (0.9 %) | 126 of 27 612 (0.46 %) | 122 (0.44 %) |
| request (ADR-0027) | 13 (0.7 %) | 14 (0.8 %) | 104 (0.38 %) | 98 (0.35 %) |
| hands over | 72 (4.0 %) | 17 (0.9 %) | 592 (2.14 %) | 153 (0.55 %) |
| asks, and hands over | 66 (3.7 %) | 18 (1.0 %) | 464 (1.68 %) | 175 (0.63 %) |

The before column is ADR-0027's drop run again, to the session. By cause, the handover
candidates' "no response" — the two senders' signature — fell from 65 to 9 and from 61 to 11;
their link timeouts, the slow fades every policy meets, stayed (7 → 8, 4 → 6). Today's policy and
the request moved by one session each, both ways across the points: traced (today's, trial 118;
the request, trial 168; both 2 300 Hz, Good, −12 dB), each first parts from its before at an
acknowledgement of an acknowledgement that is no longer keyed, the next TURN going out 2.2 s
sooner, and then meets another stretch of the fade, a slow one that timed the link out near the
end. **The grid**: a line's delay and the keyed time unchanged for every policy (the median over
the 40 points of each ratio 1.00, every point within 0.93–1.08); sessions lost 8 → 8, 7 → 7,
19 → 12, 18 → 13 of 1 200.

### On today's master, with ADR-0030

**The drop run**, 1 800 sessions a policy:

| Policy | sessions lost, before | after | lines lost, before | after |
|---|---|---|---|---|
| today's | 9 (0.5 %) | 5 (0.3 %) | 71 of 27 612 (0.26 %) | 45 (0.16 %) |
| request (ADR-0027) | 2 (0.1 %) | 1 (0.1 %) | 20 (0.07 %) | 12 (0.04 %) |
| hands over | 12 (0.7 %) | 4 (0.2 %) | 67 (0.24 %) | 33 (0.12 %) |
| asks, and hands over | 5 (0.3 %) | 7 (0.4 %) | 34 (0.12 %) | 35 (0.13 %) |

The counts are small now, and what they say is by cause: **"no response" fell from 10 to 2** over
the four policies (hands over 5 → 0, asks and hands over 2 → 0, today's 2 → 1, the request 1 → 1),
and a stalled session went. Six of the ten were two senders — every one of the handover
candidates' but one, traced (500 Hz: Poor 0 dB, trials 146, 169, 198 and 119; Moderate 0 dB, 105;
Good 0 dB, 199): the offering station took the turn back while the other held it, and the other
gave up — and none is left. **Link timeouts** — slow fades, the same sessions meeting another
stretch of them — went 17 → 15 (hands over 7 → 4, asks and hands over 3 → 7, today's 7 → 4). The
two "no response" left are one session under two policies (500 Hz, Good, −6 dB, trial 122, lost
before and after): the caller's ordinary polls unread through a fade, each answered and the answer
unread, until its tries ran out — an unanswered poll never steps the recommendation down, as an
unanswered burst does: a fault of its own, not a turn.

**The ablations**, for the handover candidates (sessions lost of 1 800, "no response" among them
in brackets):

| Engine | hands over | asks, and hands over |
|---|---|---|
| before | 12 (5) | 5 (2) |
| the answer again alone (A) | 9 (3) | 9 (2) |
| the evidence and the withdrawn acknowledgement, without the answer (C + N) | 4 (0) | 5 (0) |
| all three (after) | 4 (0) | 7 (0) |

On the bench C and N do the work: the answer again alone leaves most of the two senders, and with
the other two it adds nothing measurable (the link timeouts, 5 against 7, are the fades'). It is
kept for the case they cannot reach — an offering station that hears nothing at all, which the
bench, where every frame not keyed over is announced, rarely makes and a fade in one direction does:
with no evidence to go on the offering station takes the turn back after three TURNs, as it must
for the fade that carries no data (below), and only an answer to one of them keeps the two from
both sending. C and N without it fail the reproduction with `unheard`
(`test_a_turn_whose_answer_was_lost_is_answered_again`), and it without them fails the unreadable
poll, the count and the acknowledgement tests.

**The grid**, 1 200 sessions a policy: a line's delay and the keyed time unchanged for every
policy — the median over the 40 points of each ratio 1.00, every point within 0.85–1.09 — and
sessions lost 3 → 4, 4 → 5, 4 → 3 and 3 → 3, "no response" 5 → 1. Its link timeouts went 8 → 13
and the drop run's 17 → 15; none of the 28 after the change came while a station was offering the
turn (each was a sender waiting for an acknowledgement, or an idle receiver, in a slow fade) —
traced, today's trial 25 (2 300 Hz, Good, −12 dB) parts from its before at an acknowledgement of an
acknowledgement no longer keyed and times out later in another stretch of the fade; and a session
kept from "no response" lives on to meet the fades that end the others.

**Transfers are untouched**: `bench_link.py`'s sessions are identical in every column but the wall
time, both airs (60 rows each; 59 of 60 complete either way), on both engines.

**A fade that carries no data**, after the link has climbed (both stations on OFDM, so silence
ends a session after 45 s, not after four floor exchanges): sessions that survived, of ten a fade
length from 60 to 240 s — 10 before, 10 after, **0 under K**, with and without preamble reports.
On that path the station holding the turn answers every TURN with a burst nobody hears; what keeps
the link alive is the poll of a caller that took the turn back, which the called station answers
by yielding (ADR-0023) — offering the turn for eight TURNs on silence starves the offering station
of anything it can read for longer than its link timeout. The same held on ADR-0027's engine.

## 5. Decision

* **A TURN heard again is answered.** A station that holds the turn and reads a TURN of its
  session answers it as it answered the first (`_answer_turn`, which `_take_iss` now calls too):
  its burst again when it has frames to send — the same composition an unanswered burst goes
  again with — or a poll. It resets its unanswered count: the other station was heard. Not while
  its own transmission is still going out: that is the answer, on its way.
* **The turn is taken back only on evidence.** A station offering the turn notes whether the last
  frame it heard from the other station since its first TURN was one it could not read
  (`_turn_unread`): a frame announced by its preamble, or one that arrived and did not decode — a
  readable acknowledgement clears it. After `turn_retries` TURNs it takes the turn back as before,
  unless that is so; then it offers again, up to `max_retries` TURNs (`_turn_offers`). Silence
  keeps the old count, for the fade that carries no data (§4).
* **An acknowledgement is not acknowledged.** A receiving station (in a session) that reads an
  acknowledgement withdraws the acknowledgement that frame's preamble armed, unless a burst is
  being received. Two receiving stations — the TURN's sender and a station that could not read the
  TURN — no longer answer each other's acknowledgements, and the TURN's sender no longer keys one
  over the answer to its TURN.

## 6. Rejected

* **Offering the turn on silence (K)**: the same on the bench, and it loses the climbed session a
  fade that carries no data would otherwise leave alive (§4). Kept out by a test in both suites
  (`test_a_climbed_session_outlives_a_fade_that_carries_no_data`).
* **Answering a TURN heard again always with a poll**: any frame of a burst the offering station
  hears answers its TURN, readable or not; a poll only if read. On a path that carries control
  frames and no data a poll would reach it — and then the station holding the turn would send
  bursts nobody hears until its unanswered count ran out, where the TURNs and the yield keep
  resetting it (reasoned, not run).
* **Counting what was heard from the last TURN only (R)**: no difference on the bench; counted
  from the first, a frame that could not be read before a TURN keyed over the answer still counts.
* **Taking the turn back by polling (P)**: no better.
* **Any rule alone**: the answer again alone leaves the two senders the bench meets; the evidence
  and the withdrawn acknowledgement alone leave the ones an offering station that hears nothing
  meets (§4).

## 7. Consequences

* The model: `_answer_turn`, `_turn_unread`, `_turn_offers`; the TURN and ACK branches of
  `_on_control`; `LinkConfig.turn_retries` and `max_retries` say what they now count. The port
  mirrors it (`answer_turn`, `turn_unread`, `turn_offers`). Tests in both suites: the TURN whose
  answers are lost until it has gone out three times (`unheard`), the TURN answered by polls the
  offering station cannot read (the chat trace), the answer again (a poll, a burst, not while
  keyed, not another session's), the offering station's count (silence, a frame it could not
  read, its preamble alone, a readable acknowledgement after it), an acknowledgement not
  acknowledged, and the climbed session through a fade that carries no data. Each but the last
  fails on today's master; the last passes there and fails under K (in the model). The
  air-interface spec's link section says what a station does with a `TURN` heard again and with
  an acknowledgement it reads while receiving.
* With ADR-0030 and this, the handover candidates lose about what today's policy does (4 and 7
  sessions of 1 800, against 5). ADR-0027 kept them out for their losses; their latency gain (a
  line's median 0.48 of today's against the request's 0.64, ADR-0027 §4) is worth measuring again
  for adoption — a chat decision, ADR-0027's, not made here.
* What the drop run still loses, with ADR-0030 and this, is slow fades and the unanswered poll
  that never steps down. ADR-0027 §7(1) — the poll's wait for a late answer — is not mended on
  master yet; mended, fewer of the TURN's answers will be late.
* A station that offers the turn to one of an earlier version gets no answer to a TURN repeated,
  as before, and offers up to `max_retries` times while it hears frames it cannot read — at most
  five TURNs more than before, on a path where it could read nothing anyway.
