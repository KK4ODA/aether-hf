# ADR-0043: Each carrier weighed by its own noise; a frame's second copy is RV 0 again

**Status:** accepted, 2026-10-06. The receiver (`phy/rx.py`, `aether-phy/src/rx.rs`), the link
engine's retransmissions (`link/engine.py` `RV_SEQUENCE`, `aether-link/src/engine.rs`), the
scenario harness's new conditions (`tools/channel_server.py`: `level`, `cfo_drift_hz_per_s`,
`sro_ppm`, `agc`; a station's own `bandwidth`; `[expect] idle`) and fourteen stress scenarios
(`bench/scenarios/*`, tag `stress`). No wire change: beta.80 and beta.81 interoperate.

## 1. Context

The author asked for more 80 m and 40 m scenarios, chosen to find what nobody had looked for, with
VARA HF as the bar. Fourteen went into the harness (ADR-0042) — an asymmetric path, a VARA pair
heard by one station only, a 30 dB dropout mid-transfer, the band closing, two radios 25 Hz apart
and drifting with sound cards 80–120 ppm apart, dials 45 Hz apart, a thunderstorm with both rigs
on AGC, −7 dB on a Poor path, an RTTY contest across the passband, a clean +20 dB path for
throughput, flutter, a 2300 Hz station calling a 500 Hz one, a VARA pair on frequency, the
disturbed NVIS channel — with the channel server modelling, newly, a path whose level follows a
schedule, a drifting offset, a sound-card clock offset and a receiver AGC. Most passed. Three
things in what passed were wrong.

**Narrowband interference was trusted.** The receiver estimates noise per symbol (P2-5), which
makes an impulse an erasure, and one value for every carrier of the symbol. An RTTY station sits
on four or five of 42 carriers for the whole frame; its power was spread over all of them, so the
decoder trusted the hit carriers' bits as much as the clean ones. In the contest scenario the
link ran at rungs 5–7 on a +12 dB path; measured frame by frame through the real modem, RTTY at the
signal's power on 40 m Good let 1 frame in 12 decode, and 0 of 12 at −3 dB on Moderate.

**The SNR the rate controller is told had the same fault.** It is the pilots' residual power,
interfered pilots included: 0.4 dB reported for a 12 dB path under one 200 Hz interferer.

**A frame's second copy could not decode on its own.** Retransmissions went out at RV 0, 1, 2,
3. Measured through the real modem (40 tries each): RV 1 and RV 2 decode alone at no SNR, on any
rung; RV 3 decodes alone on the rungs whose RV 3 carries every systematic bit (wide 2300 Hz QPSK
½: 30 of 40 where RV 0 gave 34) and never on the others (the 36 bit/s tone rung on both airs, the
500 Hz OFDM rungs at rate ⅔: its RV 3 carries 69–83 % of them). On HF a retransmission is as often
of a frame the receiver never detected — a fade, a collision, a burst's faded end (ADR-0040) — as
of one it detected and could not decode, and then the next three copies were useless: the 45 Hz
offset scenario spent 85 s sending a rung-1 tone burst at RV 1, 2 and 3 at +5 dB, none of which
could decode, after its RV 0 burst had collided with an acknowledgement.

## 2. Decision

1. **Per-carrier noise** (`FrameReceiver.per_carrier_noise`, on). After equalisation, each data
   carrier's slicing error over the frame's data symbols, in units of the noise the per-symbol
   estimate gave it, is compared with the median carrier's; a carrier at or over
   `carrier_noise_threshold` (2) times the median has its noise variance multiplied by that ratio.
   The hit carriers become erasures, the clean ones keep their weight, and the per-symbol estimate
   still flags impulses. Decision-directed, so it sees an interferer between pilots too.
2. **The reported SNR leaves interfered pilots out** (`_reported_noise` / `reported_noise`): the
   per-pilot-carrier residual power, averaged over the pilots not above `OUTLIER_PILOT` (4) times
   the median pilot's.
3. **Retransmissions go RV 0, 0, 2, 3** (`RV_SEQUENCE`), then round again. The second copy
   decodes alone when the first was missed, and combined with a failed first copy it does as well
   as any (2300 Hz Poor, rung 10, 29 failed first copies: RV 0 again 27, RV 3 29, RV 2 28, RV 1
   24); RV 2 then adds the parity RV 0 does not carry, RV 3 the systematic bits with the outermost
   parity. RV 1 — the worst every way — is dropped. The receiver reads the RV from the chips and
   combines any order, so this is the sender's choice alone.

