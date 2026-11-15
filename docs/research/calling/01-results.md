# How a call is repeated: results

*Against the criteria registered in `00-plan.md` (and its §4a). K1 run 2026-11-15 on the model; K2, K3
and K4 the same day on the harness (§3).*

## 1. K1, time to connect (the model)

`tools/bench_calls.py --wait growing|short|none`: 2 airs × 4 classes × 7 SNRs × 30 trials,
1 680 calls an arm, a call given 400 s. The bench now gives each station its own configuration,
stating its air's capability byte; before, both stations of a 500 Hz point had run as 2300 Hz
ones. The switch leaves the engine as it was: the granted vectors generated with it are
byte-identical to those generated from `HEAD` on the same machine.

| arm | connected | mean time, 400 s if not | connected after try 1 | median, those |
|---|---|---|---|---|
| W0, alternating, growing wait (today) | 1 650 | 22.5 s | 96 | 50 s |
| **W1, alternating, one-frame wait** | **1 662** | **18.8 s** | 108 | 40 s |
| W2, alternating, no wait | 1 654 | 20.0 s | 100 | 54 s |
| F0, floor only, growing wait | 1 670 | 17.4 s | 116 | 46 s |
| **F1, floor only, one-frame wait** | **1 672** | **16.2 s** | 118 | 39 s |

* **W1, K1(a) passes:** 1 662 against W0's 1 650 (the floor was 1 633), and the worst cell is one
  call short (Good −16 dB, 28 against 29; the floor was three).
* **W1, K1(b) passes:** 18.8 s against 22.5 s, 16 % less.
* **F1 (§4a), K1 passes:** 1 672 connected, no cell short of W0; 16.2 s, 28 % less than W0. Against
  W1 it is no worse in any cell (1 672 against 1 662; 16.2 s against 18.8 s).

What the bench can and cannot say:

* Above −12 dB every call connects at its first try (12 s) in all three arms, so the arms differ
  only at −16 and −12 dB. At those SNRs the ordinary tries never get through and every call
  connects on a floor try. The 500 Hz half gives exactly the 2300 Hz half's outcome, call by
  call: the call frames are the same length on both airs. So there are 28 independent cells, not 56.
* The connect counts differ by chance: every arm makes eight tries, and the pipe's fades are
  short against the tries' spacing. The times are the effect. On the 2300 Hz half, a call
  answered at its third try took a median 46 s (W0), 37 s (W1) and 32 s (W2); at its fifth try
  97, 64 and 54 s; at its seventh 137, 93 and 77 s.

| cell (each air) | W0 connected, mean | W1 | W2 | F0 | F1 |
|---|---|---|---|---|---|
| Good −16 dB | 29, 64 s | 28, 61 s | 26, 80 s | 30, 43 s | 29, 42 s |
| Good −12 dB | 30, 19 s | 30, 16 s | 30, 16 s | 30, 16 s | 30, 16 s |
| Moderate −16 dB | 21, 156 s | 25, 101 s | 24, 109 s | 26, 91 s | 28, 65 s |
| Moderate −12 dB | 30, 15 s | 30, 14 s | 30, 14 s | 30, 14 s | 30, 13 s |
| Poor −16 dB | 25, 100 s | 28, 58 s | 27, 66 s | 29, 49 s | 29, 40 s |

The pipe carries no interference and no offset, so it cannot show what F1 gives up; that is
K4, on the harness.

The pipe has no second caller, so it cannot show what the random wait is for. That is K2.

## 2. Why a call alternates families, tested (not registered; for the author's question)

The author asked, before the harness runs, why an ordinary try would be answered where a floor
try was not. ADR-0016 kept the ordinary tries for one reason: "a carrier sitting on the floor's
400 Hz denies it what 2 300 Hz of OFDM with a code across its carriers still gets through".
That was argued, not measured. Measured now on the model's receivers (`Modem.decode_buffer`):
each call frame alone, tone-24 (the floor's try) against the ordinary robust mode (2 300 Hz rung
6, BPSK ⅕; 500 Hz rung 5, QPSK ½), through AWGN at +10 and 0 dB, 20 frames a point. Interferer
levels are relative to the OFDM average; the tone frame goes 5.5 dB over it, as rendered.

