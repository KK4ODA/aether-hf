# OTA test 2 — 300 ft ground wave, and why it was worse than it looked

2026-09-17/18 (23:48–00:21 UTC), KK4ODA-1 home FTDX10 into a T2D ↔ KK4ODA-2 mobile TS-480
into a screwdriver, both at 10 W, **300 ft apart**, 40 m, 500 Hz. Ten sidecars (six home,
four mobile), analysed with their audio. Companion to [OTA-FOLLOWUPS.md](OTA-FOLLOWUPS.md)
(test 1).

The operator's report — the base sounded "choppy" in the truck, and the stations only
seemed to connect when it sounded continuous — is exactly right, and the recordings say
what it means.

Everything below is held to `aetherd --replay`, which reproduces both sides of the best
pair exactly as they ran on the night (mobile 60 found / 16 decoded, base 42 / 2), so the
receiver's behaviour over this audio is deterministic and can be re-measured against any
change.

## What happened

| | |
|---|---|
| Sessions | 6 home, 4 mobile; three pairs overlap in time |
| Connections | every session reached Connected or was answering |
| Endings | **every one** in `link timeout` or `no answer` |
| Frames | 245 detections, **30 decoded (12 %)** |
| Modes that ever carried | 0, 2, 3, 4 — the floor family and the two slowest ordinary |
| Reported SNR on decoded frames | +4 to +10 dB |
| Dial in the sidecars | 7082 kHz, then 7066 kHz, then **null** for the last four |

The best pair (home `20260918-001511` ↔ mobile `20260918-001658`, 77 s of overlap) splits
cleanly into two phases.

**Phase 1 — the link works, both ways.** From t=108 to t=141 (common clock) every exchange
succeeds: the base sends a burst of five frames, the truck decodes all five at chip
confidence 3.4–4.0, the truck acknowledges, and the base decodes the acknowledgement at
+5.9, +7.0, +8.4, +8.6, +10.3 dB. Nothing is lost. 356 bytes are delivered.

**Phase 2 — total collapse.** At t=141 the base begins the test session's 1024-byte
message and keys for **26 seconds continuously**. From that moment the truck decodes
**nothing**: 40 detections, 0 decoded, confidence never above 1.29. The truck's timers
fire and it transmits twice *on top of* the base. The session times out.

The signal level does not change between the two phases — the truck's in-band envelope is
−18.5 dB throughout, steady to 2.3 dB. What changed is the shape of the transmission:
short bursts with silence between them worked; one long continuous burst did not.

## Finding 1 — the demodulator sees 7 dB where the spectrum has 26 dB

Measured on the mobile's recording over two bursts that decoded perfectly (5 frames each,
confidence 3.4–4.0), signal and noise taken in the *same* AGC state:

```
in-band PSD (1280-1720 Hz)   -44.1 dB
noise PSD above (1900-2650)  -72.1 dB      noise below (600-1150)  -68.2 dB
-> 25.6 dB in the signal bandwidth  =  +17.8 dB referenced to 3 kHz
```

The modem reported **+6.1 to +8.3 dB** on those very frames. The estimator is not wrong
about what it measures: `snr_carrier = signal_power / sigma2` at `rx.rs:421`, where
`sigma2` is the pilot residual *inside* the frame. So it is reporting effective SNR — EVM —
and **the path is carrying about 19 dB of distortion that is not noise.**

That number drives everything downstream. The rate controller stepped the link *down*
4 → 3 → 2 while every single frame was decoding and every acknowledgement arriving,
because +7 dB is what it was told. At the true +17.8 dB the narrow table would have run
several modes higher and the 1024-byte message would very likely have finished inside the
timeout.

Three candidates for the 19 dB, in order of suspicion:

1. **Transmitter ALC clipping.** OFDM has a high peak-to-average ratio; ADR-0004's peak
   reduction is upstream of the rig's own ALC. `tx_level` was 0.141 at the base and 0.250
   at the mobile — the highest of any session logged.
2. **Receiver front-end compression.** 10 W at 300 ft is an enormous signal. Both rigs were
   running with no attenuator.
3. **AGC action within a frame.** The mobile's AGC moves 13 dB between burst and no-burst
   (noise PSD −72.1 during the burst, −59.0 between). Within a burst it is steady to 2.3 dB,
   so this is the weakest of the three — but it is not nothing.

**The discriminating experiment is cheap:** repeat with the RX attenuator in at both ends
(20 dB — there is signal to spare) and the TX drive backed off until the ALC barely moves.
If the reported SNR jumps toward the spectral figure, it was 1 or 2. Run one leg with each
change alone to separate them.

## Finding 2 — acquisition false-alarms about seven times per real frame

