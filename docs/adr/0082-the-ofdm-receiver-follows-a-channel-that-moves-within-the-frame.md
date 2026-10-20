# ADR-0082: The OFDM receiver follows a channel that moves within the frame

**Status:** accepted, 2026-10-20. Model first (`FrameReceiver._motion` and `_follow` in
`model/aether_model/phy/rx.py`), then the port (`motion` and `follow` in `aether-phy/src/rx.rs`),
held together by the new `ofdm_receive` cases of `phy_vectors.json`. **Receiver only: no wire
change, no configuration key.** A station on this build reads the same frames as before, better
on a fast fade, and sends the same ones.

## 1. Context

ADR-0081 §5 found that on ITU flutter (10 Hz) the 500 Hz OFDM receiver read about −3.4 dB
whatever the path's SNR, and decoded 16-QAM ½ in 4 frames of 20 at +8 dB. The reading held the
rate controller under rung 5 and the control frames on the floor, and `80m-flutter-500` carried
its message on the tone floor at +8 dB.

The receiver estimates the channel from the comb pilots, which are on every symbol, and
averages each pilot over three symbols, a third each. On a steady channel that takes two thirds
of the pilots' noise away. On flutter the channel turns over in about three symbols (31 ms
each), so the average is of a channel that has moved, and the estimate's own error becomes the
frame's noise. The noise the receiver reports comes from the pilots' residuals against that
average, so the SNR reading measured the same error.

A twin of the channel without noise (the same fading seed) gives the true channel at every
symbol and carrier. Measured on 500 Hz flutter frames (20 a point, genie channel against the
receiver as it was against a per-symbol estimate, no time average):

| +12 dB | Receiver as it was | Per-symbol estimate | True channel |
|---|---|---|---|
| QPSK ⅔ | 15 | 20 | 18 |
| 8PSK ½ | 17 | 20 | 20 |
| 16-QAM ½ | 5 | 20 | 16 |
| 16-QAM ⅔ | 0 | 8 | 10 |

The per-symbol estimate is about 7 dB better in effective SINR at +12 dB and above. What is
left is the channel changing inside one symbol (inter-carrier interference, about 13 dB below
the signal at 10 Hz on 25 ms symbols), which no estimator removes.

## 2. Decision

