# The receive-filter estimate: Gate 0's first evidence

*2026-11-08. The real code over the recordings in hand, with `aetherd --passband-trace` (Gate 0
item 1). A first version of this note rested on a Python mirror whose quiet looks were chosen
from the sidecar's busy readings; it let in parts of the other station's frames, found the
in-band level swinging 21–35 dB between looks, and blamed the 10th percentile. The real look
selection does not bear that out (§2), and that explanation is withdrawn. The mirror's edge rule
is kept as a check: on the real looks it draws the same edges as the daemon in all 19
recordings.*

## 1. Method

`aetherd --passband-trace WAV --expect SIDECAR` runs a recording through the daemon's own
`SpectrumAnalyser` and `PassbandMonitor`, with the looks taken by `Station::sample_passband`'s
rule as near as a sidecar allows: the busy detector runs over the same audio (deaf where the
transmitter was keyed and 0.75 s after, starting from the live detector's state where the
sidecar kept it), the station counts itself receiving from a heeded announcement for the frame's
length (at least the air's longest frame) and for half a second after a frame that decoded or was
trusted, and a look is taken every 0.5 s once the quiet has lasted a window (85 ms) and folded in
once it has held for the detector's attack time more. Each look folded in is a CSV line: its own
level around 1500 Hz and the edges after it, with the reference, threshold and floor they were
drawn from. The trace starts each recording afresh; the live monitor carries its last 120 looks
from one session into the next.

The recordings are the 19 in `field/inbox/` on 2026-11-08: nine from the gateway's FTDX10
(`KK4ODA-1`, and `KK4ODA` for the P2P test) and ten from the mobile's TS-480 (`KK4ODA`, `WD5EMA`,
`KK4ODA-2`). The TS-480's IF SHIFT was turned fully left on 2026-11-08 until the author centred it
that afternoon. Recording names are UTC; the first callsign is the recording station.

## 2. The recordings

*Swing* is how much the looks' in-band level (1300–1700 Hz) varies, 10th to 90th percentile.
*Mean* is the same looks' power averaged per bin, edged by the same rule.

| Recording | Radio | Air | Looks | Swing | Trace's edges | Mean's edges |
|---|---|---|---|---|---|---|
| 20261106-223113_KK4ODA-1_KK4ODA | FTDX10 | 500 | 50 | 9.9 dB | 0–3164 | 0–3188 |
| 20261106-224135_KK4ODA-1_KK4ODA | FTDX10 | 500 | 120 | 3.8 | 0–3141 | 0–3141 |
| 20261107-135324_KK4ODA-1_KK4ODA | FTDX10 | 500 | 75 | 4.8 | 23–3141 | 12–3129 |
| 20261107-150555_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 97 | 3.7 | 12–3129 | 12–3117 |
| 20261108-005452_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 28 | 3.8 | 0–3117 | 0–3141 |
| 20261108-010116_KK4ODA-1_KK4ODA | FTDX10 | 500 | 77 | 7.2 | 0–3117 | 0–3152 |
| 20261108-010443_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 34 | 5.3 | 0–3117 | 0–3117 |
| 20261108-011000_KK4ODA-1_KK4ODA | FTDX10 | 2300 | 51 | 4.4 | 0–3129 | 0–3211 |
| 20261108-012424_KK4ODA_WD5EMA | FTDX10 | 500 | 16 | 9.0 | 0–3141 | 0–3141 |
| 20261106-223113_KK4ODA_KK4ODA-1 | TS-480 | 500 | 72 | 5.1 | 0–2812 | 0–2789 |
| 20261106-224142_KK4ODA_KK4ODA-1 | TS-480 | 500 | 120 | 4.1 | 0–2789 | 0–2789 |
| 20261107-135330_KK4ODA_KK4ODA-1 | TS-480 | 500 | 68 | 3.0 | 0–2836 | 0–2859 |
| 20261107-150601_KK4ODA_KK4ODA-1 | TS-480 | 2300 | 120 | 3.7 | 0–2742 | 0–2742 |
| 20261108-005459_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 45 | 32.0 | **0–1980** | 0–1934 |
| 20261108-010123_KK4ODA_KK4ODA-1 | TS-480, shift left | 500 | 62 | 10.4 | **0–2062** | 0–1934 |
| 20261108-010450_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 33 | 3.0 | **0–1992** | 0–1934 |
| 20261108-011008_KK4ODA_KK4ODA-1 | TS-480, shift left | 2300 | 39 | 3.7 | **0–1934** | 0–1922 |
| 20261108-012431_WD5EMA_KK4ODA | TS-480, shift left | 500 | 40 | 27.6 | **0–2273** | 0–1980 |
| 20261108-163057_KK4ODA-2_listen | TS-480, centred | 2300 | 120 | 4.2 | 0–3035 | 0–3023 |

The swing is 3–10 dB in 17 recordings of 19, and there the 10th percentile and the mean agree
within 85 Hz. Two of the mobile's Saturday sessions swing 28–32 dB; the 10th percentile reads
46 and 293 Hz wider than the mean there.