| What is in the way | Floor | Ordinary |
|---|---|---|
| Nothing | 20 | 20 |
| A steady carrier on a floor tone, 0 dB (2 300 Hz) | 20 | **0** |
| The same at +10 dB | 0 | 0 |
| A carrier at 1 500 Hz exactly, +10 dB (2 300 Hz) | 0 | 6–7 |
| Noise filling 1 300–1 700 Hz, +5 / +10 dB (2 300 Hz) | 20 / 20 | 17 / **0** |
| The same, 0 dB (500 Hz) | 20 | **0** |
| FSK 1 415/1 585 Hz at +10 dB (2 300 Hz) | 0–1 | 3 |
| FSK at 0 dB (500 Hz) | 20 | **0–2** |
| The other station tuned 120 Hz off | 20 | 20 |
| Tuned 200 or 280 Hz off | **0** | 20 |

The floor's 5.36 s of heavily coded 40 ms tones survives more in-band interference than the
ordinary frame, not less. The exceptions are a strong carrier at the passband's centre and a
strong FSK at 2 300 Hz, where the ordinary frame gets a few frames in twenty and the floor none.
The ordinary frame wins outright in one case: the other station is 150–300 Hz off frequency. The
tone detector searches ±150 Hz (`tone.MAX_CFO_HZ`, ADR-0064), the OFDM one ±300 Hz.

On a weak path the ordinary tries can never be answered: the ordinary call needs about −5 dB,
the floor's −19 dB. On the fading pipe at −16 and −12 dB, every call connected on a floor try.
That look, at those cells alone, is what led to F0 and F1 (`00-plan.md` §4a); their full grid is
in §1.

From the field, one case each way. KQ4SAX's calls of 2026-10-12 were heard only on the floor, at
−16 to −17.5 dB. KE4QCM's of 2026-10-10 decoded only in the ordinary family (ADR-0056), for a
reason that ADR does not record; the tone receiver has changed since (ADR-0060, ADR-0070).

## 3. K2, K3 and K4 (the harness)

Run 2026-11-15 in a cloud container (Linux, 4 cores), two runs at a time (`--jobs 2`), on four
daemons built from `9e5c9c8` with one line of `core/aether-link/src/granted.rs` edited: W0 as
committed, W1 (`let spread = self.robust_frame_s(true);`), W2 (`let spread = 0.0;`), and F1 (W1's
line, and `connect_floor` ending `self.timing.floor_modes > 0`). Every arm ran the same draws, the
commands of the plan, with `--drop-passing-audio`. One row a run is in `data/W0.csv`, `data/W1.csv`,
`data/W2.csv` and `data/F1.csv`. Their `bench` column says which: `K2`, `K3` and `K4` as
registered; `K2x` (§3.3), `K3b`, `K3r1`–`K3r3` and `K3v` (§3.4) added here, not registered.

### 3.1 What the harness can and cannot say here

* **A daemon's random draws come from its callsign** (`seed_from_callsign`,
  `core/aetherd/src/main.rs`). A scenario's seed changes the channel, not the calls' waits. So the
  ten draws of K2 repeat the same pair of waits: in W0 and W1 the same client won all ten, at 18
  or 22 s in W0 and 17 or 21 s in W1. K2 as registered is one sample of the waits on ten channels.
  A supplement, not registered, gives the two clients other callsigns in each draw (§3.3).
