# ADR-0104: The receivers read the noise across the band

**Status:** accepted, 2026-11-09. The work is the author's decision at the receive-filter
review's Gate 1 (`docs/research/receive-filter/02-survey.md` §7: "the receiver change goes first,
as separate work with its own ADR"); shipping both halves, the OFDM one and the tone one, with the
small cost on 500 Hz NVIS that §3 measures, is the author's choice of 2026-11-09. The receivers
only, model first, then the port: **no wire change, no configuration key, no schema bump.**
Stations on this build and on beta.143 interoperate.

## 1. Context

On the evening of 2026-11-07 the mobile's TS-480 had its IF SHIFT turned fully left. Its receive
audio was flat to 1.4 kHz, 16 dB down at 1.8 kHz and 23 dB down from 2 kHz, where the 2300 Hz
signal spans 360–2640 Hz. The receive-filter review (`docs/research/receive-filter/`) set out to
make the passband estimate warn about such a cut. On the way it measured what a cut costs and why
(`02-survey.md` §4, `tools/bench_cut.py`). Each rung's frames went through a cut applied to signal
and noise alike, then the audio chain's floor 50 dB under the band's noise, at the rung's AWGN
threshold plus 3 dB:

* a cut at the very edge of the signal cost nothing;
* a cut across the band cost a great deal. A passband ending near 1.6 kHz ("Saturday's") took
  the 2300 Hz fast tone rungs (rung 3: 6 of 30; rung 5: none) and every OFDM rung from 13 up
  (none of 30). At 500 Hz, a cut 20 dB down at either edge of the signal took every tone rung.

The cause was the same in both receivers: **they took the noise as flat across their band.**

* The OFDM receiver reads one noise figure per symbol from all the pilot carriers, and charges
  each data carrier that noise over its own channel estimate, `σ²/|h|²`. A radio's filter cuts the
  band's noise with the signal. A carrier in the skirt has a small `|h|` and a small noise, so its
  SNR is the same as anywhere else. Charged the band's average noise, it read as a carrier 20 dB
  under the others, and the decoder all but discarded it. ADR-0043's per-carrier factor only
  raises a carrier's noise, so it could not give one back.
* The tone receiver reads one noise level for all its tones. A symbol sent on a tone the filter
  cut was weaker than the uncut tones' noise, and the metric took it for one of them.

## 2. Decision

Each receiver reads the noise where the signal is, and keeps the shape it reads only where it
stands clear of the readings' own scatter. One rule does that for both
(`aether_model/phy/noise.py`, `core/aether-phy/src/noise.rs`, `shaped_levels`):

* the raw readings, one per pilot carrier or per tone, in nepers (natural log);
* smoothed over three neighbours, ¼ ½ ¼, the ends repeated;
* pulled toward one level by the empirical-Bayes factor `keep = max(0, 1 − scatter / var(level))`,
  where `scatter` is the variance each reading's own estimate has. Where nothing shapes the band
  the readings differ by their scatter alone, `keep` is about 0, and the receiver is the flat
  one it was. Under a cut the readings differ by tens of decibels, `keep` is about 1, and the
  shape is the filter's.

**OFDM** (`FrameReceiver.noise_shape`, on). Each pilot carrier's residual power over the frame's
data symbols (the residuals the per-symbol figure is read from) is a reading. Its scatter is
`2/n` for `n` data symbols: each residual shares a third of its neighbours' noise through the
three-symbol time filter (1.7–1.9/n measured on AWGN). The shaped levels, about their own mean,
are interpolated across the carriers in nepers and scaled to mean 1 over the pilots. Each data
carrier's noise is the per-symbol figure times its shape, over `|h|²`. The per-symbol figure, the
motion scaling (ADR-0082), ADR-0043's weighing and the reported SNR are unchanged.

**Tones** (`tone.NOISE_SHAPE`, on), **at the sync numerology**: the floor's kinds' data and
every kind's sync blocks. Each tone's reading is its median energy over the symbols it should
hold only noise in — every symbol but those it is the sync tone of or, in a data symbol, the
strongest — over ln 2, as the one level was. "The strongest" is judged against each tone's own
noise, read twice: against one level a cut tone, sent, is seldom the strongest, and its own signal
would count as its noise. Its scatter is `1/(n·ln²2)` for a median of `n` exponential draws, and
the shape is pulled toward the one level (`noise_level`), so no shape means exactly the old
receiver. A tone with fewer than eight noise-only symbols leaves every tone on the one level. The
metrics read each tone's energy against its own noise, and so do the sync blocks' levels, which
weigh every kind's data. **The frame's SNR is read against the one level, as before.** Read tone
by tone, on ITU NVIS the floor's frames read 1–2 dB higher (the readings there follow the echo,
below), and the rate controller, believing them, tried rungs the echo kills: the harness's NVIS
sessions ran 5–6 % longer although the decodes were the same (§3).

