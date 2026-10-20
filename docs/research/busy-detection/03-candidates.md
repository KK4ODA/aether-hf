# The candidates, and how they are judged

The failure (`00-recording-plan.md`): a strong signal anywhere in the radio's passband makes the
AGC hold the gain down; the busy detector learns the depressed noise as its floor; when the AGC
lets go the true noise reads busy until the 60 s hold runs out, 71 s in all. The survey
(`01-survey.md`) ranks a same-instant reference from the audio outside the channel first. This
file names the candidates, registers how they are judged **before** the comparison runs (§2),
and then gives the results (§3) and the decision (§4).

Before §2 was written, one clip of the corpus (`A-carrier700-40-mid`) was run through four
candidates to check that the tools work; nothing else had been measured.

## 1. The candidates

All are built into one research build of `core/aetherd/src/busy.rs` behind
`BusyConfig::gain` and `flat_hold_s`, so they run the same code on the same audio
(`aetherd --busy-trace WAV --busy-variant NAME`). Names are those of the trace's
`--busy-variant`.

**`plain` — today's detector.** The floor is the median of the window's evidence in dBFS. The
reference for every comparison.

**`normalised` — C2, a gain-normalised floor.** Every 25 ms the station hands the detector the
radio's whole 48 kHz audio as well as the channel's baseband (`BusyDetector::hear_passband`). A
`PassbandReference` takes the 30th percentile of the 300–2700 Hz bins of that block's spectrum
(40 Hz bins, smoothed by a median over 125 ms). The AGC scales the whole passband by one factor,
and a signal can only add power, so this quantile falls only when the gain or the band's noise
really falls; a narrowband signal holds a few of its sixty bins and leaves it alone. The floor is
learned as the median of `power / reference` over the evidence blocks, and drawn at
`reference now`, never above the highest reference seen on an evidence block over the last 70 s
(the window and the hold). So a gain cut lowers the floor at once and the gain coming back raises
it at once, while a signal that fills the passband — which raises the reference — cannot raise
the floor above what was learned on quiet: the channel's own energy is still judged against the
noise. Cost: one 1200-point FFT and a 60-value sort per block. *(As registered. It changed five
times while it was measured — each bin against its own level on quiet, and more: §3.1; the final
one is ADR-0078 §2.)*

**`memory` — C1, a floor memory, the channel alone.** The floor of each of the last 180 s is
remembered. When the last second has been mostly plain (steady, not peaked, not in a frame) and
its median sits within 2 dB of a remembered floor at least the threshold above the present one,
those blocks are taken as the noise again. No reference; it cannot tell the noise coming back
from a flat signal starting at the old floor's level.

**`normalised-memory` — C2 and C1 together.** For a radio whose filter is narrower than the
reference band, where C2 has nothing outside the channel to read.

**`refuse` — C3, no evidence while the gain is cut.** The floor stays in dBFS, but no block is
evidence while the passband reference stands 3 dB or more under its ceiling: the floor is held
through the cut. The floor stays at the un-cut noise while the gain is cut, so a signal in the
channel during a cut is judged against a floor that is too high.

**`short-hold` — C6, a shorter hold for flat energy.** The floor accepts the median of
everything after 15 s instead of 60 s when nothing in the window was peaked. Shortens the false
busy to about 25 s, and accepts a long flat signal (OFDM, MT63, Olivia, VARA) as noise after the
same 25 s.

**Analysed, not built:**

- **C5, the radio's S-meter over CAT** (`01-survey.md` §4). Coarse (2–4 dB a step below S9),
  radio-specific, CAT-only, polled at best 5–10 times a second, and on the FTDX10 not yet known to
  show the cuts at all. It can be a witness later; it cannot be the fix for a station keyed by a
  serial line, VOX or a host program.
- **C7, a stagnation guard** (Gerkmann & Hendriks): force the floor up after the level has stood
  over it, flat, for some seconds. In the channel alone it is C6 with another name: it cannot tell
  the AGC letting go from a long flat signal starting.
- **C8, operator guidance**: a note on the panel when the gain moves, and the field notes'
  advice. Complements whichever is chosen; fixes nothing alone.

