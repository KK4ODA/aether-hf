# ADR-0080: The four-tone receiver reads each tone's level from the data

**Status:** accepted, 2026-10-20. Model first (`tone_levels` in
`model/aether_model/phy/tone.py`), then the port (`aether-phy/src/tone.rs`), bit-exact through
`phy_vectors.json`'s four-tone receive cases. **Receiver only: no wire change, no configuration
key.** A station on this build reads the same frames as before and sends the same ones.

## 1. Context

The 500 Hz ladder's weak-path Tests sit at rung 3, `tone4x100-75` (72–74 bit/s in report 13),
because rung 5, QPSK ½, needs about 7.8 dB more. Report 12 §18.4 sized a rung between them:
130–150 data bit/s, its 10 % point near −1 dB on Moderate and −2 dB on Poor. ADR-0015 had
measured the four-tone frame at 93 bytes (rate 0.87, which it called 0.85; 134 data bit/s) and
not built it: −2.0 dB on Moderate and −0.4 on Poor with genie timing, "a cliff on all three"
fading classes.

Before reaching for a new waveform (spread tones, P9-12; an interleaver, P9-5) the question
was how much of the four-tone kinds' fading loss is the receiver's. Their non-coherent metric
is `log I0(2·sqrt(E·s))`, with `s` one SNR for every tone, interpolated between the three sync
blocks. Two things make that figure wrong for most symbols:

* the four data tones are 100 Hz apart, 300 Hz end to end, and a path with a millisecond or two
  of differential delay (Moderate, Poor) has notches 500–1 000 Hz apart: the tones fade apart;
* the sync blocks are 2.4 s apart, and Moderate's fading (0.5 Hz, 2σ) turns over in about a
  second, Poor's in half that.

A twin of the channel simulator's own fading processes gives each tone's true gain at each
symbol; with it in the metric (`−s_t + log I0(2·sqrt(E_t·s_t))`, each tone its own `s_t`), the
10 % points move by 0.5–2 dB on rung 3 and up to 6 dB on the 93-byte frame (genie timing,
100 frames a point, dB):

| Kind | Channel | Receiver as it was | True gains | One tone's mean energy |
|---|---|---|---|---|
| tone4x100-75 | Good | −1.5 | −3.3 | −2.0 |
| tone4x100-75 | Moderate | −6.1 | −7.5 | −6.6 |
| tone4x100-75 | Poor | −5.8 | −7.8 | −6.9 |
| 93 B, rate 0.87 | Moderate | −0.8 | −4.0 | −2.8 |
| 93 B, rate 0.87 | Poor | +2.2 | −4.3 | −1.7 |

The floor's sixteen-tone kinds gain little the same way: a window of the data sends each of
sixteen tones too seldom to read it.

## 2. Decision

For a kind whose data has at most four tones (`TRACKED_TONES`: the 500 Hz middle kinds), the
receiver reads each tone's level at each symbol from the data (`tone_levels`):

1. **Where the tone was probably sent.** Every tone's probability in every data symbol under
   the sync blocks' level (the softmax of the metric as it was); each tone's energy less the
   noise's, weighted by that probability, averaged over the 25 symbols around (`TRACK_WINDOW`,
   a quarter second at 100 Bd; clipped at the frame's ends). A symbol a tone was not sent in
   adds nothing to its level, and a silent one (the receiver muted) adds nothing at all.
2. **Pulled toward the sync blocks' level** by how far the two disagree beyond the estimate's
   own noise, over the frame (an empirical-Bayes shrinkage): the estimate's variance is
   `(1 + 2s)/n` for `n` symbols' weight; with `v` its mean and `d` the mean squared difference
   from the sync blocks' level, the data's figure is kept in the proportion `max(0, 1 − v/d)`.
   On a steady channel the two agree within the noise and the sync blocks' level — 24 known
   symbols — stands; in a fade the data's does.
3. **Each tone's own metric**, `−s_t + log I0(2·sqrt(E_t·s_t))`: with a common `s` the first
   term was the same for every tone and dropped out; with each tone's own it does not. That is
   the metric of a known level, and it holds where the quarter-second window follows the fade.
