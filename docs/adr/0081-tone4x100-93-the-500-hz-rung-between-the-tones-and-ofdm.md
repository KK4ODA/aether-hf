# ADR-0081: tone4x100-93, the 500 Hz rung between the four-tone kinds and OFDM: link protocol 10

**Status:** accepted, 2026-10-20. Model first (`aether_model/frame/modes.py`, `link/rate.py`),
then the port (`aether-phy`'s tables, `aether-link`'s, `aetherd`'s occupancy and sidecar
format). Builds on ADR-0015 and ADR-0080. **Link protocol 10**: a 500 Hz rung 4 names another
frame than in version 9, so a station on this build and one on beta.116 do not connect. No
configuration change.

## 1. Context

On the testers' 500 Hz paths the link sits at rung 3, `tone4x100-75` (72–74 bit/s in
report 13), because rung 5, QPSK ½ on the ordinary frame, needs about 7.8 dB more on AWGN and
more than that on the fading classes. Rung 4, QPSK ⅓, has not been offered since ADR-0073: after
its frame's header it carries 91 bit/s, less than rung 3's 107, from 7 dB more. Report 12 §18.4
sized the missing step: 130–150 data bit/s, its 10 % point near −1 dB on Moderate and −2 dB on
Poor, and put the decision on a `bench_tone.py` curve and then `bench_link --rung`.

ADR-0015 had measured the four-tone frame at 93 bytes and not built it: it fell off a cliff on
the fading classes. ADR-0080's receiver, which reads each tone's level from the data, brings it
back, and no new waveform is needed — neither spread tones (P9-12) nor an interleaver (P9-5).

## 2. Decision

**`tone4x100-93`** — the floor's frame with 440 data symbols on the four tones at 100 Bd, as
the other two middle kinds, carrying 93 bytes at rate 0.87 (139 bit/s, 134 after the DATA
header) — **takes rung 4 of the 500 Hz ladder**, in the place of QPSK ⅓:

| Rung | Kind | Payload | Data bit/s | AWGN | Good | Moderate | Poor |
|---|---|---|---|---|---|---|---|
| 3 | tone4x100-75 | 75 B | 107 | −13.0 | −3.6 | −6.2 | −7.2 |
| **4** | **tone4x100-93** | **93 B** | **134** | **−11.1** | **−0.7** | **−2.2** | **−3.8** |
| 4 (was) | QPSK ⅓, LONG | 15 B | 91 | −6.0 | +1.0 | +2.0 | −1.3 |
| 5 | QPSK ½, LONG | 25 B | 167 | −5.2 | +4.0 | +3.5 | +0.0 |