## 2. Acceptance criteria (registered before the runs)

### 2.1 The corpus

`tools/busy_corpus.py` builds 162 clips (330 minutes) of 48 kHz audio: band noise through a
2.7 kHz radio filter at −24 dBFS in 3 kHz, the harness's signals (`aether_model.qrm`, keyed on
fixed windows), and the harness's AGC model on the whole passband with four presets (hold, decay):
`fast` 50 ms, 120 dB/s; `mid` 100 ms, 50 dB/s (beta.17's measurement of the FTDX10 on AUTO in
DATA); `slow` 200 ms, 15 dB/s; `ssb-slow` 300 ms, 7 dB/s; and `off`. The AGC acts 12 dB above
the band noise's RMS. A codec floor at −92 dBFS sits under everything. Groups:

| Group | What | Clips |
|---|---|---|
| A | a carrier at 700 or 2400 Hz, RTTY at 2400 Hz or FT8 at 800 Hz, +25 or +40 dB, outside the 500 Hz channel, on three times for 20 s; every AGC preset | 32 |
| A2300 | a carrier at 2800 Hz, at the edge of a 2300 Hz channel | 4 |
| B | one signal in the channel (carrier, CW, RTTY, PACTOR, FT8, 500 Hz OFDM, 2.4 kHz OFDM) at −6…+20 dB, 30 s; AGC off and mid | 70 |
| B2300 | wide signals in a 2300 Hz channel | 9 |
| C | a signal in the channel (RTTY, 500 Hz OFDM, CW at 0/+6/+12 dB) while a +40 dB carrier outside holds the AGC down | 18 |
| D | noise alone, static crashes, a band of FT8 beside the channel; AGC off, fast, mid | 9 |
| E | a 120 s flat signal over the channel (2.4 kHz or 500 Hz OFDM, +3/+9/+15 dB) | 12 |
| F | a strong signal in the channel driving the AGC, with a 700 Hz radio filter (nothing outside to read) and with the normal filter; mid, slow, ssb-slow | 6 |
| G | the band's noise really stepping ±10 dB, no AGC | 2 |

`tools/busy_eval.py` runs every candidate over every clip, and the clip's noise alone (after the
same gain) through `plain` for the true floor. The measurements are defined in its docstring.

### 2.2 The criteria

A candidate is accepted only if it meets all of these. A failing criterion is reported with the
clip and the trace, not argued away.

1. **The release.** After every out-of-channel driver goes off (groups A and A2300, every AGC
   preset and level), the channel reads clear within **3 s**, and stays clear for 3 s. `plain`
   takes up to 71 s. The 3 s: the hangover (0.75 s), the attack window draining (0.2 s), the
   reference's smoothing (0.125 s), and the rest for an AGC's own recovery ramp; it is under the
   5 s other programs hold for (ARDOP, VarAC's wait). No release may be censored (still busy when
   the clip or the next event comes).
2. **No more missed occupancy.** For every transmission in the channel (groups B, B2300, C, E, F),
   the candidate's busy share is no lower than `plain`'s by more than **0.02**, and its first busy
   no later than `plain`'s by more than **0.05 s** (two blocks). Where `plain` calls a
   transmission busy for at least half of it, the candidate does too. For the long flat signals
   (E) the share is taken over the first 70 s — the floor's promise; past it, a signal is the
   environment by design, in every candidate.
3. **The existing tests.** Every test in `busy.rs` passes, unchanged.
4. **Nothing changes without an AGC.** With the AGC off or fast and nothing outside the channel
   driving it (groups D and G, and B with the AGC off): false busy no more than `plain`'s + 1 s
   per clip and no more episodes of 5 s or longer; the floor's median error within 0.5 dB of
   `plain`'s; each clip's busy share within 0.005 of `plain`'s.
5. **`attack_s` keeps its meaning.** `a_signal_is_called_busy_within_the_attack_time_wherever_it_starts`
   passes unchanged, and `attack_s()` is not changed: `sample_passband` still waits exactly that
   long before it trusts a window.