4. **Where the channel fades faster than that, the level is a mean, not the instant's.** The
   first build of this receiver ruined the four-tone kinds on ITU flutter (10 Hz): 12 frames of
   60 at 0 dB for tone4x100-93 where the sync blocks' common level decoded 60, because the term
   `−s_t` turns an instant's deep fade of a tone with a high mean into a confident wrong
   answer, where the common level left it an erasure. None of the benches had a flutter class,
   so the link bench passed it; the harness's `80m-flutter-500` came out a tenth slower, and that
   was what gave it away. Now each frame measures how fast its channel moves: the sync tones'
   energies in each block of eight about a straight line through them, less the noise's own
   spread, over the block's level squared (`fade_wobble`) — about 0 on AWGN and Good, 0.2–0.4 on
   Moderate and Poor, whose fades are nearly linear over a block's 0.32 s, 0.7–0.8 on flutter.
   The tone's power is then taken as Nakagami-distributed about its level, of order
   `m = 2 / wobble` (`FADE_SCALE`; a half rounds up), whose energy-detector likelihood against
   noise alone is, for a whole `m`,
   `m·ln(m/(m+s)) + x + ln Σ_{k<m} C(m−1,k)·x^k/k!` with `x = E·s/(m+s)` — the Rayleigh metric
   `x − ln(1+s)` at `m = 1`, the known level's as `m` grows; from `m` above 8
   (`FADE_ORDER_MAX`) the known level's metric itself. A muted symbol is no evidence for any
   tone (with a level for each tone, the known level's metric had favoured the weakest).

The sync blocks' level is still what the frame's SNR is reported from, and the fast kinds of
the 2 300 Hz air keep the metric as it was.

## 3. Measured

Estimators compared on the same frames (genie timing, 60 frames a point, 10 % points in dB):

| | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| rate 0.87 (93 B), as it was | −11.2 | −0.7 | −0.9 | +1.6 |
| the same, true gains | −11.2 | −0.9 | −2.5 | −4.0 |
| the same, data's levels, not shrunk | −11.2 | −0.9 | −2.2 | −2.5 |
| **the same, this ADR** | −11.2 | −0.9 | −2.2 | −2.6 |
| rate 0.89 (97 B), this ADR | −10.9 | −0.5 | −0.7 | −0.7 |

Without the shrinkage the data's levels cost the steady channel: 27 of 60 frames decoded at
−12 dB on AWGN against 33 as it was and 33 shrunk. A tone's plain mean energy over the window
(the last column of §1), which does not ask where it was sent, is about 35 % off — three symbols
in four the tone holds only noise — and reaches a third to three fifths of the gain.

Through the detector (`tools/bench_tone.py`, 100 frames a point, the baseline's own frames and
seeds; `bench/baselines/tone_floor.csv`), 10 % FER in dB, before → after:

| | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|
| tone4x100-51 (rung 2) | −14.3 → −14.3 | −7.5 → −7.8 | −10.0 → −10.2 | −10.3 → −10.8 |
| tone4x100-75 (rung 3) | −13.0 → −13.0 | −3.2 → −3.6 | −4.9 → −6.2 | −5.7 → −7.2 |
| tone4x100-93 (ADR-0081) | −11.1 | −0.7 | −2.2 | −3.8 |

Rung 3 gains 0.4, 1.3 and 1.5 dB on Good, Moderate and Poor; rung 2, at a lower rate, a
fraction of that; AWGN is unchanged, and so are the rate controller's thresholds. Good fades so
slowly that a frame sits in one fade: there is little to follow. On the link bench's fading
pipe (30 sessions of 1 kB a point, recalibrated to these curves; median bit/s) the receiver
alone makes 500 Hz sessions 4–11 % faster between −8 and −2 dB on Moderate and Poor, and changes
nothing from 0 dB up, where the link runs above the four-tone rungs.

On ITU flutter (10 Hz), frames decoded of 60 through the 500 Hz detector, the receiver before
this ADR / its first build (§2.3 alone) / this one (§2.4), the same frames:

| | −6 dB | −3 | 0 | +3 | +6 | +9 |
|---|---|---|---|---|---|---|
| tone4x100-75 | 59 / 43 / 59 | 59 / 56 / 59 | 58 / 58 / 58 | 58 / 58 / 58 | 56 / 56 / 56 | 52 / 52 / 52 |
| tone4x100-93 | 5 / 0 / 2 | 52 / 1 / 46 | 59 / 17 / 59 | 58 / 24 / 58 | 56 / 24 / 56 | 53 / 27 / 53 |

The tone4x100-93 frame gives back a few frames at the bottom of its flutter curve (−3 dB, 52
against 46). With genie timing all three decode 60 of 60 from 0 to +9 dB: the frames the
detector loses at the top of the column, with any receiver, are an acquisition matter on
flutter, left open.

## 4. Alternatives

* **One level for every tone from each symbol's total energy**: tracks the fade in time, not
  across the tones. At the one point it was measured at (Moderate −3 dB, 30 frames) it decoded
  23, as the receiver as it was, against 28 for this ADR's and for the true gains.
* **The data's levels without the shrinkage**: the table above; it gives up a fraction of a
  decibel on AWGN, where nothing is to be gained.
* **A longer OFDM frame at 500 Hz** (the 1.05 s LONG frame's data stretched to 5.3 s): with
  genie timing it reaches −3.2 dB on Moderate at 140 bit/s, but through the detector its two
  preamble symbols lose 4 dB on Good; the tone frames' three sync blocks find every frame
  wherever it can decode. Not pursued.

## 5. Consequences

* The four-tone kinds' fading curves move, and with them the fading pipe's calibration
  (`bench/baselines/fading_pipe.csv`, refitted by `tools/calibrate_fading.py`); their AWGN
  thresholds, which the rate controller reads, do not.
* It is what makes the 93-byte frame worth a rung: ADR-0081.
