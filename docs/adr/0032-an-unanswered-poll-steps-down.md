# ADR-0032: An unanswered poll steps the recommendation down, from the second in a row

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`:
`LinkConfig.poll_silences`, `_on_response_timeout`), then the port (`aether-link` `engine.rs`).
No frame, link protocol or configuration-file change.

## 1. Context

ADR-0030 §3 and ADR-0028 §4 (on its branch) traced the chat-bench drops that their fixes left
to one fault: an idle sender's ordinary poll and the other station's ordinary answer, lost
together in a slow ITU Good fade. On the 500 Hz air at −6 dB (the drop run's trials 122 and 188),
today's turn-taking, the engine at `f17fec5`:

| time (s) | |
|---|---|
| 72.01 | A, idle, polls (ordinary, 0.43 s). B hears the preamble and cannot decode the poll |
| 73.03 | B answers once its quiet after the frame has passed, in the ordinary family. A cannot decode the answer |
| 74.27 … 90.10 | A polls eight more times, 2.26 s apart, and each goes the same way |
| 92.4 | nine unanswered polls: A ends the session, "no response", 26 s before the link timeout |

At −6 dB the ordinary control frame is at the edge of what it decodes, and a Good fade (0.1 Hz
Doppler spread) lasts seconds; the tone floor's control frame, 14 dB more robust, would have
crossed it. An unanswered burst steps the sender's recommendation down two usable rungs (P9-7,
ADR-0012), and the sender's control frames go out in the family of the rung it would send at
(ADR-0009), so a burst's retries reach the floor. A poll's silence stepped nothing, and every
repeat went ordinary.

## 2. Decision

**From the second unanswered poll in a row, each poll's silence steps the recommendation down
as a burst's does** (`LinkConfig.poll_silences`, 2; `silence_step` rungs a silence). The polls
follow the recommendation to the floor, and so do the wait for their answer and the link timeout
that the answer arms. From the first OFDM rung the third poll is a floor poll. From
the top of either ladder the polls reach the floor with two floor polls (2 300 Hz) or three
(500 Hz) before `max_retries` ends the session. The answer puts the other station's
recommendation back (`_on_ack`), as after a burst.

The first silence repeats the poll in its family. Near the ordinary control frame's threshold a
poll or its answer is lost now and then on a path that carries the next one, and a floor poll
and its answer take 6.4 s where an ordinary exchange takes 0.9 s. At 0 dB the link sits on the
first OFDM rung, and two usable rungs below it is the floor, so stepping down on every silence
sent the first retry to the floor every time (§3).

## 3. Alternatives measured

Three rules on the engine with ADR-0031, the chat bench (`tools/bench_chat.py`, ADR-0027) on the
fading pipe with the floor's reading cap: −6 and 0 dB of the drop run (Good, Moderate, Poor; both
airs; 100 sessions a point, `first_trial` 100), 2 400 sessions a rule. Latency and keyed time are
the ratio to the engine without a rule, per point, both turn policies: the largest.

| Rule | "no response" | other drops | lines lost | a line at 0 dB | median line | keyed per line |
|---|---|---|---|---|---|---|
| none (ADR-0031) | 3 | 2 | 43 | — | — | — |
| every silence (`poll_silences = 1`) | 0 | 3 | 13 | +0.54 s | ≤ 1.14 | ≤ 1.11 |
| **from the second in a row** | **0** | **5** | **33** | **+0.06 to +0.19 s** | **≤ 1.05** | **≤ 1.03** |
| a repeated poll on the floor, recommendation untouched | 0 | 3 | 30 | +0.65 s | ≤ 1.14 | ≤ 1.11 |

Every rule ends the "no response" drops. The other drops are link timeouts at 0 dB on the
500 Hz air, and each rule adds its own sessions to them. They are ADR-0031's remaining fault,
the link timeout not following the sender down to the floor (a sender whose OFDM bursts go
unanswered steps down and is cut off during its first floor burst), which ADR-0033 fixes. With
it, this rule leaves none of them (§5). The rules that send the first repeat to the floor lose
fewer of them by timing alone: a floor exchange moves everything after it by seconds, and in
trial 117 (traced) the other station's next burst then fell after the fade instead of in it.
That is not a reason to pay half a second on every line at 0 dB.

The same comparison on the engine before ADR-0031, 300 sessions a point at 0 dB (3 600 a rule):
stepping down on every silence cost +0.44 to +0.54 s a line and 4–5 % more keyed time, and from
the second silence +0.08 to +0.16 s and 0.5–1.6 %.

## 4. Measured

The engine with ADR-0031 against this one, the same seeds: `tools/bench_chat.py` as above, today's
turn-taking (`base`) and ADR-0027's request (`request`), 100 sessions a point at −12, −6 and
0 dB (the drop run) and 30 a point from −12 to +12 dB (the grid).

| `bench_chat.py` | sessions | "no response" | other drops | lines lost | median, 90th percentile, keyed per line |
|---|---|---|---|---|---|
| today's, drop run | 1 800 | 2 → 0 | 4 → 5 | 55 → 43 of 27 612 | 1.00 (1.00–1.03) each |
| today's, grid | 900 | 1 → 0 | 1 → 1 | 19 → 10 of 13 710 | 1.00 (0.99–1.04) |
| request, drop run | 1 800 | 1 → 0 | 0 → 2 | 12 → 14 of 27 612 | 1.00 (0.99–1.05) |
| request, grid | 900 | 2 → 0 | 1 → 2 | 33 → 8 of 13 710 | 1.00 (0.97–1.04) |

Over the 5 400 sessions: "no response" 6 → 0, lines lost 119 → 75. The new other drops are the
fault named in §3: the drop run's three end as it describes (two traced), and ADR-0033's rule
removes all three.

`tools/bench_link.py` (2 kB, Good, Moderate and Poor, −12 to +12 dB, both airs, 20 sessions a
point): identical in all 600 sessions. A transfer polls only when it is idle.

`bench/baselines/poll_step_chat.csv` (`engine` `before` = with ADR-0031, `after`, and the two
rules not taken, `every-silence` and `floor-retry`, on the drop run's −6 and 0 dB points) and
`bench/baselines/poll_step_link.csv` have the rows.

## 5. Consequences

* Tests, model and port: `unanswered_polls_step_the_recommendation_down` (both airs, from the
  top of the ladder and from the first OFDM rung: the first silence repeats in the family, and
  the polls reach the floor within the retries) and
  `the_polls_step_down_to_the_floor_through_a_fade` (both airs: a minute and a half below the
  ordinary control frame after a strong start; the session ended "no response" before).
* With ADR-0033 as well, the drop run's 3 600 sessions lose two, both at −12 dB on Good (trial
  115 on both airs, the slow fade ADR-0027 found under every policy).
* With ADR-0028 (a poll's wait covers a late answer, on its branch), each ordinary silence lasts
  0.57–0.99 s longer, and the floor comes that much later. Stacked on ADR-0028 and ADR-0029, the
  model's and the port's link tests pass.
* `poll_silences = 1` is the every-silence rule, for benches that want it.
