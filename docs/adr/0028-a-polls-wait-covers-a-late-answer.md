# ADR-0028: A poll's wait covers the answer to a poll heard and not decoded

**Status:** accepted, 2026-09-26. Model first (`aether_model/link/engine.py`:
`LinkEngine._send_poll`, `_unread_poll_delay`), then the port (`aether-link` `engine.rs`:
`send_poll`, `unread_poll_delay`; `sim.rs` gains the model simulator's per-frame SNR offset,
`with_frame_snr_offset`). No frame, link protocol or configuration change: a station with this
waits longer for the answer to its poll, and works with one of an earlier version either way
round. It completes ADR-0030 for the poll; on top of it the benches measure no difference (§3).

## 1. Context

Found by the chat bench study (ADR-0027 §7 (1)): a session at 500 Hz on ITU Good at −6 dB
(`tools/bench_chat.py`, trial 9) ended "no response" with both stations up and the path
carrying every frame but one kind. The sending station, idle, polled on the ordinary layouts —
its recommendation had climbed after a burst on the tone floor — and the receiving station,
whose last decoded frame was that floor burst, heard the poll's preamble and could not decode
the poll:

| time (s) | |
|---|---|
| 62.22–62.65 | B polls (ordinary, 0.43 s); A hears the preamble, cannot decode the poll |
| 63.65–66.85 | A answers when its quiet after the frame runs out, on the floor (3.2 s) |
| 66.65 | B's wait for the answer expires: B polls again, over the answer's last 0.2 s |
| | B loses the answer (it is keying), A loses the repeat (it is keying at its start) |
| 71.09 … | the next repeat is heard, answered late, and run into by the one after |
| 102.13 | nine unanswered polls: B ends the session, "no response" |

A receiving station answers a poll it decodes a turnaround after it, and one whose preamble it
heard but which it cannot decode all the same — `on_preamble` arms its acknowledgement for when
its quiet after the frame runs out (`_irs_reply_delay()`: the announcement time of the family it
last heard, the burst gap and a turnaround). That is 0.57 s after the poll's end when it last
heard an ordinary frame and 0.99 s when it last heard the floor. The sender waited for the answer
as if it always began a turnaround after the poll (`_wait_for("poll", _reply_control_s())`, no
responder delay), which leaves 0.8 s of slack — the turnaround, the detection latency and the
acknowledgement margin. The ordinary quiet fits in it; the floor's does not, and a floor answer
ends 0.19 s after the repeat has begun. A floor poll the other station cannot decode fares the
same (at −12 dB on Good nearly every session met it, §3). A burst's acknowledgement has been
waited for with the receiving station's quiet in it since ADR-0013; the poll's never was.

ADR-0030 (§7 (3) of the same study, which landed first) holds a retry past a frame heard
arriving — here the answer's preamble, announced 1.53 s after the poll — and so already mends
the trace above, where the sender expected a floor answer and waited 4.0 s. It holds a retry
only when the frame announces itself before the retry falls due, and left one case to this ADR:
a sender that expects an ordinary answer repeats its poll 1.23 s after it, before a floor answer
can announce itself.

## 2. Decision

**The poll's wait includes the quiet of a receiving station that heard the poll and could not
decode it** (`_unread_poll_delay`, `unread_poll_delay` in the port), as the responder delay of
`_wait_for` — the largest `_irs_reply_delay(f)` over the families the other station may last
have heard this one in: its own control family, and the family the other station's last answer
came in (`_peer_floor`), the two a burst's acknowledgement wait already considers. It is zero
when the PHY does not announce the poll's family: an undecodable poll is then never answered,
and the wait is what it was.

The wait grows by 0.57 s on the ordinary layouts and by 0.99 s where either side is on the
floor, and an unanswered poll is repeated that much later. Where the sender expects a floor
answer the wait now outlasts the answer (4.99 s against its end at 4.19 s). Where it expects an
ordinary one the retry falls due 1.81 s after the poll, after the latest moment any answer to it
announces itself (a floor answer, 1.53 s), so ADR-0030's hold always has the answer's preamble
to hold for. With both, no answer to a poll is run into by the poll's retry, in either family,
whether or not the poll was decoded — and the wait no longer depends on the answer's preamble
being heard to cover the answer the sender can predict.

## 3. What was measured

