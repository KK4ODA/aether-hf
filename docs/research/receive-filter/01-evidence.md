# The receive-filter estimate: the evidence before Gate 0

*2026-11-08. What a mirror of today's code says about the recordings in hand, written before the
instruments of `00-brief.md` §5 exist. The mirror stands in for the daemon until
`aetherd --passband-trace` replaces it; its numbers are evidence of the mechanism, and Gate 0
will repeat them on the real code.*

## 1. Method

**The mirror** is a Python copy of `passband_width_hz` and `PassbandMonitor` as they stand at
`11a21ea`: a Hann window of 4096 samples, the bins to 4 kHz in dBFS, each bin's 10th percentile
(by Rust's rounding) over the last 120 looks, the reference the median within ±180 Hz of
1500 Hz, the floor the profile's 5th percentile, the threshold 6 dB under the reference or
halfway to the floor, whichever is lower, an edge three bins under it that stays under for
150 Hz.

**The looks** follow `sample_passband` as near as a recording allows: a look every 0.5 s once the
channel has been quiet for a window (85 ms), kept if the quiet holds 0.625 s more. Quiet means
the sidecar's busy readings (4 a second) say not busy, the station is not keyed and not within
0.75 s of a release, no frame it heard is on the air (0.3 s either side of the frame), the level
is over −70 dBFS, and the capture is past its first second. The live daemon judges quiet block
by block, so its looks are not exactly these. The live monitor also carries its 120 looks from
one session into the next, where the mirror starts each recording afresh.

**The mean** is the same looks' power averaged per bin, with the same edge rule. It is not
offered as the answer: an average of the looks is what read a band of FT8 as 850 Hz (fault 2).
It is a second reading where no FT8 is present.

**The swing** is how much the in-band level of the looks (1300–1700 Hz) varies, from its 10th
to its 90th percentile.

The recordings are the 19 in `field/inbox/` on 2026-11-08: nine from the gateway's FTDX10
(`KK4ODA-1`, and `KK4ODA` for the P2P test) and ten from the mobile's TS-480 (`KK4ODA`,
`WD5EMA`, `KK4ODA-2`). The IF SHIFT on the TS-480 was turned fully left from some time after
2026-11-07 15:06Z until the author centred it on 2026-11-08. Recording names are UTC; the first
callsign is the recording station.

## 2. The recordings

| Recording | Radio | Air | Looks | Swing | 10th pct edges (threshold under the middle) | Mean edges |
|---|---|---|---|---|---|---|
| 20261106-223113_KK4ODA-1_KK4ODA | FTDX10 | 500 | 73 | 11.3 dB | 0–3164 (−26 dB) | 0–3188 |
| 20261106-224135_KK4ODA-1_KK4ODA | FTDX10 | 500 | 120 | 3.3 | 0–3129 (−28) | 0–3141 |
| 20261107-135324_KK4ODA-1_KK4ODA | FTDX10 | 500 | 76 | 3.7 | 23–3129 (−25) | 12–3129 |
| 20261107-150555_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 90 | 3.6 | 12–3129 (−26) | 12–3117 |
| 20261108-005452_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 62 | 7.6 | 0–3117 (−25) | 0–3141 |
| 20261108-010116_KK4ODA-1_KK4ODA | FTDX10 | 500 | 66 | 4.6 | 0–3117 (−25) | 0–3164 |
| 20261108-010443_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 52 | 10.4 | 12–3105 (−25) | 0–3105 |
| 20261108-011000_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 44 | 3.5 | 0–3105 (−25) | 0–3176 |
| 20261108-012424_KK4ODA_WD5EMA | FTDX10 | 500 | 46 | 11.6 | 0–3129 (−26) | 0–3141 |
| 20261106-223113_KK4ODA_KK4ODA-1 | TS-480 | 500 | 82 | **34.5** | **0–1746** (−8) | 0–2801 |
| 20261106-224142_KK4ODA_KK4ODA-1 | TS-480 | 500 | 167 | 5.6 | 0–2812 (−18) | 0–2789 |
| 20261107-135330_KK4ODA_KK4ODA-1 | TS-480 | 500 | 87 | **33.0** | **0–2449** (−7) | 0–2906 |
| 20261107-150601_KK4ODA_KK4ODA-1 | TS-480 | 2300 | 136 | 5.7 | 0–2730 (−14) | 0–2742 |
| 20261108-005459_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 73 | **29.5** | **0–2133** (−6) | 0–1922 |
| 20261108-010123_KK4ODA_KK4ODA-1 | TS-480, shift left | 500 | 51 | **21.4** | **0–2062** (−9) | 0–1957 |
| 20261108-010450_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 45 | **25.5** | **0–2297** (−8) | 0–1957 |
| 20261108-011008_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 45 | **23.7** | **0–2191** (−7) | 0–1922 |
| 20261108-012431_WD5EMA_KK4ODA | TS-480, shift left | 500 | 35 | **25.7** | **0–2262** (−7) | 0–1934 |
| 20261108-163057_KK4ODA-2_listen | TS-480, centred | 2300 | 186 | 4.4 | 0–3000 (−16) | 0–3023 |