6. **The cost.** The extra work per 25 ms block is at most **50 µs** in a release build on the
   cloud machine (0.2 % of a core at 40 blocks a second); the author's mini PC, a few times slower,
   stays under 1 %, against a receiver that costs 8–10 % of real time (ADR-0010).
7. **The floor reads the true noise.** With the AGC acting (groups A, A2300, C outside its
   in-channel signal, D, F with the normal filter): per clip, the median |floor − true noise| at
   most **1.5 dB** and the 95th percentile at most **3 dB**, on blocks with nothing in the channel.
   In sessions, `a_weak_stations_frames_do_not_become_the_noise_floor` passes unchanged.

### 2.3 The harness A/B (`tools/session_matrix.py`)

On the new AGC scenarios and the `quick` set, at least 3 seeds each, this build against
beta.114's daemon:

1. **H1.** In the AGC scenarios, the step that waits for the channel to clear after the
   out-of-channel carrier ends passes within its bound with the fix and fails without it.
2. **H2.** Every scenario passes at least as often as with beta.114, with no more radiated
   overlaps in total, and each scenario's median air time within the nightly trend's tolerance
   (`tools/scenario_trend.py`: 15 % and 30 s past the longest baseline run).

### 2.4 On the air

When the author's recordings arrive (`00-recording-plan.md`), each AGC event found in them is
replayed through the chosen candidate: the channel must read clear within criterion 1's 3 s of
the AGC letting go, with no new episode of false busy of 5 s or longer anywhere in them.

## 3. Results

### 3.1 How the candidates were run, and what changed between runs

Seven runs, each judged by `tools/busy_eval.py --judge` against §2.2 as written. The criteria did
not change; the `normalised` candidate did, five times, and what each run found is why. **Disclosed:** groups H (a signal the AGC feels and the audio never carries — beta.17's dips
were "triggered outside the audio passband"), L (a cut longer than the window and the hold) and
C2300 (a wide signal in a 2 300 Hz channel under such a cut) were added to the corpus after the
first run, because that run showed the 2 300 Hz case and the hidden driver were not covered; they
are judged as the groups they extend (H and L as A, C2300 as C) and reported apart. Run 5's build
was run on a second draw of the whole corpus that nothing had been tuned on (`busy_corpus.py
--seed 1`), and it found a defect the first draw had not (run 6); once that was fixed, the final
build was run on all three draws, the third (`--seed 2`) held out until then (§3.3).

| Run | `normalised` as built | Clips failing criteria 1 / 2 / 4 / 7 (registered groups) | What it showed |
|---|---|---|---|
| 1 | the passband's 30th-percentile level over all its bins, drawn under its highest level on quiet | 0 / 10 / 7 / 28 | the release fixed; a signal in a 500 Hz channel lifted the reference with itself |
| 2 | the channel's bins left out where 20 remain; H, L, C2300 added | 0 / 8 / 13 / 12 | the level of the passband swings with a band of signals beside it (D-ft8band); a wide signal in a 2 300 Hz channel under a cut vanished (C2300: caught 0.00) |
| 3 | **each bin against its own level on quiet**; 25th percentile of the differences; the floor at 2 300 Hz up only within 3 dB of 0 | 0 / 6 / 3 / 11 | the floor wandered with the reading (late first busy on weak signals); an FT8 station in every period taught its bins its own level, and the gaps between periods read as cuts (D-ft8band-off: 28 s) |
| 4 | a 2 dB dead band with a one-second ease; only bins within 10 dB of the channel's | 0 / 4 / 5 / 11 | a dense band of strong FT8 beside the channel leaves three or four bins on the noise: the reading was no gain at all (D-ft8band-mid: 84 s against the old floor's 28); a fast AGC's dip after a crash was read 100 ms late |
| sweep | eight settings of smoothing and of an agreement test (a reading stands only when half the bins sit within 3–4 dB of it) on 76 of the hardest clips | — | three readings, not five; the agreement test took D-ft8band back to the old floor's figures |
| 5 | the reading the median of three; it stands when half the bins sit within 4 dB of it | 0 / 4 / 3 / 10 | the sweep's figures held on all 183 clips |
| 6 | run 5's build on a second draw of the corpus (`--seed 1`), nothing tuned on it | 0 / 1 / 3 / 12 | **a static crash in the first reading** (`D-crashes-off`, no AGC): each bin's level on quiet was started from that one reading, two seconds of warm-up steps brought it only 16 dB down, every reading after said "cut by 9 dB", and the bins, which learn only from uncut readings, never came down — busy 47 % of the time |
| 7 | **final**: each bin's level on quiet set from its 25th percentile over the two-second warm-up; the agreement test removed — with the warm-up fixed it changed nothing on the 76 clips (the FT8 bands' trouble had been the first reading taken inside an FT8 period) | 0 / 6 / 1 / 10 | §3.2; then once on a third draw (`--seed 2`), §3.3 |

