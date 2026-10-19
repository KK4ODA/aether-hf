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
noise. Cost: one 1200-point FFT and a 60-value sort per block.

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
