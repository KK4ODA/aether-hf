# ADR-0030: A sender waits out a frame it hears arriving

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`, `on_preamble`),
then the port (`aether-link` `engine.rs`). No frame, link protocol or configuration change.

## 1. Context

A caller and a prober (ADR-0016), a leaving station's DISC (ADR-0022) and a station that has
handed over the turn (ADR-0023) do not try again over a frame they hear arriving: it may be the
answer, late, and a repeat keyed over it is heard by nobody — the station repeating cannot hear
the rest of the answer, and the other station, transmitting, cannot hear the repeat. The
sender's two other retries did not wait. When the wait for a burst's acknowledgement or a
poll's answer ran out, `_on_response_timeout` sent the burst or the poll again, whatever was
arriving. ADR-0027's chat bench found it (§7, 3).

The case it traced: a receiving station that detects a poll and cannot decode it answers the
preamble once its quiet after a frame has passed (`on_preamble`'s acknowledgement deadline: the
announcement time of the family it last decoded, the burst gap and a turnaround — 0.99 s when
that family was the floor's), and answers in that family: 3.2 s on the floor. The sender, whose
poll was ordinary, waited for an answer starting within a turnaround of the poll's end — 4.0 s
with a floor answer — and polled again 0.2 s before the answer ended. It heard none of the
answer; the other station, transmitting, heard nothing of the new poll and answered the one
after it, which met the next poll. After nine polls the sender ended the session with both
stations up: "no response". On the bench this ended sessions under every turn policy.

## 2. Decision

A sender waiting for the acknowledgement of its burst or the answer to its poll that hears a
frame arriving moves its retry past the frame's end and the time to decode it — the frame's
start, its length, a turnaround, the detection latency and the ACK margin (`_response_wait(0)`),
the longest frame there is when the physical layer does not name it — as a DISC and a TURN are
moved (`on_preamble`, the `wait` timer). If the frame is the answer, it is taken. If it is not,
or does not decode, the retry goes out after it and counts as it did: `max_retries` and the
link timeout bound the session as before.

The rule holds a retry only when the frame announces itself before the retry falls due. A
retry that falls due first — an ordinary poll waiting for an ordinary answer is repeated
1.23 s after its end, and a floor answer to a poll the other station could not decode is
announced 1.53 s after it — needs a longer wait, which is ADR-0027 §7 (1) and a change of its
own.

## 3. Measured

Both benches on the fading pipe with the floor's reading cap (P9-6, ADR-0016), the engine
before (a worktree) against this one, the same seeds, run before ADR-0027 landed.
`tools/bench_chat.py` is ADR-0027's, from its branch (`chat-handover`, `fe13ca7`): today's
turn-taking on the model at `7345694`, with a copy that does without `LinkStats.turn_requests`,
which that model did not have; ADR-0027's request on its branch's model — the one that landed,
less ADR-0026's `set_air`, which the bench never calls — with and without this change. Latency and keyed time are the ratio after/before: the
median over the points, and their range.

| `bench_chat.py` | sessions | drops | lines lost | median latency | 90th percentile | keyed per line |
|---|---|---|---|---|---|---|
| today's, −12 to +12 dB, 30 a point | 900 | 8 → 3 | 83 → 26 of 13 710 | 1.00 (0.94–1.06) | 1.00 (0.93–1.06) | 1.00 (0.95–1.08) |
| today's, −12, −6, 0 dB, 100 a point | 1 800 | 15 → 9 | 126 → 71 of 27 612 | 1.00 (0.93–1.03) | 1.00 (0.96–1.02) | 1.00 (0.96–1.03) |
| ADR-0027's request, 30 a point | 900 | 7 → 4 | 77 → 40 of 13 710 | 1.00 (0.95–1.09) | 1.00 (0.96–1.07) | 1.00 (0.97–1.04) |
| ADR-0027's request, 100 a point | 1 800 | 13 → 1 | 104 → 20 of 27 612 | 1.00 (0.96–1.02) | 1.00 (0.94–1.01) | 1.00 (0.98–1.02) |

Good, Moderate and Poor, both airs. The "before" rows are ADR-0027's own: 15 drops and 126 lines
for today's turn-taking, 13 and 104 for the request. Where the spiral ran, the polls fall: at
−12 dB on Good, 1.71–1.76 polls a line → 1.27–1.30 today, 0.98–1.02 → 0.75–0.77 with the
request.

The fault itself, counted on the 100-a-point run's 1 800 sessions — a retry keyed while a frame
the station had heard arriving was on the air: today 1 484 polls → none; with the request 1 099
polls and 53 bursts → none. What is still keyed over the other station's frame began before that
frame could announce itself: new bursts in an acknowledgement's first moments (46 → 59 today,
46 → 57 with the request), idle polls over a request just begun (481 → 492, the race ADR-0027
§6 weighed and left), two retried polls (21 before), and one burst whose station was keyed
itself when the frame began.

Where today's drops went (Good and Poor at −12, −6 and 0 dB, both airs, 1 200 sessions of the
100-a-point run): "no response" 8 → 2, link timeouts 6 → 7. Both "no response" drops left are
another fault: an ordinary poll and its ordinary answer lost together in a slow Good fade,
polled again every 2.3 s until the retries ran out — a poll's timeout steps nothing down
(`_back_off` is a burst's), so the polls never fall to the floor. The link timeouts are the
slow Good fades ADR-0027 found under every policy.

`tools/bench_link.py` (2 kB transfers on Good, Moderate and Poor, −12 to +12 dB, both airs,
20 sessions a point, 600 an engine): 546 sessions identical, 596 of 600 delivered either way,
ACK timeouts 269 → 268, every point's mean time within 0.4 %. In a transfer the receiving
station answers a burst inside the wait its sender expects, and there is seldom anything to
hold for.

`bench/baselines/wait_out_chat.csv` (`engine` before/after; `policy` `base` for today's,
`request` for ADR-0027's; `first_trial` 0 the grid, 100 the drop run) and
`bench/baselines/wait_out_link.csv` (`engine` before/after) have the rows.

## 4. Consequences

* Tests, model and port: `a_poll_is_not_repeated_over_its_answer_arriving` (both airs, a path
  the floor's frames cross and the ordinary control frame does not — the sender gave up with
  "no response" before) and `a_burst_is_not_repeated_over_its_acknowledgement_arriving` (the
  retry waits for the frame's end, and the late acknowledgement is taken).
* The rule does not care why an answer is late: a receiver running behind real time
  (ADR-0010 §6), a hold at the other station or a third station's frame is waited out too,
  when it announces itself before the retry — none of which the benches model.
* A detection that is no frame at all moves a retry by up to the length it names and 0.8 s.
* With ADR-0027's request, an acknowledgement nobody asked for that arrives while the sender
  waits is waited for and taken like any answer: 13 of 1 800 sessions dropped before, 1 after.
