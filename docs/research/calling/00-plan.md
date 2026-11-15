# How a call is repeated: plan and criteria

*Registered 2026-11-15, before any run of the rule under test. The author asked whether calling
at a fixed interval, as other modems do, would be faster than Aether's call, and then asked for
this bench.*

## 1. The problem

A call is eight tries (`connect_retries`). Tries 1, 3, 5 and 7 go on the tone floor (5.36 s),
tries 2, 4, 6 and 8 in the ordinary family (1.05 s). After each try the caller grants one slot
for the acceptance, sized for a floor acceptance, and then waits a random time before the next
try (`07-protocol-8.md` §5.1):

    the jitter is uniform(0, (1 + tries) · floor frame)

so its span grows from 10.7 s after try 1 to 48.2 s after try 8. Worked out from the engine's
constants (`KEY_S`, `GUARD_S`, `SLACK_S`, `LEVELS_S`, `DECODE_BUDGET_S`) and checked against
five unanswered calls on the model:

| | fixed part | random part, mean | total, mean |
|---|---|---|---|
| a call nobody answers | 87 s | 118 s | 205 s (87–322) |
| try 5 starts (ADR-0058's move to 500 Hz) | 46 s | 32 s | 78 s |

The random part is 58 % of an unanswered call. It also lengthens the common case: a station that
is there and missed one try to a fade, an AGC or a scanning gateway's `LISTEN OFF` waits for the
next try. On 2026-11-07 Winlink Express gave up after four tries and 76 s, before the move to
500 Hz could come.

The random wait is there so that two callers whose tries overlap do not overlap again: two
clients of one gateway that cannot hear each other. Two stations calling each other need none of
it (the lower callsign yields, §5.1).

## 2. The rules under test

* **W0**, today: `uniform(0, (1 + tries) · F)`, F the floor's data frame (5.36 s).
* **W1**, the candidate: `uniform(0, F)` after every try.
* **W2**, an ablation: no random wait. Run where it is cheap, to show what the random wait buys;
  not a candidate unless it passes everything W1 must.

Nothing else changes: eight tries, the alternation, the slot, the level, the move to 500 Hz.
The random wait is the caller's own, so no frame changes and every build interoperates. The rule
is a bench switch while it is measured, `GrantConfig.call_wait_frames` (unset: W0); the port is
built with it edited in the source.

## 3. Instruments

* **The model bench**, `tools/bench_calls.py --wait`: two engines through the link bench's fading
  pipe, both airs, AWGN/Good/Moderate/Poor, −16 … +8 dB in 4 dB steps, 30 trials a cell, a call
  given 400 s. The pipe loses frames to the SNR and the fades; it has no keying latency.
* **The harness**, `tools/session_matrix.py`, two arms on the same draws, the runner noting how
  long each call took (`call N s` in `transfers`).
* **A new scenario**, `40m-gateway-hidden-callers-2300`: two Winlink clients that cannot hear
  each other (−30 dB between them) call a scanning Trimode gateway at the same moment, both on
  Moderate paths at the same SNR, so neither call captures the gateway. Measured: the air from the
  calls to the first session up.

## 4. Criteria

* **K1, time to connect (model).** Over all 56 cells × 30 trials of each arm: (a) calls connected
  within 400 s, W1 no fewer than W0 less 1 % of the calls, and in no cell more than 3 fewer;
  (b) the mean time to connect, a call not connected counted as 400 s, W1 ≤ W0.
* **K2, contention (harness).** `40m-gateway-hidden-callers-2300`, 10 draws an arm: the median air
  from the calls to the first session up, W1 ≤ W0, with a draw that has no session within 240 s
  counted as 240 s; and draws with a session within 240 s, W1 no fewer than W0 less one.
* **K3, no harm (harness).** The scenarios whose calls are retried — `40m-gateway-busy-when-called-2300`,
  `40m-gateway-three-clients-2300`, `40m-trimode-gateway-2300`, `40m-wide-calls-narrow-unprobed`,
  `80m-vox-caller-learned-gap-500`, `80m-trimode-client-vanishes-500`,
  `80m-gateway-fq-hangup-2300` — and `quick-awgn-2300`, 3 draws each an arm (variants 0–2):
  (a) runs passing, W1 no fewer than W0, and no invariant broken in W1 that W0 kept; (b) radiated
  overlaps pooled, W1 no more than W0 + 2; (c) the calls' time pooled, W1 ≤ W0.

W1 is proposed to the author as an ADR if it passes K1, K2 and K3. A narrow miss goes to the
author with its trace, case by case; it does not pass by rule. If W2 passes everything too, the
random wait buys nothing measured and that is said. The number of tries is a separate question,
left until the wait is settled.

## 4a. Amendment: every try on the floor (2026-11-15, before any harness run)

The author asked why an ordinary try would be answered where a floor try was not, and after
`01-results.md` §2, asked for this arm. That section looked first: at the weak cells of the
model bench (2 300 Hz, −16 and −12 dB), and at each call frame alone against interference and an
offset. So this arm is registered after a look at part of K1's grid. The full grid is the run
that counts, and the harness has not run any arm.

* **F1**, the second candidate: every try of a call on the tone floor, and W1's wait. The
  acceptance still goes in the request's family, so on the floor. Removed: the ordinary tries
  (ADR-0016 §2.1, which kept them for a carrier on the floor's band; §2 of the results did not
  find that case).
* **F0**, an ablation for the model bench only: every try on the floor, today's wait. It
  separates F1's two changes.
* The switch is `GrantConfig.call_floor_only` (unset: alternating). The port is built with
  `connect_floor` edited to return `self.timing.floor_modes > 0`.

F1 is judged by K1, K2 and K3 exactly as W1, against W0. One more measure:

* **K4, what F1 gives up (harness, reported, not judged).** A new scenario,
  `40m-offset-200-2300`: the called station 200 Hz off frequency, Good, +10 dB, a call, a
  kilobyte, a disconnect, 3 draws an arm. The tone detector searches ±150 Hz and the OFDM one
  ±300 Hz, so W0 and W1 are expected to connect on an ordinary try and F1 not at all. That is the
  price of F1. If F1 passes K1–K3, it goes to the author with this price beside it, and with two
  ways to keep the case: one ordinary try late in the call, or a wider floor search (which would
  need its own false-detection measurement).

If W1 and F1 both pass K1–K3, F1 is proposed when it is no worse than W1 on any of them. Otherwise
W1 is proposed alone.