The three-tap time filter becomes `[w, 1 − 2w, w]` (`[1 − w, w]` at the frame's ends), and
`w` is chosen per frame from two second differences of the pilots:

* **Across frequency, on the full pilot symbols** (every carrier known, one instant): their
  power over six is the noise, with whatever interferes between carriers and the transmitter's
  peak reduction (ADR-0004), and little of the channel for the delay spreads the prefix covers.
* **In time, on the comb pilots** (`D2`): six times the noise plus the channel's own change
  from symbol to symbol. Taken as the median over symbols, so an impulse in a few symbols is
  not read as motion.

The filter that leaves the least error has `w = 2·noise / D2`: a third on a steady channel,
which is the filter as it always was, and less the more the channel moves. The receiver
follows the channel (`motion_weight` on `ReceivedFrame`) only when:

* the motion, `D2 − 6·noise`, is at least a tenth of the pilots' power (`MOTION_SHARE`), which
  a Gaussian Doppler spectrum reaches at about 4.5 Hz (2σ) and ITU Poor (1 Hz) never does; and
* `w` is at most 0.2 (`MOTION_WEIGHT_MAX`): the motion is at least four times the noise in `D2`.
  The noise is read from 20 samples (a 500 Hz control frame) to 220. With the share alone the
  receiver followed a third of steady frames at a few decibels, where the noise had read low,
  and those frames' SNR read high by as much.

Where it follows, the symbols between the full pilot symbols are re-estimated with that
weight. The residuals from the steady estimate keep their shape (which symbols an impulse hit,
which carriers an interferer sits on, ADR-0043), scaled to the frame's noise as read across
frequency, plus the error the new filter leaves beyond what the steady one leaves on a steady
channel: `noise · (5/3 − 2w)`, never more than the steady estimate's own. The SNR reading is
taken against that same figure, so it reports what the frame could carry, about 2 dB under the
inter-carrier-interference ceiling.

Everything else is as it was: the chips are read from the steady estimate first, and a steady
frame is received bit for bit as before (tested on AWGN, ITU Poor and under impulses, both
airs).

## 3. Measured

The same frames and the same sync, the receiver as it was against this ADR's (20 frames a point;
decoded, then the SNR reading's median):

| 500 Hz flutter | +4 dB | +10 dB | +18 dB |
|---|---|---|---|
| Control frame | 18 → 19, −4.2 → −1.5 | 18 → 20, −3.3 → +0.5 | 18 → 19, −4.0 → +0.4 |
| QPSK ½ (rung 5) | 18 → 19, −3.6 → +0.4 | 19 → 20, −3.1 → +1.6 | 20 → 20, −2.9 → +3.0 |
| 8PSK ½ (rung 7) | 5 → 11, −4.1 → −0.1 | 13 → 20, −3.5 → +2.3 | 17 → 20, −2.8 → +3.7 |
| 16-QAM ½ (rung 9) | 0 → 6, −3.9 → −1.3 | 6 → 18, −3.2 → +2.0 | 7 → 19, −3.1 → +3.2 |
| 16-QAM ⅔ (rung 10) | 0 → 1 | 0 → 3, −3.3 → +2.7 | 0 → 6, −3.8 → +3.4 |

| 2 300 Hz flutter | +10 dB | +18 dB | +26 dB |
|---|---|---|---|
| QPSK ⅔ | 1 → 13, +1.6 → +5.3 | 17 → 20, +3.1 → +8.5 | 17 → 20, +3.4 → +9.3 |
| 8PSK ⅔ | 0 → 0 | 0 → 10, +2.7 → +8.7 | 0 → 16, +3.7 → +9.5 |
| 16-QAM ⅔ | 0 → 0 | 0 → 0 | 0 → 5, +3.2 → +8.9 |

ITU high-latitude moderate (3 ms, 10 Hz) at 500 Hz: QPSK ½ 5 → 9 at +10 dB and 5 → 10 at
+18, control frames 8 → 11 and 7 → 11. ITU high-latitude disturbed (7 ms, 30 Hz) decodes
nothing either way: its echo is longer than the prefix.

The reading rises with the path and stops at what the frame can carry: on 500 Hz flutter it
puts 16-QAM ½ (AWGN threshold −0.1 dB) within reach from about +10 dB, which decodes 18–20 of
20 there. At 2 300 Hz the receiver follows the channel from about +10 dB. Below that the best
weight is close to a third anyway, so the cap leaves the steady filter.

On steady channels (AWGN, ITU Good, Moderate, Poor, and impulses at 25 dB over the noise, both
airs, −2 to +26 dB, 20 frames of six modes a point) the receiver followed 0–3 frames of 20, short
control frames mostly. Decodes were unchanged except one control frame in 20 under heavy
impulses with no blanker in front (19 → 18 at −2 dB), and the median readings were unchanged
except a few control frames' (+0.3–0.8 dB).

## 4. Alternatives

* **No time average at all** (the per-symbol estimate of §1): the best on flutter at high SNR,
  but it gives up the steady channel's two thirds of the noise. That is what a weight from the
  measured motion keeps.
* **The Wiener time filter** (`channel_estimator = "wiener"`, P2-6) designed for 10 Hz: a fixed
  design loses on the steady channels where the linear estimator is the default for good
  reason (P2-6). The weight here is the same idea, reduced to the one tap that matters,
  designed per frame.
* **The noise from the time residuals alone**: they hold the motion and the noise in the same
  sum, and only a model of the Doppler spectrum separates them. The full pilot symbols give
  the noise without any model.

## 5. Consequences

* The SNR reading on a fast fade now follows the path. The rate controller and the rule that
  picks the control frames' family (the weakest of the last three readings, ADR-0069) read it as
  before, so a session on flutter climbs off the floor at the SNR where the frames decode.
* The OFDM receiver had no model vectors at the level of its symbols and noise. The new
  `ofdm_receive` cases (steady channels, and two paths whose Doppler shifts beat at 9–12 Hz,
  both airs) hold the port to the model's motion decision, offset, SNR, symbols and noise
  variances.
* The manual's SNR reading says what it reads on a fast fade.
