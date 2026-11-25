# ADR-0131: The channel estimate takes the delay out before it interpolates

**Status:** accepted, built 2026-11-25 (§10) — de-rotation only, the author's answer (§8) to the
measurements of §7. Started on the author's "start on 1 and 2" after the MER tool's first readings
(issue #15, `tools/mer_reading.py`). The model step is built behind `FrameReceiver.derotate`
(`model/aether_model/phy/rx.py`), off; the port (`aether-phy/src/rx.rs`) turns it on in the model
and the core together, with the vectors regenerated and the harness A/B (§9). No wire change, no
configuration key. The reported SNR does not move (§2.3), so the rate controller's rule is not
touched.

## 1. Context

`tools/mer_reading.py` decodes each OFDM frame again, re-encodes its payload and measures the
data carriers' error against what was sent. On the model's channel, at 24 dB on the 2 300 Hz air:

| channel | pilot SNR per carrier | data, weighted by \|H\|² | data on a channel re-estimated from the data | data's error over the noise the decoder was given |
|---|---|---|---|---|
| AWGN | 21–24 | 20–24 | about 1 dB under | +0.8 dB |
| Good | 20–25 | 19–23 | — | +0.9 |
| Moderate | 19–23 | 17–21 | 18–20 | +2.0 to +3.5 |
| Poor | 19–22 | 9.6–11.4 | 17–20 | +7.7 to +11.8 |

On Poor (two rays 2 ms apart, 1 Hz) the data carriers lose 8–11 dB that the pilots do not, and
re-estimating the channel along each carrier from the decoded data gives it back. The loss is the
channel estimate between pilots. Two causes were proposed, both in `FrameReceiver.receive`:

1. **Linear interpolation across a phase slope.** The comb pilots are every fourth carrier (160 Hz
   apart on both airs). A channel whose energy is centred τ after the receiver's timing turns its
   phase 2π·160·τ between two pilots: 58° at τ = 1 ms. A straight line between two points on a
   rotating phasor cuts the corner. A timing sweep shows it on a clean channel: the window moved
   1 ms either way costs 4 dB of data MER at 24 dB with nothing else wrong, and on Poor the best
   window sits 1 ms later than where the receiver puts it (14.7 dB against 9.4).
2. **The decoder's noise leaves the estimate's error out.** Each data symbol's noise variance is
   the pilots' residual noise over |Ĥ|². The estimate's own error is not in it, and it does not
   scale with |H|²: near a notch it is most of the error.

P2-6 measured a Wiener interpolator (`channel_estimator = "wiener"`) that beat linear on raw
interpolation error by 1.3–5.3 dB and gained nothing in frame errors; it is kept in the model only,
never ported.

## 2. Decision

1. **De-rotate before interpolating (`derotate`).** Once the full pilot symbols' chips are known
   (step 4), read the channel's phase slope across frequency from their direct estimates,
   carrier to adjacent carrier: `φ = arg Σ Ĥ[k+1]·Ĥ*[k]` over every full pilot symbol (40 Hz
   apart, so a mean delay up to 12.5 ms is unambiguous; from the comb pilots it would alias past
   3.1 ms). Every interpolation across carriers after that point multiplies the pilots by
   `e^{−jφc}`, interpolates, and multiplies the result by `e^{+jφk}`, keeping each pilot carrier's
   own value: the data symbols' estimates, and the re-estimate when the channel moves (ADR-0082).
   The chip detection (step 3) is left as it is, before φ is known.
2. **The error term is not built.** It was: on each full pilot symbol, the estimate the data
   symbols get checked against the direct one, less the direct one's own noise, added to each data
   symbol's noise before dividing by |Ĥ|². It gains nothing on a decode (E5: 917 decodes without
   it, 919 with it) and lost a frame on the field recordings (E6). The decoder is normalised
   min-sum, and an error term nearly the same on every carrier is nearly a scale, which min-sum
   ignores. Its code went before the first commit; §7 keeps its measurements.