215 false detections in 15.8 minutes — **13.6 per minute** — against 30 real decodes. The
two populations do not overlap:

```
chip confidence  1.0 - 1.5   ->  213 detections, 0 decoded (except control frames, below)
chip confidence  2.8 - 4.0   ->   22 detections, 22 decoded
```

115 of the 215 were floor-family control-frame detections, and at the base end it is worse
still — 24 of its 40 failures in one session. The floor detector is new in beta.20
(ADR-0009) and its threshold of 0.32 was set from the noise maximum **on AWGN**; a 40 m
evening with CW QRM is not AWGN. For comparison, a 156 s idle listen on 2.3 kHz (no floor
detector) recorded **zero** detections.

The beta.27 CFO gate is very nearly the right cut: exactly one phantom of the 215 reported
an offset (`66.20 s data mode 1 rv 3 -11.5 dB cfo +30.5 Hz`), so its confidence reached
`MODE_RETRY_CONFIDENCE`. The spurious offsets that dominated OTA-1 are rare now rather than
gone.

### It is not, however, why the long burst failed — that was tested and refuted

The obvious suspicion was that phantom frames blind the receiver. In `stream.rs:188-199` a
candidate whose start falls inside a span already claimed is dropped as a duplicate, and a
phantom floor frame claims 2.2-4.2 s of stream; over the 43 s of phase 2 the claimed spans
cover 90 % of the window. It fitted "short bursts work, long bursts do not" exactly.

**It is wrong.** Measured, on the 30 s of the collapse window, with the suppression
compiled out and the binary verified to contain the change:

| | candidates | decoded |
|---|---|---|
| suppression as shipped | 26 | **0** |
| suppression disabled | 1199 | **0** |

Suppression really is hiding a great many candidates — forty-six times as many — but not one
of them is a frame. The cost of the false alarms is wasted work and a useless confidence
signal, not deafness: a smaller claim than this section first made, and the measurement is
why.

### What they did cost — found by the operator, not the recordings

Watching the panel on the next evening: the level never reached 6 dB over the floor, and
the busy indicator stayed lit for seconds at a time. Every acquired preamble marked the
channel busy for two seconds, extended the receive window by a frame, and told the link
engine the other station was still transmitting — sound reasoning before the floor detector
existed, and each phantom did all three. From these sidecars, with the two-second hold:

| | held busy by phantoms | by real frames alone |
|---|---|---|
| base | **47 %** of the evening | 4 % |
| mobile | **21 %** | 6 % |

Gated on `detect_confidence` in beta.35. The engine being told a burst was still arriving
held this station's own turn, so replies waited on frames that never existed — which
belongs on the list of things that may have ended eleven sessions in eleven with a link
timeout.

The receiver's **6 s buffer bound** against a 26 s burst was the other structural suspect,
and it is refuted the same way — replayed at `max_buffer_s = 40`, the collapse window gives
40 detections and 0 decodes, identical to the shipped 6 s:

| | collapse window | decoded |
|---|---|---|
| buffer 6 s (shipped) | 40 | **0** |
| buffer 40 s | 40 | **0** |

So nothing in the receiver's bookkeeping is responsible. **The audio does not contain
decodable frames**, and the cause is at the transmitter.

### Which makes findings 1 and this the same finding

The base's drive was set on a tune tone, so it was transmitting 6-7 dB into its ALC
(finding 1, now measured). Clipping does not damage all modes alike: it costs a sparse
constellation almost nothing and a dense one everything — ADR-0004's own table has 16-QAM 3/4
going from zero frame errors to 0.45 against an ALC-like PA. Every frame that decoded all
evening was QPSK (modes 2, 3, 4). The 1024-byte message is the one transmission that would
have gone out faster than that.

That is the whole shape of phase 2: **a clipped transmitter can carry the control frames and
cannot carry the payload**, so the link establishes, converses, and then goes silent the
moment it tries to move data. It also explains why the rate controller stepping *down* never
rescued it — by then the burst was already in the air.

Not proven, because a station does not record the modes it sends (see the action items).
That one change would settle it from the sidecars already on disk.

## Finding 3 — a control frame never gets a real confidence

`rx.rs:307`:

```rust
let (mut mode, mut rv, mut runner_up, mut confidence) = (0usize, 0u8, 0usize, 1.0f64);
if sync.frame_type == FrameType::Data {
    ...  // confidence is only ever computed here
}
```

Every CONTROL frame reports confidence **exactly 1.00**, genuine or not. In this test's
data that is 143 false detections and 8 real connect/poll/ack frames, indistinguishable by
the one number meant to distinguish them. Consequences:

