# The hidden-frame bench: plan and criteria

*Registered 2026-10-24, before any run of the rule under test. Report 16 §3 proposed the rule
and said to measure it on a bench if the case came back; report 17 §6 found it twice more, and
the author asked for the bench the same day.*

## 1. The problem

The streaming receiver drops an OFDM candidate that starts inside the span of a frame it already
knows: one still pending, or one handed out in the last twenty, decoded or not
(`StreamingReceiver.feed` in `model/aether_model/phy/stream.py`, `known()` in
`core/aether-phy/src/stream.rs`). It is there so that a frame's own data symbols, which correlate
with the preamble reference at 0.3–0.4, and the lookback's re-detections are not taken for new
frames.

It also lets a false detection hide a real frame. On the air, three times:

| session | at | the false candidate | the frame it hid |
|---|---|---|---|
| ND1J's Test of 2026-10-23 16:07Z (report 16 §3) | ND1J | data r8, acquisition 1.04, 0.44 s before | KK4ODA-1's answer, about 11 dB over the noise |
| ND1J's Test of 2026-10-24 15:55Z (report 17 §6) | KK4ODA-1 | control, 1.04, 0.26 s before | ND1J's header, about 8 dB over the noise |
| the same exchange | ND1J | data r14, 1.12, 0.34 s before | KK4ODA-1's answer, about 10 dB over the noise |

The offline receiver does not have the fault. `FrameDetector.detect` takes the strongest peak in
the whole buffer first and masks only that frame's span, so offline the real frame is taken and
the false one beside it as well; the stream, which sees the false one first, keeps it. The
streaming receiver is meant to give what `Modem.decode_buffer` gives on the same input.

## 2. The rule under test

**R1: a known frame acquired under `TRUSTED_CONFIDENCE` (1.3) that has not decoded — still
pending, or handed out and failed — does not hide a candidate acquired at 1.3 or more.** The new
candidate is taken beside it; at harvest, a candidate that starts inside a frame that decoded is
dropped undecoded. A duplicate (a start within a symbol of a known one) is hidden as before.

* **Why 1.3.** It is the daemon's `DETECT_CONFIDENCE_TRUSTED`, set from the OTA-2 record: every
  frame that decoded cleared its threshold by a wide margin, and the noise triggers sat just above
  it. The three false candidates read 1.04–1.12; the frames they hid read 1.5–1.8.
* **Why it should not add false frames.** A frame's own data symbols correlate at 0.3–0.4 against
  thresholds of 0.348 (2300 Hz) and 0.56 (500 Hz): at most about 1.15 and 0.7, under 1.3 on both
  airs. A known frame that decoded, or was acquired at 1.3 or more, still hides everything.
* **R0** is the rule as built (`weak_spans_hide=True`, the default). R1 is
  `StreamingReceiver(weak_spans_hide=False)`, in the model only, bench-only.
* **Not first: R2**, the offline rule, where the stronger peak wins whatever its confidence. It
  matches `decode_buffer` exactly, but lets a data symbol stronger than a faded frame's preamble
  take the frame's place. Measured only if R1 fails K2.

## 3. What the bench measures

`tools/bench_hidden.py`, on the model's receiver, both airs.

* **M1, the field** (`field`): every two-sided recording in hand replayed through R0 and R1,
  20 ms blocks as the daemon feeds them. The three cases of §1 and everything else each rule
  found.
* **M2, recovery** (`synth`): a false candidate followed by a real frame inside its span. The
  false candidate is a preamble with nothing after it (the first two symbols of a control frame,
  cut), at a level that acquisition reads 1.0–1.25, which the receiver sees as it sees a noise
  trigger: an untrusted acquisition that does not decode. The real frame, an ordinary control
  frame or a data frame of a rung the link uses there, starts 0.1 s to one frame span later, at an
  SNR where it decodes alone. Recovered: decoded by the stream. The reference: the same frame with
  no false candidate before it.
* **M3, losses and false frames on traffic** (`traffic`): the link's own transmissions (a header
  and one to six data frames back to back; answers; tone frames) through the model's channel —
  AWGN, ITU-R F.1487 Good, Moderate, Poor and flutter — at −8 to +20 dB, minutes of air per
  condition. Frame by frame, R0 against R1: a real frame decoded under R0 and not under R1 is a
  **loss**; a candidate under R1 that starts at no real frame (more than a symbol from every real
  start) and that R0 did not take is a **false frame added**.
* **M4, noise** (`noise`): an hour of noise per air through both rules: candidates per hour.
* **M5, cost**: decode attempts per real frame, R1 against R0, from M3.

The parameters, set before the runs:

* M2: the false candidate is a cut control or a cut data preamble, at an SNR drawn from the
  window where acquisition's median reads it 1.0–1.25 (`calibrate`, per air and type); the real
  frame is an ordinary control frame or a data frame at rung 8 or 12 (2300 Hz), 7 or 11
  (500 Hz), at 1, 3, 6, 10 and 15 dB over its rung's AWGN threshold, on AWGN; 20 trials a cell,
  1 200 in all. A trial counts when the reference decodes the real frame and the as-built stream
  handed out the false candidate. Every trial is in the CSV, so the cells can also be read by the
  false and real frames' own confidences.
* M3: 120 s of air per condition (two airs, five channels, eight SNRs: 80 conditions, about
  2.7 hours). A transmission is a header and one to six data frames at one rung (55 %), a lone
  answer (30 %) or a tone frame (15 %), with 0.15–1 s between transmissions; the rung is drawn
  from the OFDM rungs whose AWGN threshold is from 10 dB under to 2 dB over the SNR.
* M4: six ten-minute chunks of noise per air.

## 4. The criteria

Registered now. A criterion missed narrowly goes to the author with its trace, case by case; it
does not pass by rule.

| | criterion | passes when |
|---|---|---|
| K1 | the field | R1 decodes all three hidden frames of §1, and loses no frame R0 decodes in any recording replayed |
| K2 | recovery | R1 decodes at least 95 % of the M2 frames that decode in the reference, at both airs, for control and data frames; R0's share is reported beside it |
| K3 | no loss | M3: no real frame decoded under R0 and not under R1, at either air, on any channel |
| K4 | false frames | M3: at most one false frame added per 1 000 real frames, and none of them decoded; M4: no more candidates per hour than R0 plus one |
| K5 | cost | M5: at most 5 % more decode attempts per real frame |

## 5. Then

If R1 passes: an ADR; R1 made the model's rule; the port's `known()` the same, with a streaming
test of a false candidate before a real frame in both suites (the stream tests already hold the
streamed output to `decode_buffer`); the field recordings replayed through the port's receiver
(`aetherd --replay`, before and after); the scenario harness, which judges what a receiver change
does to sessions; then a beta. If it fails, the failure is explained with its trace before any
other rule is tried.
