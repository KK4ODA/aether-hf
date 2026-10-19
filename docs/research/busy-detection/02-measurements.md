# Measurements

What was measured before designing, from what was in hand: the author's daemon logs from four
uploads, the session recordings in them, the code, a synthetic corpus, and the harness. The
author's purpose-made recordings (`00-recording-plan.md`) are the next section's source; until
they arrive, the AGC model below is the one beta.17 measured, and §6 says what they must settle.

## 1. The logs: how often, how long, when

`busy:` lines carry every transition with the level, the floor, the delta and the reason. Four of
KK4ODA-1's uploads (2026-10-12, 10-15 twice, 10-17; beta.104–111) hold **108 hours** of daemon
logs from 2026-10-08 to 10-17. The signature looked for: an idle busy span by level of 20 s or
more that ends with the floor at least 6 dB higher than it began.

| | |
|---|---|
| spans found | **57**, 0.53 an hour over the 108 h |
| time busy in them | 60 minutes, 0.9 % of the time logged |
| length | median **70 s**; 36 of the 57 between 69 and 73 s; longest 78 s |
| the floor at the start | 8–30 dB under the level; median 12 dB under where it had been in the two minutes before |
| the floor at the end | back at the level, −27 ± 2 dBFS on 80 m at night |
| by UTC hour | 00–04: 34; 08–11: 14; 21–23: 4; the rest 5 |

The 69–73 s cluster is the floor window (10 s) plus the hold (60 s) plus the hangover (0.75 s):
the span ends exactly when the hold expires and the floor takes the median of everything. The
night cluster is 80 m at night on 3.590 MHz, when the band is full of strong signals. The logs
cannot say where the signal that held the AGC down sat: a line is written only when the busy
state changes, and an out-of-channel signal changes nothing until the AGC lets go. In 12 of the 57
the detector had called nothing busy in the 90 s before — the signal was outside the channel or
under the shape path's notice; in the rest something was seen, which may or may not have been it.
The recordings (§6) answer this.

The session recordings in the uploads (17 of them, 2026-10-10…17) are sessions: the station's
own transmissions and the peer's frames dominate them, and none holds an idle AGC event. The one
"dip" `tools/floor_trace.py` finds in three of them is 5.6 s of digital silence in the pre-roll.

## 2. The mechanism, from the code

`core/aetherd/src/busy.rs` before ADR-0078:

1. A strong signal makes the AGC cut the gain. In the channel, the noise drops by the cut and is
   steady: every test of evidence passes (steady within 3 dB over 200 ms, above −70 dBFS, under
   no signal, not over the threshold, in no frame).
2. The median of the window's evidence follows it down once half the 10 s window has seen it.
3. The AGC lets go. The true noise is now the cut over the floor: every block is over the
   threshold, so none is evidence, and the level path calls busy.
4. Once the depressed blocks have left the window (10 s), there is no evidence at all; the floor
   is **held** for `floor_hold_s` (60 s), then takes the median of every block, which is the true
   noise, and the busy ends.

The evidence gate references the floor it updates, so once the true noise is more than the
threshold over the floor, nothing can bring the floor back but the hold — the literature's
*stagnation* (`01-survey.md` §3.2).