The other candidates were judged on run 2 (they did not change after it):

| Candidate | Criteria 1 / 2 / 4 / 7 failing (registered groups) | Why it is not chosen |
|---|---|---|
| `memory` (C1) | 24 / 1 / 2 / 53 | it can only take a floor back once the returned level has been plain for a second and matches one it remembers: releases of 2–8 s; it cut the busy share by a quarter where the AGC pumps on a band of FT8 (D-ft8band) |
| `refuse` (C3) | 0 / 19 / 6 / 53 | the floor stays at the uncut noise under the cut, so a signal in the channel under it is judged against a floor tens of dB too high (C-cw +0 dB: caught 0.00 against 0.99) |
| `short-hold` (C6) | 32 / 16 / 7 / 54 | 27 s instead of 71, and a long flat signal is noise after 25 s |
| `normalised-memory` | as `normalised`, plus F-narrow 20 → 2 s | C1's cost (above) for one case: a radio filter narrower than the reference |

### 3.2 The final candidate against the criteria

Run 7, the final build, on the corpus all the tuning was done on (183 clips; `plain` from the same
run). Before → after, by group (release: the longest, with how many never came within the clip;
false busy and episodes of 5 s or more summed over the group's clips; share: the mean share of each
transmission in the channel called busy, over its first 70 s for E):

| Group | `plain` | `normalised` | Share caught, `plain` / `normalised` |
|---|---|---|---|
| A — outside the 500 Hz channel, 96 events | 40 s, 96 never; 3 839 s false busy, 96 episodes | 1.2 s; 1 s, 0 | — |
| A2300 — the filter's edge, 2 300 Hz | 0 s false busy | 4 s | 0.00 / 1.00 (the carrier sits in the channel as the detector hears it) |
| H — a driver the audio never carries, 36 events | 40 s, 36 never; 1 436 s, 36 | 500 Hz: never busy; 2 300 Hz: 5.8 s; 41 s, 0 | — |
| L — a two-minute cut | 70 s, 6 never; 417 s, 6 | 3.0 s; 4 s, 0 | — |
| B, B2300 — a signal in the channel | 0 s | 0 s | 0.72 / 0.74, 0.58 / 0.61 |
| C — a signal in the channel under a cut | 350 s, 18 | 0 s, 0 | 0.93 / 0.93 |
| C2300 | 59 s, 3 | 4 s, 0 | 0.67 / 0.67 |
| D — no AGC trouble | 58 s, 0 | 58 s, 0 | — |
| E — a 120 s flat signal | 0 s | 0 s | 0.51 / 0.67 |
| F — a strong signal in the channel drives the AGC | 57 s, 4 | 20 s, 2 (all with a filter too narrow to read the gain) | 1.00 / 1.00 |
| G — the band's noise steps ±10 dB, no AGC | 71 s, 1 | 71 s, 1 (the noise rising for real: by design) | — |

Criterion by criterion:

1. **The release — met** on every registered clip: the channel reads clear within 1.2 s of every
   one of group A's 96 events, every AGC preset (`plain`: 96 never, 40 s). Of the later groups, H
   and L at 500 Hz never read busy; at 2 300 Hz three clips miss the 3 s, `H2300-hidden-45-ssb-slow`
   (5.7–5.8 s), `H2300-hidden-30-ssb-slow` (3.6–4.0 s) and `H2300-hidden-45-slow` (3.3–3.5 s): there
   the reading includes the channel, the floor waits until the gain is within 3 dB of normal
   (ADR-0078 §2.3), and a 33 dB cut coming back at 7–15 dB/s takes that long.
