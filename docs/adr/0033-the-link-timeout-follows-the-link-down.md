# ADR-0033: The link timeout follows the link down to the floor

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`, `_transmit`), then
the port (`aether-link` `engine.rs`, `transmit`). No frame, link protocol or configuration
change.

## 1. Context

P9-7 (ADR-0012 §4.1) made silence end a session only after the longer of `link_timeout_s`
(45 s) and four whole exchanges at the family the link runs in: 45 s on the ordinary layouts,
about two minutes on the tone floor. The deadline was reckoned when a frame arrived, from the
family then. Nothing reckoned it again when the link changed family with no frame heard, which
is how a link falls to the floor in a fade. A sender whose OFDM bursts go unanswered steps its
recommendation down (P9-7). Its unacknowledged frames keep their rung for `max_combines`
transmissions and are then re-encoded on the floor (P6-7, ADR-0031). The first tone burst goes
out half a minute after the last frame heard, lasts 27 s, and the deadline armed at that last
frame ran out during it.

Found on the chat bench (`tools/bench_chat.py`, ADR-0027) as the drops ADR-0031 and ADR-0032
left at 0 dB, all on the 500 Hz air, where a line fills six-frame bursts at the first OFDM rung.
Trial 199 of the drop run (ADR-0027's request, ITU Good) with ADR-0032:

| time (s) | |
|---|---|
| 306.74 | B takes the turn and sends six frames at rung 4; the last frame it heard is A's TURN, just before |
| 314.9 … 331.2 | three more bursts of the same frames: A's acknowledgements are lost in a fade, and B steps down to the floor |
| 339.35 | the frames' four tries are spent: B re-encodes them on the floor, five tone frames, 26.8 s |
| 351.74 | 45 s after the TURN: B ends the session, "link timeout", during the tone burst A could decode |

The same fault ended trials 117 and 180 (Poor) with ADR-0031, and 183 and 186 (Good) with
ADR-0032.

## 2. Decision

**Whenever a station sends, its link deadline becomes the last frame heard plus the timeout for
the family the link runs in now, and never less than it was** (`_transmit`, `transmit`). A
sender that has stepped down to the floor sends its next burst under a deadline four floor
exchanges long. So does a receiver whose acknowledgement recommends the floor. A frame heard
arms the deadline as before, from that frame.

The deadline still counts from the last frame heard: a station's own transmissions never keep a
dead link alive. They only choose which family's timeout applies.

## 3. Measured

The engine with ADR-0031 and ADR-0032 (`engine` `before`) against this one, the same seeds, on
the fading pipe with the floor's reading cap and bursts held to the key time. `tools/bench_chat.py`
on Good, Moderate and Poor, both airs, today's turn-taking (`base`) and ADR-0027's request
(`request`):

| `bench_chat.py` | sessions | identical | drops | lines lost | median, 90th percentile, keyed per line |
|---|---|---|---|---|---|
| today's, −12, −6, 0 dB, 100 a point | 1 800 | 1 797 | 5 → 2 | 43 → 24 of 27 612 | unchanged (≤ 1.01) |
| today's, −12 to +12 dB, 30 a point | 900 | 900 | 1 → 1 | 10 → 10 of 13 710 | unchanged |
| request, −12, −6, 0 dB, 100 a point | 1 800 | 1 798 | 2 → 0 | 14 → 0 of 27 612 | unchanged (≤ 1.01) |
| request, −12 to +12 dB, 30 a point | 900 | 899 | 2 → 1 | 8 → 7 of 13 710 | unchanged (≤ 1.01) |

The six sessions that change are the six that dropped at 0 dB on the 500 Hz air, and all six
complete. The four drops left are link timeouts at −12 dB on ITU Good (trial 115 of the drop run
on both airs, and one of the grid on each turn policy at 2 300 Hz), the slow fade ADR-0027 found
under every policy.

`tools/bench_link.py` (2 kB, Good, Moderate and Poor, −12 to +12 dB, both airs, 20 sessions a
point): identical in all 600 sessions.

ADR-0031, ADR-0032 and this one together, against the engine at `f17fec5`, over the same 5 400
chat sessions: 17 drops and one stall → 4 drops, lines lost 157 → 41. By cause: "no response"
6 → 0, link timeouts 11 → 4, stalls 1 → 0.

`bench/baselines/link_deadline_chat.csv` and `bench/baselines/link_deadline_link.csv` have the
rows (`engine` before/after, `policy`, `first_trial` 0 the grid and 100 the drop run).

On `master` with ADR-0028 and ADR-0029 merged (their branches, not yet landed), the drop run's
3 600 sessions without and with the three: drops 11 → 5, "no response" 3 → 0, lines lost 77 →
44, latency and keyed time within 0.98–1.04. Four of the five are the −12 dB slow fade. The
fifth (500 Hz, Moderate, 0 dB, trial 184) completes on either engine alone. There, after two
unanswered polls, B's third went on the floor, and A, having decoded it, answered everything it
heard and could not decode on the floor. B's next ordinary poll was answered that way, and so
were its three ordinary TURNs. B read each answer, took the turn back after the third TURN
(ADR-0029), and stepped nothing down. A decoded nothing from B for 45 s and ended the session.
A station cannot tell an answer to its frame from an answer to the frame's preamble alone, and
nothing here changes that.

## 4. Consequences

* Tests, model and port: `the_link_timeout_follows_the_sender_down_to_the_floor` (the deadline
  after an OFDM burst and after the step to the floor) and
  `a_session_whose_link_falls_to_the_floor_is_not_cut_off_on_the_way` (500 Hz: the called
  station sending at the first OFDM rung when the path falls below it and below the ordinary
  control frame; both stations ended the session "link timeout" before).
* A link that goes dead while its stations step down to the floor is given up after four
  floor exchanges from the last frame heard, about two minutes, instead of 45 s. That is
  ADR-0012 §6's cost, now paid wherever the floor is where the link ends up.