One more condition of it, found by building the reproduction: if the cut takes the noise in the
channel under −70 dBFS (the detector's digital-silence guard), the depressed blocks are not
evidence and nothing goes wrong. The author's station sits at −27 dBFS and was cut to −47: well
above it. A station whose USB audio is set low enough would never see the failure, and would see
the floor's −70 dBFS guard instead.

## 3. The tools

- **`aetherd --busy-trace WAV [--expect SIDECAR] [--bandwidth N] [--busy-variant NAME]`**
  (`core/aetherd/src/busy_trace.rs`): the daemon's own detector over a recording, block by block,
  as CSV — power, steady, over, peaked, under a signal, in a frame, evidence, held, the floor and
  threshold, busy and its reason, the shape — and from the 48 kHz audio, the power in the channel,
  in the rest of the radio's passband, the passband's median bin, the radio's gain as ADR-0078's
  reference reads it (`gain_db`) and the gain the floor was drawn at (`drawn_db`). With a sidecar
  it is muted where the station transmitted and told of the frames it found, as live.
  `--busy-variant plain` runs the floor as it was before ADR-0078.
- **`tools/floor_trace.py`**, rewritten on that trace (it re-implemented a five-second minimum
  that the detector had long stopped being): every AGC event in a recording — the gain reading 6 dB
  or more under its level on quiet for half a second or more — with its depth, length, release
  ramp, where its cause sat, what the floor did, and how long the channel read busy after it; and
  every busy-by-level span of 20 s or more. `--svg` draws it. On the corpus it recovers the AGC
  model's ramp: 44–49 dB/s for the `mid` preset's 50.
- **`tools/busy_corpus.py`**: the labelled synthetic corpus (`03-candidates.md` §2.1); `--seed N`
  draws it again with other numbers, the held-out check.
- **`tools/busy_eval.py`**: the candidates over the corpus, scored on the registered criteria;
  `--judge` holds one to them clip by clip.
- **`aether_model.qrm.Carrier`**, and **`windows`** for every QRM source: a steady carrier, and
  any signal keyed on fixed windows; the channel server takes both (`kind = "carrier"`,
  `windows = [[start, end]]`).
- **The harness's steps `at T`, `clear WHO S` and `busy WHO S`** (`tools/session_matrix.py`): a
  script lined up with a signal's windows, and the busy indicator held to a bound.

## 4. The AGC model

The harness's `Agc` (`tools/channel_server.py`): the gain steps down within a millisecond to keep
the passband's 1 ms peaks at a threshold over the noise, holds, then comes back at a fixed rate in
dB/s; it acts on the whole audio passband. The corpus's presets:

| Preset | Hold | Back at | Stands for |
|---|---|---|---|
| `fast` | 50 ms | 120 dB/s | FTDX10 FAST in DATA (160 ms decay) |
| `mid` | 100 ms | 50 dB/s | FTDX10 AUTO in DATA (MID, 500 ms); beta.17 measured 40–55 dB/s after about 100 ms |
| `slow` | 200 ms | 15 dB/s | FTDX10 SLOW in DATA (1500 ms) |
| `ssb-slow` | 300 ms | 7 dB/s | FTDX10 AUTO in USB (SLOW, 3000 ms); an IC-7300's SSB SLOW is 6 s |

Only `mid` is measured; the others are read from the decay times in the radios' manuals
(`01-survey.md` §3.8) and are provisional until the recordings fit them. The threshold sits 12 dB
over the band noise in 3 kHz, so noise alone never moves the gain. The corpus adds two cases the
harness did not have: a **hidden** driver, which the AGC answers to and the audio never carries
(a signal inside the roofing filter and outside the DSP filter — beta.17's dips were "triggered
outside the audio passband"), and a **narrow radio filter** around the channel.

## 5. The failure reproduced

**On synthetic audio, in a unit test** (`busy.rs`,
`a_gain_held_down_by_a_signal_outside_the_channel_reads_clear_when_it_lets_go`): white noise at
−29 dBFS in the channel, a carrier at 700 Hz 30 dB over it for 20 s, the `mid` AGC (21 dB down,
as the author's −27 went to −47), fed to the detector in 20 ms pieces through the same front end
as `Station::capture`. Before ADR-0078:

```
the channel read busy 47.0 s after the carrier had gone (last at 90.0 s): the floor had learned
the noise the AGC held down
```

— busy from the carrier's end to the end of the 90 s run.

**On the corpus**, the plain detector on group A (32 clips, 96 events: carriers, RTTY or FT8
outside the channel, every AGC preset): every event left the channel busy until the next event
or the clip's end, 40 s, 3 839 s of false busy in all; the floor sat a median 17 dB under the true
noise at the worst moments (95th percentile 32 dB).

**Through the station**, `station.rs`'s `the_channel_reads_clear_when_the_radios_agc_lets_go`:
the same scene at 500 Hz through `Station::capture`, the whole capture path. Before ADR-0078:

```
the channel read busy 37.0 s after the carrier had gone, the last at 80.0 s
```

— to the end of the run. Both tests pass with ADR-0078.

**In the harness**: `bench/scenarios/80m-agc-carrier-outside-500.toml` (a carrier at 700 Hz,
40 dB over the noise, for 30 s, at the caller; AGC AUTO; then `clear a 4` and a call). The result
is in §7.

## 6. What the recordings must settle

When the author's batches arrive (`00-recording-plan.md`):

1. **Depth and length** of each event, and how often per hour (`tools/floor_trace.py`).
2. **The release ramp** for each AGC setting, to fit the presets above (the FTDX10's FAST/MID/SLOW
   delays, written down by the author, give the expected values).
3. **Where the cause sat**: in the channel, elsewhere in the audio, or nowhere in the audio (a
   hidden driver). This decides how much of the fix rests on the reference outside the channel.
4. **The false busy** each event produced, and whether the chosen candidate removes it
   (`03-candidates.md` §2.4).
5. **Whether the radio's passband outside the channel follows the gain** as the model assumes —
   the reference's premise — and how far the radio's own audio noise sits under the band noise
   (the floor a deep cut runs into).

## 7. The harness

The A/B of `03-candidates.md` §2.3: ten scenarios, three draws each (`--seeds 3`), one arm on
beta.114's released daemon and one on this build, both through `tools/session_matrix.py` with
the same seeds. The ten are the four AGC scenarios added for this (a carrier 40 dB over the noise
at 700 Hz outside a 500 Hz channel for 30 s with AGC AUTO and with SLOW, the same at 1500 Hz inside
it, one at 2850 Hz on a 2300 Hz channel's edge), the whole `quick` set, and three where the busy
detector decides things (`80m-thunderstorm-agc-2300`, `20m-ft8-neighbours-2300`,
`80m-disconnect-slow-drain-500`).

| Scenario | Passed, beta.114 / new | Radiated overlaps | Air time median (range), s, beta.114 / new |
|---|---|---|---|
| 80m-agc-carrier-outside-500 | 0/3 / 3/3 | 0 / 0 | — / 355 (293–381) |
| 80m-agc-slow-carrier-outside-500 | 0/3 / 3/3 | 0 / 0 | — / 314 (298–335) |
| 80m-agc-carrier-inside-500 | 3/3 / 3/3 | 0 / 0 | 341 (328–347) / 279 (279–328) |
| 40m-agc-carrier-edge-2300 | 3/3 / 3/3 | 0 / 0 | 109 (108–110) / 109 (108–110) |
| quick-awgn-2300 | 3/3 / 3/3 | 0 / 0 | 57 / 57 |
| 40m-gateway-busy-when-called-2300 | 3/3 / 3/3 | 0 / 0 | 160 (145–196) / 158 (148–206) |
| 40m-trimode-gateway-2300 | 3/3 / 3/3 | 0 / 0 | 75 (72–89) / 75 (72–89) |
| 80m-thunderstorm-agc-2300 | 3/3 / 3/3 | 0 / 0 | 217 (213–262) / 239 (217–262) |
| 20m-ft8-neighbours-2300 | 3/3 / 3/3 | 0 / 0 | 102 (89–154) / 102 (89–154) |
| 80m-disconnect-slow-drain-500 | 3/3 / 3/3 | 0 / 0 | 336 (320–347) / 336 (328–347) |

On beta.114 both outside-the-channel scenarios fail at the step that waits for the channel to read
clear after the carrier (`clear a: not within 6 s of air`, 8 s for SLOW), so no call is made; on
this build all six pass. **H1 met.** Every scenario passes at least as often (30/30 against 24/30),
there are no radiated overlaps in either arm, and `tools/scenario_trend.py` with beta.114's runs as
the baseline finds nothing slower or failing. **H2 met.**

## 8. The session recordings through both floors

The 17 session recordings in KK4ODA-1's uploads (2026-10-10…17, 80 m and 40 m, 500 Hz) replayed
with their sidecars (muted where the station keyed, the frames it found taken out of the floor),
through the floor before ADR-0078 and the floor after it. They are sessions, not idle bands, but
they are the author's radio:

* **The passband outside the channel follows the radio's gain** — the reference's premise, seen on
  the author's FTDX10 for the first time. Each time the other station transmits, its own signal in
  the channel drives the AGC: on ND1J's session of 2026-10-15 02:37 the rest of the passband falls
  from −26 to −45…−50 dBFS for the length of each of his transmissions, the gain reading follows it
  (−17…−25 dB), and comes back with it. With the floor drawn at the gain, his signal stands about
  25 dB over the noise the AGC left beside it, and reads busy by level; the floor before had risen into
  his signal (−20 against −26) and read most of it clear. In a session the busy state holds
  nothing back; the panel's floor would show −45 there, which is what the noise in the audio is.
* **No false busy found.** In the stretches with nothing on the air — more than 0.5 s before and
  1.5 s after every frame the receiver found and every keying of the station's own, 798 s over the
  17 recordings — the old floor read busy by level for 48.7 s and the new for 81.4 s. The blocks only
  the new floor calls busy have the channel a median **7.6 dB** above the rest of the passband,
  against −5.4 dB in the blocks both call clear and +7.0 dB in those both call busy: energy in the
  channel that no frame accounts for. Most of it (17 s) is WC4Y's session of 2026-10-10 13:30, where
  flat energy (shape 7) sat 10 dB over the rest of the passband for 20 s and the old floor had taken
  it in (−18.4 dBFS against a noise of −27).
* **Digital silence** in a recording (the radio's audio muted around the station's own
  transmission past the keying the sidecar holds) reads as a gain of −70 dB; the floor holds
  through it (no block is evidence under −70 dBFS) and is back at the noise the moment audio is.

None of the 17 holds an idle AGC event of the kind the investigation is about; the author's
recordings (§6) are still what settles that.
