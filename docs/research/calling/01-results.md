# How a call is repeated: results

*Against the criteria registered in `00-plan.md` (and its §4a). K1 run 2026-11-15; K2, K3 and K4 wait for a
daemon build (below).*

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

## 3. K2, K3 and K4 (the harness): ready, not run

They need four daemon builds: W0 (today's), W1, F1, and W2 for K2's ablation. This machine has
no Rust toolchain, no MSVC build tools and no WSL. What is ready:

* the scenarios beside this note, kept out of `bench/scenarios/` so that the nightly does not run
  them before the decision: `40m-gateway-hidden-callers-2300.toml` (K2) and
  `40m-offset-200-2300.toml` (K4);
* the runner's timing of each call: `tools/session_matrix.py` writes `call a 12.3 s` (or
  `call a lost`) in `transfers` for `connect`, `host-connect` and `host-wait`;
* the port's edit, the only line between the arms, in `core/aether-link/src/granted.rs`
  (`send_connect`):

      let spread = (1 + self.connect_tries) as f64 * self.robust_frame_s(true);   // W0
      let spread = self.robust_frame_s(true);                                     // W1
      let spread = 0.0;                                                           // W2

  and for F1, W1's line with every try on the floor (`connect_floor`):

      self.timing.floor_modes > 0 && self.connect_tries % 2 == 0                 // W0, W1, W2
      self.timing.floor_modes > 0                                                 // F1

  Build each with `cargo build --release -p aetherd` and copy the daemon out before the next edit.
* the runs, each arm on the same draws:

      python tools/session_matrix.py docs/research/calling/40m-gateway-hidden-callers-2300.toml \
          --daemon <arm>/aetherd --seeds 10 --jobs 4 --out runs/calling-<arm> --csv calling-<arm>.csv
      python tools/session_matrix.py bench/scenarios/40m-gateway-busy-when-called-2300.toml \
          bench/scenarios/40m-gateway-three-clients-2300.toml bench/scenarios/40m-trimode-gateway-2300.toml \
          bench/scenarios/40m-wide-calls-narrow-unprobed.toml bench/scenarios/80m-vox-caller-learned-gap-500.toml \
          bench/scenarios/80m-trimode-client-vanishes-500.toml bench/scenarios/80m-gateway-fq-hangup-2300.toml \
          bench/scenarios/quick-awgn-2300.toml \
          --daemon <arm>/aetherd --seeds 3 --jobs 4 --out runs/k3-<arm> --csv k3-<arm>.csv

      python tools/session_matrix.py docs/research/calling/40m-offset-200-2300.toml \
          --daemon <arm>/aetherd --seeds 3 --out runs/k4-<arm> --csv k4-<arm>.csv

  K2 runs on W0, W1, W2 and F1; K3 and K4 on W0, W1 and F1. K2's measure is, per draw, the smaller
  of `call a` and `call c`, a draw with neither counted as 240 s. K3's is the CSV's pass column,
  `invariants`, `rf_overlaps` and the `call` entries. K4's is whether, and how fast, each arm
  connects.
