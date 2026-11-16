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

## 6. The harness's A/B (ADR-0126 §8)

Run 2026-11-16 in a cloud container (Linux, 4 cores), two runs at a time, on two daemons built from
`08432a0`: **T** as committed (today's call) and **C** with `capped_call: true` in
`GrantConfig::default`. Both ran the same draws, the commands of ADR-0126 §8.2, with
`--drop-passing-audio`. One row a run is in `data/capped-T.csv` and `data/capped-C.csv`; the `bench`
column names the criterion, `H5r` the two extra draws H5 asks for. The capped call works in the
daemon: on `quick-awgn-2300` one both-ways try and an OFDM acceptance, connected at 11 s against 15 s.

### 6.1 The results

**H1, nobody answers** (3 draws each; the same in every draw):

| | tries | radiated | first try's start to last try's end |
|---|---|---|---|
| T, 2 300 Hz | 8 | 25.8 s | 215.4 s |
| T, 500 Hz | 8 | 25.8 s | 212.1 s |
| C, 2 300 Hz | 2 | 12.9 s | **36.1 s** |
| C, 500 Hz | 2 | 12.9 s | 32.8 s |

At 2 300 Hz the second try went at 500 Hz (§8.1's move) and waited 3.24 s for the busy detector the
move rebuilt (`held_s` in `channel.json`; ADR-0088's wait after a rebuild). At 500 Hz there is no
move and no wait. The tries' count and air pass; the span misses 33 s by 3.1 s, all of it that wait.

**H2, the calls retried today** (3 draws each):

| scenario | passed T / C | calls' time T / C |
|---|---|---|
| `40m-gateway-busy-when-called-2300` | 3 / 3 | 317 / 267 s |
| `40m-gateway-three-clients-2300` | 3 / 3 | 145 / 86 s |
| `40m-trimode-gateway-2300` | 3 / 3 | 73 / 33 s |
| `40m-wide-calls-narrow-unprobed` | 3 / 3 | 352 / 126 s |
| `80m-gateway-fq-hangup-2300` | 3 / 3 | 75 / 33 s |
| `80m-trimode-gateway-client-vanishes-500` | 3 / 3 | 326 / 63 s |
| `80m-vox-caller-learned-gap-500` | 3 / **1** | 189 / **919 s** (3 calls lost, at 240 s each) |
| `quick-awgn-2300` | 3 / 3 | 45 / 33 s |
| all | 24 / 22 | 1 522 / 1 560 s |

No invariant broken in either arm; radiated overlaps 0 against 1. Without the VOX scenario the capped
call's calls took 641 s against 1 333 s, and the Trimode stand-in's deaf second cost it nothing.

**H3, two hidden callers, the second N s after the first** (3 draws each):

| N | first session, T | first session, C | overlaps T / C |
|---|---|---|---|
| 3 s | 22, 22, 22 s | 11, 10, 11 s | 6 / 14 |
| 7 s | 19, 19, 19 s | 11, 10, 10 s | 14 / 27 |
| 12 s | 41, 18, 41 s | 11, 11, 10 s | 8 / 10 |
| 18 s | 22, 22, 22 s | 10, 11, 10 s | 7 / 7 |
| all | 12 of 12, median 22 s | 12 of 12, median 10.5 s | **35 / 58** |

Every overlap is the losing client, which cannot hear the winner, over the winner's session (or the
two callers' tries over each other):

| on the air together | T | C |
|---|---|---|
| the loser's tries over the session | 25, 14.9 s | 29, 26.9 s |
| the loser's closing Morse identifier over the session | 2, 0.2 s | 23, 3.0 s |
| the two callers' tries over each other | 8, 12.5 s | 3, 10.2 s |

C's loser makes two tries, both inside the winner's first exchanges, and ends its call in about 37 s,
so its identifier falls inside the run (each dit is one overlap). T's loser spreads eight tries over
about 200 s, mostly over an idle session, and is still calling when the run stops.

**H4, 200 Hz off** (3 draws): T 78, 23 and 23 s; C 11 s, **not connected**, 11 s. In seed 1065 the
called station decoded neither try: the first try's OFDM part was lost on the Good path, the second
try had moved to 500 Hz, and a 2 300 Hz station reads only the tone floor of a 500 Hz call, which at
200 Hz is beyond the tone detector's ±150 Hz. T connected on its second ordinary try.

**H5, every scenario, one draw** (the eight of H2 at variant 0 included): T passed 64 of 64, C 62.
No invariant broken, no radiated overlap in either. The two C failed:

* `80m-vox-caller-learned-gap-500`, as in H2.
* `80m-band-closes-2300` (NVIS, +10 dB): the called station read the first try and accepted it in
  OFDM, which the 7 ms echo kept from the caller; the second try had moved to 500 Hz, and the called
  station, already in the session its acceptance opened, could not move and ignored it as another
  bandwidth's. The caller gave up; the called station stayed in the session. The two extra draws
  (`H5r`): T 3 of 3 (15 s each), C 2 of 3 (14 and 11 s; one acceptance on the floor, one in OFDM).

Over the 62 scenarios both passed, the air was 15 497 s against 15 748 s (median ratio 0.987), and
the first calls connected at a median 11 s against 15 s (45 calls, 598 s against 840 s). C's first
call was slower in three, all at 500 Hz: `80m-agc-carrier-inside-500` (NVIS), and
`80m-disconnect-slow-drain-500` and `80m-poor-500-pactor-signalink` (VOX holds of 600 and 400 ms) —
37–41 s against 12–16 s, each the first acceptance in OFDM lost and the second try answered.

**K2 as registered** (reported; both clients dial at the same second): T 10 of 10 draws with a
session, median 22 s, 19 overlaps; C 2 of 10, 149 overlaps. With no random wait the two clients'
tries meet at the gateway every time.

### 6.2 Verdicts

| criterion | verdict |
|---|---|
| H1, nobody answers | narrow miss at 2 300 Hz: 36.1 s against 33 s, the move's 3.2 s wait; tries and air pass |
| H2, the calls retried today | fails: (a) 22 against 24, (c) 1 560 s against 1 522 s, both the VOX scenario; (b) passes |
| H3, hidden callers apart | (a) and (b) pass: 12 against 12, 10.5 s against 22 s; (c) fails: 58 overlaps against 35 |
| H4, 200 Hz off | fails: one draw of three not connected |
| H5, no harm elsewhere | fails: 62 against 64 (`80m-band-closes-2300`, the VOX scenario); (b) and (c) pass |

### 6.3 What the failures come from

Three causes, each seen more than once:

1. **The move to 500 Hz before the last try** (§8.1, added for the port). It is behind H1's miss,
   H4's lost call and `80m-band-closes-2300`. A 2 300 Hz station reads only the tone floor of a 500 Hz
   call, so the moved try loses its OFDM half to every station not known to run 500 Hz; a station
   that accepted the first try is in its session and cannot move to read the second; and the move
   costs the busy detector's wait. It buys `40m-wide-calls-narrow-unprobed` (126 s against 352 s);
   among H5's other 56 scenarios a call moved only once, in the band that closed.
2. **The short OFDM acceptance.** 0.43 s long, keyed right after the try, it is lost whole inside a
   VOX radio's hold (400–900 ms) and does not get back through an NVIS echo the try itself came
   through. Today's call alternates families, so half its acceptances go on the floor (3.2 s, longer
   than any hold), and its eight tries teach the called station the caller's gap (ADR-0054, 250 ms a
   step); two tries teach one step a call. A called station that answered a try and hears the same
   call again knows its acceptance was lost, and answers in OFDM again.
3. **No random wait.** Two callers that start together meet at every try (K2), and a hidden loser's
   two tries fall in the winner's first exchanges (H3 (c)).

### 6.4 What it points to (not built, not benched)

* **No move for a call of fewer than five tries**, or a move only with the tries setting at 6 or
  more: a 2 300 Hz station calling a 500 Hz station it has not heard then shows that station the
  mismatch banner (ADR-0035) instead of moving; a probe, or the heard list, still moves it.
* **A try heard again is answered on the floor**, and its caller listens for that: the floor's
  acceptance outlasts a hold and comes through an echo. The VOX scenario's calls should then connect
  on their second try, about 37 s, instead of failing (expected, not measured).
* **The pause made a little random** (15 s plus up to one try's length), so that two callers who met
  on their first try do not meet on the second.
* The phantom session a failed call leaves at the called station (it stays until its link timeout,
  about 6 minutes in the VOX scenario) is today's too, but a call that gives up after two tries makes
  it commoner.

Each is the author's call; whichever are taken go into the model first, and the A/B is run again on
the same draws.

## 7. The second round (ADR-0126 §9)

The author took all three changes of §6.4: no move to 500 Hz for a call of four tries or fewer, a
both-ways try heard again answered on the floor, and a random part in the pause (up to the last
try's length). Built in the model and the port (`6a0cc71`), and run as **C2** on the same draws as
§6, against §6's T runs (the changes do nothing with the capped call off). One row a run is in
`data/capped-C2.csv`.

### 7.1 The results

| criterion | T (today) | C (first round) | C2 (second round) |
|---|---|---|---|
| H1, nobody answers: tries, radiated, span | 8, 25.8 s, 212–215 s | 2, 12.9 s, 32.8–36.1 s | 2, 12.9 s, **35.3 s** (limit 39.5) |
| H2, runs passed | 24 / 24 | 22 / 24 | **19 / 24** |
| H2, the calls' time | 1 522 s | 1 560 s | 1 612 s |
| H3, sessions; median first session | 12 / 12; 22 s | 12 / 12; 10.5 s | 12 / 12; **11 s** |
| H3, radiated overlaps | 35 (27.6 s) | 58 (41.3 s) | **92 (52.7 s)** |
| H4, 200 Hz off | 78, 23, 23 s | 11 s, lost, 11 s | **11, 40, 11 s** |
| H5, scenarios passed | 64 / 64 | 62 / 64 | **62 / 64** |
| K2, both dial at once (reported) | 10 / 10, 22 s | 2 / 10 | **8 / 10, 39 s** |

H2 by scenario (3 draws each; runs passed and the calls' time):

| scenario | T | C2 |
|---|---|---|
| `40m-gateway-busy-when-called-2300` | 3, 317 s | 3, 273 s |
| `40m-gateway-three-clients-2300` | 3, 145 s | 3, 87 s |
| `40m-trimode-gateway-2300` | 3, 73 s | 3, 33 s |
| `40m-wide-calls-narrow-unprobed` | 3, 352 s | **0**, 540 s (3 calls lost) |
| `80m-gateway-fq-hangup-2300` | 3, 75 s | 3, 33 s |
| `80m-trimode-gateway-client-vanishes-500` | 3, 326 s | **1**, 402 s (2 calls lost) |
| `80m-vox-caller-learned-gap-500` | 3, 189 s | 3, 211 s (was 1 of 3) |
| `quick-awgn-2300` | 3, 45 s | 3, 33 s |

H5's other 56 scenarios all passed on C2, `80m-band-closes-2300` among them; no invariant broken and
no radiated overlap in either arm. Over the 62 both passed, the air was 15 314 s against 15 526 s
(median ratio 0.993), and the first calls connected at a median 11 s against 15 s.

### 7.2 What each change did

* **No move for a short call.** H1 now passes (no move, no busy wait), H4 connects in every draw (seed
  1065's second try stayed at 2 300 Hz and was answered at 40 s), and `80m-band-closes-2300` passes.
  The price is the case the move was for: `40m-wide-calls-narrow-unprobed` (a 2 300 Hz station calls a
  500 Hz station it has not heard) no longer connects; the 500 Hz station hears the call's floor and
  tells its operator.
* **A both-ways try heard again answered on the floor.** The VOX caller passes 3 of 3 (its first
  call connects on the second try at 42 s, the later ones at 9–10 s once the gap is learned), and so
  does the band that closed. Where the first OFDM acceptance is lost (VOX holds of 400–900 ms, the
  NVIS echo) the call now connects, but after the pause: five scenarios' first calls took 40–45 s
  against today's 12–16 s (`80m-agc-carrier-inside-500`, `80m-band-closes-2300`,
  `80m-disconnect-slow-drain-500`, `80m-poor-500-pactor-signalink`, the VOX scenario).
* **The random pause.** K2 goes from 2 to 8 draws of 10 with a session (median 39 s, on the second
  try). The daemon draws it from its callsign, so a station's pause before a given call is the same
  in every draw.

### 7.3 What still fails, and why

* **H2 (a) and (c), H5 (a): two scenarios.** `40m-wide-calls-narrow-unprobed` is the first change's
  price, chosen. `80m-trimode-gateway-client-vanishes-500` failed in 2 draws of 3 on the call after
  the outage: the gateway decoded both floor tries, each while the Trimode stand-in had `LISTEN` off,
  which it does for one whole second in every four of air (`01-results.md` §3.1; the real program
  0.5 s in 3.5 s), and the same pause in both draws put the second try's end in the deaf second too.
  T made more tries and connected on a later one.
* **H3 (c), the hidden loser's overlaps,** which none of the changes aimed at: 92 against 35. 51 of
  them are the losing caller's closing Morse identifier keyed over the winner's session (one overlap a
  dit, 5.7 s in all); the loser's two tries over the session are 35 s. By seconds, 52.7 against 27.6.

### 7.4 What it points to (not built, not benched)

* **The 500 Hz station a wide caller has not heard.** Keep the trade (a probe, the heard list or the
  operator's choice of 500 Hz reaches it), or move only with the tries setting at 6 or more, as now.
* **A lost OFDM acceptance costs the pause.** Today's call alternates and answers on the floor within
  about 12 s; the capped call waits about 20 s more for its second try. Two ways out: answer every
  both-ways try on the floor (about 2.8 s more on every good-path connect), or send the acceptance in
  both families back to back, OFDM then floor, so a caller that lost the first reads the second.
* **The Trimode stand-in's whole-second scan** makes the harness's gateway deaf a quarter of the
  time at whole-second phases; on the air it is a seventh at random phases. Scanning on the channel's
  sub-second clock would make that scenario judge the call, not the stand-in.
* **The hidden loser's identifier.** A call that went unanswered is identified on its own after
  ADR-0055's wait, and a station that cannot hear the session it falls on keys it there. Whether that
  identifier should wait for a clear channel is the identifier's question, not the call's.

## 8. The release candidate (beta.160, link protocol 13)

The capped call made the only call (ADR-0126 §10), with what making it the default found (§10.2),
rebased on the finer air clock and the seeded Trimode scan (`e5ccdbe`), built at `3e272ad` and run
as the nightly runs it: every scenario in `bench/scenarios/`, three draws (`--variant 0, 1, 2`),
`--fail-on invariants`.

* **190 of 192 runs passed; no invariant broken; one radiated overlap.** The two that missed:
  `80m-trimode-gateway-client-vanishes-500` seed 1046 (the first try fell in the stand-in's deaf
  window, the second's OFDM acceptance was lost on the NVIS path, and the call ended) and
  `80m-agc-slow-carrier-outside-500` seed 2032 (the 1 000-byte message on ITU NVIS at 500 Hz, the
  edge-of-budget case since November). The trend tool flags both as one draw of three; none slower.
* **Air time:** the median scenario at 0.968 of the baseline's; from 0.72
  (`80m-gateway-fq-hangup-2300`) to 1.34 (`20m-ft8-neighbours-2300`). `80m-leave-together-500`, flagged
  slower on two draws, was 358, 392 and 330 s against `master`'s own daemon on the same draws at
  343, 261 and 282 s: both took the message down the same rungs, and a call that takes another
  time meets the fading at another moment. `master`'s daemon passed the Trimode scenario 3 of 3.
* **A first pass under load found a real defect**: ADR-0103's 4 s wait for an acceptance's key,
  which a both-ways try outlasts (ADR-0126 §10.2). It is 12 s; the pass above is after the fix.

`bench/baselines/scenarios.csv` is these runs (build `3e272ad`).