## 3. Measurements

**Frames through the real modem** (model; 12 per point, interferer always on):

| channel | interferer | before | after |
|---|---|---|---|
| 2300 Good +15 dB, rung 12 | none | 12 | 12 |
| 2300 Good +15 dB, rung 12 | RTTY 0 dB at +300 Hz | 1 | 10 |
| 2300 Good +15 dB, rung 12 | RTTY +6 dB at −500 Hz | 0 (5 missed) | 7 (5 missed) |
| 2300 Moderate +18 dB, rung 14 | RTTY −3 dB | 0 | 11 |
| 2300 Good +8 dB, rung 10 | PACTOR 0 dB | 1 | 7 |
| 500 Good +12 dB, rung 7 | RTTY −3 dB | 3 | 5 |

With no interferer it changes nothing: 588 of 588 frames decode either way over 18 points of
AWGN, Good, Moderate and Poor at both bandwidths (40 each), 256 of 256 on flutter and NVIS, and
the reported SNR moves by less than 0.01 dB on every profile; under a 200 Hz interferer at the
signal's power it reads 10.1 dB for a 12 dB Good path (was 0.4) and 8.4 dB at 500 Hz (was −3.5).

**Whole sessions** (the harness, this build against beta.81, the same seeds):

| scenario | beta.81 | this build |
|---|---|---|
| 80 m 500 Hz, dials 45 Hz apart, 2 kB | 253 s, 63 bit/s | 117 s, 137 bit/s |
| 40 m, 2300 Hz station calls a 500 Hz one, 2 kB | 190 s, 84 bit/s | 113 s, 142 bit/s |
| 80 m 500 Hz, a VARA pair on frequency, 2 kB | 180 s, 89 bit/s | 133 s, 120 bit/s |
| 80 m 500 Hz, −7 dB Poor, 1 kB | 206 s, 39 bit/s | 147 s, 54 bit/s |
| 80 m 500 Hz, disturbed NVIS, 1 kB | 407 s, 7 collisions | 222 s, 3 collisions |
| 80 m 500 Hz, 30 dB dropout, 8 kB | 852 s | 739 s |
| 80 m 500 Hz asymmetric (0/+6 dB), Test | message 57, file 52 bit/s | 71, 75 bit/s |
| 40 m RTTY contest, Test (seeds 19, 29) | message 39 / 100 bit/s, rungs 5 / 6, 1 of 2 complete | 66 / 267 bit/s, rungs 12 / 13, both complete, file 314 bit/s |
| 80 m flutter, Test (seeds 21, 31, 41) | message 61 / 50 / 99, 0 of 3 complete | 95 / 52 / 62, 1 of 3 |

The 2300 Hz scenarios without interference (throughput, hidden VARA, drift and clocks,
thunderstorm) are within a run's noise of beta.81.

## 4. What the scenarios found that this does not fix

* **The turnaround.** With a host that answers at once (Winlink's B2F shape: `bench_chat.py` with
  0.2–1 s between a line's delivery and the reply), a reply waits 13 s at +6 to +12 dB for the
  sender's idle poll; ADR-0027's request (chat mode) makes it 3.4–6 s with no lost lines and
  slightly less keyed time. It is on only under a host's `CHAT ON`; whether to turn it on for every
  session is the author's call.
* **The disturbed NVIS channel** (two equal paths 7 ms apart) is beyond the OFDM symbol's 5 ms
  effective prefix: no OFDM rung survives, and the link runs on the tone floor (P9-3).
* **A steady carrier** at the signal's power off the carrier grid leaks over ten carriers either
  side and still breaks a frame; it wants a notch before the transform.
* **500 Hz on fading paths** is slow everywhere (75–140 bit/s at +6 to +8 dB): a 500 Hz signal
  fades flat, and the frame has no time diversity (P9-5, paused).
* **The climb** from a call: four bursts (33 s) from rung 11 to 17 on a 20 dB Good path, and a
  16 kB message at 2.0 kbit/s overall.

## 5. Consequences

* `SELF_DECODABLE_RVS` stays {0, 3} for counting failures (ADR-0020): RV 3 now goes out only as a
  fourth copy, almost always combined.
* The harness: a scenario's path may carry `level` (`[[seconds, dB], …]`), `cfo_drift_hz_per_s`
  and `sro_ppm`; a station's radio `agc` (`off`, `fast`, `auto`, `slow`); a station its own
  `bandwidth`; `[expect] idle`; each message step's rate is in the result's `transfers`; the runner
  waits for both stations to come back to idle before judging.