`tools/bench_chat.py` (ADR-0027: a call, then 10–20 lines typed 5–30 s apart, on the fading pipe
with the floor's reading cap, bursts held to the key-time limit), Good, Moderate and Poor, both
airs, the same seeds before and after; sessions dropped and lines lost, and a line's median and
90th-percentile time and the keyed time per line as the ratio after/before
(`bench/baselines/poll_wait.csv`).

**On the engine as the study found it** (the model at `7345694` and ADR-0027's, without
ADR-0030):

| Turn-taking, run | Sessions | Dropped | Lines lost |
|---|---|---|---|
| today's, −12, −6, 0 dB, 100 a point (`first_trial` 100) | 1 800 | 15 → 5 | 126 → 28 |
| today's, the same points, 200 more a point (`first_trial` 200) | 3 600 | 34 → 11 | 315 → 80 |
| today's, −12 to +12 dB, the four classes, 30 a point | 1 200 | 8 → 2 | 83 → 16 |
| ADR-0027's request, 100 a point | 1 800 | 13 → 7 | 104 → 34 |
| ADR-0027's request, 200 more a point | 3 600 | 34 → 13 | 342 → 88 |
| ADR-0027's request, −12 to +12 dB, 30 a point | 1 200 | 7 → 5 | 77 → 47 |

The first row's "before" is ADR-0027's "today" exactly. The latency and keyed-time ratios have a
median of 1.000 in every run and lie within 0.92–1.06 at every point; with today's turn-taking on
Good at −12 dB a line's median is 2–3 s faster and its 90th percentile 3–5 s, the late answers now
heard. The fault itself, counted in the first run as a poll keyed while the other station's frame
was on the air: 1 484 of 44 914 polls, every one over an acknowledgement, in 594 of the 1 800
sessions — 95 and 96 of the 100 at −12 dB on Good, where every frame is the tone floor's —
before; none after, and 6 % fewer polls.

**On master, with ADR-0030's hold** (`f17fec5` against this change; the rows other than today's
100-a-point run and the missed announcements ran the same code patched onto the model, which
reproduces both commits' 100-a-point rows for today's turn-taking exactly):

| Turn-taking, run | Sessions | Dropped | Lines lost |
|---|---|---|---|
| today's, 100 a point | 1 800 | 9 → 8 | 71 → 47 |
| today's, 200 more a point | 3 600 | 19 → 17 | 166 → 91 |
| ADR-0027's request, 100 a point | 1 800 | 1 → 4 | 20 → 25 |
| ADR-0027's request, 200 more a point | 3 600 | 14 → 12 | 112 → 98 |
| today's, 100 a point, a fifth of all preamble announcements missed | 1 800 | 100 → 101 | 788 → 850 |
| ADR-0027's request, the same | 1 800 | 84 → 81 | 698 → 716 |

The hold already keeps every retry off an answer it hears arriving: counted as above in today's
100-a-point run, no poll is keyed over the other station's frame with the hold alone or with both
(0 of 42 255 and of 42 321). Pooled over the 100- and 200-a-point runs, 5 400 sessions a policy:
28 → 25 sessions dropped and 237 → 138 lines lost with today's turn-taking, 15 → 16 and
132 → 123 with the request — a few sessions either way, within what these counts can tell
apart. A line's time and the keyed time are unchanged (every point
0.96–1.07, median 1.000). The runs with a fifth of all preamble announcements missed — where the
hold cannot act on the answer and only the wait keeps a retry off it — cost both engines ten to
eighty times the drops, since missed announcements break far more than the poll, and the wait's
share is lost in that. On this bench, then, the change adds nothing measurable to ADR-0030; it
is taken for §2's reasons: the wait now covers the answer the sender's own peer is built to send,
closes the case ADR-0030 left, and costs nothing measured.

`tools/bench_link.py` is identical in every column of every session, on the engine as found and
on master: the default grid (8 kB, three sessions a point, −4 to +20 dB, the four classes on the
logistic pipe; 500 Hz, and 2 300 Hz as found) and 2 kB transfers on the fading pipe with the
floor's reading cap (ten sessions a point, −12 to +12 dB on Good, Moderate and Poor, both airs).
A transfer polls only when it is idle.

## 4. What remains

Every remaining drop traced (500 Hz, Good, −6 dB, trials 122 and 188 of the 100-a-point run, and
trial 18 of the grid) is another fault, the one ADR-0030 §3 also found: an ordinary poll and its
ordinary answer both undecodable through a slow fade. The sender repeats the poll nine times,
about two seconds apart, and gives up after twenty seconds — which a fade on Good outlasts —
while the path would carry the tone floor. An unanswered burst steps the recommendation down
(`_back_off`, ADR-0012), and the burst's control frames follow it to the floor; an unanswered poll
does not, so its repeats stay on the ordinary layouts. Not changed here: it changes what an idle
link does in every fade.

## 5. Consequences

* The model: `_unread_poll_delay` and the poll's wait. Tests: the wait covers a floor answer to
  an ordinary poll and is unchanged without preamble reports
  (`test_a_polls_wait_covers_an_answer_to_a_poll_heard_and_not_decoded`); a session whose polls
  never decode — the call and acceptance on the floor, the poll ordinary, every POLL arriving
  far below its threshold (`TwoStationSim(frame_snr_offset=…)`) — lives on the late answers at
  both bandwidths and carries a message afterwards, where it ended "no response" before
  (`test_a_session_whose_polls_never_decode_lives_on_their_late_answers`).
* The port mirrors both (`engine.rs` tests, `tests/protocol.rs`); the Rust simulator gains
  `with_frame_snr_offset` (`sim::FrameSnrOffset`), the model simulator's `frame_snr_offset`.
* An unanswered poll is repeated 0.6–1.0 s later; nine of them take 5–9 s longer to end a dead
  session.