2. **No more missed occupancy — six clips fail, each on the first busy of a signal on the
   threshold or on `plain`'s own false busy.** `C-rtty-+0-slow` (0.60 s against 0.30), `C-rtty-+0-mid`
   (0.40 / 0.27), `C-ofdm500-+0-slow` (0.38 / 0.27), `C-cw-+0-slow` (0.40 / 0.33), `B-cw-+0-off`
   (0.60 / 0.55): 0 dB in 3 kHz is 6.0–6.5 dB over the channel's noise, the threshold is 6, and the
   first block over it falls on either side of the other floor by chance. The two floors differ
   there by a median −0.17…+0.12 dB, sign varying, ±0.3–1.0 dB at the 5th and 95th percentiles,
   on all three draws (`C-*-+0-*`, 30–44.5 s); with no AGC, within ±0.2 dB. Over every
   transmission of the three draws, the new floor called the first busy earlier than `plain` 34
   times, later 11 times, at the same block 254 times, and no transmission lost more than 0.04 of
   its share (median change 0). `F-wide-rtty-30-ssb-slow` (0.20 / 0.00): `plain` was still busy from
   its own stagnation when the second transmission began.
3. **The existing tests — met**: every test in `busy.rs` from before ADR-0078 passes unchanged
   (they hand over no audio, so they run the floor as before), and the whole of `cargo test` passes.
