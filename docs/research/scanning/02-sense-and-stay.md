# A scan that stays on a call under way: the detector, and the numbers it gives

*For ADR-0129 §11. Run 2026-11-17 in a cloud container on the model at `65d89e7`, after S4 failed
(`01-results.md`). The author's design: a gateway need not sit a whole gap between a caller's tries
on a quiet dial; it can leave after a few seconds unless it notices a try under way, and then
stay for the next. The author asked that the detector be measured first, and that the number of
dials be capped where it is best. Scripts: `scratch/notice.py` (the detector),
`scratch/sense_timing.py` (the timing it gives).*

## 1. What a gateway that arrives partway through a try notices

`notice.py tries`: the caller's real first try to a station not heard lately (the model's engine:
the OFDM call frame at the robust rung, 1.05 s, then tone-24 with the same request, 5.36 s) through
the model's channel; a fresh receiver (the gateway just moved there) starts listening at a point of
the try, half a second after it arrived there (the move's deafness), and the first thing it
notices is recorded: an OFDM candidate, a tone frame announced as arriving, or a frame reported.
Both airs, AWGN and Moderate, +10, −10 and −16 dB, nineteen points from 1 s before the try to
6.2 s into it, six draws each.

Share of listens that noticed something at confidence 1.1 or more within 3.5 s of arriving (3 s of
listening), by where the gateway arrived (seconds into the try):

| | −1.0 … 2.6 | 3.0 | 3.4 … 5.4 | 5.8, 6.2 |
|---|---|---|---|---|
| AWGN and Moderate, +10 dB, both airs | 6 of 6 everywhere | 6 of 6 | 6 of 6 | 0 |
| −10 dB | 6 of 6 | 4–6 of 6 | 6 of 6 | 0 |
| AWGN −16 dB | 6 of 6 | 0–2 of 6 | 6 of 6 | 0 |
| Moderate −16 dB, 2300 Hz | 4–5 of 6 | 1 of 6 | 3 of 6 | 0 |
| Moderate −16 dB, 500 Hz | 3–4 of 6 | 2 of 6 | 5–6 of 6 | 0 |

* **A try is noticed from any point of it but its last second.** The tone frame's three sync blocks
  (at its start, after 45 % of its data and at its end) are each announced as an arrival by the
  receiver as it stands, without the block before them: a gateway that arrives after a try's start
  notices its middle or its end block within about 1.1–1.5 s.
* The one gap is a gateway arriving about 3.0 s into the try: its listening starts just after the
  middle block, and the end block's announcement comes after 3 s of listening. A dwell of 4 s
  (3.5 s of listening) closes it on good paths.
* A gateway that arrives before the tone frame's start reads the try as one listening all along
  does: at −16 dB on Moderate 4 of 6 at 2300 Hz and 3 of 6 at 500 Hz, about the S4 condition.

**Noise** (`notice.py noise`, ten minutes per air): at 2300 Hz one tone announcement at confidence
1.03; at 500 Hz four OFDM candidates, the largest 1.07, each reported as a failed frame. Nothing at
1.1 or more. So a notice is counted from confidence 1.1: noise then costs nothing measurable, and a
try under way loses at most one draw in six of its notices at −16 dB against counting from 1.0.

## 2. The timing it gives

`sense_timing.py` plays the rule against the engine's tries (0, 11.0, 22.0, 33.5, 45.1, 57.1,
69.1 s …), with the measured notices by arrival point, the measured share of tries read from their
start, and the measured noise, over random phases of the scan and the dial the caller is on (3 000
calls a cell). A gateway on a quiet dial leaves after the dwell; one that hears a try from its start
reads it; one that notices a try under way stays 19.2 s (the gap the engine leaves before its next
try, a try moved past a frame heard arriving, and the move's deafness) and reads the next.

Calls read, 4 s dwell, notices from 1.1:

| dials | good paths and −10 dB, call of 40 s | Moderate −16 dB, 500 Hz, 60 s | Moderate −16 dB, 2300 Hz, 60 s |
|---|---|---|---|
| 2 | 100 % | 91 % | 97 % |
| 3 | 100 % | 82 % | 86 % |
| 4 | 100 % | 85 % | 96 % |
| 5 | 100 % | 76 % | 85 % |
| 6 | 94 % | 72 % | 78 % |
| 8 | 90 % (99 % at 60 s) | 63 % | 70 % |

A 3.5 s dwell does a few points worse on good paths at 3 and 6 dials (the gap of §1); 4 s is taken.
The median call is read at the caller's second try (11 s) at every count. The model takes tries as
independent, where a fading path makes neighbouring ones alike: the harness's numbers will be lower
at −16 dB, as S4's control was against its timing model.

## 3. The choices

* **Dwell: 4.0 s** on a dial where nothing is noticed: the move's 0.5 s and 3.5 s of listening.
* **A notice:** an arrival announced, an OFDM candidate, or a frame reported that did not decode,
  at confidence 1.1 or more; a frame decoded for another station is not one.
* **The stay after a notice: 19.2 s** (19.6 s on the harness's stations), the dwell the scan had
  before: long enough for the caller's next try to begin and be announced, after which the hold
  keeps the dial while it is read. Stays do not chain on their own notices beyond the one gap.
* **What a caller must keep trying (`scan_s`):** a cycle of the dials and two stays,
  `n × 4.0 + 2 × 19.2` s rounded up: 47 s for 2 dials, 51 for 3, 55 for 4. Two stays, so a try
  noticed is followed by two that are read; the table's 60 s column is that much call, near enough.
* **The cap: 4 dials.** Up to 4 a scanning gateway reads every call on a good path and, in the
  model, more than 80 % at the weak path S4 is set at, both airs; at 5 the 500 Hz weak path falls
  to 76 %, at 6 good paths start to lose calls. Three-quarters of the Winlink HF gateways listed on
  2026-11-01 publish 4 dials or fewer (median 3, from the gateway list Pat ships).

## 4. On the harness, and the lock it found

The S3/S4 grid (`make_grid.py`, the dials 4 s each) on the daemon at `ed42e63`: S4 passed, 8 of 10
weak calls at 2300 Hz and 9 of 10 at 500 Hz (6 and 5 with the first design, 9 and 10 for a gateway on
a fixed dial). S3's good paths connected every call in a median 11–20 s and at most 31.5 s, but one
phase of the ten, the same in all four three-dial cells, never connected: three dials at exactly
4 s are a 12 s round, the caller's tries come 11.4–12.4 s apart on the harness's stations, and every
visit to the caller's dial fell in the quiet half of its cycle. The timing model had averaged over
phases and hidden it; asked for the worst of twenty phases, it shows it (three dials, good path: 93 %
on average, 64 % at the worst phase).

Each dwell drawn longer by a random part breaks the lock. Worst phase of twenty, `scan_s` from the
longest dwell:

| | 2 dials | 3 dials | 4 dials |
|---|---|---|---|
| 4 s exactly, good path | 100 % | 64 % | 100 % |
| 4–5 s, good path | 100 % | 100 % | 100 % |
| 4–5 s, Moderate −16 dB, 2300 / 500 Hz | 90 / 81 % | 86 / 77 % | 86 / 72 % |
| 4–6 s, good path | 100 % | 100 % | 98 % |
| 4–6 s, Moderate −16 dB, 2300 / 500 Hz | 80 / 74 % | 91 / 78 % | 79 / 67 % |
| a fresh order each round, 4 s, good path | 100 % | 87 % | 97 % |

A second of jitter is taken (`DWELL_JITTER_S`): it reads every call at every phase on good paths, and
more of the weak path than two or three seconds, which lengthen the round.

**The linger, on S4's draws** (the daemon at `ed42e63`, its notices' linger taken out, the contact's
kept): 3 of 10 at 2300 Hz and 5 of 10 at 500 Hz, against 8 and 9 with it; no draw connected without
it that failed with it. The linger earns its place.