* **The runner's air clock moves in whole seconds** (`channel.progress` is written once a second
  of air). Call times are to a second. The Trimode stand-in times its scan on that clock, so it is
  deaf for 1 s in every 4 s, not 0.5 s in 3.5 s, and always in the same whole second. The runner
  dials on a whole second too. In the five Trimode scenarios of K3 the gateway's first decoded
  call fell in a deaf second in 85 of 90 runs (both passes, every arm), so nearly every gateway
  call there needs a second try. Later calls the idle gateway decoded fell in one 42 times in
  206, about the stand-in's deaf share. Where the later tries fall against the scan is fixed by
  the callsigns, so the three draws of a scenario repeat it. This is the harness, not a rule under
  test, and it is the same for every arm.
* **Runs of one build do not repeat exactly.** K3 was run twice on the same draws (§3.4): 37 of
  W0's 48 calls took the same time to the second both times, but one took 12 s in one pass and
  82 s in the other, and W0's total moved by 91 s.

### 3.2 K2, contention

`40m-gateway-hidden-callers-2300`, 10 draws an arm. The measure is, per draw, the sooner of the two
calls (`call a`, `call c` in `transfers`), 240 s when neither connected; the host logs agree with
the runner on every call.

| arm | median, first session | draws with a session in 240 s | first session | each draw | radiated overlaps |
|---|---|---|---|---|---|
| W0, today | 22 s | 10 | a in 10 | 18 ×3, 22 ×7 | 24 |
| **W1** | **21 s** | **10** | a in 10 | 17 ×3, 21 ×7 | 48 |
| W2, no wait | 16 s | 10 | a 4, c 6 | 15, 16 ×6, 20 ×2, 33 | 119 |
| **F1** | **54 s** | **10** | a 8, c 2 | 25 ×2, 28 ×2, 54 ×6 | 102 |

* **W1 passes K2:** 21 s against 22 s, and 10 draws against 10. The second is one sample of the
  waits (§3.1); the supplement agrees (§3.3).
* **F1 fails K2:** 54 s against 22 s. Its draws within 240 s pass (10 against 10).
* **W2 passes both of K2's measures** (16 s, 10), at five times W0's overlaps.

Why F1 fails. Both clients call at the same second, so their first tries meet in every arm. W0 and
W1 then send an ordinary try of 1.05 s, and a second between the two clients' waits keeps those
apart: the first session came at try 2 in every draw. F1's every try is a floor frame of 5.36 s,
and W1's wait is at most one frame. The two clients' tries move apart only by the difference of
their two waits, a little at each try, so they overlap again until that offset passes a frame.
The second tries met again in all ten draws. In four the gateway read one of the two anyway
(25–28 s); in six the third tries met too, and the first session came at try 4 (54 s). W2 has
no wait, so its second tries go at the same instant and meet in every draw, but they are short
ordinary frames on two paths that fade apart, and the gateway read one of the two in nine draws
of ten.

Every radiated overlap, named by what was on the air in it (`channel.json`'s `rf_overlaps` against
each station's `radiated` and `transmissions`; `air_account.py` names a three-station overlap by
the last transmission each station began, which can be over, so it was not used here; the CSVs'
`mechanisms` column is its naming). A count, and seconds:

| mechanism | W0 | W1 | W2 | F1 |
|---|---|---|---|---|
| the two clients' first tries (by the scenario: both dial at once) | 10, 53.8 s | 10, 53.8 s | 10, 53.8 s | 10, 53.7 s |
| later tries of the two clients over each other | 0 | 0 | 11, 16.1 s | 16, 37.7 s |
| a try over the gateway's acceptance to the other client | 0 | 0 | 0 | 8, 28.6 s |
| a try over the other client's session | 14, 9.3 s | 38, 24.6 s | 65, 28.8 s | 38, 15.5 s |
| a failed call's closing identifier over the other session | 0 | 0 | 23, 3.0 s | 30, 1.9 s |
| the runner stopping the daemons at the end | 0 | 0 | 10, 0.9 s | 0 |
| all | 24 | 48 | 119 | 102 |

