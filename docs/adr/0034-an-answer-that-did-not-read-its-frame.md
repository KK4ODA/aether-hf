# ADR-0034: A POLL or TURN answered without being read goes again at once, on the floor

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`: `_unread_there`,
`_poll_floor`, `_control_floor`, the acknowledgement branch of `_on_control`), then the port
(`aether-link` `engine.rs`: `unread_there`, `poll_floor`). No frame, link protocol or
configuration change.

## 1. Context

ADR-0033 §3 left one drop of the chat bench's drop run on `master` that completes on either
engine alone: 500 Hz, ITU Moderate, 0 dB, trial 184, today's turn-taking. Traced
(`tools/bench_chat.py`, the engine at `ea81ae7`; B holds the turn with nothing to send, A
receives):

| time (s) | |
|---|---|
| 218.0 … 222.5 | Two ordinary polls from B go unanswered — A reads only the second, and B reads neither answer — and the third goes on the floor (ADR-0032). A reads it and answers on the floor |
| 229.2 | B reads the answer and takes A's recommendation back: the first OFDM rung, and with it ordinary polls |
| 234.2 | A's operator types a line |
| 239.2 | B polls, ordinary. A hears the preamble, cannot read the poll, and answers it on the floor with WANT_TX: the last frame it read from B was the floor poll |
| 243.8, 250.4, 257.0 | B takes that for the poll's answer and hands the turn over: three ordinary TURNs, each unread, each answered on the floor the same way |
| 263.6 | B takes the turn back: a readable acknowledgement says the other station is still receiving (ADR-0029) |
| 270.7 | 45 s after the floor poll, the last frame it read, A ends the session: "link timeout" |

A receiving station answers every frame whose preamble it hears: the frame may be the last of a
burst, or a poll it cannot read (ADR-0028). It answers with an acknowledgement, in the family it
last read the other station in (ADR-0009, ADR-0016). B read each of those answers and took it
for the answer to its frame. The floor's control frame is 15 dB more robust than the ordinary
one, so at 0 dB the answers got through where the frames they answered did not, and nothing in
them said so.

Two things did say so, and were not used:

* **A station that reads a TURN takes the turn**, and answers with its first burst or a poll. An
  acknowledgement after a TURN answered its preamble. ADR-0029 reads it as "the other station is
  still receiving", and repeats the TURN when the wait for the first frame of a burst runs out —
  in the TURN's own family.
* **An acknowledgement comes back in the family its sender last read the other station in**: a
  station that reads an ordinary poll answers it in the ordinary family, so a floor answer to an
  ordinary poll answered the preamble alone. An answer in the poll's own family cannot be told
  apart from one that read it.

How often, over the drop run on `master` (3 600 sessions), counted against the receiving
engine's decodes: of the 929 ordinary polls answered on the floor, every one had not been read;
of the 16 569 answered in the ordinary family, 1 734 (10 %); of the 44 027 floor polls answered
on the floor, 994 (2 %). No floor poll was answered in the ordinary family. 1 901 TURNs were
answered by an acknowledgement — 402 ordinary TURNs on the floor, 819 ordinary ones in the
ordinary family and 680 floor ones — and none of the 1 883 whose decoding could be matched had
been read. An answer in its frame's own family is mostly
lost with its frame when the path falls below that family, and the sender hears the silence
ADR-0032 steps down on; an answer in the more robust family is not lost, and says nothing.

## 2. Decision

**A POLL or a TURN the other station answered without reading goes again at once, on the
floor.**

* **A TURN answered by an acknowledgement** goes again a turnaround after the answer, on the
  floor — not in its own family when the wait for the other station's first frame runs out. The
  TURNs a station offers before it takes the turn back are counted as before (ADR-0029): an
  acknowledgement it reads says the other station is still receiving.
* **An ordinary POLL answered on the floor**: the TURN that answer prompts goes on the floor, and
  when it prompts nothing — no TURN, no burst — the poll goes again a turnaround after the answer,
  on the floor, not a keepalive later.
* **Until one is read.** The station's POLLs and TURNs go on the floor (`_control_floor`) until a
  POLL is answered in its own family — read, as far as can be told — or the turn changes hands,
  or the session ends. A floor poll answered in the ordinary family is taken as read: the other
  station last read this one in the ordinary family, and the floor is what it cannot read.
  Nothing else moves. The recommendation stays the other station's, and the data rung with it: a
  burst's acknowledgement says what it read.

On trial 184 the answer at 239.2 s sends the TURN on the floor; A reads it, takes the turn and
sends its line.

## 3. Alternatives measured

Each rule on the engine at `ea81ae7`, on the chat bench's drop-run points at −6 and 0 dB (Good,
Moderate and Poor, both airs, 100 sessions a point, `first_trial` 100, both turn policies:
2 400 sessions a rule), with the fading pipe, the floor's reading cap and bursts held to the key
time; and on **the fade** of §5's end-to-end test — the path falling from 15 dB to −12 dB, below
the ordinary control frame and above the floor's, while the called station holds the turn —
over ten seeds on each air, with a line waiting at the other station and with both stations
idle (40 sessions a rule, 600 s each). A line's time and the keyed time a line are the mean
change over the 0 dB sessions both engines completed.

| Rule | drops | lines lost | a line at 0 dB | keyed a line at 0 dB | the fade, of 40 |
|---|---|---|---|---|---|
| none (`ea81ae7`) | 1 | 12 | — | — | 0 |
| a floor answer to an ordinary POLL or TURN is a silence for ADR-0032's step-down | 0 | 0 | +0.03 s | +0.02 s | 0 |
| the receiving station recommends at most the floor's top rung when it answers an ordinary control frame it could not read | 0 | 0 | +0.35 s | +0.46 s (+3 %) | 40 |
| the next POLL or TURN in the family of an answer in the other family | 1 | 7 | +0.10 s | +0.13 s | 38 |
| the next POLL or TURN on the floor, when it falls due | 0 | 0 | +0.17 s | +0.24 s | 38 |
| as above, and a TURN again at once | 0 | 0 | −0.02 s | +0.24 s (+1.7 %) | 38 |
| **as above, and a POLL again at once (adopted)** | **0** | **0** | **−0.05 s** | **+0.30 s (+2.1 %)** | **40** |

* **The silence** (the step-down of ADR-0032, counting an unread answer as an unanswered poll):
  from the first OFDM rung, where the bench's 0 dB sessions sit, the second unread answer
  reaches the floor, which carries trial 184 — the cheapest rule on the bench. But a step is two
  usable rungs, and from the top of either ladder the floor is five or six unread answers away,
  each a keepalive apart, while the other station reads nothing: in every run of the fade it
  timed out first.
* **The receiving station's recommendation** covers every unread answer, those in the frame's
  own family too — the receiving station knows what it could not read — and old senders would
  follow it. But it moves the sender's data to the floor with its control frames, and at 500 Hz
  the first OFDM rung decodes 1.5 dB below the ordinary control frame, so at 0 dB it sent to the
  floor data the path carried (up to +11 % keyed time a line at a point). It acts on a detection
  that, on the air, may be noise, and a station offering the turn takes no recommendation today.
* **The family of the answer** leaves an ordinary TURN answered in the ordinary family to go
  again ordinary. Trial 120 (500 Hz, Poor, 0 dB, today's turn-taking) dropped that way: three
  ordinary polls and two ordinary TURNs unread over 45 s, every answer in the ordinary family
  and read.
* **On the floor when it falls due** pays the floor's 3.2 s where the ordinary repeat at 0 dB
  often gets through. **A TURN again at once** — the other station has said it is still
  receiving, and its first frame is not coming — pays that back. **A poll again at once**: when
  it waited for its keepalive, an idle station whose ordinary polls went unread was read by the
  other only every other keepalive, about every 31 s, and one floor poll lost to bad luck (a
  one-in-eight-thousand draw at −12 dB) ran past the other's 45 s: the two runs of the fade the
  rule before it lost. Again at once, every exchange of polls is read.

**Not built: an answer that says what it read.** Only the answer can say it for certain — a flag
in the acknowledgement (two of its four flag bits are unused), set when its sender read nothing
of the frame it answers. That is a change to what goes on the air: a new link protocol version,
and a station of this one would no longer talk to one of beta.70. What it would add over this
rule is a POLL unread and answered in its own family, which the rule cannot see. On the bench,
when the path falls below a family, the answers in it are lost with the polls they answer often
enough for ADR-0032's step-down to act on the silence, and no drop of the 5 400 sessions below
comes from the case. But the bench's fade is the same both ways, and on the
air a path can be several decibels better one way than the other (ND1J heard KK4ODA at about
0 dB on 80 m, and 5 dB better the other way, ADR-0017): an idle station's ordinary polls unread
while the other station's ordinary answers arrive would hold the link until the other timed
out. That is the case to look for in field recordings before a protocol change is worth making.

## 4. Measured

The engine at `ea81ae7` (`engine` `before`) against this one, the same seeds, on the fading pipe
with the floor's reading cap and bursts held to the key time. `tools/bench_chat.py` on Good,
Moderate and Poor, both airs, today's turn-taking (`base`) and ADR-0027's request (`request`);
a line's median and 90th-percentile time and the keyed time a line as the median of the
per-point ratios after/before, with their range:

| `bench_chat.py` | sessions | identical | drops | lines lost | median, 90th percentile, keyed per line |
|---|---|---|---|---|---|
| today's, −12, −6, 0 dB, 100 a point | 1 800 | 1 157 | 1 → 2 | 12 → 16 of 27 612 | 1.00 (0.99–1.04), 1.00 (0.95–1.02), 1.00 (0.99–1.04) |
| today's, −12 to +12 dB, 30 a point | 900 | 694 | 1 → 2 | 10 → 12 of 13 710 | 1.00 (0.94–1.05), 1.00 (0.96–1.03), 1.00 (0.99–1.05) |
| request, −12, −6, 0 dB, 100 a point | 1 800 | 1 203 | 4 → 0 | 32 → 0 of 27 612 | 1.00 (0.95–1.02), 1.00 (0.96–1.01), 1.00 (1.00–1.04) |
| request, −12 to +12 dB, 30 a point | 900 | 685 | 1 → 1 | 7 → 7 of 13 710 | 1.00 (0.97–1.07), 1.00 (0.95–1.01), 1.00 (0.98–1.07) |

Over the 5 400 sessions: drops 7 → 5, lines lost 61 → 35. Trial 184 completes. Every drop
before and after is a link timeout at −12 dB on ITU Good, the slow fade ADR-0027 found under
every policy, and they are not the same sessions: before, the request's trials 129 and 195 of
the drop run on both airs and one grid session under each policy; after, today's trial 180 of
the drop run on both airs and two grid sessions under today's policy and one under the
request. Traced (trial 180 and the grid's trial 9 after, trials 129 and 195 before, all at
2 300 Hz): the sender's floor bursts arrive and the acknowledgements of three of them in a row
are lost, and its link times out while the fourth is on the air. Trial 180 comes to it because
a TURN goes again as soon as the other station's acknowledgement says it was not read, 1.7 s
sooner, and the session runs on another timeline from there; 129 and 195 miss theirs so.

A line's time, averaged over the sessions both engines completed, is unchanged or shorter at
every SNR — by 0.05 s at 0 dB on the drop run and 0.17 s on the grid, at most 0.12 s elsewhere.
The keyed time a line grows at 0 dB, by 0.30 s (2.1 %) on the drop run and 0.21 s (1.4 %) on the
grid: floor control frames where ordinary ones went, some of which would have been read. At
the other SNRs it is within ±0.2 %.

`tools/bench_link.py` (2 kB, Good, Moderate and Poor, −12 to +12 dB, both airs, 20 sessions a
point): identical in all 600 sessions. A transfer polls only when it is idle, and its receiving
station has nothing to hand the turn over for.

`bench/baselines/answer_unread_chat.csv` and `bench/baselines/answer_unread_link.csv` have the
rows (`engine` before/after, `policy`, `first_trial` 0 the grid and 100 the drop run), and the
chat file the rules of §3 on the drop run's −6 and 0 dB points (`engine` `silence`, `receiver`,
`answer-family`, `floor-when-due`, `turn-at-once`).

## 5. Consequences

* Tests, model and port: `an_ordinary_poll_answered_on_the_floor_was_not_read` (both airs: the
  TURN it prompts goes on the floor; with nothing prompted the poll goes again a turnaround
  later, on the floor; a poll answered in its own family puts the next back in the ordinary
  family; a floor poll answered ordinary moves nothing to the floor);
  `a_turn_answered_by_an_acknowledgement_goes_again_at_once_on_the_floor` (an acknowledgement in
  either family: the TURN again a turnaround later, on the floor; once the turn has changed hands,
  ordinary again); and `an_answer_that_did_not_read_its_frame_sends_the_next_on_the_floor` (both
  airs, a line waiting and both stations idle: the fade from 15 dB to −12 dB with the called
  station holding the turn; before, both stations ended the session "link timeout" in every
  run). Each fails on `ea81ae7`.
* ADR-0030's `a_poll_is_not_repeated_over_its_answer_arriving` (both suites) built its case from
  a sender whose every poll stayed ordinary and was answered unread on the floor — the state
  this ADR ends. The sender's polls now alternate, ordinary and unread, then floor and read, and
  the test says so of the case it measures; its checks — no poll repeated over its answer, every
  answer heard — are unchanged.
* The air-interface spec says what a station does with an answer to its POLL or TURN that did
  not read it (§7.2), and that a sender's control frames go on the floor while that lasts (§3).
* A station whose rules admit only the floor (ADR-0018) answers on the floor what it read too,
  and the other station's polls then go ordinary and floor by turns: an exchange of floor frames
  every other poll. Nothing is lost but air time.
* What the drop run still loses is the slow fade at −12 dB on ITU Good, under every rule (§4).