Where the swing is under 12 dB (twelve recordings, both radios), the two readings agree within
75 Hz. Where it is over 20 dB (seven, all the mobile's), they part by 105–1 055 Hz, and the 10th
percentile errs both ways: narrower than the mean where the filter was fine, wider where the
shift cut it.

**The live readings** (`metrics.rx_passband_hz` in each upload's `diagnostics.json`):

| Upload | Station | Air | Reading |
|---|---|---|---|
| 2026-11-06 22:33 … 11-08 01:16 (nine) | gateway, FTDX10 | 500 or 2300 | 3094–3152 Hz |
| 2026-11-06 22:43 | mobile | 500 | 2789 |
| 2026-11-07 14:25, 15:11 | mobile | 2300 | 2754, 2730 |
| 2026-11-08 01:00, 01:04, 01:08, 01:26 | mobile, shift left | 2300 | 1980, 1957, 1957, 1969 |
| 2026-11-08 01:26 | home station as `KK4ODA` (P2P) | 500 | 3141 |
| 2026-11-08 16:32 | mobile, centred | 2300 | 3012 |

The panel warns below the signal's occupied width less 400 Hz: 1880 Hz at 2300 Hz, 80 Hz under
what the mobile read with its passband cut.

## 3. Saturday's profile, both ways

`20261108-010123_KK4ODA_KK4ODA-1`, the mobile with the shift left, each 100 Hz band from 0 Hz
relative to 1450–1550 Hz:

| Hz | 0 | 300 | 600 | 900 | 1200 | 1500 | 1700 | 1900 | 2100 | 2400 | 2700 | 3000 | 3500 | 3900 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mean of the looks | +7 | +7 | +5 | +2 | +1 | −1 | −6 | −16 | −20 | −23 | −24 | −25 | −28 | −29 |
| 10th percentile | +21 | +21 | +15 | +11 | +9 | −1 | −2 | −6 | −9 | −11 | −12 | −13 | −15 | −16 |

The mean shows the cut: a gentle slope to 1.4 kHz, 20 dB down by 2 kHz, a floor 23–29 dB down
beyond. The
10th percentile shows a slope with no step. Its looks are those where the in-band noise was at
its lowest, the radio's gain was down and the audio chain's own noise (which does not swing)
stood higher against it: the step is compressed, the bottom of the band lifted, and the edge
drawn where the slope crosses −6 to −9 dB, at 2.06–2.30 kHz.

## 4. The mobile today, centred

`20261108-163057_KK4ODA-2_listen` (97 s, no session), the mean of the quiet audio in 100 Hz bands
relative to 1500 Hz:

| Hz | 200–2400 | 2500 | 2600 | 2700 | 2800 | 2900 | 3000 | 3100 |
|---|---|---|---|---|---|---|---|---|
| dB | within ±2 (but the carrier's band, below) | −2 | −3 | −3 | −9 | −15 | −22 | −27 |

A plain SSB filter of about 2.7 kHz, which carries the 2300 Hz signal (360–2640 Hz) whole. The
level is −26 dBFS RMS with peaks at −11 dBFS, nothing clipped. A weak carrier sits at 1153 Hz,
10 dB over the noise in 1 Hz bins. The estimate's mirror reads 0–3000 Hz, the live daemon 3012.

## 5. What this says

1. Today's estimate measures the right thing (the noise's shape) by a statistic that holds only
   while the noise level is steady. A radio behind a pumping AGC, overdriven audio or a fading
   band break it.
2. Even read right, Saturday's passband would have been judged by its width, about 1.5 kHz, and
   the warning's words would have told the operator to widen a filter that was not narrow.
3. The rule for the threshold is a different test on each radio.
4. None of this needed an exotic case: one knob on one radio, and a sound card driven hot.