3. **The reported SNR does not move.** The interpolation keeps the pilots' own values, de-rotated
   or not, so the pilots' residual, the noise figures and `snr_carrier_db` / `snr_3k_db` are what
   they were; the signal power the SNR is reported against is the plain interpolation's (the
   de-rotated one does not cut the corners, so its |Ĥ|² between pilots is higher). The rate
   controller is given the same SNR to the last bit (CLAUDE.md, *The rate controller waits for the
   air*). Only the equalised symbols and their noise variances change.

## 3. What it removes

Nothing. E7 asked whether the Wiener interpolator could go; it cannot (§7): it loses on every fading
channel but wins on AWGN and 2 300 Hz NVIS. The error term (§2.2) never reached the tree.

## 4. Alternatives considered

- **Move the FFT window to the channel's centre.** Equivalent for the interpolation, but it moves
  the window inside the cyclic prefix and so trades against inter-symbol interference from the
  early ray; de-rotation changes no sample the FFT sees.
- **The estimate's error in the decoder's noise** (§2.2): measured, not built.
- **A frame-wide scale on the noise.** Changes nothing for one decode under min-sum.
- **Denser pilots, or a higher-order interpolation.** What remains on Poor after de-rotation is the
  channel's curvature between pilots (§7, E4); each is its own candidate (P9-3 for the pilots).

## 5. How it was judged (registered before any run)

The bench is `tools/bench_estimate.py`: `bench_noise_shape.py`'s points (every air, channel and
OFDM mode the committed curves hold, at the SNRs nearest 30 % and 10 % frame errors; flutter and
NVIS at Poor's points), 30 frames a point through `bench_phy.run_point`, the same frames for every
arm. The arms were `before` (both off), `derotate`, `error`, `both`, and `wiener` (the Wiener
estimator, both off). "Decodes" is summed over a group's points. The criteria named `both`, the
candidate as proposed; de-rotation alone is judged on the same lines.

- **E1, no loss where the channel is flat.** AWGN and Good, each air: `both` ≥ `before` less 2 %
  of the frames, and no point more than 3 frames (of 30) under `before`.
- **E2, the gain where the delay spread is.** Poor, each air: `both` ≥ `before` + 10 %. Moderate,
  each air: `both` ≥ `before`.
- **E3, no harm on fast fading or a long echo.** Flutter and NVIS, each air: `both` ≥ `before` less
  2 %.
- **E4, the measure agrees.** `mer_reading.py sim` at 24 dB, 10 frames a point (rungs 10 and 15 at
  2 300 Hz, 5 and 10 at 500 Hz): with both on, on Poor `pilot − mer|h|` ≤ 4 dB; on every class
  |`model`| ≤ 2 dB.
- **E5, each part earns its place.** With the other on, taking one part away loses at least 1 % of
  the decodes over Moderate, Poor and NVIS pooled across airs; a part that does not is not built.
- **E6, the air does not lose.** The model on the 16 recordings of 2026-11-17 (`field/inbox/`, the
  frames each station recorded) decodes no fewer OFDM frames with both on than before.
- **E7, the Wiener estimator.** If `both` ≥ `wiener` on every channel, pooled per air, it is removed
  (§3).

A miss goes to the author with its trace, case by case; it does not pass by rule. After the port,
before de-rotation turns on in a release: the harness A/B (every scenario × 2 draws against the
build before), judged as ADR-0104 was.

## 6. Consequences

- The model's switch is off until the port; `phy_vectors.json` and `rx.rs` are unchanged by the
  model step (the vectors regenerate within 1e-12 with it off), so CI stays as it is.
- The port turns it on and regenerates the OFDM receive vectors: the equalised symbols and noise
  variances move, the reported SNR does not.
- Nothing on the wire, so a station with this receiver talks to one without it.

