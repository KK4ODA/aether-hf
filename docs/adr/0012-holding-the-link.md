# ADR-0012: Holding the link on a fading path (P9-7)

**Status:** accepted, 2026-09-23. Model first (`aether_model/link/engine.py`), then the port.

## 1. Context

The weak-signal plan (`docs/ROADMAP.md`, "The weak-signal plan from 2026-09-23") put P9-7
second: *a start that assumes a fading path*. The author's experience with VARA HF is that it
starts slow and climbs, and the question was whether Aether's links were unreliable because
ADR-0008 starts a session too high — at the mode the connect frame's SNR supports, less two.

P9-6 made the question answerable. The link bench's old lossy pipe judged each frame
independently at the per-class average error rate, and turned out two to three times
pessimistic at low SNR; the fading pipe (`aether_model.link.fading`) puts every frame through
its own stretch of a two-ray ITU-R F.1487 fade that both stations share, and agrees with the
real modem, run with one continuous fade per session, within about a fifth at six points
(`bench/README.md`). Everything below is measured on it: 30 sessions a point, 2 kB at
2 300 Hz and 1 kB at 500 Hz.

## 2. What the start is worth

Starting lower does not make sessions more reliable, on either air. Against ADR-0008's start:
starting at mode 0 (the old behaviour), a margin of 6 or 9 dB instead of 3, or four steps in
hand instead of two, every variant completed every 2 300 Hz session from 0 dB up on all four
channels — as ADR-0008's start does — and was slower, up to three times on a clean channel.
At 500 Hz the completions moved by a session or two either way, inconsistently (Good 0 dB:
28/30 for ADR-0008, 22–24/30 for the larger margins), and the time again went up. The start
stays as ADR-0008 left it.

## 3. What does fail

Every failed session ended the same way: **link timeout**, a session given up after 45 s
without a valid frame from the peer. Three causes, one per change below:

1. **One exchange at the 500 Hz floor is nearly half a minute** (six 4.2 s frames, a 2.2 s
   control frame, two turnarounds), and on a slow fade — ITU Good fades over seconds — a
   single missed exchange is common. A fixed 45 s gave up two sessions in three at −4 dB on
   Good and on Moderate.
2. **Silence never lowered the mode.** The ISS's recommendation moves only when an
   acknowledgement brings one, and a burst and its acknowledgement fade together. A session
   whose connect frame was measured on a peak sent its first bursts at a mode the path could
   not carry and repeated them — same codewords, as HARQ wants — until the link timed out:
   one session in twenty at 0–10 dB on the 500 Hz fading bench.
3. **The acknowledgement trampled the frame it was waiting for.** The IRS holds its ACK until
   the frame a preamble announced has ended, and it took that frame's length from the
   peer's last mode and its own recommendation. When the connect frames were ordinary and the
   ISS's first burst went out on the floor — its first mode came from a later connect frame
   measured in a fade — the IRS expected 1 s frames, answered 1.8 s into every 4.2 s one, and
   the two stations talked over each other until the link timed out. This is not a bench
   artefact: the same timer drives the daemon, and the floor has decoded live since beta.49.

## 4. Decision

1. **The link timeout spans whole exchanges** (`LinkConfig.link_timeout_exchanges` = 4): the
   session ends after the longer of `link_timeout_s` (45 s) and four exchanges at the family
   the link runs in — the longest data frame either side sends or may send next, and that
   family's control frame. On the ordinary layouts that is 30 s, so 45 s stands; on the
   floor it is 113 s. Three exchanges left a session in ten at −4 dB; five changed nothing.
2. **An unanswered burst steps the ISS's recommendation down** two usable modes
   (`LinkConfig.silence_step`). The frames stranded above are re-encoded on the way, after
   `max_combines` transmissions (the mechanism P6-7 added), and the next acknowledgement puts
   the peer's recommendation back.
3. **`on_preamble` takes the announced frame's air time.** The preamble names the layout; the
   ACK deadline is that frame's end plus the reply delay. A PHY that cannot say keeps the old
   guess.

## 5. Measured (fading pipe, 30 sessions a point)

| 500 Hz | before | after |
|---|---|---|
| Good −6 / −4 / 0 dB | 1 / 10 / 28 | 17 / **30** / **30** |
| Moderate −6 / −4 / 0 / 3 dB | 1 / 10 / 27 / 28 | 2 / **27** / **30** / **30** |
| Poor −4 / 0 dB | 28 / 29 | **30** / **30** |
| 6–15 dB, any channel | 29–30 | **30** |
| AWGN, −8 to 15 dB | 30 | 30 |

At 2 300 Hz nothing moved: every point from −2 dB up completes as before (Good −2 dB 29/30).
Session times are unchanged where both complete, a little longer at −4 dB, where sessions
that used to die now finish.

## 6. Costs and limits

- A link that has really gone is given up after about two minutes on the floor instead of
  45 s.
- An acknowledgement lost while the data got through costs two steps until the next one
  arrives.
- Below −4 dB the 500 Hz floor itself runs out (Good −6 dB 17/30, Moderate and Poor 2–4/30):
  that is P9-8's business, not the link layer's.

## 7. Rejected

Larger start margins and a start at mode 0 (§2); a longer base timeout — 60 or 90 s instead
of 45 — with the exchange rule, which rescued one session in 480 and delays the end of every
dead ordinary link; five exchanges instead of four.