**The live readings** (`metrics.rx_passband_hz` in each upload's `diagnostics.json`):

| Upload | Station | Air | Reading |
|---|---|---|---|
| 2026-11-06 22:33 … 11-08 01:16 (nine) | gateway, FTDX10 | 500 or 2300 | 3094–3152 Hz |
| 2026-11-06 22:43 | mobile | 500 | 2789 |
| 2026-11-07 14:25, 15:11 | mobile | 2300 | 2754, 2730 |
| 2026-11-08 01:00, 01:04, 01:08, 01:26 | mobile, shift left | 2300 | 1980, 1957, 1957, 1969 |
| 2026-11-08 01:26 | home station as `KK4ODA` (P2P) | 500 | 3141 |
| 2026-11-08 16:32 | mobile, centred | 2300 | 3012 |

The trace of the 00:54Z session ends at 1980 Hz, the live reading uploaded at 01:00. The panel
warns below the signal's occupied width less 400 Hz: 1880 Hz at 2300 Hz.

## 3. What the estimate saw on Saturday, and where it drew the edge

The profile of the 01:04Z session (`010450`, swing 3.0 dB), each 200 Hz band relative to the
reference at 1500 Hz:

| Hz | 0 | 200 | 400 | 800 | 1200 | 1400 | 1600 | 1800 | 2000 | 2400 | 2800 | 3200 | 3800 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| dB | +4 | +7 | +4 | +2 | +1 | 0 | −5 | −16 | −23 | −24 | −27 | −28 | −31 |

The cut is plain: flat to 1.4 kHz, 16 dB down at 1.8 kHz, 23 dB down from 2 kHz. The floor the
rule found was 30.8 dB under the reference, so "6 dB under the reference or halfway to the floor,
whichever is lower" put the threshold 15.4 dB down, and the edge where the skirt crosses it, at
1992 Hz. At 6 dB down the edge falls at about 1.65 kHz, and the panel would have warned.

The halfway rule came with the second fix (`7ac47c6`, the FT8 band). It makes the edge's depth
depend on how far down the audio chain's own floor lies: 6.5–16 dB on the mobile, 25–28 dB on
the FTDX10. On clean band noise through an eighth-order filter (the trace's test signal, its own
floor 50 dB down) it reads a 300–2700 Hz passband as 211–3176 Hz, and a passband ending at
1600 Hz as ending at 2473 Hz.

## 4. The other recordings

* **The mobile on 2026-11-07 15:06Z** (`150601`, before the shift was turned): flat to 2.2 kHz,
  6 dB down at 2.4 kHz, 16 dB at 2.6, 25 dB at 2.8. The 2300 Hz signal reaches 2640 Hz, so its top
  200 Hz was 6–16 dB down that afternoon too. The estimate read 2742 Hz. Whether the knob was part
  way round then is not known.
* **The mobile today, centred** (`163057`): flat within 3 dB to 2.6 kHz, 11 dB down at 2.8 kHz,
  24 at 3.0. The whole signal passes. The trace reads 3035 Hz, the live daemon 3012.
* **The gateway's FTDX10** (`011000`): a gentle tilt (3 dB down at 1.8–2.0 kHz, 7 dB at 2.6,
  9 dB at 2.8), then a sharp cut at 2.9–3.0 kHz. The estimate reads 3129 Hz. Tilt like this is
  what a span test must not mistake for a cut.
* **The two swinging sessions** (`005459`, `012431`): the profile slopes 26–29 dB from 200 Hz to
  2 kHz and its floor is only 12–13 dB under the reference. Something in those looks is not the
  receiver's steady noise. Clipping alone does not explain it: all five of the mobile's Saturday
  sessions clip alike (1.3–1.7 % of their samples at full scale, −7 to −8 dBFS RMS over the
  recording). The corpus (Gate 0 item 3) is to find out what.

## 5. What this says

1. **The rule for the edge's depth is the main cause.** On Saturday's clean looks the cut was
   23 dB deep and the estimate saw it, then drew the edge 15 dB down instead of 6, at about 2 kHz.
2. **The width test is the wrong test.** Even read at 6 dB down, Saturday's passband would have
   warned only as "too narrow", and the words would have told the operator to widen a filter that
   was not narrow. The question is whether 360–2640 Hz is carried, and on Saturday 2640 Hz was
   26 dB down.
3. **The reference at 1500 Hz** sat 1–7 dB under the band below it on Saturday: a small
   contribution here, a larger one for a passband shifted further.
4. **The level swing is not the cause.** Withdrawn from the first version of this note. Two
   sessions with 28–32 dB of swing remain unexplained and go into the corpus.
5. **Clipping** (§6.5 of the brief): every one of the mobile's Saturday sessions clipped, 1.3–1.7 %
   of its samples at full scale. Item 7 of Gate 0 measures what that costs and what the estimate
   makes of it.