(10 % FER through the 500 Hz detector, 100 frames a point, `bench/baselines/tone_floor.csv`
and `phy_fer_500.csv`; rung 3 with ADR-0080's receiver.) It meets report 12's target by 1.2 dB
on Moderate and 1.8 on Poor, and splits the gap between rungs 3 and 5 into two steps — 4.0 and
5.7 dB on Moderate, 3.4 and 3.8 on Poor. A run on other frames and SNRs, with the first build of
ADR-0080's receiver, read −0.4, −2.8 and −3.1 dB on Good, Moderate and Poor; the detector found
every frame at every point, and on Poor decoded more than genie timing does (the detector's
timing sits between the two rays). On ITU flutter (ADR-0080 §2.4) it decodes as the receiver
before ADR-0080 decoded it, 56–59 of 60 from 0 to +6 dB through the detector.

1. **Its sync patterns** are the seeded search carried on (ADR-0014, ADR-0015): patterns 33–36,
   21.3 million draws in. The 500 Hz detector looks for it beside the other kinds; four minutes
   of noise read the same maximum as before (2.82–2.92 against the threshold 3.0) and no frame,
   and the 2 300 Hz air's detector finds nothing in a strong one.
2. **The ladder keeps fifteen rungs.** The OFDM modes on it start at QPSK ½ (mode 3, rung 5);
   modes 0–2 are on no rung. Every other rung means what it did, so a configuration's
   `max_mode` keeps its meaning and no migration is needed: a station held to rung 4 is held to
   a frame faster and more robust than the one it named.
3. **The tone floor is rungs 0–4**: a 500 Hz session's first five rungs share the floor's frame,
   its one family per burst and its control frames (ADR-0013).
4. **Its occupancy** is measured as the others' (`tools/make_occupancy.py`): ±223 Hz by power,
   −311 … +305 Hz by spectral density, as the other four-tone kinds.
5. **Link protocol 10**, the shell's `LINK_PROTOCOL` with it; the release notes say *Update
   required*.
6. **Sidecars are `aether-hf-session/5`.** The replay still reads `/4` recordings (it decodes
   their audio afresh); `field_ingest.sidecar_rung` maps a `/4` 500 Hz rung 4 to no rung.

## 3. Measured: sessions

**The link bench** on the fading pipe at 500 Hz (`bench_link.py --fading --bandwidth 500`, 30
sessions of 1 kB a point; sessions completed, then median bit/s;
`bench/baselines/link_narrow_rung4.csv`). Each arm runs its own code against the calibration of
its own curves: beta.116 from a worktree; beta.116 with ADR-0080's receiver; the same with
`--rung 4=93@-11.1~3`, the counterfactual report 12 proposed (a rung of 93 bytes from −11.1 dB
in rung 3's frame and fading); and this ADR, the rung itself with its own fitted β:

| 500 Hz | -14 dB | -12 dB | -10 dB | -8 dB | -6 dB | -4 dB | -2 dB | +0 dB | +2 dB | +4 dB | +6 dB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| AWGN, beta.116 | 30, 27 | 30, 34 | 30, 46 | 30, 58 | 30, 58 | 30, 71 | 30, 71 | 30, 116 | 30, 140 | 30, 160 | 30, 192 |
| AWGN, receiver (ADR-0080) | 30, 27 | 30, 34 | 30, 46 | 30, 58 | 30, 58 | 30, 71 | 30, 71 | 30, 116 | 30, 140 | 30, 160 | 30, 192 |
| AWGN, --rung 4=93@-11.1~3 | 30, 27 | 30, 34 | 30, 46 | 30, 58 | 30, 71 | 30, 74 | 30, 82 | 30, 116 | 30, 140 | 30, 160 | 30, 192 |
| AWGN, this ADR | 30, 27 | 30, 34 | 30, 46 | 30, 58 | 30, 71 | 30, 74 | 30, 82 | 30, 116 | 30, 140 | 30, 160 | 30, 192 |
| Good, beta.116 | 29, 17 | 30, 21 | 30, 24 | 30, 38 | 30, 49 | 30, 54 | 30, 60 | 30, 63 | 30, 72 | 30, 81 | 30, 124 |
| Good, receiver (ADR-0080) | 29, 17 | 30, 21 | 30, 24 | 30, 42 | 30, 50 | 30, 54 | 30, 61 | 30, 63 | 30, 72 | 30, 81 | 30, 124 |
| Good, --rung 4=93@-11.1~3 | 29, 17 | 30, 21 | 30, 24 | 30, 38 | 30, 51 | 30, 58 | 30, 65 | 30, 67 | 30, 78 | 30, 87 | 30, 124 |
| Good, this ADR | 29, 17 | 30, 21 | 30, 24 | 30, 38 | 30, 51 | 30, 60 | 30, 67 | 30, 68 | 30, 79 | 30, 87 | 30, 124 |
| Moderate, beta.116 | 30, 20 | 30, 23 | 30, 31 | 30, 45 | 30, 52 | 30, 65 | 30, 71 | 30, 65 | 30, 72 | 30, 85 | 30, 127 |
| Moderate, receiver (ADR-0080) | 30, 20 | 30, 23 | 30, 31 | 30, 49 | 30, 54 | 30, 71 | 30, 71 | 30, 65 | 30, 72 | 30, 85 | 30, 127 |
| Moderate, --rung 4=93@-11.1~3 | 30, 20 | 30, 23 | 30, 31 | 30, 49 | 30, 53 | 30, 65 | 30, 70 | 30, 71 | 30, 80 | 30, 91 | 30, 127 |
| Moderate, this ADR | 30, 20 | 30, 23 | 30, 31 | 30, 49 | 30, 53 | 30, 68 | 30, 71 | 30, 71 | 30, 81 | 30, 91 | 30, 127 |
| Poor, beta.116 | 30, 22 | 30, 24 | 30, 39 | 30, 46 | 30, 54 | 30, 65 | 30, 66 | 30, 64 | 30, 80 | 30, 107 | 30, 143 |
| Poor, receiver (ADR-0080) | 30, 22 | 30, 24 | 30, 40 | 30, 51 | 30, 58 | 30, 71 | 30, 71 | 30, 64 | 30, 80 | 30, 107 | 30, 143 |
| Poor, --rung 4=93@-11.1~3 | 30, 22 | 30, 24 | 30, 40 | 30, 49 | 30, 54 | 30, 65 | 30, 71 | 30, 74 | 30, 89 | 30, 107 | 30, 143 |
| Poor, this ADR | 30, 22 | 30, 24 | 30, 40 | 30, 50 | 30, 57 | 30, 71 | 30, 79 | 30, 77 | 30, 89 | 30, 107 | 30, 143 |

Every session completes in every arm. At 0 and +2 dB on the fading classes the rung makes
sessions 8–20 % faster than beta.116 (Moderate +2 dB 72 → 81, Poor 0 dB 64 → 77), Poor at −2 dB
20 % (66 → 79), at +4 dB 7 % on Good and Moderate and nothing on Poor, on AWGN from −6 to −2 dB
4–22 %; from +6 dB the link runs on OFDM rungs and nothing changes. The
counterfactual agrees with the rung within a few bit/s at every point.

Where it costs: 120 sessions a point from −6 to −2 dB on Moderate and Poor, by mean session
time (the medians at 30 sessions move by a few bit/s between runs):

| Mean session, s | beta.116 | ADR-0080 | this ADR |
|---|---|---|---|
| Moderate −6 dB | 155.1 | 147.5 | 145.4 |
| Moderate −4 dB | 129.4 | 123.1 | 123.0 |
| Moderate −2 dB | 123.4 | 122.3 | 113.1 |
| Poor −6 dB | 152.1 | 138.3 | 140.7 |
| Poor −4 dB | 123.9 | 119.4 | 114.6 |
| Poor −2 dB | 119.4 | 118.2 | 104.8 |

Against beta.116 nothing is slower. Against the receiver alone, Poor at −6 dB is 2 % slower: the
rate controller climbs by AWGN thresholds and their margin, rung 4 needs 7.3 dB more on Poor than
on AWGN where rung 3 needs 5.8, and a 1 kB session ends before the margin it learns from the
failures catches up. A per-class threshold would answer it, and is not built: the link does not
know the class of the path it is on.

**The harness** (`tools/session_matrix.py`, the 29 scenarios at 500 Hz, three draws each,
against beta.116's baseline in `bench/baselines/scenarios.csv`; `scenario_trend.py`). Every run
passes, none with a radiated overlap; nothing is slower than the baseline's range and one
scenario, `80m-asymmetric-500`, is faster beyond it. Air s is the median run's, the rates the median of the Tests:

| scenario | passed | air s (base) | message bit/s (base) | file bit/s (base) | |
|---|---|---|---|---|---|
| 15m-old-rig-offset-500 | 3/3 | 192 (192) | — (—) | — (—) |  |
| 20m-dx-500 | 3/3 | 478 (449) | — (—) | — (—) |  |
| 40m-drifting-rig-500 | 3/3 | 363 (431) | — (—) | — (—) |  |
| 80m-agc-carrier-inside-500 | 3/3 | 352 (—) | — (—) | — (—) | new |
| 80m-agc-carrier-outside-500 | 3/3 | 329 (—) | — (—) | — (—) | new |
| 80m-agc-slow-carrier-outside-500 | 3/3 | 339 (—) | — (—) | — (—) | new |
| 80m-asymmetric-500 | 3/3 | 298 (261) | 80 (59) | 109 (87) | faster |
| 80m-called-again-cwid-500 | 3/3 | 175 (171) | — (—) | — (—) |  |
| 80m-disconnect-slow-drain-500 | 3/3 | 352 (342) | — (—) | — (—) |  |
| 80m-floor-top-start-500 | 3/3 | 306 (304) | 82 (63) | 104 (84) | faster |
| 80m-flutter-500 | 3/3 | 397 (385) | 58 (65) | 70 (66) |  |
| 80m-leave-together-500 | 3/3 | 303 (327) | — (—) | — (—) |  |
| 80m-local-rfi-ladder-500 | 3/3 | 337 (320) | 86 (78) | 83 (72) |  |
| 80m-lopsided-acks-500 | 3/3 | 309 (322) | 70 (70) | 96 (82) |  |
| 80m-nvis-disturbed-500 | 3/3 | 284 (287) | — (—) | — (—) |  |
| 80m-nvis-evening-500 | 3/3 | 348 (346) | 29 (30) | 39 (32) |  |
| 80m-offset-500 | 3/3 | 196 (203) | — (—) | — (—) |  |
| 80m-poor-500-pactor-signalink | 3/3 | 274 (341) | — (—) | — (—) |  |
| 80m-qsb-dropout-500 | 3/3 | 559 (520) | — (—) | — (—) |  |
| 80m-slow-machine-500 | 3/3 | 330 (323) | 106 (106) | 94 (77) |  |
| 80m-strong-moderate-500 | 3/3 | 275 (263) | 221 (180) | 215 (234) |  |
| 80m-trimode-gateway-client-vanishes-500 | 3/3 | 840 (791) | — (—) | — (—) |  |
| 80m-vara-on-frequency-500 | 3/3 | 139 (139) | — (—) | — (—) |  |
| 80m-varac-chat-and-ping-500 | 3/3 | 90 (91) | — (—) | — (—) |  |
| 80m-vox-caller-declared-500 | 3/3 | 334 (323) | — (—) | — (—) |  |
| 80m-vox-caller-learned-gap-500 | 3/3 | 369 (393) | — (—) | — (—) |  |
| 80m-weak-500 | 3/3 | 156 (182) | — (—) | — (—) |  |
| 80m-weak-both-ways-ladder-500 | 3/3 | 322 (346) | 51 (45) | 70 (60) |  |
| 80m-winlink-exchange-500 | 3/3 | 286 (325) | — (—) | — (—) |  |

The Test scenarios on the weak and lopsided paths carry their message and file 13–36 % faster
(`80m-asymmetric-500` 59 → 80 and 87 → 109 bit/s, `80m-floor-top-start-500` 63 → 82 and 84 → 104,
`80m-weak-both-ways-ladder-500` 45 → 51 and 60 → 70). Six draws of the two the first build had
slowed, on beta.116's daemon and on this one (means):

| Six draws | beta.116 message / file | this ADR message / file |
|---|---|---|
| `80m-strong-moderate-500` | 198 / 226 bit/s | 212 / 230 |
| `80m-flutter-500` | 72 / 73 | 76 / 73 |

The two scenarios in which a 2 300 Hz station calls a 500 Hz one (`40m-wide-calls-narrow`,
`-unprobed`) pass three of three as before, and so does the quick set. The baseline's 500 Hz rows
are these runs (`v0.2.0-beta.117`).

**What the first build of ADR-0080's receiver did here** is why ADR-0080 has a §2.4: the same
run had `80m-flutter-500` a tenth slower (message 64, file 66 bit/s over six draws), because on
10 Hz flutter the link climbed onto this rung and the receiver lost most of its frames there.

## 4. Alternatives

* **97 bytes** (rate 0.91, 140 bit/s): with ADR-0080's receiver it reached −0.7 dB on Moderate
  and on Poor (genie timing, 60 frames a point), short of the target on Poor, with one or two
  frames in sixty lost on Poor up to +6 dB. A larger one was not measured on the fading classes.
* **A longer OFDM frame** (the LONG frame's data stretched to 5.3 s): ADR-0080 §4 — the
  preamble, not the code, decides it on the fading classes.
* **Renumbering the ladder** (the new kind at rung 4, the OFDM modes one up): a configuration
  migration and a sixteenth rung for a rung nobody used.

## 5. Found on the way: why `80m-flutter-500` carries its message slowly

Step 3 of ADR-0069 left `80m-flutter-500`'s message at ×0.77 of protocol 7's. It is the SNR the
500 Hz OFDM receiver reports on 10 Hz flutter, not the link's rules: through the modem
(`decode_buffer`, 20 frames a point, the reading's median and spread):

| ITU flutter | +8 dB | +12 dB |
|---|---|---|
| Control frame, QPSK ½ SHORT | 16/20 decoded, reads −3.6 dB (σ 2.6) | 17/20, −3.1 (2.7) |
| Data, QPSK ½ (rung 5) | 20/20, −3.6 (1.4) | 20/20, −3.2 (1.5) |
| Data, 16-QAM ½ (rung 9) | 4/20, −3.4 (1.4) | 8/20, −3.4 (1.4) |

On Moderate and on AWGN the same frames read within 1–2 dB of the channel. On flutter the
channel moves within a frame faster than the pilots follow, and what the receiver reports is the
equalised frame's own SNR — about −3.4 dB whatever the channel's. As a figure of what the frame
can carry it is honest: rung 9 decodes a fifth to two fifths of its frames. But it holds the rate
controller under rung 5, which decodes every frame, and the control-frame family on the floor
(the reading never clears the ordinary control frame's threshold by its 3 dB margin, and the
ordinary control frames lose 15–20 % there anyway). So the link runs on the tone floor at
+8 dB: at rung 3 before this ADR, at rung 4 with it. The open item is a reading for fast fading
— what rung a frame that decoded would have carried, beside what its equaliser measured — not
the family rule.
