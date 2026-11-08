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

## 6. The corpus (Gate 0 item 3)

`tools/rxfilter_corpus.py make DIR` builds 130 clips of a receiver with nobody on the channel:
the band's noise (−30 dBFS in 3 kHz) through one of thirteen receive responses, over the audio
chain's own white floor 50 dB under the noise (the FTDX10) or 15 dB under it (the mobile on
Saturday), under one of five conditions: steady; an AGC dipping for a strong signal the audio does
not carry (the busy corpus's *mid* preset, a carrier keyed 1.5 s in every 5); QSB (±6 dB over 8 s);
ten FT8 stations at +3 to +15 dB; and audio driven to −8 dBFS RMS, clipping as Saturday's did.
The responses are eighth-order band-passes: 2.7, 2.4, 2.1 and 1.8 kHz filters, a 500 Hz filter,
the 2.4 kHz filter moved down 300, 600 and 1100 Hz (Saturday's), moved up 300 and 600 Hz, a low
cut at 600 Hz, a 3 dB/kHz tilt above 1 kHz, and a 30 dB notch 60 Hz wide at 1800 Hz. Each clip's
labels give the response's −6 dB points and how far down it is at the edges of the 2300 Hz signal
(360 and 2640 Hz) and the 500 Hz one (1260 and 1740 Hz). Clips run 30 s, the FT8 ones 150 s and
the AGC ones 120 s: the busy detector calls those bands busy 86 % and 75 % of the time, and the
estimate looks only in the quiet. `judge DIR` runs `aetherd --passband-trace` over each and sets
its last reading beside the truth.

**Today's warning at 2300 Hz** (the width under 1880 Hz), against the deeper of the two losses at
the signal's edges:

| True loss at an edge | steady | QSB | hot | FT8 | AGC | all |
|---|---|---|---|---|---|---|
| 20 dB or more | 3/12 | 2/12 | 2/12 | 2/12 | 1/12 | **10/60** |
| 6–20 dB | 0/4 | 0/4 | 0/4 | 0/4 | 0/4 | **0/20** |
| 3–6 dB | 0/2 | 0/2 | 0/2 | 0/2 | 0/2 | 0/10 |
| under 3 dB | 1/8 | 0/8 | 1/8 | 0/8 | 0/8 | **2/40** |

* It warns for the 500 Hz filter (seven clips of ten) and for Saturday's shift once in ten. It
  never warns for a low edge cut: the 600 Hz low cut, the 1.8 kHz filter and both upward shifts
  lose 35–64 dB at 360 Hz and read 2.9–3.6 kHz wide.
* Its two false warnings are the notch over the shallow floor (steady and hot): the notch, 30 dB
  deep, goes under a threshold drawn 7.5 dB down and is taken for the edge.
* Where it reads, its edges lie outside the true −6 dB points. The high edge reads a median
  589 Hz high over the deep floor (141 to 934) and 196 Hz over the shallow one: the halfway rule
  draws the edge 25 dB down the skirt on the first, 7.5 dB on the second.
* Under the AGC over the shallow floor it reads 0–3996 Hz whatever the response: its looks are
  the AGC's dips, where the band's noise sits under the chain's floor and no filter shows.

## 7. The count (Gate 0 item 5)

About 200 lines of code: `passband_edges`, `PassbandMonitor` and their constants in
`spectrum.rs` (about 160), `Station::sample_passband` (40), `applyPassband` in the panel (20).
Twelve constants: eight in `spectrum.rs` (`PASSBAND_DROP_DB`, `EDGE_RUN`, `FLOOR_QUANTILE`,
`EDGE_HOLD_HZ`, `REFERENCE_HALF_HZ`, `MIN_OBSERVATIONS`, `HISTORY`, `NOISE_QUANTILE`), three in
`station.rs` (`AUDIO_CENTER_HZ`, `PASSBAND_SAMPLE_S`, `PASSBAND_MIN_LEVEL_DBFS`) and the panel's
400 Hz. Eight tests: five in `spectrum.rs` (the width, flat noise, the notch, the FT8 band, the
wait for looks) and three in `station.rs` and `passband_trace.rs` that hold the looks away from
frames and keying. None tests a passband that is moved, or a low edge.

## 8. What a cut costs (Gate 0 item 6, the author's answer 2)

`tools/rxfilter_cost.py OUT` writes one Test scenario per configuration and runs three draws of
each through the harness, paired by seed. The called station receives the Test's message and file;
its radio is cut (`rx_filter_hz`, new in `tools/channel_server.py`: an eighth-order band-pass
before its AGC, placed so the response is the stated depth down at the signal's edge), over its
audio chain's floor (`rx_floor_db`, 15 dB under the band's noise as the mobile's, or 50 as the
FTDX10's), or driven hot (`rx_gain_db`, clipping at full scale). The paths are strong Good ones,
15 dB at 2300 Hz and 12 dB at 500 Hz, so the Test's ladder climbs to where the outer carriers
count. Every run passes but two (both the 20 dB low cut at 500 Hz) and none radiated over the
other. The ratios are each run's Test rate against the uncut run on the same draw, median of three:

| Cut | 2300 Hz message | file | 500 Hz message | file |
|---|---|---|---|---|
| none | 1.00 | 1.00 | 1.00 | 1.00 |
| top 3 dB | 1.00 | 1.00 | 1.00 | 1.00 |
| top 6 dB | 1.00 | 0.86 | 1.00 | 1.00 |
| top 10 dB | 1.00 | 0.87 | 0.96 | 0.99 |
| top 20 dB | 0.86 | 0.98 | 0.87 | 0.86 |
| top 20 dB, deep floor | 0.86 | 0.93 | 0.90 | 1.01 |
| bottom 3 dB | 0.86 | 0.92 | 1.00 | 1.00 |
| bottom 6 dB | 0.86 | 0.98 | 0.96 | 0.99 |
| bottom 10 dB | 0.86 | 1.00 | 0.94 | 0.88 |
| bottom 20 dB | 0.67 | 0.82 | 0.30 | 0.18 (one Test lost) |
| bottom 20 dB, deep floor | 0.67 | 0.81 | 0.30 | 0.32 (one Test lost) |
| **Saturday's receiver** | **0.57** | **0.63** | **1.00** | **1.00** |

*Saturday's receiver* is the mobile's on 2026-11-08: a passband ending near 1.6 kHz (an
eighth-order low-pass at 1600 Hz), the shallow floor and the codec 8 dB hot. On the harness it
costs the 2300 Hz Test 37–43 % and the 500 Hz one nothing, as the operator found on the air
(the 2300 Hz sessions paused, the 500 Hz ones ran smoothly).

How to read the rest:

* **The measure's own step is about 14 %.** The Test's message is small and its rate takes a few
  values (1358, 1247, 1133 bit/s at 2300 Hz); one rung more or less on one draw moves the median a
  step. A ratio of 0.86 alone is a step, not a cost. The cut changes each session's course, so
  every configuration is another draw of the same three seeds.
* **Cuts of 3–10 dB at an edge cost nothing beyond that step**, at either bandwidth, either edge.
* **A 20 dB cut costs**: 2–33 % at 2300 Hz, more at the bottom edge than the top; at 500 Hz 0–14 %
  at the top and 68–82 % at the bottom, with one Test in three lost. At 500 Hz an eighth-order
  filter 20 dB down at an edge has its corner past the centre (1305 or 1680 Hz), so it takes much
  of the channel, not only its edge.
* **The chain's floor hardly changes it**: the 20 dB cuts cost the same over the deep floor as
  over the shallow one. The expectation that a cut costs only where it pushes the signal under the
  chain's floor (the band's noise is cut with the signal) does not hold: something in the receiver
  or the link pays for the cut itself. Which, and why the bottom edge costs more than the top, is
  open (Gate 1).
* **A warning line** follows: somewhere between 10 and 20 dB down at either edge of the signal.
  The FTDX10's own tilt is 5–9 dB down at 2.6–2.8 kHz, and a line at 10 dB leaves it alone.
  Where exactly is Gate 2's to register.

## 9. Clipping (Gate 0 item 7, the author's answer 5)

**In the field.** The mobile's five Saturday sessions clipped in 79–98 % of their seconds, 1.3–1.7 %
of their samples at full scale, at −7 to −8 dBFS RMS (−4 to −5 dBFS in the loudest second). Its
four sessions of 2026-11-06 and 11-07 sat at −24 to −29 dBFS and never touched full scale, as did
its quiet minute on 2026-11-08 afternoon (−26 dBFS) and every one of the gateway's nine
recordings (−26 to −35 dBFS). So the receive level rose about 20 dB between 2026-11-07 15:06Z and
2026-11-08 00:54Z, about when the IF shift was turned, and came back down when it was centred.

**What the panel said.** The Status tab's *Receive level* tile says "clipping: turn the receive
level down" once more than 0.1 % of samples clip (`applyRxLevel`). Saturday's 1.3–1.7 % passed
that line in every session. The tile is in the Channel card, which **Compact hides**
(`body.compact #channel-card`), and the operator ran the mobile in Compact.

**What it costs** (the same harness, the codec driven `rx_gain_db` over the scenario's own level,
the signal at −15 dBFS RMS before it):

| Drive | RMS | 2300 Hz message | file | 500 Hz message | file |
|---|---|---|---|---|---|
| +6 dB | about −9 dBFS | 1.00 | 0.93 | 1.00 | 1.00 |
| +10 dB | about −5 dBFS | 1.00 | 0.86 | 1.00 | 0.94 |
| +14 dB | about −1 dBFS | 0.52 | 0.70 | 0.83 | 0.94 |

Saturday's −7 to −8 dBFS lies between the first two rows: within the measure's step on its own,
and a share of the 37–43 % that *Saturday's receiver* cost with its cut. Clipping costs plainly
from about −1 dBFS RMS. The warning that exists fires at 0.1 % of samples, well before that.

## 10. Gate 0, in short

1. **The estimate measures the right thing and draws the wrong conclusions from it.** It sees a
   cut in the noise; its edge rule ("halfway to the floor") puts the edge 7–25 dB down the skirt,
   so it reads every passband wide, and its test is the width, so a moved passband or a cut low
   edge never warns. Over the corpus it warns for 10 of 60 cuts of 20 dB or more and none of 20
   smaller ones, and falsely for a notch over a shallow floor.
2. **What matters is the signal's span**: 10 dB or less down at either edge costs nothing
   measurable on the harness; 20 dB costs, the bottom edge more than the top; Saturday's
   receiver cost the 2300 Hz Test 37–43 %.
3. **Its looks fail under an AGC over a shallow floor** (the band's noise dips under the chain's,
   and the quietest looks show no filter), and take a minute or two to gather on a busy band.
4. **Clipping has a warning, in the wrong place**: hidden by Compact.
5. **Open for Gate 1**: why a cut costs over a deep floor at all, and why the bottom edge costs
   more than the top; and what the decoded frames' channel estimate would have shown.