* *Later tries over each other* is what the random wait is for. W0 and W1 have none; W2 one a
  draw (the second tries) and one more in one draw; F1 sixteen.
* *A try over the other client's session* is the hidden station. The losing client cannot hear
  the winner, and the gateway answers in the slot the winner granted, which the loser cannot know
  of. Its tries go on until its call ends: the losing calls made 34 tries in all in W0, 50 in W1,
  80 in W2 and 70 in F1. A shorter wait puts more of them inside the run.
* *A try over the gateway's acceptance* (F1): the gateway's floor acceptance to the winner, keyed
  in its slot over the loser's floor try, which was already on the air.
* *A failed call's closing identifier*: a call that ends unanswered sends its Morse identifier
  after listening (ADR-0075), and the listening cannot hear the other client. Many short
  overlaps, one identifier each. It comes inside the run where the losing call ends inside it (W2,
  F1; W1 in the supplement).
* *The runner stopping the daemons*: one W2 draw, the gateway's `DISC_ACK` and identifier when the
  runner stopped it, over the losing client's identifier.

The run stops about 180 s after the first session, so a losing call that lasts longer is cut off:
W0's, whose tries spread over about 200 s, never reaches its identifier inside the run. The
overlap counts are what the run holds, not a whole call's.

### 3.3 K2 with the waits sampled (not registered)

The same scenario and channel draws (`seed` 64 + 1000 k), with the clients' callsigns changed in
each draw: a W4AAA, K4BBB, N4CCC, KA4DDD, KB4EEE, KC4FFF, KD4GGG, KF4HHH, KG4III, KI4JJJ, and c
W1KKK, K1LLL, N1MMM, KA1NNN, KB1OOO, KC1PPP, KD1QQQ, KF1RRR, KG1SSS, KI1TTT. Rows `K2x` in the
data.

| arm | median, first session | draws with a session in 240 s | first session | each draw | radiated overlaps | later tries over each other |
|---|---|---|---|---|---|---|
| W0 | 23.5 s | 10 | a 5, c 5 | 17, 17, 20, 20, 22, 25, 26, 26, 28, 43 | 34 | 2 |
| W1 | 22.5 s | 10 | a 2, c 8 | 17, 20, 21, 21, 22, 23, 24, 24, 24, 42 | 98 | 3 |
| W2 | 16 s | 10 | a 4, c 6 | 15, 16 ×5, 20 ×3, 33 | 118 | 11 |
| F1 | 40.5 s | 9 | a 6, c 3 | 24, 25, 25, 26, 40, 41, 43, 55, 74, none | 85 | 23 |

It reads as the registered run. W1 is a second faster than W0 at the median. F1 is slower, and in
one draw (`-x8`) neither client connected: their eight floor tries overlapped every time, and the
four the gateway decoded fell in the stand-in's deaf second. W2 does not depend on the callsigns
(it draws no wait) and repeats its registered run. W1's overlaps are 98 against 34: a try over
the other client's session 46 against 19, a closing identifier 36 against 0.

### 3.4 K3, no harm

The eight scenarios, 3 draws each an arm (variants 0–2), as registered. The table's calls are each
scenario's calls over its three draws, added (`call` entries in `transfers`, from a call's `CONNECT`
to its session up). Every call connected in every run. Because one build's runs are not identical,
the whole set was run a second time on the same draws (rows `K3b`). Each cell gives the first
pass, then the second.

| | W0 | W1 | F1 |
|---|---|---|---|
| runs passed | 24 / 24 | 24 / 23 | 24 / 24 |
| invariants broken | 0 / 0 | 0 / 0 | 0 / 0 |
| radiated overlaps | 0 / 0 | 0 / 0 | 0 / 0 |
| the calls' time, 48 calls | 1 432 / 1 523 s | 1 436 / 1 398 s | 1 412 / 1 355 s |

