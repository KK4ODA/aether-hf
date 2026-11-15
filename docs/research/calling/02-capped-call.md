# The capped call on the model bench

*ADR-0126, judged by its §5 and §7.2, the criteria registered before the runs. Run 2026-11-15 in
a cloud container, on the model only. The author accepted §5's results and the revision went on to
the port. The first design (§1–§4) is removed from the model since; it is at `343be84`, where these
commands run. The revision's commands below take `--calls capped` alone now: `--details-in-call`
was folded in and the pause defaults to 15 s.*

## 1. What was run

The capped call is built in the model behind `GrantConfig.capped_call` (`frames.py`: `CallFrame`,
`AcceptFrame`, `call_codes`; `granted.py`: the plans, the three kinds of try, the answer rule,
the details at the head of the caller's data). With the switch off the engine is unchanged: the
granted and link vectors regenerate byte for byte. `model/tests/test_capped_call.py` holds the
frames, the plans, the 20 s limit, the answer rule, the details check and data behind the details.

`tools/bench_calls.py` gained `--calls capped`, `--heard none|true|<dB>`, `--again` (a second call
after a failed one), `--step` and `--what call`. Four arms, each on both airs, AWGN, Good, Moderate
and Poor, at −20 … +8 dB and a cell nobody hears (−60 dB), 30 calls a cell, a 0.1 s clock:

    python tools/bench_calls.py --variant today --wait growing --calls alternate --again --step 0.1 --what call --snr=-60,-20,-18,-16,-14,-12,-10,-8,-4,0,4,8 --out today.csv
    python tools/bench_calls.py --variant capped-unheard --calls capped --heard none  (same options)
    python tools/bench_calls.py --variant capped-heard   --calls capped --heard true  (same options)
    python tools/bench_calls.py --variant capped-stale   --calls capped --heard 10    (same options)

`capped-heard` is the station heard at the path's SNR: S S under +0.9 dB, C C C S over it.
`capped-stale` is a station heard strong (+10 dB) whatever the path is now. Each cell's counts and
medians are in `data/capped-call-cells.csv`. The 500 Hz air gives the 2 300 Hz air's counts in all
but two cells, so the tables below are 2 300 Hz.

As registered, a capped call counts as connected once the called station has the caller's details;
today's once both stations are in the session. The tables give both moments for the capped call.

## 2. Verdict

| criterion | result |
|---|---|
| **R1**, a call nobody answers ends within 20 s | **passes**: H H 16.5 s, S S 15.6 s, C C C S 15.7 s, on both airs (today: 135–253 s, median 200 s) |
| **R2 (a)**, one capped call reaches as often as today's, from −14 dB up | **fails** on Good at −14 dB: 24 against 30 (also 28 against 30 at −12) |
| **R2 (b)**, two capped calls reach as often as today's one, every cell | **fails** on Good at −16, −14 and −12 dB, by 1–2 calls (27/29, 28/30, 29/30) |
| **R3**, the capped call faster at the median where both reach 25 of 30 | **fails** in 50 cells: on every path under about 0 dB the details take longer than today's call |
| **R4**, the informed openings no worse than H H | **narrow miss**: one cell (Good 0 dB, S S) 0.2 s slower at the median; otherwise as fast or faster, and C C C S 3.5–6.5 s where H H takes 6.7–9.7 s |

## 3. What the numbers say

**The call itself is faster everywhere.** Both stations are in the session at a median of 7.6 s
with nothing heard (4.8 s from +4 dB up), 7.2 s when heard (1.6 s on a strong path), against
11.5 s today. It reaches more often than today at the floor's edge on Moderate and Poor (−18 dB:
16 against 3, 14 against 2). Pooled over every cell, one capped call connects 2 312 times of 2 640,
today's 2 272.