- nothing downstream can reject a false floor-frame acquisition;
- the CFO gate added in beta.27 (`MODE_RETRY_CONFIDENCE` = 1.3) suppresses the CFO of every
  control frame, including the real ones;
- `modem.rs:346`'s runner-up retry treats all of them as low confidence.

The floor family carries connect, poll and acknowledgement — the frames the link cannot do
without. They are the ones with no confidence at all.

## Finding 4 — the "choppy" audio was the protocol, not a fault

Not a transmit dropout. The base's PTT log shows 1–2 s keyings with 3–24 s gaps whenever
it was retrying control frames under backoff, and 6.4 s and 13.3 s keyings when data was
actually moving. Choppy *is* the sound of a link that is not getting its acknowledgements
through; continuous is the sound of one that is. The ear read it correctly.

The two 0.6 s holes that appear inside the base's 26 s burst on the mobile's recording are
the **mobile's own transmitter** muting its receiver (same spectrum as its other keyings:
rms −30 dBFS, everything above 600 Hz gone). Which is its own problem — the mobile
transmitted twice on top of a burst it had stopped being able to decode.

## Finding 5 — operating notes

- **The mobile's filter was wide open the whole time**, ~2.7 kHz flat, in both recordings —
  not the 1000 Hz hi-cut. The modem band-limits anyway, so this costs nothing in the
  detector, but the rig's **AGC** rides on the full 2.7 kHz including the CW QRM, and that
  is what makes a 500 Hz signal duck. A ~500 Hz filter centred on 1500 Hz is worth setting.
- **The dial went null.** The last four sidecars carry `frequency_hz: null` — CAT stopped
  reporting. The first two say 7082 and 7066 kHz; the operator's note says 7064. A
  recording without its dial is much less useful later.
- **Distance did not help and would not have.** At 300 ft the link is not noise-limited at
  all (26 dB spectral SNR). Everything above is a modem problem or a rig-setup problem;
  moving the truck further away would only have added the noise that was missing.

## Action items

1. **Find the 19 dB.** Part of it is now accounted for: drive was being set against a tune
   tone whose peaks sit 5.8-7.0 dB *below* the waveform's (measured at the sound card), so
   the rig was being run that far into ALC limiting. `drive.set` and `metrics.tx_peak_dbfs`
   (beta.32) fix the instrument; the next test measures how much of the 19 dB it was.
   What remains: the attenuator leg — the video of the base shows **ATT OFF with IPO AMP1**,
   a preamp on, at 300 ft from a 10 W transmitter.
2. **Raise the floor detector's threshold, or qualify it.** 13.6 false alarms a minute on a
   real band. Needs a measurement against recorded band noise, not AWGN — `tools/floor_trace.py`
   is the shape of the tool. ADR amendment to 0009.
3. ~~**Record the modes a station transmits.**~~ — **done** (beta.32). Both structural
   explanations for the silent 26 s burst are refuted by measurement (claimed spans, and
   the receive buffer), which leaves the transmitter — and the leading account is a clipped
   PA carrying QPSK control frames but not a denser payload mode. It could not be confirmed,
   because a sidecar recorded only what the station *received*. A sidecar now carries `sent`
   (t_s, kind, mode, rv, floor, bytes for every frame put on the air) and a `tx_peak` event
   per transmission, so the next test answers both questions from its own recordings. The
   OTA-2 recordings themselves predate it and cannot be re-read this way.
4. ~~**Give control frames a real confidence**~~ — **done** (beta.32). `FrameSync` carries
   the acquisition peak through to the receiver as the model's always did, and
   `detect_confidence` — the peak over the threshold that accepted it — is reported on the
   frame event and in the sidecar, so the next test's phantoms can be counted from the
   modem's own telemetry rather than offline.
5. Carry forward from test 1: the **compression desync** ADR (compression was off in this
   test, so nothing new was learned).

## Next test

Same 300 ft geometry — it is a good bench, because the path is not the variable. Add:
20 dB attenuator both ends, TX drive backed off to almost no ALC, mobile filter at ~500 Hz,
AGC FAST or OFF both ends, **the preamp off (IPO) or the attenuator in**, a dial that CAT
actually reports, and one leg on a quiet part of 40 m (7.052–7.065) rather than 7064 with CW
QRM on top. Set drive with **Set drive**, not the tune tone, and note `tx_peak_dbfs`.

One caution on `wait_for_clear`: on 7064 that evening the shipped busy detector read busy
**0 % of the time** on genuine receive audio, because the band was so uniformly occupied
that the learned floor sat at the occupancy level and nothing ever stood 6 dB above it. A
minimum-statistics floor cannot tell a quiet band from a wall-to-wall busy one, so the
setting protected nothing. That belongs with item 2.