| scenario | calls | W0 | W1 | F1 | median air, W0 | W1 | F1 |
|---|---|---|---|---|---|---|---|
| `40m-gateway-busy-when-called-2300` | 6 | 315 / 311 s | 364 / 307 s | 350 / 320 s | 146 / 155 s | 148 / 148 s | 168 / 143 s |
| `40m-gateway-three-clients-2300` | 9 | 129 / 145 s | 135 / 135 s | 147 / 147 s | 223 / 222 s | 215 / 222 s | 221 / 221 s |
| `40m-trimode-gateway-2300` | 3 | 71 / 73 s | 51 / 55 s | 76 / 75 s | 96 / 85 s | 83 / 82 s | 81 / 84 s |
| `40m-wide-calls-narrow-unprobed` | 3 | 352 / 352 s | 231 / 231 s | 275 / 275 s | 195 / 195 s | 167 / 167 s | 179 / 179 s |
| `80m-trimode-gateway-client-vanishes-500` | 6 | 256 / 334 s | 350 / 361 s | 247 / 218 s | 842 / 817 s | 829 / 893 s | 733 / 753 s |
| `80m-gateway-fq-hangup-2300` | 3 | 75 / 75 s | 71 / 76 s | 83 / 86 s | 98 / 103 s | 110 / 110 s | 108 / 109 s |
| `80m-vox-caller-learned-gap-500` | 15 | 189 / 188 s | 189 / 188 s | 189 / 189 s | 364 / 361 s | 362 / 355 s | 358 / 358 s |
| `quick-awgn-2300` | 3 | 45 / 45 s | 45 / 45 s | 45 / 45 s | 47 / 47 s | 47 / 47 s | 47 / 47 s |

* **(a) passes for W1 and F1** in the registered pass: 24 runs of 24 in every arm, no invariant
  broken. In the second pass W1 lost one run, so it goes to the author with its trace (below).
* **(b) passes for both:** no radiated overlap in any arm, either pass.
* **(c) F1 passes** in both passes (1 412 s and 1 355 s against W0's 1 432 s and 1 523 s).
* **(c) W1 misses narrowly in the registered pass:** 1 436 s against 1 432 s, 4 s in 48 calls.
  In the second pass it is 1 398 s against 1 523 s. W0's own total moved 91 s between the two
  passes, so the 4 s is inside one build's spread. It goes to the author with its trace, as the
  plan says, and does not pass by rule.

The trace of the 4 s, W1 less W0 in the registered pass, by scenario:

* `40m-wide-calls-narrow-unprobed`, −121 s: the effect the wait was meant to have. A 2 300 Hz
  call to a 500 Hz station moves to 500 Hz after four unanswered tries (ADR-0058); W1 makes the
  four sooner (77 s against 116–120 s, every draw).
* `80m-trimode-gateway-client-vanishes-500`, +94 s: the first call took 96 s against 50 s in all
  three draws, both passes. The gateway decoded three or four of W1's tries before it answered
  one, nearly all of them in the Trimode stand-in's deaf second; it answered W0's second decoded
  try. Where a try falls against the scan is fixed by the callsign, so the draws repeat it.
* `40m-gateway-busy-when-called-2300`, +49 s: one draw (seed 1062), the second client's call into
  a busy gateway, 150 s against 99 s. After the first session ended, three of W1's tries in a row
  were decoded in the stand-in's deaf second, and the fourth was answered. In the second pass the
  same draw took 91 s on W1 and 100 s on W0.
* `40m-trimode-gateway-2300`, −20 s; `40m-gateway-three-clients-2300`, +6 s;
  `80m-gateway-fq-hangup-2300`, −4 s; the other two, 0 s.

So the miss is set by where the tries fall against the stand-in's scan, which the harness
quantises to whole seconds, more than by the wait.

The run W1 lost in the second pass is `80m-trimode-gateway-client-vanishes-500`, seed 1046. Both
calls connected; then the client's 1 000-byte transfer missed its 400 s budget and the session
timed out. That draw was run three more times on each build, and seven more draws of the scenario
once each (not registered; rows `K3r1`–`K3r3` and `K3v`):

| `80m-trimode-gateway-client-vanishes-500` | W0 | W1 |
|---|---|---|
| seed 1046, all five runs | 5 passed | 2 passed |
| seeds 3046–9046 | 7 passed | 7 passed |

Every failed run sent the kilobyte first at rung 10, an OFDM rung this path's 7 ms echo kills,
and stepped down through 7 or 6, 5 and 3 to floor pieces before the budget ran out. The runs
that passed started at rung 2, or at rung 6 and were on the floor within four transmissions. The
first rung is the rate controller's choice from the session's readings. W1 starts this session at
another moment (its first call took 96 s against 50 s), so on another stretch of the channel, and
on this one draw the controller starts high. The nightly's failures of this scenario in early
November went the same way, and were left to the rate controller's review. It is not a call that
failed: every call in K3 connected, in every arm and every run.

### 3.5 K4, what F1 gives up

`40m-offset-200-2300`, 3 draws an arm: the called station 200 Hz off, Good, +10 dB.

| draw | W0 | W1 | F1 |
|---|---|---|---|
| seed 65 | 78 s, try 4 | 52 s, try 4 | not connected |
| 1065 | 23 s, try 2 | 21 s, try 2 | not connected |
| 2065 | 23 s, try 2 | 21 s, try 2 | not connected |

As expected. F1's eight tries are all on the floor, the called station decoded none of them (the
tone detector searches ±150 Hz), and the call ended "no answer". W0 and W1 connect on an ordinary
try. In draw 65 the called station read try 2 at 6.6 dB and so accepted it on the floor
(ADR-0056), which the caller, 200 Hz off for it too, could not hear; a later ordinary acceptance
got through. So the ordinary tries keep this case only while the call reads strong enough for an
ordinary acceptance.