4. **Nothing changes without an AGC — one clip fails, by 0.001.** `B-ofdm500-+0-off`: busy share
   0.130 against 0.136, 0.005 allowed (the transmission's share within 0.02). D, G and the other
   B clips: met. (On run 5 the dense FT8 bands of D failed here; with the warm-up fixed they read
   exactly as `plain` does.)
5. **`attack_s` — met**: unchanged, and its test unchanged.
6. **The cost — met**: 10.7 µs more a 25 ms block (21.0 against 10.3 for the detector alone,
   release build, the cloud machine's 2.1 GHz Xeon), against 50 µs allowed: 0.04 % of a core.
7. **The floor reads the true noise — every median met; ten clips fail on the 95th percentile.**
   `A-rtty2400-40-*` (4.8–6.4 dB): a +40 dB RTTY's keying sidebands put about the noise's own level
   into the channel 1 kHz away (through the event the channel reads 4.5 dB over the clip's
   noise-alone audio), so the floor is right about what is there and the truth is not (`plain`
   misses by 26–31 dB). `C-ofdm500-+0-*` (3.7–4.9): the same at the cut's edges. `D-crashes-fast/-mid`
   (9.5/11.9; `plain` 1.5/13.0): a crash moves a fast AGC within 0.2 s, the floor follows the gain,
   and the truth is a 1 s running median that does not. `D-ft8band-fast/-mid` (27.8/28.2): exactly
   `plain`'s — there the gain cannot be read and the floor behaves as it did. Of the later groups,
   `H2300-hidden-45-ssb-slow` (8.4), criterion 1's ramp.

### 3.3 Held out

**Run 6** (run 5's build on the second draw, `--seed 1`) found the warm-up defect run 7 fixed:
`D-crashes-off` busy 47 % of the time. That draw was then no longer held out.

**The third draw** (`--seed 2`), built after run 7's code was fixed and run once with that frozen
build (`aetherd-final2`), nothing looked at before:

| Group | `plain` | `normalised` | Share caught, `plain` / `normalised` |
|---|---|---|---|
| A | 40 s, 95 never; 3 807 s, 96 | 1.2 s; 2 s, 0 | — |
| H | 40 s, 36 never; 1 436 s, 36 | 6.1 s (2 300 Hz); 42 s, 0 | — |
| L | 70 s, 6 never; 417 s, 6 | 2.9 s; 4 s, 0 | — |
| C | 350 s, 18 | 0 s | 0.97 / 0.96 |
| F | 67 s, 5 | 19 s, 2 | 1.00 / 1.00 |
| B, D, E, G | 4, 58, 15, 71 s | 5, 58, 15, 71 s | 0.74 / 0.76, —, 0.63 / 0.63, — |

Judged: criterion 1 met on every registered clip; the later groups' three 2 300 Hz clips again
(3.2–6.1 s). Criterion 2: `C-cw-+0-mid` (0.40 / 0.30), `C-ofdm500-+0-mid` (share 0.35 / 0.38, first
busy 1.10 / 0.48) — signals on the threshold — and `F-wide-rtty-30-slow` (0.20 / 0.00, `plain`'s
own false busy). Criterion 4: `G-noise-step+10`, floor error median 2.81 dB against 1.54: both
floors stagnate the same 70 s on the band's real 10 dB rise (2 799 blocks off by 10 dB in each), and
with half the clip's blocks that far off, the median of |error| sits on the edge between the two
groups and moves with small differences elsewhere. Criterion 7: nine of run 7's ten clips, for the
same reasons. Nothing new.

### 3.4 The harness (§2.3)

Ten scenarios × three draws, beta.114's daemon against this build (`02-measurements.md` §7):
both AGC scenarios with the carrier outside the channel fail all six runs on beta.114 (the channel
never reads clear within 6–8 s of the carrier going, so no call is made) and pass all six on this
build — **H1 met**; every scenario passes at least as often (30/30 against 24/30), no radiated
overlap in either arm, and nothing slower by the nightly's trend rules — **H2 met**.

### 3.5 On the air (§2.4)

The author's first recordings (2026-10-18: 40 m, AUTO, FAST and SLOW; `02-measurements.md` §9),
replayed with `tools/floor_trace.py`. **Each recording alone, the build of run 7 meets §2.4**: on the
slow-AGC recording the channel read clear 0.5 s, 0 and 0 s after its three 13 s cuts (`plain`:
16.9 s, 21.1 s, and 40 s to the end of the file), and it adds no false busy of 5 s or more anywhere —
every stretch it reads busy and `plain` did not holds a signal in the channel or on its edge.

**The seven joined end to end, as a running daemon would have heard them, it fails.** A 26 dB rise
of the whole passband at the radio, before the first recording, left the reference reading 23 dB over
its levels on quiet once the hold had taken the new level as the environment, and every cut after it
read as no cut: six false busy spans of 33–71 s, as `plain`'s six. Fixed in the reference's rebase
(ADR-0078 §5); with the fix, one span — the 68 s both floors take to accept the rise itself, as any
rise of the band's noise (group G).

The fix judged again on the three draws of the corpus: the first and the third identical, clip by
clip, to run 7; on the second, one clip more fails criterion 4, `G-noise-step+10` (floor error median
1.050 dB against `plain`'s 0.545, 0.5 allowed: 0.005 over). It is the third draw's `G-noise-step+10`
again (§3.3): the floors after the hold has run out differ by 0.1 dB (−18.3 against −18.2 dBFS), and
with half the clip's blocks 10 dB off for the minute both floors take on the rise, the median of the
error sits on the edge between the two groups. Its false busy, its episodes and its share are
`plain`'s.

## 4. Decision

`normalised` as built for run 7 (ADR-0078). It meets the criterion the investigation is about — the
release — on every registered clip of both draws it was judged on, within about a second at 500 Hz
and within the AGC's own ramp at 2 300 Hz, against 40–70 s. The failures of criteria 2, 4 and 7 are
each traced above to a signal on the threshold (over all three draws it was called busy earlier than
by the old floor three times as often as later), a band the reference cannot see (where it behaves
as the old floor did), or a truth that is not the channel's. The 2 300 Hz releases over 3 s are the
price of not losing a wide signal in the channel under a cut (C2300: 0.67 of it caught, as by the
old floor; 0.00 in run 2, before the floor waited at 2 300 Hz). No other candidate comes close on
criterion 1 without failing criterion 2 by tens of clips. The session recordings
(`02-measurements.md` §8) found no false busy it adds, and the harness (§3.4) is the last check
before it ships.