## 7. Measured (2026-11-17)

**The bench** (`tools/bench_estimate.py`, `bench/baselines/channel_estimate.csv`: 98 points, 30
frames each, every arm on the same frames). Decodes summed per air and channel:

| air | channel | frames | before | derotate | error | both | wiener |
|---|---|---|---|---|---|---|---|
| 2 300 | AWGN | 180 | 157 | 156 | 159 | 156 | 180 |
| 2 300 | Good | 360 | 296 | 296 | 296 | 296 | 197 |
| 2 300 | Moderate | 360 | 315 | 324 | 315 | 325 | 193 |
| 2 300 | Poor | 300 | 164 | **236** | 162 | 235 | 129 |
| 2 300 | flutter | 300 | 81 | 80 | 82 | 82 | 28 |
| 2 300 | NVIS | 300 | 4 | 4 | 5 | 3 | 16 |
| 500 | AWGN | 150 | 124 | 125 | 126 | 129 | 139 |
| 500 | Good | 240 | 197 | 199 | 199 | 199 | 184 |
| 500 | Moderate | 210 | 181 | 193 | 181 | 194 | 118 |
| 500 | Poor | 180 | 107 | **154** | 106 | 156 | 52 |
| 500 | flutter | 180 | 59 | 57 | 61 | 59 | 24 |
| 500 | NVIS | 180 | 6 | 6 | 6 | 6 | 5 |

The `before`, `derotate` and `wiener` arms ran first, with the error term's first build in the same
run (the data symbols' noise subtracted, where the full pilot symbols' own was meant: the peak
reduction clips those harder, and on the 500 Hz PSK rungs the decoder was told 2–4.5 dB more noise
than the data met); the `error` and `both` arms above are the rerun on the corrected build, on the
same frames. The first build's totals differed by at most four frames a group and changed no
verdict. `bench_estimate.py --judge` judges the record for de-rotation.

**The verdicts, as registered:**

- **E1, E2, E3 pass** for `both`, and for de-rotation alone: Poor +44 % decodes on both airs,
  Moderate +3 % (2 300) and +7 % (500), and no point anywhere more than one frame under `before`
  (on Poor every point gains at least three).
- **E4 misses.** `mer_reading.py sim`, 24 dB, 10 frames, both on (the gap is `pilot − mer|h|`;
  `model` the data's error over the noise the decoder was given):

  | air | channel | rung | gap before | gap after | model before | model after |
  |---|---|---|---|---|---|---|
  | 2 300 | Poor | 10 | 10.0 | 3.9 | +9.7 | +0.8 |
  | 2 300 | Poor | 15 | 10.2 | 5.7 | +9.8 | +0.8 |
  | 2 300 | AWGN, Good, Moderate | 10, 15 | 0.9–1.6 | 0.9–1.6 | +0.9 | −0.7 to +0.3 |
  | 500 | Poor | 5 | 10.8 | 5.7 | +10.5 | +2.2 |
  | 500 | Poor | 10 | 11.8 | 9.1 | +11.6 | −0.4 |
  | 500 | AWGN | 5 | 0.7 | 0.7 | +0.0 | −3.1 |
  | 500 | Good | 5 | 1.0 | 1.0 | +0.2 | −2.5 |

  Before and after are the same frames (`mer_reading.py sim` with and without `--receiver`, the
  same profiles in the same order); a run over fewer profiles draws other frames. With
  de-rotation alone the gaps are the same and `model` stays at +3.7 to +9 dB on Poor: the decoder
  is not told, and on one decode it does not need to be (E5).

  De-rotation halves the gap but leaves 4–9 dB on Poor, over the 4 dB registered: what remains is
  the channel's curvature between pilots (two rays ±1 ms about the centre still turn 58° apart
  across 160 Hz), which a phase slope cannot take out.