### 3.6 Verdict

| criterion | W1 | F1 |
|---|---|---|
| K1, the model (§1) | passes | passes |
| K2, contention | passes: 21 s against 22 s, 10 draws against 10 | **fails**: 54 s against 22 s |
| K3 (a), runs passing, invariants | passes: 24 against 24, none broken; the second pass 23 against 24 (one draw, the transfer) | passes: 24, none broken; 24 again |
| K3 (b), radiated overlaps | passes: 0 against 0 | passes: 0 |
| K3 (c), the calls' time | **narrow miss**: 1 436 s against 1 432 s | passes: 1 412 s |
| K4, reported | connects in 3 draws of 3 (52, 21, 21 s; W0 78, 23, 23 s) | connects in none |

* **F1 is not proposed.** It fails K2: with every try a floor frame and a wait of at most one
  frame, two hidden callers' tries keep meeting. §4a: when F1 does not pass K1–K3, W1 is proposed
  alone.
* **W1 goes to the author with two narrow results on K3**, each with its trace in §3.4. K3 (c) misses
  by 4 s in 1 432 s, set by the harness's whole-second scan and reversed in a second pass of the
  same draws. K3 (a) passes as registered, but the second pass lost one run, on a draw where W1's
  timing put a session's transfer on the rate controller's bad start (2 of 5 runs passed, against
  5 of 5 on W0; seven other draws of the scenario passed on both). It passes K1, K2 and K3 (b).
* **W2 passes both of K2's measures** (16 s, 10 draws). It was not run on K3, so the plan's case
  of W2 passing everything is not reached. What the random wait buys shows in K2's overlaps:
  W2 has 119 against W0's 24, eleven of them later tries meeting again.
* **What W1 costs, not judged:** the losing client of K2 calls into a session it cannot hear more
  often within the run, 48 radiated overlaps against 24 (98 against 34 in the supplement). None is
  the two clients' later tries meeting (0 against 0; 3 against 2 in the supplement).