**A fast or four-tone kind's data keeps one level.** Read tone by tone at the data's own
numerology, it lost on ITU NVIS (two rays 7 ms apart, 1 Hz), the 80 m path the 500 Hz sessions
mostly run on (§3): with four tones each sent a quarter of the time, or sixteen with 10 ms symbols
under a 7 ms echo, a tone's own faded and smeared symbols count as its noise, and the readings
followed each tone's gain (correlation +0.3 to +0.5, a 5 dB spread, against 0.7 dB on AWGN). The
detector, the contradiction test (ADR-0014/0083) and the echo share (ADR-0087) keep the one level
too.

**What it removes.** The assumption that the noise is flat across the band, from the OFDM decode
and from the tone metrics at the sync numerology. Nothing else: ADR-0043's per-carrier factor
stays, because an interferer between two pilots is caught by the slicing error and not by the
pilots; the shape also catches one that sits on a pilot. The bench switches stay in the model
(`noise_shape`, `NOISE_SHAPE`) so the change can be measured again; the port has no switch for
tones.

## 3. What was measured

**Under the cuts** (`tools/bench_cut.py`, 30 frames a point, the same frames each way, at the
rung's AWGN threshold + 3 dB; `--receiver noise_shape=false` / `tone.NOISE_SHAPE=false` is the
receiver before; `bench/baselines/noise_shape_cuts.csv`). Frames decoded, before → after:

| | uncut, 10 dB cuts, 20 dB at the 2300 Hz edges | Saturday's | 500 Hz, 20 dB top | 500 Hz, 20 dB bottom |
|---|---|---|---|---|
| 2300 Hz rung 1, tone-36 | 30 → 30 | 30 → 30 | | |
| 2300 Hz rung 3, tone50-75 | 30 → 30 | 6 → 6 | | |
| 2300 Hz rung 5, tone100-153 | 30 → 30 | 0 → 0 | | |
| 2300 Hz rung 10, QPSK ½ | 30 → 30 | 27 → 30 | | |
| 2300 Hz rungs 13, 16 | 30 → 30 | 0 → 30 | | |
| 2300 Hz rung 19, 64-QAM ⅚ | 30 → 30 | 0 → 25 | | |
| 500 Hz rung 1, tone-36 | 30 → 30 | 30 → 30 | 0 → 30 | 0 → 19 (20 found) |
| 500 Hz rung 2, tone4x100-51 | 30 → 30 | 30 → 30 | 0 → 26 | 22 → 30 |
| 500 Hz rung 3, tone4x100-75 | 30 → 30 | 30 → 30 | 0 → 18 | 1 → 29 |
| 500 Hz rung 4, tone4x100-93 | 30 → 30 | 30 → 30 | 4 → 28 | 1 → 30 |
| 500 Hz OFDM rungs | 29–30 → 29–30 | 30 → 30 | 30 → 30 | 29–30 → 29–30 |

Two losses are left. Rung 1 under the 500 Hz bottom 20 dB cut is the detector's: it found 20
frames of 30 before and after, and the decoder now reads 19 of them. The 2300 Hz fast kinds under
Saturday's cut are their data's one level: read tone by tone they decoded 30 and 30, but that
reading lost on NVIS (below), the rate controller has the OFDM rungs this ADR gives back on that
receiver, and a cut across the band is the review's to warn of.

**Where nothing cuts** (`tools/bench_noise_shape.py`, `bench/baselines/noise_shape_paths.csv`):
every (air, channel, mode or kind) at the SNRs nearest 30 % and 10 % frame errors on the committed
curves (`phy_fer*.csv`, `tone_floor.csv`), flutter and ITU NVIS at Poor's points, 30 frames a
point, the same frames each way. Frames decoded:

| | frames a column | flat | this ADR |
|---|---|---|---|
| OFDM, both airs, AWGN, Good, Moderate, Poor, flutter, NVIS | 2 940 (98 points) | 1 699 | 1 698 |
| tones, every kind, the same channels, through the detector | 3 150 (105 points) | 2 335 | 2 330 |
| the same tone frames at their true start and offset | 3 150 | 2 330 | 2 327 |

The differences are a frame or two a channel, both ways. Smoothed but not pulled toward flat, the
OFDM shape decoded 1 671 where flat decoded 1 682 (the same points without NVIS): on AWGN at
2300 Hz it lost 7 frames in 160, its own scatter, about 0.6 dB a carrier, taken for a shape.
Pulled, it reads exactly flat on most uncut frames and within half a decibel on the rest. Raw
per-pilot readings, not smoothed, decoded the cuts exactly as the smoothed ones did.

**On ITU NVIS**, which the committed curves do not hold. The harness found it (below): with the
data of the fast and four-tone kinds read tone by tone, the 500 Hz NVIS sessions ran 5 % longer
and one draw of `80m-leave-together-500` missed its message's budget. Frames decoded, 40 a point
at 4, 8 and 12 dB, the same frames each way:

| | flat | data read tone by tone | its echo left out of each tone's reading | data shaped from the sync bins | data on one level (this ADR) |
|---|---|---|---|---|---|
| tone4x100-51, through the detector | 81 | 74 | — | — | — |
| tone4x100-75, through the detector | 9 | 3 | — | — | — |
| tone4x100-51/-75, tone50-75, tone100-105, at the true start | 204 | 187 | — | 182 | 204 |
| tone-36, tone4x100-51/-75, tone50-75, tone100-105, through the detector | 405 | 387 | 372 | — | — |

The floor's kinds near their threshold on NVIS (tone-control, tone-24, tone-36 at −18 to −12 dB,
480 frames a column) decode 280 flat and 282 read tone by tone: at 40 ms a symbol the echo is a
sixth of it, and the readings stay noise.

**On the harness** (`tools/session_matrix.py`; each run paired by its draw with the daemon from
before this ADR, built from the same tree):

* **Every scenario, two draws** (122 runs): 122 → 121 passed, pooled air ×1.003 (median per run
  ×1.000), Test rates ×1.000 (38 Tests), no radiated overlap. The one failure,
  `80m-agc-carrier-outside-500` s1031, missed its 300 s message budget, a scenario at its budget's
  edge since ADR-0099; the same draw passed on a second pass.
* **The cuts** (`tools/rxfilter_cost.py`, 32 configurations, three draws): Test rates ×1.000
  (median of 94 paired runs). Saturday's cut at 2300 Hz: message 717/717/1192 → 832/752/1288
  bit/s, file 1247/1026/1236 → 1258/1262/1196. The harness's Saturday is the mobile's whole chain:
  its floor 15 dB under the band's noise and its drive 8 dB hot, and in the filter's skirt that
  floor, not the receiver, takes the SNR, so the session gains less than the bench's frames. The
  500 Hz bottom 20 dB cut: 4 of 6 → 6 of 6 passed.
* **500 Hz NVIS.** The first build (data of every kind read tone by tone, the SNR too) ran these
  sessions 5–6 % longer; that is how the two changes in §2 were found. On fresh draws of the seven
  NVIS scenarios the shipped build ran 28 paired runs ×1.016 (27 and 27 passed). The three at
  their budget's edge, 18 draws each, two passes of every build (36 runs a column):

| | before | OFDM half only | tone half only | both (this ADR) |
|---|---|---|---|---|
| air, both passes | 5 870, 5 713 s | 5 873, 5 803 s | 5 801, 5 978 s | 6 236, 5 931 s |
| passed | 35 | 35 | 36 | 33 |
| `80m-agc-carrier-inside-500`, mean | 312 s | 326 s | 342 s | 350 s |
| `80m-agc-carrier-outside-500`, mean | 326 s | 328 s | 325 s | 333 s |
| `80m-leave-together-500`, mean | 327 s | 320 s | 315 s | 331 s |

  One build repeats only 2–3 of the 18 runs to the second, and a failure moves between passes, so
  the cost is about 5 %, mostly in the scenario with the AGC, and it is not certain. Replaying the
  recordings of two failed runs through both daemons decodes the same frames, give or take one
  either way, with the same readings. What differs is the course: over 36 fresh NVIS draws the
  shipped receiver decoded rung 5 (the first OFDM rung) 40 times in 224 where the old one decoded
  35 in 182, and the rate controller, which chooses by measured delivery, tried that rung more
  (233 frames sent against 194), on a path whose 7 ms echo kills it. That is the rate
  controller's to answer (its review owes the OFDM rungs on NVIS); the author chose to ship both
  halves with it. The nightly's trend check may flag `80m-agc-carrier-outside-500` or
  `-inside-` on a bad night; the baseline is not refreshed for it.

## 4. Consequences

* The phy vectors carry OFDM frames and tone frames through a receive filter
  (`OFDM_FILTERED_CASES`, `tone_filtered`), so the port is held to the model with a shape read,
  not only flat. The tone
  receive vectors, whose stand-in noise is seven fixed tones, now read a shape too (their soft
  bits moved; their SNRs did not). An OFDM vector on two rays 1 ms apart reads a shape at high
  SNR: the transmitter's peak reduction (ADR-0004) puts a distortion in the frame that follows
  the channel's notches, so the noise there is not flat either, and the shaped figure is the
  truer one.
* The interferer tests of ADR-0043 compare the weighing against a receiver that reads the noise
  flat as well, as the receiver before either did: the shape alone catches the interferer on a
  pilot.
* The receive-filter review goes on to its Gate 2 on this receiver: what a cut still costs, now
  much less, decides where the warning's line goes.
* Not changed: the tone detector weighs a sync tone against the other tones in the symbol, so a
  cut that takes most of the 500 Hz floor's tones still loses frames before the decoder sees them.