- **E5: de-rotation passes, the error term misses.** Over Moderate, Poor and NVIS, both airs:
  without de-rotation 775 decodes, without the error term 917, both 919. Where the error term could
  matter is combining copies (HARQ), which this bench does not exercise.
- **E6: de-rotation passes, both miss by one frame.** On the 16 recordings of 2026-11-17 (40 m,
  little delay spread) the model decodes 570 OFDM frames before, 570 with de-rotation (one frame
  each way between rungs 11 and 12) and 569 with both.
- **E7 misses: the Wiener estimator stays.** It loses badly on every fading channel (Poor 129 and
  52 against 236 and 154) but wins on AWGN (180 of 180 against 156 at 2 300 Hz, 139 against 125 at
  500 Hz) and on 2 300 Hz NVIS (16 against 4). On AWGN it is the frequency smoothing: linear
  interpolation keeps the pilots' noise, a filter matched to a short delay spread removes most of
  it.

## 8. The author's answer (2026-11-17)

"De-rotation only": build §2.1, drop the error term. Left for candidates of their own, each
registered before its runs: the curvature E4 leaves on Poor (a higher-order interpolation, or the
denser pilots of P9-3), and the AWGN gain the Wiener estimator shows (a frequency smoothing where
the channel is flat, about 1 dB at the FER points), and a combining bench should the error term
ever come back.

## 9. The port

On a machine with a Rust toolchain, in one change:

1. `aether-phy/src/rx.rs`, `FrameReceiver::receive`: after the known values of the full pilot
   symbols are set (step 4), the slope `φ = arg Σ Ĥ[k+1]·Ĥ*[k]` over the full pilot symbols' direct
   estimates; the linear interpolation of the data symbols (and of ADR-0082's re-estimate) takes
   `e^{−jφc}` off the pilots, interpolates, puts `e^{+jφk}` back, and keeps each pilot carrier's own
   value; the signal power the SNR is reported against stays the plain interpolation's, so
   `snr_carrier_db` is unchanged to the bit. The chip detection is untouched.
2. `FrameReceiver.derotate = True` in the model, the port's equivalent on by default, and
   `python tools/make_phy_vectors.py` regenerates `phy_vectors.json` (the `ofdm_receive` cases'
   symbols and noise move; their SNR does not); add one case whose rays put the channel's centre
   about a millisecond from the timing, so the slope is exercised.
3. The harness A/B (every scenario × 2 draws against the build before), then a beta, with the
   manual unchanged (nothing an operator sees or sets changes).

## 10. Built (2026-11-25, the author's "port it now")

As §9 planned, in one change. `FrameReceiver::derotate` in `aether-phy/src/rx.rs`, on by default,
and `FrameReceiver.derotate = True` in the model. The slope is read over the full pilot symbols
after their chips are known; the data symbols' estimate and ADR-0082's re-estimate take it out,
interpolate and put it back, each pilot carrier keeping its own value; the chip detection before it
stays plain, and the signal power the SNR is read from is the plain interpolation's, of the rows
the data symbols were estimated from (the followed ones when ADR-0082 follows).

* **Vectors.** `phy_vectors.json` regenerated: in the eight `ofdm_receive` cases that existed, every
  SNR is the same to the bit and the symbols moved (up to 10 in the moving channel at 2300 Hz). Two
  cases were added after them, `OFDM_SLOPE_CASES` (a 2300 Hz frame eight samples late; a 500 Hz
  frame on two rays six and ten samples late), last in the list so the cases before keep their
  noise. A sign planted in the port's slope fails the vectors.
* **Tests.** The model's `test_the_estimate_takes_the_delay_out_before_it_interpolates` names its
  plain receiver (`derotate=False`), and `tools/bench_estimate.py`'s `before` arm says
  `derotate=false`: the default changed under them. The Rust workspace (955 tests) passes,
  including the field tier: no recording in `field/sessions/` decodes fewer frames.
* **The harness A/B** against the build before is in §11.