**The details cost more than the call saves on a weak path.** The caller's connect body is the
first data of the session. On a weak path the master's first transmission is a floor header and a
floor data frame, and the called station has the callsigns 8.9 s after the session is up (16.5 s
from the call's start, AWGN −18 … −4 dB). On a fading path the first data frame often goes at a
rung the path refuses: the rate controller chooses it from the floor's reading of the call, and at
−10 dB on Moderate it chose rung 5 (tone100-153) in two calls of three, which failed and went
again an exchange later (26.1 s). Today's call has the callsigns in the call, and its 11.5 s
includes them.

**Two tries are too few on the slowly fading Good path.** ITU Good fades for many seconds; two
tries 8 s apart often fall in one fade, where today's eight tries spread over 200 s get through.

**A stale reading costs reach.** Called as heard strong when the path has gone weak, C C C S
spends about 7 s on quick tries the path cannot carry and has one floor try left: on Good at −14 dB
18 of 30 against 24, on Moderate at −14 dB 25 against 30.

2 300 Hz, connected of 30, and medians (`—`: none connected):

**Good**

| SNR | today | capped, one call | two calls | in the session | callsigns known | heard at the path's SNR |
|---|---|---|---|---|---|---|
| −18 dB | 23, 45.5 s | 10 | 25 | 7.6 s | 38.9 s | S S, 11, 35.6 s |
| −16 dB | 29, 45.4 s | 20 | 27 | 7.6 s | 35.2 s | S S, 18, 33.5 s |
| −14 dB | 30, 11.5 s | 24 | 28 | 7.6 s | 26.1 s | S S, 24, 25.7 s |
| −12 dB | 30, 11.5 s | 28 | 29 | 7.6 s | 26.1 s | S S, 27, 25.7 s |
| −8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 21.3 s | S S, 30, 16.1 s |
| −4 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 16.5 s | S S, 30, 16.1 s |
| +0 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 13.1 s | S S, 30, 13.3 s |
| +4 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 9.7 s | C C C S, 30, 6.2 s |
| +8 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 6.7 s | C C C S, 30, 3.5 s |

**Moderate**

| SNR | today | capped, one call | two calls | in the session | callsigns known | heard at the path's SNR |
|---|---|---|---|---|---|---|
| −18 dB | 3, 49.9 s | 16 | 20 | 15.8 s | 48.3 s | S S, 11, 43.4 s |
| −16 dB | 21, 42.5 s | 26 | 29 | 7.6 s | 34.4 s | S S, 26, 38.5 s |
| −14 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 26.1 s | S S, 30, 25.7 s |
| −8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 26.1 s | S S, 30, 25.7 s |
| −4 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 16.5 s | S S, 30, 16.1 s |
| +0 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 13.1 s | S S, 30, 12.7 s |
| +4 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 9.7 s | C C C S, 30, 6.5 s |
| +8 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 9.7 s | C C C S, 30, 6.5 s |

**Poor**

| SNR | today | capped, one call | two calls | in the session | callsigns known | heard at the path's SNR |
|---|---|---|---|---|---|---|
| −18 dB | 2, 50.2 s | 14 | 24 | 7.6 s | 41.7 s | S S, 13, 41.3 s |
| −16 dB | 25, 11.5 s | 30 | 30 | 7.6 s | 26.1 s | S S, 29, 25.7 s |
| −14 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 26.1 s | S S, 30, 25.7 s |
| −8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 26.1 s | S S, 30, 25.7 s |
| −4 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 16.5 s | S S, 30, 16.1 s |
| +0 dB | 30, 11.5 s | 30 | 30 | 7.6 s | 13.1 s | S S, 30, 12.0 s |
| +4 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 9.7 s | C C C S, 30, 6.5 s |
| +8 dB | 30, 11.5 s | 30 | 30 | 4.8 s | 9.7 s | C C C S, 30, 5.0 s |

AWGN: today 30 of 30 at 11.5 s from −18 dB up; the capped call 30 of 30, in the session at
7.6 s, callsigns known at 16.5 s up to −4 dB, 9.4 s at 0 dB, 6.7 s (C C C S: 3.5 s) from +4 dB.

## 4. What it points to (not built, not benched)

R1 is what the author asked for, and the call alone delivers it. What fails is the details step
and the two-try plan on slow fades. Three changes, each a candidate to register before it runs:

* **Keep the callsigns in the call.** A both-ways try made of today's two call frames, back to
  back (the OFDM data frame, 1.05 s, and tone-24, 5.36 s, each the full 19-byte connect body),
  answered by the short control-frame acceptance (§2.1's ACCEPT, 0.43 s or 3.2 s). No codes, no
  details step, `CONNECTED` at the acceptance as today. Estimated from the engine's timing: a good
  path 7.9 s, a weak path 10.7 s, two tries nobody answers 22 s (about 2 s over the cap; a quick
  first try on a known strong path stays as C C C S's 3.5 s). This would replace the codes, the
  details at the head of the stream and their check.
* **If the details stay in the session, send them at the floor's most robust rung**, not the rate
  controller's first choice: the fading paths' second exchange (about 10 s) goes, the first floor
  exchange (about 9 s) stays.
* **On slow fades, space the two tries.** Silence between them holds nobody's channel: a pause of
  some 15 s before the second try spreads the call over a fade, at no cost in channel time, and
  answers R2 on Good if the pause is long enough. Its length wants a bench of its own.

The contention test (two stations that cannot hear each other) and the 200 Hz offset test need the
harness, so the port, and come after the author's choice of revision.

## 5. The revision: the callsigns in the call, and a pause (ADR-0126 §7)

Registered in ADR-0126 §7 (commit `18bd978`) before these runs, at the author's decision. Built
behind two more switches (`GrantConfig.call_details_in_call`, `call_pause_s`); off, the vectors
regenerate byte for byte. Each try carries today's connect body in the robust OFDM data frame,
tone-24, or both back to back; the acceptance is the short control frame; a silent 15 s pause goes
before the plan's last try. Same bench, same cells, same `today` runs:

    python tools/bench_calls.py --variant rev-unheard --calls capped --details-in-call --heard none --pause 15 --again --step 0.1 --what call --snr=-60,-20,-18,-16,-14,-12,-10,-8,-4,0,4,8 --out rev-unheard.csv

and `rev-heard` (`--heard true`), `rev-stale` (`--heard 10`), `rev-unheard-p0` (`--pause 0`) and
`rev-unheard-p30` (`--pause 30`). Counts and medians are in `data/capped-call-cells.csv` beside the
first runs'.

| criterion | result |
|---|---|
| **R1′**, channel time at most 22.5 s, over within 40 s | **passes**: H H 22.1 s (over at 37.1 s), S S 20.0 s (35.0 s), C C C S 19.7 s (34.7 s), both airs |
| **R2 (a)**, one call from −14 dB up | **passes** |
| **R2 (b)**, two calls against today's one, every cell | **narrow miss**: Good −16 dB, 27 against 29, both airs; every other cell passes |
| **R3**, faster at the median | **passes**: 10.4 s (H H) and 9.3 s (S S) where today takes 11.5 s; 7.6 s on a good path with nothing heard, 2.2 s heard strong |
| **R4**, the informed openings | **passes** |

**The narrow miss.** In three of the 30 Good −16 dB draws (trials 2, 11 and 15) today's call
connected at its third or fifth try, after 45.5, 94.0 and 100.5 s; the revised call's two calls,
which span about 74 s with their pauses, both fell in the fade. Good fades for longer than the
pause on these draws. A third call, or a longer pause, would reach them; neither was registered.

2 300 Hz, connected of 30 and the median time to the session (the 500 Hz air gives the same counts):

**Good**

| SNR | today | revised, one call | two calls | median | heard at the path's SNR |
|---|---|---|---|---|---|
| −18 dB | 23, 45.5 s | 19 | 23 | 10.4 s | S S, 19, 34.3 s |
| −16 dB | 29, 45.4 s | 25 | 27 | 10.4 s | S S, 21, 9.3 s |
| −14 dB | 30, 11.5 s | 29 | 30 | 10.4 s | S S, 28, 9.3 s |
| −12 dB | 30, 11.5 s | 29 | 30 | 10.4 s | S S, 29, 9.3 s |
| −8 … +0 dB | 30, 11.5 s | 30 | 30 | 10.4 s | S S, 30, 9.3 s |
| +4, +8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | C C C S, 30, 2.2 s |

**Moderate**

| SNR | today | revised, one call | two calls | median | heard at the path's SNR |
|---|---|---|---|---|---|
| −18 dB | 3, 49.9 s | 5 | 10 | 10.4 s | S S, 8, 21.8 s |
| −16 dB | 21, 42.5 s | 22 | 27 | 10.4 s | S S, 20, 9.3 s |
| −14 … +0 dB | 30, 11.5 s | 30 | 30 | 10.4 s | S S, 28–30, 9.3 s |
| +4, +8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | C C C S, 30, 2.2 s |

**Poor**

| SNR | today | revised, one call | two calls | median | heard at the path's SNR |
|---|---|---|---|---|---|
| −18 dB | 2, 50.2 s | 3 | 6 | 36.4 s | S S, 5, 9.3 s |
| −16 dB | 25, 11.5 s | 27 | 29 | 10.4 s | S S, 24, 9.3 s |
| −14 … +0 dB | 30, 11.5 s | 30 | 30 | 10.4 s | S S, 28–30, 9.3 s |
| +4, +8 dB | 30, 11.5 s | 30 | 30 | 7.6 s | C C C S, 30, 2.2 s |

AWGN: 30 of 30 from −18 dB up, 10.4 s (H H) and 9.3 s (S S) up to +0 dB, 7.6 s and 2.2 s above; at
−20 dB 12 of 30 against today's 3.

**The pause** (reported, not judged; 2 300 Hz, one call / two calls of 30, nothing heard):

| cell | today | first design, no pause | revised, no pause | revised, 15 s | revised, 30 s |
|---|---|---|---|---|---|
| Good −18 dB | 23/27 | 10/25 | 13/24 | **19/23** | 16/18 |
| Good −16 dB | 29/29 | 20/27 | 20/28 | **25/27** | 23/26 |
| Good −14 dB | 30/30 | 24/28 | 26/29 | **29/30** | 28/29 |
| Moderate −16 dB | 21/28 | 26/29 | 23/28 | 22/27 | 21/26 |
| Poor −16 dB | 25/30 | 30/30 | 28/30 | 27/29 | 24/28 |

The pause is what brings Good near today's reach; 30 s gains nothing over 15 s. On Moderate and Poor
at the floor's edge the first design reached more often than the revision (Moderate −18 dB: 16
against 5): its floor try was the floor control frame, about 2 dB deeper on fading than tone-24, the
revision's.

**Stale knowledge** (reported): called as heard strong on a path gone weak, C C C S waits for its
quick tries and the pause, then has one floor try: the median 34.0 s from −18 to −8 dB, and fewer
calls through (Good −16 dB: 17 against 25; Moderate −16 dB: 10 against 22). The heard list's 15
minutes are what keep that rare; a shorter window, or a floor try earlier in C C C S, is the
author's call.

**Where this leaves the call.** Against today's call on this bench, the revision gives the channel
back in about 22 s instead of about 205 s when nobody answers, connects a good path in 7.6 s (2.2 s
when the station was heard strong) instead of 11.5 s, a weak one in 9.3–10.4 s, and reaches as often
except on Good's slowest fades at −16 dB and below, where a second call does not always make up for
today's eight tries. The harness's tests (contention, a station 200 Hz off) and the port remain.
